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
- 0x00 = Concat
- 0x01 = Discard
- 0x02 = Reduce (reserved for future par_fold)
- 0x03 = Queue

`element_size` and `return_size` integer args disappear at the
runtime call shape — the worker fn's own signature carries them
(the type checker already validates this; phase 4 lifts the surface
to use the type system end-to-end).  For phase 3, the existing
parser code computes them per-call and stores them in the new
`Stitch::Concat { elem_size, ret_size }` payload.

Wait — the spec above says `Stitch::Concat` has no payload.  Reread
DESIGN.md D1.

The DESIGN.md D1 enum is the **target** shape after phase 4.  In
phase 3 (transitional), `Stitch::Concat` carries `elem_size` /
`ret_size` for backward compat with the today-shape runtime; phase 4
moves those out by inferring them from the worker fn's type
signature.

## Per-commit landing plan

### 3a — `Stitch` enum + dispatcher skeleton

- Add `Stitch` enum to `src/parallel.rs` with `Concat { elem_size,
  ret_size }`, `Discard`, `Reduce { combine }`, `Queue { capacity }`.
- Add `n_parallel_native(input, threads, fn, stitch) -> DbRef` that
  inner-dispatches on `stitch`:
  - `Concat` → today's `run_parallel_direct` / `_text` / `_ref` (via
    a runtime check on `ret_size`-flavour for now).
  - `Discard` / `Reduce` / `Queue` → `unimplemented!()` (return
    error; phase 7 fills `Queue`; future phases fill the others).
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

### 3d — opcode payload simplification

After the three return-type branches are unified, the single
`Stitch::Concat` variant's `elem_size` and `ret_size` fields are
the only runtime variation left.  Phase 3d audits whether they can
be inferred at codegen time from the worker fn's signature instead
of being passed at runtime.

Phase 3d is **conditional** on phase 4's typed surface — if
phase 4 is landing in the same milestone window, phase 3d folds
into 4a (typed input/output).  If phase 4 slips, phase 3d ships
the local optimisation: codegen reads the worker fn's `Type` from
`Data` and embeds the sizes in the opcode at codegen time, removing
the runtime arg entirely.

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
| Codegen needs to inspect worker fn signature to compute `ret_size` for `Stitch::Concat` payload | Already does (today's parser computes `return_size: integer` in `parser/builtins.rs::parse_parallel_for`).  Phase 3 just moves the computation from runtime arg to opcode payload. |
| The `Stitch::Concat { elem_size, ret_size }` payload duplicates info available from the worker fn type | Yes — phase 4 retires this duplication.  Phase 3 accepts the temporary redundancy as a transitional state |
| Removing 3 opcodes + 7 native fns in one phase risks bytecode-format breakage | The bytecode format is internal — `.loftc` cache was retired in plan-01 (integer-i64 migration).  Phase 3 is a free internal rearrangement |

## Out of scope

- Typed input/output surface (phase 4).
- Auto-light analyser (phase 5).
- Cleanup / doc rewrites (phase 6).
- Fused for-loop construction (phase 7).

## Hand-off to phase 4

After phase 3 lands:
- One native fn (`n_parallel_native`).
- One opcode (`OpParallel`).
- Four `Stitch` variants (only `Concat` actually used in 3a–3d).
- The runtime arg shape `(input, threads, fn, stitch)` still has
  `Stitch::Concat { elem_size, ret_size }` carrying redundant size
  info.

Phase 4 lifts the surface to type-system input/output: the worker
fn's `Type` carries `T → U`, and the runtime stops passing
`elem_size` / `ret_size` because the type system has them.
