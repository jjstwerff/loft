<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# COMPATIBILITY.md — the promise: at contract 1, loft does not break your program

> **Status: arc A of [@PLN102](https://github.com/loft-lang/plans/issues/102) (the
> stability contract), 2026-07-10.** This is the *policy*. Its mechanism is the
> `contract` version ([plans/102-stability-contract/versioning-decision.md](plans/102-stability-contract/versioning-decision.md));
> arc C is how a deprecation *steers*; arc E is which surface is frozen at 1.0.

## The promise (absolute)

> **When loft introduces `contract` 1 (the 1.0 freeze), any loft program that FUNCTIONS
> at that moment keeps running — identically — on every future loft, whatever we do. A
> deviation is not a "breaking change" to be managed; it is a BUG, and it is fixed before
> any other work.**

This is the AS/400 standard [GOALS.md](GOALS.md) names — *"the platform never broke its
users; the cost of change was paid by the maker, not the customer"* — taken at full
strength. It is the floor loft exists to be: a substrate that *sometimes* breaks a working
program betrays the whole pitch. So this is not a versioning scheme with escape hatches; it
is a commitment, and the rest of this document is how the commitment is kept.

**It is stronger than the usual "semver + deprecation window."** There is no
warn-for-a-cycle-then-remove for anything a program depends on: you do not remove a stdlib
function a working program calls, ever. Evolution is **additive**. When a change would make
a functioning program deviate, that change is wrong — reverted or reworked — not shipped
with a migration note.

## What is covered — language, errors, and libs (and the data underneath)

The promise covers **everything a functioning program can observe** — enumerated explicitly
so the scope is not quietly narrowed:

1. **The language** — both *syntax* (how code is written) and *semantics* (what it means and
   how it runs: evaluation order, coercion/narrowing, null/`??`, ownership/free timing a
   program can observe).
2. **Errors** — the diagnostic/error surface a program can observe: *which* error fires and
   *when*, its identity/type, and the content a program can read or catch. A program that
   handles or branches on an error keeps observing it identically. *(Purely human-facing
   error **prose** that no program observes may still be improved — cosmetic polish, like
   any UI text; the moment a program can observe it, it is frozen.)*
3. **The libs** — the standard library (`default/*.loft`) **and** published libraries: a
   program calling a stdlib function or a `use`d library keeps working.

And the surfaces a running program *depends* on, so "keeps running" includes "its data and
dependencies keep working":

4. **Store / heap layout** — a store persisted by an older loft keeps reading correctly
   (@PLN97's layout-identity hash is the authority).
5. **On-disk + wire format** — serialized data, the IR codec, any protocol: old data keeps
   reading to the same values.
6. **Package format** — `loft.toml`, the package layout, the registry: existing packages
   keep resolving.

## The promise is a ratchet — everything we add is forever

The promise does not stop at the 1.0 baseline. **Every addition from contract 1 onward joins
the frozen contract and gets the same treatment**: a feature shipped at contract 2 is, from
that moment, something a contract-2 program may rely on forever — loft will not take it away
or change what it does. The reliable surface only *grows*; it never shrinks or shifts.

This is the promise stated as a **benefit**, not only a constraint: a program or library
author can build on *any* capability loft has ever shipped and know it will still be there,
behaving identically, for the life of their program. That is the AS/400 gift — you never
have to chase the platform.

It also sets the standing discipline for evolution: **because every addition is permanent, we
add carefully.** The pre-freeze audit below is not a one-time gate before 1.0 — its rigor
applies to *each* new capability epoch, because each is a forever-commitment the moment it
ships. loft accretes slowly and deliberately; a feature added in haste is a wart carried for
life. Rarely and well beats often and regretted.

## The classification — additive, or a regression

Under the promise there are only two kinds of change to a covered surface:

| | What it is | Status |
|---|---|---|
| **Additive** | every program, error, and store valid before is valid *and behaves identically* after; something new is available | **allowed** — ship it |
| **Regression** | a functioning program deviates: a different result, a store that reads wrong, an error that no longer fires (or now fires), a removed/renamed symbol | **a bug** — fixed before other work |

The old draft's middle category — a "managed breaking change" with a deprecation window —
is **gone for covered surfaces.** A *loud* regression (a removed function → `Unknown
function`) is still a regression: the program that called it stops working. Loudness makes
it *safe to detect*, not *acceptable to ship*. The only thing loudness buys is that our
CI and the developer catch it immediately — which is why the detectors below matter.

## The error surface is one-directional — you can drop an error, never add one

Errors have an asymmetry the other surfaces do not, and it flips the pre-freeze disposition:

- **Dropping an error is safe** (loosening). A program that used to be *rejected* now compiles
  or runs — no program that *functioned* is broken, because the rejected one never functioned.
  So loft may always become *more* permissive.
- **Introducing an error is a break** (tightening). A program that compiled and ran now fails —
  that is exactly a functioning program breaking. So after the freeze loft may **never** become
  *less* permissive on a covered surface.

Therefore the frozen error set is the **maximum** loft will ever have; it can only shrink. That
**inverts the disposition for errors**: everywhere else the pre-freeze mandate is "improve, then
you can't"; for errors it is **"be strict now, because you can always relax later but never
tighten."** The audit question for the error surface is precisely *"do we need **more** errors?"*
— every place loft is *too permissive* (silently accepts something dubious, or produces a
plausible-wrong value where it should reject or fault) is a **last-chance-to-add**: add the error
while contract 0 allows, convert the programs it catches, and the freeze then locks in a strict
floor we can only ever loosen. (Caveat: a *runtime* fault a program can **catch** is observable
both ways — dropping one can change a program that handled it — so the clean "drop is always
safe" rule is sharpest for **compile-time** errors; runtime faults still follow the general
"functioning program unchanged" test.)

This is why the silent-wrong findings from the lib and formal audits (`text as integer` → null,
integer overflow → null, a classifier true on `""`) are really **missing-error** findings: the
fix is usually *to add a diagnostic or a fault*, and adding it is the one-way door.

## Deprecation is soft steering, never warn-then-remove

A deprecation under this promise **steers** toward a better idiom; it never announces a
coming break. The old idiom keeps working **forever**. So a deprecation warning says *"there
is now a nicer way"* — free to ignore ([GOALS.md](GOALS.md) Goal F: warnings are the only
channel that may bill the programmer, and even this one bills nothing you must act on). It
never means *"this will be removed."* That is arc C's channel, and arc C inherits this
constraint: it warns, it does not threaten.

## Folding — how we engineer around never-remove

"Never break" **plus no usage telemetry** makes the callable surface a **one-way ratchet**: you
can add and steer, but you can never prove zero holdouts, so you can never remove. Deprecation-
toward-removal is therefore *structurally impossible* here — a "deprecation" never reaches a
removal, so it is really **recommended-idiom signposting**, not a path to a break. (The word
"deprecation" survives only for familiarity; it never means here what it means elsewhere.) You can
entice users onto a nicer method, but you will never *know* the last holdout migrated — so the old
method stays forever. The way out is not removal; it is **folding** — we *engineer around* the
constraint. The cost splits across two axes, and only one of them has to grow:

- **The interface** (the callable name / idiom) grows forever — genuinely. The lever is never
  removal; it is **discipline at the door**: because every addition is permanent, add slowly and
  rarely. The ratchet pays the cost *upfront* (add carefully) instead of downstream (deprecate
  later) — this is the operative meaning of "we add carefully."
- **The implementation** does **not** have to grow. *"Never drop the old one"* means never drop the
  callable **name**, not carry its independent **code** forever. When a nicer primitive arrives,
  **fold** the old idiom onto it: reimplement the old name as a thin shim over the new primitive and
  delete the old implementation. The permanent *surface* stops meaning permanent *duplicate code*.

So the collectible win of a steer is the **fold** — fewer independent implementations — never
surface shrinkage. Every recommended-idiom steer should ship *with* its fold, or it buys nothing but
more surface to maintain.

**Folding's limit.** It works only when the new primitive is a genuine **superset** — same
observable semantics, nicer form. A semantically-*different* replacement cannot be folded (the old
behavior is not expressible over the new), so there you carry both — which is exactly why a semantic
replacement must be rare and **contract-keyed** (the escape valve below), never casual.

**The one visibility we do have: the registry.** We can statically scan every published library and
know precisely which *public* libs still use an old idiom. That cannot authorize removal (private
programs are invisible, so zero is unprovable), but it maps the public ecosystem's migration state —
enough to prioritize *which* folds are worth doing and to target the author-facing steer.

## The escape valve for the genuinely unavoidable — key it on the contract

Some change is occasionally forced (a security fix; an API whose old behavior is actively
harmful). The promise still holds, by the AS/400 method: **do not break old programs — keep
their old behavior, keyed on the `contract` they declared.** A program built for contract 1
runs with contract-1 behavior *even on a later loft that changed it* — loft carries both,
edition-style. The new behavior is what a program gets when it declares the newer contract.
This is deliberate, rare, and documented; it is the *only* way an observable behavior ever
changes, and even then no existing program sees the change. A regression that slips through
*unintentionally* is not this — it is a bug, fixed stop-the-world.

## The `contract` integer under this promise

Because forward-compatibility is now **guaranteed**, the `contract` version is simpler than
a breaking-change tracker:

- It is a **monotone capability floor.** A program/library declares `contract >= N` — "I
  need loft's contract-N capabilities." Since loft never breaks forward, **loft at contract
  E ≥ N always runs it** (accept); **E < N** is a clean reject ("needs a newer loft").
- A **bare `contract = N` is `>= N`** (forward-open) — *not* "tested-at N, warn on newer."
  There is nothing to warn about on a newer loft, because a newer loft does not break it.
  *(This supersedes the original versioning-decision default; the mechanism's bare case was
  flipped `=`→`>=` accordingly.)*
- Where the escape valve above is ever used, the contract additionally **keys which behavior
  a program gets** — so the integer a program declares is both its capability floor and its
  behavior epoch.
- **Contract 0 is pre-1.0 — the only era with no promise.** Until the freeze, every surface
  may move; this whole document takes effect at the `0 → 1` flip.

## Before the flip: the pre-freeze audit (the one-way door)

The `0 → 1` flip is **irreversible** — every wart it freezes is frozen forever. So the flip
is **gated on a thorough, surface-by-surface examination** of everything the promise will
cover, done while contract 0 still lets us change it. Anything we would not want to live
with permanently is fixed *before* the flip, not grandfathered into the contract.

**The stakes: a miss is permanent.** If the audit overlooks a wart and it ships in contract
1, we do **not** break it later to fix it — that would break the very programs this promise
protects. We **live with it and engineer around it**: the wart stays, and a better idiom is
added *alongside* it (additive), the way every long-lived platform carries the mistakes it
froze and grows past them without removing them. That is the whole reason the audit must be
exhaustive — the cost of a miss is not a later fix, it is a permanent scar the language
carries forever. Post-freeze, "engineer around, never break" is the standing rule for any
wart we discover too late.

This audit is the critical-path work of the 1.0 line (arc E), not a formality:

- **Syntax** — every construct reviewed for a shape we would regret; the last in-flight
  syntax changes settled and reviewed as part of this.
- **Semantics** — every observable rule (evaluation order, coercion/narrowing, null/`??`,
  ownership/free timing) examined for a wart we would be stuck with.
- **Errors** — which errors fire, their identity, and any content a program can observe —
  reviewed *before* they become frozen (this surface is new to the audit as of the owner's
  scope note).
- **Stdlib** — every public name, signature, default, and behavior — naming consistency,
  missing coverage, wrong defaults — because a bad name or default is permanent after the
  freeze.
- **Store/heap layout, wire format, package format** — every format reviewed for anything we
  would not want to keep readable forever.

**The audit's ledger already exists in part:** [INCONSISTENCIES.md](INCONSISTENCIES.md)
catalogues known asymmetries and tensions by severity — every entry is a candidate
"fix-or-consciously-accept before the freeze," and a **High** (silent-wrong) entry is a
must-fix. The audit resolves that ledger, sweeps each surface for what it misses, and only
then is the flip earned. Consciously-accepted items are recorded as deliberate (a
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) entry), so the freeze is a decision, not an
oversight.

### The road to contract 1 is two phases — language, then libs

The audit is not one rushed pass; reaching the freeze is **dual-phase**, and the order is
forced by the work:

1. **Close the open plans → settle the language.** You cannot audit a surface still in
   motion, so the language side (syntax, semantics, errors) is settled *first* by landing the
   in-flight feature/language plans. This phase is underway. As each plan closes its surface
   stops moving and becomes auditable.
2. **Then a dedicated, unhurried pass on the lib side.** The standard library and the core
   published libraries are an *equally permanent* surface — every public name, signature,
   default, and behavior is frozen forever (§ The promise is a ratchet). They get their **own
   focused phase**, not a rushed tail on the language work: naming consistency, coverage gaps,
   wrong defaults, the method-vs-free-function asymmetries in
   [INCONSISTENCIES.md](INCONSISTENCIES.md) — all decided while contract 0 still permits it.

Only when both phases are done — the language settled and audited, the lib side deliberately
gone over — is the `0 → 1` flip earned. This is *why* the freeze is not imminent despite the
type surface being feature-complete: the language nearing done is phase 1; the libs are phase
2, and they are given their time.

## Per-surface — additive is the path; here is what a regression looks like

| Surface | Additive (allowed) | Regression (a bug) |
|---|---|---|
| **Language syntax** | a new construct that leaves existing parses unchanged | old source no longer parses; a new reserved word colliding with an identifier; a precedence/associativity change that re-groups valid code |
| **Language semantics** | a new capability with no effect on existing programs | same source, different runtime result (the C86 class); changed evaluation order, overflow, null, coercion a program observes |
| **Errors** | a *new* error for previously-*undefined*/UB input; better human-only prose no program observes | an error that used to fire no longer does (or vice-versa); a changed error identity/type a program catches; changed observable content |
| **Stdlib API** | a new function/method/type; a new optional parameter | removing/renaming a public function; a signature change that breaks existing calls; a *different result* for the same inputs |
| **Published libs** | a new library; a new library version | a registry/resolution change that stops an installed library loading; see the note below on library authors' own obligation |
| **Store/heap layout** | a versioned layout that still reads old stores | a layout change that makes an older store read wrong (@PLN97 hash changes) |
| **On-disk + wire** | new optional fields with defaults; a new message type old readers ignore | old data/messages read to different values |
| **Package format** | a new optional `loft.toml` field (e.g. `[package] contract`, added additively) | removing/renaming a required field; a layout change that stops old packages resolving |

**Published libraries have two obligations, not one.** loft promises the *language +
stdlib + runtime* a library is built on will not break it. A **library author** owes the
same promise to *their* consumers — and declares the `contract` they target so loft can
honor it. The registry-validation gate is where a library that broke its own consumers (or
that loft would have broken, before this promise) is caught.

## Bug fixes — the one careful line

A fix whose old behavior was a **crash, a fault, or undefined behavior** is always allowed:
a program that crashes was not *functioning*, so nothing that functions relies on it.

A fix that changes the **observable result of a functioning program** is a **regression**,
even when the old result was "wrong" — because the program functioned, and the promise is to
that program, not to our sense of correctness. The reconciliation is the escape valve: if
the behavior genuinely must change, key the new behavior on a newer contract and keep the old
one for old programs. "We fixed a bug" is never a licence to change what a working program
does.

## Making a regression impossible to ship silently

The promise cannot rest on remembering to check. Each surface has a detector that turns a
regression into a **CI failure** — because a *silent* regression (right-looking wrong answer)
is the one a human review misses:

| Surface | Detector |
|---|---|
| Store / heap layout | @PLN97 layout-identity hash (a changed hash with no keyed migration → fail) |
| On-disk + wire | format-version + round-trip golden tests |
| Language + stdlib + **errors** semantics | a **golden-behavior corpus** — pinned program → (output **and** diagnostics); any drift between releases is a regression to fix or an escape-valve keying to justify |
| interp ↔ native | the @PLN89 differential oracle |
| published libs | `registry-validation` — every published library re-run against loft@main |

The layout hash, the oracle, and registry-validation exist; the golden-behavior corpus (now
explicitly including **diagnostics**, per the errors surface) is the open CI work, shared
with arcs C/E. **Named residual:** a regression in a shape no corpus covers can still slip —
the dogfood loop is the backstop that turns an unseen shape into a corpus cell, and when one
does slip it is a bug fixed before other work, per the promise.

## How this fits @PLN102

- **The `contract` version** (implemented) is the capability floor + behavior key this
  policy drives.
- **Arc C — deprecation** is *soft steering only* here (never warn-then-remove), plus the
  keying mechanism for the escape valve. This policy sets that constraint; arc C builds the
  channel within it.
- **Arc E — the 1.0 line** decides *which* surface is frozen (what is in the promise vs
  marked experimental) at contract 1. This policy says what the promise *means* for whatever
  E includes.

## Open decisions (owner's call)

1. **The 1.0 freeze scope (arc E).** The promise is absolute for what is *in* it; E draws
   the line between frozen and still-experimental at contract 1.
2. **How far "observable errors" reaches.** Where a program can read raw error content vs
   only an error's identity/type — the tighter that surface, the more error *prose* stays
   improvable. Depends on loft's error-handling model; pin it as arc C designs the channel.
3. **The escape valve's ergonomics.** How a contract-keyed behavior split is written and
   tested (edition-style) — designed when first genuinely needed, not before.

## See also

- [plans/102-stability-contract/README.md](plans/102-stability-contract/README.md) — the
  plan; this is arc A.
- [plans/102-stability-contract/versioning-decision.md](plans/102-stability-contract/versioning-decision.md)
  — the `contract` axis (bare = `>=` under this promise).
- [GOALS.md](GOALS.md) — the AS/400 standard and Goal F.
- [DESIGN_DECISIONS.md § C86](DESIGN_DECISIONS.md) — the archetypal silent regression.
- [plans/97-layout-contract/](plans/97-layout-contract/) — the layout-identity hash.
- [RELEASE.md](RELEASE.md) — release cadence + calendar versioning (the release tag, kept
  distinct from `contract`).
