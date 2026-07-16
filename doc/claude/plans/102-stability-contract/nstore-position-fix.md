<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design — N-Store diagnostic position: anchor to the stored value, not the cursor

**Status: DESIGN.**  Fixes the null-flow (@PLN25/@PLN102) N-Store warning reporting the
WRONG source position — it points at the *next function definition* instead of the
offending expression.  Surfaced during the library warning-cleanup
([lib-warning-cleanup.md](../lib-warning-cleanup.md) C1): gridmesh `chunk_loc`'s nullable
`v % cs` tail was reported at `chunk_of:172`, costing ~15 probes to locate.

## The defect (grounded in the code)

`n_store_violation` (`src/parser/mod.rs:2107`) emits every N-Store diagnostic via
`diagnostic!(self.lexer, …)` (lines ~2165 / ~2197), which reads **`self.lexer`'s CURRENT
cursor position**.  That is correct only when the check runs while the stored value's
tokens are still under the cursor.  It is WRONG for a check DEFERRED to **block
finalization**, where the block is fully parsed and the cursor has advanced to the
block's closing `}` — i.e. onto (or past) the *following* definition.

**Empirically pinned (throwaway probes, `--interpret`):**

| store shape | example | reported | correct? |
|---|---|---|---|
| implicit tail expression | `fn f()->int { … v[i] }` | the NEXT `fn`'s line | ✗ block-final |
| tail call's argument | `fn f()->int { sink(v[i]) }` | the `}` line | ✗ block-final |
| explicit mid-block `return` | `if c { return v[i]; }` | the `return v[i]` | ✓ cursor there |
| non-tail argument | `r = sink(v[i]);` | the call | ✓ cursor there |
| assignment / field | `x = v[i];` / `T{f: v[i]}` | the store site | ✓ cursor there |

The seven call sites (`grep n_store_violation`): `objects.rs:2827` field · `operators.rs:1567`
`?? return` · `control.rs:1178` **implicit tail** · `control.rs:10704` explicit `return`
· `expressions.rs:2413` assignment · `mod.rs:2140` element (recursion) · `mod.rs:6214`
argument.  Only the two **block-finalization** ones misreport.

## The invariant (one sentence)

> An N-Store diagnostic anchors to the STORED VALUE's own source span — the value node
> the caller already holds — never to `self.lexer`'s cursor, which is correct only while
> the value's tokens are under it and is wrong for any store CHECKED AT BLOCK
> FINALIZATION (the implicit tail expression and a tail call's argument), where the
> cursor has moved to the block's `}`.

The mid-block callers (explicit `return`, non-tail argument, assignment, field) already
sit on the value when they check, so `self.lexer` is right for them — they are **not**
touched.  Only the deferred callers gain an explicit anchor.

## Failure paths (write them down — where the invariant is earned)

1. **The value node is not span-wrapped.**  A bare-var tail (`{ … r }`) or a
   transform-rewritten tail may have lost its `Value::Span`, so `span_pos()` returns
   `None`.  → **Fallback to the current function's position**
   (`self.data.def(self.context).position()`, always available) — still the RIGHT
   function, never the next one.  NEVER fall back to `self.lexer` for a deferred check.
2. **A deferred caller is not migrated.**  It keeps the old (wrong) `self.lexer` position
   — no NEW breakage, because the change is additive (see mechanism); caught by that
   shape's Step-0 matrix cell.
3. **The anchor points at the whole tail, not the nullable sub-expression.**  Acceptable:
   the mid-block arg/field callers already point at the whole value expression, and any
   position inside the right function is a categorical win over the next function.
4. **A test pins the old (wrong) line:col.**  The one Rust test that touches a
   return-value N-Store (`tests/runtime_warnings.rs:1029`) asserts the MESSAGE only, not
   the position; the `tests/testing.rs` tolerated list matches message prefixes.  So the
   ripple is near-zero — but Step-0 records every position-asserting N-Store test to be sure.

## Re-assertion sites — count N (the brittleness tell)

Exactly **N = 2** deferred call sites must supply the value position: `control.rs:1178`
(implicit tail) and `mod.rs:6214` (tail argument).  Omission is **not silent breakage** —
it leaves the OLD behavior (additive design), and each shape has a Step-0 matrix cell, so
a missed migration shows as a still-wrong cell, not a wrong result elsewhere.  There is no
`N > 1` spray of a NEW invariant across producers: the correct callers are left exactly as
they are (the over-broad guard below).

## Mechanism — additive optional anchor (no `self.lexer` mutation)

All machinery already exists — this is wiring, not new infrastructure:

- `diagnostic_at!` (`src/diagnostics.rs:168`) → `lexer.pos_diagnostic(level, pos, msg)`
  (`src/lexer.rs:544`) — emit at an explicit `&Position`.
- `Value::span_pos()` (`src/data.rs:681`) → `Option<&Position>` off a `Value::Span`.
- `Definition::position()` (`src/data.rs:2829`) — the function's own position (fallback).

Add one parameter:

```rust
fn n_store_violation(&mut self, value_tp: &Type, target_tp: &Type,
                     what: &str, at: Option<Position>) -> bool
```

`at == None` → the current `diagnostic!(self.lexer, …)` path, **byte-identical** for the
five correct callers.  `at == Some(p)` → `diagnostic_at!(self.lexer, &p, …)`.  The tuple
recursion (`mod.rs:2140`) passes `at` **through** so each element inherits the anchor.
(Owned `Position`, not a borrow, to sidestep the `&self.data`/`&mut self.lexer` conflict.)

**No `Type::…` change, no lexer-position save/restore, no signature change to the diagnostic
macros.**  The blast radius is: 1 fn signature + `None` at 4 unchanged call sites +
`Some(anchor)` at 2 call sites.

## The over-unification guard (design-protocol §4)

The tempting over-reach: *"just make every caller pass the value position — one uniform
rule."*  **Probed and rejected.**  The mid-block callers were measured CORRECT (explicit
`return` → `2:26`, non-tail arg → `3:18`, assignment → value site).  Rewriting them to
pass an anchor would (a) shift their reported column for zero UX gain and (b) ripple every
position-asserting test — pure blast radius to repair a fault that lives only in the two
deferred paths.  The invariant is scoped to *deferred* checks precisely because the probe
falsified the "all callers are wrong" claim.  Universal stays the wrong default here.

The cleanest remaining claim — *"the value node always has a usable span"* — is probed at
Step 0 by the **bare-var tail cell** (`fn f()->int { r }` where `r = v[i]`): if `span_pos()`
is `None` there, the function-position fallback (path 1) is what carries it, and the cell
proves the fallback lands in the right function.

## Small safe steps (each contained; a parse-time diagnostic → backend-independent, but
run through the test harness which is where the warn mode fires)

| # | Step | Verify |
|---|---|---|
| 0 | **Boundary matrix (throwaway).**  One tiny `fn first(){…nullable…} fn second(){}` per shape: implicit-tail-var, implicit-tail-if, tail-arg, explicit-mid-return, non-tail-arg, assignment, field.  Hand-record CURRENT line (wrong for the two deferred; right for the rest) and TARGET (inside `first`).  Grep every test that asserts an N-Store `:line:col`. | spec + test list recorded |
| 1 | **Add `at: Option<Position>` (INERT).**  Thread it into the two `diagnostic!` sites (DN3 + DN1 branches) as `diagnostic_at!` when `Some`; pass `at` through the tuple recursion; `None` at ALL 7 call sites. | builds; whole suite byte-identical (None everywhere) |
| 2 | **Implicit tail-return (the reported bug).**  `control.rs:1178`: `let at = l[last].span_pos().cloned().or_else(|| Some(self.data.def(self.context).position().clone())); self.n_store_violation(t, result, "the return value", at)`. | gridmesh `chunk_loc` + the repro report inside `first`, not `second`; `runtime_warnings.rs` still green (message-only) |
| 3 | **Tail argument.**  `mod.rs:6214`: anchor to the argument value's span (the arg `Value` in `process_call_args`), same fallback.  Confirm the NON-tail arg cell is UNCHANGED (still correct) and the tail-arg cell moves inside `first`. | both arg cells correct; no other cell moves |
| 4 | **Prove the correct callers untouched.**  Re-run Step-0: explicit-return / non-tail-arg / assignment / field cells report the SAME line:col as before Step 1. | byte-identical for the 5 `None` callers |
| 5 | **Graduate + docs.**  Add the two fixed shapes to `tests/runtime_warnings.rs` (assert the reported line is the offending function's, not the next), note the fix in `lib-warning-cleanup.md`, drop the "workaround" note from memory. | `make ci` |

## What this deliberately does NOT do

- Does **not** re-anchor the mid-block callers (correct today; the guard above).
- Does **not** pin the exact nullable SUB-expression — the whole stored value's span is
  the anchor, matching the arg/field callers' existing granularity.
- Does **not** touch the warn/error split, the DN1/DN3 logic, or `nullflow_enabled()` —
  position only.
