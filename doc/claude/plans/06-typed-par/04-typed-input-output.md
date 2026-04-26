<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 4 — Typed input/output surface

**Status: scoping changed by phase-1 G2/G3/G4 work — see "Deviation note" below.**

## Deviation note

The original phase-4 plan assumed that `element_size` /
`return_size` would retire only when the typed surface lands.
Phase-1 implementation work (G2/G2.1/G3/G4) had to reach into
the same area earlier:

- **G2 / G2.1 — primitive-input dispatch**: workers receiving
  primitive inputs need the inline value in slot 0, not a
  DbRef.  Required new `execute_at_raw_primitive_input` and
  per-worker-input-type detection.  Closed before phase 4.
- **G3 — text-input dispatch**: workers receiving `text` args
  need a 16-byte `Str` in slot 0.  Required new
  `execute_at_raw_text_input` + `read_text_at`.  Closed before
  phase 4.
- **G4 — fn-ref return (partial)**: workers returning fn-refs
  (20 bytes) exceed the 8-byte primitive return cap.  New
  `execute_at_raw_to(.., dst, return_size)` writes arbitrary
  bytes from worker stack.  Worker codegen layout reconciliation
  pending — phase 4 finishes G4 by aligning the calling
  convention.
- **3d — runtime DispatchMode derivation** (commit `e8ffd87`):
  the runtime no longer relies on the parser's `0 / -1 / 1..=8`
  sentinels.  It inspects `def.returned` directly:
  `Type::Text` → text mode (size=4), `heap_def_nr Some` → ref
  mode (size from db), `Type::Function` → 20-byte primitive,
  else → primitive (size from def).  The parser still emits
  the historic sentinels for binary compatibility but the
  runtime ignores them as the primary signal.

What remains for phase 4: drop the sentinel encoding from the
parser side entirely (only the runtime backstop remains in
phase 5+), and the typed `parallel_for(input: vector<T>, fn:
fn(T) -> U, threads: integer) -> vector<U>` surface.

## Goal

Replace the integer-positional encoding of today's `parallel_for`
with a fully typed surface where the worker fn's `T → U` signature
drives everything.  Remove the runtime `element_size` / `return_size`
integer args; both are inferred from types via `Data::fn_return_type`
(DESIGN.md D3) **plus** a parser-side compiler special-case (the
same shape `map` uses today — see "Loft-side prerequisites" for
why).

Today:

```loft
fn parallel_for(input: reference, element_size: integer,
                return_size: integer, threads: integer,
                func: integer) -> reference;
```

After phase 4:

```loft
pub fn parallel_for(input: vector<T>,
                    fn: fn(T) -> U,
                    threads: integer) -> vector<U>;
```

Two integer arguments retire (`element_size`, `return_size`); the
parser's compiler special-case (mirroring `map`) extracts `T` from
the input vector and `U` from the worker fn's return type, then
validates the worker fn signature against the input vector's
element type.

Note: the `<T, U>` after `parallel_for` in the declaration is
**not** bounded-generic syntax — it's a type-variable placeholder
recognised by the parser's compiler-special-case path, exactly as
`pub fn map<T, U>(...)` works today.  See "Loft-side
prerequisites" for the verified-against-source explanation.

## What changes user-visibly

For end users: nothing.  Today's `par(...)` and `par_light(...)`
sugar already hide the integer args; phase 4 affects only the
internal `parallel_for` fn that the sugar lowers to.  The
expression-position desugar from phase 7c continues to work.

For internal callers (in `default/01_code.loft`, `lib/`, tests):
the integer-positional `parallel_for(input, elem_size, ret_size,
threads, fn)` is no longer a valid call shape.  Migration:

| Today's call | After phase 4 |
|---|---|
| `parallel_for(input, 8, 8, 4, my_fn)` | `parallel_for(input, my_fn, 4)` |
| `parallel_for_int(input, 8, 8, 4, "my_fn")` | (retired entirely — call site rewritten to use the typed `parallel_for`) |
| `parallel_for_light(input, 8, 8, 4, my_fn)` | (retired entirely — phase 5's auto-light heuristic picks the light path) |

Phase 4 lands the surface change; phase 5's auto-light retires
`parallel_for_light`; phase 7c's desugar wires `par(...)` to the
new typed surface.

## Per-commit landing plan

### 4a — typed `parallel_for` declaration

- Update `default/01_code.loft` to declare the typed shape:
  ```loft
  pub fn parallel_for(input: vector<T>,
                      fn: fn(T) -> U,
                      threads: integer) -> vector<U>;
  ```
- Add `Data::fn_return_type` accessor (per DESIGN.md D3).
- Add a parser-side compiler-special-case `parse_parallel_for` in
  `src/parser/builtins.rs` mirroring `parse_map` in
  `src/parser/collections.rs:1490` — extract `T` from the input
  vector type, extract `U` from the worker fn's return type
  (via the new `Data::fn_return_type`), validate the worker's
  arg type matches `T`, return `vector<U>` as the call's result
  type.  No generic monomorphisation runs — the parser special-case
  is the only mechanism.
- Migrate every internal caller in `default/`, `lib/`, `tests/`:
  drop the `elem_size` and `ret_size` args; the function call now
  has 3 args, not 5.

Acceptance: phase-0 characterisation suite passes; the parser
emits the same `OpParallel(0x00)` opcode regardless of which
surface form (typed vs. integer-positional) was used during the
transition (one parser branch checks arg count and pattern-matches).

### 4b — retire integer-positional encoding

- Delete the integer-positional `parallel_for` declaration in
  `default/01_code.loft`.
- Delete the parser branch that accepts the 5-arg form.
- The parser's diagnostic for 5-arg calls becomes:
  `parallel_for now takes 3 args (input, fn, threads); the integer
  size args were retired in 0.9.0`.
- `parallel_for_int(...)` (string-based dispatch) retires entirely
  — every internal caller has already been migrated to the typed
  form in 4a.

Acceptance: `default/01_code.loft` size drops by ~30 lines;
phase-0 suite still passes.

### 4c — rename `Stitch::ConcatLegacy` → `Stitch::Concat` (drop payload)

- After 4a + 4b, the worker fn's `Type` is the source of truth for
  element / return sizes.  The `Stitch::ConcatLegacy { elem_size,
  ret_size }` payload from phase 3 is redundant — codegen already
  embeds sizes from `Data::fn_return_type` (per phase 3d) and from
  `vector<T>`'s element type.
- Rename the variant `ConcatLegacy` → `Concat` (matches DESIGN.md
  D1b — the **final** shape).  Drop the `{ elem_size, ret_size }`
  payload.
- Opcode payload shrinks by 2 bytes per call (per DESIGN.md D1).
- Update every `Stitch::ConcatLegacy` match arm in `src/parallel.rs`
  and `src/codegen_runtime.rs` to `Stitch::Concat`.

Acceptance: `grep ConcatLegacy src/` returns zero matches after
4c; opcode count stable; payload size measurably smaller (verified
by `LOFT_LOG=static` dump comparison vs. phase-3 baseline).

## Loft-side prerequisites

- **Parser-side compiler special-case (mirroring `map`).**
  Verified by reading `src/parser/collections.rs:1490::parse_map`:
  loft's `pub fn map<T, U>(input: vector<T>, fn: fn(T) -> U) ->
  vector<U>` is **not** monomorphised by a bounded-generics pass —
  it is a compiler special-case that the parser inlines as a
  for-comprehension.  `parse_map` extracts the input vector's
  element type and infers the output element type from the worker
  fn's return type.  No generic substitution machinery executes.

  Phase 4 follows the same pattern: a new
  `parse_parallel_for` compiler special-case in
  `src/parser/builtins.rs` extracts `T` from the input vector
  and `U` from the worker fn (via `Data::fn_return_type`), then
  emits the typed `OpParallel` opcode with the resolved types.
  Cost: ~120 LOC mirroring `parse_map`.

- **`Data::fn_return_type` accessor.**  Per DESIGN.md D3.  Verified
  not to exist as of 2026-04-25; phase 4a adds it.

- **Type-checker call-arity diagnostic.**  When the parser sees a
  5-arg call to `parallel_for`, emit the migration message.

**What phase 4 does NOT need.**  Phase 4 does not require
bounded-generic substitution, monomorphisation across call sites,
or any new generic-resolution infrastructure.  Treating
`parallel_for` as a parser-side special-case (option 1 in DESIGN.md
D3's "Why not 'reuse map's machinery'") is the explicit chosen
default; the alternative (landing real bounded generics) is
out-of-scope for plan-06.

## Test fixtures

| Fixture | Asserts |
|---|---|
| `tests/issues.rs::par_phase4_typed_args` | `parallel_for(xs, foo, 4)` parses and runs; the type checker rejects `parallel_for(xs, foo)` (missing threads) and `parallel_for(xs, foo, "4")` (wrong threads type) |
| `tests/issues.rs::par_phase4_generic_substitution` | `parallel_for(vector<i32>, fn(i32) -> f64, 4) -> vector<f64>` works; the result vector's element type is correctly `f64` |
| `tests/issues.rs::par_phase4_5_arg_diagnostic` | A test program calling `parallel_for(xs, 8, 8, 4, foo)` receives the migration diagnostic; the existing 3-arg call still works |
| `tests/issues.rs::par_phase4_no_runtime_size_args` | `LOFT_LOG=static` dump shows the opcode no longer carries `elem_size` / `ret_size` payload after 4c |

## Acceptance criteria

- Phase-0 characterisation suite passes byte-for-byte.
- All internal callers (`default/`, `lib/`, `tests/`) migrated to
  the 3-arg form.
- Bench-1 / 2 / 3 within ±5 % of phase 3 baseline (no regression;
  phase 4 is mostly a parser / type-checker change).
- `default/01_code.loft` shrinks by ~30 lines after 4b retires the
  legacy declarations.
- Opcode payload size drops by 4 bytes per call after 4c.

## Risks

| Risk | Mitigation |
|---|---|
| Bounded-generic substitution does not exist as plan-06 originally assumed | Verified against `src/parser/collections.rs:1490::parse_map` (2026-04-25): `map` is a parser-side compiler special-case, not generic monomorphisation.  Phase 4 follows the same pattern explicitly — no new generics infrastructure required.  See "Loft-side prerequisites". |
| External callers using `parallel_for(input, elem_size, return_size, threads, fn)` directly | The 5-arg form was always documented as "compiler-checked internal"; users who hand-typed it get the migration diagnostic |
| `Stitch::ConcatLegacy` → `Concat` rename in 4c breaks an internal caller | 4c is purely a Rust-source rename + payload removal; `cargo build` would fail at every legacy callsite if any existed in handwritten code (no callsite is generated by codegen-emitted-Rust on the native path because the `Stitch` enum is constructed only inside `src/parallel.rs` and `src/codegen_runtime.rs`).  `make ci` catches the rest. |
| The `parallel_for_int(func: text, ...)` string-based dispatch was used for runtime fn lookup | Today's only caller is the legacy par interface; verify by grep, then retire entirely.  No replacement — the typed form covers every use case |

## Phase 4d — flat-vector-only input iteration fix

**Status: designed 2026-04-26 after implementation surfaced 5 ignored
canaries with a shared root cause; not yet shipped.**

The par dispatcher today assumes (a) input collection storage is a flat
vector AND (b) inline elements are ≤ 8 bytes.  That single pair of
assumptions blocks 5 of the 13 currently-ignored par canaries:

- `par_sorted_input_t4`, `par_hash_input_t4`, `par_index_input_t4` —
  keyed-collection storage isn't a flat vector; the parser at
  `src/parser/collections.rs:1123` rejects these inputs with
  *"par(...) requires a vector<T> input"*.
- `par_tuple_input_int_int`, `par_tuple_input_int_text`,
  `par_vec_of_fns_input_t4` — flat-vector elements but >8 bytes (16
  for `(integer, integer)`, 20 for fn-ref).
  `primitive_first_arg_slot_size` in `src/native.rs:31-43` returns 0
  for these; the dispatcher falls through to the DbRef-passing path;
  the worker hangs trying to dereference a row pointer instead of
  reading the inline bytes.

Both failures share a root cause but split into two independently-
shippable sub-phases.

### 4d.A — Large-inline element support

Closes `par_tuple_input_int_int`, `par_tuple_input_int_text`,
`par_vec_of_fns_input_t4`.

Replace the sentinel-encoded `primitive_input_size: u32` channel with
a typed enum, following the precedent set by `DispatchMode` at
`src/native.rs:51`:

```rust
enum InputKind {
    Ref,                    // worker takes a DbRef in slot 0 (struct-by-ref)
    Text,                   // worker takes a 16-byte Str in slot 0
    Primitive { size: u8 }, // worker takes `size` bytes inline (1..=64)
}
```

The `u32::MAX` text sentinel from G3 retires; `0`-as-DbRef-fallback
becomes `InputKind::Ref`; `prim_in: u32` becomes `input_kind: InputKind`
across `run_parallel_light` / `run_parallel_direct`.

Five concrete code changes:

1. **`primitive_first_arg_slot_size`** at `src/native.rs:31-43` becomes
   `input_kind_for_first_arg(def) -> InputKind`.  Returns
   `Primitive { size }` for any inline-typed first arg, computing
   `size` via `crate::data::element_size` for tuple, struct, fn-ref
   (=20).  **Cap at 64 bytes** — anything larger falls back to
   `InputKind::Ref` so the worker stack stays bounded (one cache line
   plus slack).

2. **`read_primitive_at`** at `src/parallel.rs:957-972` generalises
   from a 1/4/8 match to a `[u8; 64]` buffer + length, returning the
   bytes the dispatcher will memcpy into the worker frame.  Stack-
   allocated array (no per-row `Vec` allocation) since the cap is
   bounded.

3. **`State::execute_at_raw_primitive_input`** at
   `src/state/mod.rs:1880-1944` accepts the bytes + size and pushes
   them as a single chunk into slot 0.  `args_size` becomes
   `size + 8 * extras.len()` (it currently is just the slot width).

4. **The three dispatch sites** in `run_parallel_direct` (lines
   470-545) and `run_parallel_light` (lines 800-930) become
   `match input_kind` against the typed enum instead of cascading
   `if prim_in == u32::MAX { ... } else if prim_in > 0 { ... }
   else { ... }`.

5. **Add `debug_assert!(var_size(first_arg) == size)`** before
   pushing.  The worker frame's variable table assumes `size`
   matches what the tuple/fn-ref field-access opcodes expect — this
   catches any future drift between `data::element_size` and
   `variables::size`.

4d.A is independent of 4d.B.  Tuples and fn-refs are already in
flat-vector storage, so `vector::get_vector(input, stride, idx)`
returns the right `row_ref` — the bug is solely on the worker-call
side.

### 4d.B — Keyed-collection materialisation

Closes `par_sorted_input_t4`, `par_hash_input_t4`,
`par_index_input_t4`.

Replace the rejection at `src/parser/collections.rs:1123` with a
**parser-side desugar**.  When `in_type` is `Type::Sorted / Hash /
Index / Spacial`, emit IR equivalent to:

```loft
let __par_mat: vector<reference<T>> = [];
for x in input { __par_mat += &x; }
<par over __par_mat>
```

then rebind `vec_expr = Var(__par_mat)` and `in_type =
Vector(Reference(T))` and let the rest of `parse_parallel_for_loop`
run unchanged.

Five details:

1. **Use the existing `OpIterate` / `OpStep` machinery** at
   `src/state/io.rs:700` and `:861`.  These ops are already emitted
   by the non-par `for x in sorted_items { ... }` codegen at
   `src/parser/collections.rs:174-222` — re-use that template via
   `fill_iter`.  Both ops are read-only to the collection (safe under
   D2.0).

2. **Materialise to `vector<reference<T>>`** (12-byte DbRef per
   element), not `vector<T>`.  Workers receive the same 12-byte DbRef
   the `par_vec_of_refs_input_t4` (already-closed) canary uses, so
   the existing `InputKind::Ref` dispatch handles it without further
   work.  Inline copies would force re-doing 4d.A's wide-slot work
   AND deep-copy of text/owned-field semantics inside the
   materialisation step.

3. **Free `__par_mat` via the existing scope-exit machinery** —
   `create_unique` + `defined`, mirroring how `par_results` is freed
   at `src/parser/collections.rs:1333`.  No explicit `OpFreeRef`.
   The vector lives in parent stores and is read-only to workers
   (D2.0 holds); freed when the for-loop scope exits.

4. **Uniform path for all three keyed types.**  Hash already requires
   upfront materialisation (no iterator slicing possible — see
   `n_hash_sorted` at `src/native.rs:1325` + `build_hash_sorted_vec`
   at `src/database/allocation.rs:317`).  Sorted and Index *could* be
   iterator-sliced (worker N walks `[N*K..(N+1)*K]`), but that needs
   a parallel-aware `OpStep` variant per backing structure — three
   new ops, three test matrices.  Materialisation is one IR shape
   covering all three; defer the smarter sliced path until profiling
   demands it.

5. **Cost contract:** par over a keyed collection is now O(N)
   materialisation + N × 12-byte temporary vector + the par work
   itself.  Documented as a known cost; users who need to avoid it
   construct a `vector<reference<T>>` explicitly and pass that to
   par.

### Ship order

- **4d.A first**, because it's contained to the dispatcher and closes
  3 canaries with a self-contained refactor.
- **4d.B second**, after 4d.A's `InputKind` enum lands, because
  4d.B's materialised `vector<reference<T>>` lands on the existing
  `InputKind::Ref` arm with no further dispatch work.

### Acceptance

**4d.A:**
- `par_tuple_input_int_int`, `par_tuple_input_int_text`,
  `par_vec_of_fns_input_t4` un-`#[ignore]`d and pass.
- `par_struct_to_struct_t4` (existing positive test) still passes —
  `InputKind::Ref` arm wasn't broken by the refactor.
- `par_text_input_t4` still passes — text path retired its
  `u32::MAX` sentinel cleanly.
- Unit test: `input_kind_for_first_arg` returns
  `InputKind::Primitive { size: 16 }` for a tuple-(int,int) first
  arg, `Primitive { size: 20 }` for fn-ref, `Ref` for a struct-Ref
  first arg, `Ref` for any inline first arg whose
  `element_size > 64`.

**4d.B:**
- `par_sorted_input_t4`, `par_hash_input_t4`, `par_index_input_t4`
  un-`#[ignore]`d and pass.
- The materialisation walk is observable in the dump: `OpIterate` +
  `OpStep` opcodes appear in the par-call's pre-loop section.
- `par_struct_to_struct_t4` (the canonical
  vector\<Struct\> → primitive case) still passes — the desugar only
  fires for keyed inputs.
- Cross-cutting test: `par_sorted_input_with_extras` verifies the
  desugar doesn't mangle the par()'s extra-args list.

**End-to-end:**
- `cargo test --release --no-fail-fast --test threading_chars` —
  ignored count drops from 13 → 7 (6 canaries closed).
- `cargo test --release --no-fail-fast --test issues
  --test parse_errors --test threading` — no regressions.

## Out of scope

- Auto-light heuristic (phase 5).
- Cleanup / doc (phase 6).
- Fused for-loop construction (phase 7).
- Heterogeneous worker results.
- Iterator slicing for keyed collections smarter than full
  materialisation — deferred until profiling shows it matters.

## Hand-off to phase 5

After phase 4:
- The typed surface is live (`parallel_for(input, fn, threads)`).
- `parallel_for_int` retired.
- `parallel_for_light` still exists as a separate user-facing
  declaration (will be retired in phase 7c after phase 5's
  auto-light heuristic picks the light path automatically).

Phase 5 introduces the heuristic that decides "this worker is
light-safe" without the user opting in.  The user-visible
`parallel_for_light` becomes redundant; phase 7c removes it from
the surface.
