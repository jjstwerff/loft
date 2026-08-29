<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# USER_DOCS.md — the documentation a distribution owes its users

> loft stopped being one interpreter with a tutorial and became a **distribution**: a
> language, a stdlib, 42 published libraries, a registry, four build targets and a
> compatibility contract being driven toward absolute. The documentation still describes the
> first thing. This is the design for the second.
>
> It covers three surfaces that fail for the same reason — **the text exists but no reader
> path reaches it**: the library documentation, the front-door README, and the code examples
> on the website, which can already be driven by a REPL and a debugger that the pages do not
> expose.
>
> Written as a design, not a plan: one invariant, a count of what has to re-state it, the
> failure paths written down before any code, and a check pinned to each claim
> ([design-protocol](../../.claude/skills/design-protocol/SKILL.md)). The build order is
> last, deliberately — the order is a consequence of the design, not the design.

---

## Two constraints that shape everything below

**1. The machinery is mostly built. This is a wiring problem, not a construction problem.**
Every measurement in *What already exists* below was taken by running the command, not by
reading a doc. `loft doc graphics` renders nineteen API sections and a guide into HTML
today. `loft api --registry` lists all 42 packages with descriptions today. `src/wasm_debug.rs`
implements breakpoints and expression evaluation in the browser today, end-to-end tested.
None of it is reachable from a page a user lands on. A design that proposes building any of
it again is wrong before it starts.

**2. A reader arrives with one of three questions, and they are not interchangeable.**
*What is there?* — *How do I start?* — *What is the exact signature?* Today loft answers the
third one well, the second one twice out of forty-two, and the first one not at all on the
web. Every tier below exists because a reader in one of those states is served badly by the
answer to a different one. An API list is not an introduction, and an introduction is not a
reference.

---

## The invariant

> **A library's user-facing text has exactly one home — the library itself — and every
> reader path renders that source rather than restating it.**

This is the one-home rule the repo already applies to code (`QUALITY.md`), turned on prose.
It is worth stating as an invariant rather than a preference because the alternative has
already failed here, measurably, and the failure is the kind nobody notices: two copies that
agree when written and drift in silence.

### Counting the re-assertion sites

Nine places currently carry, or could carry, a description of what a library is and how to
use it:

| # | Site | Hand-written? | Reaches a user? |
|---|---|---|---|
| 1 | the library's `README.md` | **yes** | only via GitHub |
| 2 | the library's `docs/*.loft` guide | **yes** | only via `loft doc` |
| 3 | `tests/docs/NN-<lib>.loft` in the loft repo | **yes** | yes — the doc site |
| 4 | the registry `index.json` description | **yes** | via `loft api --registry` |
| 5 | `LIBRARIES.md` | generated | no — local build, git-ignored |
| 6 | `loft api <name>` | generated | CLI only |
| 7 | `loft api --registry` | generated | CLI only |
| 8 | `loft doc <name>` | generated | local HTML only |
| 9 | a library page on the doc site | — | **does not exist** |

Four hand-written homes for the same facts. **The drift is not hypothetical: `random`'s guide
exists at sites 2 and 3 and the two files no longer match** — `docs/21-random.loft` in
`loft-libs-core` hashes `fb3cf619…`, `tests/docs/21-random.loft` in this repo hashes
`3f80cc26…`. Nobody edited both. Nothing reported it. Both are published, to different
readers.

The design collapses the four to **two**, each with a job the other cannot do:

- **`docs/*.loft` in the library** — the guide. Executed prose; the single source for every
  rendered page.
- **the registry `index.json` description** — the one-line hook. Already the registry's job,
  already the thing `loft api --registry` prints, already reviewed at publish time.

Sites 1 and 3 stop being sources. The README is *generated* from the guide plus the manifest
(§ *The README*, below, applies the same rule to loft's own front door); the loft repo's
`tests/docs/NN-<lib>.loft` files move out to the libraries they document.

---

## What already exists — measured

Run, not read. Each line is a command whose output was checked.

| Capability | State | Evidence |
|---|---|---|
| API extraction from `pub` + doc comments | **complete** | `loft api graphics` → 618 lines, signatures *with* their doc comments |
| Registry catalogue, CLI | **complete** | `loft api --registry` → 42 packages, each with a description |
| Per-package HTML generation | **complete** | `loft doc graphics` → *"1 guide(s), 19 API section(s) → ~/.loft/doc/graphics-0.8.0"* |
| Guide format (executed prose) | **complete** | `@NAME`/`@TITLE` topic files; the same format the 37 executed language pages use |
| Maintainer catalogue | **complete** | `make libcatalogue` → `LIBRARIES.md`, 42 libraries with full public API |
| Browser breakpoints + expression eval | **complete** | `src/wasm_debug.rs`; `tests/wasm_debug_relay.rs` drives `bp` / `run` / `eval` / `resume` headless |
| Doc-site generation | **complete** | `gendoc` → 74 pages, nav, sitemap, search index, PDF |
| Shell-transcript execution in docs | **complete** | `tests/doc_commands.rs` — an indented `$ ` line is run, its shown output checked |
| Library revalidation against loft | **complete** | `scripts/revalidate_libs_local.sh` → 42 pass, 0 compile-break |

The generator, the extractor, the executor, the catalogue and the debugger all exist and all
work. What does not exist is any path from a reader to their output.

### And what a reader actually gets

| | |
|---|---|
| Published libraries | **42** |
| …with a user doc page on the site | **3** — `imaging` (14-image), `random` (21-random), `time` (32-time) |
| …with a hand-written guide in their own repo | **2** — `graphics`, `random` |
| …with any `doc/` directory | **0** |
| …with any examples | **3** — `game_protocol` (17), `drawing` (1), `graphics` (1) |
| …with no `README.md` at all | **3** — `cbor`, `pluginabi`, `hex_grid` |
| …whose README is nine lines of boilerplate | **2** — `html`, `markdown` |
| Library pages on the doc site | **0** |
| Links from the doc site to any library repo | **0** |
| Libraries named anywhere on the doc site | **2** (`imaging`, `random`, in an install line) |
| `@F` catalogue entries covering a library | **0 of 117** — the catalogue is core-only by construction |

`html` and `markdown` are the two with no real README, and both are load-bearing for the
better-PHP direction [WEB_STACK.md](WEB_STACK.md) designs. `stage` and `graphics` carry 136
public items each, documented by one README apiece.

### The plan of record is stale

[DOC.md § Two tiers of library documentation](DOC.md) already describes a design for this,
and it no longer matches the tree:

- It says bundled libraries live in `lib/` and are `imaging` and `random`. `lib/` holds
  `audience_crystal`, `engine_host` and `git`; `imaging` and `random` are registry packages.
- It specifies a **Package Catalog** page generated from `registry.txt`, with an optional
  `docs` column. `registry.txt` does not exist — the registry is `index.json` in
  `loft-lang/registry` (@PLN112), and no catalog page was ever generated.
- Its four discoverability paths ("index cards, search, nav bar, catalog") describe a site
  that has none of them for libraries.

That section is superseded by this document. It should be cut down to what it accurately
describes — how `gendoc` renders a topic — and point here for the rest.

---

## What is missing

Three things, and only the third needs new mechanism:

1. **A reader path to output that already exists.** `loft doc` writes HTML nobody publishes;
   `loft api --registry` prints a catalogue no page shows.
2. **Guides.** 2 of 42 libraries have one. The format, the executor and the renderer are all
   in place; the prose is not written.
3. **One home enforced.** Nothing today would catch the `random` drift, or a README that
   stopped matching its guide.

---

## The design

Three tiers, each answering exactly one of the reader's three questions, each generated
from the one home.

### Tier 0 — Discovery: *what is there?*

**One `doc/libraries.html`, generated from the registry index**, listed in the nav on every
page and in the sitemap. One row per package: name, version, the one-line description the
registry already carries, its category, and links to its guide, its API reference and its
repository.

Generated from `index.json` alone, so it needs no library to be installed, cannot lag the
registry, and costs the doc build nothing. It is the web twin of `loft api --registry`, which
already prints exactly this — the same source, a second renderer.

This is the single highest-value item in the document and it depends on nothing else.

### Tier 1 — The guide: *how do I start?*

**One executed guide per library, at `docs/01-getting-started.loft` in the library repo**, in
the `@NAME` / `@TITLE` topic format the 37 executed language pages already use. It is a `.loft` file,
so it compiles and runs; its prose is comments and its examples are the program.

The contract for a guide, in order — the shape the good language pages already have:

1. **What it is**, in one paragraph, and what it is *not* — the neighbouring library a reader
   might have wanted instead.
2. **The smallest complete program that does something.** Runnable, with its real output.
3. **The three or four calls that carry the library**, each with a worked example. Not the
   whole surface — that is Tier 2's job.
4. **The one thing that surprises people.** Every library has one; `graphics`'s is that
   `rgb()` and a hand-written hex literal differ in the alpha byte.
5. **Where to go next** — the API reference, and the library's own tests as examples.

Executed by the **library's** CI, because the library owns it. This is what removes the
hardcoded delegation list: `tests/wrap.rs:52` and `tests/doc_lib_examples.rs:63` name
`14-image.loft` and `21-random.loft` as string literals, so a new library page is silently
uncovered until someone edits both. A guide that lives in its library is run by that library's
own suite, by the rule that it is a `.loft` file in the package — no list to forget.

### Tier 2 — The API reference: *what is the exact signature?*

**Already built.** `loft doc <name>` renders it; the only change is publishing the output for
each library alongside its guide page, and linking both from Tier 0.

Generated from `pub` declarations and their doc comments, so it cannot drift from the code by
construction — which is why it is the tier that already works, and the argument for pushing
the other two toward generation as well.

### The one-home rule, and what it retires

| Today | After |
|---|---|
| `tests/docs/14-image.loft` (loft repo) | `imaging/docs/01-getting-started.loft` |
| `tests/docs/32-time.loft` (loft repo) | `time/docs/01-getting-started.loft` |
| `tests/docs/21-random.loft` **and** `random/docs/21-random.loft`, drifted | `random/docs/01-getting-started.loft`, one file |
| 42 hand-written `README.md` | generated from the guide + manifest, with a drift guard |
| `tests/wrap.rs` SUITE_SKIP, `doc_lib_examples.rs` filename list | nothing — the library runs its own guide |

The doc site renders a library's guide from the **registry cache** — the same source
`loft api graphics` reads without a clone — so the loft repo never holds a copy of a library's
prose.

---

## The pages a reader can drive: REPL and debugger in the browser

### What is built

`src/wasm_debug.rs` implements the browser side of the debugger against the running wasm
program: `bp <fn>` sets a breakpoint, `run` / `resume` / `step` move, and `eval <expr>`
evaluates an arbitrary expression over the paused frame. `eval` is not a value peek — it binds
the live locals as arguments, compiles the expression as a synthetic function and runs it, so
`eval len(v)`, `eval h["a"].v` and `eval fib(10)` are all in range, the last one because the
program's own definitions are in scope. `tests/wasm_debug_relay.rs` drives the whole chain
headless — an agent, through a server, into a browser wasm client — and asserts the replies
for `bp`, `run`, `eval` and `resume`.

### What the pages expose

`doc/playground.html` has **one button: ▶ Run.** `doc/examples.js` carries 99 examples — every
`tests/docs/*.loft` file, folded in by `scripts/build-playground-examples.loft` — and every one
of them is run-only. The 37 executed language topic pages show their code as static text.

So the capability that most distinguishes loft from a language with a syntax-highlighted
snippet on a page is built, tested, and invisible.

### The design

**Every code block on a doc page becomes drivable, in three steps that share one runtime.**

1. **Run** — what exists. The block executes, output appears beneath it.
2. **REPL** — a prompt beneath the output. The reader types an expression; it is evaluated
   against the program that just ran, through the same `eval` path the debugger uses. Calling
   the page's own functions is the point: on the vector page a reader types `sum(v)`, on the
   text page `caesar("hello", 3)`.
3. **Debug** — a gutter click sets a breakpoint (`bp`), execution pauses, the frame's locals
   are listed, and the REPL prompt is now evaluating *over the paused frame*. Step and resume
   from the same panel.

One honest constraint, and it shapes the surface: **`eval` evaluates over a frame**. A REPL
against a program that has finished has no frame to bind. So the page's Run auto-pauses at
the end of `main` before the frame unwinds — the reader's prompt is a paused frame that
happens to be the last one — and the panel says so, rather than presenting a stateless
calculator that mysteriously cannot see `v`.

### The example that makes it concrete

One new page, `tests/docs/38-call-it-yourself.loft`, whose entire purpose is to be driven
rather than read. It defines a handful of small, pure, zero-dependency functions — a
`fib(n)`, a `primes_below(n)`, a `caesar(s, k)`, a `stats(v)` returning a struct — and its
prose is the invitation: *these four are live below; call them.* The REPL panel lists the
callable names off the program's own definitions, so the reader does not have to scroll up to
find out what to type.

It is a page in `tests/docs/`, so it is executed on both backends like every other, and folded
into `doc/examples.js` like every other. The interactive panel is the page's renderer, not a
one-off.

Why a purpose-built page and not just the panel: a capability nobody is *told* to use is
indistinguishable from one that is missing. The page is the invitation, and it is also the
thing that fails loudly if the REPL wiring breaks — every other page would merely look
slightly less interactive.

---

## The README

### What is wrong with the current one

Nothing is inaccurate — I checked every claim: `doc/learn-loft.md` exists, `examples/` holds
exactly seven files, Brick Buster is 1,849 lines, `editors/vscode` is there, ten skills ship.
One number is stale: *"~73k lines of documentation"* is now **115,560**.

The problem is the **positioning**. The title is *"build small games, share a link, anyone
plays"* and the first 40% is one arcade game. That was the right README for a project proving
it could produce something; it is the wrong one for a distribution with 42 libraries, four
targets, a registry, a compatibility contract and eight applications built on it. A reader
evaluating loft as a language they might depend on has to get past a game to find out that
`loft install` exists, and the libraries appear as a five-row table two-thirds of the way
down, described as "batteries".

Nothing that follows argues for making it drier. Brick Buster is the best evidence in the
repository that the thing works, and it stays. It stops being the *thesis* and becomes the
*proof*.

### What it must say instead, in order

1. **What loft is**, in one sentence a reader can repeat: a statically typed language that
   reads like Python, compiles to a native binary or to the browser, and ships as a
   distribution with a registry.
2. **Evidence it works** — the playground, Brick Buster, the gallery. Three links, compact.
3. **The code**, unchanged; it is good and it is short.
4. **Install and first program.**
5. **The distribution.** The 42 libraries by category, with `loft install <name>` and
   `loft api --registry`. This is the section that does not exist today and is the single
   biggest gap between what loft is and what its front page says it is.
6. **Maturity and compatibility.** Calendar versions, the contract-1 goal, what is stable
   today and what is not — the honest paragraph a reader needs before depending on it.
7. **Documentation** — the site, the reference PDF, `loft doc`.
8. **How loft is built**, the bus-factor section. Distinctive, true, and it belongs late,
   because it answers a question a reader only has after deciding the thing is interesting.
9. **Contributing**, **License**.

### The one-home rule, applied here too

The library table in the README is the same facts the registry index carries, and a
hand-maintained table of 42 rows is a drift generator. It is generated from `index.json`
between markers, refreshed by the same script that builds `LIBRARIES.md`, with a CI check that
fails if the committed block does not match. Same rule as § *The invariant*: one home, every
reader path renders it.

---

## Failure paths

Written before the code, because each one is silent by default.

| Failure | Why it is silent | The design's answer |
|---|---|---|
| A library is not installed when the site builds | Its page simply does not appear; the nav is one shorter | The page is generated from the registry index regardless, with the API and guide sections marked *not built*. The build **reports the count** and the count is expected to be zero. |
| A guide stops compiling | Nothing on the site runs a registry library's guide today | `revalidate_libs_local.sh` already extracts and runs every published library's `tests`. It runs `docs/*.loft` too. |
| The site renders 0.8.0 while the registry serves 0.9.0 | Both look correct in isolation | Every generated page carries the version it was built from; `registry-index-snapshot.json` is the oracle and the drift is a check, not a reading. |
| A generated README is hand-edited | It looks better and is now a second home | The same drift-guard shape `tests/doc_hygiene.rs` already uses for `doc/examples.js`. |
| A library has no guide | Its page 404s, or worse, silently vanishes from the nav | The page always exists, shows the API, and says *no guide yet*. The count of guide-less libraries is the metric the monthly review reads. |
| The REPL panel breaks | Every page still renders; the code blocks just stop being interactive | `38-call-it-yourself.loft` exists to fail. Its whole content is the interaction. |
| `eval` returns `<unavailable>` for a `text` local | The reader reads it as "loft cannot do this" | The panel distinguishes *not supported here* from *no value*, and the page says which expressions are in range. This is a known limit of `eval_expr`, documented rather than hidden. |

---

## Verification — on rails that already exist

No new harness. Each claim below is pinned to an instrument the repo already runs.

| Claim | Check |
|---|---|
| Every library page renders | `gendoc` reports libraries rendered / total; a mismatch fails the build |
| Every guide compiles and runs, both backends | `revalidate_libs_local.sh` (already 42/42) extended to `docs/*.loft` |
| Every shell line a guide shows is real | `tests/doc_commands.rs` — the indented `$ ` rule, unchanged |
| Generated README matches its source | drift guard, the `doc/examples.js` shape in `tests/doc_hygiene.rs` |
| The library table in this repo's README matches the registry | same guard, same script that writes `LIBRARIES.md` |
| The REPL panel actually evaluates | `tests/wasm_debug_relay.rs` already proves the protocol; the page adds a headless driver asserting one `eval` round-trip |
| A guard would fail if the thing broke | `make falsify` on each new guard, recorded as `@falsified-at:` |
| Guide quality does not rot | `make libraries-review` and the watermark table in [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md), which already exist and currently have almost nothing to review |

The API-surface lint (`scripts/api_lint.py --check <lib>`) and the per-library documentation
sections of [LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) are already the release gate for a
library. The guide becomes a checklist row there rather than a new process.

---

## Build order

A consequence of the design, not the design. Ordered by value-per-unit-work; each step is
independently shippable and none blocks the next except where marked.

1. **Baseline the missing text.** READMEs for `cbor`, `pluginabi`, `hex_grid`; real content
   for `html` and `markdown`. Five files, no mechanism, and it closes the worst holes.
2. **`doc/libraries.html` from the registry index**, in the nav and the sitemap. Needs no
   library change and turns 0 discoverable libraries into 42.
3. **The new README.** Independent of everything above; can go first if the front door
   matters more this week than the catalogue.
4. **Publish `loft doc` output** for each library, linked from Tier 0. The generator exists;
   this is the site build calling it.
5. **The guide contract** — the five-part shape into `LIBRARY_AUTHORING.md` and
   `LIBRARY_CHECKLIST.md`, then guides for the six that carry the most weight: `graphics`,
   `stage`, `server`, `web`, `html`, `markdown`.
6. **The REPL and debug panel**, plus `38-call-it-yourself.loft`. Independent of 1–5; the
   only step whose value is not library documentation at all.
7. **Move the three guides home** — `14-image`, `32-time`, `21-random` — and delete the
   hardcoded delegation lists. Last, because it is the step that is pure cleanup, and it is
   only safe once the site renders guides from the library side.
8. **Retire [DOC.md § Two tiers](DOC.md)**, replacing it with a pointer here.

Steps 1–3 are each under a day and together change what a new user experiences more than
4–8 combined.

---

## Open questions

- **Does the doc site build get to install 42 libraries?** Tier 0 needs nothing installed;
  Tier 2 needs each package's source. The registry cache is the obvious answer and the CI
  cost has not been measured. Until it is, Tier 2 pages may have to be built on the box that
  publishes, not in CI.
- **Should the `@F` catalogue grow a library tier?** It is core-only by construction and that
  is defensible — a catalogue of the *language* is a different artefact from a catalogue of the
  *distribution*, and Tier 0 is the second one. The alternative is one catalogue with a `kind`
  of `library`, which would put 42 more entries in a 117-entry set and change what `@F` means.
  Recommendation: keep them separate, and cross-link.
- **Who owns a guide for a library the loft org does not maintain?** Today all 42 are
  first-party, so the question is theoretical — but the answer decides whether Tier 1 is a
  publish requirement or a request.
- **Does the browser REPL need history and multi-line input?** Deferred deliberately: the
  design's claim is that one-expression evaluation against a live frame is the valuable
  90%, and that claim should be tested on `38-call-it-yourself.loft` before any editor work.

---

## See also

- [DOC.md](DOC.md) — how `gendoc` renders a topic (§ *Two tiers* is superseded by this document)
- [DOC_QUALITY.md](DOC_QUALITY.md) — how the prose itself should read
- [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md) — the monthly by-hand pass and its watermarks
- [LIBRARY_AUTHORING.md](LIBRARY_AUTHORING.md) / [LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) — where the guide contract lands
- [WEB_STACK.md](WEB_STACK.md) — the design whose two libraries (`html`, `markdown`) are the worst-documented in the distribution
- [BUS_FACTOR.md](BUS_FACTOR.md) — why documentation outranks code here
