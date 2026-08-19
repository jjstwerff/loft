<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library documentation review — monthly by-hand protocol

> Origin: [@PLN141](lib_plans/141-library-worked-examples/README.md) (worked
> examples). Run once per monthly release cycle, alongside the
> [RELEASE.md](RELEASE.md) checklist. This is a **hygiene ratchet, not a gate** —
> it never blocks a release; the automated `check_doc_drift.sh examples` gate
> does that.

## Why a by-hand pass exists

The automated gate (`scripts/check_doc_drift.sh examples`, blocked on by CI)
catches the two failures a machine can see: a worked-example tag that **dangles**
(cited, but no test carries it) or **duplicates** (one tag on two functions). It
cannot see the two failures that actually rot a library's docs over time:

- **Staleness** — a `///` doc comment that still resolves and still reads
  cleanly, but no longer describes what the function *does now* (a parameter
  changed meaning, the return contract shifted, a behaviour note went out of
  date). The prose is internally consistent, so nothing flags it; only a human
  reading it against the current body notices.
- **Example quality** — a cited example that is valid and runs, but is no longer
  the *clearest* demonstration: a better real call site has since appeared in a
  consumer, or the tagged test drifted to exercise an edge case rather than the
  common path.

Both need judgment. This protocol is the monthly pass that supplies it, kept
cheap by a **watermark + changed-since worklist** so each month reviews what
actually moved, not all ~350 public functions.

## Cadence and scope

- **When:** once per monthly cycle (the `YYYY-MM` branch), before tagging the
  release. Libraries are not release-coupled for *publishing* (RELEASE.md § What
  forces a release), but their docs share the monthly beat for *review*.
- **Who:** one reviewer per pass — a human, or an agent steered through the steps
  below. Splitting libraries across passes is fine; the watermark carries state.
- **What:** the whole distribution — the loft stdlib (`default/`), the in-tree
  libraries (`lib/*`), and every package in the registry. `make libraries-review`
  names the population and says which part of it this pass owes; you never pick
  the list by hand.
- **The other half of the pass:** the feature catalogue (`@F`/`@I`) rides the same
  monthly beat through `make features-review`, with `make features-check` as its
  pre-flight. Same two questions, same non-gate status — the halves differ only in
  what they review, so run both and treat the union as one worklist.

## The pass — per library

### 0. Pre-flight (automated — must be green first)

```bash
scripts/check_doc_drift.sh examples     # no dangling / duplicate citations
make features-check                     # feature-catalogue shadow in sync (if touched)
make libcatalogue                       # the catalogue builds without breakage
```

`make libcatalogue` is a hard precondition for step 1, not just hygiene: the library
aid reads the snapshots it writes, and those are a **local build by design** (@PLN112 —
a committed copy silently lagged `origin/main`). Skip it and the "what moved" answer is
last month's, which is worse than no answer.

Green here is **necessary, not sufficient** — it means no cheap failure remains,
so the manual budget goes entirely to staleness and quality.

### 1. Generate the worklist

Two steps, coarse then fine. First, which libraries does this pass owe anything?

```bash
make libraries-review
```

It answers the only two questions a program can: what is **structurally missing** (a
library with no watermark row, so it has never been reviewed; one whose source cites no
worked example and carries no `examples-exempt.tsv` verdict; a watermark row naming a
library that no longer exists), and which reviewed libraries have **moved** since the
commit their watermark records — with the commit count and how many `pub fn` lines
changed, so "three commits, zero signatures" can be read and dismissed in seconds. It
is a report: it never fails, and it never judges whether a doc is *good*.

Then, for each library the aid put on the list, the per-function worklist:

```bash
scripts/doc-review.sh --since <its-watermark-commit> <library-tree>
```

It prints three things: a **coverage** count (a health signal, not a target —
most functions are self-evident and correctly carry no example), the **citation
inventory** (every `// Example:` site), and the high-signal part — every public
function whose **signature changed** since the last review. A changed signature
is the number-one source of a stale doc.

### 2. Re-read the changed docs (staleness)

For each function on the changed-since list, read its `///` doc against its
current body: do the description, each parameter's meaning, the return contract,
and any inline example still match what the code does **now**? Doc edits are XS —
**fix drift on the spot**.

### 3. Fill the highest-value coverage gap (examples)

From the *uncited* public functions, pick the ones a reader "knows exists but
cannot use from the signature alone" — the ratchet's rule (@PLN141 § Scope
discipline). Author a worked example for **one or two per pass** (a tagged test,
or a citation to a real consumer call site). The ratchet only goes up; there is
no sweep, and a function whose use is self-evident is left alone.

### 4. Spot-check example quality (freshness)

For a **rotating handful** of already-cited functions (the inventory from step
1), open the cited test: is it still the *clearest* demonstration of the common
path? Has a new consumer usage become a better example? Re-point or improve if
so — a worked example is only worth its tag while it teaches.

### 5. Record and route findings

Fix XS/S (doc edits, a clearer example) in the same pass. File M+ — a function
whose doc drift reveals an actual behaviour bug, or a gap that needs design — as
a GitHub issue per the [bug-filing policy](../../CLAUDE.md) (`Fixes #N`), never
inline. Don't scope-creep the review into unrelated fixes.

### 6. Bump the watermark

Update the library's row in the table below — `reviewed through` to this cycle, `at
commit` to the ref this pass ended on (`git rev-parse --short HEAD` in that library's
repo). A library reviewed for the first time gets a **new row**; that is what moves it
out of the aid's "never reviewed" list.

Leaving `at commit` empty is not free: next month the aid can only say *reviewed, but
no commit recorded — nothing to diff against*, and that library falls back to a full
re-read. The watermark is the entire mechanism that keeps a quiet month cheap.

## Watermark table

"Reviewed through" = the last monthly pass that read this library's docs; "at
commit" = the ref that pass ended on — the baseline `make libraries-review` diffs
against to say what has moved since.

**A row means a review happened.** The libraries that have *not* been reviewed are
not listed here — `make libraries-review` derives that backlog from the actual
population (the in-tree trees plus every package in the registry snapshot), so a
placeholder row would be a second list of the same fact, drifting the moment a
library is published or renamed. Six such rows had already gone stale by 2026-08:
`lib/html` and `lib/markdown` had moved out to the `loft-libs-docs` repo, two more
were spelled differently from the tree they named, and two were prose standing in
for a list — a four-library cell, and a catch-all "registered libs (…)" row that
named six of thirty-four.

The **key column is machine-read** — one library per row, spelled exactly as the aid
keys it: the path for an in-tree tree (`default`, `lib/git`, `lib/lexer.loft`), the
package name for a published one (`graphics`). A key that matches nothing is reported
as a STALE ROW rather than ignored.

| library | reviewed through | at commit | notes |
|---|---|---|---|
| `default` | 2026-08 | `7786d28c` | @STD-001..012 authored across text / collections / JSON / files-IO; docs read while tagging |
| `lib/git` | 2026-08 | `7786d28c` | @GIT-001..005 tagged to live uses in `scan.loft` + `refresh.loft`; 13 pub fns read while tagging |
| `lib/lexer.loft` | 2026-08 | `7786d28c` | @LEX-001 (matches/test/identifier), @LEX-002 (anchor/revert backtracking) — both tagged to live uses in `parser.loft` (`function`, `object`), exercised by the `16-parser` doc test; format-protocol/comment fns still owe examples (need a non-rendered demo) |
| `lib/audience_crystal` | 2026-08 | `7786d28c` | @ACR-001..003 tagged to the `01-editor-helpers` test (picking inverse, incr editor loop, erase) |
| `lib/engine_host` | 2026-08 | `7786d28c` | @EHK-001..004 tagged to CI-spawned audience-demo kernels (run loop, broadcast, sync lanes, run_client drain); 37 pub fns read while tagging |

`7786d28c` is the commit that authored every in-tree worked-example tag (squash-merged
PR #971, 2026-08-18) — the 2026-08 pass *was* that tagging, so it is the pass's real
end ref rather than the `(bootstrap)` placeholder these rows carried first.

## What this is NOT

- **Not a gate.** It never blocks a release — the `examples` gate blocks on
  dangling/duplicate; this is a report, like `make speed`.
- **Not a full re-sweep.** The watermark + changed-since worklist bound each
  pass to what moved. A month with no library changes is a five-minute pass.
- **Not a coverage mandate.** A low citation count is healthy when the uncited
  functions are self-evident. The target is "every *non-obvious* function has a
  *current, clear* example", never "every function has one".

## See also

- [@PLN141](lib_plans/141-library-worked-examples/README.md) — the worked-example
  mechanism (tag family, `check_doc_drift.sh examples`, `idx` ingestion).
- [DOC_QUALITY.md](DOC_QUALITY.md) — how the docs themselves should read.
- [RELEASE.md](RELEASE.md) — the monthly cadence this pass rides.
- `scripts/doc-review.sh` — the per-function worklist generator invoked in step 1.
- `make libraries-review` — the per-library worklist that picks what step 1 drills into
  (`scripts/check_doc_drift.sh libraries-progress`); `make features-review` is its
  feature-catalogue twin.
