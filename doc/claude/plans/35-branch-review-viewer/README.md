<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# BRANCH_REVIEW_VIEWER — a frozen loft binary for VM-based code review

**Status:** Active — promoted to current 2026-05-13.

A small loft application that runs in the user's development VM
and serves a branch-aware doc + code review dashboard to a
browser on the host (via SSH port-forward).  Built IN loft to
dogfood the server lib + plan-22 closures + 0.8.3 coroutines.
Compiled to a **standalone binary that ships with the repo and
runs decoupled from loft tree state** — so the user can review
loft-in-flight even when loft itself is mid-refactor or
mid-broken-build.

## Drivers

Two distinct audiences, both pointing at the same tool:

**Driver 1 — the user's own review workflow** (origin of this
plan).  Scrolling through chat-pasted code snippets is too
narrow for substantive review of in-flight work.

**Driver 2 — friend-onboarding (added 2026-05-13)** —
loft has crossed the threshold where developer friends with
some Rust / ML experience can be invited to try it.  But the
project is now ~250 markdown files in `doc/claude/` plus a
deeply-linked plan tree.  A friend who wants to grasp "what is
loft, what's done, what's planned, why does the codebase look
like this" cannot do that by `grep`-ing 20+ files manually.
The viewer becomes their **navigation surface**: open the
dashboard, click into the relevant docs, follow cross-doc
links naturally.  See [ROADMAP § Near-term focus —
friend-readiness](../../ROADMAP.md#near-term-focus--friend-readiness-added-2026-05-13).

Three concrete needs surfaced from the personal-review angle:

1. **Read full files in context** — a 192 KB `PROBLEMS.md` or
   3.4 KB `PLANNING.md` cannot be reviewed via chat snippets.
2. **See what's changing on the current branch at a glance** —
   files touched, recent commits, diffs vs `main`, broken-link
   sentries.  Today this requires `git log --stat` /
   `git diff` / `git show` per file in the terminal.
3. **Follow cross-doc references** — the plan tree is densely
   linked (`[link](plans/finished/22-mutable-closures/03-case-c.md#major-finding)`),
   and `make serve` (which runs `python -m http.server` on
   `doc/`) serves `.md` files raw — clicks 404 or download.

The viewer also doubles as a flagship test of three plans of
loft work that have shipped:

- **`lib/server`** (already shipped — `lib/server/src/server.loft`):
  HTTP routing, response builders, WebSocket support.  Viewer
  exercises `respond_html`, `respond_css`, `serve_dir`, plus
  the streaming-write story.
- **Plan-22 mutable closures** (shipped 2026-05-13, currently
  on `demo_dev`): route handlers expressed as closures
  capturing the viewer's shared state (template cache, file
  tree).  The "outer reads + writes seen by closure" semantics
  from the cell + auto-Reference machinery are exactly what a
  shared cache wants.
- **0.8.3 coroutines**: streaming responses for big files
  (192 KB `PROBLEMS.md` doesn't need to materialise as one
  contiguous text buffer).

If a viewer feature surfaces a loft gap that's worth filling
in the language itself, that's a **driver-level finding** and
gets filed against `plans/35-…` rather than worked around in
the viewer's loft source.

## Architecture — frozen-binary contract

The viewer binary lives at a stable path under
`tools/viewer/` with its own `loft.toml` package manifest.
Build is **deliberate, not automatic** — `make view-build`
compiles the binary; subsequent `make view` invocations run
the existing artifact.  The artifact is committed to the repo
(or, in a later phase, attached to releases).

This isolation gives three properties:

1. **Stability**: a broken `src/parser/foo.rs` change does not
   break the viewer.  The user can keep reviewing.
2. **Reproducibility**: the viewer was built against a known
   loft commit recorded in its `loft.toml`.
3. **Constrained surface**: the viewer can only use loft
   features that landed before its build commit.  This forces
   us to ship the viewer against a "release-quality" loft
   slice rather than chasing tip-of-tree.

When the user wants viewer updates, they `make view-build`
deliberately.  CI does not auto-rebuild.

## Ground rules

- **Loft-native source** at `tools/viewer/`.  No Python, no
  Node.js, no Rust outside what loft itself uses.  Every gap
  the viewer hits in loft becomes either a viewer-side
  workaround OR a loft-language enhancement (filed as a
  separate plan or P-issue with this plan as the driver).
- **Git access via wrapper script** — `tools/viewer/refresh.sh`
  dumps `git diff --name-status main...HEAD`, `git log
  --oneline -10`, `git status --short`, and per-file diffs
  to JSON / text files under `tools/viewer/state/`.  The
  viewer reads those files; refreshing state means re-running
  `make view`.  Loft has no subprocess primitive today and
  this plan does NOT block on adding one — see the
  [out-of-scope](#out-of-scope) section.
- **Markdown rendering minimal in v1, tables in v2** — the
  user's pain point with current tools is poor table rendering;
  proper tables are a planned phase, not v1.  See
  [phase 06](06-tables-design.md).
- **Single user, single VM, SSH-forwarded port** — no
  authentication, no multi-tenant concerns.  Default bind to
  `0.0.0.0` for SSH forwarding; `LOFT_VIEW_BIND=127.0.0.1`
  opt-out documented.
- **Zero JavaScript in v1** — plain HTML5 + CSS.  Sidebar
  collapsing via `<details>`, "diff toggle" via plain links.
- **Plain HTML + embedded CSS** — no template engine, no
  asset pipeline.  HTML is built via loft string concatenation;
  CSS embedded in the binary as a string constant.

## Phases

Each phase ships a usable viewer.  No phase requires the next
to be useful — if work pauses after phase 03, the user has a
working file browser with markdown rendering.

| # | Phase | Effort | What ships | Status |
|---|---|---|---|---|
| 0 | [Skeleton + binary build](00-skeleton.md) | XS | `tools/viewer/` package, "hello world" binary, `make view-build` + `make view` Makefile targets, frozen-binary contract documented | Open |
| 1 | [HTTP server + static + tree](01-http-routes.md) | S | Binary serves the project tree over HTTP; clicking a file shows raw bytes; sidebar lists `doc/`, `lib/`, `src/`, `tests/` | Open |
| 2 | [Code-file rendering with `<pre>` + line numbers](02-code-files.md) | XS | `.rs` / `.loft` / `.toml` / `.py` / `.sh` / `.json` files render with HTML escaping + per-line `<a>`-anchored line numbers | Open |
| 3 | [Minimal markdown subset](03-markdown-minimal.md) | M | Headings, bold/italic, inline code, fenced code blocks, lists, links, paragraphs.  Cross-doc `.md` link rewriting + GitHub-compatible heading slugs.  ~250-line loft module under `tools/viewer/src/markdown.loft` | Open |
| 4 | [Git state via wrapper script](04-git-state-wrapper.md) | S | `tools/viewer/refresh.sh` dumps git state to `tools/viewer/state/*.json`; viewer dashboard reads JSON and renders branch header + changed-files list + recent-commits list + uncommitted-files list | Open |
| 5 | [Diff + commit views](05-diff-and-commit.md) | M | `/diff/<path>` renders unified diff against `main`; `/commit/<sha>` renders commit message + per-file diffs.  Top-right `[Rendered ¦ Diff]` toggle on every file page.  Refresh script extended to dump per-file diffs and per-recent-commit diffs | Open |
| 6 | [Proper tables (forward-looking)](06-tables-design.md) | M | Full GFM table support — alignment, multi-line cells, escaped pipes, nested formatting inside cells.  This is the phase that distinguishes the viewer from existing tools (the user's pain point) | Open |
| 7 | [Closeout — release binary, docs, retrofit](07-closeout.md) | S | First "release" of the viewer binary tagged in the repo; DEBUG.md § Branch review viewer; CLAUDE.md § Key commands updated; CHANGELOG entry; viewer used to review the next plan-07 phase as proof of concept | Open |

**Phase boundaries are commit boundaries.**  Each phase lands as
its own commit on a focused branch (or directly on the
viewer's working branch).

## Acceptance — full plan

- `make view-build` produces `tools/viewer/bin/loft-view`
  (loft-compiled binary, ~stable size).
- `make view` runs the binary; user opens
  `http://localhost:8765/` via SSH port-forward and reviews the
  current branch end-to-end without dropping into the terminal.
- Worst-case file (`doc/claude/PROBLEMS.md`, 192 KB) renders
  in <2 sec on the user's hardware.
- All cross-doc relative links resolve to served URLs;
  GitHub-compatible heading slugs for anchor fragments.
- GFM-style tables render with column alignment in phase 6.
- Frozen-binary contract holds: a deliberately-broken `main`
  build does NOT prevent `make view` from working off the
  pre-built binary.
- DEBUG.md has the user-facing usage section (port-forward
  example, env var reference).
- CLAUDE.md § Key commands lists `make view` next to
  `make serve`.

## Risks

| Risk | Mitigation |
|---|---|
| Loft features the viewer needs surface gaps | Each gap becomes a P-issue or its own plan with this plan as the driver.  Viewer ships a workaround in the meantime (e.g. simpler markdown until a feature lands). |
| Frozen binary diverges from current loft, can't be rebuilt | Loft commits to backwards-compatible `lib/server` API surface.  The viewer's `loft.toml` pins a specific loft commit; rebuild against newer loft is a deliberate exercise. |
| Wrapper-script approach feels janky vs. proper subprocess | Acceptable trade-off for shipping in weeks not months.  If subprocess support lands in loft (separate plan), refresh-script approach can be retired. |
| Tables phase (06) blocks closeout | Phase 06 is its own milestone; phases 00-05 close as a usable v1 even if 06 is still in flight.  Plan can split into `35a` (v1 close) + `35b` (tables) if 06 takes too long. |
| User wants live reload sooner than expected | Coroutines + WebSocket already in `lib/server`; live-reload becomes phase 08 if needed. |

## Stretches (post-v1, listed for traceability)

### Newcomer mode — curated onboarding landing page

The 250-file `doc/claude/` tree is overwhelming for someone
arriving at loft for the first time.  A `?mode=onboarding`
query param (or a `/welcome` route) flips the dashboard into a
curated newcomer view that hides the dev-internal docs and
exposes a small, guided path:

1. **What is loft** — short paragraph + link to root README.
2. **Try it in 5 minutes** — links to `examples/hello.loft`,
   `examples/structs.loft`, `examples/match.loft` rendered with
   a "click to copy + run instructions" pattern.
3. **Learn the language** — link to `doc/learn-loft.md`.
4. **The standard library** — link to `STDLIB.md`.
5. **One real program** — link to a flagship example like
   `lib/graphics/examples/25-brick-buster.loft` or the
   audience-demo when it ships.
6. **Where to get help** — link to GitHub issues + the
   project's communication channel.

The default dashboard (branch state + changed files + commits)
remains the user's review tool; newcomer mode is a separate
landing page friends arrive at.  No filtering of the actual doc
tree — friends can navigate anywhere from there if curious;
this just gives them a starting path.

File as **phase 08** of this plan when v1 ships.

### Public-instance hosting

The viewer binary is the same shape regardless of "for me in
my VM" vs "for friends on the public web."  After phases 00-06
ship, a small follow-up could deploy the viewer on a tiny VPS
(or a free-tier serverless target) pointed at a public mirror
of the loft repo.  Friends visit `https://loft-lang.example/`
and land on the newcomer-mode page without any local setup.

Acceptance:
- Public URL serves the curated newcomer landing.
- Read-only (the dashboard's git-state shows the public mirror's
  current branch — typically `main`).
- No write paths exposed (already the case — viewer has no
  write routes).
- Bound to `127.0.0.1` behind a reverse proxy that adds TLS.

File as **phase 09** of this plan when phase 08 ships.

### Why not just GitHub Pages with rendered HTML?

A common alternative is to render `doc/claude/*.md` to HTML at
CI time and host on GitHub Pages.  Considered and rejected for
the friend-onboarding use case:

- **Lag** — Pages rebuilds on push to `main`; in-flight branch
  state never appears.  The user's review-this-week workflow is
  invisible.
- **Overwhelming default** — Pages would render every doc in
  the tree at the same prominence.  Friends don't need to see
  PROBLEMS.md first; they need a guided path.
- **No git awareness** — diff/commit views (the personal-
  review use case) aren't expressible in static HTML.
- **No filtering** — the newcomer-mode landing requires
  application logic that static rendering can't provide.

GitHub Pages **is** a viable secondary surface for "I want to
read the loft docs without installing anything" — but it
should not be confused with the viewer's role.  If a static
mirror is wanted, pre-rendering can be added as a separate
small task after phase 09 (effectively `loft-view --static
--out=site/`); not in plan-35's main scope.

## Out of scope (deferred / separate plans)

- **Subprocess support in loft** — file as a separate
  `lib_plans/future/<NN>-subprocess/` if the wrapper-script
  approach stops being enough.  Not blocking this plan.
- **Full GFM markdown parser as a `lib/markdown/` library** —
  the viewer ships its own minimal subset.  Promoting the
  parser to `lib/markdown/` is a separate plan with this one
  as a driver.
- **LSP integration / jump-to-definition** — VS Code
  remote-ssh covers this.  The viewer is for review, not
  editing.
- **PDF export per file** — browser print-to-PDF works.
- **Tracker-tag indexer (`@P259` / `@PLAN22-2-ii`)** — the
  broader proposal the user evaluated earlier; separate plan
  if/when promoted.
- **Cross-file full-text search** — browser Ctrl-F covers
  single files.  If cross-file becomes essential, add
  ripgrep-via-wrapper later.
- **Live reload via WebSocket** — phase 08 if F5 stops being
  enough.
- **Multi-branch / PR review** — single-branch (current vs
  `main`) is the user's actual workflow.

## Cross-references

- [`lib/server/src/server.loft`](../../../../lib/server/src/server.loft)
  — the HTTP / WebSocket primitives the viewer depends on.
- [`plans/finished/22-mutable-closures/`](../finished/22-mutable-closures/README.md)
  — closures are the natural shape for route handlers; viewer
  is the first non-test consumer.
- [`lib_plans/future/08-server/README.md`](../../lib_plans/future/08-server/README.md)
  — server-lib roadmap; viewer surfaces specific feature gaps.
- [`lib_plans/future/01-regex/README.md`](../../lib_plans/future/01-regex/README.md)
  — regex would simplify markdown parsing; not blocking.
- [`DEBUG.md`](../../DEBUG.md) — gets the user-facing usage
  section in phase 07.
- [`CLAUDE.md`](../../../../CLAUDE.md) — gets the
  `make view` entry under § Key commands in phase 07.
