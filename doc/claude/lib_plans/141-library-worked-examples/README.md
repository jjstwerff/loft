<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN141 — Worked examples for the current libraries

> Tracker: [loft-lang/plans#141](https://github.com/loft-lang/plans/issues/141)
> (`subject:libs` + `status:future`). **Home:** place this file at
> `doc/claude/lib_plans/141-library-worked-examples/README.md` when work starts on a
> branch (kept out of the unrelated install branch on purpose). Origin: dryopea's
> `docs/EXAMPLES.md` gate (commits `9cf8a01` + `1f4c35f`, 2026-08-17) — this idea
> rolled out across the loft library + feature ecosystem.

## Status

ACTIVE — first slice shipped. The loft **stdlib** is the starting library (source
3, in-repo, loft's own gate — the cleanest arrow): three text functions
(`starts_with_at`, `chr`, `join`) carry `// Example: @STD-00N` citations resolving
to tagged tests in `tests/scripts/945-stdlib-worked-examples.loft`, and the gate is
`scripts/check_doc_drift.sh examples` (dangling + duplicate), wired into the `all`
run that CI already blocks on — proven red on both a dangling citation and a
duplicate tag. Next: broaden the stdlib clusters, then Phase B (acronym registry)
and the registered libraries.

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

## The mechanism (adapted from dryopea)

- **Tag family `@AAA-###`** — `@`, three-letter acronym, hyphen, three digits
  (`@ARG-001`). Same indexer family as loft's `@P`/`@PLN`; the hyphen keeps it
  distinct. One acronym per library, claimed **ecosystem-globally** (two libraries
  claiming `@GRM-` collide in a shared index even though neither repo sees the
  other).
- **Citation** — `// Example: @ARG-001, @ARG-002` directly above a `pub fn`, or
  `// Example: none — <reason>` for an opted-in file's function that needs none.
- **Definition** — the tag sits in a comment block directly above the test it names
  (a blank line breaks the block, so a header tag can't drift down and claim an
  unrelated test).
- **Gate** — `examples.sh` per library, checking `dangling` / `duplicate` /
  `orphan` / (opt-in) `uncovered`, with a self-test that proves each fault *fires*.

### Design decision — which way the cross-repo arrow points

A library must **not** gain a code dependency on its consumers, and a first-class
app is not a dep in the library's `loft.toml`. So the two sources resolve
differently, and this split is load-bearing:

| source | tag DEFINED in | citation lives on | resolved by | orphan checked by |
|---|---|---|---|---|
| library's own test | the library's `tests/` | the library's `pub fn` | the library's own `examples.sh` (primary tree) | the library's gate |
| loft in-repo demo / example program | loft's `examples/` (or a demo's test) | the stdlib fn / feature issue | loft's **own** gate — same tree, no arrow | loft's gate |
| real use in a first-class app | the **app's** tests | (discoverable via the indexer) | the **ecosystem indexer** (Phase C) — an inert doc pointer, never a build edge | the **app's** own gate |

The library's gate only ever checks its **own** in-repo citations — the library
still builds and tests standalone with every consumer absent. "Seen used in `moros`"
is an *indexer* feature, not a library-gate feature. This is exactly dryopea's own
split: `EXAMPLES_TEST_ROOTS` resolves citations into registered libs, and `orphan`
is asked only of the primary tree so a consumer is never red for a dependency's tags.
The demo-program row is the middle case that needs neither: a loft `examples/`
program documenting a stdlib function or feature is in the **same tree** as what it
documents, so it resolves like a library's own test — no cross-repo arrow, no
indexer dependency. That is why it is the preferred source for stdlib + feature
examples.

## Phases

Cut per the two-bounds rule (each can go red on its own, for a real reason).

### Phase A — Probe: does the cross-repo pointer work, and does it stay inert? (XS)

Pick **one** library with genuinely non-obvious usage — `arguments` (parser
lifecycle) — and **one** first-class consumer that already uses it.
- Port `examples.sh` + its 8-control self-test into `arguments`.
- Tag two things: one test in `arguments/tests`, one real usage in the consumer's
  tests. Add `// Example:` on the two `pub fn`s.
- **Validation (must be able to go red):** self-test's 8 controls pass; deleting the
  cited test makes `dangling` fire; **and** the consumer-side citation does NOT make
  `arguments` fail to build/test when the consumer is absent (the pointer is inert).
- **Output:** the arrow-direction + resolution config from the table above, confirmed
  or corrected in writing. Kills the design for the cost of one compile if cross-repo
  resolution is unworkable.

### Phase B — The acronym registry (S)

The namespace is ecosystem-global, so it needs one home. Stand up the minimal
registry (a table in `loft-registry`, the ecosystem's shared place — not a
per-library copy) and assign acronyms to the current libraries.
- **Validation:** a duplicate-acronym check with a deliberate-collision fixture that
  makes it go red. A green run over the real set means no two libraries collide.
- Proposed acronyms (ratified here): `ARG` arguments · `CRY` crypto · `GMP`
  game_protocol · `GRM` gridmesh · `RND` random · `SRV` server · `SHP` shapes ·
  `WEB` web · `HXG` hex_grid · `HXT` hex_terrain · `HXW` hex_world · `MKD` markdown ·
  `GFX` graphics · `FTR` the feature catalogue (Phase C2). (`DRY`/`FIX`/`TST`
  already claimed by dryopea.)

### Phase C — Shared gate + indexer ingestion (M)

- Promote `examples.sh` to **one** upstream copy libraries consume, instead of each
  copy-pasting it.
- Teach loft's indexer (`make index` / `scripts/idx`) to ingest `@AAA-###` tags so
  `scripts/idx tag:@ARG-001` resolves the tagged test — this is the piece dryopea's
  `EXAMPLES.md § What is NOT decided` explicitly defers to "wherever the indexer is
  defined." This is what makes a "real use in a first-class app" *discoverable*.
- **Validation:** `idx tag:@ARG-001` resolves; a dangling tag reports; the indexer
  crawls a `~/.loft/`-style hidden root (dryopea's traversal-from-root bug — a
  `--exclude-dir='.*'` scan reads zero files under any hidden path).
- **Deferrable:** if per-library gates suffice for now, C can defer with the trigger
  "cross-repo 'used in <app>' links wanted".

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

One library per phase (the skill's one-call-site-at-a-time shape). For each: opt in
the files worth documenting (`// #examples`), tag the highest-value functions from
its own tests and, where it exists, a real consumer usage — and where **no** clear
demonstrating test exists, **author one in retrospect** (lifting the pattern from a
first-class app that already uses the function correctly) rather than skipping the
function; wire `examples.sh` into the library's `test.sh`/CI.
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

## Open questions

- **Registry home** — `loft-registry` table vs a loft doc, if the registry isn't
  ready to carry it. (Phase B decides; leans `loft-registry`.)
- **`// Example:` spelling** — cheap to change now, expensive once citations exist.
  Keep dryopea's spelling for one shared indexer unless Phase A surfaces a reason.
- **In-repo `lib/moros_*`** — these are first-class-app-internal libs; likely lower
  priority than the registered/sibling libraries a broad audience consumes.

## See also

- dryopea `docs/EXAMPLES.md` — the origin design + the `examples.sh` gate this adapts.
- dryopea `plans/26-the-fixed-step` — first library shipped under the convention.
- The loft-side gap this plan closes: `scripts/idx` + `make index` recognise
  `@P`/`@PLN`/`@F`/`@GH` today, not `@AAA-###`, and crawl only the loft tree.
- Feature catalogue (Phase C2): `loft-lang/features` issues + the
  `make features-fetch/gen/check` shadow; the generator's first-` ```loft `-fence
  RUN test is the compiles-and-runs gate the real-usage tag complements.
