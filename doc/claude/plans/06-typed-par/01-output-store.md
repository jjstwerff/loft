<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 1 — Workers write to per-worker output Stores

**Status: open**

## Goal

Replace the three different "where does worker output go?" paths
(`out_ptr` raw bytes, `Vec<String>` channel, `Vec<DbRef>` channel)
with one uniform path: every worker writes its output into a
pre-allocated **slot in its own `WorkerStores.allocations`**,
exactly the way every loft fn already writes its return value.

The output slot is a regular `Store` inside the worker's
`WorkerStores` table — addressable via a normal `DbRef` and
written via existing `OpSet*` opcodes.  After join, the parent
calls `WorkerStores::take_slot(N)` to extract that one Store and
`Stores::adopt_store(store)` to install it into the parent's
allocations.  See DESIGN.md D2 / D2.1 for the rationale (no
opcode surface change; uses the existing return-value convention
across the thread boundary).

Phase 1 is **transitional**: the three native fns
(`n_parallel_for_native` / `_text_native` / `_ref_native`) still
exist, dispatch is unchanged, the user-visible surface is unchanged.
What changes is **where the worker writes the result**.  Stitching
into a single result vector still goes through the existing copy
logic — phase 2 retires that.

**Important finding from phase 0a's source survey.**  The
interpreter and native-codegen paths were originally **structurally
different**:

- `src/parallel.rs::run_parallel_direct` (the interpreter path)
  IS parallel — uses `thread::scope` with three `#[cfg]` variants
  (threading + wasm, threading + non-wasm, no-threading).  The
  worker is a bytecode fn dispatched via `state.execute_at_raw()`.
- `src/codegen_runtime.rs:1582::n_parallel_for_native` (the
  native-codegen path) **was sequential by mistake** — a plain
  `for i in 0..n` loop calling the worker closure inline.

**G4 status (verified 2026-04-25):** phase 1a's
`thread::scope` fix has **already landed** — the comment header
at `codegen_runtime.rs:1582` documents the closure ("Plan-06
phase 1a (G4): runs workers in parallel via `thread::scope` with
per-worker `Stores` clones").  Bench/11_par should now reflect
real native parallelism; verify with `make bench` before
calling phase 1a fully done.

Phase 1b (text output stores) and 1c (reference output stores)
remain open — the per-worker-output-store migration for those two
return shapes still needs the workers to write into output Stores
instead of channels.

Phase 1's migration shape is therefore the **same** for the
remaining text/reference paths as for the now-shipped primitive
path:

- Each worker thread receives an exclusive output Store.
- The parent reads from per-worker stores after join.
- The interpreter path already has the threading scaffolding;
  phase 1 just changes where the worker writes.
- The native path needs the threading scaffolding added; the
  generated Rust closure must be Send-friendly (most compute
  workers already are).

After phase 1 the native bench should be in rust's range — that's
the regression test: loft-native ≤ ~5 ms on the bench-11 workload
(today's 4 ms rust threshold + a small loft-overhead margin).
If it's not, phase 1's native-side work is incomplete.

## Why transitional

Trying to unify everything (output store + stitch + native dispatch)
in one phase would mean ~1500 lines of churn in one PR.  Phase 1
isolates the cheapest change (the worker side) so we can validate
that "everything writes to a store" works before touching the
collection side.  Phase 0's bench harness gates the perf regression.

## What changes per return-type path

| Today's path | Worker writes to | After phase 1 worker writes to |
|---|---|---|
| Direct (primitive) | `out_ptr` raw byte slice owned by main thread | Output slot in worker's `WorkerStores` (regular Store, written via `OpSetInt`/`OpSetLong`); main thread `take_slot` + `adopt_store` |
| Text | `Vec<String>` sent via mpsc channel | Output slot with text fields (`OpSetText`); main thread reads via the adopted store |
| Reference (struct) | `Vec<DbRef>` sent via mpsc channel + `copy_block` + `copy_claims` | Output slot containing the worker's struct records (`OpSetRef`/`OpVectorAdd`); main thread `copy_block`s from the adopted slot into the result store (one less indirection — no channel; phase 2 retires the `copy_block` itself) |

The three paths still **dispatch to three different native fns**
because the result-vector type differs (vector<i32> vs. vector<text>
vs. vector<Struct>).  Phase 3 collapses the dispatch.  Phase 1 only
makes the workers' write target uniform.

**No new opcodes** — workers use the same `OpSet*` opcodes any loft
fn uses to write its return value.  The output slot is just a
regular `WorkerStores.allocations[N]` entry; the dispatcher tells
the worker its slot number `N` via the call frame.

## Changes per cross-cutting concern

| Concern (from DESIGN.md) | Phase 1 contribution |
|---|---|
| D1 Stitch policy | Not introduced yet; phase 1 keeps today's `Concat` behaviour implicitly |
| D2 Worker / parent relationship | Each worker's output store is owned exclusively by the worker thread.  Parent stores still locked-cloned (today's mechanism) until phase 2 lifts the relationship. |
| D5 Empty / degenerate inputs | New code paths must handle empty input (allocate 0-element output store, skip workers) and single-worker (allocate one output store, no parallelism) |
| D6 WASM single-threaded | The output-store allocation happens in the calling thread; workers run sequentially.  No queue, no stitch. |

## Per-commit landing plan

### 1a — primitive output stores

Touch points:
- `src/parallel.rs::run_parallel_direct` (currently writes via `out_ptr`).
- `src/codegen_runtime.rs:1581-1700` (`n_parallel_for_native` and the four `parallel_get_*` getters).
- `src/database/allocation.rs:449` (`clone_for_worker` — needs to leave a slot at index `N` writable for the output).
- `src/database/mod.rs` (`WorkerStores::add_output_slot`,
  `WorkerStores::take_slot`, `WorkerOutputSlot { store_nr: u16 }`
  marker, `Stores::adopt_store`).

Mechanic:
1. Before spawning workers, the dispatcher constructs each worker's
   `WorkerStores` (cloned from parent) and calls
   `WorkerStores::add_output_slot(slot_words)` to append a fresh
   empty Store at index `N`.  Returns `WorkerOutputSlot { store_nr: N }`.
   `slot_words` is sized for `(input_count + threads - 1) / threads`
   elements at the worker fn's return-type element width.
2. Each worker receives its `WorkerStores` AND its
   `WorkerOutputSlot` via the dispatcher's call frame.  The worker
   writes return values into a `DbRef { store_nr: N, rec, pos }`
   using the existing `OpSet*` opcodes — no new opcode path.
3. After join, main thread calls
   `worker_stores.take_slot(N) -> Store` to extract the output
   buffer, then `parent.adopt_store(store) -> u16` to install it
   in the parent's `allocations`.  The Store keeps its bytes —
   no memcpy.
4. For phase 1's transitional state, the parent then walks the
   adopted store and copies values into the existing
   `out_ptr`-shaped result vector using existing `vector_add`-style
   ops.  Phase 2 retires this copy via the rebase pass.

Post-commit: `make ci` green; phase 0 characterisation suite passes;
bench-1 (1 M `i64`, 4 threads) within ±5 % of phase 0 baseline (the
expectation is **slightly slower** than today's raw `out_ptr` write
because we add one indirection — the slowdown is acceptable as the
cost of giving up the unsafe pointer; phase 2 + 3 will reclaim the
cost via a faster stitch).

### 1b — text output stores

Touch points:
- `src/parallel.rs::run_parallel_text` (currently uses
  `mpsc::channel<Vec<String>>`).
- `src/codegen_runtime.rs::n_parallel_for_text_native`.

Mechanic:
1. Worker's output slot allocated as a `vector<text>`-shaped Store
   in the worker's `WorkerStores.allocations[N]`.
2. Worker writes its text result via the existing `OpSetText`
   opcode targeting `DbRef { store_nr: N, rec, pos }` — same path
   text always uses inside loft.
3. After join, main-thread `take_slot(N)` + `adopt_store` per
   worker; copies the text-pointer entries from each adopted
   store into the result vector.  No channel.

Specific issue resolved: today's `Vec<String>` channel allocates a
`String` per result + an `mpsc` slot + a final main-thread copy.
Phase 1b reduces this to one `OpSetText` write per result + main-
thread copy; the channel is gone.

Bench-3 (100 K text results) expected: ±5 % of phase 0 baseline,
likely slightly faster from removing the channel.

### 1c — reference output stores

Touch points:
- `src/parallel.rs::run_parallel_ref` (currently uses
  `mpsc::channel<DbRef>` + main-thread `copy_block` + `copy_claims`).
- `src/codegen_runtime.rs::n_parallel_for_ref_native`.

Mechanic:
1. Worker's output slot allocated as a `vector<Reference<T>>`-shaped
   Store in the worker's `WorkerStores.allocations[N]`.
2. Worker constructs its struct result inside its own
   `WorkerStores` (any slot); the result-vector record gets pushed
   into slot `N` via `OpVectorAdd` — same path any loft fn
   returning a struct uses.  Internal worker stores
   (allocations[0..N-1] sub-stores allocated during the worker's
   computation, e.g. text bytes pointed at by the output records)
   stay in the worker's `WorkerStores` — they will also be adopted
   in phase 2 (the rebase pass walks all referenced sub-stores).
3. After join, main-thread `take_slot(N)` + `adopt_store` per
   worker; `copy_block`s each adopted slot's vector contents into
   the result vector.  Channel removed.

The `copy_claims` machinery still runs in phase 1c — phase 2 retires
it via the rebase pass (which adopts ALL referenced worker stores,
not just the output slot, and rewrites cross-store DbRefs).

Bench-2 (100 K struct results) expected: similar to phase 0
baseline; the channel removal saves a small amount, the extra
intermediate store costs a similar amount.

## Loft-side prerequisites

Three new accessors — all small, all on existing types.  See
DESIGN.md D2.1 for why these collapse to slot-marker + adoption
instead of a parallel wrapper type.

```rust
// src/database/mod.rs
/// Marker telling the parent which slot in this worker's
/// WorkerStores to extract after join.  Just a u16 — no Drop logic;
/// the worker's WorkerStores owns the Store until take_slot.
pub struct WorkerOutputSlot {
    pub store_nr: u16,
}

impl WorkerStores {
    /// Append a fresh empty Store at the end of allocations and
    /// return its slot index.  Called by the dispatcher right
    /// after clone_for_worker, before handing the WorkerStores
    /// to the worker thread.
    pub fn add_output_slot(&mut self, slot_words: u32) -> WorkerOutputSlot;

    /// Extract the Store at `slot_nr`, leaving a freed sentinel
    /// in its place.  Called by the parent after join, before
    /// adopting into its own table.
    ///
    /// # Panics
    /// Panics if the slot has already been taken.
    pub fn take_slot(&mut self, slot_nr: u16) -> Store;
}

impl Stores {
    /// Install an externally-allocated Store into this Stores'
    /// allocations table.  Returns the parent-side store_nr.
    /// Used by the parent thread to adopt a worker's output slot.
    pub fn adopt_store(&mut self, store: Store) -> u16;
}
```

Lifetime contract: the output slot lives inside the worker's
`WorkerStores` until `take_slot` extracts it; if the worker
panics and the parent never calls `take_slot`, the slot is freed
along with the rest of the worker's `WorkerStores` (via
`Store::Drop`).  Adoption is unidirectional — worker writes →
parent reads, never the reverse.

## Test fixtures

All existing phase-0 characterisation tests **must keep passing
byte-for-byte**.  Phase 1 changes the runtime path but not the
output.

New fixtures:

| Fixture | Asserts |
|---|---|
| `tests/issues.rs::par_phase1_output_slot_lifetime` | Output slots allocated, written, taken, and adopted in matched pairs; no leak under `LOFT_STORES=warn` |
| `tests/issues.rs::par_phase1_text_no_channel` | `mpsc` channel allocation is zero (verify by patching `tests/parallel_intrumentation.rs` to count channel allocs; the count drops to zero after 1b) |
| `tests/issues.rs::par_phase1_empty_input_no_worker_alloc` | `len(input) == 0` returns immediately; no output slot allocated |
| `tests/issues.rs::par_phase1_panic_propagation` | Worker panics on element 5; parent receives panic; the worker's `WorkerStores` (including the output slot) is dropped cleanly with no orphan stores; `LOFT_STORES=warn` reports zero leaks |
| `tests/issues.rs::par_phase1_no_new_opcodes` | `LOFT_LOG=static` dump of a par-using program shows no new `OpSet*Output` opcodes — workers use existing `OpSetInt` / `OpSetText` / `OpSetRef` / `OpVectorAdd` |

## Acceptance criteria

- All phase-0 characterisation tests pass byte-for-byte.
- New fixtures pass on Linux / macOS / Windows.
- Bench-1 (primitive) within ±5 % of phase 0 baseline.
- Bench-2 (struct) within ±5 % of phase 0 baseline.
- Bench-3 (text) within ±5 % of phase 0 baseline (small improvement
  expected from channel removal but not required).
- `make ci` green at every sub-commit (1a / 1b / 1c).
- `LOFT_STORES=warn` reports zero leaked stores across the full
  suite.

## Risks

| Risk | Mitigation |
|---|---|
| Per-worker output store allocation dominates bench-1's tight loop | Pool output stores by `(elem_type, slot_count)` keyed cache; reuse across calls.  Evaluate at end of 1a; defer if unneeded. |
| Three native fns now allocate output stores three different ways | Phase 1 keeps the dispatch; phase 3 collapses it.  Triple paths is acceptable as transitional state. |
| Struct results in 1c need worker-store-to-result-store copy | `copy_block` already handles this; phase 2 retires `copy_block` in favour of the rebase pass. |
| WASM target's sequential fallback also needs the output-store allocation | Allocate output store in the calling thread, write directly into the result vector (no separate worker store).  Identical-output sequential path. |

## Out of scope

- Any change to the dispatch (3 native fns stay).
- Any change to the user surface.
- The `claims` HashSet overhead from D2 — retired in phase 2.
- Stitch policy enum — phase 3.
- Auto-light heuristic — phase 5.

## Surface gaps closed by phase 1

Phase 0's characterisation work surfaced 3 pre-existing par
limitations (G1, G2, G3) that the new uniform output-store mechanism
resolves.  Implementation surfaced 4 more (G2.1, G4, G5/G5.1, G6)
that needed targeted dispatcher work beyond the original plan.

The inventory below captures what landed vs what remains.  Each
gap has an `#[ignore]`d test in `tests/threading_chars.rs`; the
gap's "closed" status means the matching test is now positive.

### G1 — struct-enum return types — **closed**

Original diagnosis: parser at `src/parser/collections.rs:1209`
rejected worker return types with `var_size > 8`.

Actual fix (deviation): the parser was already routing
`Type::Enum(_, true, _)` (struct-enums) through the ref path with
`return_size = -1`, but `is_primitive_return` in
`src/parser/collections.rs` did NOT exclude that type, so workers
auto-routed to `n_parallel_for_light` instead.  The light path's
`execute_at_raw` truncates returns at 8 bytes, silently dropping
the variant payload — the test returned `0` instead of `30`
because all output slots stayed uninitialised (dump showed
`unknown(0)` / `unknown(160)` instead of variant data).

Fix: added `Type::Enum(_, true, _)` to the
`is_primitive_return` exclusion list so struct-enums route to the
heavy `n_parallel_for` path that has the deep-copy machinery.
`tests/threading_chars.rs::par_struct_to_struct_enum_t4` is now
positive.  Closes via commit `0717c7f`.

### G2 — primitive-element input vectors — **closed**

Original diagnosis: workers read input elements with a 12-byte
DbRef stride regardless of the actual narrow encoding.  Workers
of `vector<integer>` etc. saw garbage.

Actual fix (deviation): the bug had two layers, not one.

1. **Worker first-param dispatch** — workers whose first
   parameter is a primitive type (bool / byte / integer / single /
   float / character / non-payload enum) need the inline value in
   slot 0, NOT a `DbRef`.  The original plan assumed the existing
   ref-passing convention worked for primitives; it doesn't.
   Required: new `State::execute_at_raw_primitive_input(fn_pos,
   value, slot_size, extras, ret_size)` that pushes the primitive
   at its native byte width (1/4/8) into slot 0; new
   `read_primitive_at(stores, row_ref, size)` helper.
   `run_parallel_light` and `run_parallel_direct` gain a
   `primitive_input_size` parameter selecting the dispatch path.
   Detection via `primitive_first_arg_slot_size(def)` in
   `src/native.rs`.

2. **Narrow-integer stride (G2.1)** — the parser-side
   `elem_size` for `vector<u8>` / `vector<i32>` used
   `var_size(Type::Integer(_), Argument)` which returns `8` for
   any Integer.  The actual storage stride for typedef-narrowed
   integers honours `IntegerSpec::vector_narrow_width()` (1/2/4).
   Required: parser fix to query `vector_narrow_width()` first,
   matching the iterator dispatch in `collections.rs:105-113`
   for non-par for loops.

3. **Slot-width promotion (G2.1 cont)** — even with stride
   fixed, narrow integer ARGS get promoted to 8-byte integer
   slots by loft's calling convention (e.g. `fn dbl(x: i32) ->
   integer` reads `x` via `OpVarInt` → 8 bytes from slot 0).
   Required: split `read_size = element_size` (vector stride)
   from `push_size = primitive_first_arg_slot_size(def)` (worker
   slot width, always 1/4/8 from arg type).  `read_primitive_at`
   reads at the smaller width; `execute_at_raw_primitive_input`
   pushes into the wider slot.

Closes 5 canaries: `par_enum_input_t4` (1-byte enum),
`par_int_to_int_t4_primitive_input` (8-byte integer),
`par_float_input_t4` (8-byte float), `par_i32_input_t4` (4-byte
narrow), `par_u8_input_t4` (1-byte narrow).  Commits `b13d01c`,
`a9c2f1e`.

### G3 — `--native-wasm` rejects par at codegen — **scoped to phase 8**

Pre-existing wasm codegen issue documented in original plan;
deferred to phase 8 (browser workers).  Phase 1 doesn't touch it.

### G3 (different gap) — text-input par dispatch — **closed**

Surfaced during G2 work: workers whose first param is `Type::Text(_)`
need a 16-byte `Str { ptr, len }` slot in slot 0, not a 12-byte
`DbRef`.  The original plan didn't separate this from the
primitive case, but the slot widths differ (text=16 vs primitive
≤ 8) and Str values must be reconstructed from the row's text
pointer.

Fix: `State::execute_at_raw_text_input(fn_pos, str, extras,
ret_size)` pushes a 16-byte Str; `read_text_at(stores, row_ref)`
reconstructs the Str by reading the u32 text-rec at `row.pos`
and calling `store.get_str()`.  `primitive_input_size` gains a
sentinel `u32::MAX` selecting the text path.

`tests/threading_chars.rs::par_text_input_t4` is now positive.
Commit `11f1a73`.

### G4 — fn-ref return — **partial scaffolding, codegen layout pending**

Not in original plan.  Surfaced when investigating remaining
canaries: `par_struct_to_fn_t4` returns a 20-byte fn-ref (8B
d_nr + 12B closure DbRef) which exceeded the 8-byte primitive
return cap.

Partial fix landed: `State::execute_at_raw_to(fn_pos, arg, extras,
return_size, dst)` copies arbitrary `return_size` bytes from
worker stack to a destination buffer; `run_parallel_direct` uses
it for `ret_sz > 8`.  Parser accepts `Type::Function` returns
(sz=20) and excludes them from the light path.  `n_parallel_for`
derives `return_size=20` for fn-ref returns.

Remaining blocker: stack snapshot at Return time shows the
worker's d_nr correctly at `stack[4..11]` but the closure DbRef
sentinel appears at `stack[20..23]` instead of `stack[12..23]` —
the codegen's `stack_pos` accounting for `n_pick` uses a
different convention than `execute_at_raw_to`'s setup, and
reconciling needs deep codegen-side work.  Deferred to phase 4
(typed surface).

`par_struct_to_fn_t4` remains `#[ignore]`d.

### G5 / G5.1 — par-safety analyser precision — **closed**

Not in original plan.  Surfaced after the deep par-safety check
landed at `Level::Error`: legitimate workers writing to
worker-LOCAL vectors got false positives from the
`OpAppendVector` / `OpNewRecord` recursion.

G5: the deep walker now treats `Purity::Impure(ParentWrite)`
calls as safe when the first argument is a `Value::Var(v)` and
`v` is NOT an argument of the calling function.  Required
threading `current_fn: u32` through `walk_deep_parent_write`.

G5.1: heap-typed return values get promoted by `ref_return()` to
a hidden caller argument with `argument: true` AND
`def.attributes[…].hidden = true`.  These hidden destinations
are worker-local output buffers, not parent state.  G5 was
extended to look up the variable's name in `def.attributes` and
treat it as local if the matching attribute has `hidden: true`.

The end-to-end test
`par_warning_fires_for_direct_parent_write_worker` was rewritten
to pass a vector PARAMETER (a true parent ref) instead of relying
on the now-correct local-vector behaviour.  3 unit tests in
`scopes::par_deep_tests` cover the precision rules.  Commits
`6b71d79`, `70b6018`.

### G6 — vector-return par dispatch — **partial scaffolding, destination plumbing pending**

Not in original plan.  Surfaced when retrying
`par_struct_to_vector_t4` after G5/G5.1 cleared the par-safety
side.

Partial fix landed: parser routes `Type::Vector(_, _)` returns
through the ref path (`return_size = -1`); `is_primitive_return`
excludes vectors so they go through the heavy path; the
extra-arg check at `parse_parallel_worker` skips
`def.attributes[…].hidden = true` so it doesn't complain about
the missing hidden destination arg.

Remaining blocker: vector-returning workers like
`out: vector<integer> = []; ...; out` get the local var
promoted to a hidden caller destination arg, but the par
dispatcher passes 0 extras — the worker writes into the
parent's locked store at thread join.  Fix needs the par
dispatcher to allocate per-worker destinations in worker output
stores and pass them as the hidden arg.  Deferred to plan-06
phase 7 (par_fold) where this hidden-arg machinery lives.

`par_struct_to_vector_t4` and `par_struct_to_keyed_collection_t4`
remain `#[ignore]`d.

### Why these aren't in PROBLEMS.md

Plan-06 is the single source of truth for "what par needs to
support after the redesign".  The `#[ignore]`d tests are canaries
that get un-`#[ignore]`d when the relevant phase lands; the plan
file owns the inventory.  Filing per-gap PROBLEMS.md entries would
duplicate the plan and create maintenance churn.

When a gap's fix lands, the same commit:
1. Removes the runtime restriction.
2. Un-`#[ignore]`s the corresponding tests in `threading_chars.rs`.
3. Updates this section to mark the gap closed.

## Hand-off to phase 2

After phase 1 lands, every worker writes to an output store but
the main thread still uses today's `copy_block` + `copy_claims`
collection.  Phase 2 introduces the store-rebase pass that retires
those, removing P1-R3 (`claims` HashSet overhead) and P1-R5 (no
Rust-level proof of non-aliasing).  Surface gaps G1 (struct-enum
returns) close in phase 1; G2 (primitive-input) progresses in
phase 1 and finishes in phase 4.
