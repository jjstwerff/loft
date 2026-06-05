---
name: engineering-rigor
description: >-
  The shared discipline for any non-trivial change — fixing a bug OR
  designing/refactoring a load-bearing algorithm. A coherent explanation is a
  hypothesis, not a conclusion; acting on the first one ships fragility you
  couldn't see. Before acting, build the instrument that makes the whole CLASS
  visible (a boundary matrix for a bug, falsification probes for a design, a usage
  sentinel for any "which code actually runs?" question), find the ONE invariant,
  enforce it at the chokepoint (no narrower, no wider), and verify against what you
  wrote down. USE THIS whenever you are about to fix a
  non-trivial bug (especially a crash, silent corruption, or wrong result),
  design or refactor a load-bearing algorithm (core representation, runtime,
  memory, codegen, a public contract or data format), or whenever a change feels
  fragile or "longer than expected for what it does" — even when nobody asked
  for rigor and the first fix looks obvious. Routes to the matrix-first debug
  protocol and Design Protocol 1; does not replace them.
user-invocable: true
---

# Engineering Rigor — See the Class Before You Act

## The one idea

**A coherent explanation is a hypothesis, not a conclusion — most of all an
elegant one.** Whether you are fixing a bug or designing a system, the failure is
the same: you act on the first story that fits, and that story was built from the
*one instance in front of you*. It is correct for that instance and silent about
the cases you could not see — so it breaks on them.

This is a **sight discipline, not a willpower one.** You do not ship fragility on
purpose. You ship it because a single instance cannot show you the class it
belongs to. So the cure is never *"be more careful"* (a values lever — it does
nothing for a sight gap). The cure is **better eyes**: build the instrument that
makes the whole class visible, then act on what it shows. The instrument is the
organ of sight; the rest of this skill is how to build it and what to do once you
can see.

## Two modes, one shape

The discipline has two faces depending on whether you are *explaining behavior
that exists* or *choosing behavior that doesn't yet*. They are the **same shape
one level apart** — learn the column you're in, but know it's one method.

| Step | DEBUG — explain existing behavior | DESIGN — choose future behavior |
|---|---|---|
| The thing you have | a symptom (crash, wrong result, corruption) | a problem + a candidate design |
| The trap | fix the first cause you find | build the first coherent design |
| The hypothesis | a **root cause** | an **invariant** |
| The instrument (your eyes) | a **boundary matrix** — vary one axis per probe | **falsification probes** — try to break each claim |
| Where the truth hides | in the *class*, invisible in any one repro | in the *class*, invisible in the prose |
| The act | fix at the chokepoint, enforce the invariant | build the chokepoint every site derives from |
| Verify against | the full matrix, on every mode the result can diverge across | the written prediction |

## The shared loop (run it in either mode)

1. **Don't act on the first read.** A coherent story — especially an elegant
   one-line fix or a clean unifying abstraction — is the *signal you haven't
   earned the action yet*. Real bugs are complex-variant; real designs carry
   weight you haven't probed. The clean story is usually the part of the picture
   you haven't looked at.

2. **Build the instrument — it is your eyes.** Debug: a boundary matrix of `/tmp`
   probes, varying **one axis per probe** (type-kind, construction-path, depth,
   null, backend — the composition axes). Design: the cheapest test that could
   *falsify* each load-bearing claim — a probe, a targeted read, one boundary
   case. Control-flow (*which code actually runs*): a **usage sentinel** — route
   the uses through one chokepoint and make it loud (its own § below).
   *"I can't see the root / can't name the invariant yet"* means **the
   instrument isn't finished** — never a license to act on the one case in hand.

3. **The truth is visible in the class, invisible in the instance.** The shared
   mechanism behind a family of "different" symptoms shows up *in the matrix* and
   in no single repro. The false unification in a design ("…and it also handles
   X") shows up *under a probe* and in no re-reading of the prose. This is *why*
   step 2 is non-negotiable: the thing you're hunting is a property of the class,
   so you must be able to see the class.

4. **Find the ONE invariant; enforce it at the chokepoint — no narrower, no
   wider.** *Narrower* (a per-case/per-type patch; an N-way spray) leaves the
   siblings broken — the same problem, unfinished. *Wider* (re-resolving more
   than the failing region; unifying genuinely distinct cases under a false
   invariant) drags blast radius and is its own brittleness. The proportionate
   move enforces *exactly* the invariant the whole failing region violates.

5. **Verify against what you wrote down**, not against the action's own momentum.
   Debug: re-run the full matrix on **every mode the result can diverge across**
   — for a system with more than one execution path (e.g. an interpreter and a
   compiler), each path, because cross-mode divergence is real. Design: compare
   the built thing to the **written prediction**. An external commitment is what
   makes the check honest — without it you grade your own homework.

## A third instrument: the usage sentinel ("which code actually runs?")

The matrix varies **inputs** to make a behavior-class visible. Some questions aren't
about inputs — they're about **control flow**: which code actually runs, which
sites route through a point. There the obvious instrument lies: a **static count**
(`grep` the call sites) **over-counts** — it sees dead and live uses with the same
eyes, so it can't say which still *fire*.

What can is a **usage sentinel** — route every use through one observable chokepoint
and make it loud (count it + name the caller, or trip under a flag). One run turns a
control-flow *guess* into a runtime *fact*: the chokepoint rule (step 4) aimed at
**observability**, not enforcement. The same fact answers three questions — removal is
only one:

| Use | Sentinel question | The answer you want |
|---|---|---|
| **Removal** | is anything still using this? | **none** — zero across the whole suite → safe to delete |
| **Debug** | does the path I *think* runs actually run? | **the expected sites fire** — catches a fast path that's never taken, a handler that's secretly dead |
| **Design** | is my chokepoint the *sole* route every site takes? | **all sites, only here** — the runtime form of one-home-per-fact |

**Silence is evidence only after a positive control.** A silent sentinel reads the
same whether the consumer is dead *or the sentinel sits on a dead path* — so prove it
*can* fire first. Removal gets this free (the whole-suite run is the control — if
anything used it, you'd see it); debug and design must show the chokepoint fires for a
known-live case before trusting a zero. Skip it and you call a handler "dead code"
when you've only proven "my probe never reached it."

## The two ways to fail (symmetric — both modes share them)

| Failure | What it is | Caught by |
|---|---|---|
| **Under-reach** | N mechanisms for what is really **one** family (a spray, a patch per case) — *or* the alarm fired and lost to momentum | counting the re-assertion sites; the matrix showing the sibling cases |
| **Over-reach** | **one** mechanism forced over genuinely **N** families — a false invariant that breaks when the cases assert their differences | a probe that tries to *falsify* the unifying claim |

The instrument is what **distinguishes** them: it tells you whether the cases
actually share an invariant. Under-reach is the failure of the lazy; over-reach
is the failure of the clever. Both are brittleness; both are invisible without
the matrix/probe.

## The tell (cheap, always-on sensor)

The robust version covers N cases with **one** mechanism, so it lands *short*. The
brittle version covers them with **N** mechanisms, so it *accretes* — and the bulk
is the first thing you feel: **"this is longer / more than I expected for what it
does."** Treat that as the alarm to *look* — before any matrix, profile, or crash.

But the tell says **look, not shorten.** Sometimes length is *essential* (genuinely
N families, or an invariant spelled out so it's explicit rather than hidden in
cleverness). The tell triggers the search for a missing invariant; finding one →
shorter + robust; honestly finding none → the length is essential, accept it. The
alarm did its job by making you check.

## Keep it light — the procedure must not consume the judgment it serves

The *tell* is free and always on. The full build-the-instrument procedure is the
expensive part, and it fires **only when the tell trips on something
load-bearing** — never reflexively per function. An ever-on checklist is friction
and burns the very attention the essential-vs-accidental judgment needs. Writing
the prediction / matrix down is a load *reducer*: it moves the prior onto the page
so working memory is free for the building and the judgment.

## Which mode am I in? (and they compose)

- **A discrepancy exists** — something crashes, corrupts, or returns the wrong
  answer → **DEBUG.** You are explaining behavior.
- **You are choosing behavior** — a new algorithm, a refactor, a data format, a
  public contract → **DESIGN.** You are committing to an invariant.
- **They compose.** A bug fix that touches a load-bearing algorithm is *both*: the
  matrix finds the class, and the proportionate fix is itself a small design whose
  invariant you should be able to name. When an investigation reveals the real fix
  is a substrate change, you've crossed from debug into design — pick up the design
  column without dropping the matrix.

## The residual (why this isn't enough on its own)

The instrument only covers axes you *know* to vary. The composition no probe
imagined survives any discipline; only real use reveals it. That is the second
engine: the **dogfood loop** (real consumers, not toys) converts an unknown axis
into a known one, and each harvested lesson widens the next matrix. This skill
makes the *visible* axes safe; the dogfood loop grows what is visible.

## Go deeper — route here, don't reinvent

This skill is the **synthesis + the router**. Everything above is **tree-agnostic
— the discipline is identical in any codebase.** The routes below are specific to
*this* repository (loft); carrying this skill into another tree (e.g. loft2) means
keeping the body unchanged and repointing these links at that tree's equivalents —
its debugging policy, its design docs, its test layout. The depth lives in the
canonical docs; read the one for your mode.

- **DEBUG method** — `CLAUDE.md` § "Before fixing a non-trivial bug: build the
  boundary matrix" (the matrix-first protocol, the chokepoint-invariant rule).
- **What to vary in the matrix** — `doc/claude/plans/README.md` § The composition
  axes.
- **DEBUG mechanics (loft)** — the `loft-debug` skill: `LOFT_LOG` presets, dump
  files, `--interpret`-first seeing loop, native-env traps that fake failures.
- **DESIGN method** — `doc/claude/DESIGN_PROTOCOL.md` (Design Protocol 1 — A
  Design Is a Testable Hypothesis): name the invariant, count the re-assertion
  sites, probe to falsify, build, validate against the prediction.
- **DESIGN reference** — `doc/claude/DESIGN_VERIFICATION.md` § C1: the six
  verification questions (name-the-invariant, consequence/cause ratio,
  cost-of-next-case, one-home-per-fact, subtraction-not-a-guard, matched-to-domain)
  and the countable brittleness form.
- **Heavyweight investigations** — `doc/claude/plans/_INVESTIGATION_TEMPLATE.md`.
- **The deep why** — `doc/claude/GOALS.md` Goal E: robustness by *subtraction* —
  the reason the short version is usually the robust one.
