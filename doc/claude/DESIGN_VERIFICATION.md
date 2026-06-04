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

**Status:** recorded concern; protocol pending verification.  First test: the
string/text-allocation memory-bound work (a brittleness problem — works in the
tested regime, OOM-cliffs when the load pattern shifts; the shape `Stores` had
before plan-57).

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

**See also:**
[plans/README.md § The matrix is how you see the root](plans/README.md#the-matrix-is-how-you-see-the-root--and-the-proportionate-fix-is-the-invariant)
·
[plans/README.md § The composition axes](plans/README.md#the-composition-axes--the-dimensions-a-matrix-varies)
· [GOALS.md](GOALS.md) (Goal E) ·
[CLAUDE.md § Debugging policy](../../CLAUDE.md)
