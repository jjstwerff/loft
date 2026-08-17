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
- **What:** the loft stdlib (`default/*.loft`) and the in-tree / registered
  libraries (`lib/*`). The feature catalogue (`@F`/`@I`) rides the same pass via
  its own `make features-check` pre-flight.

## The pass — per library

### 0. Pre-flight (automated — must be green first)

```bash
scripts/check_doc_drift.sh examples     # no dangling / duplicate citations
make features-check                     # feature-catalogue shadow in sync (if touched)
make libcatalogue                       # the catalogue builds without breakage
```

Green here is **necessary, not sufficient** — it means no cheap failure remains,
so the manual budget goes entirely to staleness and quality.

### 1. Generate the worklist

```bash
scripts/doc-review.sh --since <last-watermark-commit> <library-tree>
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

Update the library's row in the table below so next month starts from here.

## Watermark table

"Reviewed through" = the last monthly pass that read this library's docs; "at
commit" = the ref to pass as `--since` next month.

| library | reviewed through | at commit | notes |
|---|---|---|---|
| stdlib `default/` | 2026-08 | (bootstrap) | @STD-001..012 authored across text / collections / JSON / files-IO; docs read while tagging |
| `lib/git` | 2026-08 | (bootstrap) | @GIT-001..005 tagged to live uses in `scan.loft` + `refresh.loft`; 13 pub fns read while tagging |
| `lib/lexer` | 2026-08 | (bootstrap) | @LEX-001 (matches/test/identifier via `parser.loft`); format-protocol/comment/backtracking fns owe examples (need a non-rendered demo) |
| `lib/audience_crystal` | 2026-08 | (bootstrap) | @ACR-001..003 tagged to the `01-editor-helpers` test (picking inverse, incr editor loop, erase) |
| `lib/engine_host` | 2026-08 | (bootstrap) | @EHK-001..004 tagged to CI-spawned audience-demo kernels (run loop, broadcast, sync lanes, run_client drain); 37 pub fns read while tagging |
| `lib/html` | — | — | not yet reviewed |
| `lib/markdown` | — | — | not yet reviewed |
| `lib/input` · `lib/logger` · `lib/lexer` · `lib/parser` | — | — | not yet reviewed |
| registered libs (`arguments`, `hex_grid`, `gridmesh`, `crypto`, `server`, `web`, …) | — | — | rolled in as @PLN141 Phase D reaches each |

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
- `scripts/doc-review.sh` — the worklist generator invoked in step 1.
