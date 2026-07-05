<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# COPY_DIAGNOSTICS.md — make every structure copy visible

Companion to [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md). That doc is the beacon for how
loft *decides* copy-vs-borrow; this one is the contract for how loft *tells the user*
when it copies — and why.

## The problem

loft's model already aliases heap values by default ([OWNERSHIP_MODEL.md § The law](OWNERSHIP_MODEL.md):
a binding/param to a struct, vector, or element *aliases* its source; only scalars copy).
A deep copy of a **record** or a **vector of records** happens as the **fallback** the
compiler emits when it cannot prove an alias is safe — and it is a real performance hit:
a fresh store allocation plus a per-element copy. Today loft emits those fallback copies
**silently** — the programmer cannot see, from the source, where the cost landed.

Worse, the compiler can *manufacture* fallback copies on patterns that should alias. The
@PLN85 `jo_copy_borrowed_arm_yield` synthesis rewrites a borrowed return into an owned
copy on one of the most common patterns there is (`match e { Filled { items } => { items } }`):
if such copies are silent, nearly every match that returns a field pays for a copy nobody
asked for and nobody can see.

The goal: keep aliasing the default, keep copies the fallback — and **make every fallback
copy visible**, with its reason. A copy that genuinely cannot be aliased is fine; it must
be *shown*, not blindly performed.

Three reasons a copy must never be silent — each on its own justifies warning on **every**
structure copy, not just "large" ones:

1. **Unbounded, runtime-sized cost.** A deep copy's size is a *runtime* property — the
   same `Verdict::Copy` site copies 3 records on one run and hundreds of megabytes on the
   next. The compiler cannot threshold it by static size; the only safe rule is to surface
   the copy itself.
2. **Conservative "just to be sure" copies are invisible and often avoidable.** The
   analysis defaults to `Copy` whenever it cannot *prove* an alias is safe — so the most
   expensive copies are exactly the ones nobody chose, emitted defensively, frequently
   removable by a small restructure the programmer would happily make *if they knew*.
3. **A copy silently changes behaviour.** loft aliases heap values by default; a fallback
   copy is *independent* — a later mutation does not reach the original. A programmer
   relying on the alias-default gets a program they did not write. The copy is a
   correctness surprise, not only a cost.

The warning has a **second audience — the loft compiler developers**. Every conservative
"just to be sure" copy is a place where the borrow analysis is not yet complete enough to
prove the alias. Those gaps are **invisible in the current code** — nothing marks them, so
we cannot find or prioritise them. The diagnostic turns that invisible set into a concrete,
rankable worklist: each avoidable-copy warning is a candidate to make the analysis borrow.
It is both a user-facing perf signal and our own instrument for driving
[OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md)'s completeness goal — surfacing the cases we might
still fix, instead of letting them hide.

## North-star: ELIMINATE the copy, don't just warn about it

The warning is the **instrument, not the goal**. The goal is the compiler *automatically*
not copying when it can prove a borrow is safe — **we never copy "just because."** loft
already has the elimination engine: a `Borrow` verdict produces an `ElidePlan` and the
borrow rewrite inlines the copy away ([OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md); the var-
buffer case ships today). The whole arc is to **grow that `Borrow` set** so copies
disappear with no user action.

So every copy sorts into four buckets, in priority order:

1. **Auto-eliminated** — the analysis proves the borrow is safe → it elides the copy. No
   copy, no warning. *Growing this bucket is the north-star.*
2. **Avoidable** — a borrow *would* be safe but the analysis cannot yet prove it (the
   conservative "just to be sure" default), or a small user restructure would let it.
   **Warned** — and **the warned-avoidable set is precisely the worklist for bucket 1**:
   each is a copy the compiler should learn to eliminate.
3. **Implicit** — **no unbound structure is produced**, so there is nothing to surface: a
   **scalar** copy (copy *is* the model), a **freshly-born owning value** (the source is a
   literal / a value built expressly for this binding — `S { f: [1,2,3] }`, `v[i] = Item{…}`
   — nothing pre-existing is duplicated), or a **move** (the source is consumed at this point,
   so its single backing *transfers* into the owner). The programmer asked for an owning value
   and **no live structure was duplicated** to give it to them. **Silent — never warned.**
4. **Forced** — an **unbound** copy that a borrow/move genuinely *cannot* replace: the source
   survives the copy **and** is later mutated independently, or it is a temporary subject that
   cannot outlive the result. Required as written, but a restructure could avoid it.
   **Indicated** (informational), never silent.

The diagnostic exists to drain bucket 2 into bucket 1, keep bucket 3 quiet, and make
bucket 4 honest. It is a means to *fewer copies*, not an end in itself; and it must not cry
wolf on bucket 3 — but the quiet bucket is *bound* results (a move / literal / borrow), **not**
"every `S { f: src }`". An `S { f: src }` that **duplicates a still-live `src`** is an unbound
copy and belongs in bucket 2 or 4; only the *move* form (`src` consumed here) is Implicit. The
"almost every match would warn" failure came from mis-classifying **borrows** as copies (bucket
1, eliminated) — not from surfacing genuine duplications.

**Bucket 5 — Internal (the two audiences, made concrete).** An unbound copy whose **source is
a compiler-generated temporary** (`_`-prefixed: `__ref_N`, `___par_mat_e_N`, `_comp_N` — names
the parser forbids for user vars) is **not user-actionable**: the user never wrote the source,
so "avoid this copy" points at nothing they can change. Such copies are routed to `Internal` —
**excluded from the user-facing report, but kept in the developer worklist** (a copy *we* may
still eliminate). This is the [OWNERSHIP_MODEL § second audience](OWNERSHIP_MODEL.md) split
made concrete: the user report shows buckets 2+4 (sources they wrote); our worklist adds bucket
5. On the first survey (371 scripts) this moved 139 of 173 indicated copies out of the user
report — the noise reduction that makes a user-facing report trustworthy. (A follow-up will
*attribute* an Internal copy to the user value behind the temp, resurfacing the actionable ones.)

## This is a perf lint, NOT a borrow checker (reconciliation)

[OWNERSHIP_MODEL.md § Internal and invisible](OWNERSHIP_MODEL.md) records a **decided**
position: ownership in loft is internal, never surfaces an *ownership error*, needs no
annotations — the compiler always finds a correct lowering (copying when it cannot prove
an alias), and the one user-facing ownership lever is `&`. This design does **not** breach
that. A copy diagnostic is:

- **informational, never an error** — it never blocks compilation, never demands an
  annotation, never says "cannot borrow". The naive program still compiles and runs; the
  lowering is unchanged. *Fun-on-pickup* ([GOALS.md](GOALS.md)) is preserved.
- a **performance** signal, not a correctness one — like a clippy lint, off the default
  path until asked for (see Open decisions).
- pointed at the **existing** levers, not new syntax: "avoid this copy" resolves to
  *let it alias* (don't reassign / don't mutate independently / keep the source alive) or
  the sanctioned `&` reference — never to a lifetime annotation.

The declined thing is a *checker* that makes the user solve ownership puzzles. This is a
*lint* that tells the user where a copy costs them, which they may ignore.

## The invariant (one sentence)

> **Every deep structure copy in the emitted program is decided at one place — the
> copy-vs-borrow verdict — and surfaced there with its reason, so a copy is never
> silent and a warning never fires without an actual copy.**

The "for the same reason" that makes this a design and not a pile of cases: the
diagnostic is a property of the copy **decision**, never of a syntactic pattern. Every
emitted copy descends from a `Verdict::Copy`; every `Verdict::Copy` is reportable at the
binding's source location; a `Verdict::Borrow` never warns. That is what kills the two
ways this feature dies — **silent copies** (a copy with no warning) and **false
positives** (a warning where no copy happens, e.g. warning on a match arm that actually
borrows). Both are excluded *by construction* when the warning hangs off the verdict.

## The silent/indicate line: bound vs unbound (not the syntactic construct)

The one rule that decides *silent vs indicated* is a property of the copy's **source**, not
of its destination construct:

> **Indicate every copy that produces an _unbound_ structure; stay silent when the result is
> _bound_ (or scalar).**

- **Unbound** — a fresh, independently-owned structure is materialised that *duplicates a
  pre-existing, still-live source* (a named var / field / element). Two independently-owned
  structures now exist where the alias-default implied one. This is the runtime cost **and**
  the behaviour divergence (a later mutation of one no longer reaches the other). **Always
  indicated**, whatever syntax produced it — a var binding, `S { f: src }`, `v[i] = src`, a
  bare `OpCopyRecord`.
- **Bound** — no independent duplicate coexists with a live source: an **alias / borrow** (no
  copy at all), a **move** (the source is consumed here, so its one backing *transfers* into
  the owner — one backing, not two), or a **freshly-born value** (the source is a literal /
  built expressly for this binding, so nothing pre-existing is duplicated). **Silent.**
- **Scalar** — a by-value scalar copy *is* the model; never a structure, never indicated.

This corrects the earlier, too-coarse rule "*construction / slot-assignment is inherently
silent because the field owns its data.*" *Owning the data* is silent-worthy only when the
field owns it **by birth** (a literal) or **by move** (a consumed source) — **not by
duplication.** A field that owns a *duplicate* of a still-live source is exactly an unbound
copy and must be shown. The destination construct (`S{…}`, `v[i]=…`) never decides silence;
the **source's fate** (consumed → bound; survives → unbound) does. The implementation
therefore keys the Implicit/Avoidable/Forced split on source survival, never on the op that
emitted the copy — see [plans/90-copy-diagnostics/unbound-copy-lint.md](plans/90-copy-diagnostics/unbound-copy-lint.md).

## Re-assertion sites — the load-bearing count

The decision is already centralised: `src/use_analysis.rs::Verdict { Borrow, Copy }`
computes one verdict per binding, conservative-default-`Copy`, and **already carries a
`reason` string** (`"reassigned (multiple defs)"`, `"local mutated or escapes"`, …) —
exactly the "why this copy is forced" the user wants surfaced. Hang the diagnostic off
the verdict and the warning lives in **one** place.

The copy **emission**, by contrast, is scattered across ~20 sites — `OpCopyRecord` and
`OpAppendVector` are emitted directly in `src/state/codegen.rs`, `src/parser/{collections,
operators,objects,expressions}.rs`. Warning at each emission site would be `N ≈ 20 ×
silent` brittleness (forget one site → a silent copy, not a compile error). So the
warning must NOT live at emission. It lives at the decision.

**This only holds if the decision covers every emitted copy.** It does not yet — and
this is the design's sharpest claim to falsify, below.

## The over-reach to falsify: does the verdict cover every copy?

The clean story — "the verdict is already the one chokepoint, just read it" — is
**false as stated**. `use_analysis` scopes its verdict to *vector-copy bindings* ("the
verdict for a single vector-copy binding"). The ~20 emission sites include copies the
verdict does **not** classify today:

- **struct construction from an existing value** (`s2 = s1`, `S { f: existing }`) →
  `OpCopyRecord`, no verdict row.
- **pass-by-value of a structure** to a function argument.
- **assignment** of a record/vector into a longer-lived owner.

If the warning reads only the verdict, those copies stay **silent** — the invariant
fails for everything that is not a vector-copy binding. So the design's first
deliverable is not the warning; it is **making the copy-vs-borrow decision the sole
arbiter consulted by every structure-copy emission** (extend the verdict's domain, or
route every `OpCopyRecord`/copying-`OpAppendVector` through one `emit_structure_copy`
helper that consults it). Until that holds, the warning is partial and must *say so*
(scope it explicitly to the covered cases, never imply completeness it does not have).

This is the OWNERSHIP_MODEL corollary applied to diagnostics: a missed copy-warning is
never a "warning bug" — it is a **hole in the copy-vs-borrow decision**.

## The three Copy buckets — which warn

Bucket 1 (auto-eliminated) never warns — the copy is gone. Of the remaining three, only
**Avoidable** is a warning the user should act on; **Implicit** is silent and **Forced** is
informational:

- **Avoidable** (bucket 2) — a borrow *would* be sound (source outlives the result, no
  independent mutation) but a copy is emitted, because the analysis could not yet *prove*
  the borrow (the conservative default), or a small restructure would let it. Diagnostic:
  *"this copies <T>; a borrow would avoid it — <how>"*. **This is the north-star worklist:**
  teach the analysis to prove these and move them to bucket 1, so the warning disappears
  *because the copy did* — not because the user silenced it.
- **Implicit** (bucket 3) — **no unbound structure is produced**: a scalar, a freshly-born
  owning value (literal source), or a **move** (`src` consumed here, its one backing
  transfers). Nothing pre-existing is duplicated, so there is no independent copy to surface.
  **Silent.** (This is *not* "every `S { f: src }`" — only the birth/move forms; a construction
  that duplicates a live `src` is unbound → bucket 2/4.)
- **Forced** (bucket 4) — an **unbound** copy a borrow/move cannot replace: the source
  survives **and** is mutated independently later, or it is a temporary subject that cannot
  outlive the result. Diagnostic: *"a copy of <T> is required here because <reason>"*.
  Informational — the cost is real and unavoidable as written, shown, not hidden.

The line that matters: **implicit** is a *bound* result (a move / literal / borrow — no
duplicate coexists with a live source); **avoidable** and **forced** are both *unbound*
duplicates (they differ only in whether the analysis *could* have borrowed/moved — avoidable —
or genuinely could not — forced). Source-survival is the first cut (bound vs unbound); the
lifetime/mutation facts the `deps` analysis holds are what then sort 2 vs 4.

## The user's escape hatch — "clearly indicated" cuts both ways

Sometimes the programmer *wants* the owned copy (independent mutation, decoupling a
lifetime). They must be able to say so and silence the diagnostic — an **explicit copy
intent** (e.g. a `.copy()` / `own(...)` form, exact syntax TBD). The contract is
symmetric: the compiler indicates copies it forces; the user indicates copies they
intend. A copy is acceptable when *someone* declared it — never when it is silent.

**The opt-out is SPARSE and SPECIFIC — a per-copy-site annotation, never a global switch.**
There is deliberately no file-level `allow(copies)` / global flag: that would re-hide the
whole class and defeat the endgame. The annotation marks *one* copy, so the accepted set stays
individually visible and auditable. This is what lets a **library PR be copy-clean** (the
project requires libraries to ship warning-free): a library is clean when every copy it emits
is either **eliminated** (the analysis proved a borrow) or **annotated** (an acknowledged
forced copy) — not when it flipped a blanket suppression. The annotation is the inverse of `&`
and the same grain as `&`: local, at the site, one decision at a time.

## What counts as "a structure copy" (scope)

Indicate every **unbound** deep copy of a **heap structure** — a record, or a vector of
records, materialised as a fresh independent duplicate of a still-live source (allocate a
fresh store + copy every element). **Every** such copy, regardless of static size: the
runtime size is unknown and unbounded (reason 1 above), so there is no honest size threshold.
The line is *deviation from the model*: indicate when the lowering produced an **independent
copy of a value the model says should alias** (a live heap structure). Do **not** indicate:
- **scalars** (`integer`, `single`, a small all-scalar struct) — a by-value scalar copy *is*
  the model ([OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md): scalars copy), never a surprise or a
  meaningful cost;
- **bound** results — a **move** (the source is consumed here, so its one backing transfers,
  no duplicate) or a **freshly-born owning value** (a literal source: nothing pre-existing was
  duplicated). These are silent even though the destination owns a heap structure, because *no
  live structure was duplicated* to fill it.

Noise is controlled the right way — not by hiding copies under a threshold, but by
**making borrows the default** (so a copy is genuinely rare) and by the **explicit
copy-intent** opt-out (so an intended copy is silent). A flood of warnings is then a true
signal that too much is still copying — exactly what we want to see while we drive borrows
in.

## Failure paths (enumerated — this is where the invariant was found)

| # | Failure | Cure |
|---|---|---|
| F1 | A copy emitted at a site that never consulted the decision → **silent copy** | single decision arbiter, consulted by every emission (the §coverage deliverable) |
| F2 | Warning fires where no copy happens (e.g. a match arm that borrows) → **false positive**, trust erodes | warn off the verdict (the actual decision), never a parse-time syntactic pattern |
| F3 | A forced copy reported as avoidable → user cannot act → frustration | classify with the lifetime/mutation reason; only call it avoidable when a borrow is sound |
| F4 | Too many warnings → noise → feature disabled | NOT a size threshold (runtime-sized; a "small" copy can be huge) — control noise by making borrows the default (copies become rare) + explicit copy-intent opt-out; scalars **and bound results (moves / literals)** are already silent, only *unbound* duplicates are surfaced |
| F7 | A **move** or a **literal** construction mis-classified as an unbound copy → **false positive** on a bound result (the noise F4 fears) | key the Implicit split on *source survival*, not the emitting op — a consumed/literal source is bound (silent); prove it per cell in the matrix (a move cell must read `implicit`) |
| F5 | User genuinely wants the copy → must silence without fighting the compiler | explicit copy-intent syntax that suppresses the diagnostic |
| F6 | The verdict is wrong (copies what could borrow, or vice versa) → diagnostic misleads | the warning is only as sound as the verdict — depends on the OWNERSHIP_MODEL work |

## Dependencies and the live connection

- **Soundness rides on the verdict** (`use_analysis` / `deps`), i.e. on
  [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md). A wrong verdict gives a wrong diagnostic.
- **Borrow-correctly is the prerequisite, not the owned-copy synthesis.** The @PLN85
  match-return work showed loft already copies the struct-field return (`{ b.rows }`
  materialises into the return buffer) and the borrowed match-yield (`{ items }`) is a
  true borrow that today mis-compiles. The right direction is to make the borrow
  *compile correctly* (a borrowed vector return needs no return-buffer ABI — return the
  alias), so borrows are the default and a copy is the rare, warned exception. The
  owned-copy synthesis manufactures the very copies this feature would warn on, on a
  near-universal pattern — keep it only as the fallback for the genuine forced case (a
  temporary subject that cannot outlive the return).

## The lifecycle — a temporary flag, staged to close

This feature is **not** a permanent always-on warning; it is a flag we turn on to gather
data, then drive to a close. The stages (full recipe:
[plans/90-copy-diagnostics/unbound-copy-lint.md](plans/90-copy-diagnostics/unbound-copy-lint.md)):

1. **Survey (flag-gated report) — LANDED.** The correct classification (the bound/unbound
   survival split) ships behind **`loft --report-copies`** (`LOFT_REPORT_COPIES=1`), which prints
   a **report** — per-site rows (location · copied type · reason) + an aggregate rollup + the
   Avoidable worklist — over a whole library or program. It answers two questions: *what still
   copies that we can fix* (the Avoidable worklist — for us) and *where the hidden cost is* (the
   survey — for lib authors and users). It shows only the user's source-duplication copies (the
   var-buffer/return-buffer class stays in the developer dump `LOFT_MATERIALIZE_DUMP`).
   **Off by default.**
2. **Drain.** Grow the auto-elision engine (`Borrow` → `ElidePlan`) to remove the Avoidable
   copies the survey surfaces — the worklist shrinks toward the genuinely-forced remainder.
3. **Accept the remainder.** The copies that truly cannot borrow/move are accepted; a library
   marks each with the **sparse per-site opt-out annotation** above so its PR is copy-clean.
4. **Close (promote to enforced).** With Avoidable drained and the forced remainder annotated,
   the flag graduates to an **enforced** lint / library-PR gate — every remaining copy is now
   eliminated or explicitly acknowledged. Plan #90 closes here.

## Open decisions (for the user)

1. **Coverage first or report first?** The report is only as complete as the decision. Either
   extend the copy-vs-borrow decision to all structure copies before shipping the report, or
   ship a *scoped* report (stating its own scope). Recommendation: widen the decision (the
   survival split covers construction/slot-set) first; a partial report that looks complete is
   worse than none.
2. **Flag → enforce, not warn-by-default from day one.** The stage-1 report is opt-in
   (`--report-copies`), never a default warning — we read the data before enforcing. Promotion
   to a default warning / library-PR gate is stage 4, gated on draining Avoidable (stage 2) and
   the opt-out annotation (stage 3), not a day-one choice.
3. **Explicit-copy syntax** (F5, the stage-3 opt-out) — the surface form (the inverse of `&`:
   opt into an independent copy and silence the report at *that site*). **Sparse and per-site by
   design — never a global `allow`** (§ escape hatch).

## Probes run while writing this (the prediction-vs-reality record)

- *Claim: copies route through one decision.* Probe: grep the emission sites —
  `OpCopyRecord`/`OpAppendVector` appear at ~20 sites across parser+codegen; the
  *decision* (`Verdict`) is one place but scoped to vector-copy bindings. → the
  coverage gap above is real and is the #1 risk, not an afterthought.
- *Claim: the reason for a forced copy is already available.* Probe: `VerdictRow.reason`
  exists and carries human-readable justifications today. → the forced/avoidable split
  is cheap to surface for the covered cases.
