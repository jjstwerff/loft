<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# COMPATIBILITY.md — what loft promises not to break, and what "breaking" means

> **Status: arc A of [@PLN102](https://github.com/loft-lang/plans/issues/102) (the
> stability contract), first draft 2026-07-10.** This is the *policy* — the definition
> of a breaking change, per surface. Its two enforcement mechanisms are the `contract`
> version ([plans/102-stability-contract/versioning-decision.md](plans/102-stability-contract/versioning-decision.md))
> and the deprecation channel (arc C, in design). The 1.0 line — *which* surface is
> frozen — is arc E.

## The promise (the standard this serves)

[GOALS.md](GOALS.md) names the aspiration loft is trying to win back from the AS/400:

> backward compatibility was a contract the maker kept: **the platform never broke its
> users; the cost of change was paid by the maker, not the customer.**

So a change is **breaking** when it moves the cost of change onto the *customer* — code
or data that was valid under the previous contract now behaves differently, or stops
working, through no action of its author. The whole of this document is one question
made decidable: *does this change move that cost, and if so how is it paid back?*

## The one distinction everything turns on

Not every change that alters behaviour is equally dangerous. The load-bearing split —
established by the versioning decision, and the reason the `contract` axis is a single
integer — is **how the break announces itself**:

| Class | Definition | Danger | Enforcement |
|---|---|---|---|
| **Additive** | every program and store valid before is valid *and behaves identically* after | none | free — ship it |
| **Loud break** | previously-valid code now **fails to compile or errors at runtime** | low — self-announcing; the user sees it and cannot ship a wrong result | deprecation window + the compiler's own error |
| **Silent break** | previously-valid code still compiles and runs, but produces a **different result** — or old persisted data reads wrong | **high** — no error; a plausible wrong answer | **bump `contract`** + deprecation warning + migration path |

The silent break is the class this whole plan exists to kill — its archetype is
[DESIGN_DECISIONS.md § C86](DESIGN_DECISIONS.md) (plain-bind copy), which made
`hex_terrain`'s heights land in throwaway copies: it did not crash, it computed
`0 land cells`. A loud break is *safe by loudness* — a removed function is an
`Unknown function` error, not a wrong number — which is exactly why the `contract`
integer tracks **silent** breaks only. (Probed: a program using a symbol loft lacks
fails loudly at compile, so the loud class needs no version guard — see the versioning
decision.)

**Why loud breaks do not bump `contract`.** Bumping it on a loud break would warn a
library that does not even use the removed thing (a spurious drift warning) while
adding nothing for one that does (it already fails to compile). Silent breaks accept
that over-warning because there is no compile error to catch them; loud breaks do not
need it. Loud breaks are still governed here — by the deprecation window below — just
not by the version integer.

## Before 1.0 there is no promise

loft's `contract` is **0 until the 1.0 freeze** ([RELEASE.md](RELEASE.md); the language
surface is still settling). At contract 0 this policy is *aspirational*: the language is
explicitly unstable and any surface may move. The promise below **takes effect at
contract 1 == loft 1.0**, which is the same milestone arc E defines. Everything after
this line describes the post-1.0 contract.

## The surfaces, and what "breaking" is on each

The contract is not one thing; it is five surfaces, and a change is classified per
surface. Additive is always free; the table gives the loud and silent forms.

### 1. Language syntax — *how* code is written
- **Loud:** removing or changing a construct so existing source no longer parses;
  promoting an identifier to a reserved keyword (old code naming it now errors).
- **Silent:** a change that still *parses* existing source but *re-associates* it —
  e.g. an operator precedence or associativity change so `a - b - c` groups
  differently. Rare and especially dangerous; treat any precedence/associativity change
  as silent-breaking.

### 2. Language semantics — what code *means*
- **Silent (the dominant risk):** identical source, identical parse, **different
  runtime result**. Binding copy-vs-alias (C86), evaluation order, integer-overflow
  behaviour, null/`??` semantics, coercion and narrowing rules, ownership/free timing
  that a program can observe. Every one of these bumps `contract`.
- **Loud:** tightening a static rule so previously-accepted code is now *rejected* (a
  new type error). Breaking, but caught at compile.

### 3. Stdlib API — the functions and types `default/*.loft` ships
- **Loud:** removing or renaming a public function/method/type; changing a signature
  (arity or parameter/return types) so existing calls no longer type-check.
- **Silent:** changing a function's **behaviour or return value for the same inputs** —
  a different rounding mode, a changed empty-token rule in `split`, a different sort
  stability. Same call, different answer → `contract`.

### 4. Store / heap layout — how live and persisted data is laid out
- The authority is @PLN97's **layout-identity hash** ([plans/97-layout-contract/](plans/97-layout-contract/)).
- **Silent:** a layout change that makes a store *persisted by an older loft* read back
  wrong (fields at shifted offsets, a changed null sentinel). → `contract`, and the
  layout-hash CI gate below makes an un-bumped layout change a hard failure.
- **Additive:** a layout evolution that keeps old stores readable (a versioned/tagged
  format that migrates on load).

### 5. On-disk + wire format — serialized data, the IR JSON codec, any protocol
- **Silent:** reading data or messages written by an older loft yields wrong values.
  → `contract`.
- **Loud:** a format-version field that is *detected and rejected* on mismatch — safe;
  the mismatch is announced.
- **Additive:** new optional fields with defaults; a new message type older readers
  ignore.

### 6. Package format — `loft.toml`, the package layout, the registry schema
- **Additive:** a new optional manifest field (e.g. `[package] contract`, added by this
  very plan — additive by construction).
- **Loud:** removing/renaming a required field, or changing the package directory
  layout so existing packages no longer resolve.
- **Silent:** changing the *meaning* of an existing field (what `loft = ">=X"` binds).
  → `contract`.

## Bug fixes are not automatically breaking — but reliance can make them so

Correcting behaviour that was always *wrong against the documented contract* is **not**
a breaking change: the buggy behaviour was never promised. The default classification of
a defect fix is **non-breaking**, even though output changes.

The exception is **reliance**: when the buggy behaviour is documented, long-standing, or
demonstrably depended upon, fixing it silently changes real programs' results — so it is
a **silent break** and must be paid for as one (bump `contract`, deprecate). The test is
not "did output change" (a fix always changes output) but "was the old behaviour part of
the contract a reasonable user relied on." This is a judgement; the CHANGELOG records
which way it was called and why.

## What the maker owes, per class (the cost paid by the maker, not the customer)

- **Additive** — nothing beyond a CHANGELOG line.
- **Loud break** — a **deprecation window**: the construct/function/field is marked
  deprecated and warns (arc C's channel) for **at least one full release cycle** before
  removal, with a migration note in the CHANGELOG; removal then fails loudly for
  stragglers. *(Window length — one cycle vs longer — is a cadence/brand call for the
  owner; one cycle is the floor.)*
- **Silent break** — the strongest obligation, because the customer cannot see it:
  1. **Bump `contract`** (paired with a CHANGELOG entry that names the break).
  2. **Deprecation warning** (arc C): stale libraries — tested against an older contract
     — warn on drift so the author republishes. Per [GOALS.md](GOALS.md) Goal F,
     warnings are the only channel that may bill the programmer.
  3. **A migration path**: the mechanical change that moves a program/library across the
     break (documented, and automated where feasible).

## Making misclassification loud (so the promise is not just discipline)

A policy that relies on a maintainer *remembering* to classify a change is only as good
as their memory — the versioning decision named this as failure-path 1. Each surface
therefore gets a detector that turns an un-bumped silent break into a **CI failure**,
not a shipped defect:

| Surface | Detector | An un-bumped silent break shows up as |
|---|---|---|
| Store / heap layout | @PLN97 layout-identity hash | hash changed with no `contract` bump → gate fails |
| On-disk + wire format | format-version + round-trip golden tests | a round-trip that no longer reproduces bytes/values |
| Language + stdlib semantics | a **golden-behaviour corpus** (pinned program → output) | a golden output that changed between releases → must be classified |
| interp ↔ native divergence | the @PLN89 differential oracle | an accept/reject or value divergence across backends |

The layout-hash and oracle gates exist; the golden-behaviour corpus + the "hash changed
⇒ contract must bump" wiring are the open CI work shared with arcs C and E. **Residual
(named, not hidden):** a semantic break in a shape no corpus covers can still ship
un-bumped — the dogfood loop is the backstop that converts an unseen shape into a corpus
cell, exactly as [GOALS.md](GOALS.md) describes.

## How this fits the rest of @PLN102

- **The `contract` version** (versioning decision, implemented) is the *mechanism* this
  policy drives: this document says *what* bumps it; the loader enforces the bound.
- **Arc C — the deprecation channel** is *how* the warnings above actually fire (the
  wording, and whose compile they reach — the open Q3). This policy says *what* must be
  deprecated; arc C says *how*.
- **Arc E — the 1.0 line** says *which* surface is frozen (what is in vs still moving at
  1.0). This policy says what "breaking" means *for* whatever E freezes. They meet at
  contract 1.

## Open decisions (owner's call)

1. **Deprecation-window length** — one release cycle (the floor stated above) vs a
   longer guarantee. A cadence/brand decision.
2. **Scope of the 1.0 freeze** — which surfaces/features are *in* the promise at 1.0 vs
   marked experimental (arc E). This policy applies to whatever is in; E draws the line.
3. **Reliance threshold for bug fixes** — how much observed reliance flips a fix from
   non-breaking to silent-break. Left to case-by-case judgement, recorded in the
   CHANGELOG, until a sharper rule is warranted.

## See also

- [plans/102-stability-contract/README.md](plans/102-stability-contract/README.md) — the
  plan; this is arc A.
- [plans/102-stability-contract/versioning-decision.md](plans/102-stability-contract/versioning-decision.md)
  — the `contract` axis this policy drives.
- [GOALS.md](GOALS.md) — the AS/400 standard (the promise) and Goal F (warnings are the
  only channel that may bill the programmer).
- [DESIGN_DECISIONS.md § C86](DESIGN_DECISIONS.md) — the archetypal silent break.
- [RELEASE.md](RELEASE.md) — release cadence + calendar versioning (the release tag, kept
  distinct from `contract`).
- [plans/97-layout-contract/](plans/97-layout-contract/) — the layout-identity hash (the
  store-surface detector).
