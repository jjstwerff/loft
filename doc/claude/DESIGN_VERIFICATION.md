<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design Verification List

A running list of **design concerns** to check a design against — *before* it
ships, when the design feels load-bearing or you doubt it will hold under change.
It is **not a protocol and not a gate.**  Ignore it most of the time; pull it out
when a design is non-trivial, when an algorithm will bear weight (stdlib,
runtime, memory, codegen), or when something *feels* fragile and you want the
question made explicit.

Two uses:

1. **Reference** — run a design past the relevant concern's verification
   questions.  Most designs touch one or two concerns, not all of them.
2. **Incubator** — each concern is a *candidate* protocol.  A concern earns
   promotion to a real protocol ([CLAUDE.md § Debugging policy](../../CLAUDE.md),
   [`plans/_INVESTIGATION_TEMPLATE.md`](plans/_INVESTIGATION_TEMPLATE.md),
   [`plans/README.md`](plans/README.md)) only *after* a real consumer has
   verified the way-of-working.  Until then it lives here as a recorded concern,
   not a rule.

**Append, don't prune.**  New concerns are appended as they are discovered — the
same way the
[composition-axes list](plans/README.md#the-composition-axes--the-dimensions-a-matrix-varies)
grows, each entry something we once got wrong and learned to look at.  A concern
leaves this list only by *graduating* into a protocol, and then its entry here
shrinks to a one-line pointer to where the protocol now lives.

---

## C1 — Brittleness over bugs

**Status:** *the primary design concern.*  Recorded; protocol pending
verification.  First test: the string/text-allocation memory-bound work (a
brittleness problem — works in the tested regime, OOM-cliffs when the load
pattern shifts; the shape `Stores` had before plan-57).

**The concern.**  The real risk in a design is not *bugs* (inevitable, discrete,
local — caught per-cell by a matrix) but **brittleness**: an algorithm correct
*for the tested cases and for local reasons*, with no margin, that **breaks
rather than bends** under the input / scale / composition you didn't foresee.  A
brittle algorithm can be 100% bug-free and still be the larger problem — and a
*fully green matrix can be maximally brittle* if each cell passes for its own
local reason (a special-case per type/path is a lookup-table of patches,
guaranteed to break at the first un-enumerated point).

Brittleness is a **sight failure, not a values failure**: it is produced by
missing the full picture, not by choosing fragility.  The
[matrix](plans/README.md#the-matrix-is-how-you-see-the-root--and-the-proportionate-fix-is-the-invariant)
is the organ that makes the whole class visible so a unifying invariant can
surface — *see the family, then enforce its invariant*.  You cannot enforce the
invariant over a class you cannot see; the matrix is the precondition of the
robust fix, not a sibling to it.

**The early tell — length against expectation, before the matrix and before the
crash.**  The brittle version is *almost always longer than you expect, and less
efficient* — and the **length is the first thing you feel**: code longer than it
should be for what it does already trips the worry, before any matrix, profile, or
crash.  That is not a coincidence; it is the inverse of subtraction (question 5).
The robust design covers N cases with **one** mechanism (one invariant, one path,
the coincidence deleted), so it lands *short*.  The brittle design covers them with
**N** mechanisms (a special-case per case — question 3, O(N sites); a guard per
disagreement; a branch per type), so it *accretes* — and that accretion shows up
immediately as **bulk** (and then, at runtime, as **cost**).  So treat "this is
longer than I expected for what it does" as the first **brittleness alarm**: stop
and look for the invariant you're missing — the shorter version is usually the more
robust one.

**But this is a prior, not a verdict — which is why it stays a hard problem.**  The
length tell says *look*, not what you will *find*.  Sometimes a right implementation
genuinely needs a lot of code, and the real judgment is **essential vs accidental
length**.  *Accidental* (brittle) is N mechanisms for cases that are really **one
family** — a missed invariant; finding it makes the code shorter and more robust
together.  *Essential* (correct) is N mechanisms for genuinely **N families** (no
invariant exists to find), or code spelled out to make an invariant **explicit**
where the short version would hide it in cleverness (clever-short is brittleness too
— Goal E cuts both ways), or the honest cost of covering the **full input space**
instead of betting on the unenumerated cell.  No metric decides which it is — it
takes actually understanding whether the cases share a deeper structure (the matrix
shows the cases; the *collapse* is the hard insight).  So the alarm triggers the
**search** for the invariant; it never dictates shortening.  Search and find one →
shorter + robust.  Search honestly and find none → the length is essential, accept it
— the alarm did its job by making you check.  The two ways to fail: **ignore** the
alarm (ship accreted brittleness), or **obey it blindly** — compress
genuinely-distinct cases under a false invariant, which is itself brittleness
(question 6, *wider than the domain*), breaking when the cases assert their real
differences.  (A deliberate, measured fast-path is the remaining exception:
brittle-but-faster by intent.)

**Verification questions** (run a design past these):

1. **Name the invariant.**  Can you state, in one sentence, the single invariant
   under which a case you *never tested* would behave correctly, for the same
   reason the tested ones do?  If not — green matrix or not — the design is
   brittle: the matrix is *constitutive* (the cells are the only reason it works)
   rather than *confirmatory* (evidence that an invariant holds).
2. **Consequence/cause ratio.**  Does a small perturbation cause a catastrophic
   divergence?  (plan-58: `i32`→`i16`, a 2-byte change, produced a wild pointer /
   SIGSEGV.)  Robust = bounded consequence for bounded perturbation.
3. **Cost of the next case.**  Is a new type / path / scale absorbed by the
   existing invariant (O(1)), or does it need a fresh special-case per addition
   (O(N sites))?  O(N sites) is the brittleness meter.
4. **One home per fact.**  Is each fact the design introduces derived in **one**
   place every path consults, or re-derived in several?  A fact re-derived in N
   places is N−1 disagreement bugs waiting for the input where the derivations
   split (the plan-58 stride class).
5. **Subtraction, not a guard.**  Is robustness won by *removing* the second
   source of truth (so nothing can disagree), rather than adding a guard that
   *catches* the disagreement (brittle-with-a-net)?  The robust version is
   usually **simpler** — this is [Goal E](GOALS.md): remove hidden machinery,
   don't add cleverness.
6. **Matched to the domain.**  Not *narrower* (breaks inside the domain) and not
   *wider* (an abstraction carrying weight nothing rests on — over-bought
   robustness is its own brittleness).

**The residual.**  These questions only cover axes you *know* to vary.  The axis
invisible at design time — the composition no probe imagined — survives any
discipline; only real use reveals it.  The
[composition-axes list](plans/README.md#the-composition-axes--the-dimensions-a-matrix-varies)
is the accumulated boundary of the visible; the **dogfood loop** is the engine
that converts an unknown axis into a known one — real consumers wander into
regions invisible from the desk, and each harvested lesson is a new axis.  "Real
consumers, not toys," because a toy only exercises the axes already visible.

**Candidate protocol shape (graduation target — not yet active).**  When a real
consumer has verified the way-of-working, the protocol this concern graduates into
mirrors the matrix-before-fix shape one level up — *commitment before action, so the
action is checked against something other than its own momentum*:

1. **Before writing the code, write down the expected shape** — the one invariant you
   expect to carry it, the axes it must cover, and a rough size/structure (one
   function? one match? a table?).  If you can't name the invariant up front, that is
   already the first flag.
2. **Build it.**
3. **Validate the build against the written shape.**  A divergence — most often
   *bigger / more mechanisms than predicted* — is an alarm, **not a verdict**: route
   it to the essential-vs-accidental search above.  Find the invariant → the code
   collapses back toward the predicted shape.  Find none honestly → the length is
   essential, and the surprise just taught you a domain axis you couldn't see at
   prediction time; feed that into the next estimate, and into the
   [composition-axes list](plans/README.md#the-composition-axes--the-dimensions-a-matrix-varies)
   if it is a new one.

The written prediction is what turns "longer than expected" into a concrete,
falsifiable comparison instead of a feeling you can rationalise away after the fact —
and it **externalises the prior**, so the check works even when the gut estimate
doesn't.  (Step 3 stays a *search*, never a pass/fail on size — a length protocol that
asserted "match the prediction" would be the over-unification this concern warns
against, applied to itself.)

**Keep it light — the procedure must not consume the judgment it serves.**  The
*tell* ("longer than expected") is the cheap, always-available sensor; the full
write-down-predict-validate procedure is the expensive part, and it fires only when
the tell trips on something load-bearing — never reflexively per function.  An
ever-on checklist would be friction ([Goal F](GOALS.md)) and would burn the very
capacity the essential-vs-accidental judgment depends on.  And writing the shape down
is a load *reducer*, not a tax: it moves the prior onto the page so working memory is
freed for the building and the judgment.  Design is intrinsically heavy — it holds
the whole composition space at once — so the page is where you put down what you'd
otherwise drown carrying.  (Robustness by subtraction, applied to attention.)

**How it graduates — by measurement, not assertion.**  Whether this procedure *helps*
or *overloads* is itself a claim to test like a debug matrix, not to assert.  Run real
design tasks (starting with the string-allocation invariant) **with** the
predict-validate step and, where feasible, a **without** control, and compare on
*artifacts, not on how it felt* (self-report on one's own load is unreliable): outcome
robustness (did the written prediction catch a brittle version the control shipped?),
final code shape (lines / mechanisms / is the invariant nameable?), and cost (effort
spent vs brittleness prevented).  The pattern across tasks decides it — and if it helps
on load-bearing designs but overloads on trivial ones, that *is* the empirical basis
for the tell-gates-the-procedure split above.

**See also:**
[plans/README.md § The matrix is how you see the root](plans/README.md#the-matrix-is-how-you-see-the-root--and-the-proportionate-fix-is-the-invariant)
·
[plans/README.md § The composition axes](plans/README.md#the-composition-axes--the-dimensions-a-matrix-varies)
· [GOALS.md](GOALS.md) (Goal E) ·
[CLAUDE.md § Debugging policy](../../CLAUDE.md)
