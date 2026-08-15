<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN31 — Keyed collection validation (hash / sorted / index / spacial)

**Status: CLOSED 2026-07-09 — superseded / nothing left to fix.**  The
motivating panic (`index out of bounds` at `src/database/structures.rs:609`) no
longer exists — line 609 is now `vector_set_size` (relocation-safe) and no such
panic remains in the file.  The three live keyed types (hash / sorted / index)
are validated cross-mode by the `tests/scripts/1xx-keyed-*.loft` corpus (119,
120, 122, 126–137) + `62-index-range-queries.loft`,
`129-sorted-index-field-deepcopy.loft`.  The 4th type, **spacial**, is
unimplemented and fully owned by the open **@PLN48** (spacial index) — its
validation folds there.  The matrix in [00-matrix.md](00-matrix.md) is preserved
as historical record.

The 2026-05-04 pre-flight survey saw a runtime panic
(`index out of bounds: the len is 66 but the index is 65535` at
`src/database/structures.rs:609`) on basic `sorted<>` and
`index<>` cleanup, both backends.  A follow-up run-30-times
hammer on the same inputs produced 0 panics, on an identical
binary, with no intervening source change.  The panic is either
flaky (timing/memory-layout-dependent) or required some
intermediate session state I can't identify post-hoc.

**Trigger to unpause:** any user-reported `index out of bounds`
panic at `src/database/structures.rs:609`, or a deterministic
reproducer that surfaces during @PLAN15/16/17/18/19 cell runs.
The matrix in [00-matrix.md](00-matrix.md) is preserved as
documentation; the phase ladder is correct if/when the bug
re-surfaces.

## Goal

Validate that loft's keyed-collection family — `hash<T[key]>`,
`sorted<T[field]>`, `index<T[fields]>`, `spacial<T[…]>` — round-trip
every meaningful **value-element type** through every meaningful
**operation**, with **interp/native byte-identical stdout**.

## Why now — pre-flight survey

3 quick tests (2026-05-04), 2 panics:

| Shape | Result |
|---|---|
| `hash<Entry[key]>` insert/lookup/remove with `key` field name | ❌ caught by parser ("reserved for hash iteration") — documented restriction, not a bug. **Retired 2026-08-15 (loft#932):** the pseudo-field it reserved never existed, it covered `hash` but not `sorted`/`index` or any local, and the issue-83 panic it stood in for no longer reproduces. The shape runs on both backends — `tests/scripts/932-key-is-an-ordinary-field-name.loft` |
| `sorted<Score[value]>` insert + iterate | ✅ output correct, then **`thread 'main' panicked at src/database/structures.rs:609:25: index out of bounds: the len is 66 but the index is 65535`** on cleanup |
| `index<Item[name, -price]>` insert + iterate | ✅ output correct, same `index out of bounds` panic on cleanup |

**67% bug rate** but likely a single root cause — the panic message
is identical between the `sorted<>` and `index<>` cases, suggesting
a shared cleanup/free path that mishandles the keyed-collection
backing store (the `65535 = u16::MAX` value points at a null-store
sentinel being dereferenced).

Both interp and native panic.  This is a serious production bug —
basic keyed-collection usage panics on scope exit despite producing
correct output.

## The matrix

Three nested axes (collapsed to keep the cell count manageable).

### Axis 1 — collection type

| ID | Type | Notes |
|---|---|---|
| K1 | **hash** | Hash table; can't iterate directly |
| K2 | **sorted** | Sorted vector |
| K3 | **index** | B-tree index, multi-key, asc/desc |
| K4 | **spacial** | 2D/3D spatial query collection |

### Axis 2 — operation

| ID | Op | Notes |
|---|---|---|
| O1 | **Insert** | `c += [item]` or `c += item` |
| O2 | **Lookup by key** | `c[key]`; returns null if absent |
| O3 | **Remove** | `c[key] = null` |
| O4 | **Iterate-aggregate** (sorted/index/spacial only) | `for x in c { … }` |
| O5 | **Cleanup on scope exit** | The active panic surface |
| O6 | **Dynamic resize** (rebalance / hash grow) | Stress shape |

### Axis 3 — value-element type (sub-axis)

Cells specialise the value type when relevant.  Default value type
is single-field struct (matching the loft-write skill restriction
that hash values must be struct fields).

- E1: scalar key + scalar value (e.g. `Entry { id: integer, count: integer }`)
- E2: text-field value
- E3: Reference-field value
- E4: tuple-field value (cross-cuts @PLAN14 phase 05)

## Phase layout

| Phase | Outcome |
|---|---|
| [00 — matrix freeze + harness wiring](00-matrix.md) | Frozen matrix; `tests/collection_matrix.rs` binary; smoke test against a known-passing cell.  No production change. |
| 01 — fix the cleanup panic (O5 across K2/K3) | **Active risk; phase 01 closes the pre-flight finding.**  Investigate `src/database/structures.rs:609`; the panic message + 65535 sentinel suggests a missing-store guard.  Likely surfaces a P-issue and a small fix. |
| 02 — hash basics (K1 × O1–O3, O5) | Insert / lookup / remove / cleanup for hash collections.  K1 has no O4 (hash can't iterate). |
| 03 — sorted/index basics (K2/K3 × O1–O5) | Iterate cells must produce sorted output; cross-mode equivalence on iteration order. |
| 04 — spacial (K4) | Spatial queries; less-tested surface. |
| 05 — value-type sub-axis (E1–E4) | Specialise cells with text/Reference/tuple value types.  Cross-cuts @PLAN14 phase 05. |
| 06 — freeze + doc | Update STDLIB.md collection sections, loft-write skill restrictions table, PLANNING.md. |

## Pre-flight gate

If phase 01 closes the cleanup panic via a small fix and the rest
of phases 02-04 pass mostly green, close phase 05 as deferred
(value-type sub-axis becomes documentation).

## Acceptance for the whole plan

- The pre-flight cleanup panic (`index out of bounds` in
  `src/database/structures.rs:609`) closed with a P-issue and a
  regression test pinning the fix.
- Matrix in [00-matrix.md](00-matrix.md) fully populated.
- Every PASS cell has a `cross_mode!` test in
  `tests/collection_matrix.rs`.
- Loft-write skill restrictions table matches the matrix's CLOSED
  cells exactly.

## Out of scope

- **Vector** as a "keyed" collection — `vector<T>` with `[i]` access
  is a sequence, not a keyed collection; covered elsewhere.
- **Hash iteration semantics** — explicitly non-goal per the
  loft-write skill ("Hash cannot be iterated directly").  CLOSED
  row in the matrix.
- **Nested keyed collections** (hash-of-vectors, sorted-of-hashes)
  — niche; revisit only if a real consumer surfaces.

## Cross-references

- [STDLIB.md § Collections](../../STDLIB.md) — API reference.
- [LOFT.md § Composite types](../../LOFT.md) — language reference.
- `src/database/structures.rs:609` — the panic site (active risk).
- [DESIGN.md § keyed collections](../../DESIGN.md) — architecture.
- [@PLAN14 phase 05](../finished/14-tuple-validation/05-struct-field.md) —
  tuple-value-type cross-reference.
