<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Nested-vector representation — scope investigation (loft/loft #475)

**Identity:** driven by **loft/loft issue #475** (a bug), NOT a `loft-lang/plans`
`@PLN` issue — the `475` there is a bug number; do not mint a `@PLN475` plan dir
from it. This is a bug-scoped investigation: probes + scope map, feeding a fix.

**Status:** Stage A complete — the full scope is mapped on the `main` baseline
(`probes/scope.sh`, run on `--interpret` AND `--native`). Fix direction chosen
(below). Fix not yet landed on `main`.

## The bug in one line

A `vector<vector<T>>` element is a rec-id handle, but the element **stride** is
computed inconsistently across codegen paths — the codebase's *de-facto* stride is
**8** (inner-scalar collapse: `known_type(vector<T>) = T`), and two paths deviate
(**struct-field construction = 4**, **iteration = 16**), so those two mismatch the
stride-8 index/read → crash / silent-wrong. It is NOT "all nested vectors broken."

## Scope map (main baseline — `probes/scope.sh`, both backends identical)

| cluster | shape | result |
|---|---|---|
| A inner width | local, all of int/float/single/char/bool/text | ✅ PASS |
| B context | local append/read/`len` | ✅ PASS |
| B context | **struct field** (and via `&ref` — the reported repro) | ❌ **CRASH** |
| B context | return / reassign / reassign-return | ✅ PASS |
| C construction | literal / literal-loop / copy-assign | ✅ PASS |
| C construction | **comprehension** `[[…] \| i in …]` | ❌ **CRASH** |
| D access | index / double-index / element-assign `v[i][j]=x` | ✅ PASS |
| D access | **nested iteration** `for row in v { for x in row }` | ⚠️ **WRONG** (silent) |
| D access | **pass inner `v[i]` to a fn** | ❌ **CRASH** |
| E nesting | triple `vector<vector<vector<T>>>` | ✅ PASS |
| F scale | local n=5000 | ✅ PASS |
| X control | flat `vector<int>`, `vector<Struct>` | ✅ PASS |

**Broken set (the real reach of #475):** struct-field construction · comprehension
· pass-inner-to-fn (all CRASH) · nested iteration (SILENT WRONG). Everything else
works. So an "error on nested vectors" decline is the wrong call — it would break
the large working surface + the 22 existing nested-vector tests.

## Clusters (verified vs hypothesized)

- **B-struct-field / B-struct-ref — VERIFIED.** Construction strides at 4
  (`self.size(vector<T>)=4`, `record_new` field path) but index strides at 8
  (`elm_size_raw.max(4)` collapses to inner) → read walks off stride-4 storage →
  crash once the store grows. `parser/fields.rs` + `parser/vectors.rs`.
- **D-nested-iter — VERIFIED.** Iteration strides at 16 (`database.size(main_vector
  wrapper)`) → reads every other element → silent half. `parser/collections.rs:281`.
- **C-comprehension, D-pass-inner — CRASH, mechanism HYPOTHESIZED** (own stride
  computations in the comprehension materialiser / arg-marshalling; not yet traced).

## Fix direction (chosen from the scope)

**Unify the two anomalies to the working de-facto stride, don't move everything to
the "correct" 4.** The WIP branch attempt (move all sites to the 4-byte handle)
fixed struct-field but *regressed working local cases* (reassign-return) — it
fought the grain because more paths collapse to 8 than were changed. Smaller/safer:
bring **struct-field construction (4→8)** and **iteration (16→8)** in line with the
stride-8 index that local already uses and passes; then trace + fix comprehension
and pass-inner. (A later, separate change could canonicalize the representation to
the 4-byte handle everywhere — a deliberate refactor, out of scope for the bug fix.)

WIP `4dedf55a` on branch `tuxedo-475-nested-vector-stride` is the *wrong-direction*
(unify-to-4) attempt — preserved as an anti-example; the fix should supersede it.

## Roadmap

1. struct-field construction stride 4→8 (or reconcile with index) — gate: `scope.sh`
   B-struct-field/ref PASS, no regression elsewhere.
2. iteration stride 16→8 — gate: D-nested-iter PASS.
3. trace + fix comprehension (C-comprehension) and pass-inner (D-pass-inner).
4. full suite both backends; graduate `scope.sh` survivors to
   `tests/scripts/475-*.loft`.
