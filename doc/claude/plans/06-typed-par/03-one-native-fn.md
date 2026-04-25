<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 3 — Collapse to one polymorphic native fn

**Status: open**

## Goal

Replace the three native dispatch fns (`n_parallel_for_native`,
`n_parallel_for_text_native`, `n_parallel_for_ref_native`) with one
polymorphic `n_parallel_native` fn parameterised by the `Stitch`
policy from DESIGN.md D1.  Drop the four primitive getters
(`parallel_get_int`, `parallel_get_long`, `parallel_get_float`,
`parallel_get_bool`) — output already lives in the result Store
after phase 2; users access it via normal vector ops.

## What collapses

| Today | After phase 3 |
|---|---|
| `n_parallel_for_native(input, elem_size, return_size, threads, fn)` | `n_parallel_native(input, threads, fn, stitch)` |
| `n_parallel_for_text_native(...)` | (same as above) |
| `n_parallel_for_ref_native(...)` | (same as above) |
| `parallel_get_int(result, i)` | `result[i]` (existing vector indexing) |
| `parallel_get_long(result, i)` | `result[i]` |
| `parallel_get_float(result, i)` | `result[i]` |
| `parallel_get_bool(result, i)` | `result[i]` |
| `OpParallelFor`, `OpParallelForText`, `OpParallelForRef` opcodes | `OpParallel(stitch_id)` opcode |

Net: 7 native fns + 3 opcodes retire.  One fn + one opcode lands.
The opcode payload encodes the stitch policy:
- 0x00 = ConcatLegacy (transitional — phases 3a..4b)
- 0x01 = Discard
- 0x02 = Reduce (reserved for future par_fold)
- 0x03 = Queue

Phase 3 ships the **transitional** `Stitch` enum from DESIGN.md
D1a — `ConcatLegacy { elem_size: u8, ret_size: u8 }` carries
size info that the runtime still needs because the typed surface
(phase 4) has not landed.  Phase 4c renames `ConcatLegacy` →
`Concat` and drops the payload (D1b).

**Why two names, not "phase 3 has Concat with sizes; phase 4
removes the sizes".**  Mid-phase contradictions in the enum
discriminant (same name, different payload) make `match` arms
flip-flop across phases and force readers of the source to track
"which phase am I looking at".  Two distinct names — `ConcatLegacy`
during 3a–4b, `Concat` from 4c — are unambiguous: a function
matching `Stitch::ConcatLegacy` is provably from the transitional
window and can be deleted by `grep` after phase 4c.

`element_size` and `return_size` integer args disappear at the
runtime call shape — the worker fn's own signature carries them
(the type checker already validates this; phase 4 lifts the surface
to use the type system end-to-end).  For phase 3, the existing
parser code computes them per-call and embeds them in the
`ConcatLegacy` payload at codegen time (not as runtime args).

## Per-commit landing plan

### 3a — `Stitch` enum + dispatcher skeleton

- Add `Stitch` enum to `src/parallel.rs` with `ConcatLegacy {
  elem_size: u8, ret_size: u8 }`, `Discard`, `Reduce { fold_fn:
  u32 }`, `Queue { capacity: u32 }`.
- Add `n_parallel_native(input, threads, fn, stitch) -> DbRef` that
  inner-dispatches on `stitch`:
  - `ConcatLegacy { ret_size }` → today's `run_parallel_direct` /
    `_text` / `_ref` (via a runtime check on `ret_size`-flavour for
    now: 0 = primitive, sentinel for text, sentinel for reference).
  - `Discard` / `Reduce` / `Queue` → `unimplemented!()` (return
    error; phase 7 fills `Queue`; phase 3e fills `Reduce`).
- Codegen emits the new `OpParallel(0x00)` opcode for existing
  parser sites; old opcodes still work in parallel for one commit
  to validate the new path.

Acceptance: phase-0 characterisation suite passes through both old
and new opcodes (parser flag toggles which path; suite runs twice,
once per).

### 3b — retire old native fns + opcodes

- Delete `n_parallel_for_native`, `_text_native`, `_ref_native` from
  `src/codegen_runtime.rs`.
- Delete `OpParallelFor`, `OpParallelForText`, `OpParallelForRef`
  from `default/01_code.loft` and `src/fill.rs`.
- Update `default/01_code.loft::parallel_for(...)` to lower to the
  new `OpParallel(0x00)` opcode.  Same lowering for
  `parallel_for_int` and `parallel_for_light` (until phase 4
  retires them).

Acceptance: phase-0 suite passes; opcode count in `src/fill.rs`
drops by 3 (verified by `grep '^Op' src/fill.rs | wc -l` before /
after).

### 3c — retire `parallel_get_*` accessors

- Workers' results live in the result Store (post phase 1+2).  The
  result Store IS a `vector<U>`.  User code that called
  `parallel_get_int(result, i)` should call `result[i]`.
- All four `parallel_get_*` declarations removed from
  `default/01_code.loft`.
- Existing in-tree call sites (mostly in
  `tests/scripts/22-threading.loft` and a few `lib/` examples)
  rewritten to use vector indexing.
- A parser diagnostic `parallel_get_int has been removed; use
  result[i] instead` fires for any external code that hand-typed
  the call (same as phase 7c handles `par_light`).

Acceptance: phase-0 suite still passes after the call-site
rewrite; no `parallel_get_*` references in `default/`, `lib/`,
`tests/`, or `doc/`.

### 3d — codegen-time embedding (always lands in 3d, not deferred)

After 3a–3c, the `ConcatLegacy { elem_size, ret_size }` payload
is the only runtime variation left.  Phase 3d ensures those sizes
are **embedded at codegen time** from the worker fn's signature
(via `data.rs::element_size` on the worker's argument and return
types) rather than re-computed per call.  This is local — no
typed-surface dependency.  Phase 4c later renames `ConcatLegacy`
→ `Concat` and drops the payload entirely once the typed surface
makes the sizes derivable at *runtime* from the fn's `Type`.

Phase 3d **always lands in this phase**, regardless of phase 4's
schedule.  The earlier "conditional on phase 4" wording was
incorrect — embedding sizes at codegen time is correct under both
the legacy and typed surfaces.

### 3e — `Stitch::Reduce` runtime

Implements the third stitch policy from DESIGN.md D1.  `Reduce`
takes a fold fn `(U, T) -> U` and an initial value of type `U`,
runs the fold in parallel across the input slices, then combines
per-worker partial results into one final `U`.  The user surface
arrives in phase 7e (`par_fold(...)`); phase 3e ships only the
runtime mechanism.

**Per-worker partial:** each worker allocates a 1-element output
store of type `U` initialised to `init`.  The worker's loop body
becomes:

```
acc = init                            // worker-local register
for input_idx in worker_slice {
    elem = input_store[input_idx]
    acc = fold_fn(acc, elem)
}
output_store.set(0, acc)              // store the partial
```

**Main-thread combine:** after join, the parent thread folds the
per-worker partials into the final result, starting from `init`:

```
result = init
for w in 0..worker_count {
    partial = worker_outputs[w].get(0)
    result = fold_fn(result, partial)
}
```

The combine pass walks per-worker output stores in worker-id
order, not input-index order — assumes the fold operation is
associative.  Document explicitly: `par_fold` requires `fold` to
form a monoid with `init`.  Non-associative folds produce
implementation-defined results.

**Memory:** allocates `threads` 1-element output stores instead of
the `2 × threads`-slot bounded queue (Queue) or per-worker
N-element output stores (Concat).  Smallest of the four policies.

**Cost:** one fold-fn call per input element (same as serial),
plus `threads − 1` combine calls on join.  For pure-arithmetic
folds, the per-element cost dominates and parallel speedup is
near-linear.

**Touch points:**
- `src/parallel.rs::run_parallel_reduce`: new fn, ~80 lines.
  Mirrors `run_parallel_direct`'s slice partitioning but with a
  scalar accumulator per worker instead of a result-vector slice.
- `n_parallel_native`'s match adds a new arm dispatching to
  `run_parallel_reduce` when stitch is `Reduce`.
- WASM single-threaded fallback: `run_parallel_reduce_sequential`
  in the `#[cfg(not(feature = "threading"))]` block; identical
  output to threaded path because the fold is monoidal.

**Tests** (in 3e's commit, not waiting for 7e):
- `tests/issues.rs::par_phase3e_reduce_sum` — `Stitch::Reduce`
  with integer addition; assert correct result, single-element
  output store per worker, no queue allocation.
- `tests/issues.rs::par_phase3e_reduce_text_concat` — text fold;
  assert worker-id order is preserved (deterministic for any
  ordering choice).
- `tests/issues.rs::par_phase3e_reduce_empty_input` — `init`
  returned unchanged.
- `tests/issues.rs::par_phase3e_reduce_single_thread` — worker
  count clamped to 1; partial == result.

Acceptance: phase-0 suite still passes (Reduce isn't user-visible
yet — phase 7e wires the surface); the new fixtures pass; bench
of `Stitch::Reduce` on bench-1's workload (1 M `i64` summed) shows
a wall-clock floor matching today's `par(...)` then `sum()` two-
pass approach **minus** the result-vector allocation cost.

## Cross-cutting interactions

| DESIGN.md item | Phase 3 contribution |
|---|---|
| D1 Stitch policy | Lands as runtime enum.  Phase 7 fills the `Queue` variant; future plans (par_fold) fill `Reduce` |
| D7 `Value::ParFor` IR | Not introduced yet — phase 3 still uses `Value::Call(parallel_for, ...)`.  The new opcode payload includes a `Stitch` discriminant so codegen can route either IR to one runtime |

## Test fixtures

Existing phase-0 suite passes byte-for-byte (this is required by
the global plan rule).

New fixtures:

| Fixture | Asserts |
|---|---|
| `tests/issues.rs::par_phase3_one_opcode` | `LOFT_LOG=static` dump shows `OpParallel` for every par call site, regardless of return type.  No `OpParallelForText` or `OpParallelForRef` in the dump. |
| `tests/issues.rs::par_phase3_get_diagnostics` | A test program calling `parallel_get_int(...)` after phase 3c receives the parser diagnostic `parallel_get_int has been removed; use result[i] instead` |
| `tests/issues.rs::par_phase3_native_fn_count` | A grep over `src/codegen_runtime.rs` confirms `n_parallel_for_*` are gone and `n_parallel_native` is the only par entry point |

## Acceptance criteria

- Phase-0 characterisation suite passes byte-for-byte.
- Opcode count in `src/fill.rs` drops by 3 (verified by hash, not
  text — the file is auto-generated).
- Native-fn count in `src/codegen_runtime.rs` for the par section
  drops from 3 + 4 to 1 (verified by grep).
- `default/01_code.loft` size drops by ~80 lines (the four
  `parallel_get_*` declarations + their `#native` annotations + the
  three `OpParallelFor*` opcode declarations).
- Bench-1 / bench-2 / bench-3 within ±5 % of phase 2 baseline.

## Risks

| Risk | Mitigation |
|---|---|
| Existing user code calls `parallel_get_int(...)` directly | The user surface for that fn was always documented as "internal"; tests/lib uses are renamed in phase 3c.  External users get the deprecation diagnostic with a one-token fix. |
| Codegen needs to inspect worker fn signature to compute `ret_size` for `ConcatLegacy` payload | Already does (today's parser computes `return_size: integer` in `parser/builtins.rs::parse_parallel_for`).  Phase 3 just moves the computation from runtime arg to opcode payload. |
| The `ConcatLegacy { elem_size, ret_size }` payload duplicates info available from the worker fn type | Yes — phase 4c retires this duplication by renaming the variant to `Concat` (no payload).  Phase 3 accepts the transitional redundancy with explicit `Legacy` naming so the deletion target is greppable. |
| Removing 3 opcodes + 7 native fns in one phase risks bytecode-format breakage | The bytecode format is internal — `.loftc` cache was retired in plan-01 (integer-i64 migration).  Phase 3 is a free internal rearrangement; every `make` rebuilds bytecode from source.  Test-fixture golden dumps under `tests/dumps/*.txt` are regenerated per build via `LOFT_LOG=static`, so no migration step is needed for them.  See DESIGN.md D1's "Binary-format change" note. |

## Out of scope

- Typed input/output surface (phase 4).
- Auto-light analyser (phase 5).
- Cleanup / doc rewrites (phase 6).
- Fused for-loop construction (phase 7).

## Hand-off to phase 4

After phase 3 lands:
- One native fn (`n_parallel_native`).
- One opcode (`OpParallel`).
- Four `Stitch` variants (`ConcatLegacy` actually used in 3a–3d;
  `Reduce` lands in 3e; `Discard` and `Queue` reserved for phase 7).
- The runtime arg shape `(input, threads, fn, stitch)`; the
  `ConcatLegacy { elem_size, ret_size }` payload carries
  codegen-embedded sizes (no longer runtime args).

Phase 4 lifts the surface to type-system input/output: the worker
fn's `Type` carries `T → U`, and phase 4c renames `ConcatLegacy`
→ `Concat` (DESIGN.md D1b), dropping the payload entirely.
