<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 121 — Example tests for API docs

## Status

**Open — design complete, nothing implemented.** Tracked as
[@PLN121](https://github.com/loft-lang/plans/issues/121).

## Goal

An example in a `pub` item's doc comment is executed and asserted, and only an example
that ran ships with that item's API.

## What already exists — read this before designing anything

**loft already verifies doc examples, and has for a long time.** `tests/docs/*.loft` are
runnable test files whose comments are the prose and whose code is the example;
`gendoc` reads them and generates `doc/*.html`. 33 pages, **536 asserts**. The doc
cannot disagree with the code because the doc is *generated from* the executed source.

So this plan is **not** "give loft doctests". The mechanism is proven at scale; what is
missing is its reach. The measured gap:

| corpus | `pub` items | with a doc comment | whose doc mentions code |
|---|---|---|---|
| stdlib `default/*.loft` | 224 | 179 (79%) | **49** (21%) |
| the 34 published packages | 1093 | 730 (66%) | **115** (10%) |

**164 doc comments already contain code-shaped prose that nothing executes.** Those are
what `loft api`, `loft search`, and the registry's per-version `ApiItem { sig, doc }`
put in front of consumers.

### Two placements, and they are NOT one family

The tempting unification — "one mechanism for every verified example" — is wrong, and
naming why keeps it from being attempted:

| | language docs (`tests/docs/`) | API docs (this plan) |
|---|---|---|
| lives in | its own page file | beside the item, in `src/*.loft` |
| shape | prose-led, page-shaped | code-led, item-shaped |
| direction | **doc generated from the test** | **example extracted from the doc** |

The direction is genuinely opposite, because an API doc cannot be relocated into a
separate page — it has to sit next to the function it documents. Forcing one mechanism
over both would be over-unification; the cases would assert their difference the first
time an API doc needed to move.

What they **do** share, and must keep sharing, is the invariant below and the assertion
style. Which answers the first open question empirically rather than by preference:
**`assert`, not a Go-style `// Output:` block.** loft already made that choice 536
times, it composes with everything else in the language, and adding an
output-comparison format would be a second way to say the same thing.

## The invariant

> **An example that ships in documentation is an example that ran.**

The single rule under which an untested case behaves correctly for the same reason the
tested ones do: rendering never has a path to an unverified example, so a doc that
disagrees with the code cannot be published — not because someone checked, but because
the unverified form never reaches a renderer.

## Re-assertion sites — the brittleness count

Four places would independently decide "this example is fine to show": the runner,
`loft api`, the registry `ApiItem.doc`, and gendoc/HTML. **N = 4, and omission is
silent** — an unverified example renders identically to a verified one, which is the
worst possible failure for a feature whose entire value is trustworthiness.

**The cure is to collapse N to 1 by construction, not by discipline:** one extractor
produces examples, and it emits an example *only* as the output of a run that passed.
Renderers consume that output and have no other source. There is then no site at which
"remember to check" can be forgotten, because no unverified example exists to render.

Reject the alternative that will suggest itself — a `verified: bool` on each item, with
renderers filtering. That is N = 4 again with a flag, and a renderer that forgets the
filter is silently wrong.

## Failure paths

| # | what happens | required behaviour |
|---|---|---|
| E1 | example in a doc comment never runs | the gap being closed |
| E2 | example runs but asserts nothing | **must be reported.** It executed and proved nothing — a hole in the *existing* mechanism too, and cheap to close |
| E3 | example needs a file / socket / display | explicit opt-out marker, recorded; **never a silent skip** (the retention-guard lesson) |
| E4 | passes on `--interpret`, fails on `--native` | both backends, always — a one-backend example is a claim about a backend, not about loft |
| E5 | example needs a dependency | runs with the package's own deps resolved, exactly as its tests do |
| E6 | stdlib examples cannot `use` the stdlib | different harness from library examples; stdlib is already loaded |
| E7 | registry ships version X's doc, example verified against Y | examples are extracted from the *published source*, so they travel with the version they were verified against |
| E8 | a doc example is prose that merely *resembles* code | must not be executed. Opt **in** (a fenced block), never heuristic detection over the 164 |

E8 is why extraction is explicit: 164 doc comments "mention code" by a loose regex, and
guessing which are runnable would produce failures in text nobody meant as an example.

## Steps

Each is independently landable, useful alone, and cannot break anything before it —
the shape that worked for the library compatibility contract.

### Step 1 — measure, change nothing

Count doc comments containing a **fenced** example, per package and for the stdlib. The
number above (164) is a loose regex over anything code-shaped; this is the real
denominator. **Falsifier:** if almost nobody writes fenced examples, the feature has no
corpus and the plan should stop here — the honest outcome is a `## Open work` row, not
a plan.

### Step 2 — the assert-less lint, advisory

An example that runs and asserts nothing proves nothing. This closes it for
`tests/docs/` too, so it delivers value **before any extraction exists** and is
independent of every step after it.

### Step 3 — extract, run nothing

`loft api --examples` lists the fenced examples it finds. Read-only, additive, and it
proves the extractor works across the whole real corpus before anything depends on it.
Extraction is line-based over source text, exactly as `parse_pkg_api` already is — no
parser change.

### Step 4 — run them, both backends, report only

Never gating. Measure the noise across stdlib + all 34 packages *before* anything
depends on the result. Every check in this repo that had to be walked back skipped this
step.

### Step 5 — gate, once step 4 measures zero noise

A broken example fails the library's own CI. Gate only after the evidence, and only for
packages that have examples — nothing is enforced against a package with none, the same
shape the compatibility contract's step 5 used.

### Step 6 — ship verified examples with the API

`loft api` and the registry `ApiItem` carry examples, and carry only ones that ran.
This is the step that makes the invariant load-bearing, so it goes **after** the gate,
never before: shipping an unverified example into the registry puts a claim in front of
consumers that nothing checked.

### Step 7 — conditional: examples as a compat corpus

Only if step 4 shows examples are common enough to matter.

**Narrowed from the original claim.** The plan issue said this would become "a third
compat axis". It is not a new axis — it is a **cheaper corpus for the behaviour axis
that already exists**: run a published version's examples against the working tree,
exactly as `loft compat test` runs its test suite. Its value is specific and bounded:
it works for the 5 packages whose published test corpora no longer run at all
(`cbor 0.1.1`, `game_protocol 0.1.1`/`0.1.0`, `hex_terrain 0.1.0`, `imaging 0.1.0`,
`markdown 0.1.0`), because a doc example has far less surface to rot than a suite.

## What would falsify this design

- **Nobody writes fenced examples** (step 1). Then the invariant is real but the domain
  is empty, and this is a lint, not a plan.
- **The 164 turn out to be mostly prose** rather than intended examples. Then extraction
  is a nuisance and E8 is the whole story.
- **Examples rot as fast as test suites do.** Step 7's premise is that they rot slower,
  because they are smaller and have no fixtures. If a published version's examples fail
  as often as its suite, step 7 buys nothing and should be dropped.

## Composition matrix — Stage A

| axis | cells |
|---|---|
| backend | `--interpret` · `--native` |
| example shape | expression · statements · multi-line · asserts nothing · does not compile |
| location | free fn · method · struct · enum · stdlib vs library |
| outcome | passes · fails · opted out (E3) · not an example (E8) |

Every cell green on both backends before step 5 gates; probes graduate to
`tests/scripts/`.

## Sub-arcs

| Item | Status |
|---|---|
| **1** — measure the fenced-example corpus | Open |
| **2** — assert-less lint (advisory) | Open, independent |
| **3** — extraction + `loft api --examples` | Open |
| **4** — run both backends, report only | Open, needs 3 |
| **5** — gate | Open, blocked on 4's measurement |
| **6** — ship into `loft api` + registry `ApiItem` | Open, needs 5 |
| **7** — compat corpus (conditional) | Open, blocked on 4 |

## Open design questions

Two of the original five are answered above (assertion style, by precedent; and the
compat axis, narrowed to a corpus). Remaining:

1. **Where does an opt-out marker live** for E3, and what does the report say about it?
   A skipped example must be as visible as a failing one.
2. **Does the registry store example text, or only that it passed?** Text makes
   `loft search` useful offline and costs index size; status alone is cheap and much
   less useful.

## See also

- [`../library-compat-contract/README.md`](../library-compat-contract/README.md) — step
  7's corpus, and the 5 stale suites that motivate it
- [`../../TESTING.md`](../../TESTING.md) · [`../../DOC.md`](../../DOC.md) ·
  [`../../DOC_QUALITY.md`](../../DOC_QUALITY.md)
- `src/documentation.rs::parse_pkg_api` — the line-based extractor this reuses
- [@PLN121](https://github.com/loft-lang/plans/issues/121)
