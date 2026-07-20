<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN112 — Library-API provenance: one always-current view

## Status

| Field | Value |
|---|---|
| Plan id | [@PLN112](https://github.com/loft-lang/plans/issues/112) |
| Status | `active` |
| Subject | libs |
| Phase 1 (published enrichment) | **DONE** — version + API sigs + triggers in `LIBRARIES.md` (published-only plain list), regenerated from live, snapshot-`--check` green |
| Phase 2 (`unreleased` origin/main tier) | **DONE** — `scripts/refresh-unreleased.py` builds the sha-cached `unreleased-snapshot.json`; the catalogue tags `🟢 unreleased` additions (regex `search`/`split_on`, arguments +11, gridmesh +6, time +1). Found a **registry data gap**: 13 of 22 libs record an empty `api` field, so their published API can't be diffed — handled honestly (origin/main shown plain + a note), and filed as follow-up (populate the registry `api` on publish). |
| Phases 3–7 | designed, not built |

Motivated by a real failure: an agent read stale local clones + the stale installed copy
and concluded a merged regex rename (`find`→`search`, PR loft-libs-core#23, `d5e4195` on
`origin/main`) "never happened" — because no single view showed **which reality each fact
came from**. `loft api <name>` already extracts a library's public functions, but from the
**stale installed copy**, so it lied. The fix is not to delete copies; it is one view that
labels every fact by its source.

## Goal

> For every library, present its public API **once**, and — wherever sources disagree —
> label each function by the source(s) it lives in. When there is nothing to disagree
> about (a single source, or all sources identical) the layout is a **plain API list**.
> No source pretends to be another; **nothing is auto-deleted**; the reader always knows
> what they can call *here*, what is installable, what is in-flight, and what is merely
> proposed.

**And it is ONE surface, not a tenth tool.** The overriding aim is to **fold the scattered
library-info processes into this single view** — API lookup, the catalogue, api-compat,
superseded steering, branch/version state, registry currency — so there are *fewer*
lib-specific commands and one always-correct place to look. The trigger was "6 commands +
commit hashes across 2 agents to learn a library's state"; the target is one. See
§ What this consolidates.

## What this consolidates — fewer lib-specific commands

The value is integration + one entry point, not a rewrite: each fold **reuses the existing
engine**, it just stops being a separate command an agent must know and run.

| Scattered today | Folded into @PLN112 |
|---|---|
| `loft api <name>` — API lookup, but reads the **stale installed copy** | the core view, correct across all sources |
| `gen-library-catalogue.py` → `LIBRARIES.md` — the catalogue | the committed `published` + `unreleased` view |
| `make api-compat` / `loft api-surface --check` / `api_diff` — breakage check | `⚠ BREAKING` flags on the source-pair diffs |
| `#superseded` / `LOFT_NO_STEER` — arc-C steering | `⬇ superseded → X` in the docs (one channel, two faces) |
| `lib-branch-audit.sh` — ahead/behind vs published | the `unreleased` tier + divergence detection |
| `list-installed` — installed / orphaned versions | `pinned` resolution + provenance |
| `check_registry_coverage.sh` — registry currency | published-vs-`origin/main` currency, shown inline |

Reduce "6 commands + hashes across 2 agents" to a single always-correct surface: the
committed catalogue for the shared truth, `loft api <name>` for the per-context overlay.

## Effort + design

### The source spectrum (settledness)

Five legitimate sources — **the fix is splitting them, not dropping any.** The confusion
was never `local` itself; it was **lumping `local` (a machine working checkout) and
`unreleased` (`origin/main`) into one "unpublished" bucket.** Split apart, each is clear.

| Tag | Source | Shared? → where | Answers |
|---|---|---|---|
| `✓ published` | registry latest / `~/.loft/registry/<pkg>-<ver>/` (pristine, sha256) | shared → **committed** | *what can I `loft install` now?* (can LAG origin/main) |
| `🟢 unreleased` | the library's `origin/main` — merged, not yet published | shared, deterministic → **committed** | *what is the library's correct CURRENT state?* (the regex `find→search` case) |
| `🔶 local` | a machine working checkout ahead of `origin/main` (uncommitted / unpushed WIP) | machine-specific → **overlay** | *what am I — the dev editing this lib — changing right now?* |
| `📌 pinned` | the version a project `.loft.lock` resolves to | per-project → **overlay** | *what does the code in front of me call today?* |
| `🌱 proposed` | **one or MORE** external unmerged candidates — proposal issue / PR / branch / dir, each a one-fn delta up to a whole-library rewrite | opt-in → **overlay** | *what is/are people proposing, not accepted?* |
| `⬇ superseded → X` | an authored `#superseded "X"` marker (loft's arc-C steering, @PLN102) | integrated into every view | this item is being replaced — the docs show the SAME `X→Y` steering the `LOFT_NO_STEER` lint enforces at compile time |

- `published` + `unreleased` are shared + deterministic (a released tarball; a specific
  `origin/main` commit) → they belong in the **committed, CI-checkable catalogue**, which
  is what makes it carry the *correct* current state, not just what's installable.
- `local` + `pinned` + `proposed` are machine-/context-/opt-in-specific → the **on-demand
  overlay** (`loft api <name>`). `local` is a **first-class source** for the dev editing
  that lib — never dropped; it just doesn't belong in a *shared* file, so it's overlay-only.

Once `local` and `unreleased` are distinct sources, neither confuses the other: a merged
origin/main change is shared truth in the committed doc; a dev's WIP is their own truth in
their overlay.

### `superseded` is integrated, not a side-badge — it's the arc-C steering, in the docs

The `#superseded "X"` markers are loft's existing **arc-C recommended-idiom channel**
(@PLN102 — the `LOFT_NO_STEER` lint warns a caller *"`find` is superseded — use `search`"*
at compile time). This plan **surfaces the SAME markers in the library docs**: the
generator reads each function's `#superseded` marker from every source and renders
`⬇ superseded → X` inline, so the steering the compiler enforces is *visible where an
agent browses the API*. One channel, two faces — the lint at build time, the catalogue at
read time — never two separate truths about "which name to use now."

### The layout rule — presentation driven by supersede LINKAGE, not by count

The rewrite-vs-delta split is decided by whether the diverging functions are **linked by
`#superseded` markers**, not by how many differ — because a fully-marked rename (regex
`find`→`search`, `split`→`split_on`) changes *most* functions yet is obviously a tidy
delta, not a rewrite:

1. **Plain list** — a single source, or all identical. One-line header, no badges.
2. **Tagged interleave (a DELTA)** — the diverging functions are **linked** old→new by
   `#superseded` markers (a rename/refactor, however sweeping). One list; the `⬇ → X`
   pointers do the work, the reader follows `find → search`.
3. **Side-by-side blocks (a REWRITE)** — the new source's functions are **unlinked** to the
   old (a genuinely different API, no supersede chain). Two coherent blocks + a delta
   summary (`kept · removed · added · sig-changed`), judged as a UNIT. This creates a
   healthy incentive: mark your renames and they read as a clean delta; ship an unmarked
   wholesale replacement and it correctly reads as a rewrite to evaluate.

### API-compatibility — the view FLAGS breakage, integrated from the existing checker

The reconciliation already diffs source APIs pair-wise; run loft's compatibility verdict on
each pair and surface the breaks. Reuse `api_diff::diff(old, new) → Verdict` — the
"identical-or-added is the whole rule" checker (@PLN102 C1, behind `loft api-surface
--check` / `make api-compat`):

- **`Superset`** (only additions) → compatible; no flag.
- **`Break(symbols)`** (a function removed, or a signature narrowed / changed) → each broken
  symbol gets a `⚠ BREAKING` badge and the library carries a `⚠ N breaking change(s) in
  <source>` header.

Where it bites, per source pair:
- **published → unreleased** — a break means `origin/main` will ship a contract-violating
  change; COMPATIBILITY.md says that is a **BUG at contract 1, not a managed change**, so
  flagging it in the *shared* catalogue surfaces it BEFORE it publishes.
- **published → proposed** — shows whether adopting a proposal / rewrite would break callers,
  as part of evaluating it.
- **pinned → published** — shows whether a project can safely upgrade off its pin.

`superseded` is the migration path, not an excuse. A properly **folded** rename (the old
name is kept and steered to the new — COMPATIBILITY.md § Folding) is a `Superset` → **not**
a break. A removal that merely *names* a replacement in a marker is still a `Break` (the old
symbol is gone) — so the docs show `⚠ BREAKING · find removed → use search`: the break flag
AND the migration pointer, together.

### `proposed` intake — a formal proposal issue external devs can file

Today there is a heavyweight PUBLISH flow (REGISTRY_SUBMIT.md — tag a release, build a
tarball, PR against `loft-lang/registry`) for a *finished* library, but **no lightweight
way for an external dev to propose a library or an API change/rewrite and have maintainers
see + evaluate it.** This plan adds that intake as the `proposed` source's front door:

- A **`library_proposal` GitHub issue template** (structured: name · purpose · **intended
  use case (fit)** · proposed public-API sigs · category · deps · existing repo/PR/branch if
  any), labeled `proposal`. That is the "fill in a gh issue we can see" — visible,
  triageable, one place. The *intended use case* field is load-bearing: it is what lets a
  reviewer judge fit-to-direction (see below), not just "does it run."
- `loft api <name> --proposal <issue#|PR#|owner/repo@branch|dir>` overlays the proposed API
  as `🌱 proposed`, diffed against `published` — including the **api-compat verdict** (would
  adopting it break callers?) and the rewrite-vs-delta layout.
- The loop closes end to end: external **proposal issue** → *seen + evaluated in the same
  view* → accepted → the existing REGISTRY_SUBMIT publish flow. One structured intake, and
  the provenance view is where you judge it — no separate evaluation tooling.
- **Multiple proposals per library are the norm, not an error.** Several proposal issues /
  PRs can target the same lib — competing deltas and rewrites. Each is a DISTINCT `proposed`
  candidate, tagged by its `#N`; `loft api <name>` lists *every* open proposal for the lib
  (each with its own api-compat verdict + delta-vs-rewrite classification against
  `published`), and `--proposal <ref>` overlays one — or several **side-by-side to compare**
  (published + candidate A + candidate B, an N-column matrix). The view helps you judge
  competing candidates; it **never auto-picks a winner** (provenance, not decision).
- **Functional ≠ fitting — the dangerous proposal.** The proposal to be careful about is
  not the broken one; it is the one that WORKS for many common use cases but does not serve
  the project's ENVISIONED one. A simple MariaDB connector (`~/workspace/loft-lib-mariadb`,
  the intake example) — one connection, simple SQL, a scalar `-> text` result, no pool — is
  fully functional and covers a lot of CRUD/scripting, yet it does NOT fit loft's envisioned
  database-support design — **the store IS the database** (persistence and in-memory state
  share one model: [DATABASE.md](../../DATABASE.md), [BROADENING.md § store-based heap as a
  language-level database](../../BROADENING.md)). A *fitting* connector materialises rows as
  loft structs/vectors IN THE STORE, at multi-record scale, backing the durable store — not
  a text-scalar external SQL client. The view
  surfaces the API, the api-compat verdict, and the divergence — but **fit-to-direction is
  human judgment the tool SUPPORTS (full provenance, side-by-side, the proposer's own
  use-case framing), never replaces.** This is *why* the view never auto-adopts, and why the
  `library_proposal` template asks for the **intended use case** — so a reviewer judges fit,
  not just "does it run." The seductive-looking-but-misfit proposal is the whole reason
  provenance beats automation here.

### Reconciliation — union, tagged by membership (both directions)

Key each function by `name` (+ arity). Take the union across every present source; tag by
membership. Load-bearing rule: **membership never implies `superseded`.**

- in `published` (whether or not local/pin has it) → `published` — installable today; a
  published fn a lagging source LACKS is *still* `published` ("upgrade to get it"), never
  hidden. (The "newer published version has functions not in the local version" case.)
- in `local`/`proposed` but NOT published → `local` / `proposed` (not installable yet).
- `superseded → X` **only** from an authored `#superseded "X"` marker — never inferred
  from "one side lacks it."
- same name, different sig → show each sig with its tag.

### No auto-clean — provenance, not pruning

We do **not** auto-delete anything (installed downloads, checkouts, proposal branches).
Each is potentially a source of truth: a `.loft.lock` references an old download on
purpose; a checkout holds WIP; a proposal branch is a live candidate. Deleting loses
information — the exact failure that started this. `loft list-installed` / the branch audit
**report and flag** (orphan / ahead-of-published) as *information*, never remove. Cleanup,
if wanted, stays a deliberate manual act.

### Shared doc vs machine-specific state — split by shared-ness

- **Committed `LIBRARIES.md` = `published` + `unreleased`** — both are shared and
  deterministic (a released tarball; a specific `origin/main` commit), so both are
  CI-`--check`-able. The generator reads the registry AND each library's `origin/main`
  (a cheap sha/content check; fetch the diverging API only where they differ) and tags the
  merged-but-unreleased functions. This is what makes the shared doc carry the CORRECT
  current state, not merely what's installable — the exact gap that hid the regex rename.
  Cost: the committed doc regenerates when a library's `origin/main` moves (a scheduled /
  CI regen, not churn the reader manages).
- **Overlay = on-demand, per-context** — `loft api <name>` adds the machine-/context-
  specific sources: `local` (a dev working checkout — first-class truth for whoever is
  editing that lib), `pinned` (this project's lock), `proposed` (an explicit ref). Output
  is stdout / a gitignored file, so git never carries a machine's WIP/pins/proposals — but
  each is a valid source, shown whenever it is present. None is dropped; they are simply
  not *shared*, so they live in the overlay rather than the committed catalogue.

### Reuse — nothing new to extract

registry `api` field ← `documentation::pkg_api_items` (also `loft api`); `pkg_api_items`
on any dir for local/pinned/proposal source; `gh api …/contents` for a proposal ref;
`scripts/lib-branch-audit.sh` (squash-safe) for ahead/behind; `homepage` for name→repo.

## Sub-arcs (phases — each independently landable)

1. **Published enrichment** (prototyped): registry `api` → `LIBRARIES.md` (version + sigs
   + triggers) + the plain/tagged adaptive renderer. Commit + CI `--check`.
2. **`unreleased` tier IN the committed doc:** for each lib, `gh`-check whether its
   `origin/main` diverges from the published API (reuse the branch-audit content-compare);
   where it does, fetch + tag the merged-but-unreleased functions into `LIBRARIES.md`.
   Deterministic → still CI-`--check`-able; regenerate when a lib's `origin/main` moves.
   This is the tier that makes the shared doc CORRECT (closes the regex-rename gap).
3. **Overlay engine (per-context):** `loft api <name>` adds the machine-/context-specific
   sources — `local` (dev working checkout) and `pinned` (lockfile) — via the union +
   adaptive renderer → stdout / gitignored. Each is a valid source, shown when present.
4. **`--proposal <ref>`:** fetch + overlay an external candidate; detect rewrite →
   side-by-side blocks.
5. **API-compat flags:** run `api_diff::diff` on each source pair (published↔unreleased in
   the committed doc; published↔proposed and pinned↔published in the overlay); render
   `⚠ BREAKING` badges + a per-library `⚠ N breaking change(s)` header. A folded rename is
   a `Superset` → no flag; an unmarked removal/sig-change is a `Break`. Reuses
   `loft api-surface` / `make api-compat`.
6. **Docs:** `LIBRARIES.md` header + `CLAUDE.md` — "single current source, correct to each
   lib's `origin/main`, flagging any breakage; `loft api <name>` for pinned/proposed; never
   read a lib clone to learn an API; we never auto-delete a copy."
7. **Automation (see § Staying current):** wire the triggers → idempotent `--refresh` regen
   → auto-merged bot PR. The committed doc self-updates; the overlay is on-demand.

## Staying current — full automation (the triggers)

Two halves, and only one can go stale:

- The **overlay** (`local` / `pinned` / `proposed` via `loft api <name>`) is computed **on
  demand**, so it is **never stale by construction** — no trigger needed.
- The **committed catalogue** (`published` + `unreleased`) is a snapshot, so it needs
  triggers. It is **fully automatable** — no human ever runs a command:

| Trigger | Fires | Keeps fresh |
|---|---|---|
| a library **publishes** a version | the registry repo's publish workflow → `repository_dispatch` at the loft repo | `published` |
| a loft-libs-* **`origin/main`** merge changes the public API | that repo's on-push-to-main workflow → same dispatch | `unreleased` |
| **nightly cron** in the loft repo | scheduled — regen from live registry + every lib's `origin/main` | the FALLBACK — catches any repo missing the dispatch, and any drift |
| a **loft-repo PR** | CI `--check` (already exists) | correctness gate — no human can commit a stale catalogue |

Two properties make this safe and churn-free:
- **Idempotent regen** — `gen-library-catalogue.py --refresh` (published + unreleased +
  compat + superseded) yields byte-identical output when a library commit did NOT change
  its public API → **no commit, no churn**; only a real API change moves the doc.
- **Branch policy respected** — main is PR-only, so the job opens / updates an
  **auto-merged bot PR**, never a direct push. (Reuses the existing nightly lib-CI +
  workflow-scope-push setup.)

Event-driven dispatches give minutes-fresh; the nightly is the robust floor; the CI check
is the gate. So the committed doc self-updates on any publish or API-changing merge, and
the overlay is always-current because it is recomputed every call.

## Performance & caching

**The invariant: check cheap, reuse the rest.** One batched sha-verify detects external
changes across ALL libs in ~1 round-trip; every source whose sha is unchanged is reused
from the content-addressed cache; only what actually MOVED is refetched + re-extracted. The
steady-state cost is one cheap freshness check, not N fetches — and because the cache is
sha-keyed, "not stale" is a *proof* (matching sha = identical content), not a guess.

**Cost model — network dominates, not CPU.** The routine is bounded by `gh` round-trips for
`unreleased` / `proposed`; `published` is a local JSON read (the registry `api` field),
extraction (`pkg_api_items`) is a moderate parse, and the api-compat `diff` + render are
in-memory and cheap.

**Two-layer content-addressed cache — a hit is provably correct.** Because we diff MANY
versions of the same lib (published × N proposals, unreleased, a pin), cache in two layers,
both keyed by the **source SHA** (registry `sha256` / `origin/main` sub-path commit sha / PR
head sha):

1. **Source cache — the `.loft` source itself.** This is the expensive-to-fetch **ground
   truth**; for `published` it already IS the installed tarball (`~/.loft/registry/<pkg>-
   <ver>/`), extended to `unreleased`/`proposed` under the same sha key. Immutable, so it
   holds *every* version we've diffed (ready for a re-diff, an N-way compare, or a
   source-level side-by-side) and — unlike a git checkout — it can never masquerade as
   "current": the sha names exactly which version it is.
2. **Derived cache — extracted API + compat verdicts**, keyed by *source-sha +
   extractor-version*. Derived from layer 1; if the extractor (`pkg_api_items`) improves,
   re-extract from the CACHED SOURCE with **no network**.

The sha is the content, so a hit is provably correct — never stale-wrong. Caching the
SOURCE (not just the API) is exactly why a re-diff, an extractor upgrade, or a *new* rival
proposal to compare against costs **zero refetch** for any version already seen.

**Verifying staleness = ONE cheap sha check, batched.** To decide if an entry is stale,
fetch only the source's current **sha** (tiny), never its content — and **all N libs'
`origin/main` shas come back in a single GraphQL round-trip** (`repository { object /
history(path:) }` batched across repos). So checking the *entire* catalogue's freshness is
~1 network call; only the libs whose sha MOVED are refetched + re-extracted. Sha match →
cache valid (skip the fetch+parse); sha change → refetch. Cheap verify, correct by
construction — and note there is NO local clone to keep fresh (the exact stale-source that
started this), because every read is `gh` against the authoritative ref.

**Event-driven beats polling.** The automation's `repository_dispatch` invalidates the
affected lib's cache entry the moment it changes, so the cache is usually warm before a
read; the batched sha-verify is the nightly FALLBACK, not the hot path.

**Net latencies.** `loft api <name>` (one lib) → cache hit sub-second (read cache, compat +
render in memory), miss = a couple `gh` calls + one parse. Committed regen (all libs) → the
~1-GraphQL sha-verify + re-extract only the changed libs; idempotent, so an unchanged run is
a fast no-op (no re-extract, no commit). The committed catalogue is itself the top-level
cache — valid until some lib's published version or `origin/main` sub-path sha moves.

## Open design questions

- Overlay as a second gitignored file vs a fenced, CI-stripped block in `LIBRARIES.md`?
- Dev-root discovery: scan `~/workspace` vs a per-lib path config?
- `proposed` intake — the `library_proposal` issue template's fields; which `--proposal
  <ref>` forms first (issue# / PR# / `owner/repo@branch` / dir); the N-way compare UX for
  competing proposals.

## Edge cases (must hold)

- Single source, or all sources identical → plain list, no tags.
- Published NEWER than pin/local → shown `published` ("upgrade to get it"), never hidden,
  never marked superseded for being absent from a lagging source.
- Pin OLDER than published → pinned set is the *effective* (callable-here) API; newer
  published fns shown but flagged not-callable-until-upgrade.
- Diverged (ahead on some, behind on others) → each fn tagged independently.
- `superseded` only from an authored marker, never inferred.
- Proposed (or local) is a whole-library **rewrite** → side-by-side + delta summary, never
  a per-line badge storm; judged as coherent units, neither silently merged.
- Committed `LIBRARIES.md` contains ZERO non-published lines (CI proves it); overlay is
  gitignored/stdout.
- Nothing is ever auto-deleted.

## See also

- [LIBRARIES.md](../../LIBRARIES.md) — the committed published catalogue (phase 1 target).
- `scripts/gen-library-catalogue.py` — the generator (phase 1 + overlay engine).
- `scripts/lib-branch-audit.sh` — squash-safe ahead/behind content compare (phase 2).
- `src/documentation.rs` (`pkg_api_items`) + `loft api` (`src/main.rs`) — API extraction.
- `src/api_diff.rs` (`diff → Verdict::Superset | Break`) + `loft api-surface --check` /
  `make api-compat` — the breakage checker phase 5 reuses (@PLN102 C1).
- [PKG_REGISTRY.md](../../PKG_REGISTRY.md) · [LIBRARIES rules in CLAUDE.md](../../../../CLAUDE.md).
- loft-libs-core#23 (`d5e4195`) — the merged regex rename whose invisibility motivated this.
- [COMPATIBILITY.md § Folding](../../COMPATIBILITY.md) + @PLN102 arc C — the `#superseded`
  steering channel (`LOFT_NO_STEER`) this plan integrates into the library docs (one
  channel, two faces: the lint at build time, the catalogue at read time).
- **The envisioned database-support design** — what a DB-connector proposal is judged for
  fit against: [DATABASE.md](../../DATABASE.md) (the store IS the database — Store/DbRef +
  @PLN43 durability + `store_persist_bind`) and [BROADENING.md](../../BROADENING.md)
  ("store + language co-designed embedded-DB"). `~/workspace/loft-lib-mariadb` is the
  external-scalar-SQL MISFIT that motivated the functional≠fitting caution above.
