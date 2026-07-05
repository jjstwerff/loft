<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN90 phase A — the survival split lands (flag-gated) + the first survey

The bound-vs-unbound survival split ([unbound-copy-lint.md](unbound-copy-lint.md)) is
**implemented and gated** (`LOFT_COPY_SURVIVAL`, default OFF). This records the initial
survey — *where it fires on the tests* — so we can decide what to resolve before flipping it
default-on (phase D). Design: [COPY_DIAGNOSTICS.md § bound vs unbound](../../COPY_DIAGNOSTICS.md).

## What landed (Steps 0–4)

`src/use_analysis.rs`: `construct_copy` / `record_copy` now carry `(dest, src, copy-site-end
pos, in-loop)`; a new `last_use_pos` (any-position last use) drives `survival_class`, which
sorts each construction / record copy:

| source fate | bucket | user-facing |
|---|---|---|
| literal / freshly-built (`src` none) | Implicit | silent |
| **move** — `src` last-used at the copy (no use after) | Implicit | silent |
| **unbound, read-only survivor** | Avoidable | indicated (worklist) |
| **unbound, mutated / escapes after** | Forced | indicated (informational) |
| in-loop copy, no straight-line survivor | Avoidable (caveat reason) | indicated, flagged for review |

**Gate = at the call site**: flag OFF returns the *original* phase-1 `Implicit` + reason
verbatim, so the default dump and the whole suite are **byte-identical** (full suite green;
`tests/use_analysis.rs` 16/16 unchanged). `survival_class` runs only when the flag is on.
Validated on `bytecode-comparisons/survival-corpus.loft`: move → implicit, survives → avoidable,
survives+mutated → forced (both backends, value-clean).

**Reproduce the survey:**
`LOFT_COPY_SURVIVAL=1 LOFT_MATERIALIZE_DUMP=1 loft --interpret prog.loft 2>&1 | grep '^MAT'`
— per-fn rows + a `MAT-WORKLIST avoidable=… implicit=… forced=…` rollup.

## The survey — 371 `tests/scripts/*.loft`, deduped

| bucket | pre item 1 | after item 1 | **after item 3** | meaning |
|---|---|---|---|---|
| **Forced** | 152 | 29 | **1** | unbound copy, source **written** after — required as written |
| **Implicit** (move/literal) | 73 | 73 | **203** | silent — no unbound structure produced |
| **Avoidable** | 21 | 6 | **39** | a borrow/move would remove it — the drain worklist |
| **Internal** (item 1) | — | 139 | **10** | copy of a compiler-generated source — developer worklist, **not** user-facing |

The **user-facing indicated set** (Avoidable + Forced) is now **40** rows.

> **Item 3 uncovered a latent phase-1 bug — the `OpCopyRecord` source/dest were swapped.**
> `OpCopyRecord(source=arg0, dest=arg1)` (verified against `State::copy_record` + `find_written_vars`),
> but the phase-1 `record_copy` recorder labelled `tgt=arg0, src=arg1` — so the survival split had
> been classifying the **destination**, not the source. Fixing the swap re-drew every bucket: most
> record-copy sources are literals/moves → `implicit` (73→203); the true compiler-generated
> **sources** are only 10 (not 139 — that was the mislabelled dests); and the genuine read-only
> survivors surface as a real 39-row `Avoidable` worklist. `Forced` collapsing to 1 is correct —
> "copy a value then mutate the source" is genuinely rare. Codegen unaffected (record rows never
> produce an `ElidePlan`); full suite green.

- **stdlib baseline = 0.** An empty program emits **no** unbound construction/record copies —
  the stdlib's own construct/record copies are all moves/literals. The split does not indict
  the stdlib; every row above is application / test code.
- 246 unique survival rows total; **54** of the Forced rows are unnamed `<record>` targets
  (`v[i] = e`, `tgt` is an element, not a named var).

## What to resolve before flipping (the phase-A → phase-B/D worklist)

The bound/unbound **cut is sound** (the corpus proves it, stdlib is clean). Four things must be
resolved before the flag becomes a user-facing report/gate — each is a *reporting fidelity*
issue, not a soundness one:

1. **~~Exclude / attribute synthetic temporaries.~~ ✅ DONE (commit pending).** A copy whose
   **source** is compiler-generated (`is_compiler_generated` — a `_`-prefixed name, which the
   parser forbids for user vars: `__ref_N`, `___par_mat_e_N`, `_comp_N`, …) is routed to a new
   `CopyClass::Internal` — a *developer-worklist* copy (one WE may eliminate) that is **excluded
   from the user-facing Avoidable/Forced set**. Effect on the 371-script survey: the user-facing
   indicated set collapsed **173 → 35** (139 rows moved to `internal`; 6 Avoidable + 29 Forced
   remain, all naming a real source). Guard: `tests/use_analysis.rs::survival_split_bound_vs_unbound_and_internal`.
   The dump tally gained `internal_copies=N`. Residual (a follow-up, not a blocker): full
   *attribution* — trace a synthetic source back to the user value it holds (`__ref = user.field`)
   — so a genuinely-actionable copy hidden behind a temp resurfaces; deferred, the Internal set
   is where that tracing will happen.
2. **~~Source locations for `<record>` targets.~~ ✅ PARTIALLY DONE (commit pending).** Each
   survival row now carries a source location (`VerdictRow.loc`, printed as ` at file:line:col`).
   The copy ops (`OpCopyRecord`/`OpAppendVector`) carry **no span of their own**, so the location
   is a **breadcrumb** — the nearest enclosing spanned node, tracked during the walk and **gated
   on the flag** (`track_pos`; the default hot path pays no `Position` clone and stays
   byte-identical). It lands for the common `v[i] = e` element-set inside a spanned statement
   (the right statement line, verified on `recloc`/`mixed`). **Residual (needs the parser to
   span the copy ops — a follow-up):** construction field-appends are emitted in an *unspanned
   preamble* (no location), and keyed-set / cast / nested record copies (p311–p315) sit without
   a preceding span (~11 of 19 `<record>` rows in the survey lack a location). The breadcrumb is
   a *nearest-statement* approximation, not a precise sub-expression span. Guard extended:
   `survival_split_bound_vs_unbound_and_internal` asserts a located `recset` copy row + that the
   flag-off dump carries no ` at ` suffix (byte-identical).
3. **~~Tighten the Avoidable/Forced split.~~ ✅ DONE (commit pending).** The old test keyed on
   `other_max_pos > copy_end` (**any** non-reader use — a read-only pass-to-callee or being
   another copy's source counted as "escape"), inflating Forced. Replaced with a **position-aware
   write fact** `mut_max_pos` — the max position a var is actually *written* (a `Set` target, or
   any first-arg-write op / `OpCopyRecord` dest, via a new `write_first_arg` set mirroring
   `find_written_vars`). Forced now = source survives **and is written after the copy**;
   read-only survivors (incl. read-only callee args) are Avoidable. `other_max_pos` is left
   untouched so the shipped var-buffer elision stays byte-identical. **En route this fixed the
   `OpCopyRecord` source/dest swap** (see the survey table note) — the two together re-drew the
   distribution to Forced=1 / Avoidable=39 / Internal=10. Residual (conservative-toward-Avoidable,
   safe): a source passed to a *mutating callee* after the copy isn't caught (a false-Avoidable
   the phase-B elision — the real borrow checker — rejects). Guard: the corpus `cmut`
   (`inner += [4]`) → Forced pins the write-detection.
4. **Real in-loop survival (the caveat rows).** The `in-loop copy` reason conservatively marks a
   loop-body copy as surviving. A per-iteration *local* source (`for i { x = mk(); v[i] = x }`)
   is actually a move; only a source defined **outside** the loop truly survives. Needs the
   source's definition position vs the loop entry, else these are false positives.

**Order:** (1) then (2) make the report trustworthy to show a lib author; (3) then (4) make the
Avoidable worklist precise enough to drive the drain (phase B). None block landing the *gated*
classifier — they block flipping it on.

## Next

Phase B = drain the Avoidable set (grow `Borrow`→`ElidePlan`); phase C = the sparse per-site
opt-out annotation; phase D = flip to an enforced library-PR gate once (1)–(4) + the drain are
done. Run the survey over real libraries (`lib/markdown`, `lib/audience_crystal`) and demos to
size the *hidden cost* audience (question 2) once (1)+(2) land.
