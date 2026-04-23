<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase B.3 follow-up — par() IR type-lie fix

## Status — 2026-04-23: attempted, fix is insufficient

The 5-line parser fix described below was implemented and tested.
It swaps the failing direction of the matrix but does **not**
eliminate the bug: with the fix in place, `bool_int` regresses
from passing to failing while `int_bool` regresses from failing to
passing.  Root cause below.

**Reverted.**  The right fix is the wrapper-struct design (see
companion doc or re-evaluation).  Keep this file for the post-mortem.

### Why the fix oscillates

`build_parallel_for_ir`'s `result_name` is the user's chosen
variable name (`b` in `par(b = worker(a), N)`).  When two par
blocks in the same function both use `b`, `create_var("b", …)`
returns the **same** `b_var` for both — `Function::add_variable`
is keyed by name and hits the existing-var early-return.  Every
`set_type` call therefore overwrites the previous par's type:

```
[B3 par-type-fix] override b_var=8 from Integer to Float
[B3 par-type-fix] override b_var=8 from Float to Boolean
[B3 par-type-fix] override b_var=8 from Boolean to Integer
```

At end of parsing, `b_var=8` has whichever type was written last.
All par blocks using `b` emit the same `OpPut*` dispatch,
mismatching the push width for every par whose worker's return
type does not match the last writer.

### What "two different b variables" in the IR actually is

The IR dump displays `b(3):integer` and `b(7):integer` (different
parenthesised numbers).  This is **not** two variables — it is
`vars.scope(v)` from one variable, rendered at two use sites.  The
scope is set once (in `scopes.rs::set_scope`), but the display
indexes into the use-site context, not the variable's stored
scope.  `var_nr` is `8` in both cases (verified by instrumentation).

### Implications for the fix

Option 2 (the 5-line `set_type` override) is structurally incapable
of handling multiple par blocks with the same result_name.  Real
fix options:

A. **Give each par a unique `b_var`**: use `create_unique` (adds
   `_N` suffix) and register "b" as a parse-time alias only within
   the par body.  ~30 lines; touches name-aliasing machinery.
B. **Wrapper-struct design** (b3-par-wrapper.md if written):
   synthesize a per-worker struct type; `b = elem.value` is a
   proper typed field read.  Structurally avoids name-sharing
   because the struct's `value` field has a single, unambiguous
   type.  ~100 lines; cleaner overall.

Option B is the recommended path.  Option A is a narrower fix but
still preserves the fragile "b may be the same var across pars"
design.

## Context

After the atomic B.3 bundle landed (`06a8d14` on develop —
function-entry `OpReserveFrame(frame_hwm)`, slot-move deleted,
positional-init / push-then-`OpPut*` first-Set dispatch), two wrap
tests regress:

- `tests/wrap.rs::threading` (driver: `tests/docs/19-threading.loft`)
- `tests/wrap.rs::script_threading` (driver: `tests/scripts/22-threading.loft`)

The failure is a `SIGSEGV` in `get_vector` — a `DbRef` with a
store_nr in the thousands is dereferenced when the second par loop
reads its result vector.  Reproducible via
`target/debug/loft --interpret <file.loft>` (the CLI default is
`--native`, which routes around the bytecode and therefore masks the
bug — this is why the regression was missed in interactive testing).

## Failure matrix — 2 consecutive pars by return-type

| 1st \ 2nd | int | float | single | bool | enum | text | struct |
|---|---|---|---|---|---|---|---|
| **int** (8B)     | ✓ | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ |
| **float** (8B)   | ✓ | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ |
| **single** (4B)  | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ | ✗ |
| **bool** (1B)    | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ | ✗ |
| **enum** (1B)    | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ | ✗ |
| **text** (ptr)   | ✗ | ✗ | ✗ |  · | ✗ | ✓ | ✗ |
| **struct**       | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✓ |

- Diagonal (same-type pairs) all pass.
- Single par scenarios all pass (10 / 10 sampled).
- Primitive pairs pass when **1st par's element size ≥ 2nd par's element size**; fail otherwise.

## Root cause

`src/parser/collections.rs::build_parallel_for_ir` chooses the type
of `b_var` — the loop-body result variable — via:

```rust
let b_type = if matches!(ret_type, Type::Unknown(_)) {
    I32.clone()                          // first pass when ret_type is Unknown
} else if fn_d_nr == u32::MAX {
    Type::Unknown(u32::MAX)              // first pass when worker not yet resolved
} else if let Type::Text(_) = ret_type {
    Type::Text(Vec::new())
} else {
    ret_type.clone()                     // second pass — the "correct" path
};
let b_var = self.create_var(result_name, &b_type);
```

`Function::add_variable` updates an already-created variable's type
only if the existing type `is_unknown()`:

```rust
// src/variables/mod.rs:791
if let Some(nr) = self.names.get(name) {
    if self.variables[*nr as usize].type_def.is_unknown() {
        self.variables[*nr as usize].type_def = type_def.clone();
    }
    return *nr;
}
```

Two interactions bite:

1. For a `Type::Unknown` ret_type on the first pass, the branch uses
   `I32.clone()` (8 B integer).  On the second pass, ret_type is
   known (e.g. `Type::Boolean`, 1 B).  `add_variable` finds the
   existing var with `type_def == Integer`, sees it is **not**
   unknown, and declines to update.  `b_var` stays `Integer` even
   though the loop-body value is a boolean / single / text.
2. For a non-`Unknown` first-pass ret_type that later changes
   (rare), the same issue applies.

The in-loop `b = get_call` IR then has the shape:
`Set(b_var:Integer, OpEqInt(OpGetByte(…), 1))` where the value
actually produces 1 byte.

Under **HEAD before B.3**, `gen_set_first_at_tos`'s slot-move
coincidentally absorbed the mismatch: it re-pointed `b_var.stack_pos`
to the current TOS and let `generate(value)` push its 1 byte there —
whatever the declared width was, the push landed "at b's slot."  A
subsequent `OpFreeStack(0, 1)` at loop-body end dropped the byte.
No `OpPut*` was emitted.

Under the **atomic B.3 bundle**, `gen_set_first_at_tos`'s fall-through
dispatches `OpPut*` by `b_var`'s declared type:

```rust
match stack.function.tp(v).clone() {
    Type::Integer(_) => stack.add_op("OpPutInt", self),   // pops 8 B
    Type::Boolean   => stack.add_op("OpPutBool", self),   // pops 1 B
    …
}
```

For `b_var:Integer` whose value pushed only 1 B, `OpPutInt` pops 8 B,
underflowing the eval stack by 7 B.  Per iteration the stack drifts
downward; after N iterations it has eaten into the `_par_results_N`
slot region, corrupting the DbRef.  The next `OpGetVector` on that
`_par_results_N` reads a garbage `store_nr` and SIGSEGVs.

The ordering asymmetry in the matrix (int_bool ✗ but bool_int ✓)
maps onto the direction of drift: an 8-B `OpPut*` over a 1-B push
underflows (destructive); a 1-B `OpPut*` over an 8-B push overpops
(discards intended bytes but does not corrupt the result-vector
pointer slot below).

## Design — parser-side fix

**Principle.** The IR should not lie about the type of `b_var`.
`build_parallel_for_ir` knows what the worker returns on the
second pass, and `Function::set_type` exists and is safe to call.

### Change (single edit, localised)

In `src/parser/collections.rs::build_parallel_for_ir` (around
codegen.rs:1274):

```rust
let b_type = if fn_d_nr == u32::MAX {
    Type::Unknown(u32::MAX)
} else if let Type::Text(_) = ret_type {
    Type::Text(Vec::new())
} else {
    ret_type.clone()
};
let b_var = self.create_var(result_name, &b_type);
self.vars.defined(b_var);

// B.3 follow-up: if `b_var` was created on a prior pass with a
// stale placeholder (`I32` under the dropped `Unknown`-check
// branch, or any non-matching type from a first-pass guess), force
// it to the resolved `b_type` now.  `add_variable`'s
// "update-if-unknown" guard protects against legitimate
// narrowing but misses this case; `set_type` is the explicit
// override for exactly this shape.
if fn_d_nr != u32::MAX && self.vars.tp(b_var) != &b_type {
    self.vars.set_type(b_var, b_type.clone());
}
```

This:

- Drops the `if matches!(ret_type, Type::Unknown(_)) { I32.clone() }`
  branch entirely — it only existed to paper over a first-pass
  codegen issue that the explicit `Type::Unknown(u32::MAX)`
  placeholder already handles.
- Adds an unconditional second-pass correction via `set_type` so
  that a variable whose first-pass type was a guess (any type, not
  just Unknown) gets updated to the actual `ret_type`.
- Preserves the Text-dep stripping and first-pass placeholder logic
  unchanged.

### `Function::set_type` visibility

`set_type` lives in `src/variables/mod.rs:1268` and is already
`pub`.  No new API surface.

### `is_unknown()` semantics

Keep `add_variable`'s "update-if-unknown" guard intact — it is
still correct for most call sites.  The par() call is special
because the first pass operates with an intentionally
not-yet-resolved worker return type; it needs the second-pass
override.

## Tests

Add a focused test file that exercises each primitive-pair
scenario from the matrix.  Suggested location:
`tests/scripts/22-threading.loft` already covers many; extend with
one cumulative fixture that pairs the sizes inside one `main`:

```loft
// 22-threading.loft addition (or new 22a-par-type-matrix.loft):
fn w_int   (r: const S) -> integer { r.v * 2 }
fn w_bool  (r: const S) -> boolean { r.v > 0 }
fn w_float (r: const S) -> float   { r.v as float }
fn w_single(r: const S) -> single  { r.v as single }
fn w_enum  (r: const S) -> G       { if r.v > 0 { C } else { A } }

fn main() {
    q = L { }; q.items +=[S{v:10}];
    // Two pars of every size combination, each checking the accumulator.
    // Fails under B.3 without the parser fix; passes with it.
    …
}
```

Verification steps:

1. `cargo test --test wrap script_threading` — passes.
2. `cargo test --test wrap threading` — passes (docs/19-threading.loft).
3. `target/debug/loft --interpret tests/scripts/22-threading.loft` — exits 0.
4. `./scripts/find_problems.sh --bg --wait` — full suite green.

Regression guards:

- Add the matrix fixture as `tests/expressions.rs::par_type_matrix`
  (or similar), asserting the accumulators for every pair — catches
  future drift.

## Risks

- **Second-pass `set_type` on a variable whose type was set by an
  earlier passing-pass unrelated mechanism** — mitigated by guarding
  on `fn_d_nr != u32::MAX` (only overrides after worker resolution)
  and comparing against the computed `b_type` (no-op if already
  matches).  This is narrower than blanket "always update on second
  pass."
- **`Type::Unknown(u32::MAX)` leaks to codegen if `fn_d_nr == u32::MAX`
  persists past parsing** — it does not: the caller of
  `build_parallel_for_ir` errors out earlier (`par_for_d_nr == u32::MAX`
  short-circuit at collections.rs:1311) when the worker cannot be
  resolved, and emits `Value::Null` instead of reaching codegen.

## Non-goals

- Fixing HEAD behaviour (before B.3).  HEAD was buggy-but-hidden;
  the fix lands on top of B.3 and must leave HEAD untouched (we are
  not reverting the atomic bundle).
- Changing `add_variable`'s update-if-unknown rule globally.  Other
  call sites rely on it.
- Rewriting `build_parallel_for_ir`'s type-inference to use a proper
  two-pass shape (deferred — the `set_type` override is sufficient).

## Estimated effort

**30–60 minutes** to land the 5-line parser change + one matrix
fixture + full `find_problems.sh --bg --wait` run.  The fix is
genuinely localised once the type-lie pattern is understood.

## Related

- Atomic B.3 bundle: `06a8d14` on develop.
- Slot-move deletion: `src/state/codegen.rs::gen_set_first_at_tos`
  (removed `set_stack_pos(v, stack.position)` preamble).
- Fall-through `OpPut*` dispatch: same file, post-2h.h block.
- V1's implicit "slot-move covers type lies" was the last
  compatibility surface that the atomic bundle relied on **not**
  being needed — this fix retires that silent reliance.
