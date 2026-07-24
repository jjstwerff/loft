<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# First-contact work — the outside view, made concrete

**What this is.** A design for the presentation / first-run work proposed by an
external review (2026-07-24). The review carried the *visitor's* perspective —
what a first-time reader sees in five minutes — which is the one thing an agent
deep in language internals cannot reconstruct for itself. This doc keeps that
perspective and turns each item into a **decision**, not a restated task.

**Not a plan directory.** Per [plans/README.md](plans/README.md), most items here
are bug-fix / operational / light-TODO shaped and belong in direct commits. Only
T2 (gendoc SEO) is plausibly plan-shaped, and a plan's identity is a tracker issue
number — the owner's to mint. Promote later if it earns it.

**Tone constraint that governs every item:** understated-honest. State facts once,
plainly. No superlatives, no credential adjectives. The performance page showing
the interpreter losing to CPython stays exactly as it is. Restraint is the signal.

---

## Inspection — what the review got right, and four corrections

Verified against the working tree, 2026-07-24. The review's items were checked
against source rather than the release binary, which moved three of them.

| Item | Verdict |
|---|---|
| T0.1 release-binary warning | **Confirmed** (source), and the proposed fix fails its own acceptance test — see C1 |
| T0.2 line offset + missing snippet | **Confirmed, and re-localized** — see C2 |
| T0.3 `int` has no did-you-mean | **Confirmed** — `error: Undefined type int`, no suggestion, snippet renders fine |
| T1.1 badge / dead link | **Confirmed** — badge hardcodes `2026.7.1`; README:136 links a non-existent dir |
| T1.3 duplicate Time page | **Confirmed, worse** — see C3 |
| T2.1 no meta tags | **Confirmed** — `doc/index.html` has zero description / OG / Twitter tags |
| T4.1 root clutter | **Confirmed for tracked files, but mis-framed** — see C4 |

### C1 — the once-per-user marker fails the acceptance test it is given

T0.1 proposes a state marker (`~/.loft/native-notice-shown`) and then asks for
*"fresh container, unzip release, run twice → warning at most once"*. A fresh
container has no `~/.loft`, so a marker fires **on every first run in every
container** — exactly the CI/demo/Docker path where the nag is most visible. The
marker also adds writable-state on a path that must work when `$HOME` is unset or
read-only.

**Decision: gate on intent, not on state.** Say nothing on the default path;
explain only when the user asked for native (`--native`, or a native-only
operation). Rationale in one line: someone who downloaded a release binary chose
*not* to install a toolchain, so "install Rust and rebuild" is not an action they
want — it is not a warning, it is a non-sequitur. No new state, no new file, and
it passes the container case the marker fails.

Keep the full explanation in `--help` and in the release `QUICKSTART.md`.
Record as a `DESIGN_DECISIONS.md` entry (the review asks for this either way).

### C2 — T0.2 is the bare-script wrapper, not the `default/` prelude

The review guessed prelude concatenation. It is not: an explicit `fn main()`
reports correctly, and only *bare* scripts are shifted.

| source | reported | snippet |
|---|---|---|
| explicit `fn main()`, error on line 2 | `2:3` ✓ | yes |
| bare script, error on line 1 | `2:1` ✗ | **no** |
| bare script, error on line 2 | `4:1` ✗ | **no** |

Bare-script lines map to roughly **2×** the true line, so the fault is in the
synthetic-`main` wrapper's span mapping, not the prelude. That also **confirms the
review's "two symptoms, one cause" hunch**: the computed line does not exist in the
file, so snippet lookup finds nothing and renders silently. Fix the mapping and the
snippet returns for free — do not chase the missing snippet separately.

The release binary reported `:5:2` where source reports `:4:1`; the offset is not a
fixed constant, which is further evidence it scales with the wrapper rather than a
fixed prelude length.

### C3 — there are three Time pages, not two

`doc/22-time.html`, `doc/32-time.html`, **and** `doc/stdlib-time.html`. Decide one
canonical page before merging, or the nav gains a third entry the next time
`gendoc` runs.

### C4 — root clutter: the two biggest files are invisible, and two stray dirs are not

The review measured a local checkout. A visitor sees the **GitHub file list**, which
shows *tracked* files only, sorted **alphabetically with directories first** — file
size is irrelevant to what they see.

- `result.txt` (311 KB) and `isolated_stair.glb` (302 KB) are **untracked**. They are
  the two largest things in the directory and no visitor ever sees them. Ignore.
- The tracked offenders are as the review says: `.lint_comments_baseline` (100 KB),
  `Makefile` (79 KB), `DRAWING.md` (53 KB).
- **Missed: `dz/` and `f8e2e/`** — two stray directories committed by accident in the
  *same* PR (#463, @PLN86 sandbox v1). `dz/dz.loft` is a divide-by-zero probe;
  `f8e2e/` is a sandbox e2e fixture. `grep` finds **no reference** to either from any
  `.rs`, `.toml`, `Makefile`, `.sh` or doc. They are safe to delete, and they sit
  near the top of the alphabetical listing with names a visitor cannot interpret.
- Root has **44 tracked entries** (27 dirs + 17 files). The problem is count and
  legibility, not bytes.

`docs` is a symlink to `doc/` — intentional (Pages), leave it.

---

## Design per item

Ordered by the review's tiers. Each entry states the decision; the acceptance test
is the definition of done.

### T0.1 — silence the native notice on the default path
Gate the notice on explicit native intent (C1). Delete the "rebuild from source"
sentence from the default path entirely; keep one plain sentence under `--native`
naming what a release build can and cannot do. Add a `DESIGN_DECISIONS.md` entry.
**Accept:** fresh container, unzip, run twice → **zero** notices; `--native` prints
one clear explanation and exits non-zero only if it cannot proceed.

### T0.2 — fix the bare-script span mapping
One fix at the wrapper (C2). **Accept:** the 2-line `printt` repro reports `:2:`
*and* renders the `|` snippet; a bare 1-line script reports `:1:`. Regression test
covering both, on `--interpret` **and** `--native`.

### T0.3 — did-you-mean for type names
Reuse the existing function-name machinery: edit distance **plus** a small alias
table for cross-language habits (`int→integer`, `str`/`string→text`,
`bool→boolean`, `i64`/`i32`/`u64`→ nearest width type, `f64`/`f32→float`).
Suggestion only — do **not** make them legal (explicit non-goal; a language change
would need its own decision). **Accept:** `int` → *did you mean 'integer'?*; every
alias-table row covered by a test.

### T1.1 — README truthfulness
Dynamic version badge (`shields.io/github/v/release/loft-lang/loft`); repoint the
dead "full example list" link at the gallery page and at
`scripts/build-gallery-examples.loft` as the true source. Add a **`make linkcheck`**
target. Keep it out of `make ci` initially — an external-link checker makes CI
depend on the network and on other people's uptime, which buys flakiness for a class
of rot that moves slowly. Run it in the nightly job instead, where a red run is
information rather than a blocked merge.

### T1.2 — chunk-repo contradiction **[needs Jurjen]**
Determine whether `graphics`/`imaging` actually work today; update whichever README
is stale and fix the `jjstwerff/loft` → `loft-lang/loft` link. Prepare the patch
here if the push cannot be made from this repo.

### T1.3 — doc index
Pick one canonical Time page of the three (C3); move `15-lexer` / `16-parser` out of
the beginner tour into a "Libraries in depth" group. Both are `gendoc` ordering.

### T2.1–T2.2 — meta tags, sitemap, robots
`gendoc` emits per-page `description`, `og:title`/`description`/`type`/`url`/`image`,
`twitter:card`, plus `sitemap.xml` and `robots.txt`. Title format
`Structs — Loft, a language for small browser games` (page first, pitch second —
the pitch is what shows in a search result). Default `og:image` is the hero PNG at an
**absolute** URL; relative OG images do not resolve for crawlers.
**Accept:** a sample page's `<head>` contains every tag; `sitemap.xml` lists all
public pages; `make linkcheck` passes.

### T2.3 — off-repo **[needs Jurjen]**
Search Console + Bing submission; GitHub **social preview image** (this is what
renders on every HN/Discord/Slack share — highest reach per minute of the whole
document); repo topics.

### T2.4 — gallery thumbnails
Static PNG per demo via the existing headless tooling in `tools/`, used as each
tile's poster so the gallery is not grey boxes before WebGL warms. Bonus: per-demo
`og:image`.

### T3.1 / T3.2 / T3.3 / T3.4 — story **[T3.1 + T3.4 need Jurjen]**
T3.1 provenance paragraph: one statement, no employer, no domain, no adjectives —
wording approval only. T3.2 is free and independent: move the bus-factor section
above "Packages & libraries" and add the one-line invitation under the hero
(*"The whole game is one file — read all 1,983 lines"*). T3.3 animated hero: script
it if the headless tooling can capture frames, else flag. T3.4 "Why Loft is the way
it is": draft from `DESIGN.md` + `DESIGN_DECISIONS.md`; entries tracing to previous
production DSLs stay generic ("in a previous production language I maintained…").

### T3.5 — a game-shaped example
Add one ~40-line graphical example plus an `examples/README.md` routing
game-seekers to the gallery. The README is the cheaper half and lands first.

### T4.1 — root surface
Delete `dz/` and `f8e2e/` (C4 — unreferenced strays). Move `DRAWING.md`,
`.lint_comments_baseline`, `.feature_coverage_baseline` under `doc/claude/` or
`.claude/`, updating whatever reads those paths. Leave `Makefile` and the `docs`
symlink.

### T4.2 — Discussions **[needs Jurjen]**
Enable before any launch post; link from README + `SUPPORT.md`.

---

## Sequencing — five batches, not twenty PRs

`CLAUDE.md` says bundle subjects into one CI cycle; a PR per item would burn ~30
minutes of CI each for changes that are individually minutes of work. Batch by
*risk class*, so a red gate points at one kind of thing:

| Batch | Contents | Risk |
|---|---|---|
| **B1 — first-run bugs** | T0.1, T0.2, T0.3 | Code + tests; the only batch touching the parser |
| **B2 — truthfulness** | T1.1, T1.3, T4.1 | Docs, `gendoc` ordering, file moves/deletes |
| **B3 — discoverability** | T2.1, T2.2, T2.4 | `gendoc` output; verify a sample `<head>` |
| **B4 — story** | T3.2, T3.5, plus drafts of T3.1/T3.4 for approval | README + new page, no code |
| **B5 — owner** | T1.2, T2.3, T3.1 wording, T3.3, T4.2 | Outside the repo |

B1 first (two are plain bugs and all three are "first five minutes"). B2 next — a
README that lies undoes everything after it. Then B3. B4/B5 in any order.

**Cross-cutting gates.** Every generated-site change: `cargo run --bin gendoc` +
`make gallery`, then check a sample page's `<head>` and run `make linkcheck`. Every
diagnostic change: `--interpret` **and** `--native` covered by tests. Everything
through `make ci`. Remove items from this file as they land.

## Non-goals

No new language features, no new pillars. No superlatives, no credential
adjectives, no weakening of the honest benchmark presentation. Do **not** silently
make `int`/`str` legal — suggestion only, unless a design decision is taken.

## See also

- [PLANNING.md](PLANNING.md) — the backlog and house rules this follows.
- [plans/README.md](plans/README.md) — why most of this is *not* a plan directory.
- [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) — check before re-litigating; T0.1
  lands an entry here.
