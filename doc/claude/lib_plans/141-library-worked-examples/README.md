<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN141 — Worked examples for the current libraries

> Tracker: [loft-lang/plans#141](https://github.com/loft-lang/plans/issues/141)
> (`subject:libs`, `status:future` → move to `status:active`). Origin: dryopea's
> `docs/EXAMPLES.md` gate (commits `9cf8a01` + `1f4c35f`, 2026-08-17) — this idea
> rolled out across the loft library + feature ecosystem. Built on branch
> `mac-install-dylib-fix` (alongside @PLN142).

## Status

ACTIVE — stdlib rollout underway. The loft **stdlib** is the starting library
(source 3, in-repo, loft's own gate — the cleanest arrow): twelve functions across
four clusters carry `// Example: @STD-0NN` citations — text (`starts_with_at`,
`chr`, `join`), collection aggregates (`min_of`, `max_of`, `sum`, `tree_walk`), JSON
(`json_parse` navigation @STD-007, the `json_array`/`json_object` builders @STD-008,
the `struct_from_jsonvalue`/`struct_to_json` round-trip @STD-009), and files/IO (the
missing-vs-empty `content`/`read_bytes`/`list_dir` `T?` contract @STD-010, `lines()`
CRLF-normalisation @STD-011, the `FileResult`/`ok()` classify idiom @STD-012) —
resolving to tagged tests (two examples reuse existing clear tests —
`445-generic-tree-walk.loft` for `tree_walk`, `562-file-read-missing-null.loft` for
the null contract — the rest live in `945-stdlib-worked-examples.loft` and the
filesystem-isolated `946-stdlib-file-worked-examples.loft`, all green on both
backends). The gate is `scripts/check_doc_drift.sh examples` (dangling + duplicate),
wired into the `all` run that CI already blocks on — proven red on both a dangling
citation and a duplicate tag. The stdlib clusters are covered; next is either
**Phase C** (teach `scripts/idx`/`make index` to ingest `@AAA-###`) or the first
**registered library** (`arguments`/`hex_grid`/`gridmesh`).

**Cross-repo resolution now works (Phase B + first-class-app source).** The acronym
registry lives at `scripts/example_repos.tsv` (acronym → repo → git url → branch).
Projects are assumed checked out as **siblings** in one parent dir, so a repo's
checkout is simply `../<repo>` (no per-entry path navigation; the current repo is
scanned in place). A `// Example: @AAA-###` citation is resolved by acronym:
- the owning repo's tag may sit above **any `fn`** — a `test_*` OR a real function in
  a first-class application's own source (dryopea tags both `tests/` and `src/`);
- a repo with a **local checkout is validated offline against it** (preferred) and
  the check emits the real git blob link (verified live: `@DRY-001` →
  `dryopea/blob/main/tests/25_m3_the_ground_gl.loft#L43`);
- a cross-repo tag whose repo is **not** checked out is reported `unvalidated` (a
  warning, not a hard failure — keeps CI deterministic) with the link still emitted.

Follow-up: hard *online* validation without a checkout (fetch a published per-repo
tag index) — today the offline local-checkout path is the validated one.

## Goal

For a library function whose **correct use is not obvious from its signature +
doc**, point the reader at a **real call site** instead of a prose snippet. Three
sources, in priority order:

1. **The library's own tests** — a test that already demonstrates the function,
   made findable by a stable tag.
2. **A real use in a first-class application** (`moros`, `dryopea`, `crawler`) or
   its tests — the strongest example, because it is the function doing a real job.
3. **A loft in-repo demo application or example program** — `examples/*.loft` and
   the `gallery` / `game` / `play` / `crystal-editor` / `native-editor` demos. For a
   **stdlib function or a language feature** this is often the best *and* cleanest
   source: it is a real program a user runs, and it lives in the loft tree itself, so
   it resolves through loft's own gate with **no cross-repo arrow** (unlike source 2).

The same applies to the **loft feature catalogue** (`@F`/`@I` issues): a language
feature also wants a tag pointing at how it is used **in real life**, on top of the
synthetic run-snippet the catalogue already generates (Phase C2).

The example is a *pointer to working code*, never a snippet in a comment: a snippet
rots silently at the first signature change; a tagged test cannot drift from working
code because it **is** working code (dryopea `EXAMPLES.md § Why not a snippet`).

**Scope discipline (the heart of this plan).** Tag a function **only where a real
call site teaches more than its signature does**. A one-line accessor, a function
whose usage is self-evident from its type — no example; the doc already suffices.
No retroactive sweep of every `pub fn`. This mirrors dryopea's opt-in ratchet, and
is what keeps the gate from going red on hundreds of trivially-obvious functions the
day it lands.

**When no clear example exists yet, write one — in retrospect.** The two sources
above may need to be **created**, not merely cited: if a non-obvious function or
feature has no test that shows it used *correctly*, authoring that test is part of
the deliverable (dryopea `EXAMPLES.md` rule 4 — write a new test if none of the
existing ones is CLEAR). The signal for *which* functions owe one is concrete, and
is the motivation for this plan: **a reader — often an AI agent — that knows a
function/feature exists but cannot use it effectively from its doc alone, and only
succeeds once pointed at an application that already uses it, is exactly the case
that owes a worked example.** Lift the pattern from that real usage into a tagged
test, so the next reader never needs the pointer. This is not a contradiction of the
ratchet: the ratchet says don't tag the *obvious*; this says for the *non-obvious*,
a missing example is a gap to fill, not a reason to skip.

## The mechanism (as built — adapted from dryopea)

- **Tag family `@AAA-###`** — `@`, three-letter acronym, hyphen, three digits
  (`@STD-001`). Distinct from loft's `@P`/`@PLN`/`@F`/`@GH` families (none has that
  hyphen). One acronym per repo, claimed **ecosystem-globally**.
- **Citation** — `// Example: @STD-001` on a comment line directly above the
  function it documents (stdlib fn in `default/`, library fn in `lib/`).
- **Definition** — the tag sits in a comment block directly above the `fn` it names
  (a blank line breaks the block). The `fn` may be a `test_*` **or a real function
  in a first-class application's own source** — a live use documents as well as a
  test does.
- **Acronym registry** — `scripts/example_repos.tsv`: `acronym → repo → git_url →
  branch`. Projects are assumed checked out as **siblings** in one parent dir, so a
  repo's checkout is `../<repo>`.
- **Gate** — `scripts/check_doc_drift.sh examples` (part of the `all` run CI's
  doc-hygiene job already blocks on). Faults: `dangling` (cited, no fn carries it),
  `duplicate` (one tag on two fns in this repo), `unregistered` (acronym not in the
  registry) — all drift; `unvalidated` (cross-repo tag whose sibling isn't checked
  out) — warning only, link still emitted. Proven red on dangling + duplicate.
  *Not yet built:* `orphan`, opt-in `uncovered`, and a self-test harness — deferred
  (the non-vacuous set of real @STD citations is what keeps the green honest today).

### Design decision — which way the cross-repo arrow points

A library must **not** gain a code dependency on its consumers, and a first-class
app is not a dep in the library's `loft.toml`. So the two sources resolve
differently, and this split is load-bearing:

| source | tag DEFINED in | citation lives on | resolved by (as built) |
|---|---|---|---|
| library's own test | this repo's `tests/` | the fn's doc comment | `check_doc_drift.sh examples`, scanned in place |
| loft in-repo demo / example program | this repo's `examples/` / `tools/` | the stdlib fn / feature issue | same — same tree, no cross-repo arrow |
| real use in a first-class app | the app's `tests/` **or its `src/`** | the stdlib / library fn | the registry maps the acronym to `../<repo>`; validated offline against that sibling checkout, git link emitted |

The gate only ever checks a citation against the repo its **acronym** names, and a
foreign repo is consulted **read-only** via its sibling checkout — never a build
edge, so every library still builds and tests standalone. A cross-repo tag whose
sibling isn't checked out is a warning, not a failure, so CI stays deterministic.
The demo-program row is the case that needs no foreign repo at all: a loft
`examples/` program documenting a stdlib function or feature is in the **same tree**
as what it documents — the preferred source for stdlib + feature examples.

## Phases

Cut per the two-bounds rule (each can go red on its own, for a real reason).

### Phase A — Probe: does the cross-repo pointer work, and does it stay inert? — DONE

Done differently than first sketched (no `arguments`/`examples.sh` port): the probe
rode the real gate. A temporary `// Example: @DRY-001` in loft's stdlib **validated
against the sibling `../dryopea` checkout** and emitted the git blob link; `@DRY-999`
went `dangling`; `@MOR-001` (moros not checked out) stayed an `unvalidated` warning
without failing. Confirms all three: the cross-repo pointer resolves, it produces a
real link, and it is inert (read-only, never a build edge) when the foreign repo is
absent.

### Phase B — The acronym registry — DONE (foundation), still to broaden

Built as `scripts/example_repos.tsv` (a loft-repo file for now, not yet the
`loft-registry` shared home — see Open questions). Entered so far: `STD` loft ·
`GIT` loft (the in-tree `lib/git`; one repo may own several acronyms) · `DRY`
dryopea · `CRW` crawler · `MOR` moros. To add as the rollout reaches them: `ARG`
arguments · `CRY` crypto · `GMP` game_protocol · `GRM` gridmesh · `RND` random ·
`SRV` server · `SHP` shapes · `WEB` web · `HXG` hex_grid · `HXT` hex_terrain · `HXW`
hex_world · `MKD` markdown · `GFX` graphics · `FTR` the feature catalogue (Phase C2).
(`DRY`/`FIX`/`TST` also claimed by dryopea.)
- **Still to do:** a duplicate-**acronym** guard (two repos claiming one acronym) —
  the gate today guards duplicate *tags* within a repo, not acronym collisions across
  the registry.

### Phase C — Shared gate + indexer ingestion — PARTIAL

- **Done:** the gate is one shared script — `scripts/check_doc_drift.sh examples` —
  not a per-library copy; it already resolves cross-repo citations and **emits the
  git link** (the "carry a real link" half of what the indexer owes).
- **Done:** loft's tag indexer (`make index` / `scripts/idx`) now ingests
  `@AAA-###`, so `scripts/idx tag:@STD-011` resolves both the citation and the
  demonstrating fn. Added one branch to `tools/indexer/src/scan.loft`'s per-byte
  `scan_line` dispatch (three uppercase letters, hyphen, three digits; the interior
  hyphen disambiguates from `@P`/`@PLAN`/`@F`/`@GH`, and it is checked AFTER `@F`/`@I`
  so `@FTR-001` is not eaten by the `@F` branch). `tag_is_valid` already accepts the
  shape via its fallthrough, so `idx broken` stays `[]`; the query side (`scripts/idx
  tag:…`) is a generic bucket lookup and needed no change. Verified: all 16
  `@AAA`-shaped tags in the tree index (12 `@STD` + the plan's `@DRY`/`@MOR`/`@FTR`
  mechanism examples), no accidental matches, `idx broken` + `broken-links` clean.
- **Still to do (Follow-up from Status):** hard *online* validation of a cross-repo
  tag when its sibling isn't checked out (fetch a published per-repo tag index),
  crawling a `~/.loft/`-style hidden root correctly (dryopea's traversal-from-root
  bug — a `--exclude-dir='.*'` scan reads zero files under any hidden path).

### Phase C2 — Worked examples for the feature catalogue (S)

The same mechanism applies to the **feature catalogue** (`loft-lang/features`,
`@F<N>` feature / `@I<N>` infra), not just library functions — a reader learning a
language *feature* is best served by seeing it used **in real life**, not only by a
synthetic snippet.

- **What exists today:** the generator promotes the **first** ` ```loft ` fence in a
  feature issue into a RUN test (`tests/docs/features/*.loft`). That proves the
  feature compiles and runs, but it is a teaching snippet — it does not show the
  feature carrying real work.
- **What this adds:** a feature issue gains an `Example:` citation to a `@AAA-###`
  tag naming a **real** test or first-class-app usage that exercises the feature.
  Same tag family, same indexer (Phase C), same `dangling`/`orphan` gate. The two
  are complementary and both kept: the first-fence RUN test is the minimal
  compiles-and-runs gate; the tag is the pointer to idiomatic real-life use. The
  **preferred source for a feature** is a loft in-repo demo/example program (source 3
  above) — same tree, no cross-repo arrow. Where a feature has no test or demo
  showing real use, **author one in retrospect** — the same rule as the library
  rollout.
- **Where the citation lives:** in the **issue body** (canonical), never in the
  generated shadow (`index/features.json`, `doc/features/`, `tests/docs/features/`)
  — the `features-check` drift guard fails on hand-edits. Edit the issue, then
  `make features-fetch && make features-gen`.
- **Acronym:** the catalogue gets its own — `FTR` — registered in Phase B alongside
  the libraries', since a feature tag shares the ecosystem-global namespace.
- **Validation (can go red):** `make features-check` stays green; a feature citing a
  deleted test goes `dangling` through the same gate; `idx tag:@FTR-001` resolves
  the real-usage test.

### Phases D… — Per-library rollout, one library per phase (S each)

**Started with the loft stdlib** (`@STD`, source 3, in-repo): text
(`starts_with_at`, `chr`, `join`, @STD-001..003), collection aggregates
(`min_of`/`max_of` @STD-004, `sum` @STD-005, `tree_walk` @STD-006 — the last reusing
an existing test), JSON (`json_parse` navigation @STD-007, the
`json_array`/`json_object` builders @STD-008, the struct↔JSON round-trip @STD-009),
and files/IO (the missing-vs-empty `content`/`read_bytes`/`list_dir` contract
@STD-010 — reusing `562-file-read-missing-null.loft` — `lines()` CRLF-normalisation
@STD-011, the `FileResult`/`ok()` classify idiom @STD-012). The four stdlib clusters
are covered; the rollout moves to the registered libraries below.

**First library shipped: `lib/git`** (`@GIT-001..005`). The rollout's ideal
source — not authored tests but **real, exercised call sites** in two in-tree
consumers that run every `make index` / `make view`: `tools/indexer/src/scan.loft`
(`tracked_files` @GIT-001) and `tools/viewer/refresh.loft` (`ahead_behind`
@GIT-002, `log` vs `head` date @GIT-003, `changed` rename→new-path @GIT-004,
`numstat` `-1`-is-binary @GIT-005) — each tag on the exact function whose silent
trap the library's own doc calls out. `GIT` is registered → `loft` (a repo may
own several acronyms; the def scan finds `tools/` too), so `idx tag:@GIT-002`
resolves both citation and call site. Gate green, both consumers still run.

**In-tree libraries roll out first (edit-only-this-repo).** The priority order
below leads with cross-repo registered libraries (`arguments`/`hex_grid`/…), but
the dogfood rule forbids editing a consumer's repo, so their tags must be authored
by *their* agents. What this stream can do directly is the **in-tree** libraries
(`lib/git` — done — then `lib/html`, `lib/markdown`, `lib/input`, `lib/logger`),
where the strongest example is a **real call site in an in-tree consumer** that
already runs in CI/tooling (`lib/git` tagged live uses in `scan.loft` +
`refresh.loft`, no test authored). A cross-repo library's rollout waits until its
own repo opts in.

One library per phase (the skill's one-call-site-at-a-time shape). For each: tag the
highest-value functions from its own tests and, where it exists, a real consumer
usage — and where **no** clear demonstrating test exists, **author one in retrospect**
(lifting the pattern from a first-class app that already uses the function correctly)
rather than skipping the function; the shared `check_doc_drift.sh examples` gate
covers it (no per-library `examples.sh` to wire).
- **Validation per library:** gate green on that library **and** a deliberately
  deleted test makes a citation dangle.
- **Priority order** (non-obvious usage first, where a real call site pays most):
  1. `arguments`, `hex_grid`, `gridmesh` — stateful/geometry APIs where the shape of
     a correct call is the whole question.
  2. `crypto`, `server`, `web` — protocol/sequence APIs (order of calls matters).
  3. `markdown`, `graphics`, `random`, `shapes`, `game_protocol` — as they're touched.
  4. `hex_terrain`, `hex_world`, in-repo `moros_*` — opportunistic (opt in when a file
     is finished; the ratchet only goes up).

### Phase (last) — Convention doc + CI ratchet (S)

One upstream doc of the convention (libraries share it, not a per-repo copy), and
each library's CI runs its gate. Ratchet: no sweep, a file opts in when someone
finishes work in it — a gate red on every function the day it lands is switched off
within a week.

### Monthly by-hand review — the ongoing home (DONE, first pass 2026-08)

The automated `check_doc_drift.sh examples` gate only sees a citation that
*dangles* or *duplicates*. Two failures need a human: a doc that still resolves
yet no longer matches the code (**staleness**), and an example that is valid but
no longer the clearest (**quality**). Those are handled by a monthly by-hand pass
at the release beat — [../../LIBRARY_DOC_REVIEW.md](../../LIBRARY_DOC_REVIEW.md),
driven by `scripts/doc-review.sh` (coverage + citation inventory + a
changed-since-watermark worklist), wired into [RELEASE.md](../../RELEASE.md)'s
monthly cadence. It is a hygiene **ratchet, never a release blocker** — the
watermark bounds each pass to what moved. This is where the per-library rollout's
staleness/quality upkeep lives once a library is tagged.

## Open questions

- **Registry home** — built as `scripts/example_repos.tsv` in the loft repo. Should
  it migrate to `loft-registry` (the ecosystem-shared home) so every repo reads one
  copy? Leans yes eventually; the loft-repo file is fine while loft is the only
  consumer.
- **Online cross-repo validation** — a sibling checkout validates offline today; a
  repo not checked out only warns. A published per-repo tag index (one small file
  fetched by raw URL) would let it hard-validate offline-of-clone. (Phase C follow-up.)
- **`// Example:` spelling** — kept dryopea's spelling for one shared indexer.
- **In-repo `lib/moros_*`** — first-class-app-internal libs; lower priority than the
  registered/sibling libraries a broad audience consumes.

## See also

- dryopea `docs/EXAMPLES.md` — the origin design + the `examples.sh` gate this adapts.
- dryopea `plans/26-the-fixed-step` — first library shipped under the convention.
- The loft-side gap this plan closes: `scripts/idx` + `make index` recognise
  `@P`/`@PLN`/`@F`/`@GH` today, not `@AAA-###`, and crawl only the loft tree.
- Feature catalogue (Phase C2): `loft-lang/features` issues + the
  `make features-fetch/gen/check` shadow; the generator's first-` ```loft `-fence
  RUN test is the compiles-and-runs gate the real-usage tag complements.
