<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Matrix freeze + harness wiring

**Status: DONE 2026-05-09.**  `tests/template_matrix.rs` shipped
with smoke + 4 PASS-pre cells (`harness_smoke_template`,
`u1_b4_addable_dbl_float`, `u2_b0_no_bound_identity_int`,
`u2_b1o_ordered_max_int`, `u2_b2_multibound_eq_or_gt_int`) — all
pass under both interp and native via the existing
`cross_mode!` harness (no new harness code).

**Phase 01 (basic body + T-return baseline) DONE 2026-05-09.**
Six new cells added (`u1_b0_no_bound_unused_t`,
`u1_b1o_ordered_compare`, `u1_b1e_equatable_check`,
`u1_b1a_addable_sum_three`, `u2_b1e_equatable_pick`,
`u2_b1ao_addable_ordered_min_sum`).  All 11 cells pass under
both backends; the README's predicted "Most B1×U1 cells today
fail (type inference gap)" turned out to already be closed by
the (B) parser fix that landed 2026-05-04 (commit referenced in
plan-17 README).  The phase 01 work was therefore purely cell
coverage, not bug fixing.

Finding (cosmetic, not blocking): stdlib `Addable` declares
only `+`, not `-` (binary subtraction).  `-` requires either a
concrete type or a hypothetical Subtractable bound that doesn't
exist.  Any user expectation that `Addable` covers `-` (e.g.,
the README claim "(+, -)" in plan-17's earlier draft) is
inaccurate — the actual interface in `default/01_code.loft` is
`+` only.  Documented inline in `u1_b1a_addable_sum_three`'s
cell comment.

**Phase 02 (tuple-of-T returns + multi-arity + inline) DONE
2026-05-09.**  Nine new cells added (U3, U7, U8 rows).  The
pre-flight predicted parser breakage on tuple-of-T returns;
the (A) fix in `Parser::substitute_type` (closed 2026-05-04)
already handled the uniform-T canonical case.  Phase 02
verified the fix across multiple bound shapes + arities +
nested + destructure + inline-format paths.

**Phase 02 surfaced 2 new P-issues** while writing cells:
- **P237** — bound-supplied operator INSIDE a tuple constructor
  element (`(x + x, 1)`, `(a, a + b)`, etc.) breaks both
  backends.  Interp SIGSEGVs / silent garbage; native rustc
  E0308 (T not substituted in operator-method calls inside
  tuple constructors).  Workaround: hoist the operator to a
  local first.  Reproducer: /tmp/p_followups/p237_*.loft.
- **P238** — uniform `(T, T)` tuple-return with `T = text`
  fails native compilation (`expected &str, found String`).
  Interp works; integer + float monomorphisations work.
  Reproducer: /tmp/p_followups/p238_*.loft.

Cells covering the P237/P238 shapes stay OUT of the binary
until those P-issues close — the cross_mode harness asserts
both backends pass.  The U3.B1.A cell uses the workaround
form (hoisted local) so the regression net catches future
breakage of THAT path.

**Phase 03 (Printable + vector-of-T) DONE 2026-05-09.**  Three
new cells added: `u2_b1p_printable_show`, `u1_b1p_printable_concat`
(verifies P208 closure + (C) built-in satisfaction across
integer/float/boolean/text monomorphisations), and
`u6_b0_no_bound_passthrough` (vector pass-through — no
iteration).  All 23 cells pass under both backends.

**Phase 03 surfaced 1 new P-issue** plus confirmed P237 in
broader scope:
- **P239** — `for x in v` over `vector<T>` in a generic
  function (any bound or no bound, x used or unused) crashes
  both backends.  Interp SIGSEGVs on opcode dispatch; native
  rejects with rustc E0610 (`i64` doesn't have field `.rec`)
  because the iterator codegen always emits a DbRef-shaped
  null check regardless of T.  No clean workaround — caller
  must specialise the function or take a `fn(T)` callback.
  Blocks ALL U5 (vector-of-T input) cells and U6 cells with
  internal iteration.  Reproducer: /tmp/p_followups/p239_*.loft.
- **P237 in broader scope** — confirmed `(x.to_text(), "x")`
  triggers the same bug as `(x + x, 1)`; the bug is about ANY
  bound-supplied method/operator INSIDE a tuple constructor
  element.  PROBLEMS.md P237 entry updated 2026-05-09 to
  reflect the broader scope.

**Phase 04 (multi-bound + user-defined interface) DONE
2026-05-09.**  Two new cells added: `u2_b3_user_iface_shape_area`
(Shape interface with Circle/Square impls + T return),
`u1_b3_user_iface_showable_concat` (Showable interface body
op + text concat, verifies user-iface dispatch + multi-impl
+ body-op composition).  All 25 cells pass under both
backends.  Multi-bound (B2) coverage was already complete
from earlier phases (`u2_b2_multibound_eq_or_gt_int`,
`u3_b2_addable_ordered_pair`, `u2_b1ao_addable_ordered_min_sum`).

**Phase 04 surfaced 1 new P-issue:**
- **P240** — bounded-generic body computing 2+ bound-supplied
  operator results into locals + returning them in a tuple
  constructor produces wrong values on one backend.  Direction
  flips depending on whether the body has intervening
  side-effects (e.g. `println!`).  Reproducer: `classify<T:
  Ordered>(a, b) -> (integer, integer) { lt = if a<b ...; gt
  = if a>b ...; (lt, gt) }` invoked with `classify(3, 5)` —
  interp returns `(0, 0)` WRONG; native returns `(1, 0)`
  correct.  Adding `println!` in body before tuple flips:
  interp correct, native wrong.  Single-local + literal works
  on both.  Concrete-type version works on both.  No clean
  workaround.  Reproducer: /tmp/p_followups/p240_*.loft.

**Phase 05 (op-sugar nested + two-T generics) DONE 2026-05-09.**
One new cell added: `u8_b4_addable_to_text_format` — verifies
the workaround for P242 (op-sugar result hoisted to local then
explicit `to_text()` for format-string interpolation).  All 26
cells pass under both backends.

**Phase 05 verified two feature gaps + surfaced 2 new
P-issues:**
- **B5 two-T generics** — confirmed FEATURE GAP (parse-error
  on `<A: B1, B: B2>`, `<A, B>`, and `where`-clauses).  Per
  the plan-17 README: "scheduled feature work, not bug-fix
  gating."  All B5 cells flip from FIX:05 to
  CLOSED:feature-gap-two-T.
- **U4 generic structs** — verified FEATURE GAP (`struct
  Box<T> { val: T }` parse-errors).  All U4 cells flip from
  CLOSED:no-generic-structs (verify in 01) to CLOSED:no-
  generic-structs (verified).
- **P241** (new) — building or pushing into `vector<T>`
  inside a generic function crashes both backends.  Sibling of
  P239 (consume side); P241 is the construct/push side.  Same
  root cause family.  Reproducer:
  /tmp/p_followups/p241_*.loft.
- **P242** (new) — format-string interpolation of a `T`
  variable in a generic body fails on both backends.
  Workaround: explicit `to_text()` first, then interpolate
  the resulting text.  Reproducer:
  /tmp/p_followups/p242_*.loft.

The matrix is now fully populated:
- 26 PASS cells across U1, U2, U3, U6, U7, U8 × B0–B4.
- BLOCKED cells reference the relevant P-issue (P237 / P239 /
  P241 / P242).
- CLOSED cells reference the feature gap (no-generic-structs
  / two-T).

Phase 06 (matrix freeze + doc) and the plan-17 closeout land
in the next commit.

## Goal

Lock the bounded-generic / interface validation matrix and wire
`tests/template_matrix.rs` to the existing `cross_mode!` harness
from `tests/common/cross_mode.rs`.  No new harness code; same
plan-14 phase-00 infrastructure.

## The frozen matrix

Cell legend: `PASS:test_name` / `FIX:phase` / `CLOSED:reason`.

The B1 column is split into four sub-columns (one per stdlib
bound) since each interacts differently with the type system.

| | B0 (no bound) | B1.O Ordered | B1.E Equatable | B1.A Addable | B1.P Printable | B2 multi-bound | B3 user-iface | B4 op-sugar | B5 two-T |
|---|---|---|---|---|---|---|---|---|---|
| **U1** body op | PASS:u1_b0_no_bound_unused_t | PASS:u1_b1o_ordered_compare | PASS:u1_b1e_equatable_check | PASS:u1_b1a_addable_sum_three (`+` only — Addable does not include `-`) | PASS:u1_b1p_printable_concat (P208 closure verified) | PASS:u2_b1ao_addable_ordered_min_sum (covers B2 op-mix) | PASS:u1_b3_user_iface_showable_concat | PASS:harness_smoke_template (B4 dbl) | CLOSED:feature-gap-two-T |
| **U2** T return | PASS:u2_b0_no_bound_identity_int | PASS:u2_b1o_ordered_max_int | PASS:u2_b1e_equatable_pick | PASS:u2_b1ao_addable_ordered_min_sum (Addable+Ordered) | PASS:u2_b1p_printable_show (covers (C) closure) | PASS:u2_b2_multibound_eq_or_gt_int (cmp_eq) | PASS:u2_b3_user_iface_shape_area | PASS:u1_b4_addable_dbl_float | CLOSED:feature-gap-two-T |
| **U3** tuple-of-T return | PASS:u3_b0_no_bound_pair_int | PASS:u3_b1o_ordered_min_max + u3_b1o_ordered_destructure | PASS:u3_b1e_equatable_pair_when_eq | PASS:u3_b1a_addable_pair_with_hoisted_sum (inline form blocked by P237) | BLOCKED:P237 (Printable's to_text inside tuple element same root cause) | PASS:u3_b2_addable_ordered_pair | BLOCKED:P237 (user-iface method inside tuple element) | (covered by U1.B4 smoke + U3.B1.A hoist form) | CLOSED:feature-gap-two-T |
| **U4** struct field of T | CLOSED:no-generic-structs (verified — `struct Box<T>` parse-errors) | CLOSED | CLOSED | CLOSED | CLOSED | CLOSED | CLOSED | CLOSED | CLOSED |
| **U5** vector-of-T input | BLOCKED:P239 | BLOCKED:P239 | BLOCKED:P239 | BLOCKED:P239 | BLOCKED:P239 | BLOCKED:P239 | BLOCKED:P239 | BLOCKED:P239 | BLOCKED:P239 + CLOSED:feature-gap-two-T |
| **U6** vector-of-T output | PASS:u6_b0_no_bound_passthrough (no-iter pass-through; iter + push forms blocked by P239 / P241) | BLOCKED:P239+P241 | BLOCKED:P239+P241 | BLOCKED:P239+P241 | BLOCKED:P239+P241 | BLOCKED:P239+P241 | BLOCKED:P239+P241 | BLOCKED:P241 | CLOSED:feature-gap-two-T |
| **U7** multi-T tuple-return arity ≥3 | (covered by U3) | PASS:u7_b1o_ordered_triple_arity_three + u7_b1o_ordered_nested_pair | (covered by U3) | (covered by U3) | BLOCKED:P237 | (covered by U3) | (covered by U2.B3) | (covered) | CLOSED:feature-gap-two-T |
| **U8** T inside format / inline expr | BLOCKED:P242 (`{x}` for x:T) | PASS:u8_b1o_ordered_inline_format (call-result `.0`) | BLOCKED:P242 | PASS:u8_b4_addable_to_text_format (workaround: explicit `to_text()`) | BLOCKED:P242 | BLOCKED:P242 | BLOCKED:P242 | PASS:u8_b4_addable_to_text_format | CLOSED:feature-gap-two-T |

`PASS-pre` = a pre-flight survey passed; the cell test still gets
written (so the matrix is uniform and the regression net catches
later breakage), but no production code change is needed.

## Harness reuse

```rust
// tests/template_matrix.rs (new binary)

mod common;

cross_mode!(my_template_cell, r#"
    fn dbl<T: Addable>(x: T) -> T { x + x }
    fn test() {
        a = dbl(7);
        b = dbl(3.5);
        print("{a}|{b}\n");
        assert(a == 14, "u1_b4_int");
        assert(b == 7.0, "u1_b4_float");
    }
"#);
```

**No new harness code.**  `cross_mode!` already marks every cell
`#[ignore]` so default `cargo test` skips them.  Run with:

```bash
cargo test --release --test template_matrix -- --ignored
# single cell:
cargo test --release --test template_matrix -- --ignored u1_b4_int
```

## Cell name convention

Cell names use the template-specific prefix `u<U>_b<B>[<sub>]_<sub>`
so they don't collide with plan-14 (`e<E>_d<D>`), plan-15
(`c<C>_d<D>`), or plan-16 (`y<Y>_x<X>`).  Examples:

```
u1_b0_no_bound_int
u1_b1o_ordered_int                   // Ordered
u1_b1e_equatable_int                 // Equatable
u1_b1a_addable_int                   // Addable (PASS-pre baseline)
u1_b1p_printable_concat_text         // Printable + ++ — pre-flight failure
u2_b0_no_bound_return
u2_b1p_printable_return_text
u3_b1o_tuple_of_T_return             // pre-flight failure
u3_b2_multi_bound_tuple_return
u4_b0_struct_field_of_T              // CLOSED if loft has no generic structs
u5_b1p_printable_vector_consumer     // pre-flight failure (built-in sat)
u6_b1a_addable_vector_producer
u7_b1o_tuple_arity_3_of_T_return
u8_b1p_format_string_inline
u_dynamic_dispatch_rejected           // CLOSED — INTERFACES.md non-goal
u_inheritance_rejected                // CLOSED — INTERFACES.md non-goal
```

A CLOSED cell uses a manual `#[test]` with `code!(...).error(...)`
in `tests/parse_errors.rs` rather than `cross_mode!` — same pattern
as plans 14/15/16.

## Pre-flight summary (5 quick tests, 2026-05-04)

These results inform the FIX-cell allocation in the matrix above:

```
PASS — u1_b4_addable_dbl              (// dbl<T: Addable>(x: T) -> T)
PASS — u1_b2_ordered_eq_compare       (// <T: Ordered + Equatable> three-way compare)
FAIL — u3_b1o_ordered_tuple_return    (parser: tuple .0 / destructure / format)
FAIL — u1_b1p_printable_concat        (type inference: ++ on bound's to_text result)
FAIL — u5_b1p_printable_vector        (satisfaction: integer vs Printable)
```

## Acceptance for phase 00

- New file `tests/template_matrix.rs` exists with one smoke test
  exercising the harness against `u1_b4_addable_dbl` (a known-
  passing pre-flight cell).
- Matrix table in this file fully populated — no "TBD" cells.
- README phase ladder matches matrix.
- `make ci` green.
- No production code change.

## Risks

| Risk | Mitigation |
|---|---|
| The B1 sub-axis (one column per stdlib bound) makes the matrix wider than plans 14/15/16 | Sub-axis is needed — each bound interacts differently with the type system (Addable enables `+`, Printable adds `to_text`, etc.).  Plan tooling is identical; only the cell count grows.  Heavy by default mitigates the runtime cost. |
| Cells where a T-usage permutation isn't legal in loft (e.g. struct generics if not supported) | Each suspicious cell starts with a one-line `code!(...)` probe in `tests/parse_errors.rs` to learn the actual reject diagnostic; the cell flips to CLOSED with the diagnostic recorded.  No production code added until the language gap is understood. |
| Phase 03 satisfaction-of-built-ins decision drags | Decision section in phase 03 is filled in before any code lands; reviewer sign-off + 24h timer.  Either choice is correct; no-decision is the failure mode. |

## Cross-references

- [README.md](README.md) — full matrix; this phase fixes its shape.
- `tests/common/cross_mode.rs` — shared harness.
- [plan-14 phase 00](../14-tuple-validation/00-matrix.md) — donor
  template.
- [INTERFACES.md](../../INTERFACES.md) — bound design.
- `default/01_code.loft` — stdlib interfaces and built-in
  satisfaction (currently Ordered / Equatable / Addable; Printable
  status disputed — see phase 03).
