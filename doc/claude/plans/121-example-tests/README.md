<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 121 — Example tests

## Status

**Open — shape settled, five design questions unanswered.** Nothing implemented.
`tests/docs/*.loft` are generated from `loft-lang/features` issues, not from `///`
comments, so a doc example in library or stdlib source is checked by nothing today and
can rot silently. Tracked as [@PLN121](https://github.com/loft-lang/plans/issues/121).

## Goal

A `///` doc example is compiled, executed, and its output verified — and the verified
example travels with the package's API into the registry.

## Effort + design

- **Effort:** M
- **Design:** ~ (partial — the five open questions below gate implementation)
- **Last touched:** 2026-07-28

## Why this earns a plan

Go's `func ExampleFoo()` is its one genuinely distinctive testing idea: compiled, run,
output-verified, *and* rendered in the docs. Rust doctests are the same shape. loft has
neither.

Three things make it fit loft now rather than generically:

**It closes the hole the compatibility contract still has.** `arguments::parse` is the
case that justified the contract's step 3 — signature unchanged, result inverted, so
`API: drop-in` on the shape axis and a break in reality. The only thing that catches
that today is running the *old release's own test suite*, and the 2026-07-28 sweep found
5 of 35 packages whose corpora are too stale to run at all (`cbor 0.1.1`,
`game_protocol 0.1.1`/`0.1.0`, `hex_terrain 0.1.0`, `imaging 0.1.0`, `markdown 0.1.0`).
An example is a behaviour assertion that lives in the source and cannot go stale without
going red.

**The registry already carries the delivery mechanism.** `registry_index::ApiItem
{ sig, doc }` is derived per-version at publish time and is what `loft search` shows. If
`doc` carried verified examples, a consumer reading version 0.3.4's API would see
examples *known to have run against 0.3.4* — and `loft compat` would gain a behaviour
axis that still works for packages whose old suites will not run.

**The parsing already exists.** `Definition.position.{file,line}` points into real
`.loft` source; that is how `///` docs are recovered. Extracting fenced blocks and
running them is additive.

## Composition matrix — Stage A

The feature adds a new *source surface* (doc comments), not a new value/type/operation,
so the axes that matter are execution-side:

| axis | cells |
|---|---|
| backend | `--interpret` · `--native` |
| example shape | expression · statements · multi-line output · no output · compile-fail |
| location | free fn · method · struct · enum · stdlib vs library |
| outcome | passes · output mismatch · does not compile · unrunnable |

Every cell green on **both** backends before it ships; the probes graduate to
`tests/scripts/`.

## Sub-arcs

| Item | Status |
|---|---|
| **A** — extract fenced examples from `///` comments | Open |
| **B** — synthesise + run one test per example, both backends | Open, needs A |
| **C** — verify declared output | Open, blocked on question 1 |
| **D** — surface in `loft api` + the registry per-version `api` field | Open, needs C |
| **E** — decide whether this becomes a third `compat` axis | Open, blocked on question 5 |

## Phase ordering

1. **A + B first, with no output verification.** An example that merely *compiles and
   runs* already catches the commonest rot, and it settles the extraction and
   test-synthesis surface without committing to a comparison format.
2. **C** once question 1 is answered — this is the arc that makes examples load-bearing.
3. **D** after C, since shipping an unverified example into the registry would put a
   claim in front of consumers that nothing checked.
4. **E** last, and only on evidence: it changes what a declared floor means.

## Open design questions

1. **How is expected output declared?** Go's trailing `// Output:` block, an `assert`
   inside the example, or both. Output-comparison reads better in docs; assertions
   compose better and work today. This decides whether examples are a new mechanism or a
   thin wrapper over the existing runner.
2. **Does an example gate library CI, or only `loft test`?** Gating makes it
   load-bearing — and means a new loft warning could redden a shipped library, which the
   `revalidate-libs` discipline says must never happen.
3. **Does the registry store the example text, its verified status, or both?** Status
   alone is cheap; the text makes `loft search` useful offline.
4. **What happens to an example that cannot run** (needs a file, a socket, a display)?
   A silent skip is not acceptable — that is the retention-guard lesson restated. An
   explicit opt-out marker, or unrunnable-by-construction detection?
5. **Does this become a third `compat` axis?** It is the cheapest behaviour verification
   available for packages whose old suites no longer run — but it changes what a floor
   asserts, so it needs its own evidence rather than being assumed.

## Cross-arc dependencies

- **Library compatibility contract** (`../library-compat-contract/README.md`) — arc E
  would extend its axes; arcs A–D are independent of it.

## See also

- [`../library-compat-contract/README.md`](../library-compat-contract/README.md) — the
  behaviour axis this could add, and the 5 stale corpora that motivate it
- [`../../TESTING.md`](../../TESTING.md) — the framework this extends
- [`../../DOC.md`](../../DOC.md) · [`../../DOC_QUALITY.md`](../../DOC_QUALITY.md)
- [@PLN121](https://github.com/loft-lang/plans/issues/121) — the tracking issue
