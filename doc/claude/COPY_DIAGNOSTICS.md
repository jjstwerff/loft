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
3. **Implicit** — the copy is **inherent to the ownership model**: constructing an owning
   structure (`S { f: src }` — the field owns its data) or assigning into an owning slot
   (`v[i] = e` — the element owns its record) copies *because that is what the construct
   means*. The programmer asked for an owning value; it is not a surprise and not something
   to "fix." **Silent — never warned.** (A copy is only worth flagging when it is *not*
   the obvious consequence of the code the programmer wrote.)
4. **Forced** — the value must be owned by *circumstance*, not by an obvious ownership
   boundary: the source is short-lived (a temporary subject), or mutated later so it cannot
   be aliased. Required as written, but a restructure could avoid it. **Indicated**
   (informational), never silent.

The diagnostic exists to drain bucket 2 into bucket 1, keep bucket 3 quiet, and make
bucket 4 honest. It is a means to *fewer copies*, not an end in itself; and it must not cry
wolf on bucket 3 — warning on every `S { f: src }` would bury the avoidable copies that
actually matter (exactly the "almost every match would warn" failure that motivated this).

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
- **Implicit** (bucket 3) — the copy is the obvious consequence of the construct: a
  struct/enum field or a vector slot **owns** its data, so `S { f: src }` and `v[i] = e`
  copy by definition. The programmer asked for an owning value. **Silent** — warning here
  would fire on near-every constructor and bury the avoidable copies that matter.
- **Forced** (bucket 4) — a borrow is unsound by *circumstance*, not by an obvious ownership
  boundary: the source does **not** outlive the result (a temporary subject), or it is
  mutated later so it cannot be aliased. Diagnostic: *"a copy of <T> is required here
  because <reason>"*. Informational — the cost is real and unavoidable as written, shown,
  not hidden.

The line that matters: **avoidable** is a copy the analysis is merely too weak to elide
(calling it forced would hide the north-star work); **implicit** is a copy the model
*defines* (warning it is noise); **forced** is a copy a restructure *could* remove (worth
saying once). The lifetime/mutation facts the `deps` analysis holds are what sort 2 vs 4.

## The user's escape hatch — "clearly indicated" cuts both ways

Sometimes the programmer *wants* the owned copy (independent mutation, decoupling a
lifetime). They must be able to say so and silence the diagnostic — an **explicit copy
intent** (e.g. a `.copy()` / `own(...)` form, exact syntax TBD). The contract is
symmetric: the compiler indicates copies it forces; the user indicates copies they
intend. A copy is acceptable when *someone* declared it — never when it is silent.

## What counts as "a structure copy" (scope)

Warn on a deep copy of a **heap structure** — a record, or a vector of records (allocate
a fresh store + copy every element). **Every** such copy, regardless of static size:
the runtime size is unknown and unbounded (reason 1 above), so there is no honest
size threshold. The line is *deviation from the model*: warn when the lowering chose a
**copy for a value the model says should alias** (a heap structure). Do **not** warn on
**scalars** (`integer`, `single`, a small all-scalar struct) — there a by-value copy *is*
the model ([OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md): scalars copy), so it is neither a
surprise nor a meaningful cost.

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
| F4 | Too many warnings → noise → feature disabled | NOT a size threshold (runtime-sized; a "small" copy can be huge) — control noise by making borrows the default (copies become rare) + explicit copy-intent opt-out; scalars are already out of scope |
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

## Open decisions (for the user)

1. **Coverage first or warning first?** The warning is only as complete as the
   decision. Either extend the copy-vs-borrow decision to all structure copies before
   shipping the warning, or ship a *scoped* warning (vector-copy bindings only) that
   states its own scope. Recommendation: land the borrowed-return codegen fix + widen
   the decision first; a partial warning that looks complete is worse than none.
2. **Default severity:** warning vs opt-in lint (`--warn-copies`)? A new warning on
   existing code that copies will be loud at first. Recommendation: opt-in lint until
   borrows are the default, then promote to a default warning.
3. **Explicit-copy syntax** (F5) — what is the surface form (the inverse of `&`: opt
   into an independent copy and silence the warning).

## Probes run while writing this (the prediction-vs-reality record)

- *Claim: copies route through one decision.* Probe: grep the emission sites —
  `OpCopyRecord`/`OpAppendVector` appear at ~20 sites across parser+codegen; the
  *decision* (`Verdict`) is one place but scoped to vector-copy bindings. → the
  coverage gap above is real and is the #1 risk, not an afterthought.
- *Claim: the reason for a forced copy is already available.* Probe: `VerdictRow.reason`
  exists and carries human-readable justifications today. → the forced/avoidable split
  is cheap to surface for the covered cases.
