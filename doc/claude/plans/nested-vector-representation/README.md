<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Nested-vector representation — scope investigation (loft/loft #475)

**Identity:** driven by **loft/loft issue #475** (a bug), NOT a `loft-lang/plans`
`@PLN` issue — the `475` there is a bug number; do not mint a `@PLN475` plan dir
from it. This is a bug-scoped investigation: probes + scope map, feeding a fix.

**Status:** FIXED on branch `tuxedo-475-nested-vector-stride` (commit `2a2f64e3`),
validated on `--interpret` AND `--native` — the corrected `probes/scope.sh` is
24/24 PASS both backends. The real reach was **two** shapes (struct-field
construction, nested iteration); the fix brings both to the working stride-8.
Two cells I first scored as crashes — comprehension, pass-inner — were **probe
bugs**, not #475 (see the correction note below). Graduated to
`tests/scripts/444-issue-475-nested-vector-handle-stride.loft`.

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
| B context | **struct field** (and via `&ref` — the reported repro) | ❌ **CRASH** → FIXED |
| B context | return / reassign / reassign-return | ✅ PASS |
| C construction | literal / literal-loop / copy-assign | ✅ PASS |
| C construction | comprehension `[for i in … { […] }]` | ✅ PASS †|
| D access | index / double-index / element-assign `v[i][j]=x` | ✅ PASS |
| D access | **nested iteration** `for row in v { for x in row }` | ⚠️ **WRONG** (silent) → FIXED |
| D access | pass inner `v[i]` to a fn | ✅ PASS †|
| E nesting | triple `vector<vector<vector<T>>>` | ✅ PASS |
| F scale | local n=5000 | ✅ PASS |
| X control | flat `vector<int>`, `vector<Struct>` | ✅ PASS |

**Real reach of #475 — TWO shapes:** struct-field construction (CRASH) and nested
iteration (SILENT WRONG). Both now fixed. Everything else works. So an "error on
nested vectors" decline is the wrong call — it would break the large working
surface + the 22 existing nested-vector tests.

**† Correction — two false positives.** My first pass scored comprehension and
pass-inner-to-fn as CRASH. Both were **probe bugs**, not #475: the comprehension
probe used a Python-style `[e | i in r]` (loft is `[for i in r { e }]`) so it
never parsed, *and* its oracle value was hand-miscomputed (`10 285` for what is
really `20 135`); the pass-inner probe declared `fn sum`, which collides with the
stdlib `sum`. Both PASS on the clean `main` baseline once the probes are fixed —
they were never broken. Root cause of the mis-score: exit-1 conflates a
parse/compile error with a runtime panic, and `2>/dev/null` hid the real message.
Lesson folded into `probes/scope.sh` (the `classify` note) and memory.

## Clusters (verified)

- **B-struct-field / B-struct-ref — VERIFIED + FIXED.** Construction strided at 4
  (`self.size(vector<T>)=4`, `record_new`'s `Parts::Vector` arm) but the index
  strides at 8 (`elm_size_raw.max(4)` collapses to the inner scalar) → the read
  walked off stride-4 storage → crash once the store grows. Fixed in
  `src/database/structures.rs` `record_new`: when the content is itself a vector,
  stride by `size(content(c)).max(4)` — the append sibling of the already-proven
  copy-path clamp at `structures.rs:419` (@PLAN58).
- **D-nested-iter — VERIFIED + FIXED.** Iteration strided at 16
  (`database.size(main_vector wrapper)`) → read every other element → silent half.
  Fixed in `src/parser/collections.rs`: a nested-vector element strides by
  `element_size(inner).max(4)`.

## Fix direction (chosen from the scope, then validated)

**Unify the two anomalies to the working de-facto stride (8), don't move everything
to the "correct" 4.** The WIP branch attempt (move all sites to the 4-byte handle)
fixed struct-field but *regressed working local cases* (reassign-return) — it
fought the grain because more paths collapse to 8 than were changed. The landed fix
brings **struct-field construction (4→8)** and **iteration (16→8)** in line with the
stride-8 index that local already uses and passes; the two false-positive shapes
(comprehension, pass-inner) needed no change (they were probe bugs — see above). A
later, separate change could canonicalize the representation to the 4-byte handle
everywhere — a deliberate refactor, out of scope for this bug fix.

WIP `4dedf55a` on this branch is the *wrong-direction* (unify-to-4) attempt,
preserved in history as an anti-example; commit `2a2f64e3` supersedes it.

## Outcome

1. ✅ struct-field construction stride 4→8 — `structures.rs` `record_new`.
2. ✅ iteration stride 16→8 — `parser/collections.rs`.
3. ✅ comprehension + pass-inner — proven false positives (probe bugs), no code needed.
4. ✅ corrected `scope.sh` = 24/24 PASS both backends; graduated to
   `tests/scripts/444-issue-475-nested-vector-handle-stride.loft`. Pending: full
   cargo suite green (running) → PR.
