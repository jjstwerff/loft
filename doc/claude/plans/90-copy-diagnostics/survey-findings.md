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

| bucket | unique rows | meaning |
|---|---|---|
| **Forced** | 152 | unbound copy, source mutated/escapes after — required as written |
| **Implicit** (move/literal) | 73 | silent — no unbound structure produced |
| **Avoidable** | 21 | a borrow/move would remove it — the drain worklist |

- **stdlib baseline = 0.** An empty program emits **no** unbound construction/record copies —
  the stdlib's own construct/record copies are all moves/literals. The split does not indict
  the stdlib; every row above is application / test code.
- 246 unique survival rows total; **54** of the Forced rows are unnamed `<record>` targets
  (`v[i] = e`, `tgt` is an element, not a named var).

## What to resolve before flipping (the phase-A → phase-B/D worklist)

The bound/unbound **cut is sound** (the corpus proves it, stdlib is clean). Four things must be
resolved before the flag becomes a user-facing report/gate — each is a *reporting fidelity*
issue, not a soundness one:

1. **Exclude / attribute synthetic temporaries (29 rows).** `__ref_N`, `___par_mat_e_N`,
   `_comp_N`, `__retbuf` etc. are compiler-generated; a user cannot act on "`__ref_11` is an
   avoidable copy". The report must suppress synthetic sources/targets or attribute them to the
   user construct that generated them. **This is the #1 blocker for a user-facing report.**
2. **Source locations for `<record>` targets (54 Forced rows).** A `v[i] = e` copy shows the
   target as `<record>` (no named var). The report needs `file:line`, not a var name, to be
   actionable — the diagnostic must carry the copy site's span.
3. **Tighten the Avoidable/Forced split (the 152 Forced is inflated).** `mutated/escapes after`
   keys on `other_max_pos > copy_end`, which counts **any** non-reader use after the copy —
   including passing `src` to a read-only callee. That over-classifies genuine avoidables as
   Forced. The bound/unbound cut is unaffected; only the sub-split (worklist vs informational)
   needs the mutation fact sharpened (reuse the parser's interprocedural `find_written_vars`
   for the source, as the var-buffer path already does for `¬D2`).
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
