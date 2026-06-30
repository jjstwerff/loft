<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN90 phase 1 — copy inventory + coverage gap

Phase 1 goal: make the copy-vs-borrow **decision** the sole arbiter consulted by every
structure-copy **emission**. First step (this doc): build the instrument that makes
copies visible, inventory every copy on a corpus, and measure the gap against the
current decision. Corpus: [copy-corpus.loft](copy-corpus.loft).

## The instrument — `LOFT_COPY_DUMP`

`LOFT_COPY_DUMP=1` prints one line per executed **deep structure copy**, off by default,
zero hot-path cost when off (cached flag `keys::copy_dump_enabled`). It lives in
non-generated runtime code (`src/fill.rs` is auto-generated — the hooks are in the copy
*implementations*):

- **`src/database/structures.rs::vector_add`** — `[copy] vector-append elements=N tp=…`
  (a non-empty source means the source's `N` elements are deep-copied into the dest store).
- **`src/state/io.rs::do_copy_record`** — `[copy] record line=N tp=…` (a real record
  deep-copy; the no-op-alias and null cases are skipped — only real copies print).

It is the **runtime ground truth**: every deep structure copy executes one of these two
ops, so this dump sees *all* of them. (Construction copies show as `vector-append`; there
is no source line at `vector_add` because `Stores` has no `State` — the source location is
the compile-time decision's job, phase 2.)

## Inventory (corpus + probes)

| shape | copies at runtime? | op | covered by the verdict? |
|---|---|---|---|
| `o += src` (append a vector) | **COPY** | vector-append | YES — `Copy` |
| `b.rows` (whole struct-field return) | **COPY** (into the return buffer) | vector-append | **NO** |
| `Struct { f: vec }` / `Enum { f: vec }` construction | **COPY** | vector-append | **NO** — and this is the *dominant* category |
| `a = s` (plain bind of a heap local) | **COPY** (conservative "just to be sure") | vector-append | **NO** |
| `v[i] = e` (element-slot set) | **COPY** | record | n/a (not a binding) |
| `o = b.rows` (bind a param's field) | borrow | — | YES — `Borrow` |
| `match e { Filled { items } => { items } }` | borrow | — | (@PLN85 path) |
| `o += [literal]` (append a fresh literal) | construct (new data) | — | not a copy |

## The gap (measured, `LOFT_MATERIALIZE_DUMP` vs `LOFT_COPY_DUMP`)

The current copy-vs-borrow **verdict** (`use_analysis`) classifies only the **vector-copy
binding** shape — `o = src` / `o += src`. On the corpus it emits exactly two rows
(`append_vec` → `Copy`, `assign_field` → `Borrow`). But the runtime dump shows **four**
copies: `append_vec`, `field_return`, and **two construction copies** (`Box { rows: s }`,
`Filled { items: s }`). So three of the four copies — and the whole **construction**
category, plus field-returns and the plain `a = s` bind — are **outside the decision
today**. They copy silently, exactly as the design predicted (only bigger: construction
is everywhere).

`a = s` copying is the sharpest confirmation of the plan's premise: a one-line bind of a
heap local silently deep-copies (the conservative default), and at runtime that is
unbounded — the "hundreds of MB just to be sure" case.

## The chokepoint insight (for the rest of phase 1)

At **runtime** every structure copy already funnels through exactly **two ops**
(`vector_add`, `do_copy_record`) — even though ~20 *compile-time* sites emit them. So the
phase-1 target ("one decision consulted by every emission") reduces to: **attribute every
emission of those two ops to a copy-vs-borrow decision with a reason.** Two routes:

1. **Extend the verdict's domain** beyond vector-copy bindings to construction,
   field-returns, and element-sets — the analysis becomes the one arbiter.
2. **Classify at emission** — a single `decide_structure_copy(...)` consulted at each of
   the ~20 emit sites (and at the construction path), returning avoidable/forced + reason.

Route 1 keeps the decision in one analysis pass (preferred — it matches OWNERSHIP_MODEL's
"the decision is the `deps` analysis"); route 2 is the fallback if the emit sites carry
context the analysis cannot reconstruct.

## Route decision — ROUTE 1 (extend the verdict's domain)

Decided by reading where the facts live:

- **The classification facts are only in the post-parse verdict pass.** `analyze_fn`
  (`use_analysis.rs`) runs after the function is parsed and uses the parser's mature
  interprocedural mutation analysis (`find_written_vars`) plus ordering and fresh-buffer
  facts. The copy **emit sites** (`parser/objects.rs::OpAppendVector` for construction,
  etc.) are **mid-parse and eager** — they do not yet know whether the source is later
  mutated, escapes, or out-lives the result. Route 2 would have to re-derive exactly what
  the verdict already computes; that is not "one arbiter."
- **The verdict already walks the full IR and the right ops.** It tracks
  `OpAppendVector`/`OpDatabase`/`OpGetField`, and `base_var` already resolves a
  field-source (`b.rows`) to its base var (`b`). The only reason construction and
  field-returns fall through is **what it records**, not what it can see:
  - it records an append only when the **target** is a plain `Value::Var`
    (`use_analysis.rs` ~line 215) — so construction's field-target
    `OpAppendVector(OpGetField(freshRec, fld), src)` is skipped;
  - the fresh-buffer test requires the target var to be an `OpDatabase` local — so the
    return buffer `__retbuf` (field-return) is skipped.

So extending the verdict is a **broadening of the same analysis** (recognise two more
copy-idiom shapes: append-into-a-fresh-record-field, and append-into-the-return-buffer),
not a new mechanism — the facts and the IR walk are already there.

**Extension work (route 1):**
1. Record appends whose target is a **fresh-record field** (construction), keyed on the
   record + field, reusing the existing `src` / mutation / ordering facts.
2. Recognise the **return buffer** as a fresh buffer (field-return).
3. Classification (`src_is_param` / `src_unmutated` / ordering) is unchanged — but add the
   forced reasons specific to these shapes (a struct/enum field **owns** its data, so a
   construction copy is often *forced* unless the source provably out-lives the record;
   that reason feeds phase 2's avoidable-vs-forced split).

Next: implement step 1 (construction — the dominant category) and re-run the corpus so
`LOFT_MATERIALIZE_DUMP` covers the copies `LOFT_COPY_DUMP` shows.

## Status

DONE this step: instrument (`LOFT_COPY_DUMP`) landed in non-generated code, corpus built,
inventory + gap measured. Suite green (issues 746); instrument off-by-default and
byte-identical when off. NEXT: choose route 1 vs 2 for closing the coverage.
