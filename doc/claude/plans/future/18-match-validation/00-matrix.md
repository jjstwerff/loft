<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Matrix freeze + harness wiring

**Status: open**

Locks the (subject type × pattern shape) matrix and wires
`tests/match_matrix.rs` to the existing `cross_mode!` harness.

## The frozen matrix

Cell legend: `PASS:test_name` / `FIX:phase` / `CLOSED:reason`.
S5 (tuple subject) is intentionally absent — covered by plan-14.

| | P1 wild | P2 literal | P3 bind | P4 range | P5 or-pattern | P6 `@` bind | P7 guard | P8 null | P9 nested |
|---|---|---|---|---|---|---|---|---|---|
| **S1** scalar | FIX:02 | FIX:02 | FIX:02 | PASS-pre | FIX:01 (hang) | FIX:01 (hang) | FIX:03 | FIX:03 | FIX:04 |
| **S2** text | FIX:02 | PASS-pre | FIX:02 | CLOSED:no-text-range | FIX:01 | FIX:01 | FIX:03 | FIX:03 | CLOSED:tuple-only |
| **S3** plain enum | FIX:02 | FIX:02 | FIX:02 | CLOSED:no-enum-range | FIX:02 | FIX:02 | FIX:03 | FIX:03 | FIX:04 |
| **S4** struct enum | FIX:02 | FIX:02 | PASS-pre | CLOSED:no-variant-range | FIX:02 | FIX:02 | FIX:03 | FIX:03 | FIX:04 |
| **S6** vector | FIX:05 | FIX:05 | FIX:05 | CLOSED:no-vec-range | FIX:05 | FIX:05 | FIX:05 | FIX:05 | FIX:05 |

`PASS-pre` cells passed the pre-flight survey directly.
S5 (tuple) row covered by plan-14 phase 03.

## Cell name convention

`s<S>_p<P>_<sub>` — e.g. `s1_p4_int_range`, `s4_p5_enum_or_three`.

## Per-cell test inventory (subset)

```
s1_p4_int_range
s1_p4_char_range_inclusive
s1_p5_int_or_two_arms              // pre-flight ❌ — fix in 01
s1_p5_int_or_three_arms            // pre-flight ❌ — fix in 01
s1_p6_at_binding_in_or             // pre-flight ❌ — fix in 01
s2_p2_text_literal
s3_p2_enum_variant
s4_p3_struct_enum_capture          // pre-flight ✅ (Circle { radius })
s4_p7_struct_enum_with_guard
```

## Acceptance for phase 00

- New `tests/match_matrix.rs` exists with one smoke test against a
  known-passing pre-flight cell.
- Matrix table fully populated.
- `make ci` green.
- No production change.

## Cross-references

- [README.md](README.md)
- [plan-14 phase 00](../14-tuple-validation/00-matrix.md) — donor template.
