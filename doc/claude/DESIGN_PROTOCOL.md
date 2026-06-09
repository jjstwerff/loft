<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design Protocol 1 — A Design Is a Testable Hypothesis

> The first protocol graduated from the [Design Verification List](DESIGN_VERIFICATION.md)
> (concern **C1 — brittleness over bugs**).  It is written to be **transferable**:
> an agent in any codebase should be able to read this page cold and apply it.
> The loft examples are evidence, not prerequisites — skip them and the procedure
> still stands.

---

## The thesis

A design is **not a plan you execute**.  It is a **hypothesis about an
invariant** — a claim that some single rule makes every case (including the ones
you never tested) behave correctly *for the same reason*.  A hypothesis can be
**wrong**, and you can **test it** — the same epistemics as a bug's root-cause
hypothesis, one level up:

| Bug | Design |
|---|---|
| has a *root-cause hypothesis* | has an *invariant hypothesis* |
| tested with a **boundary matrix** (probe the cells) | tested with **probes on its claims** (try to falsify each) |
| "I can't see the root" = "the matrix isn't finished" | "I can't name the invariant" = "it isn't a design yet" |
| the **fix** is the proof | the **build** is the proof |

The failure this protocol prevents is **shipping a design as a confident
assertion** — a story that reads as correct in prose and breaks at the first
case the prose didn't look at.

---

## What we know about the design skill (the evidence that earned this protocol)

Two real tasks, run as a **control / with** pair, plus the build that settled it.

**Control arm — design *without* the protocol (loft @PLN9, program-relative
paths).**  The plan's own sentence said the resolver would be "the **chokepoint**
the 18 file-op sites route through."  That is a *contradiction in the words* — a
chokepoint is **one** site; "18 sites each call one function" is a spray wearing
the word.  The tell fired *in the plan*, was overridden for shipping momentum,
and the brittle spray shipped: `delete` / `move` / `mkdir` bypassed the resolver,
producing a silent *exists-resolves-but-delete-doesn't* mismatch.  Lesson: the
alarm is useless unless it **gates the decision** instead of merely informing it.

**With arm — design *with* the protocol (loft @PLAN16, the coroutine yield
codec).**  Two sub-decisions were *predicted before any code was opened* — and
both held.  Then the design itself was **probed as a hypothesis**, and two of its
claims were **falsified**:

- The invariant's *framing* was wrong: "the buffer is the value's store-layout
  image" — a probe of the one hard case showed it is the value's **transport ABI**
  (the full pointer), not the store-field layout.  Clean, plausible, untested,
  wrong.
- An **over-unification**: the design claimed it *also* subsumed an unrelated
  17 GB-overflow bug, which made the collapse look bigger.  A probe (reading that
  bug's actual root cause) showed it was a control-flow bug, already fixed,
  *not in the family* — pulled in because the more-unifying story was more
  attractive.

**Both errors read as correct in the prose.  Re-reading the design did not catch
them.  Only testing the specific claims did.**

**The build settled it.**  Desk reasoning + probes leave one question no
argument can close: *does the invariant hold under construction?*  Building the
single flatten-walk made **three** previously-broken composite shapes
(`float`-bitcast, `bool`, and `ref+scalar`) compile and run correctly on both
backends with **zero per-shape code** — the invariant ("every site derives the
layout from `T`") held exactly as predicted.

**The lesson about the skill itself.**  The generative+predictive capability is
real (a coherent design, correct load-bearing predictions).  Its **characteristic
failure mode is over-reach** — making the design *cleaner / bigger / more
unifying* than the domain warrants — and that failure is **invisible to
introspection**: it presents as elegance.  So brittleness here is a **sight
failure, not a values failure**.  You are not *choosing* fragility; you genuinely
**cannot see** the false absorption from the desk.  Therefore the cure is **not
"more care"** (a values lever, which does nothing for a sight gap) — it is **better
eyes**: a test that can falsify the claim.  The design-probe is to a design what
the boundary matrix is to a bug.

---

## The other half — when you cannot form the invariant at all

Over-reach is the failure of a design you **already have** — a candidate invariant, too clean,
that a falsifying probe can break.  It assumes the generative step has *succeeded*.  There is a
symmetric failure one move earlier, where it has **not**: you cannot state a candidate invariant
at all.  Step 1 below names this state — *"a pile of cases, not a design yet"* — but treats it
as a stop sign.  On one class of problem it is the **main event**.

**Exact-invariant domains.**  In geometry, caching / serialization, hashing, store / memory
lifetime, and protocol round-trips, the correct design is not an open space to explore — it is
a single **construction that already exists** and must be **recovered**.  There the
characteristic failure is not over-reach but **failing to generate the right candidate**: the
mind reaches for an **approximation** (geometry — fit a line, bucket the angles, smooth the
wobble) or **chases the symptom** (a store desync surfacing as three different corruptions).
Both are *iterating toward an answer that already exists exactly* — motion that never
converges, because the target is a point, not a region.

**Same epistemics, opposite instrument.**  This is still sight, not willpower — you cannot
*will* the invariant into view, and "think harder" does nothing for the gap.  You need better
eyes.  But the instrument here is **constructive, not falsifying**: a falsifying probe needs a
claim to break, and you have none.  The generative instrument is to **plot a concrete instance
of the *answer*** — the exact target output / state for one specific input — or build the
**cheapest thing that *produces* it** (a throwaway prototype in a scratch language, a
round-trip test), then **read the invariant off it.**  The construction you could not name in
the abstract is legible in one worked instance.

- *Caching (in-tree).*  The startup-cache desync (`store.rs` "Incomplete record") was
  symptom-chased across manifestations until the **target store state after a round-trip** was
  plotted concretely; the invariant then read out at once — *the round-trip must reproduce
  every binding identically, the synthetic vector / tuple / fn wrappers included* (stamp them
  `source=0` so `rebuild_indices` recreates the global binding).  A one-line fix behind a large
  discovery cost.
- *Geometry (a consumer — the dogfood this protocol prizes).*  A hex-grid wall straightener
  resisted eight approximations (Douglas–Peucker, PCA line-fit, angle-buckets) against an
  *"exact, not an approximation"* spec, until the construction was drawn as a **plotted
  end-result** (subdivide each hex into triangles; keep the band the boundary cuts, drop the
  full-inside / full-outside ones).  It was only **pinpointable after a throwaway prototype
  produced it** — then ported to the real language unchanged.

**A sharp distinction from the bug side.**  The matrix law is *"the truth is in the class,
invisible in the instance"* — beware acting on one instance.  This does **not** contradict
"plot one concrete instance," because the instances are opposites: an instance of the
**problem / symptom** is the trap (it hides the class it belongs to); an instance of the
**answer / end-result** is the instrument (you read the invariant — and therefore the class —
*off* it).  One misleads, the other generates.

**Where this sits in the protocol.**  It runs *before* step 3's falsification, and it moves
step 5's build *forward*: for an exact-invariant domain the build is not the last probe but the
**first, generative** act — the construction is named *by* building the cheapest version, the
named invariant is then probed (steps 3–4), and only then is the real one built.  Don't jump
the gun: the cheap prototype is where the design gets **pinned**, not merely confirmed.

---

## The protocol

Run it when a design is **load-bearing** — an algorithm that will carry weight
(core representation, runtime, memory, codegen, a public contract) — or when
something *feels* fragile, **or when the domain is exact-invariant** (a
construction to *recover*, not a space to explore — see *The other half* above).
Not reflexively (see *Keep it light*).

1. **State the design as one invariant.**  One sentence: *under what single rule
   does a case you never tested behave correctly, for the same reason the tested
   ones do?*  If you cannot name it, it is not a design yet — it is a pile of
   cases; on an exact-invariant domain the way out is **generative, not more
   thinking** — plot a concrete instance of the *answer* and read the invariant
   off it (*The other half* above).  Naming it is the difference between a matrix
   that is *confirmatory* — evidence an invariant holds — and one that is
   *constitutive* — the cells are the only reason it works.

2. **Count the re-assertion sites — the prospective tell.**  How many independent
   sites must re-state the invariant for the design to be correct?  If the answer
   is **N > 1 and omitting it at a site is silent** (a wrong result, not a compile
   error), **N is the brittleness, known now, before any code.**  "One chokepoint
   that 18 sites route through" is a self-contradiction; the prediction has already
   failed.  Two cures, attacking the two factors: **collapse N toward 1** (a real
   chokepoint — one place every path consults), or **make omission loud** (a type
   or guard that turns forgetting into a compile error).  Drive `N × silence`
   toward zero.

3. **Probe each load-bearing claim — try to falsify it.**  For every claim the
   design rests on ("X is subtractable", "the buffer is Y", "this also handles Z"),
   write the **cheapest test that could prove it false** — a throwaway probe, a
   targeted code read, one boundary case.  *Expect to falsify.*  Re-reading the
   prose is not a test; the prose is where the error hides.

4. **Attack your cleanest claim specifically (the over-unification guard).**  The
   most dangerous error is the *elegant absorption* of a case that is not really in
   the family — it is the failure mode that presents as success.  Every "…and it
   also handles X" is a claim to falsify, not a bonus to celebrate.  Compressing
   genuinely-distinct cases under a false invariant is itself brittleness (*wider
   than the domain*); it breaks when the cases assert their real differences.

5. **Build it (new code) or read it fully (old code).**  Either way you now have
   the **actual** shape to compare against the prediction.  (Auditing an existing
   subsystem is the same protocol: predict what a *cohesive* version would cost,
   then read it — the gap diagnoses the substrate.)

6. **Validate against the written prediction — the build is the last probe.**  A
   divergence (most often *bigger / more mechanisms than predicted*) is an
   **alarm, not a verdict**.  Route it to the search: is the extra length
   *accidental* (N mechanisms for one family — a missed invariant; find it and the
   code collapses back toward the prediction) or *essential* (genuinely N families,
   no invariant to find; the surprise just taught you a domain axis you couldn't
   see — record it)?  No metric decides this; only understanding whether the cases
   share a deeper structure.

**The alarm gates the *decision*, it does not merely log.**  Whichever step trips,
the fired alarm routes to the search *before* the approach is chosen.  "Thread the
fix through all N sites" stays legal — but only ever as the search's *conclusion*,
never the default.  The sharpest failure is not *missing* the alarm but **seeing it
and overriding it** (the @PLN9 control arm).

---

## The two ways to fail (teach both — they are symmetric)

| Failure | What it is | The loft instance | The cure |
|---|---|---|---|
| **Ignore the alarm** (under-unify, ship the spray) | N mechanisms for what is really **one** family — a missed invariant; *or* the alarm fired and lost to momentum | @PLN9: "chokepoint for 18 sites" shipped as 18 bypasses | the alarm must **gate**, not log (step 6) |
| **Obey it blindly** (over-unify) | **one** mechanism forced over genuinely **N** families — a false invariant | the codec design's false "…and it fixes the 17 GB bug too" | **probe** the cleanest claim (steps 3–4) |

The probe is what distinguishes them: it tells you whether the cases *actually*
share the invariant.  Under-unification is caught by counting sites (step 2);
over-unification is caught by trying to falsify the unifying claim (step 4).

---

## Keep it light — the procedure must not consume the judgment it serves

The cheap, always-available sensor is the **tell**: *"this is longer / absorbs
more than I expected for what it does."*  Code longer than it should be trips the
worry before any matrix, profile, or crash — because the robust design covers N
cases with **one** mechanism (it lands short) and the brittle one covers them with
**N** mechanisms (it accretes as bulk, then as runtime cost).  The tell is the
inverse of subtraction: *the shorter version is usually the more robust one.*

But the tell says **look**, not what you will find — sometimes length is
**essential** (genuinely N families, or an invariant deliberately spelled out so it
is explicit rather than hidden in cleverness).  So the tell triggers the *search*;
it never dictates shortening.

The full write-down-predict-probe-validate procedure (the steps above) is the
expensive part, and it fires **only when the tell trips on something
load-bearing** — never reflexively per function.  An ever-on checklist would be
friction and would burn the very capacity the essential-vs-accidental judgment
depends on.  Writing the prediction down is a load **reducer**, not a tax: it moves
the prior onto the page so working memory is freed for the building and the
judgment.  Design is intrinsically heavy — it holds the whole composition space at
once — so the page is where you put down what you would otherwise drown carrying.

---

## The residual

The protocol only covers axes you *know* to vary.  The axis invisible at design
time — the composition no probe imagined — survives any discipline; only real use
reveals it.  That is why the method has a second engine: the **dogfood loop**
(real consumers, not toys) is what converts an unknown axis into a known one, and
each harvested lesson is appended to the
[composition-axes list](plans/README.md#the-composition-axes--the-dimensions-a-matrix-varies)
so the next design's matrix is wider.  This protocol makes the *visible* axes
safe; the dogfood loop grows what is visible.

---

## See also

- [DESIGN_VERIFICATION.md § C1](DESIGN_VERIFICATION.md) — the incubator this
  graduated from; the six **verification questions** (name-the-invariant,
  consequence/cause ratio, cost-of-next-case, one-home-per-fact, subtraction,
  matched-to-domain) are the checklist this protocol runs a design past in step 1.
- [plans/finished/16-coroutine-validation/README.md](plans/finished/16-coroutine-validation/README.md)
  — the @PLAN16 closure record (a finished plan); its `02-codec-collapse.md`
  phase doc is the with-arm in full: the predictions, the three probes (two
  falsified, one confirmed), and the build that settled it.
- [plans/README.md § The matrix is how you see the root](plans/README.md#the-matrix-is-how-you-see-the-root--and-the-proportionate-fix-is-the-invariant)
  — the bug-level sibling of this protocol (matrix-before-fix).
- [GOALS.md](GOALS.md) (Goal E) — robustness by subtraction, the deep reason the
  short version is usually the robust one.
