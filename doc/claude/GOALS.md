<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# GOALS.md — what loft is for, and the goals that serve it

loft is not the goal. loft is the **foundation**: the lowest layer of plumbing.
The real goal is the **libraries and tools built on top of it** — lavition (the
engine), the hex-world library, the editor, the games. loft exists so those can
be built, picked up, and enjoyed.

Its technical north star is **stability**. This document splits the work into
**six goals (A–F)**. Each goal carries a **Check**: a command to run, or a fact to
observe. The point is to *measure* progress, not claim it. A Check stays the same
over time; its result does not — run it to see where you stand today.

## Purpose — what loft is for

One drive runs through every layer: **do the hard plumbing yourself, deeply, so
someone else can just pick it up and have fun.**

- **loft** handles memory, types, and the store, so you write the logic with no
  ceremony.
- the **hex-world library** handles terrain, walls, collision, and rendering, so
  you make a world game without building world infrastructure.
- the **editor** handles authoring, so you shape worlds.
- the **server** handles networking, so a shared game just works.

These are not separate projects. They are one drive, repeated. The destination is
lavition's identity: not "the hex-world engine" but *the engine where the hard
parts are already done.*

**The acceptance test: a thing is done when picking it up is *fun*.** Not
feature-complete — fun. A library can ship every feature and still be a fight to
use. That library is not done.

The fun is the point in itself, not a way to attract a crowd. We build the idea
because it is worth building. Adoption is a **result, not a steering input**: real
value reaches the people who share the idea in its own time. Every decision is
made on two things only — staying true to the idea, and depth.

The six goals below are **foundation goals**: loft must be sound (A), shipped and
clear (B), capable (C), portable (D), predictable (E), and friction-free (F). A
crack in the foundation becomes a crack in everything above it. The same bar holds
for the libraries on top — and they are the real end. They **do not meet the bar
yet**. The shift now underway: loft is finishing its job, and the work is moving up
to bring the libraries to the same standard.

---

## The six goals at a glance

| | Goal | In one line |
|---|---|---|
| **A** | Soundness | no silent corruption — now, and across toolchain bumps |
| **B** | Release & legibility | it ships, and the value is legible on contact |
| **C** | Capability | real consumers keep building and running |
| **D** | Parity | identical on every OS × backend |
| **E** | Predictable memory | the source is the truth — *surpass Rust here* |
| **F** | Friction-free | the language serves the programmer, never the compiler |

The bar covers loft (the foundation, nearly there) *and* the libraries on top
(the real end, which fall short today). Each goal carries a **Check** — something
to run or observe — so progress is measured, not claimed.

---

## North star

**loft is stable when it is correct now AND stays correct as the world changes
underneath it** — new rustc/LLVM toolchains, new platforms, new ways consumers
write code. The failure mode we guard against is **latent undefined behaviour
(UB)**: a bug that passes every test on today's toolchain, then shows up as silent
data corruption after a compiler upgrade or on a stricter platform. "Passes today"
is not "stable."

## The two engines (both required; neither finds the other's bugs)

- **Dogfood — "is loft useful?"** Build real programs in loft (the branch-review
  viewer, the tracker indexer, `lib/markdown`, the games moros/dryopea). Learn
  what the language is missing, fix it, and ship the result. This finds the bugs
  users hit *today*: missing features, awkward ergonomics, feature-interaction
  bugs, design inconsistencies. (See CLAUDE.md § "Development cadence".)
- **Sanitizer — "is loft safe?"** Use tools to detect UB — Miri, the homegrown
  `stack_align_guard`, and fuzzing — covering bad alignment, use-after-free,
  uninitialised reads, aliasing, and leaks, whether or not the output looks
  correct today. This finds the correct-today, corrupt-tomorrow bugs that
  dogfooding cannot see.

These feed four goals: **A** (soundness) is the sanitizer engine; **C**
(capability) and **B** (release) are the dogfood engine; **D** (parity) uses
sanitizer-style differential testing but serves both.

---

## Goal A — Soundness (no silent corruption)

**Definition.** loft never produces wrong or corrupt results from undefined
behaviour, and stays that way across toolchain upgrades. This is a *continuous*
property: it is met by a sanitizer that keeps running and keeps catching new UB,
not by one clean run.

**Check.**
- `ci.yml`'s per-PR `guard` and `asan` jobs are green on `main` (and on the PR
  under review).
- `cargo test --features stack_align_guard` fires zero times across the whole
  corpus (every test binary, not just `issues`).
- The nightly `miri.yml` gate is green.

Healthy when all three hold *and keep holding* as new code lands.

---

## Goal B — Release & legibility

**Definition.** loft actually **ships** on a regular cadence, and its value is
**clear the moment you meet it** — installable, with a clean starting path — so the
people who would value it *can* find and recognise it. A language that never
reaches a stable release, or that only its authors can run, never gives those
people the chance. Adoption itself is a *result, not a goal* (see Purpose); this
goal is met by the starting path *existing*, not by a user count.

**Check.**
- A release tag exists within the project's release cadence (`git tag` → latest),
  each with a CHANGELOG.md entry.
- `loft install <name>` resolves and fetches a published library, end to end,
  against the registry.
- On a clean machine, a new user can install loft, run a first program, and `use`
  a library — from the docs alone, with no extra guide.

**A reading, not a target.** Whether anyone outside the project publishes a library
is *observed, not chased*. It tells you the value is landing; it is not a number to
optimise. Removing it as a steering input keeps the work pure (true to the idea,
plus depth — see Purpose).

---

## Goal C — Capability via dogfood

**Definition.** loft keeps gaining the features and ergonomics that real programs
need, and the main consumer programs keep building and running.

**Check** — the consumer build matrix; each row is a command, scored `N/total`:
- the branch-review viewer builds and runs on HEAD;
- the tracker indexer (`make index`) succeeds;
- the `lib/markdown` suite passes;
- the games moros / dryopea build and run against current loft;
- the last release's CHANGELOG has a consumer-driven harvest section.

Never "finished"; it reads as a fraction, and a falling fraction is the alarm.

---

## Goal D — Cross-platform + cross-backend parity

**Definition.** loft behaves identically on **ubuntu / macOS-ARM / windows** and
across the **interpret / native / wasm** backends. No result depends on the
platform or the backend. macOS-ARM matters most here: it requires aligned memory
reads, so an unaligned read is a real fault there. The three backends are three
separate implementations of the same language.

**Check.**
- The 3-OS CI matrix (`ci.yml`) is green on `main`.
- A differential run executes one shared set of programs on interpret / native /
  wasm and checks for **identical output and diagnostics** — zero differences.
  Per-backend green is *not* enough; the backends must agree on the same input.

---

## Goal E — Predictable memory (the programmer's model *is* the truth)

**Definition.** The runtime's memory behaviour matches the **plain reading of the
source**: a value's heap memory is freed when its **scope** ends, with **no
exceptions the programmer has to learn**. The model is small enough to hold fully
in your head — write a vector inside a block, and it is freed at the end of that
block, full stop.

This is **different from Goal A.** A program can be perfectly *sound* (no
corruption) and still hold far more memory than the source suggests, because the
runtime quietly keeps it alive. Goal A asks "is it safe?"; Goal E asks "**can the
programmer predict it?**"

**Why it's a goal, and the bar it sets.** The appeal of this kind of control is
that the memory model is small, the source is the truth, and when something goes
wrong the fault is the programmer's — knowable and fixable. Rust buys safety with a
static analysis the programmer must reason about, so debugging often becomes "what
did the compiler decide?". loft makes a different bet: safety from a **runtime
discipline** instead of a static proof. That lets the *rule* be the whole model.
The aim, stated plainly:

> **On the single axis of safe AND predictable (the programmer stays in control),
> surpass Rust** — not at performance, concurrency, or ecosystem, but here,
> because Rust cannot separate safety from its hidden machinery and loft can.

What makes or breaks it: the **rule the programmer sees stays exceptionless** — "a
value dies when its scope dies". All the cleverness — last-use analysis, reference
handling, deciding slot vs heap — lives **only in the implementation, out of
sight**. The moment the *rule* grows an "except when captured / iterated /
reassigned…" that the programmer must memorise, loft has rebuilt Rust's hidden
complexity in a new form. When loft's behaviour surprises the plain reading of the
source, that is a **loft bug** — never "harmless", never worked around in docs.

**Check.**
- `LOFT_STORE_GUARD=1` is **silent across the corpus** — no block-scoped vector
  store is freed later than the block it belongs to. (Detector shipped; see
  [plans/future/57-vector-store-watermark/fix-design-store-lifetime.md](plans/future/57-vector-store-watermark/fix-design-store-lifetime.md).)
- That guard is promoted to a `#[cfg(debug_assertions)]` assertion, so the rule
  cannot quietly regain exceptions as new code lands — the guard *forbids* the
  drift, it does not merely report it.

Healthy when the guard is silent corpus-wide *and* the assertion holds *and keeps
holding*. A guard that starts firing is the alarm that the model and the runtime
have drifted apart.

**Progress — reference counting removed** (plan-57,
[`@PLN2`](https://github.com/loft-lang/plans/issues/2), closed 2026-06). The store
reference count is gone (`ref_count` / `inc_rc` / `dec_rc` / `OpIncRc` deleted;
`Store.pinned` for const/global). It is replaced by a single-ownership free at scope
end plus a closure-record cascade. This advances Goal E directly: **no hidden
counter decides when a value dies — the scope does**, which is the plain reading of
the source. The reference count was the clearest case of sound-looking machinery
that hid the real lifetime; removing it makes the lifetime visible.

---

## The method mirrors the goals

Goal E's law — *the stated model must match reality; a divergence is a bug, fixed
by **removing** hidden machinery, not by adding cleverness* — is also how loft is
**built**, not only what it ships. The same law, one level up, governs our own
reasoning:

- an investigation's stated thesis must match its contents — don't slip an
  unrelated bug's fix into it (that makes the record disagree with what it claims
  to be);
- a bug verdict must match the bug's **verified** shape — don't claim "zero blast
  radius" over code you haven't probed. An unchecked confidence does the same
  thing the old reference count did: it lays a clean story over a reality you
  didn't check.

The edge-probe-first habit and the sibling-bug scope hygiene
([plans/README.md § Edge-probe](plans/README.md#edge-probe-before-fixing--the-lightweight-default-for-lofts-complex-variant-bugs))
are this same rule turned on our own claims. A team that tolerates hidden machinery
in its own reasoning cannot credibly ship a language whose whole promise is
no-hidden-machinery.

One related heuristic: when you hit the same wall again and again, the cause is
usually not your current attempt but an **old conservative mechanism** that was
correct before you had today's information and has outlived the gap it covered (the
store reference count is the model case). The move is to find that old assumption
and **narrow it with what you now know**, not to pile cleverness on top.

### Bugs hide things — clear them before trusting the model

A bug is not just a broken spot. It **hides everything downstream of it**, and —
worse — it can make broken things look fine. Examples are routine:

- a `parallel {}` block that silently did nothing on `--native` made test-80 and
  test-81 *pass* — "the asserts held" hid that native ran no arm at all;
- a read-only-store crash hides what a heap mutation would actually do;
- an over-retained store under the old reference count *never crashes*, so a wrong
  free site looks correct.

So clearing bugs is a **precondition** for verifying Goal E, not a separate task:
you cannot confirm "the model is the truth" through a bug that hides what the
runtime really does. This is why the standing detectors (the sanitizer for A,
`LOFT_STORE_GUARD` for E, a differential backend sweep for D) earn their place:
they turn hidden behaviour into a visible signal, so the model becomes checkable.

### Don't tolerate re-derivation patterns — engineer the class away

Some bugs are not a broken spot but a **broken pattern**: shared state that several
routines each work out *independently*, instead of reading it from one place. When two
of those routines disagree, the result is wrong — yet no single routine is at fault, so
the failure is **silent**. It slips past [Goal A](#goal-a--soundness-no-silent-corruption),
and it often shows up only as a **backend disagreement** — interpret says one thing,
`--native` another ([Goal D](#goal-d--cross-platform--cross-backend-parity)). That
hidden, re-derived state is the same shape of machinery
[Goal E](#goal-e--predictable-memory-the-programmers-model-is-the-truth) removes from the
memory model: a counter or convention that quietly decides an outcome the source never
names.

Three examples from loft's own internals:

- **variable slots** — slot numbers were once worked out per pass; two passes could
  land on different numbers and collide.
- **the store reference count** — a hidden counter, not the scope, decided when a value
  died (removed in plan-57; see Goal E above).
- **native pre-evaluation identity** — the collect walk and the emit walk each
  re-derived a hoisted sub-expression's name from a codegen counter, and disagreed when
  the counter drifted (the `PreEvalSet` work; see
  [COMPILER.md § Synthesised-identity stability](COMPILER.md#synthesised-identity-stability--the-counter-coupling-hazard)).

When we find a pattern like this we do **not** tolerate it. Tolerating wears four
respectable disguises, and we refuse all of them: **patch one site** (the siblings stay
broken); **add a workaround or a guard** (a new rule someone must maintain — itself fresh
friction, against [Goal F](#goal-f--friction-free-surface-the-language-serves-the-programmer-not-the-compiler));
**file it for later** (you re-pay to re-derive the scope you understand right now); or
**add an assertion and move on** (both derivations stay alive, so it returns).

We engineer the **class** away, on the same four-step arc the slot and reference-count
work used:

1. **Observe** the brittleness — name the state being re-derived, and the silent failure
   it causes.
2. **Reify** it — give the state one explicit home (the slot table; `PreEvalSet`), so the
   implicit thing becomes a value you can hold and inspect.
3. **Prove coverage** — check that *every* routine can be served from that one home,
   **without changing behaviour yet**. This is a gate, not a courtesy: cutting over before
   coverage is proven breaks the one consumer that cannot yet read from it. A standing
   detector that reports the remaining gaps (`LOFT_STORE_GUARD` for memory; the pre-eval
   drift report for identity) is how this stays measurable.
4. **Cut over** — the routines *read* the one home, and the second derivation is
   **deleted**. The class is gone only when nothing re-derives the state, because then
   there is nothing left to disagree.

The definition of done is step 4: until the second derivation is deleted, the bug can
come back. Making the hidden state **visible** (steps 1–2) is diagnosis; making it the
**only** state (step 4) is the cure — the constructive form of Goal E's law that a
divergence is fixed by *removing* hidden machinery, not by adding cleverness.

#### The largest instance — the two backends, and where the arc must stop at step 3

The biggest re-derivation in the whole project is the **interpreter vs the native
backend**: two implementations that each derive "what this program means" from the same
IR. Its silent-failure symptom is exactly [Goal D](#goal-d--cross-platform--cross-backend-parity)
backend divergence — interp says one thing, `--native` another, no error. The pattern is
*fractal*: #272 was a tiny re-derivation **inside** native (collect vs emit, pre-eval
identity from a counter) that surfaced as a full parity break (`inline=true` vs `false`).
The small disagreement *was* the large one — fixing the sub-derivation restored parity.

The **native-library work** (`@PLN11` "Data as a store" + store-backed IR; the C71
shared-store dispatch in [NATIVE.md § N9](NATIVE.md#n9--native-library-shared-store-dispatch-c71))
is this same arc applied to the substrate, not a one-off fix:

- **Reify** — data and the IR *become a store*, one substrate both backends index into,
  instead of each holding its own representation.
- **Cut over** — a native library and its interpreted caller share **one** store, not two
  copies kept in sync. The N9 open item *"binary schema, no source re-parse"* is the same
  move: re-parsing source to recover a library's types is a second derivation of the
  schema; the binary schema is the reified single truth both sides read.

But here the arc **cannot reach step 4 at the top level**: the three backends are
deliberately three implementations (interp for debug/wasm, native for speed — Goal D), so
"make native read the interpreter's execution" is impossible *by design*. This is exactly
the case the arc's fallback covers — *when you cannot delete the second derivation, make
its violation loud.* That is what the **differential backend sweep** is (Goal D's Check:
"identical output and diagnostics — zero differences"): the standing detector for the one
re-derivation we are forced to keep. So the native-library work and this principle are one
effort from two directions — **every piece of shared state reified into one truth (one
store, one schema, one IR, one node-identity) is one fewer place the backends can silently
disagree**, shrinking the irreducible remainder down to just the execution strategy, small
enough that the differential sweep can actually guard it.

---

## Goal F — Friction-free surface (the language serves the programmer, not the compiler)

**Definition.** No syntax, annotation, or blocking error exists just to feed the
*compiler's* analysis. The programmer writes what expresses intent. The compiler's
internal needs — lifetime tracking, confinement proof, slot assignment — are the
**implementation's** problem and stay invisible. When the compiler cannot prove
what it wants, it **takes the cost itself** (a missed optimisation, a postponed
feature); it does **not** hand the programmer a form to fill in. Warnings are the
one allowed channel: they describe the consequences of the programmer's *own*
choices, and they are **free to ignore**.

**Why — the Rust grievance, stated plainly.** Rust bought safety by pushing its
analysis onto the syntax: `'a` lifetimes, `move`, turbofish, `Pin` — ceremony that
serves the borrow checker, not the author. Once that syntax ships, it cannot be
removed; Rust's long-running ergonomics effort shows how hard that is to undo. loft
refuses the first step: **rather miss a feature than add friction.** This is paid
for by Goal E's bet — safety from a runtime discipline, not a static proof. No
static proof means **no proof obligations to put on the user**. E removes the
machinery from the *model*; F removes it from the *syntax* — the same coin.

**The friction test.** For any syntax or error, ask: *does this serve the
programmer or the compiler?*
- a type on a signature (documents intent), a warning about an unused value (the
  programmer's choice, ignorable) — serves the programmer: **keep**;
- a lifetime annotation, a `move`, a "restructure it so I can prove it" error —
  serves the compiler: **refuse** — infer it, default it, or drop the feature.

**Missing a feature is the preferred side — and is not the same as friction.**
Refusing an operation the language **cannot do safely** (the unsound-capture error
in `parallel{}` → "use `for par`") is *missing a feature*, not pushing work onto
the user. It says "this isn't available yet, here is the supported path" — never
"annotate X so I can allow it." The line is exact: an error that **bounds the
language** is fine; an error that **bounds the user into serving the compiler** is
the friction F forbids.

**Grounding.** F's deeper reason is the [Purpose](#purpose--what-loft-is-for) — *do
the hard plumbing so it is fun to pick up*. That makes friction **fatal, not
cosmetic**: a Goal-F violation means the plumbing isn't finished, so whoever picked
it up gets a *fight* instead of fun, and leaves. The crawler dogfood made this
literal — its "survival guide" of store-lifetime workarounds (C1/C3/C4/C18 →
[loft#248](https://github.com/loft-lang/loft/issues/248)) *is* the plumbing not yet
done.

**Check.** No feature design ever reaches "…and the user must write X so the
compiler can Y." When it does, the *feature* is wrong, not the user — infer X,
default it, or cut it. The store-confinement analysis is the worked model: zero
user-facing surface, and a **silent fallback** to a higher watermark when it cannot
prove confinement — the programmer never learns the analysis exists.

**Relation.** F is **orthogonal** to the useful → safe → predictable axis: it limits
the user-friction cost of delivering any of A–E. It is closest to E (both keep
hidden machinery from leaking out), but E guards the **memory model** while F guards
the **whole syntax**.

---

## The two floors — why dogfood is paused, and when it resumes

The dogfood loop normally sets the agenda (CLAUDE.md § "Development cadence"). It is
**paused right now, on purpose** — because the loop did its job and hit two walls:

- it **kept surfacing instability** → the **soundness floor** (Goal A);
- it **fought the library/package structure** → the **structure floor** (Goal B's
  packaging half).

Building a game on either un-cleared floor means building on a base that can still
shift under you. So Goal C (the game work especially) waits on A and B — *by
choice, not neglect*.

The risk of a deliberate pause is that open-ended floors make it permanent by
drift: soundness can always take one more sanitizer leg, packaging one more polish.
So each floor has an **explicit resume bar, tied to what a game actually needs** —
not "all of A" or "all of B":

- **Soundness floor — cleared when:** the sanitizer gate is green on `main` **and**
  the Miri/ASan set covers the surfaces the games use (eval stack, store
  claim/copy/resize, vectors, fn-refs, text). *Not* "every Goal-A coverage leg
  shipped."
- **Structure floor — cleared when:** the libraries a game depends on (graphics,
  game_client / game_protocol, server) are extracted, installable, and
  version-stable through the registry. *Not* "the whole package toolchain
  polished."

When both bars read true, the pause ends and the dogfood loop sets the agenda
again. Until then, A and B are the work *because* C and D asked for them.

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

Dogfood makes loft *worth using*. The sanitizer makes it *safe to keep using*. Goal
E makes it *predictable to reason about* — safe is not enough if the programmer
cannot hold the memory model in their head. A (*no corruption*) and E (*no
surprise*) are different properties: Rust achieves the first but not the second, and
separating them is where loft aims to surpass it. Goal F sits across all of them:
each of A–E must be delivered **without billing the programmer** in syntax or proof
obligations.

## See also

- [CLAUDE.md](../../CLAUDE.md) § "Development cadence — the dogfood loop" — Goal C.
- [plans/finished/53-sanitizer-ci-lever/](plans/finished/53-sanitizer-ci-lever/README.md)
  — Goal A: the sanitizer lever and the stack-alignment work.
- [plans/future/55-program-level-fuzzing/](plans/future/55-program-level-fuzzing/README.md)
  — Goals A/D: program-level fuzzing feeds both the UB sweep and the cross-backend
  differential.
- [plans/future/56-sanitizer-coverage-expansion/](plans/future/56-sanitizer-coverage-expansion/README.md)
  — Goal A continuing: remaining sanitizer-coverage items.
- [plans/future/57-vector-store-watermark/fix-design-store-lifetime.md](plans/future/57-vector-store-watermark/fix-design-store-lifetime.md)
  — **Goal E**: the predictable-memory fix design + `LOFT_STORE_GUARD` detector
  (free a vector store at its scope, decoupled from the slot).
- [lib_plans/12-library-extraction/](lib_plans/12-library-extraction/README.md) — Goal B's structure floor: the package-ecosystem extraction.
- [PKG_REGISTRY.md](PKG_REGISTRY.md) / [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md) — Goal B's registry on-ramp.
- [API_SURFACE.md](API_SURFACE.md) — Goals F + B at the named-API level: verifying the language/stdlib **and** the libraries for dup/confusable/undocumented/footgun functions ([LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) is the per-library bar).
- [ROADMAP.md](ROADMAP.md) / [PLANNING.md](PLANNING.md) — feature backlog feeding Goal C.
