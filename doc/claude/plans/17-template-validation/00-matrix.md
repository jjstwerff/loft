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

Phase 02+ (FIX cells) lands in subsequent commits.

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
| **U1** body op | PASS:u1_b0_no_bound_unused_t | PASS:u1_b1o_ordered_compare | PASS:u1_b1e_equatable_check | PASS:u1_b1a_addable_sum_three (`+` only — Addable does not include `-`) | FIX:03 (++ inference gap) | PASS:u2_b1ao_addable_ordered_min_sum (covers B2 op-mix) | FIX:04 | PASS:harness_smoke_template (B4 dbl) | FIX:05 |
| **U2** T return | PASS:u2_b0_no_bound_identity_int | PASS:u2_b1o_ordered_max_int | PASS:u2_b1e_equatable_pick | PASS:u2_b1ao_addable_ordered_min_sum (Addable+Ordered) | FIX:03 | PASS:u2_b2_multibound_eq_or_gt_int (cmp_eq) | FIX:04 | PASS:u1_b4_addable_dbl_float | FIX:05 |
| **U3** tuple-of-T return | FIX:02 | FIX:02 (parser gap) | FIX:02 | FIX:02 | FIX:03+02 | FIX:02 | FIX:04+02 | FIX:05+02 | FIX:05 |
| **U4** struct field of T | CLOSED:no-generic-structs (verify in 01) | CLOSED | CLOSED | CLOSED | CLOSED | CLOSED | CLOSED | CLOSED | CLOSED |
| **U5** vector-of-T input | FIX:03 | FIX:01 | FIX:01 | FIX:01 | FIX:03 (built-in satisfaction) | FIX:04 | FIX:04 | FIX:05 | FIX:05 |
| **U6** vector-of-T output | FIX:03 | FIX:03 | FIX:03 | FIX:03 | FIX:03 | FIX:03 | FIX:04 | FIX:05 | FIX:05 |
| **U7** multi-T tuple-return arity ≥3 | FIX:02 | FIX:02 | FIX:02 | FIX:02 | FIX:03+02 | FIX:02 | FIX:02 | FIX:05+02 | FIX:05 |
| **U8** T inside format / inline expr | FIX:02 (tuple-elem-in-format) | FIX:01 | FIX:01 | PASS-pre | FIX:03 | FIX:01 | FIX:04 | FIX:05 | FIX:05 |

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
