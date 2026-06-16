<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Matrix freeze + harness wiring

**Status: open**

Locks the (variant payload × dispatch context) matrix and wires
`tests/struct_enum_matrix.rs` to the existing `cross_mode!` harness.

## The frozen matrix

| | C1 `is` | C2 is-capture | C3 match | C4 variant method | C5 enum method | C6 store + match | C7 return as enum |
|---|---|---|---|---|---|---|---|
| **V0** no fields | PASS-pre | CLOSED:no-fields | FIX:01 | CLOSED:no-fields | FIX:03 | FIX:01 | FIX:01 |
| **V1** scalar field | PASS-pre | PASS-pre | PASS-pre | PASS-pre | FIX:03 | FIX:01 | FIX:01 |
| **V2** text field | FIX:02 | FIX:02 | FIX:02 | FIX:02 | FIX:03 | FIX:02 | FIX:02 |
| **V3** Reference field | FIX:05 | FIX:05 | FIX:05 | FIX:05 | FIX:05 | FIX:05 | FIX:05 |
| **V4** multi-field mixed | FIX:04 | FIX:04 | FIX:04 | FIX:04 | FIX:04 | FIX:04 | FIX:04 |
| **V5** tuple field | FIX:04 (depends on @PLAN14 phase 05) | FIX:04 | FIX:04 | FIX:04 | FIX:04 | FIX:04 | FIX:04 |
| **V6** nested struct-enum | FIX:05 | FIX:05 | FIX:05 | FIX:05 | FIX:05 | FIX:05 | FIX:05 |

## Cell name convention

`v<V>_c<C>_<sub>` — e.g. `v1_c4_circle_area`, `v2_c5_tag_classify`.

## Per-cell test inventory (subset)

```
v0_c1_pure_tag_is_check
v1_c1_circle_is_check               // pre-flight ✅
v1_c2_rect_is_capture               // pre-flight ✅
v1_c3_match_with_capture            // pre-flight ✅
v1_c4_variant_method                // pre-flight ✅
v1_c5_enum_method_via_dot           // pre-flight ❌ — fix in 03
v1_c5_enum_method_via_free_fn       // alternate form
v2_c4_text_payload_method
v4_c3_multi_field_match
v5_c1_tuple_payload_is_check        // depends on plan-14 phase 05
```

## Acceptance for phase 00

- `tests/struct_enum_matrix.rs` exists with one smoke test.
- Matrix table fully populated.
- `make ci` green.

## Cross-references

- [README.md](README.md)
- [@PLAN14 phase 04](../finished/14-tuple-validation/04-references.md)
- [@PLAN14 phase 05](../finished/14-tuple-validation/05-struct-field.md)
