<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-06 — Real-value Development Arc

## Why this exists

PRIORITY.md (the "spine") and README.md (the "phases") have drifted
apart.  Phases were authored as work surfaced; the spine reordered
them; step 8 has since absorbed phases 5b, 6, and most of 10.  The
result is a planning surface where:

- Step status looks tidier than reality (e.g. headline "−250 LOC"
  while the working tree is already at −583).
- Workarounds passing today's tests get marked DONE without flagging
  the latent risk (e.g. 8d.3's per-thread 16-slot cap).
- Off-spine items (4d.A.2, 4d.C, 4e, 5c) are technically open but
  effectively orphaned.
- Real value — fewer dispatch arms, fewer marshalling shapes, real
  browser parallelism, the closing of 8 ignored canaries — is
  spread across multiple bookkeeping layers.

This document is **the single source of truth** for what's left, in
the order it should ship.  Each arc step is one PR-sized unit with
hard scope locks, a named acceptance test, links to the existing
designs and known problem records (PROBLEMS.md P-IDs), and concrete
file-site changes.

## Ground rules (non-negotiable)

1. **One arc step per PR.**  No "Aₙ.1, Aₙ.2, Aₙ.3" sub-step expansion
   inside one branch.  If a step turns out larger than estimated,
   close what's done, file the rest as the next arc step.
2. **No uncommitted multi-day stash.**  At session end, either the
   work commits or it reverts.  If a step needs more than one
   session, close the first session at a green-tests boundary.
3. **Every step closes a canary, deletes code, or unlocks the next
   step.**  Steps that do none of these are paperwork — fold them
   into a step that does.
4. **Bench gate every step.**  `bench/11_par` ±5 % on the loft-interp
   and loft-native columns vs. THREADING.md "Plan-06 phase 0
   baseline" (loft-interp 44 ms, loft-native 12 ms).  No silent
   regressions.
5. **No partial-DONE in this doc.**  A step is OPEN, IN-FLIGHT, or
   DONE.  If it's DONE-with-caveat, the caveat becomes the next step.
6. **Goldens regen lives in the step that shifted them.**  Not a
   trailing chore.

## Acceptance state — what "plan-06 done" means

Three concrete criteria, all measurable:

1. **Single dispatcher family.**  After A8: one generic
   `run_parallel_queue<R>` + `run_parallel_discard` +
   `run_parallel_reduce`.  `grep -c "fn run_parallel_" src/parallel.rs`
   returns ≤ 3.
2. **Single user surface.**  `par_light` removed from
   `default/01_code.loft`; auto-light decided at compile time from
   the worker's effect annotation (DESIGN.md D8 / phase 5e).
3. **Zero ignored par canaries.**  `grep -c "^#\[ignore"
   tests/threading_chars.rs` returns 0.  Today: 8.

When all three hit, plan-06 closes.  Until all three hit, it doesn't.

## Inventory — what's actually in flight

### Uncommitted working tree (as of 2026-04-30)

```
src/native.rs    -345 LOC  (parallel_execute_and_collect, DispatchMode, helpers gated)
src/parallel.rs  -238 LOC  (run_parallel_direct, run_parallel_ref deleted)
```

Heavy `parallel_execute_and_collect` retired.  Light path
(`parallel_light_execute_and_collect`) still alive.  2 goldens
(`25_runtime_panic_builtin`, `28_runtime_unwrap_none`) need regen.
`n_parallel_for` now delegates to `n_parallel_for_light`.

### 4 ignored canaries — all blocked on T1.8a external prerequisite (2026-05-01: A6.b closed; all 4 unblocked A6 canaries done in single session)

| Test | Line | Blocker | Plan link |
|---|---|---|---|
| `par_tuple_return_int_int` | 749 | tuple return; var_size=16 > 8 byte cap | T1.8a + phase 9c ([09-tuple-support.md](09-tuple-support.md)) |
| `par_tuple_return_int_text` | 767 | tuple return | T1.8a + phase 9c |
| `par_tuple_return_struct_text` | 785 | tuple return | T1.8a + phase 9c |
| `par_tuple_destructure` | 807 | fused destructure binding | phase 9d |

### Known fragilities (not canaries — won't be caught by tests)

- ~~**8d.3 per-thread reserved-slot cap = 16.**  Workers needing > 16
  fresh stores per element panic.  Latent until a workload exceeds
  it.  No test covers this today.~~  **CLOSED A2 2026-04-30** (commit
  `217b3ac`) — fixed-cap retired; shared-atomic dispenser landed.
- **`worker_slot_offset != 0` edge case.**  If parent's allocations
  table is empty when reservation runs, `offsets[0] == 0` and the
  legacy push-at-end fallback kicks in.  Harmless in practice,
  surprising in pathological setups.
- **`mem::swap` after worker join trusts that every worker
  allocation went through the offset-aware path.**  No assertion
  enforces this.  A regression that adds a path bypassing
  `database_named` would silently leak data.

### Cross-references to existing PROBLEMS.md entries

| P-ID | Title | Relevant arc step |
|---|---|---|
| **P196** | ~~Tuple-of-fn-ref native codegen `(u32, DbRef).0 as i32`~~ | **CLOSED 2026-04-30** — fixed in `output_call_template` (project `.0` from fn-ref tuple → widen to i64 for the OpSetInt4 template), no longer requires 4d.C closure-storage redesign; A6.c can now proceed independently |
| **P198** | ~~`tests/scripts/95-alias-copy.loft` leaks Database 3 — regression on `roadmap-lsp-eclipse`~~ | **CLOSED 2026-05-01** in commit `30b01ce` — `scan_set` + native deep-copy emit both now unwrap `Value::Span` before pattern matching; A1 gate passes |
| **P199** | Native codegen E0499 double `&mut stores` borrow on `n_assert(..., n_add_pair(...), ...)` | A7 (gated on T1.8a tuple-return convention) |
| **P200** | Native codegen E0308 width mismatch on `f += <integer>` against binary file | Out of scope for ARC; tracked separately |
| **P201** | `tests/html_wasm.rs` Mutex-poison cascade | Out of scope for ARC; test infra |

---

## The arc — 11 steps in shippable order

Each step has: scope-locked bullet list, **design** section, **acceptance
test** (named), **risks** specific to that step, and **out of scope**.

Effort estimates: S = small (≤ 0.5 session), M = medium (~1 session),
L = large (1.5–2 sessions), XL = 2+ sessions.

---

### A1 — Land the uncommitted step-8e cuts cleanly

**Status:** DONE 2026-04-30 (commits `b9ad7af` + `7153390` + the
ARC/THREADING update commit that follows this section).
**Effort:** S (~0.25 session) — actual: ~0.5 session.
**Acceptance test:** `make ci` clean; `find_problems.sh` shows zero
unexpected failures; bench 11 ±5 %.

**Closure notes:**

- Code deletes (commit `b9ad7af`): `parallel_execute_and_collect`
  (~170 LOC), `run_parallel_direct` (~190 LOC), `run_parallel_ref`
  (~40 LOC); `n_parallel_for` collapsed to delegate to
  `n_parallel_for_light`; 4 cfg-gated symbols verified clean under
  `cargo build --no-default-features`.
- Goldens regen (commit `7153390`):
  `25_runtime_panic_builtin.expect` and `28_runtime_unwrap_none.expect`
  re-blessed via `UPDATE_GOLDEN=1`; diffs are pure pc shifts
  (`303→285`, `279→261`).
- P198 alias-copy-leak (commit `7153390`): test still fails at the
  new pc=4842 (was 4788 pre-A1).  Confirmed not A1-caused — the
  retired code paths are unreachable from non-par scripts.  Note
  appended to PROBLEMS.md P198; remains a candidate for an
  A2-prerequisite spot fix in `scopes.rs::scan_set` Span/ParFor
  passthrough.
- Bench-11 ±5%: passed against host-relative `main` baseline
  (interp +3 % median: 98ms main → 101ms branch).  The 44ms
  THREADING.md absolute baseline was a different host; native
  column blocked by P199 (re-opens under A7).  Recorded in
  THREADING.md § "ARC.md A1 host-relative check".
- Clippy debt swept (commit `afc9f70`): branch had accumulated 40
  pre-existing clippy errors from spine commits 87c2ce8 onwards
  (Discard/Queue/Purity/ParFor work).  All cleared via clippy
  autofix + targeted `#[allow]` on legitimate too-many-args /
  similar-names / single-char patterns, and re-flowed doc list
  continuation indents.  No semantic changes.
- `cargo nextest run --profile ci`: passes everywhere except
  `loft::html_wasm::moros_editor_html_smoke` — confirmed
  pre-existing P199 manifestation (`OpCopyRecord(stores,
  n_build_chunk(stores, …), …)` E0499).  Same shape closes under
  A7's hoist-inner-`&mut stores` fix.  Documented at PROBLEMS.md
  P199 § "A1 status".

#### Why first

The working tree has been carrying −583 LOC across session
boundaries.  Every additional change risks tangling.  Get back to a
committed baseline before subsequent steps branch from it.

#### Design

Three concerns in one commit (atomic — no split):

1. **Code deletes already in working tree.**
   - `parallel_execute_and_collect` (~170 LOC) removed from
     `src/native.rs`.
   - `run_parallel_direct` (~190 LOC) and `run_parallel_ref` (~40
     LOC) removed from `src/parallel.rs`.
   - `n_parallel_for` body collapsed to delegate
     `n_parallel_for_light(stores, stack)`.
   - `DispatchMode` enum and tuple-input helpers gated
     `#[cfg(feature = "threading")]` (already done — verify).

2. **Goldens regen.**  Two test fixtures shifted by native.rs line
   movement:
   - `tests/error_messages/cases/25_runtime_panic_builtin.loft`
   - `tests/error_messages/cases/28_runtime_unwrap_none.loft`

   Run `cargo test --release --test error_messages baselines_are_locked_in
   -- --ignored` (or whichever sub-flag generates new baselines per
   `tests/error_messages.rs`'s `BLESS_BASELINES` env var).  Diff the
   updated `.expected` files; verify only line-number / pc shifts.

3. **P198 investigation gate.**  Before committing, run
   `cargo test --release p146_script_95_alias_copy_leak`.  If it
   passes today on `roadmap-lsp-eclipse`, ignore (P198 may already
   be moot).  If it fails, file a one-paragraph note on PROBLEMS.md
   P198 with the current symptom — but **do not block A1 on it**.
   The leak existed before A1's diff and is independent.

#### Risks

| Risk | Mitigation |
|---|---|
| Goldens regen reveals additional unrelated failures | Investigate; if related to par work, document in CHANGELOG.  If unrelated, file as new PROBLEMS.md entry and proceed. |
| `n_parallel_for` → `n_parallel_for_light` delegation breaks `par_light`-explicit user-surface tests | Audit `default/01_code.loft` to confirm both surfaces still resolve to native fns; only the runtime path is now shared. |
| The 4 `cfg`-gated symbols (`InputKind`, `INPUT_PRIMITIVE_MAX_BYTES`, `tuple_first_arg_types`, `input_kind_for_first_arg`) drift between `--features threading` and `--no-default-features` | `cargo build --no-default-features` already verified clean (this session); add to `make ci` if not present. |

#### Out of scope

Narrow-prim Queue extension (A3).  fn-ref work (A6).  Doc rewrites (A11).

---

### A2 — Stress-test the per-thread slot cap; resolve the fragility

**Status:** DONE 2026-04-30 (this session — see Status table for commits)
**Effort:** M (~1 session) — actual: ~1 session
**Acceptance test:** new test
`tests/threading.rs::par_queue_ref_unbounded_allocations_per_element`
panics today, passes after fix; `assert!` in `database_named`
catches a synthetic offset-bypass.

**Closure notes:**

- 8d.3's fixed `SLOTS_PER_THREAD = 16` per-thread reservation
  retired in favour of a shared `Arc<AtomicU16>` dispenser.
- Three `Stores` fields removed (`worker_slot_offset`,
  `worker_slot_limit`, `worker_slot_local_count`); two added
  (`worker_slot_dispenser: Option<Arc<AtomicU16>>`,
  `worker_allocated_indices: Vec<u16>`).
- `reserve_worker_slots` / `release_worker_slots` removed; new
  `make_worker_slot_dispenser()` returns the shared atomic
  initialised at `parent.allocations.len() + 1` (the `+1` skips
  each worker's stack-store index).
- `database_named`'s slot-pick now: dispenser path first (when
  attached + `disable_slot_reuse=true`), then push-at-end legacy,
  then `find_free_slot`.  Maintains `max == highest_allocated + 1`
  invariant via `slot >= self.max` (was `slot == self.max`, which
  missed the dispenser's index-skip pattern).
- A2.3 invariant `assert!` (always-on, not gated by
  `debug_assertions` because the loft library compiles with
  `debug-assertions = false` in the test profile per
  `[profile.dev.package.loft]`) catches the bypass case where a
  worker has a dispenser but `disable_slot_reuse` is cleared.
- Bench-11 ±5%: passes (~101ms median, host-relative; same as A1).
- All 37 existing `tests/threading.rs` tests + 31
  `tests/threading_chars.rs` tests stay green.

#### Why second

8d.3 shipped with a known panic risk.  We need to either prove
unreachability or remove it before extending Queue further.  Doing
this before A3 keeps the allocator design honest as new return
shapes get added.

#### Design

Three sub-tasks in one PR.

##### A2.1 — Stress test (write the failing test first)

```rust
// tests/threading.rs
#[test]
fn par_queue_ref_unbounded_allocations_per_element() {
    // Each worker invocation allocates ~30 fresh stores
    // (more than SLOTS_PER_THREAD=16).
    let src = r#"
        struct Big { fields: vector<integer> }
        fn balloon(x: const integer) -> Big {
            b: Big = Big { fields: [] };
            for i in 0..30 { b.fields += [i]; }
            b
        }
        fn run() -> integer {
            xs: vector<integer> = [1, 2, 3, 4];
            total = 0;
            for x in xs par(b = balloon(x), 4) { total += b.fields[0]; }
            total
        }
    "#;
    assert_eq!(run_loft_int(src, "run"), 4 + 6); // never reached today
}
```

This test panics in current 8d.3 because `worker_slot_local_count`
exceeds `SLOTS_PER_THREAD = 16`.

##### A2.2 — Pick the structural fix

Three options carried from the original investigation
(`/home/ubuntu/.claude/plans/investigate-8d-3-on-plans-jiggly-koala.md`):

| Option | Cost | Cap risk | When to pick |
|---|---|---|---|
| **(a) Dynamic per-thread `Vec<u16>`** of allocated indices, replacing the fixed-size range | 1 session | None (grows) | If existing code paths can be refit cleanly |
| **(b) Atomic `fetch_add` counter** in `parallel_workers` for per-worker offset reservation, growing on demand | 1 session | None | If contention is acceptable; cleanest semantics |
| **(c) Post-process rebase** — drop reservation entirely; each worker pushes at end via `disable_slot_reuse=true`; parent rebases via thread-id-prefixed map | 1.5 sessions | None | Original PRIORITY.md 8d.3 design; rejected as complex but is structurally correct |

**Recommendation: (a)**.  Smallest delta from current code.  The
per-thread slot range becomes a per-thread `Vec<u16>` of indices;
`reserve_worker_slots` returns `(start, capacity_hint)` instead of
a fixed range; `database_named` pushes a fresh slot via
`free_bits.push(false); allocations.push(Store::new(100))` (in the
parent vec, under the per-thread mutex) and records the index in
the worker's `Vec<u16>`.  The post-worker swap iterates the
per-thread `Vec<u16>` instead of `[off, off+limit)`.

If (a) turns out to need more than 1 session — for instance because
`free_bits` mutation under contention forces locking — fall back to
(b).  Don't bundle.

##### A2.3 — Invariant assertion

Add a debug-only assertion in `database_named` that catches the
bypass case:

```rust
#[cfg(debug_assertions)]
fn assert_offset_path(&self, where: &str) {
    if self.worker_slot_limit > 0 {
        // We're a worker; every named alloc must go through the
        // offset-aware path.  If we reach here without honouring it,
        // panic loudly.
        debug_assert!(
            self.went_through_offset_path,
            "{}: worker bypassed offset-aware allocation",
            where,
        );
    }
}
```

Add a test that synthesises the bypass (call `database` directly
without going through `database_named`) and confirms the assertion
fires.

#### Risks

| Risk | Mitigation |
|---|---|
| Option (a) hits a contention point that turns the simple `Vec<u16>` into a `Mutex<Vec<u16>>` and degrades parallelism | Bench 11 gates the merge.  If perf regresses, fall back to (b). |
| The stress test passes today by accident (the workload doesn't actually exceed 16) | Increase the allocation count in the worker until it does panic; that's the real lower bound. |
| The fix exposes an unrelated DbRef-rebase bug that was masked by the cap | Triage; if the new bug existed before A2 it's a separate finding to file, not an A2 blocker. |

#### Out of scope

Narrow-prim Queue (A3).  Don't bundle.

---

### A3 — Extend Queue dispatch for narrow-primitive returns

**Status:** DONE — narrow-Integer 2026-05-03; Boolean/Character/Enum-no-payload 2026-05-03 (A3.5); Single/Float 2026-05-07 (A3.6).
**Effort:** L (~1.5 sessions) — narrow-Integer landed in <1 session.

**Narrow-Integer subset closure:**

- A3.1 (par_narrow_buffer_stack field) — DONE.
- A3.2 (3 narrow Queue native fns in src/native.rs) — DONE; codegen-side native fns (`n_parallel_queue_narrow_native` / `n_parallel_buf_get_narrow_native` / `n_parallel_buf_drop_narrow_native`) added to `src/codegen_runtime.rs` 2026-05-03.
- A3.3 (parser gate) — DONE for narrow Integer (forced_size 1/2/4).  `route_narrow_int_queue` flag added to both early and main dispatch sites in `src/parser/collections.rs::build_parallel_for_ir`; narrow-int returns now route through `n_parallel_queue_narrow` / `_buf_get_narrow` / `_buf_drop_narrow` instead of the legacy materialised-vector path.  Body's `b` accessor calls `parallel_buf_get_narrow(idx, return_size, signed)` (signed passed as integer 0/1).
- A3.4 (codegen extras) — DONE; extras-push and extras-subtract entries already covered `n_parallel_queue_narrow`.
- Codegen `ParallelQueueEmitter` extended to detect narrow-Integer returns and emit `n_parallel_queue_narrow_native` instead of the wide variant.  `n_parallel_buf_get_narrow` and `n_parallel_buf_drop_narrow` registered with `ParallelBufRenameEmitter`.
- Bug fix discovered during implementation: `n_parallel_queue_narrow` (interpreter) was calling `run_parallel_queue` with `return_size=4`, but `execute_at_raw` reads only the low 4 bytes via `get_stack::<u32>` for return_size=4 — this loses the actual i32 value because workers push 8 bytes (i64-promoted) but only the low 4 are read into one slot, leaving the high 4 to leak into adjacent state.  Fix: always pass `return_size=8` to `run_parallel_queue` from inside the narrow runtime, then truncate to stride bytes during packing.  The actual narrow value lives in the low `stride` bytes of the i64 (zero/sign-extended), so the truncation is correct.
- `parallel_buf_get_narrow` declared signature changed from `signed: boolean` to `signed: integer` — the runtime reads i64 from the stack (8 bytes), so a boolean (1 byte) push left 7 bytes of garbage on the stack, corrupting subsequent pops.

**Still PENDING (split by difficulty after a 2026-05-03 re-survey):**

##### A3.5 — Boolean / Character / Enum-no-payload — DONE 2026-05-03

Landed via a unified `narrow_route_for(ret_type) -> Option<NarrowRoute>`
helper at the top of `src/parser/collections.rs`.  Returns the
per-row stride, sign-extension flag, and a `NarrowWrap` enum
describing how to wrap the i64 buf_get result back to the worker's
declared type:

- `NarrowWrap::None` — narrow Integer (the body's `r as integer`
  accepts the i64 natively).
- `NarrowWrap::OpCall("OpConvCharacterFromInt")` — Character.
- `NarrowWrap::OpCall("OpCastEnumFromInt")` — Enum-no-payload.
- `NarrowWrap::NeZero` — Boolean.  `OpNeInt(buf_get_call, 0)`
  rather than `OpConvBoolFromInt`, because the existing
  `OpConvBoolFromInt` has *null-check* semantics (`v != i64::MIN`),
  not value semantics — buf_get always returns 0/1, never i64::MIN,
  so the conv would yield true for both.

The `NarrowRoute` descriptor unifies the four shapes through a
single `route_narrow_queue` gate flag (replacing the previous
`route_narrow_int_queue` which only handled Integer narrow).  Both
the early and main dispatch sites in `build_parallel_for_ir` use
the same descriptor.

**Native-side closure fix**: the codegen emitter's existing
`ClosureShape::Scalar` arm emits `worker(...) as i64`, which is
INVALID Rust for `bool` (Rust rejects `bool as i64`).  Special-cased
Boolean to emit `worker(...) as u8 as i64`.  Character maps to
Rust `i32` (per `rust_type` table) and Enum-no-payload to `u8`,
both of which support `as i64` natively — no extra special case.

**`n_parallel_queue_narrow` runtime fix**: the original implementation
passed `return_size=8` to `run_parallel_queue` (working around the
i32-truncation bug from A3).  But that breaks Boolean (worker pushes
1 byte, runtime reads 8 → "No elements left on the stack 5 < 8")
and similar shape-natural narrow returns.  Fix: derive
`worker_return_size` from the worker's declared return type via
`var_size(&ret_type, &Context::Argument)` — returns 8 for Integer
(i64-promoted), 1 for Boolean / Enum-no-payload, 4 for Character.
The buffer pack still truncates to the declared `stride` (1/2/4)
which is correct for both narrow Integer (low bytes) and the
shape-natural shapes (stride == worker_return_size, no-op truncation).

**Acceptance:** `par_struct_to_bool_t4`, `par_struct_to_enum_t4`,
and `par_struct_to_character_t4` (if a canary is added later) all
pass via the narrow path.  Existing `par_struct_to_int_t1`,
`par_struct_to_int_t4`, `par_struct_to_byte_t4`, `par_struct_to_i32_t4`
unchanged.  35/35 `threading_chars` tests, 43/43 `threading`,
540/540 `issues`, 18/18 `codegen_emitter` (baseline refreshed for
the one diverging file `22-threading.rs` where Boolean-return par
now routes through narrow Queue).

**Bug-hunt yield:** 2 latent bugs surfaced (the `bool as i64` Rust
rejection in the native closure emit + the `OpConvBoolFromInt`
null-check semantics not matching value semantics).  Both would
have hit anyone trying to add Boolean queue routing without the
test coverage A3.5 added.

##### A3.6 — Single / Float — DONE 2026-05-07

**Closed via simpler design than the original (no new IR Ops).**
The original plan proposed two new bit-cast IR Ops
(`OpBitcastSingleFromInt`, `OpBitcastFloatFromInt`) wrapping the
existing `parallel_buf_get_narrow` / `parallel_buf_get`
i64-returning readers.  That works but adds Ops purely to
compensate for the buf_get returning the wrong type.

**Simpler observation:** `Store::set_single` / `set_float` already
write f32/f64 as raw bytes (typed-pointer memcpy at a slot — see
`src/store.rs:1396-1421`).  When a worker returning `single`
populates its return slot, the slot bytes ARE the f32 bit pattern.
`execute_at_raw` reads those bytes as u64; the parallel buffer
stores them via `to_le_bytes()` truncation (stride 4 for Single,
8 for Float).  The bytes are correct — only the *reader* side
needed a typed accessor.

**Implementation (one commit):**

- `src/native.rs`: added `n_parallel_buf_get_single` (reads stride-4
  bytes from `par_narrow_buffer_stack` via `f32::from_bits`) and
  `n_parallel_buf_get_float` (reads u64 from `par_buffer_stack` via
  `f64::from_bits`).  Registered in the `FUNCTIONS` table.
- `default/01_code.loft`: added stdlib decls
  `parallel_buf_get_single(idx) -> single` and
  `parallel_buf_get_float(idx) -> float`.
- `src/codegen_runtime.rs`: added `_native` siblings
  (`n_parallel_buf_get_single_native`, `n_parallel_buf_get_float_native`)
  and registered both in `CODEGEN_RUNTIME_FNS`.
- `src/generation/ops/mod.rs`: registered both new natives with the
  pass-through `ParallelBufRenameEmitter`.
- `src/generation/ops/parallel.rs`: extended `is_narrow_int_return`
  to include `Type::Single` so the native emitter routes Single
  through `n_parallel_queue_narrow_native` (stride 4 packing).
- `src/parser/collections.rs`:
  - Added `NarrowWrap::TypedBufGet(&'static str)` variant.
  - `narrow_route_for(Type::Single)` returns
    `NarrowRoute { width: 4, signed: false, wrap: TypedBufGet("n_parallel_buf_get_single") }`.
  - `route_int_queue` gate (both `early_ret_size_8` and main
    `ret_size_8`) extended to fire for `Type::Float`.
  - Body's `get_call` constructor: handles `TypedBufGet` (uses the
    named typed buf_get fn directly, no wrap) and routes Float
    through `n_parallel_buf_get_float` instead of the generic wide
    `n_parallel_buf_get`.

**Bug surfaced + fixed during implementation:** `n_parallel_buf_get_single`
initially used `stores.put(stack, f64::from(val))` (8-byte write),
based on a stale comment that "single slots are 8 bytes wide
post-2c".  In fact `variables::size(Type::Single, Argument)` is 4
(see `src/variables/mod.rs:1319`).  The 8-byte write overflowed the
slot by 4 bytes and SIGSEGV'd at the smashed adjacent stack slot.
Fixed: write as f32.

**Acceptance:** `tests/scripts/22-threading.loft` extended with
exact-sum tests (multi-thread `score_as_single` and `score_as_float`
both summing to 60.0 / 60.0f).  `cargo test --release --test native
native_scripts` passes (was failing pre-patch with "n_parallel_buf_get_float_native
not in scope" then with "par_narrow_buffer_stack is empty").
`cargo test --release --test threading` 47/0; `--test threading_chars`
43/0; `--test issues` 605/0.  A temporary `eprintln!` in
`parallel_light_execute_and_collect` confirmed the materialised
path is no longer reached for Single/Float.

**Wart-budget gate bump avoided** — the typed-buf_get design needs
no new IR Ops, so `dispatch_op_arm_budget_not_exceeded` stays at
the current cap.

**A4 unblocked.**  Every primitive return now routes through
Queue; the legacy `parallel_light_execute_and_collect` is reachable
only by the wide-input dispatch corner cases (none exercised in
the test corpus).  A4 can ship as a near-pure delete next.

##### Sequencing

A3.5 landed 2026-05-03 as a cheap follow-up to A3 (narrow Integer).
A3.6 landed 2026-05-07 via the simpler typed-buf_get design (no new
IR Ops needed).  Original sequencing note retained below for
historical context: After
A3.5, A4 (retire light Concat path) becomes actionable for the
bool/char/enum shapes too.  After A3.6, A4 cleanup is total.

Today's narrow-Integer subset already unblocks A4 for the
i32/u8/u16/i8/i16 return shapes that `light` was the fallback for.

**Acceptance test:** `par_struct_to_i32_t4` and `par_struct_to_byte_t4` route through the new narrow Queue path (verified by smoke-testing `target/release/loft /tmp/par_i32_b.loft` showing the expected `sum=-10`); `cargo test --release --test threading_chars` passes 35/35; `cargo test --release --test threading` passes 43/43; `cargo test --release --test issues` passes 540/540.

#### Why third

Gating work for retiring the light Concat path.  Without narrow-prim
Queue support, A4 cannot ship.  After A2 the allocator is stable
enough to extend.

#### Design

The current `n_parallel_queue` returns `Vec<u64>` (8-byte rows).
Narrow returns (1, 4 bytes; signed and unsigned variants) need
either width-parameterised reads or a separate buffer stack.

**Decision: width parameter on the existing buffer + a new getter.**
A second buffer stack would be cleaner but doubles the per-call
clone-reset cost; the width parameter is uglier but cheaper at
runtime, matching plan-06's "everything is a store" axiom (rows are
opaque bytes).

##### A3.1 — Buffer-stack changes

```rust
// src/database/mod.rs
pub struct Stores {
    // Existing:
    pub par_buffer_stack: Vec<Vec<u64>>,
    // Each entry is (raw bytes, per-element stride in bytes)
    // for narrow returns.  Stride = 1, 2, 4 — 8B uses par_buffer_stack.
    pub par_narrow_buffer_stack: Vec<(Vec<u8>, u8)>,
    // ...
}
```

`par_narrow_buffer_stack` mirrors `par_buffer_stack`'s 4 init sites
(constructor, Clone, both `WorkerStores::new` paths).

##### A3.2 — Native fns

```loft
# default/01_code.loft
pub fn parallel_queue_narrow(... return_size: integer) -> integer
    #rust "n_parallel_queue_narrow"
pub fn parallel_buf_get_narrow(idx: integer, return_size: integer,
                                signed: boolean) -> integer
    #rust "n_parallel_buf_get_narrow"
pub fn parallel_buf_drop_narrow()
    #rust "n_parallel_buf_drop_narrow"
```

`n_parallel_queue_narrow` runs the worker as today but pushes raw
bytes (return_size each) into a `Vec<u8>`; `_buf_get_narrow` reads
those bytes, sign-extends if `signed`, returns as `i64`.

##### A3.3 — Parser gate

Extend `route_through_queue` in `src/parser/collections.rs`:

```rust
fn route_narrow_queue(ret_type: &Type) -> Option<(u8, bool)> {
    use Type::*;
    match ret_type {
        Boolean | Enum(_, false, _) => Some((1, false)),
        Single | Character          => Some((4, false)),
        Integer(spec) if spec.size < 8 => Some((spec.size, spec.signed)),
        // 8-byte ints / Float go through the existing Queue (i64 buffer).
        _ => None,
    }
}
```

Float (8B) keeps using the regular `n_parallel_queue` — its bit
pattern fits in `u64`; the consumer reinterprets via `from_bits`.

##### A3.4 — Codegen extras

Both push-extras (state/codegen.rs:1948) and subtract-extras
(state/codegen.rs:2117) need entries for `n_parallel_queue_narrow`,
`n_parallel_buf_get_narrow`, `n_parallel_buf_drop_narrow`.  This is
the same shape as the 8c regression — adding only one site silently
drifts the codegen tracker by 8 bytes per call.

**Hardening:** add a regression test that exercises the
push/subtract symmetry directly.  Walk the codegen tables and assert
every entry in the push-extras map has a matching entry in the
subtract-extras map.

#### Risks

| Risk | Mitigation |
|---|---|
| Sign extension wrong for narrow signed ints (`i8` → `i64` reading bit 7 as sign) | Test each width individually; add `par_struct_to_i8_t4` if not present. |
| `n_parallel_buf_get_narrow`'s 3-arg shape blows the existing 2-arg buf-get codegen path | This is new code; design the codegen entry from scratch with the right arity. |
| Boolean / Enum-no-payload returns hit the `Type::Enum(_, false, _)` arm but get the wrong stride for an enum with > 256 variants | Loft caps single-byte enums at 256 variants; document this in the gate. |
| Codegen extras drift recurs | The proposed push/subtract symmetry test catches drift at parse time. |

#### Out of scope

Retiring `parallel_light_execute_and_collect` itself (A4).  Adjusting
the ref-buffer stack (untouched).

---

### A4 — Retire the light Concat path — DONE 2026-05-07

**Status:** DONE 2026-05-07.  After A3.6 routed Single/Float through
Queue, the light Concat path had no live callers in the test
corpus.  This commit deleted the implementation, replaced the
public natives with panic stubs, and pruned the
~110-LOC light-eligibility analysis in the parser.
**Effort:** S — landed in <0.5 session as projected.

**Closure summary:**

- `parallel_light_execute_and_collect` in `src/native.rs` —
  **deleted** (was ~50 LOC; the original ~300 LOC estimate
  conflated this with `n_parallel_for_light` and the supporting
  `run_parallel_light` infra).
- `n_parallel_for` and `n_parallel_for_light` in `src/native.rs` —
  bodies replaced with `unreachable!()`-with-diagnostic stubs.  The
  FUNCTIONS table entries are retained so def_nr lookups don't
  break and so any future stray emit gives a clear runtime panic
  pointing back at A4.  Old combined body was ~120 LOC (arg unpack,
  context fetch, `WorkerProgram` construction, return-size
  derivation, `parallel_light_execute_and_collect` call).
- `Stitch::Concat` enum variant in `src/parallel.rs` — **deleted**.
  The `stitch_tests::variants_distinguishable_by_match` test
  updated to drop the Concat case.
- `parallel_for_light` stdlib decl in `default/01_code.loft` —
  **deleted** (replaced with a back-pointer comment).
- `check_light_eligible` + 5 call-graph-walk helpers
  (`has_recursive_allocation`, `fn_allocates_stores`,
  `count_ref_vars`, `extract_callees`, `collect_callees`) in
  `src/parser/builtins.rs` — **deleted** (~110 LOC).  The light
  eligibility analysis was the gate that decided whether a worker
  could skip per-thread store deep-copy by routing through the
  light pool.  Irrelevant now.
- `actual_par_d_nr` selection in
  `src/parser/collections.rs::build_parallel_for_ir` — simplified
  to `par_for_d_nr` (was a 2-branch select that picked
  `n_parallel_for_light` if `light_m.is_some()`).
- `parallel_for(...)` user-facing builtin (also in
  `src/parser/builtins.rs`) — `light_m` selection removed; always
  routes to `n_parallel_for` (which panics if reached, since the
  user-facing call path is unexercised in the test corpus today).
- `src/scopes.rs::deep_par_call_callee_is_safe` test updated to
  use `parallel_queue` instead of `parallel_for_light` as the
  ParCall purity stand-in.
- `tests/threading.rs::purity_annotations_parsed_from_stdlib`
  updated similarly.

**Acceptance:**

- Original gate (`git grep -l parallel_light_execute_and_collect`
  returns 0) — partially met.  The function name appears in 5
  files post-A4: 3 documentation files (CHANGELOG_TECHNICAL.md,
  PAR_PRESENTATION.md, ARC.md), 1 source comment in `src/parallel.rs`
  documenting the `WorkerPool::new` dead-code allow, and 1 test
  comment in `tests/scripts/22-threading.loft` documenting why the
  exact-sum check pins the new Queue route.  No live code refers
  to the function.  Reinterpreting the gate as "no callable
  reference" (the original intent), met.
- `cargo test --release --test issues` 605/0.
- `cargo test --release --test threading` 47/0 (after the stale
  `n_parallel_for_light` purity assertion was retargeted to
  `n_parallel_queue`).
- `cargo test --release --test threading_chars` 43/0.
- `cargo test --release --test native native_scripts` ok.
- `cargo test --release --test wrap loft_suite` ok.
- CI gate: `cargo fmt --check`, `cargo clippy --release --all-targets
  -- -D warnings`, `cargo build --release --no-default-features` all
  clean.

**Net LOC change:** approximately −230 production LOC
(parallel_light_execute_and_collect ~50, light eligibility helpers
~110, n_parallel_for/_light bodies ~120, Stitch::Concat variant + test
case ~15, parser materialised-path branching ~15, stdlib decl 1)
balanced against ~50 LOC of unreachable! stubs and back-pointer
comments.

**Out of scope (A4 boundary unchanged):** `par_light` user-surface
removal (A9 — keyword still resolves; runs into the panic stub
today).  Trait dispatch unification (A8 — Queue family is still
five separate functions).

---

### A5 — `Stitch::Reduce` runtime + `par_fold` surface

**Status:** runtime DONE 2026-05-03; user-facing parser builtin DONE 2026-05-07 (interp); native runtime A5b DONE 2026-05-07.
**Effort:** M (~1 session) — runtime + tests in <1 session.
**Acceptance test:** `par_fold_int_sum` (sum 1..=100 = 5050),
`par_fold_max`, `par_fold_empty_input`, and
`par_fold_single_thread_matches_multi_thread` all pass in
`tests/threading.rs`.

**Runtime closure (this session):**

- `run_parallel_fold` added to `src/parallel.rs` (threading +
  no-threading variants).  Workers split input across N threads,
  each accumulates over its slice via `fold(acc, row) -> acc`
  starting from `init`, then main thread combines per-worker
  partials with the same fold fn (preserving worker-completion
  order).  V1 restricted to integer accumulator + integer row type.
- `n_parallel_fold` native fn in `src/native.rs` pops args
  (input/init/fold_d_nr/threads/extras) from the runtime stack,
  invokes `run_parallel_fold`, pushes the i64 result.
- Stdlib decl `fn parallel_fold(input, init, fold, threads) ->
  integer` in `default/01_code.loft` (line ~1140).
- Codegen extras-push + extras-subtract entries in
  `src/state/codegen.rs` mirror the existing queue-family pattern.
- 4 round-trip tests in `tests/threading.rs` exercise the runtime
  via direct `run_parallel_fold` calls (compile + load worker fn
  + invoke).

**User-facing parser builtin DONE 2026-05-07 (interp):**

`par_fold(items, init, fn_name, threads)` is now a parser builtin
in `src/parser/builtins.rs::parse_par_fold`, dispatched from
`src/parser/control.rs:3030`.  The builtin:

- Validates V1 type constraints (items: `vector<integer>`, init:
  `integer`, fold: `fn(integer, integer) -> integer`, threads:
  `integer`).
- Resolves the fold function reference (a bare name; the parser
  produces `Value::Int(d_nr)` with `Type::Function(...)` for it).
- Emits a single `Call(n_parallel_fold, [input, init, fold,
  threads, n_extra=0])`.  The `n_extra` count mirrors
  `build_parallel_for_ir`'s analogous emit at
  `src/parser/collections.rs:1902`.

Acceptance tests: `tests/scripts/22b-par-fold.loft` covers all
four canaries (sum 1..=100 = 5050; max; empty input; 1-thread vs
multi-thread agreement).  Runs interp via `wrap loft_suite`.

**A5b DONE 2026-05-07 — native runtime:**

Closed the native gap left by A5.  Before the fix, the native
backend's `parallel_fold` call hit the auto-generated 5-arg stub
(`fn n_parallel_fold(cell, input, init, fold, threads) -> i64`);
the parser emits a 6-arg call (with the `n_extra` count) and rustc
rejected with E0061.  Landed:

- `n_parallel_fold_native<F>` in `src/codegen_runtime.rs` with the
  closure-based shape used by `n_parallel_queue_native` etc.  The
  worker closure has signature
  `Fn(&UnsafeCell<Stores>, i64, i64) -> i64 + Send + Sync`
  (cell, acc, row → acc).
- `run_native_workers_fold` helper (mirror of
  `run_native_workers_primitive`): each worker accumulates over its
  slice via `worker(cell, acc, row_val)` from `init` and returns
  the final partial; main thread combines partials in
  worker-completion order via the same closure.  V1 reads i64 rows
  inline via unaligned load (vector<integer> stride 8).
- `ParallelFoldEmitter` in `src/generation/ops/parallel.rs`
  registered for `n_parallel_fold`; rewrites the Call to
  `n_parallel_fold_native(cell, input, init, threads, |cell, acc,
  row| worker(cell, acc, row))`.  Differs from the for/queue
  emitters in arg layout (worker fn at `args[2]` instead of
  `args[4]`; init at `args[1]`).
- `n_parallel_fold` added to `collect_calls` in
  `src/generation/mod.rs` so the worker fn (referenced via
  `args[2]`) is included in the reachable set — without this,
  rustc fails with E0425 ("cannot find function `n_<worker>`").
- `tests/scripts/22b-par-fold.loft` removed from
  `SCRIPTS_NATIVE_SKIP` — `cargo test --release --test native
  native_scripts` exercises all 4 canaries on native.

V1 ignores `args[4]` (n_extra=0); extras pass-through is a future
ARC step if a use case surfaces.  Heterogeneous types (V2:
`vector<T>` + `fn(R, T) -> R` with R ≠ T) and A5.3 auto-detection
remain separate items.

The auto-detection variant (A5.3 — rewrite `sum(parallel_for(...))`
patterns into `par_fold(...)`) remains an open follow-up.  Builds
on the explicit `par_fold` surface landed today.

**Bug-hunt yield this session:** zero — the runtime built cleanly
on top of existing infrastructure (`execute_at_raw_primitive_input`,
`parallel_workers`, `merge_batches`).  No latent bugs surfaced;
the well-trodden `(acc, row, extras)` parameter shape mapped
directly to existing primitive-input dispatch.

#### Why fifth

With Concat retired, Reduce is the last missing Stitch policy.  After
this all common par shapes have a non-materialising runtime path.

#### Design

Per DESIGN.md D7, Reduce is the third policy.  Each worker keeps a
**partial accumulator**; main thread combines partials with the
user-supplied fold operator.

##### A5.1 — Runtime

```rust
// src/parallel.rs
pub fn run_parallel_reduce<R, F>(
    stores:        &mut Stores,
    program:       &Program,
    fn_pos:        u32,
    input:         DbRef,
    init:          R,           // identity
    fold:          F,           // (R, R) -> R: monoid combine
    n_threads:     u32,
    extra_args:    &[u8],
    return_size:   u32,
) -> R
where
    R: Send + Clone,
    F: Fn(R, R) -> R + Sync,
{
    parallel_workers(n_threads, |t, ws| {
        let mut acc = init.clone();
        for row_idx in worker_slice(t, n_threads, input.len()) {
            let row_result = execute_at_raw(ws, program, fn_pos, row_idx, ...);
            acc = fold(acc, row_result);
        }
        acc
    }).into_iter().fold(init, fold)
}
```

The monoid contract (associativity, identity) is the user's
responsibility — the runtime preserves left-to-right order within
each worker, then combines partials in worker-completion order.
Documentation calls this out.

##### A5.2 — Surface

```loft
# default/01_code.loft
pub fn par_fold<T, U>(
    input:    vector<T>,
    init:     U,
    fold:     fn(U, T) -> U,
    threads:  integer,
) -> U #rust "n_parallel_fold"
```

User writes:
```loft
total = par_fold(items, 0, fn(acc, x) -> acc + x.score, 4)
```

##### A5.3 — Auto-detection

Phase 5's walker (`Parser::check_par_result_singlepass` in
`src/parser/mod.rs`) already classifies par-result uses.  Extend it
to detect fold patterns:

```loft
total = sum(parallel_for(items, score_of, 4))
// rewrites to:
total = par_fold(items, 0, fn(acc, x) -> acc + score_of(x), 4)
```

The walker matches `Sum(ParCall) | Max(ParCall) | Min(ParCall) |
Concat(ParCall) | Product(ParCall)` and rewrites.  Any other use is
already a hard error per spine step 7.

##### A5.4 — Native fn

`n_parallel_fold` mirrors `n_parallel_queue`'s shape: pop input,
init, fold-fn, threads from the runtime stack; call
`run_parallel_reduce`; push the scalar result.  Codegen extras
both sites.

#### Risks

| Risk | Mitigation |
|---|---|
| Reduce's "user-supplied fold operator" type-checks differently from a simple closure (e.g. capturing closures need the 4d.C closure-storage redesign) | Initial scope: bare named fn only.  Capturing closure-fold deferred to follow-up. |
| Auto-detection rewrites a non-monoid `sum(...)` (user wrote a lossy op) | Rewrite rule applies only to recognised monoidal ops in stdlib (`sum`, `max`, `min`, `product`, text `concat`).  User-defined "sum" — leave alone. |
| Empty input edge case: rayon dispatches zero workers, identity returns | Test `par_fold_empty_input` covers this; default to identity, no special arm. |

#### Out of scope

Trait-dispatch unification of Queue variants — that's A8.  Auto-light
selection for Reduce workers — A9.

---

### A6 — Close the 4 fn-ref / vector / keyed-collection canaries

**Status:** DONE 2026-05-01 (all 4 sub-steps A6.a / A6.b / A6.c / A6.d landed; commits `b9f7fc1`, `17bb33f`, `f048d20`, `792ea7b`).  Acceptance test passes — all 4 canaries (`par_struct_to_vector_t4`, `par_struct_to_fn_t4`, `par_vec_of_fns_input_t4`, `par_struct_to_keyed_collection_t4`) un-`#[ignore]`'d and green.
**Effort:** L (~2 sessions) — actual: closed in a single session due to P196 unblock retiring the 4d.C closure-storage prerequisite.
**Acceptance test:** all 4 canaries un-`#[ignore]`'d and passing.

#### Why sixth

These are the type-coverage canaries plan-06 was chartered to close.
Each represents user-facing functionality that doesn't work today.
They cluster because they share infrastructure (4e hidden-arg
destination, 4d.C closure storage).

A6 is split into 4 sub-PRs because each canary has an independent
fix path.  Each sub-PR closes exactly one canary.

#### A6.a — `par_struct_to_vector_t4` (vector return) — **DONE 2026-05-01**

**Closure notes:**

- `execute_at_ref` (`src/state/mod.rs`) gained a `hidden_dests: &[DbRef]`
  parameter.  Each destination is pushed as 12 bytes after the input
  arg and before regular extras — matching the parameter order the
  codegen assumes for `ref_return`-promoted hidden args.
- `run_parallel_queue_ref` (`src/parallel.rs`) gained an
  `n_hidden_dests: usize` parameter.  For each row, allocates that
  many backing stores via `state.database.database(100)` (NOT
  `null()`, which returns a u32::MAX-sized sentinel store with no
  usable storage — the worker's `out += [i]` writes silently fail
  on it).  Each destination's store is dispenser-allocated, so
  adoption + rebase + revive_record_chain pull it back into parent
  alongside the result DbRef.
- `n_parallel_queue_ref` (`src/native.rs`) computes `n_hidden_dests`
  from `def.attributes.iter().filter(|a| a.hidden && !a.name.starts_with("__")).count()`
  before the mutable borrow of `stores`.
- Two `tests/threading.rs` direct call sites updated for the new
  arity.  Canary `par_struct_to_vector_t4` un-`#[ignore]`'d and
  passing.  Ignored canary count: 7 → 6.

**Design (original):** Implement phase 4e (`ref_return()` hidden-arg
destination machinery, designed in
[04-typed-input-output.md:459](04-typed-input-output.md)).

Five concrete changes:

1. **Detect hidden args at the dispatcher.**  In
   `src/native.rs::n_parallel_for` (or A4's successor that picks
   Queue), inspect `def.attributes` for entries with `hidden = true`.
   Build a `Vec<usize>` of hidden-arg indices.

2. **Extend dispatch to carry destination kind.**  Add
   `OutputKind::RefHiddenDest { type_nr: u16 }` so the dispatcher
   knows it must allocate per-row destinations:
   ```rust
   enum OutputKind {
       Primitive { size: u8 },
       Text,
       RefDirect,
       RefHiddenDest { type_nr: u16 },  // new
   }
   ```

3. **Per-row destination allocation in workers.**  Before invoking
   the worker, the dispatcher allocates a destination DbRef in the
   worker's output store of `type_nr` and pushes it as the hidden
   first extra.

4. **Result collection.**  Worker writes into the destination; on
   thread join, the destination DbRef is rebased into the parent
   namespace via existing 8d.3 per-thread machinery.

5. **Stitch::Queue plumbs the per-row DbRef into
   `par_ref_buffer_stack`.**  The body's `_get_ref(idx)` reads the
   destination, which now holds the worker's vector.

**Closes:** `par_struct_to_vector_t4` (line 639); G6 in
`01-output-store.md`.

#### A6.b — `par_struct_to_fn_t4` (fn-ref return) — **DONE 2026-05-01**

**Closure notes:**

Closed by a packed-buffer Queue route (mirrors A3 narrow-buffer + A6.c
patterns).  L1 (post-A1 truncation) + L2 (body's get_field-as-i32
misread) both bypassed by routing fn-ref returns through a dedicated
buffer + getter that pushes 20 bytes directly onto the operand stack.

**Six layers wired:**

1. **`Stores::par_fn_buffer_stack: Vec<Vec<u8>>`** — sibling stack to
   `par_buffer_stack` / `par_text_buffer_stack` / `par_ref_buffer_stack` /
   `par_narrow_buffer_stack`.  Each entry is `n_rows * 20` bytes,
   one fn-ref per row in Rust's reordered DbRef layout.  Init at 4
   sites (`Stores::new`, `Clone`, both `WorkerStores::new`).
2. **`run_parallel_queue_fn`** in `src/parallel.rs` — workers write
   each row's 20-byte fn-ref directly via `State::execute_at_raw_to`
   to disjoint slots in a packed `Vec<u8>` (concurrent writes safe;
   row ranges from `parallel_workers` are non-overlapping).  Scope:
   DbRef-input only (matches A6.c discipline).
3. **3 native fns** — `n_parallel_queue_fn` / `_buf_get_fn` /
   `_buf_drop_fn` in `src/native.rs` + FUNCTIONS table + decls in
   `default/01_code.loft`.
4. **Codegen extras** at both push and subtract sites for
   `n_parallel_queue_fn`.
5. **Parser gate** in `src/parser/collections.rs` — `route_fn_queue`
   bool at early + late sites; new dispatch arm for fn-ref returns.
6. **Body substitution divergence** — for fn-ref returns, do NOT
   call `replace_var_in_ir`.  The body's `f(10)` parses as
   `CallRef(b_var, [Int(10)])` and `replace_var_in_ir`
   (`src/parser/collections.rs:replace_var_in_ir`) walks
   `CallRef(_, args)` into `args` but doesn't substitute the first
   u16 (the fn-ref var index).  Inline substitution would leave
   b_var as a dangling reference.  Instead, keep b_var as a real
   variable with a 20-byte slot, mark it `in_use(true)` for the
   slot allocator, and prepend `Set(b_var, Call(buf_get_fn, [idx]))`
   to the body block so each iteration's CallRef reads the fresh
   fn-ref blob.

**Critical decl detail:** `parallel_buf_get_fn` is declared with
return type `fn(integer) -> integer` (NOT `integer`) so
`variables::size(Type::Function, Argument) = 20` matches the runtime
push via `put_stack::<[u8; 20]>`.  Declaring as `integer` would make
the codegen tracker advance by 8 while runtime advances by 20 → 12-
byte tracker drift → CallRef reads d_nr at the wrong offset → bogus
d_nr → panic.  The fn signature is canary-specific today; future
fn-ref-return canaries with different signatures may need decl
variants or a more generic mechanism.

**Closes:** `par_struct_to_fn_t4` (line 686); G4 in
`01-output-store.md`.  Ignored canary count: 5 → 4.

**L1 — return_size truncation in light path (mechanical fix; ready to ship):**
After commit `b9ad7af` retired the heavy path, fn-ref returns route
through `n_parallel_for_light` → `parallel_light_execute_and_collect`
→ `run_parallel_light` → `execute_at_raw`.  Two sub-bugs in this
chain:

1. `n_parallel_for_light` clamps `return_size` to `1..=8` (`src/native.rs`
   ~line 676): `let rs = if (1..=8).contains(&derived) { derived }
   else { v_return_size.clamp(1, 8) as u32 };`.  For Type::Function,
   `derived = 20`, so the clamp drops to 8.  Fix: relax to allow the
   derived size when ≥ 1 (only fall back to v_return_size when
   `derived = 0`).
2. `run_parallel_light`'s body calls `execute_at_raw` (returns `u64`,
   8 bytes) and then `copy_nonoverlapping(ret_sz)` from `&u64` —
   reads UB Rust stack bytes for ret_sz > 8.  Fix: when `prim_in == 0
   && ret_sz > 8`, route directly through `execute_at_raw_to` which
   writes ret_sz bytes from the worker's stack top to dst.

L1 verified:
- Worker correctly writes 20-byte fn-ref to the heap result vector.
- Bytes at heap row[0..7] = correct d_nr (e.g. 0x21e = 542 for triple).
- Bytes at heap row[8..19] = correct closure DbRef sentinel
  (Rust reorders DbRef to {rec u32, pos u32, store_nr u16, padding},
   so the u16::MAX lands at heap-row offsets 16-17, NOT 8-9 — but
   that IS the correct in-memory layout for `*addr_mut::<DbRef>`).

**L2 — body's `get_field` for Type::Function returns (deep codegen):**

After L1 lands, the heap result vector contains correct 20-byte
fn-refs.  But the body's read path is wrong:

- Parser substitutes `Var(b_var: fn-ref)` with `get_field(vec_tp,
  usize::MAX, OpGetVector(results, 20, idx))` (`src/parser/collections.rs:1880`).
- For Type::Function, `vec_tp = type_def_nr(Type::Function) =
  def_nr("i32")` (`src/data.rs:2944`).  So `get_field(i32_d, usize::MAX, ...)`
  treats the fn-ref vector element as an i32 (4-byte int).
- The body's `f` ends up with a 4-byte i32 read where 20 bytes of
  fn-ref blob are needed.  `OpCallRef` then reads d_nr at the wrong
  offset and panics with `d_nr=1344256 out of range`.

**Why this is L2 (deep codegen)** — the existing get_field +
OpGetVector pattern is shared across all return types; making it
20-byte-fn-ref-aware requires either:
- A typed wrapper around get_field that emits OpVarFnRef-style 20-byte
  reads at the row's record position when ret_type is Type::Function, or
- Routing fn-ref returns through a packed-buffer Queue path (similar
  to A3 narrow-buffer) with a typed `n_parallel_buf_get_fn(idx) -> fn(_) -> _`
  getter that pushes 20 bytes onto the operand stack.

The packed-buffer route is structurally cleaner — it bypasses the
heap-result-vector abstraction entirely and matches A6.c's working
pattern for fn-ref vector inputs.  Estimated ~1 session.

**Plan**: ship L1 + L2 together when L2 is approached.  Don't ship L1
alone — it removes the truncation at the cost of adding UB at the
get_field step (silently corrupted reads vs. earlier silent
truncation).

**Closes:** `par_struct_to_fn_t4` (line 686); G4 in
`01-output-store.md`.  P196 was closed independently on 2026-04-30
via `output_call_template` projection (no longer requires 4d.C).

#### A6.c — `par_vec_of_fns_input_t4` (fn-ref vector input) — **DONE 2026-05-01**

**Closure notes:**

The bug turned out to be different from the original 3-bug cascade
documented in [04d-followups.md](04d-followups.md) — the post-A1
dispatcher landscape made the codegen stack-tracker issue
unreachable.  Two surgical fixes closed the canary:

1. **`Data::narrow_vector_content` (`src/data.rs`)** extended to
   route `Type::Function` to `database.int(0, false)` (a real
   `Parts::Int` type with `size = 4`).  Previously, `vector_of`
   fell through to `def_nr("i32").known_type` which lands on a
   placeholder type with `size = 0`.  With stride 0, every literal
   element write in `[dbl, triple, quad]` overlapped offset 8 —
   `length=3` but only the last d_nr survived.
2. **`read_tuple_at_wide` (`src/parallel.rs`)** special-cased for
   `Type::Function` elements: the worker's argument slot is 20
   bytes (8B i64 d_nr + 12B closure DbRef) but the storage is 4
   bytes (just d_nr).  Plain memcpy of 4 bytes left the closure
   DbRef portion zero, so OpCallRef in the worker dereferenced
   `(store_nr=0, rec=0, pos=0)` — a real DbRef into store 0,
   which SIGSEGV'd.  Fix: zero-extend the d_nr to 8 bytes and
   write a `(u16::MAX, 0, 0)` sentinel for the closure portion
   (vector<fn> can only store non-capturing fns; capturing-lambda
   storage is part of the open 4d.C closure-storage redesign).

Diagnosis path: traced via eprintlns through OpNewRecord,
OpFinishRecord, vector_append, vector_finish, and read_tuple_at_wide
to identify the size=0 stride at `record_new`'s `Parts::Vector(c)`
arm.

**Original design (superseded):** Resolve the 4d.A.2 cascade
documented in [04d-followups.md](04d-followups.md).  Three remaining
bugs:

1. **Vector storage layout** — fn-ref vector elements need 4-byte
   stride (current parser fix landed) but read-back assumes 8 bytes.
2. **Worker entry path** — `execute_at_raw_primitive_input_wide`
   reads the wide first arg; for fn-ref, the apply opcode's
   compile-time stack tracker disagrees with the worker entry's
   runtime stack layout (tracker says `stack_pos=24` at `CallRef`,
   runtime is 36).
3. **Native codegen** — undefined `t65` reference; closure
   `(u32, DbRef)` type mismatch.

Each bug needs a focused fix.  The closure-storage redesign (4d.C)
makes the layout consistent — 16 bytes everywhere — eliminating
bugs 1 and 3.  Bug 2 (codegen tracker) is the residual deep work.

**Closes:** `par_vec_of_fns_input_t4` (line 530).

#### A6.d — `par_struct_to_keyed_collection_t4` (keyed-collection return) — **DONE 2026-05-01**

**Closure notes:**

- Three sites in `src/parser/collections.rs` extended to accept
  `Type::Sorted` / `Hash` / `Index` / `Spacial`: (1) the
  `return_size = -1` ref-mode matcher at line ~1404, (2)
  `early_route_ref_queue` at line ~1566, (3) `route_ref_queue` at
  line ~1816.
- The runtime `run_parallel_queue_ref` + `Stores::adopt_worker_excess`
  + `rebase_walk_record` path required no changes — `data::owned_elements`
  already enumerates each keyed type's internal owned-DbRef fields, so
  the rebase walk is type-correct.
- Canary `par_struct_to_keyed_collection_t4` un-`#[ignore]`'d and
  passing.  Ignored canary count: 8 → 7.

**Original design:** Worker fn returns a keyed collection (sorted /
hash / index / spacial).  Today's engine rejects with "Parallel worker
return type ... is not supported".

Keyed collections are `Reference<KeyedT>` under the hood.  After
8d's Queue ref-return path is stable, accepting keyed-collection
returns is a parser gate adjustment in `route_ref_queue`:

```rust
match ret_type {
    Type::Reference(_, _)
    | Type::Enum(_, true, _)
    | Type::Vector(_, _)
    // After A6.d:
    | Type::Sorted(_, _) | Type::Hash(_, _, _)
    | Type::Index(_, _, _) | Type::Spacial(_, _) => true,
    _ => false,
}
```

The worker's output store must be type-compatible (the keyed
collection's underlying records).  4d.B's
`materialise_keyed_for_par` already handles keyed-collection
**input**; this is the symmetric case for output.

**Closes:** `par_struct_to_keyed_collection_t4` (line 663).

#### Risks (A6 overall)

| Risk | Mitigation |
|---|---|
| 4d.C closure-storage redesign is its own session of work and lands as A6.c.0 | Acceptable; file as such in this doc.  Don't bundle into A6.c. |
| A6.b stack_pos reconciliation drags codegen rewrites unrelated to par | Time-box at 1 session.  If it bleeds into a second, extract codegen fixes as a separate arc step. |
| Sub-PRs land in different orders → integration drift | Pick a fixed order (a → c → b → d) so each builds on stable prior work. |

#### Out of scope

Tuple returns — A7.  Tuple inputs — out of plan-06.

---

### A7 — Close the par-tuple canaries (T1.8a unblocked 2026-05-04)

**Status:** OPEN — unblocked 2026-05-04 by T1.8a fix in branch
`plan-14-tuple-validation` (commit `023ca15`).  T1.8a's actual-error
survey turned the original ~200-LoC design (new `Value::ReturnTuple`
IR variant + `OpReturnTuple` opcode + caller-pre-allocated slot)
into a ~30-LoC fix in `src/generation/{mod.rs,emit.rs,dispatch.rs}`
— see PLANNING.md § T1.8a for details.  No `OpReturnTuple` opcode
exists; the par dispatch wires through the same Variable-context
tuple-element type-routing the general fn-return path uses.

A7's per-arity expectation list grew from the plan-14 phase 02 /
type-spectrum audit (2026-05-04, plan-06 type-spectrum commit
`9db18fd`).  The 4 originally-tracked canaries plus the 3 new
broader-coverage canaries are now A7's full target set.

**Effort:** L (~2-3 sessions, post-survey 2026-05-07).  Original
M estimate revised after the actual-error survey revealed three
distinct fix surfaces (see "Actual-error survey 2026-05-07" below).
**Acceptance test:** all 7 tuple canaries pass; D11b "✅ when
tuples land" placeholder retires.

#### Actual-error survey 2026-05-07

The 6 ignored canaries were un-ignored briefly to read the actual
failures.  Result: NOT one fix surface but **three**, with very
different scopes:

| Surface | Canaries | Blocker | Sub-step | Effort |
|---|---|---|---|---|
| **A7.1** | `par_tuple_return_int_int`, `par_tuple_return_int_text`, `par_tuple_return_three_arity`, `par_tuple_return_nested` | Parser gate at `collections.rs:1539` rejects tuples > 8B.  Relaxing the gate exposes the next layer (`n_parallel_for unreachable!` — routing falls back to the materialised path A4 retired). | New tuple_queue family: `par_tuple_buffer_stack` field + `n_parallel_queue_tuple` + `n_parallel_buf_get_tuple` + `n_parallel_buf_drop_tuple` + stdlib decls + parser routing extension.  Comparable scope to A5b (closed 2026-05-07). | M (~1 session) |
| **A7.2** | `par_tuple_destructure_in_for` | General parser feature gap — `for (a, b) in items { ... }` rejects with "Expect variable after for" **even without par**.  Filed as P235. | Extend the for-loop parser to accept `(name1, name2, …)` destructure; desugar to `for _t in items { name1 = _t.0; name2 = _t.1; … }`.  Closes both par and non-par destructure with one rewrite. | S-M (~0.5–1 session) |
| **A7.3** | `par_tuple_return_struct_text` | Lexer bug — `tuple.0.field` syntax fails with "Problem parsing float" because `0.field` is tokenised as a malformed float literal.  Reproduces outside par.  Filed as P234.  Plus runtime gap: function-return tuples containing Reference / Text / etc. corrupt at the call boundary on `--native`. | Lexer fix (number-tokeniser when `.` is followed by a non-digit identifier start) + runtime fix routing lifetime-bearing tuple returns through synthetic `__tuple<…>` struct (Plan-14 phase 07).  Both DONE 2026-05-08; canary un-ignored. | M (~1 session) |

The original ARC.md "uniform across the 5 return-cases" risk-row
claim was wrong: only A7.1's 4 canaries share a fix surface.  A7.2
turns out NOT to be par-specific — any for-loop user benefits.
A7.3 is upstream of par entirely; A7's canary just happens to
exercise it.

##### A7.1 — Wide-return tuple runtime (Surface 1)

**Status (2026-05-08): DONE.**

Closed by the size-based gate widen + work-ref unification stack
that also closes [P236](../../PROBLEMS.md):

- `src/parser/definitions.rs::parse_function` rewrites the
  `returned` type from `Type::Tuple(elems)` to
  `Type::Reference(tuple_def(elems))` when EITHER any element has
  a lifetime concern (Phase 07) OR the tuple's stack size exceeds
  the 8-byte primitive return slot (A7.1).  Pure 1-arity scalar
  tuples that fit in 8B keep the Rust ABI.
- `src/parser/control.rs::rewrite_tail_tuple_to_synthetic_struct`
  recurses through `Value::If` / `Value::Block` / `Value::Insert`
  / `Value::Span` tails with one SHARED work-ref via
  `rewrite_tail_tuple_with_work_ref`, so all branches construct
  into the same record.  The recursion guard `tail_has_tuple_leaf`
  fires only when at least one tuple leaf is reachable.
- `src/parser/expressions.rs` destructure path accepts both
  `Type::Tuple([...])` and `Reference(__tuple<...>)` shapes.
- The shared-work-ref pattern matches what P236's struct-case
  unification (`unify_if_branches_work_refs`) does, so both
  paths converge on the same scope/native-codegen behaviour.

All 5 par_tuple_return canaries are now un-ignored and PASSING:
`int_int`, `int_text`, `struct_text`, `three_arity`, `nested`.

---

Original A7.1 plan (new tuple_queue family — superseded by the
unification approach which had zero new opcodes / runtime helpers):

The fn-ref pattern at `parallel.rs:1347` (`run_parallel_queue_fn`)
is the closest template: workers pack their N-byte returns into
a `Vec<u8>` via `state.execute_at_raw_to`, the buffer is pushed
onto a per-stack-of-buffers field on `Stores`, and the body's
`b_var` reads via `Set(b_var, Call(buf_get, [idx]))`.  Adapt
to variable size:

- `Stores::par_tuple_buffer_stack: Vec<(Vec<u8>, u32)>` — bytes
  + per-row size (workers in one queue all use the same size).
- `n_parallel_queue_tuple` — pops args (input, elem_size,
  return_size, threads, fn, extras...), runs workers, pushes
  the buffer.
- `n_parallel_buf_get_tuple(idx, return_size)` — copies
  `return_size` bytes from `buf[idx*return_size..]` to the stack.
  Variable size means we cannot use `stores.put<[u8; N]>` with a
  fixed `N`; needs a `put_bytes(stack, &slice, n)` helper or a
  custom opcode.
- `n_parallel_buf_drop_tuple` — pops the buffer.
- Stdlib decls in `default/01_code.loft` (3 fns).
- Parser changes: relax the gate to admit `Type::Tuple(_)` ≤ 64B,
  add `route_tuple_queue` in both early and main gate sites
  (collections.rs:1641 + :1934 areas), wire body's `b_var` Set.
- Codegen extras-push entry for `n_parallel_queue_tuple`.

The variable-size-write challenge: `b_var`'s slot is sized to
the tuple's actual layout (not a fixed cap).  A 16B tuple's slot
is 16B, not 64.  So `buf_get_tuple` must memcpy exactly the right
number of bytes.  The cleanest fix is a new helper — `put_bytes`
in `Stores` — that takes a slice and copies it; the alternative
(per-size family `buf_get_tuple_8/16/24/.../64`) is uglier.

##### A7.2 — For-loop tuple destructure (Surface 2)

NOT a par-specific fix.  Tracked as P235.  The desugar is
mechanical:

```loft
for (a, b) in items { use(a, b) }
// rewrites to:
for _t in items { a = _t.0; b = _t.1; use(a, b) }
```

Implementation: extend the for-loop parser dispatch to detect
`(` after `for`, parse the comma-separated identifier list,
allocate a synthetic temp `_t`, then prepend element-extraction
`Set` ops to the body.

**Non-par half DONE 2026-05-07** in `src/parser/collections.rs::parse_for`:
parser detects `(` after `for`, parses the identifier list,
synthesizes a temp loop var named `__destructure_t_<line>_<pos>`,
defines each user-named binder as a proper variable typed as the
matching tuple element, and prepends `Set(name_i, get_val(loop_var,
offset_i))` ops to the body block.  Handles both direct
`Type::Tuple([…])` and the common `Type::Reference(__tuple<…>)`
shape (vector<(T1,T2)> via P189b's element-access path).  Pinned
by `tests/issues.rs::p235_for_tuple_destructure_{two_arity,three_arity,int_text}`.

**Par half OPEN**: ARC.md's original "par variant is automatic"
claim was wrong — the existing par dispatch passes ONE
per-iteration arg (the loop element) and N context args (same
every iteration).  Destructure wants multiple per-iteration args
derived from the tuple, which the dispatch shape doesn't support.
Cleanest follow-up: at parse time, when destructure is paired
with par, synthesize a wrapper worker
`__par_destructure_w<N>(t: tuple_t) -> R { worker(t.0, t.1, …) }`
and rewrite the par expression to call the wrapper with the
tuple loop element.  Until then `par_tuple_destructure_in_for`
stays ignored.

##### A7.3 — Tuple-of-struct member access (Surface 3)

NOT a par-specific fix.  Tracked as P234.

**Lexer half DONE 2026-05-07** in `src/lexer.rs::number`: extended
P195's `prev_was_field_dot` branch to fire regardless of what
follows the second `.` (digit, identifier, anything).  Pre-fix
only the `n.0.0` digit-after-dot case was split; now `r.0.x`
(identifier-after-dot) and `r.0.;` (anything else) all emit
integer + queue `.` so the next token re-lexes fresh.  Pinned by
`tests/lexer::test::p234_tuple_index_then_field_does_not_glue_into_float`
plus the surface-level reproducer
`tests/issues.rs::p234_lexer_accepts_tuple_index_then_struct_field`.

**Runtime half DONE 2026-05-08** by routing tuple-with-lifetime-concern
returns through the existing synthetic `__tuple<…>` struct
(Plan-14 phase 07 — see
[../14-tuple-validation/07-p234-runtime.md](../14-tuple-validation/07-p234-runtime.md)).
`src/parser/definitions.rs::parse_function` rewrites the function's
`returned` from `Type::Tuple(elems)` to `Type::Reference(tuple_def(elems))`
whenever any element has a lifetime concern (Text, Reference,
Vector, Enum-struct, keyed collection, RefVar, or a nested tuple
containing one).  `src/parser/control.rs::block_result` then
detects body-tail `Value::Tuple(...)` literals and rewrites them
via `rewrite_tail_tuple_to_synthetic_struct` into the same
allocation + per-field-init sequence inline struct literals
produce.  All existing struct-return ownership-transfer machinery
applies unchanged.  Pure-value tuples keep the Rust tuple ABI;
T1.8a's `(text, text)` text-tuple machinery becomes superseded
but kept as defensive fallback.

`par_tuple_return_struct_text` is now un-ignored and PASSING
(2026-05-08) — the canary's expected value of 11 was an author's
miscount; correct sum is 12.

#### Canaries closed by A7

| Canary | Shape | Closed by sub-step |
|---|---|---|
| `par_tuple_input_int_int` | 2-arity scalar input | A7.0 (input dispatch) — already closed by phase 4d.B + P189; verify still green |
| `par_tuple_input_int_text` | 2-arity mixed input | A7.0 — closed by P189d; verify |
| `par_tuple_return_int_int` | 2-arity scalar return | A7.1 (Wide-return path) |
| `par_tuple_return_int_text` | 2-arity mixed return | A7.1 |
| `par_tuple_return_struct_text` | 2-arity ref + text return | A7.1 |
| `par_tuple_return_three_arity` *(new 2026-05-04)* | 3-arity scalar return — pins "any arity" claim | A7.1 |
| `par_tuple_return_nested` *(new 2026-05-04)* | `((A, B), C)` nested-tuple return | A7.1 |
| `par_tuple_destructure_in_for` | fused-for tuple destructure binding | A7.2 (Destructure binding) |

Adjacent canary, NOT closed by A7 (different fix surface):
- `par_vec_of_capturing_fns_t4` — heterogeneous capturing closures
  in `vector<fn(...)>`.  Failure is at vector-construction (lambda
  → vector storage path), not at par dispatch.  Tracked in plan-15
  D4; cross-referenced by DESIGN.md D11a row 8.

#### Why seventh

Tuple support is an orthogonal feature (T1.8a — function
tuple-return convention) that benefits any `-> (A, B)` function.
Plan-06 surfaces the rough edges, but T1.8a is the underlying
work and should ship as a standalone milestone first.

#### Design

##### A7.0 — T1.8a function-return convention (prerequisite)

Designed in [09-tuple-support.md:99 §9a](09-tuple-support.md).

Tuple returns use a **caller-supplied destination**: the tuple's
backing store is the caller's stack frame.  Worker writes elements
to known offsets; caller pops the tuple as a contiguous block.

Five-arg case (`(int, int, int, int, int)`) is the smoke test;
mixed-type case (`(int, text)`) requires per-element inflation
via `read_tuple_at_wide` — already in place from P189d.

T1.8a ships as its own arc step (or separate plan); A7 punts if
it slips.

##### A7.1 — Wide-return path

After T1.8a: route tuple returns through Queue's wide-return path.

`route_through_queue` extension:
```rust
match ret_type {
    Type::Tuple(elems) if total_size(elems) <= 64 => RouteWide,
    Type::Tuple(_) => RouteRef,  // > 64 bytes uses ref-return
    // ...
}
```

64-byte cap mirrors phase 4d.A's `INPUT_PRIMITIVE_MAX_BYTES` —
larger tuples fall through to ref-return.

##### A7.2 — Destructure binding

Phase 9d: fused `for (a, b) in pairs par(...) { ... }` binds
elements directly without an intermediate var.  Parser rewrite in
`build_parallel_for_ir`:

```loft
for (a, b) in pairs par(r = work(a, b), 4) { use(r) }
// desugars to:
for _t in pairs par(r = work(_t.0, _t.1), 4) { use(r) }
```

#### Risks

| Risk | Mitigation |
|---|---|
| ~~T1.8a slips beyond plan-06's window~~ | *Retired 2026-05-04* — T1.8a closed via commit `023ca15`. |
| P199 (native E0499 double-borrow on tuple) lands as a blocker for native tuple compilation | Document in PROBLEMS.md P199 follow-up; A7 covers interpreter mode first; native-mode tuple par becomes A7.1.  Note: A1 confirmed P199 also fires in `tests/html_wasm.rs::moros_editor_html_smoke` (`OpCopyRecord(stores, n_build_chunk(stores, …), …)`) and in `bench/11_par/bench.loft` native column (`format_float(&mut s, t_5float_round(stores, …), …)`) — A7's hoist-inner-`&mut stores` fix closes all three simultaneously. |
| Native par dispatch rejects `Type::Tuple` worker returns at `Parallel worker return type 'tuple(...)' (size 16) is not supported` | A7.1 is exactly this fix — the per-canary failure mode is uniform across the 5 return-cases; one Wide-return route closes all of them.  Pre-flight verified the failure shape is independent of element type / arity. |

#### Out of scope

Tuple input through par (`vector<(T, U)>` input) — out of plan-06.

---

### A8 — Collapse Queue / Queue_text / Queue_ref / Queue_narrow into one

**Status:** OPEN
**Effort:** M (~1 session)
**Acceptance test:** `grep -c "fn run_parallel_" src/parallel.rs` ≤ 3
(Discard / Queue / Reduce); all par tests green; ~30 generated test
fixtures regenerated cleanly.

#### Why eighth

With all canaries closed and all return shapes working, the four
Queue dispatchers diverge only in their result buffer shape.  A
trait + generic dispatcher unifies them — the final simplification
PRIORITY.md called for in 3b.2.

#### Design

##### A8.1 — Trait

```rust
// src/parallel.rs
pub trait QueueResult: Send + 'static {
    type Buffer: Send + Default;
    fn push_row(buf: &mut Self::Buffer, raw: &[u8]);
    fn read_row(buf: &Self::Buffer, idx: usize) -> Self;
}
```

##### A8.2 — Implementations

Four impls match the existing per-shape dispatchers:

- `impl QueueResult for i64` — `Buffer = Vec<u64>`, push_row reads
  8B, read_row extracts.
- `impl QueueResult for String` — `Buffer = Vec<String>`, push_row
  reads 16B Str + clones, read_row clones into scratch.
- `impl QueueResult for DbRef` — `Buffer = (Vec<DbRef>, Vec<u16>)`
  (tuple of refs + adopted store_nrs), push_row rebases via
  per-thread allocator, read_row returns the rebased ref.
- `impl QueueResult for NarrowBytes` — `Buffer = (Vec<u8>, u8)`,
  push_row writes `stride` bytes, read_row reads `stride` bytes
  with sign extension as a parameter.

##### A8.3 — Generic dispatcher

```rust
pub fn run_parallel_queue<R: QueueResult>(
    stores: &mut Stores,
    // ... same shape as today's run_parallel_queue ...
) -> R::Buffer {
    parallel_workers(n_threads, |t, ws| {
        let mut buf = R::Buffer::default();
        for row in worker_slice(...) {
            let raw = execute_at_raw(...);
            R::push_row(&mut buf, &raw);
        }
        buf
    }).into_iter().reduce(merge_buffers::<R>).unwrap_or_default()
}
```

##### A8.4 — Native fn collapse

Today: 4 fns (`n_parallel_queue`, `_text`, `_ref`, `_narrow`).
After A8: 1 fn (`n_parallel_queue`) with a `stitch_id` parameter
that selects the impl at the call site:

```rust
fn n_parallel_queue(stores: &mut Stores, stack: &mut DbRef) {
    let stitch_id = pop_u8(stack);
    match stitch_id {
        0 => run_dispatch::<i64>(...),
        1 => run_dispatch::<String>(...),
        2 => run_dispatch::<DbRef>(...),
        3 => run_dispatch::<NarrowBytes>(...),
        _ => panic!(),
    }
}
```

`stitch_id` set by parser based on `route_through_queue`'s outcome.

##### A8.5 — Generated fixture regen

PRIORITY.md flags ~30 generated test fixtures.  Confirm count via
`find tests/generated -name '*.rs' | xargs grep -l n_parallel_queue
| wc -l`.  Regenerate via the existing `cargo run --bin gen_fixtures`
(or whatever the per-test code-gen path is).

#### Risks

| Risk | Mitigation |
|---|---|
| Trait dispatch loses inlining → bench 11 regresses | Mark trait impls `#[inline]`; bench gates the merge.  If perf regresses > 5 %, revisit (likely: monomorphise the four cases via `#[inline(always)]` + `match` on `stitch_id` directly). |
| 30+ fixtures regen takes longer than expected | Run regen as the first commit of the PR; subsequent commits are the dispatcher refactor proper. |
| `String` impl's clone-into-scratch path differs from current `n_parallel_buf_get_text` semantics | Direct port — the impl mirrors today's behaviour; assertion in test for round-trip equivalence. |

#### Out of scope

Reduce trait abstraction.  `par_light` user-surface removal (A9).

---

### A9 — Drop `par_light` user surface; auto-light via fixed-point

**Status:** OPEN
**Effort:** S (~0.5 session, after 5e-style fixed-point lands)
**Effort with 5e:** M (~1 session)
**Acceptance test:** `grep "par_light" default/*.loft tests/*.loft
tests/scripts/*.loft bench/*/*.loft` returns 0 hits.

#### Why ninth

`par_light` is now redundant — auto-light selection from the worker's
effect annotation decides it at compile time.  Phase 5's stdlib
annotation sweep (5a, done) provides the data.  This step closes
the public-API simplification.

#### Design

##### A9.1 — Phase 5e fixed-point analyser

Per [05-auto-light.md:372 §5e](05-auto-light.md), implement
`analyse_purity_fixpoint`:

```rust
fn analyse_purity_fixpoint(data: &Data) -> HashMap<u32, bool> {
    let callers = data.build_caller_index();          // 5b' shipped
    let mut classification = HashMap::new();
    for d_nr in 0..data.definitions() {
        let initial = match data.def(d_nr).purity {
            Purity::Pure => true,
            Purity::Impure | Purity::Unknown => is_user_fn(d_nr, data),
        };
        classification.insert(d_nr, initial);
    }
    let mut worklist: VecDeque<u32> = data.user_fn_d_nrs().into_iter().collect();
    while let Some(d_nr) = worklist.pop_front() {
        if !classification[&d_nr] { continue; }
        if !walk_with_current_classification(&data.def(d_nr).code, &classification) {
            classification.insert(d_nr, false);
            for &caller in callers.get(&d_nr).unwrap_or(&vec![]) {
                if classification[&caller] { worklist.push_back(caller); }
            }
        }
    }
    classification
}
```

Termination: monotonic — classifications only flip `true → false`.
Cost: ≤ 2× walk per user fn.

##### A9.2 — Compiler emit

When parser emits a par call:
```loft
for x in items par(r = worker(x), 4) { ... }
```

Look up `classification[&worker_d_nr]`:
- `true` (light-eligible) → emit `n_parallel_queue` with light flag.
- `false` (full-eligible) → emit `n_parallel_queue` with heavy flag.

The runtime's heavy/light split lives in `clone_for_worker` —
unchanged.  Only the dispatcher chooses the path based on flag.

##### A9.3 — User-surface removal

In `default/01_code.loft`:
```loft
# REMOVED (was: pub fn par_light(...) #rust "n_parallel_for_light")
pub fn par(...)   #rust "n_parallel_queue"
```

The `par_light` keyword in user code becomes a deprecation pass:
parser detects `par_light` invocation, emits warning ("par_light
deprecated; use par"), routes to `par` with the same args.

##### A9.4 — Test corpus migration

`tests/scripts/22-threading.loft` and similar use `par(...)` already.
Confirm via `grep -rn par_light tests/ bench/` — current tree is
clean (the keyword is documented but not used in tests).

#### Risks

| Risk | Mitigation |
|---|---|
| Fixed-point analyser flip-flops on a malformed annotation | Stdlib annotations are `Purity::{Pure, Impure}` + `Unknown`; `Unknown` defaults to `false` (heavy).  No reverse flips. |
| User has a custom fn the analyser classifies wrong | Compile-time error (DESIGN.md D8 — par-safety verdict is binary).  User fixes the fn. |
| `par_light` keyword removal breaks existing programs | Deprecation pass emits a warning, not an error.  Migration is mechanical (search/replace). |
| 5c (Arc-wrap parent stores) is needed for some workers but not in scope | True — file as separate work; A9 ships without it. |

#### Out of scope

Phase 5c Arc-wrap parent stores.  Non-`par_light` deprecation paths.

---

### A10 — Browser parallel via wasm-bindgen-rayon

**Status:** OPEN — strategic showcase track.  Advanced when
validation phases hit natural breakpoints; **does not displace
validation work** (loft-improvement is the first priority per
the user's 2026-05-04 priority statement; A10 is the highest-
priority recruitment deliverable but its severity is S2 —
sequential WASM works, A10 lifts it to parallel — so it doesn't
qualify for the severity-override of "finish plans first").
See `USER_FACING.md § Strategic showcase track` for sequencing.

The named consumer is the user themselves: parallel chunk-mesh
generation for browser-rendered 3D worlds.  The supporting
infrastructure (WebGL bindings, native OpenGL, gallery runner,
`brick-buster.html`) is already shipped.  A10 is the missing
parallelism unlock.

**Effort:** XL (2-3 sessions, split into 3 sub-PRs)
**Acceptance test:** the gallery's `bricks_par.loft` example runs at
≥ 2× single-thread speed in Chrome with `crossOriginIsolated`;
output equivalence vs native par; WebGL goldens unaffected.

#### Why tenth

The streaming model (post-A4) maps cleanly to Web Worker postMessage
transfer.  Phase 8's original sub-phases (8a/8b/8c/8d/8e/8f/8g)
collapse considerably once Concat is gone — no result-vector to
transfer, just per-row Queue results.

A10 splits into 3 sub-PRs; each is testable in isolation.

#### A10.a — Build infrastructure

Add `wasm-bindgen-rayon` dependency.  Configure rayon thread pool
under `#[cfg(target_arch = "wasm32")]`:

```rust
// src/parallel.rs
#[cfg(target_arch = "wasm32")]
fn parallel_workers<R, F>(n_threads: u32, f: F) -> Vec<R>
where R: Send + 'static, F: Fn(usize, &mut WorkerStores) -> R + Sync + Send + 'static
{
    use wasm_bindgen_rayon::init_thread_pool;
    static POOL_INIT: std::sync::Once = std::sync::Once::new();
    POOL_INIT.call_once(|| {
        init_thread_pool(n_threads as usize).await; // top-level await
    });
    // dispatch via rayon's existing scope
    rayon::scope(|s| { ... })
}
```

COOP/COEP headers:
```rust
// web/serve.rs
.header("Cross-Origin-Opener-Policy",   "same-origin")
.header("Cross-Origin-Embedder-Policy", "require-corp")
```

JS shim runtime check:
```js
if (!self.crossOriginIsolated) {
    console.warn("crossOriginIsolated false — falling back to single-threaded");
    return single_threaded_dispatcher();
}
```

Hashed WASM filenames (`loft.${git_sha}.wasm`) prevent stale cache
hits during deployment.

#### A10.b — Per-worker output Stores survive postMessage transfer

Phase 2's `StoreRebase` runs after transfer to rewrite worker-local
`store_nr` fields (DESIGN.md D13).

Workers in browser are SharedArrayBuffer-backed; `Store::new`'s
`Vec<u64>` becomes a slice into shared memory.  The 8d.3 per-thread
allocator (post A2) maps cleanly: the per-thread `Vec<u16>` of slot
indices is per-Web-Worker.

Test infrastructure: `tests/browser/` runs headless under Chrome
(via `wasm-bindgen-test` + browser harness).  Parallelism gates:
- output equivalence vs. native par on the same workload
- ≥ 2× speedup at threads=4 vs. threads=1
- DbRef rebase verification: every result DbRef in the parent
  resolves; no worker-local indices leak.

#### A10.c — Parallelism + visual gates

Bench harness in `scripts/browser/`:
- `bricks_par.loft` example runs under Chrome with timing.
- WebGL goldens (existing `tests/wasm_goldens/`) unaffected.
- Crash recovery: if one Web Worker crashes mid-dispatch, the others
  must complete or panic cleanly — no zombie workers.

#### Risks

| Risk | Mitigation |
|---|---|
| `wasm-bindgen-rayon` requires nightly Rust features | Document the pinned toolchain; lock via `rust-toolchain.toml`. |
| SharedArrayBuffer not available in some hosting setups | A10.a's runtime check fails closed; user gets single-threaded experience instead of a crash. |
| `init_thread_pool` is async (top-level await) → main loft entry must adapt | Wrap entry in async block; rayon init is the only async surface. |
| 3 sub-PRs land in different orders | A10.a (infra) must land first; b and c can land in either order. |

#### Out of scope

Firefox / Safari beyond confirming `crossOriginIsolated` detection.
Browser-pool tuning beyond fixed 4-worker.  Service-worker variants.

---

### A11 — Final cleanup + documentation rewrite

**Status:** OPEN
**Effort:** S (~0.5 session)
**Acceptance test:** all 3 acceptance criteria from "what plan-06
done means" hit; `make ci` clean; D11 type-spectrum tracker shows
every canary closed.

#### Why last

After A1–A10 the runtime is genuinely simple.  This step writes that
down.

#### Design

##### A11.1 — Doc rewrites

Rewrite `doc/claude/THREADING.md` to reflect the streaming-only
model:
- Remove the "3 native fns × 4 getters" table (no longer accurate).
- Add the "1 Stitch trait, 3 policies, 1 dispatcher" diagram.
- Document `par_fold` surface, deprecation of `par_light`,
  auto-light fixed-point.
- Update bench numbers from THREADING.md "Plan-06 phase 0
  baseline" with post-arc measurements.

##### A11.2 — Plan-06 README final pass

Mark all phases done in `doc/claude/plans/06-typed-par/README.md`.
Link to ARC.md as canonical history.  PRIORITY.md gets a final
"superseded" marker (already in place from earlier this session).

##### A11.3 — CHANGELOG entries

`CHANGELOG.md` (user-facing):
```
- Parallel `par(...)` simplified: single user surface, three
  Stitch policies (Discard / Queue / Reduce), automatic
  light/heavy selection.  par_light is deprecated; rewrite as par.
```

`CHANGELOG_TECHNICAL.md` (technical):
```
- plan-06 closed.  Cumulative: 5 → 3 dispatchers, 8 → 4 native
  fns, 2 → 1 user surface, 8 → 0 ignored canaries, ~−1100 LOC
  net.  See doc/claude/plans/06-typed-par/ARC.md for the arc.
```

##### A11.4 — D11 type-spectrum tracker

`doc/claude/plans/06-typed-par/00-baseline-and-bench.md § 0d`:
update each row to show closing arc step.  D11b's "✅ when tuples
land" placeholder retires.

##### A11.5 — Ground-truth checks

Run all three acceptance criteria checks one last time:
```bash
grep -c "fn run_parallel_" src/parallel.rs           # ≤ 3
grep -c "par_light" default/01_code.loft             # 0
grep -c "^#\[ignore" tests/threading_chars.rs        # 0
```

If any returns the wrong number, fix before merging.

#### Risks

None substantive — A11 is mechanical doc work.

#### Out of scope

`par_to_vec` opt-in materialiser — separate arc when needed.

---

## Cumulative shape after each arc step

The "Dispatchers" + "Native fns" columns count distinct functions
in source.  The original projections (rows A1-A2 onwards) assumed
the Queue/Concat consolidation work would land alongside each step;
in practice the variant family grew before A8 collapses it.  See
the **Actual today** row for what `grep` returns now.

| After | Dispatchers | Native fns | User surfaces | Ignored canaries | LOC vs baseline | Bench 11 expected |
|---|---|---|---|---|---|---|
| **Actual today (verified 2026-05-03)** | **8** in src/parallel.rs (`raw`, `text`, `queue_ref`, `int`, `discard`, `queue`, `queue_fn`, `light`) | **13** in src/codegen_runtime.rs (incl. buf_get/buf_drop variants from spine 8d) | 2 (par/par_light) | **4** (all tuple-return; A6 closed 4 in May) | A1's −583 cuts shipped, A6 added +N for hidden-arg infra | 44 ms / 12 ms |
| A1 (projected) | 4 (no heavy Concat; `parallel_light_execute_and_collect` still alive) | 7 | 2 | 8 | −583 | 44 ms / 12 ms |
| A2 (projected) | 4 | 7 | 2 | 8 | −600 | 44 ms / 12 ms |
| A3 (projected) | 5 (Queue_narrow added) | 8 | 2 | 8 | −540 | 44 ms / 12 ms |
| A4 (projected) | 4 (Light retired) | 6 | 2 | 8 | −840 | 44 ms / 12 ms |
| A5 (projected) | 5 (Reduce added) | 7 | 2 | 8 | −690 | 44 ms / 12 ms |
| A6 (DONE 2026-05-01) | 5 | 7 | 2 | **4** | −690 | 44 ms / 12 ms |
| A7 (projected) | 5 | 7 | 2 | 0 | −690 | 44 ms / 12 ms |
| A8 (projected) | **3** (Discard / Queue / Reduce) | 5 | 2 | 0 | −900 | 44 ms / 12 ms |
| A9 (projected) | 3 | 4 | **1** (par only) | 0 | −1000 | 44 ms / 12 ms |
| A10 (projected) | 3 | 4 | 1 + browser | 0 | varies | + browser numbers |
| A11 (projected) | 3 | 4 | 1 | 0 | **−1100** | unchanged |

Three of plan-06's headline numbers — "1 dispatcher" (close: 3
because Stitch policies stay distinct), "1 user surface" (yes),
"~−1500 LOC" (close: −1100 net) — fall within hailing distance of
the original projection.  The shortfall is honest: phase 2's rebase
machinery (~150 LOC) stays as library code for future `par_to_vec`,
and per-thread allocator work added ~50 lines that the original
plan didn't anticipate.

## What this arc explicitly does NOT cover

These items appeared in plan-06's earlier framing but are
**out of scope** for closing plan-06:

- **`par_to_vec` opt-in materialiser** (was phase 11).  Re-add only
  when a real user needs it.  Phase 2's `StoreRebase` machinery
  stays in tree as `#[allow(dead_code)]` library; `par_to_vec`
  becomes a separate ~half-session arc when needed.
- **Phase 5c Arc-wrap parent stores.**  Standalone perf work,
  unrelated to par's runtime simplification.  File as a separate
  PERFORMANCE.md item.
- **Phase 9 tuple input** (`vector<(T, U)>` input).  A7 covers
  tuple *return*; tuple *input* through par is a separate
  ergonomic feature, not a simplification target.
- **P200, P201** — out of scope for ARC; tracked separately.

## Risk register (cross-step)

Honest list of what could derail the arc:

| Risk | Mitigation |
|---|---|
| A2's stress test reveals the per-thread approach is structurally wrong → larger rewrite needed | A2 explicitly carries 3 fix options (a/b/c).  If all three exceed budget, fall back to PRIORITY.md's unified-rebase design. |
| A3's narrow-prim Queue extension surfaces a codegen mismatch like 8c's missing extras-subtract | The 8c lesson is locked in: every new native fn must add **both** push-extras AND subtract-extras entries.  A3.4 adds a regression test that asserts symmetry. |
| A6.c's fn-ref vector storage (4d.A.2 cascade) is bigger than 1 PR's worth | Acceptable to ship the storage redesign as a separate arc step (call it A6.c.0).  Don't bundle. |
| T1.8a tuple-return convention slips | A7 punts to a future arc.  Plan-06 closes without tuple-par support; D11b's placeholder remains. |
| A8 trait dispatch loses inlining → bench 11 regresses by > 5 % | Investigate before merging; monomorphise via `#[inline(always)]` + match-on-stitch-id if needed. |
| A9's 5e fixed-point analyser is more work than budgeted (cycle handling, mutual recursion) | The algorithm in [05-auto-light.md:372](05-auto-light.md) is concrete and bounded.  If implementation surfaces edge cases, document them, ship a conservative version (assume `false` for any cycle), file the optimistic version as follow-up. |
| A10.a's `init_thread_pool` async semantics force broader async refactor in main entry | Wrap top-level entry in an async block; rayon init is the only new async surface.  No deep refactor. |
| Bench 11 regresses by > 5 % at any step | Investigate before merging.  No "we'll fix it in the next step" — that's how complexity creeps in via the back door. |
| P198 (alias-copy regression) turns out to be plan-06 par-safety series breakage | Investigate during A1; if confirmed, file the fix as a new arc step before A2.  Don't proceed with leaks. |

## Cross-references — where to find each design

| Topic | Reference |
|---|---|
| Per-worker output stores | [01-output-store.md](01-output-store.md) |
| StoreRebase machinery | [02-stitch-not-copy.md](02-stitch-not-copy.md), `src/parallel.rs` `StoreRebase` |
| 4e hidden-arg destination | [04-typed-input-output.md:459](04-typed-input-output.md) |
| 4d.C closure-storage redesign | [04d-fn-ref-closure-storage.md](04d-fn-ref-closure-storage.md) |
| 4d follow-ups (P196, etc.) | [04d-followups.md](04d-followups.md) |
| Auto-light fixed-point (5e) | [05-auto-light.md:372](05-auto-light.md) |
| `Value::ParFor` IR shape (D7) | [DESIGN.md:558](DESIGN.md) |
| `is_par_safe` analyser (D8) | [DESIGN.md:598](DESIGN.md) |
| Caller-graph (D12) | [DESIGN.md:869](DESIGN.md) |
| Browser rebase (D13) | [DESIGN.md:912](DESIGN.md), [08-browser-workers.md](08-browser-workers.md) |
| Tuple support / T1.8a | [09-tuple-support.md](09-tuple-support.md) |
| Stream-only no-output-vector | [10-no-output-vector.md](10-no-output-vector.md) |
| Bench numbers / D11 tracker | [00-baseline-and-bench.md](00-baseline-and-bench.md) |
| Spine (historical) | [PRIORITY.md](PRIORITY.md) |
| Phase status (per-topic detail) | [README.md](README.md) |

## Out of band — what to do with PRIORITY.md and the README

- **PRIORITY.md** stays as historical artifact (header already
  added: `> SUPERSEDED by ARC.md`).  Spine steps 1–7 are committed
  there; tail (8d.4 / 8e / 9 / 10) lives in ARC's A1–A11.
- **README.md's status column** updates per arc step.  When a
  README phase fully closes (all sub-items in ARC), strike it.

## How to use this doc going forward

When starting a session on plan-06:

1. Find the lowest-numbered OPEN arc step.
2. Read its scope-locked bullet list and design.  Do not exceed
   scope.
3. Hit the named acceptance test.  No partial-DONE.
4. Update this doc's status field for the step (OPEN → IN-FLIGHT →
   DONE).
5. Single PR; commit; move to the next.

If a step turns out infeasible at its budgeted size, **do not
expand it inline**.  Close what's done, file a new arc step at the
right point, update the status table.  The arc is allowed to grow
forward, never sideways.

---

## Dependency graph

Hard dependencies (must land first) and soft dependencies (could be
parallel but recommended order).  `→` is hard, `⤳` is soft.

```
A1 (commit WIP) ──→ A2 (slot-cap fix) ──→ A3 (narrow-prim Queue) ──→ A4 (retire light)
                                                                          │
                                                                          ↓
                                                                       A5 (Reduce)
                                                                          │
                                                                          ↓
                                                       ┌──────────────── A6 ─────────────────┐
                                                       │      (4 sub-PRs in order a→c→b→d)    │
                                                       │                                      │
                                                  A6.a (vector return)         A6.c (vec-of-fns input)
                                                       │                                      │
                                                       ↓                                      ↓
                                                  A6.b (fn-ref return)         A6.c.0 (4d.C closure storage)
                                                       │                          (extracted if A6.c overflows)
                                                       ↓
                                                  A6.d (keyed-collection return)
                                                       │
                                                       ↓
                                                  A7 (tuple canaries) ⤳ T1.8a (external prerequisite)
                                                       │
                                                       ↓
                                                  A8 (trait dispatch unification)
                                                       │
                                                       ↓
                                                  A9 (drop par_light + 5e fixed-point)
                                                       │
                                                       ↓
                                                  A10 (browser parallel) — 3 sub-PRs (a → b ⤳ c)
                                                       │
                                                       ↓
                                                  A11 (cleanup + docs)
```

**Critical observations:**

- **A1 → A2 → A3 → A4 is a hard chain.**  No reordering possible.
- **A5 (Reduce) can run in parallel with A6** in principle — they
  touch different files.  Recommended sequential to avoid merge
  conflicts in `default/01_code.loft` and `src/native.rs`'s
  `FUNCTIONS:` table.
- **A6 sub-PRs land in order a → c → b → d.**  A6.a establishes the
  `RefHiddenDest` machinery; A6.b reuses it for fn-refs; A6.c is
  independent (4d.A.2 cascade) and lands earlier so A6.b can rely on
  4d.C's closure layout; A6.d is the smallest and lands last.
- **A7 is BLOCKED on T1.8a** (external).  If T1.8a slips, A7 punts —
  the rest of the arc still closes.
- **A8 needs all canaries closed first** so the trait abstraction
  doesn't have to handle special cases.
- **A9 needs A8** because the per-call `stitch_id` lives in the same
  dispatch table that `par_light` removal modifies.
- **A10 can run in parallel with A11**, but A10 should land first
  (A11 documents the final state; A10 changes it).

## A1 starter checklist (next session)

When ready to start A1, the concrete sequence:

```bash
# 1. Confirm working tree state
git status
git diff --stat src/native.rs src/parallel.rs
# Expected: ~-583 LOC across the two files; no other touched files.

# 2. Check the find_problems.sh background run finished (if still queued)
ls -la /tmp/loft_problems.txt
# If exists: cat /tmp/loft_problems.txt and triage.
# Expected failures: only the 2 baseline goldens (25_runtime_panic_builtin,
# 28_runtime_unwrap_none).  Anything else needs investigation.

# 3. Regenerate the 2 goldens
BLESS_BASELINES=1 cargo test --release --test error_messages baselines_are_locked_in
# Or whichever bless flag tests/error_messages.rs accepts; check the file.

# 4. Verify only line-number / pc shifts in the regenerated baselines
git diff tests/error_messages/cases/25_runtime_panic_builtin.loft
git diff tests/error_messages/cases/28_runtime_unwrap_none.loft
# If anything beyond pc / line numbers changed → STOP, investigate.

# 5. P198 gate (alias-copy regression check)
cargo test --release p146_script_95_alias_copy_leak
# If it passes today: ignore for A1.
# If it fails: file a one-paragraph note on PROBLEMS.md P198 with
# current symptom; do NOT block A1 on it.

# 6. CI gate
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --no-default-features
make ci

# 7. Bench gate
cd bench/11_par && make bench
# Verify loft-interp ≤ 46 ms, loft-native ≤ 13 ms (44 ms ± 5 % / 12 ms ± 5 %).

# 8. Commit
git add -p src/native.rs src/parallel.rs
git add tests/error_messages/cases/25_runtime_panic_builtin.loft \
        tests/error_messages/cases/28_runtime_unwrap_none.loft
git commit -m "$(cat <<'EOF'
plan(06-arc-A1): retire heavy parallel_execute_and_collect

Removes ~570 LOC of legacy Concat dispatch:
- parallel_execute_and_collect (~170 LOC)
- run_parallel_direct (~190 LOC)
- run_parallel_ref (~40 LOC)
- DispatchMode + tuple-input helpers gated behind threading feature
- n_parallel_for delegates to n_parallel_for_light (light path
  remains for narrow-prim returns; A3/A4 retire it)

Goldens regenerated for 25_runtime_panic_builtin and
28_runtime_unwrap_none for native.rs line shifts.

See doc/claude/plans/06-typed-par/ARC.md A1 for the design.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"

# 9. Update ARC.md status: A1 OPEN → DONE
```

After A1 commits cleanly, the working tree returns to a known
baseline.  A2 can then branch off it without entanglement.

## Per-step verification commands

Quick lookup for "is this step done?" — paste into any session.

```bash
# A1 — heavy Concat retired
git grep -l "parallel_execute_and_collect" src/        # expect 0 hits
git grep -l "run_parallel_direct" src/                 # expect 0 hits
git grep -l "run_parallel_ref" src/                    # expect 0 hits

# A2 — slot cap removed
grep -n "SLOTS_PER_THREAD" src/database/                # expect 0 hits or const removed
cargo test --test threading par_queue_ref_unbounded_allocations_per_element

# A3 — narrow-prim Queue
grep -n "n_parallel_queue_narrow" src/native.rs        # expect 1 entry
grep -n "par_narrow_buffer_stack" src/database/mod.rs  # expect ≥4 init sites

# A4 — light Concat retired
git grep -l "parallel_light_execute_and_collect" src/  # expect 0 hits
git grep -l "n_parallel_for_light" src/                # expect 0 hits

# A5 — Reduce shipped
grep -n "run_parallel_reduce\|n_parallel_fold" src/parallel.rs src/native.rs
grep -n "par_fold" default/01_code.loft

# A6 — canaries closing
grep -n "^#\[ignore" tests/threading_chars.rs          # 8 → 4 → 0

# A7 — tuple canaries
grep -n "par_tuple_return\|par_tuple_destructure" tests/threading_chars.rs | grep ignore  # expect 0

# A8 — trait dispatch
grep -c "^fn run_parallel_" src/parallel.rs            # expect ≤ 3
grep -n "trait QueueResult" src/parallel.rs            # expect 1 hit

# A9 — par_light gone
grep -n "par_light\|parallel_for_light" default/01_code.loft  # expect 0 hits
grep -rn "par_light" tests/scripts/ bench/             # expect 0 hits
grep -n "analyse_purity_fixpoint" src/scopes.rs        # expect 1 hit

# A10 — browser parallel
grep -n "wasm-bindgen-rayon" Cargo.toml                # expect 1 entry
grep -n "crossOriginIsolated" web/                     # expect runtime check

# A11 — final acceptance gate
grep -c "^fn run_parallel_" src/parallel.rs            # ≤ 3
grep -c "par_light" default/01_code.loft               # 0
grep -c "^#\[ignore" tests/threading_chars.rs          # 0
```

If any check returns the wrong value, the corresponding step isn't
done — regardless of what its status field says.

## Pre-flight checklist (every step)

Before opening any arc-step PR:

- [ ] Branch from `main`, named `plan-06-arc-AN-<short>` (e.g.
      `plan-06-arc-a1-commit-cuts`).
- [ ] Read the step's design section in this file.  Do not skip the
      "Out of scope" line.
- [ ] Check the dependency graph above — confirm prior steps are
      DONE in this arc's status table.
- [ ] Run `make ci` on the unmodified branch tip to confirm a clean
      starting point.
- [ ] Run `bench/11_par/bench.sh` (or equivalent) and record the
      baseline numbers in your scratchpad.
- [ ] If the step touches `src/native.rs`'s `FUNCTIONS:` table, plan
      to update **both** push-extras (state/codegen.rs:1948) and
      subtract-extras (state/codegen.rs:2117) — the 8c regression
      teaches us this drift is silent.
- [ ] If the step adds a new buffer-stack field, plan all 4 init
      sites (Stores::new, Clone, both WorkerStores::new paths).

After the PR's first commit:

- [ ] `make ci` clean.
- [ ] `find_problems.sh --bg` running (or full suite green).
- [ ] Bench 11 ±5 % vs. recorded baseline.
- [ ] Acceptance test from the step passes.
- [ ] ARC.md updated: status field OPEN → IN-FLIGHT → DONE.
- [ ] CHANGELOG_TECHNICAL.md entry recording the step's delta.
- [ ] If the step closes a canary: un-`#[ignore]` it; if it opens a
      new one: file the canary first, fix later.

## Status table

Single source of truth for arc step status.  Update on every commit
that advances a step.

| Step | Status | Effort | PR / commit |
|---|---|---|---|
| A1  | DONE 2026-04-30 | S | b9ad7af + 7153390 |
| A2  | DONE 2026-04-30 | M | 217b3ac |
| A3  | DONE 2026-05-07 (incl. A3.5 + A3.6) | L | f3d0e05 + earlier |
| A4  | DONE 2026-05-07 | S | 0adac9c |
| A5  | DONE 2026-05-07 (interp + native) | M | 8677b32 + this commit |
| A6.a | DONE 2026-05-01 | M | (this branch) |
| A6.b | DONE 2026-05-01 | M | (this branch) |
| A6.c | DONE 2026-05-01 | M | (this branch) |
| A6.d | DONE 2026-05-01 | S | (this branch) |
| A7.1 | OPEN (post-survey) | M | — |
| A7.2 | non-par DONE 2026-05-07 (P235 lexer half + parser); par half OPEN | S-M | this commit |
| A7.3 | lexer DONE 2026-05-07; runtime DONE 2026-05-08 via Plan-14 phase 07 (synthetic-struct routing); canary un-ignored | M | this commit |
| A8  | OPEN | M  | — |
| A9  | OPEN | M  | — |
| A10.a | OPEN | M | — |
| A10.b | OPEN | M | — |
| A10.c | OPEN | M | — |
| A11 | OPEN | S  | — |

**Total budget: ~13–17 sessions** distributed across these steps
(A6 split into 4 + A10 split into 3 → 16 sub-PRs).  At a session a
day, plan-06 closes in 2-3 working weeks.  Stretch budget for
unforeseen rework: + 30 %.

## Final note on scope discipline

Plan-06's earlier two-layer structure (spine + phases) drifted
because every absorbed scope addition felt small in isolation.  The
arc's scope locks are the antidote, but only if they're enforced.

Three tells that scope is creeping:

1. A step's Acceptance test grew between its design and its PR.
2. A step's "Out of scope" list got items removed mid-flight.
3. A step's commits start with "while we're at it…".

If any of these surface during a step, **stop, close the partial
work, file the rest as a new arc step.**  The arc is the contract;
work that doesn't fit goes in the next slot, never inside the
current one.
