<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Matrix freeze + harness wiring

**Status: open**

Locks the (collection type × operation) matrix and wires
`tests/collection_matrix.rs` to the existing `cross_mode!` harness.

## The frozen matrix

| | O1 insert | O2 lookup | O3 remove | O4 iterate | O5 cleanup | O6 resize |
|---|---|---|---|---|---|---|
| **K1** hash | FIX:02 | FIX:02 | FIX:02 | CLOSED:no-iterate | FIX:02 | FIX:02 |
| **K2** sorted | FIX:03 | FIX:03 | FIX:03 | FIX:03 | FIX:01 (panic) | FIX:03 |
| **K3** index | FIX:03 | FIX:03 | FIX:03 | FIX:03 | FIX:01 (panic) | FIX:03 |
| **K4** spacial | FIX:04 | FIX:04 | FIX:04 | FIX:04 | FIX:04 | FIX:04 |

Value-element sub-axis (E1–E4) is applied per-cell within each
phase; not expanded into the matrix to keep the table readable.

## Cell name convention

`k<K>_o<O>_<elem>_<sub>` — e.g. `k2_o1_int_insert`,
`k1_o2_text_value_lookup`, `k2_o5_int_cleanup_no_panic`.

## Per-cell test inventory (subset)

```
k1_o1_hash_insert_int_value          // E1
k1_o2_hash_lookup_text_value         // E2
k1_o3_hash_remove_via_null_assign
k1_o5_hash_cleanup_clean             // pin: no panic on scope exit
k2_o1_sorted_insert_int              // pre-flight ✅ for body
k2_o4_sorted_iterate_ascending       // pre-flight ✅ for output
k2_o5_sorted_cleanup_no_panic        // pre-flight ❌ — fix in 01
k3_o4_index_iterate_multi_key
k3_o5_index_cleanup_no_panic         // pre-flight ❌ — fix in 01
```

## Acceptance for phase 00

- `tests/collection_matrix.rs` exists with one smoke test against a
  known-working pre-flight body (e.g. `k1_o1_hash_insert_int_value`
  with skipped O5 — the cleanup panic is a separate cell).
- Matrix table fully populated.
- `make ci` green.

## Cross-references

- [README.md](README.md)
- `src/database/structures.rs:609` — pre-flight panic site.
