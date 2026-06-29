<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Ownership analysis — validation + the exact gaps (what the compiler must do)

The Stage-1 `Owned|Borrowed|Join` classifier (`src/use_analysis.rs`) is the INPUT the
Stage-3 compiler fix reads. Before wiring any free site we must know its gaps — *"we
can only detect what we need to do in the compiler once we know the exact gaps in the
analysis"* (user). This is that validation.

**Instrument:** [`fuzz/classify_vs_runtime.py`](fuzz/classify_vs_runtime.py) correlates,
per probe cell, the **classification** (the `OWN fn=…` dump) against the **runtime
over-free outcome on BOTH backends** (CRASH/LEAK, with compile-errors + asserts
separated out — they are not over-free signals). Run over the generated 54-cell matrix
(`grammar_gen.py`) + the 28 `probes/over-free-sweep/` probes + the 5 `462-*` repros.

## The headline: the analysis is SOUND (zero misses)

> **Across all 87 probe cells, every value that actually over-frees (CRASH or LEAK) is
> classified `Join` or `Borrowed` — NEVER pure `Owned`.** The analysis never tells the
> compiler "free this" about a live borrow. `SOUND MISSES: NONE`.

This is the load-bearing property: the classifier is a SOUND foundation. The gaps below
are about **completeness** (surfacing the free SITES) and **precision** (carrying the
base, scoping), not soundness.

## The gap map (none-churn matrix; classification × runtime)

| shape (struct) | class at escape | interp | native | reading |
|---|---|---|---|---|
| **elem_accumulate** | `pick:ret=Join` | **CRASH** (UAF) | clean | flag-OK; **interp-only** over-free |
| **match_return** | `deliver:ret=Join` | **CRASH** (UAF) | clean | flag-OK; **interp-only** over-free |
| **local_source** | `one:reassign(chosen) prior=Owned rhs=Join` | **LEAK** | **LEAK** | flag-OK; **both backends** |
| field_return / field_local / field_reassign / nested_field | `…:ret=Borrowed` | clean | clean | safe borrow — "don't free" is correct |
| index_read | `deliver:ret=Join` | clean | COMPILE-ERR | Join accurate; native bug (below) |
| if_return | — | COMPILE-ERR | COMPILE-ERR | doesn't type-check (below) |
| *all scalar* | `ret=Owned` / `ret=Join` | clean | clean | scalar never over-frees |

Key refinement: **`elem_accumulate` and `match_return` over-free on INTERP only — native
is already correct** (right values, no leak). Only `local_source` leaks on both. So native
is a correctness REFERENCE for two of the three shapes — mirror what it does, don't invent.

## The exact gaps — and the compiler action each unblocks

> **UPDATE — Gaps A + B are now CLOSED (Stage 1.5, still inert).** `use_analysis::free_sites`
> surfaces both missing free sites, each with the freed value's class + the borrow base; the
> `ownership_surfaces_free_sites` test pins them, and the free sites now correlate EXACTLY with the
> over-free outcome (see § Stage 1.5 below). The original gap text is kept for the record.

### Gap A — the analysis classifies VALUES; two of three fixes act at a FREE SITE it doesn't surface
The over-free decision lives at a free site, and the analysis currently exposes only two of
the three site kinds:

| shape | the over-free SITE | analysis surfaces it? | compiler action |
|---|---|---|---|
| `local_source` | the owned-slot **reassign** | ✅ `reassign_sites` gives `prior=Owned rhs=Join` | **READY** — wire `scopes.rs` free-placement (both backends leak) |
| `elem_accumulate` | the **append element source-free** (`OpCopyRecord` `0x8000` on `pick`'s Join return, in `collect`) | ❌ only `pick:ret=Join`, not "collect's append source-frees a Join" | needs **append-source-free site classification** → then fix interp's source-free |
| `match_return` | the **arm-return delivery** (`materialize_vector_arms_into` reassigns the buffer to a borrowed enum field) | ❌ only `deliver:ret=Join`, not the delivery site | needs **arm-return delivery site classification** → then fix interp's delivery |

So: **`local_source` can be wired NOW; `elem_accumulate` + `match_return` need the analysis
extended to surface their free sites first** (a Stage-1.5 increment — still inert, still
testable, before any emit change).

### Gap B — `Own::Borrowed`/`Join` carry no BASE
The Join fix is "materialise the borrow arm to owned at the escape" — which needs to know
WHICH store to copy. `Own::Borrowed` is currently opaque (Stage 1 dropped base tracking).
**The fact must carry the borrow base** (the projection's arg-0 / source var) before the
materialise can be emitted.

### Gap C — `Join` is over-flagged on clean cells (precision) — but closed by acting at the SITE
`match_return scalar` and `index_read` classify `Join` yet run clean. A value-level
"materialise every Join" would do needless work and risk regressing them. **Resolved by
Gap A's framing:** act at the FREE SITE, which only fires for record-element stores — so
scalar joins are never touched, no narrowing predicate needed. (Confirms the fix is
site-driven, not value-driven.)

## Adjacent bugs the sweep surfaced (NOT the over-free class; branch-internal)

Both around an empty `[]` literal used as a `vector<T>` default in a branch/coalesce tail
— a single likely root (empty-vector tail not typed/coerced to `vector<T>`):

1. **`if cond { v.rows } else { [] }`** → `error: expected vector<E>, got void on else`
   (the whole `if_return` shape fails to type-check). POISON-independent.
2. **`vv[i] ?? []`** → interp clean, **native `error[E0308]: mismatched types`** (the
   `index_read`/#426B shape; an interp↔native codegen divergence).

These block two probe shapes from even reaching the over-free analysis. Documented here
(branch-internal, stacked on @PLN25); not the join-ownership work — fold or file separately.

## Stage 1.5 — the free sites + base, surfaced (DONE, inert)

`use_analysis::free_sites(data, d_nr)` reports, per function (also dumped as `OWN fn=… free …`):
- **`AppendSource`** — each `OpCopyRecord(src,_,tp)` with the `0x8000` source-free bit, when `src`
  classifies `Borrowed`/`Join` (the `out += [pick()]` site). `elem_accumulate`'s is `Join`,
  `base=None` (the source is the inline `pick()` call → materialise = deep-copy the whole value).
- **`ParamDeliver`** — a heap-parameter return buffer reassigned to a **direct** `Borrowed`/`Join`
  projection (`base.is_some()`). `match_return`'s is `_mv_items_1 = OpGetField(e,…)`, `class=Borrowed`,
  `base=e` (the field to copy into the buffer).

The free sites now correlate **exactly** with the over-free outcome across the matrix:
`elem_accumulate`→`AppendSource`, `match_return`→`ParamDeliver`, `local_source`→the reassign site;
**every clean shape reports no site.**

**Precision finding (the `base.is_some()` filter on `ParamDeliver`):** `field_reassign`'s `best =
rows(b)` reassigns the retbuf via a CALL that delivers into it — which MATERIALISES a copy (best is
genuinely Owned), unlike `match_return`'s raw `OpGetField` projection (which aliases). The
discriminator is the base: a direct projection has a local base; a retbuf-materialising call has
none. Filtering `ParamDeliver` to `base.is_some()` drops the clean `field_reassign` false-positive
(which would otherwise mislead Stage 3 into NOT freeing an owned store → a leak) and guarantees every
reported site carries a usable materialise base. **Residual (Gap C, deferred):** `match_return scalar`
still reports `ParamDeliver` (same projection shape) though it runs clean — materialising it is safe
(a redundant copy); Stage 3 may gate on a record-element type if it matters.

## The unification — one oracle every site reads (in progress)

The per-site approach hit a wall at `elem_accumulate`: my codegen gate grew to a
**10-condition thicket** — itself the red flag `STABILITY_METHOD.md` warns about (a fact
re-derived through a conjunction of proxies). And the validation showed native's *own* inline
guard re-derives the same decision and gets the owned arm **wrong** (it materialises it →
`462-elem-accumulate-owned-branch-CLEAN` leaks on native). So both backends carry a wrong
per-site re-derivation; a third copy — however correct — is the wrong structure.

The fix is the OWNERSHIP_MODEL collapse: **one ownership oracle** every own-vs-borrow site
*reads* instead of re-deriving (the study's "single unification refactor"). The carried fact is
`Own { Owned | Borrowed{base} | Join{base} }` — the `base` is the var the value aliases, the
witness the `Join` runtime guard needs.

**Step 1 — DONE (inert):** `use_analysis::ownership_of` (the consolidation of Stage 1 `classify`
+ Stage 1.5 `borrow_base`) now folds the `base` into `Own` and resolves it
**interprocedurally** — a call's borrowed-view return maps the callee's borrowed param to the
CALLER's argument (`collect`'s `out += [pick(t,i)]` source resolves to base `t`, the witness).
Validated by `ownership_resolves_the_borrow_base` against hand-computed ground truth across the
shapes (`pick→Join(base=t)`, `deliver→Join(base=e)`, `nested→Borrowed(base=o)`, the `pick_cond`
reassign borrow arm → `pool`). Suite byte-identical (inert).
- KNOWN APPROXIMATION (pinned in the test): a whole-field / whole-arg return delivered through
  `__retbuf` resolves its base to `__retbuf`, not the true source (`getf`/`whole`). Harmless —
  clean field-return sites, never an over-free — but a precision gap a later fix should close.

**Step 2 — DONE (first chokepoint collapse, interp, `LOFT_JOIN_OWN`):** the interp first-bind
(`state/codegen.rs`, the `owned_ref` call-bind) now READS the oracle instead of the type-shape proxy.
A `Join` return bound into an owned slot emits `OpBindOrCopy` — the runtime arg-aliasing guard
**witnessed by the oracle's interprocedurally-resolved base** (collect's `t`): it ADOPTS the owned
`m_none()` arm (the source-free then frees it) and MATERIALISES the borrowed `t[i]` arm (the
source-free hits the copy; `t` intact). This FIXES BOTH arms of `elem_accumulate` on interp — the
thing the per-site re-derivation could not, because the witness needed the interprocedural base only
the oracle resolves. `join_own_fixes_elem_accumulate_interp` pins it; suite green flag-off; clean
shapes + scalar (`pick` returns Owned → no bind) + `local_source` all unaffected.
- The bytecode-stack discipline that bit twice: a PUSH op (`OpVarRef` witness) takes `var_pos` BEFORE
  `add_op` (reads at the pre-push position); the call result must be on the stack first so the
  witness offset accounts for it.

**Step 3 — DONE (native first-bind collapse, `LOFT_JOIN_OWN`):** `generation/dispatch.rs::output_set`
now reads the oracle too. For a `Join` return it replaces the buggy `_src.store_nr == _dst.store_nr`
guard with the oracle's base WITNESS: `adopt iff _src is null OR does not alias var_<witness>`, else
materialise. This fixes native's owned-arm leak (`owned-branch-CLEAN` was LEAK on native → clean).
**`elem_accumulate` is now correct on BOTH backends, both arms** — and both backends READ THE SAME
fact (the oracle), so they cannot diverge on it. `join_own_fixes_elem_accumulate_both_backends` pins
it; suite green flag-off.

**Next: `match_return` — DIAGNOSED (a different, deeper mechanism than the first-bind).** Both
backends ALIAS the retbuf to the enum field (`_mv_items_1 = OpGetField(e,4)`; native emits the same
`DbRef{..pos+4}` alias). The interp LEAK is NOT the delivery — `LOFT_LEAK_SITES` pins it to `cell`
(the `Filled` enum) + its `inner` vector in MAIN: because the returned vector aliases `cell`'s field,
interp's free analysis conservatively SKIPS freeing `cell`/`inner` (to keep the alias valid) → they
leak. Native aliases too but frees them anyway. So the chokepoint is the over-free class's LEAK
mirror (a borrow alias suppresses the owner's free), and the fix is to MATERIALISE the Filled arm —
copy `e`'s items into the cleared retbuf so `deliver` returns OWNED, breaking the alias and letting
`cell`/`inner` free normally. The `ParamDeliver` site (`Borrowed(base=e)`) is already surfaced by the
oracle; the collapse is a SCOPES-level rewrite of that Set (alias → `OpAppendVector(retbuf, e.field)`)
read off `free_sites`, NOT a codegen guard (the arm delivery is generated pre-oracle by
`materialize_vector_arms_into`). Likely interp-only (native already frees correctly).

Then flip default-on once all three sites are green on both backends + the full matrix + POISON. Then
fold the remaining own-vs-borrow re-derivations (the return-delivery thicket) onto the oracle as cleanup.

## Next step (per-site — SUPERSEDED by the unification above for sites 2/3)

1. **`local_source`** — ✅ DONE (commit `a639433d`, behind `LOFT_JOIN_OWN`). Root cause (nailed by an
   FRD runtime trace): `chosen = dflt()` move-adopts the fresh store into `chosen`; the caller's
   `__ref_2` retbuf keeps its null sentinel (it never owned the store), so the cleanup
   `FreeRefIfDistinct(__ref_2, chosen)` guards the wrong ref and the store orphans on reassign. Since
   `free` is store-level, a naive reassign-free is unsound (it would whole-store-free the pool). FIX:
   `use_analysis::displaced_owned_slots` flags `chosen`; `scopes.rs::scan_set` strips its `["pool"]`
   dep so the OWNED path deep-copies the borrow into its own store + frees it at scope exit (reusing
   the instances-1/2 `make_independent` pattern). Proven both backends; gate + fix tests in
   `tests/use_analysis.rs`.
2. **`elem_accumulate`** — DIAGNOSED (the loft-codegen "prove the working bytecode" step is done);
   implementation pending. **Interp-only** (native already correct). The divergence is the first-bind
   of a borrowed-view call return (`__lift_1 = pick(t,i)`, `pick -> M { t[i] ?? m_none() }`):
   - **Native** runtime-guards it (`generation/dispatch.rs::output_set` ~L358): `let _dst = old; let
     _src = pick(...); if _src.store_nr == MAX || _src.store_nr == _dst.store_nr { adopt _src } else {
     OpDatabase(_dst); OpCopyRecord(_src→fresh, NO source-free) }` — then the append source-frees the
     result. The owned arm (`m_none`) ADOPTS (the append's `0x8000` frees it correctly); the borrow
     arm (`t[i]`) MATERIALISES a copy (the append frees the copy; `t` intact).
   - **Interp** (`state/codegen.rs` `owned_ref` first-bind ~L1672): (a) gates the deep-copy on a
     type-shape proxy ("callee has a visible Reference/Enum param") that MISSES `pick`'s Vector-param
     borrow — study **instance 1**, fixed in native, NOT interp — AND (b) its deep-copy is an
     UNCONDITIONAL materialise (`OpDatabase` + `OpCopyRefOrNull`) with no adopt branch.
   - PROVEN (experiment, reverted): merely aligning the gate to `returns_borrowed_view()` fixes the
     borrow UAF (`CRASH→clean`) but LEAKS the owned arm (`clean→LEAK`) — the static `is_borrowed_view`
     suppresses the source-free uniformly (right for borrow, wrong for owned). A static gate cannot
     work — confirming the study's "the source-free is load-bearing for the owned branch". **The fix
     must mirror native's RUNTIME store-identity guard**, cleanest as a new interp op
     `OpBindOrCopy(src, pos, tp)` (adopt if `src.store_nr ∈ {MAX, old_v.store_nr}`, else fresh +
     deep-copy), emitted for a borrowed-view return in place of the unconditional deep-copy, gated
     `LOFT_JOIN_OWN`. Interp-only (native does it inline). Add via a `default/*.loft` `#rust` template
     + `make fill`; validate `462-elem-accumulate-{source-free,owned-branch-CLEAN}` + matrix + POISON
     on both backends.
3. **Then** `match_return` (interp `ParamDeliver` delivery) — surfaced by `free_sites`.
4. Flip default-on once all three + the full matrix are green on both backends and POISON-clean.
