<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 4 — Typed input/output surface

**Status: open**

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

## Out of scope

- Auto-light heuristic (phase 5).
- Cleanup / doc (phase 6).
- Fused for-loop construction (phase 7).
- Heterogeneous worker results.

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
