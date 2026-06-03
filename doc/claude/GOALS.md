<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# GOALS.md — what "loft stable" means, made concrete

The north star for loft is **stability**.  This document splits that one word
into four goals.  Each goal carries a **Check** — a command to run or a fact to
observe — so progress is something you *evaluate*, not something you assert.  The
Check is timeless; its result is not recorded here (run it to find out where you
stand today).

## North star

**loft is stable = it is correct now AND stays correct as the world changes
underneath it** — new rustc/LLVM toolchains, new platforms, new consumer code
patterns.  The canonical failure mode this is defined against is latent
undefined behaviour: UB that passes every test on today's toolchain and
surfaces as silent corruption after a compiler bump or on a stricter platform.
"Passes today" is not "stable."

## The two engines (both required; neither finds the other's bugs)

- **Dogfood — "is loft useful?"**  Build real consumers (branch-review viewer,
  tracker indexer, `lib/markdown`, the games moros/dryopea), harvest the
  language lessons, fix the language, ship the harvest.  Finds missing features,
  ergonomic pain, feature-interaction bugs, design inconsistencies — the bugs
  users hit *today*.  (See CLAUDE.md § "Development cadence".)
- **Sanitizer — "is loft safe?"**  Mechanically detect UB (alignment, UAF,
  uninitialised reads, aliasing, leaks) with Miri + the homegrown
  `stack_align_guard` + fuzzing, independent of whether output is currently
  correct.  Finds the *correct-today-corrupt-tomorrow* bugs dogfooding is blind
  to.

The two engines feed four goals: **A** (soundness) is the sanitizer engine;
**C** (capability) and **B** (release & adoption) are the dogfood engine; **D**
(parity) is verified by sanitizer-style differential testing but serves both.

---

## Goal A — Soundness (no silent corruption)

**Definition.** loft never produces wrong or corrupt results from undefined
behaviour, and stays that way across toolchain bumps.  This is a **continuous**
property: it is satisfied by a sanitizer engine that runs and keeps catching new
UB, not by a single clean run.

**Check.**
- `ci.yml`'s per-PR `guard` and `asan` jobs are green on `main` (and on the PR
  under review).
- `cargo test --features stack_align_guard` zero-fires across the whole corpus
  (every test binary, not just `issues`).
- The nightly `miri.yml` Miri gate is green.

Met-and-healthy when all three hold *and keep holding* as new code lands.

---

## Goal B — Release & adoption

**Definition.** loft actually ships on a cadence and is usable by people who did
not write it.  A language that never reaches a stable release, or that only its
authors can run, is not stable in any sense that matters.

**Check.**
- A release tag exists within the project's release cadence (`git tag` →
  latest), each carrying a CHANGELOG.md entry.
- `loft install <name>` resolves and fetches a published library end-to-end
  against the registry.
- A clean-machine on-ramp — install → first program → `use` a library —
  completes from the docs alone.
- *Adoption proxy* (no clean counter exists): at least one library has been
  published by someone outside the project.

---

## Goal C — Capability via dogfood

**Definition.** loft keeps gaining the features and ergonomics real consumers
need, and the canonical consumers keep building and running.

**Check** — the consumer build matrix; each row is a command, score it `N/total`:
- branch-review viewer builds and runs on HEAD;
- tracker indexer (`make index`) succeeds;
- `lib/markdown` suite passes;
- the games moros / dryopea build and run against current loft;
- the last release's CHANGELOG carries a consumer-driven harvest section.

Never "finished"; it reads as a fraction, and a falling fraction is the alarm.

---

## Goal D — Cross-platform + cross-backend parity

**Definition.** loft behaves identically on **ubuntu / macOS-ARM / windows**
across the **interpret / native / wasm** backends — no platform- or
backend-specific divergence.  macOS-ARM is the strict-alignment platform where
unaligned reads are real faults; the three backends are three independent
implementations of the same language semantics.

**Check.**
- The 3-OS CI matrix (`ci.yml`) is green on `main`.
- A differential run executes one shared corpus on interpret / native / wasm and
  asserts **identical output and diagnostics** — zero divergences.  Per-backend
  green is *not* the criterion; agreement between backends on identical input is.

---

## Goal E — Predictable memory (the programmer's model *is* the truth)

**Definition.** The runtime's memory behaviour matches the **obvious reading of
the source**: a value's heap allocation dies when its **scope** dies, with **zero
exceptions a programmer has to learn**.  The model is small enough to hold
completely in your head — write a block-scoped vector and it is freed at block
end, full stop.  This is **distinct from Goal A**: a program can be perfectly
*sound* (no corruption) yet hold far more memory than the source implies because
the runtime quietly retains it (plan-57 clusters I/III — a block-scoped store
pinned to function exit).  Goal A asks "is it safe?"; Goal E asks "**can the
programmer predict it?**"

**Why it's a goal, not a nicety — and the bar this sets.**  The appeal of C is
*locus of control*: the memory model is small, the source is the truth, and when
something goes wrong the fault is the programmer's — knowable and fixable.  Rust
buys safety with a static analysis the programmer must reason about, so debugging
shifts toward "what did the compiler decide?".  Those are welded together in
Rust; you cannot have its guarantee without its machinery.  loft makes a
different bet — safety from a **runtime discipline** instead of a static proof —
which lets the *rule* be the entire model.  The explicit aim:

> **On the one axis of safe AND predictable (programmer-in-control), surpass
> Rust** — not at perf, concurrency, or ecosystem, but here, deliberately,
> because Rust structurally cannot unbundle safety from its opaque machinery and
> loft can.

The discipline that makes or breaks it: the **programmer-facing rule stays
exceptionless** ("a value dies when its scope dies").  *All* sophistication —
last-use / least-common-ancestor confinement analysis, rc handling, slot-vs-heap
decoupling — lives **only in the implementation, invisible**.  The moment the
*rule* grows an "except when captured / iterated / reassigned…" the programmer
must memorise, loft has rebuilt Rust's opacity in a new shape.  When loft's
behaviour surprises the obvious reading of the source, that is a **loft bug**,
never "benign" and never documented-around.

**Check.**
- `LOFT_STORE_GUARD=1` is **silent across the corpus** — no block-confined vector
  store is scoped (and freed) later than the block it is confined to.  (Detector
  shipped; see [plans/future/57-vector-store-watermark/fix-design-store-lifetime.md](plans/future/57-vector-store-watermark/fix-design-store-lifetime.md).)
- That guard is promoted to a `#[cfg(debug_assertions)]` assertion, so the rule
  cannot silently re-acquire exceptions as new code lands — the guard *forbids*
  the divergence, it does not merely report it.

Met-and-healthy when the guard is silent corpus-wide *and* the assertion holds
*and keeps holding*.  A guard that starts firing is the alarm that the model and
the runtime have drifted apart.

### The method mirrors the goal

Goal E's law — *the stated model must match reality; a divergence is a bug, fixed
by **removing** hidden machinery, not by adding cleverness* — is also how loft is
**developed**, not only what it ships.  The same law, one level up, governs our
own reasoning:

- an investigation's stated thesis must match its contents — don't smuggle an
  unrelated bug's fix into it (that makes the ledger diverge from what it claims
  to be);
- a bug verdict must match the bug's **verified** shape — don't assert "zero blast
  radius" over a region you haven't probed.  An unverified confidence is the rc
  *"the count handles it"* gloss wearing a new hat: a clean story laid over a
  reality you didn't check.

The edge-probe-first discipline, the kept probe landmarks, and the sibling-bug
scope hygiene
([plans/README.md § Edge-probe](plans/README.md#edge-probe-before-fixing--the-lightweight-default-for-lofts-complex-variant-bugs))
are this same exceptionless-transparency law turned on our own claims.  This is
not decoration: **a team that tolerates hidden machinery in its own reasoning
cannot credibly ship a language whose whole promise is no-hidden-machinery.**  The
process is the proof of concept for the product.

### Bugs are veils — clearing them is a precondition for Goal E, not a sibling

A bug is not just a broken spot.  It is a **veil**: it blinds you to everything
downstream of it, and — worse — it can make *broken things look fine*.  The
evidence is routine:

- a `parallel {}` block that silently no-ops on `--native` made test-80 and
  test-81 **pass** — the veil ("arms ran, asserts held") hid that native runs *no
  arm at all*;
- a read-only-store crash masks what a heap mutation would actually do;
- an over-retained store under the refcount **never crashes**, so a *wrong free
  site looks correct*.

That last one is the key: **a bug is a local refcount.**  The objection to rc is
not unsoundness — it is that rc *glosses over the lifetime*, sound-looking
machinery that makes a wrong thing look fine.  A leftover bug does exactly that to
the system around it.  "Bugs hide things from our view" and "rc glosses over
details" are the **same objection — transparency — generalised** from one
mechanism to all of them.

So clearing bugs is a **precondition** for Goal E, not a parallel goal: *you
cannot verify "the model is the truth" through a veil.*  Every unfixed bug is a
region where the runtime is doing something you can't see, so it is a region where
the model **cannot be confirmed to hold** — soundness isn't merely violated
locally, your ability to *check it anywhere downstream* is compromised.  This is
why the standing detectors (the sanitizer for A, `LOFT_STORE_GUARD` for E, a
differential backend-parity sweep for D) and the soundness-floor pattern (turn a
silent no-op/crash into a visible compile error) all earn their place: **they are
veil-lifters** — they convert masking into signal so the model becomes checkable.
Lift them in dependency order; the lowest veil is the one hiding the most.

### The wall is usually an old conservatism — narrow it, don't out-clever it

When you hit the same wall repeatedly, the constraint is usually not your current
attempt but a **conservative mechanism that was correct before you had today's
information** and has outlived the gap that justified it.  The store refcount (kept
because the lifetime was unknown) and the body-0 work-ref hoist (function-scoped
because the confined scope was unknown) are the same shape: machinery that
over-reaches to stay safe *without* lifetime information.  The move is not to
iterate harder at the current level or pile cleverness on top — it is to find the
old assumption and **narrow it with the information you now have.**  That is just
Goal E again: free at the real scope once you know it.  Old conservatisms are not
mistakes to blame; they are the surface where new information turns into progress.

---

## Goal F — Friction-free surface (the language serves the programmer, not the compiler)

**Definition.** No syntax, annotation, or blocking error exists to feed the
*compiler's* analysis.  The programmer writes what expresses intent; the
compiler's internal needs — lifetime tracking, confinement proof, slot
assignment — are the **implementation's** problem and stay invisible.  When the
compiler cannot prove what it wants, it **absorbs the cost** — a missed
optimization, a deferred feature — it does **not** hand the programmer a form to
fill in.  Warnings are the one allowed channel: they describe consequences of the
programmer's *own* coding choices and are **freely ignorable**.

**Grounding — the drive beneath the principle.**  Goal F is the language-layer
face of a single motivation that runs the whole stack: *do the hard plumbing
yourself, deeply, so someone else can just pick it up and have fun.*  It is the
same sentence at every layer — **loft** (memory, types, the store done → write the
logic, no ceremony) · the **hex-world library** (terrain, walls, collision,
rendering done → make a world game, no world-infra) · the **editor** (authoring
done → shape worlds) · the **server** (networking done → a gathering game just
works).  Not four projects — **one drive, recursively**; and it is precisely
**lavition's identity**: not "the hex-world engine" but *the engine where the hard
parts are already done.*  It hands the project its own acceptance test, sharper
than any feature list: **a thing is done when picking it up is *fun*** —
fun-on-pickup, not feature-complete.  A library can ship every feature and still
be a fight to hold; that library is not done.  And the *fun* here is **intrinsic,
not instrumental** — held because it is what *finished* means, not to draw a crowd:
this is a **singular idea built for its own sake**, on whatever horizon it takes,
and whether one person picks it up or none, the bar does not move.  Removing
adoption and speed from the judgement is exactly what keeps it pure — every call
decided by *fidelity to the one idea* and *depth*, nothing else.  This is why
friction is **fatal, not cosmetic** — a Goal-F violation means the plumbing isn't finished, so whoever
picked it up gets a fight instead of fun, and leaves.  The **crawler** dogfood
made it literal: its "survival guide" of store-lifetime workarounds
(C1/C3/C4/C18 → [loft#248](https://github.com/jjstwerff/loft/issues/248)) *is* the
plumbing not yet done — each workaround a spot where loft handed the programmer a
fight.  Clearing that family is not hygiene; it is **fidelity to the whole point**,
and *"can they pick it up and have fun"* is the test that tells on-mission work
from work that only looks like it.

**Why — the Rust grievance, stated plainly.**  Rust bought safety by pushing its
analysis onto the syntax: `'a` lifetimes, `move`, turbofish, `Pin` — ceremony
that serves the borrow checker, not the author.  Once that syntax ships it cannot
be walked back; the long-running ergonomics effort is the proof of how hard
un-ringing that bell is.  loft refuses the first step — **rather miss a feature
than impose friction.**  This is *bought* by Goal E's bet — safety from a
**runtime discipline**, not a static proof: no static proof means **no
proof-obligations to discharge on the user**.  E removes the machinery from the
*model*; F removes it from the *syntax* — the same coin.

**The friction test.**  For any syntax or error, ask: *does this serve the
programmer or the compiler?*
- a type on a signature (documents intent), a warning about an unused value (the
  programmer's choice, ignorable) — programmer's, **keep**;
- a lifetime annotation, a `move`, a "restructure it so I can prove it" error —
  the compiler's, **refuse**: infer it, default it, or drop the feature.

**Missing a feature is the *preferred* side — and is not the same as friction.**
Refusing an operation the language **cannot do soundly** (the `parallel{}`
unsound-capture error → "use `for par`") is *missing a feature*, not pushing work
onto the user: it says "this isn't available yet, here is the supported path,"
never "annotate X so I can allow it."  The boundary is exact — an error that
**bounds the language** is fine; an error that **bounds the user into serving the
compiler** is the friction F forbids.

**Check.**  No feature design ever reaches "…and the user must write X so the
compiler can Y."  When it does, the *feature* is wrong, not the user — infer X,
default it, or cut it.  The store-confinement analysis is the worked model: zero
user-facing surface, **silent fallback to a higher watermark** when it cannot
prove confinement — the programmer never learns the analysis exists.

**Relation.**  F is **orthogonal** to the useful→safe→predictable axis — it
constrains the *user-friction cost* of delivering any of A–E.  It is closest to E
(both forbid hidden machinery from leaking out) but distinct in surface: E guards
the **memory model**, F guards the **whole language syntax**.  And it is the
compile-time twin of the runtime rule that recoverable faults *warn-and-continue*
rather than halt — friction at compile time is the same wrong tax as a halt at
runtime.

---

## The two floors — why dogfood is paused, and when it resumes

The dogfood loop is the agenda-setter (CLAUDE.md § "Development cadence"), but it
is currently **paused by deliberate decision** — because the loop did its job and
hit two walls:

- it **kept surfacing instability** → the **soundness floor** (Goal A);
- it **fought the lib/package structure** → the **structure floor** (Goal B's
  packaging half).

Building a game on either un-cleared floor is building on sand.  So Goal C (and
the game work specifically) is gated on A and B — *by choice, not neglect*.

The danger of a deliberate pause is that open-ended floors make it permanent by
inertia: soundness can always absorb one more sanitizer leg, packaging one more
polish.  So each floor has an **explicit resume-bar tied to what a game actually
needs** — not "all of A" or "all of B":

- **Soundness floor — cleared when:** the sanitizer gate is green on `main`
  **and** the curated Miri/ASan set covers the surfaces the games exercise (eval
  stack, store claim/copy/resize, vectors, fn-refs, text).  *Not* "every Goal-A
  coverage leg shipped."
- **Structure floor — cleared when:** the libraries a game depends on (graphics,
  game_client / game_protocol, server) are extracted, installable, and
  version-stable through the registry.  *Not* "the whole package toolchain
  polished."

When both bars read true, the pause ends and the dogfood loop goes back to
setting the agenda.  Until then, A and B are the work *because* C and D asked for
them.

---

## How the goals relate

```
       useful ───────────► safe ───────────► predictable
   C (capability)      A (soundness)      E (predictable memory)
   B (release)         D (parity)             the programmer's model
        │                                     is the truth
        └── paused until the soundness floor (A) and
            structure floor (B) clear, then resumes as agenda-setter

   F (friction-free surface) ── orthogonal: the user-friction cost
        of delivering any of A–E must stay near zero
```

Dogfood makes loft *worth using*; the sanitizer makes it *safe to keep using*;
Goal E makes it *predictable to reason about* — **safe is not enough if the
programmer cannot hold the memory model in their head.**  A — *no corruption* —
and E — *no surprise* — are different properties: Rust achieves the first and not
the second, and unbundling them is exactly where loft aims to surpass it.  Goal F
sits across all of them: each of A–E must be delivered **without billing the
programmer** in syntax or proof-obligations — friction is the tax Rust pays for
its guarantees and the one loft refuses.

## See also

- [CLAUDE.md](../../CLAUDE.md) § "Development cadence — the dogfood loop" — Goal C.
- [plans/finished/53-sanitizer-ci-lever/](plans/finished/53-sanitizer-ci-lever/README.md)
  — Goal A: the sanitizer lever and the stack-alignment work.
- [plans/future/55-program-level-fuzzing/](plans/future/55-program-level-fuzzing/README.md)
  — Goals A/D: program-level fuzzing feeds both the UB sweep and the
  cross-backend differential.
- [plans/future/56-sanitizer-coverage-expansion/](plans/future/56-sanitizer-coverage-expansion/README.md)
  — Goal A continuing: remaining sanitizer-coverage items.
- [plans/future/57-vector-store-watermark/fix-design-store-lifetime.md](plans/future/57-vector-store-watermark/fix-design-store-lifetime.md)
  — **Goal E**: the predictable-memory fix design + `LOFT_STORE_GUARD` detector
  (free a vector store at its scope, decoupled from the slot).
- [lib_plans/12-library-extraction/](lib_plans/12-library-extraction/README.md) — Goal B's structure floor: the package-ecosystem extraction.
- [PKG_REGISTRY.md](PKG_REGISTRY.md) / [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md) — Goal B's registry on-ramp.
- [ROADMAP.md](ROADMAP.md) / [PLANNING.md](PLANNING.md) — feature backlog feeding Goal C.
