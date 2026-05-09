<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library plans

Multi-phase library design + implementation initiatives that
span more than one session.  Each subdirectory holds the
README (goal + index) plus one markdown file per phase.

**Distinct from `plans/`.**  `plans/` tracks core-language and
compiler / runtime initiatives (validation matrices, codegen
arcs, language features).  `lib_plans/` tracks **library**
initiatives — `server`, `game_client`, graphics / OpenGL,
regex, package format, asset pipeline, web examples, IDE.
Libraries are ergonomic surfaces built on top of the language;
their planning has different acceptance criteria (API shape,
example coverage, downstream consumer impact) than core-language
work.

## Companion indexes — every parked item is discoverable

- **[`DEFERRED.md`](DEFERRED.md)** — internal index of every parked
  library plan, deferred design decision, and "noted but not now"
  item specific to libraries.  Each row carries an explicit
  `Trigger to unpause:` value.
- Cross-cuts with `plans/`: when a library plan blocks on a
  core-language gap, the row points at the relevant `plans/`
  P-issue or future plan that has to land first.

## Conventions

- Subdirectory names are numbered (`NN-slug`) so they sort in
  the order they were opened.  Numbering is independent of
  `plans/` — `lib_plans/01-foo/` is unrelated to `plans/01-bar/`.
- A new initiative opens with an `NN-slug/README.md` stating the
  goal, phase layout, and ground rules, plus a first phase plan
  file (conventionally `00-<first-phase>.md`).
- Every phase plan file begins with `Status: open | in-progress |
  done` so a fresh session can orient quickly.
- When an initiative is fully closed (all phases committed, no
  open follow-ups), move its entire subdirectory into `finished/`.
- When an initiative is intentionally paused — well-described,
  no driving bug or feature, picked up only when triggered —
  move its entire subdirectory into `deferred/`.  Deferred
  library plans differ from finished plans: they're not done,
  they're parked.  Their READMEs must state the **trigger** that
  would unpause them.
- "We will do this, just not yet" → `future/`.
- "We won't do this absent a concrete trigger" → `deferred/`.
- Same `≤3 active` discipline as `plans/`: hold the in-flight
  count to 2-3 library initiatives.  Promote one from `future/`
  when a current one closes.

## Ground rule — library plans never break downstream consumers

A library plan's job is to evolve a library's API + implementation
without surprising the consumers (loft programs that import the
library, example galleries, downstream embedders).  Every phase,
and every step within a phase, must:

- Preserve every currently-green test across the full suite,
  including library-specific test scripts.
- Preserve every currently-correct downstream example unless
  the phase explicitly migrates it.
- Either ship a new API or a no-op refactor — never a
  break-now-fix-later bargain on a published surface.

When a phase migrates a downstream example (e.g., the moros
editor moves to a new server API), the migration lands in the
SAME commit as the API change — the example becomes part of
the phase's deliverable, not a follow-up TODO.

## Ground rule — file pre-existing bugs surfaced during a library hunt

Same rule as `plans/`: a library phase fixing one feature
routinely surfaces *other* bugs while probing variants — file
those P-issues before the phase closes, not later.  See
[CLAUDE.md § Bug-filing policy](../../../CLAUDE.md#bug-filing-policy--mandatory).

Library-specific notes:
- Bugs in the library code itself stay in PROBLEMS.md (single
  cross-cutting issue tracker).  No separate library issue
  tracker.
- Bugs in the **core-language layer** that block a library
  phase get a P-id and the library phase pauses (or works
  around with an annotation pointing at the P-id).

## Current library initiatives

Maximum 2-3 library plans in flight at a time.  When a current
plan closes, promote the next-highest-priority library plan
from `future/` into this section.

| Dir | Initiative | Status |
|---|---|---|
| _(none yet — populate as library plans get promoted from `future/`)_ |  |  |

## Future library initiatives

Library plans we intend to do, just not right now.  Ready-to-
resume — pre-flight already done, design already drafted.
Promote one into "Current" when a current library plan closes.

| Dir | Initiative | Pre-flight status |
|---|---|---|
| [`future/01-regex/`](future/01-regex/) | Standalone regex library — replaces the `r"..."` raw-literal plan and "regex arm in match" plan that were sketched earlier (both withdrawn).  Library lives in stdlib via the lazy-loading mechanism (LAZY_STDLIB.md), gives full regex semantics without bloating the language surface.  First lazy-loaded stdlib consumer.  Cooperates with MATCH_PEG.md (PEG-style sequence patterns) — REGEX handles all text matching; MATCH_PEG handles structural / numeric / Unicode-class patterns. | Design draft (was `doc/claude/REGEX.md`).  Not yet implemented; depends on LAZY_STDLIB.md infrastructure landing first.  Ship order: REGEX R1 (linear NFA, basic features) is the first scheduled item once LAZY_STDLIB lands; R2-R4 (named groups, Unicode properties, backtracking fallback) gated on demand. |

## Deferred library initiatives

Library plans that are well-described but intentionally paused —
picked up only when a concrete trigger arrives.  Distinct from
`future/`, which is "we will do this, just not yet."  Deferred
items are "we won't do this unless something specific changes."

| Dir | Initiative | Trigger to unpause |
|---|---|---|
| _(empty)_ |  |  |

## Finished library initiatives

| Dir | Initiative | Closed |
|---|---|---|
| _(empty)_ |  |  |
