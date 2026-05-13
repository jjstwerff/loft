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
- `future/` = **paused, not abandoned** — real commitment to
  finish, eventually.  The long-term direction is to drain
  `future/` to zero by shipping each plan through to
  `finished/`.  May take years; that is fine.
- `deferred/` = **won't do absent a concrete trigger** —
  different intent.  May never run.
- Same `≤3 active` discipline as `plans/`: hold the in-flight
  count to 2-3 library initiatives.  Promote one from `future/`
  when a current one closes.  Work flows in one direction:
  `future/` → current → `finished/`, never back to
  "indefinitely parked."

### Closing a plan — documentation must move out

Same rule as `plans/`.  Apply the 6-step procedure in
[`../plans/_LIFECYCLE.md`](../plans/_LIFECYCLE.md);
the rule's full specification is in
[`../plans/README.md § Closing a plan`](../plans/README.md#closing-a-plan--documentation-must-move-out).

For library plans specifically, the natural reference home
when a plan closes is **inside the library itself**:
- `lib/<name>/README.md` — top-level library reference.
- `lib/<name>/src/*.loft` — inline doc comments on
  user-facing types and functions.
- Per-feature deep-dive docs go alongside the code they
  describe, not in `doc/claude/`.

The `finished/<NN>-<slug>/README.md` keeps only the closure
record; reference content moves to `lib/<name>/`.  Other
docs link to the library, not to the closed plan.

### Authoring a new library plan

Copy [`../plans/_TEMPLATE.md`](../plans/_TEMPLATE.md) to
`<NN>-<slug>/README.md` (next free integer in the
`lib_plans/` counter — independent from `plans/`).  Same
canonical shape applies; library-specific concerns
(downstream consumer compatibility, `lib.toml` schema, etc.)
fold into the Sub-arcs / Open questions sections.

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

Library plans we intend to do — paused, not abandoned.  Each
future plan is a real commitment to finish; the long-term
direction is to drain `future/` to zero by completing each
plan into `finished/`.  Ready-to-resume — pre-flight already
done, design already drafted.  Promote one into "Current" when
a current library plan closes.

| Dir | Initiative | Pre-flight status |
|---|---|---|
| [`future/01-regex/`](future/01-regex/) | Standalone regex library — replaces the `r"..."` raw-literal plan and "regex arm in match" plan that were sketched earlier (both withdrawn).  Library lives in stdlib via the lazy-loading mechanism (LAZY_STDLIB.md), gives full regex semantics without bloating the language surface.  First lazy-loaded stdlib consumer.  Cooperates with MATCH_PEG.md (PEG-style sequence patterns) — REGEX handles all text matching; MATCH_PEG handles structural / numeric / Unicode-class patterns. | Design draft (was `doc/claude/REGEX.md`).  Not yet implemented; depends on LAZY_STDLIB.md infrastructure landing first.  Ship order: REGEX R1 (linear NFA, basic features) is the first scheduled item once LAZY_STDLIB lands; R2-R4 (named groups, Unicode properties, backtracking fallback) gated on demand. |
| [`future/02-graphics/`](future/02-graphics/) | Graphics library bundle — covers 2D RGBA drawing, 3D mesh representation, scene management, multi-backend rendering (desktop OpenGL, browser WebGL, GLB file export).  Four companion files: README.md (top-level library design, opcode integration, performance hooks); IMPLEMENTATION.md (ordered checklist — canvas → GLB → OpenGL → WebGL, GLB-first because it needs no GPU / window / SDK); RENDERER.md (high-level scene-driven PBR layer on top of the low-level `gl_*` API); GALLERY.md (web example gallery + unified rendering across native / WebGL / GLB from one API). | Design (was `doc/claude/{OPENGL,OPENGL_IMPL,RENDERER,WEB_EXAMPLES}.md`).  Low-level `gl_*` API already covers current use cases; the renderer layer is "designed, not scheduled".  Pre-flight: phase ordering and most architectural decisions locked in.  Implementation in 4 staged backends per IMPLEMENTATION.md. |
| [`future/03-lazy-stdlib/`](future/03-lazy-stdlib/) | Conditional stdlib loading — trigger-based module load, pay-for-what-you-use cold start.  Modules listed in `default/lazy/*.loft` load only when first triggered (regex on first `r"..."` use, http on first `http_get` call, etc.).  Cuts cold-start binary size and parse time for programs that don't use the heavyweight modules. | Design (was `doc/claude/LAZY_STDLIB.md`).  Not yet implemented.  **Critical-path infrastructure**: REGEX (lib_plans 01) is the first scheduled consumer once this lands; subsequent libraries (http client, image processing, etc.) inherit the same trigger pattern. |
| [`future/04-asset-pipeline/`](future/04-asset-pipeline/) | Game asset pipeline — three-phase workflow: (1) Claude builds prototype with procedural placeholder sprites / sounds via `fill_rect` / `build_atlas()` etc.; (2) artist creates real assets in external tools (Aseprite for pixel art, Audacity / sfxr for sound, etc.); (3) integration via `load_sprite_sheet()` and friends — code tries the PNG first, falls back to procedural placeholder if missing.  Lets a game look polished early without blocking on art. | Design (was `doc/claude/PIPELINE.md`).  Some loader entry points already exist; the documented workflow is end-to-end designed but not yet exercised across a complete real game.  Brick Buster + moros editor are the natural first consumers. |
| [`future/05-game-infra/`](future/05-game-infra/) | Game infrastructure grab-bag — designs for items that don't yet have a dedicated design doc.  Sub-arcs: G1-G7 (sprite-sheet loading, drawing, tilemap rendering, 2D collision, sound effects, background music with crossfade, first playable demo game); GL6.6 (keyboard + mouse input via DOM events); W1.1 (single-file HTML export — also covered by HTML_EXPORT.md); FFI.1-FFI.4 (generic `#native` marshaller, generic cdylib loader, eliminate per-fn glue, zero-boilerplate native-fn guide); W-warn (Clippy-inspired developer warnings). | Design (was `doc/claude/GAME_INFRA.md`).  Several items roadmap-scheduled for 0.8.6 (FFI.1-4, W-warn).  This is a kitchen-sink doc; over time individual arcs may split into their own plans. |
| [`future/06-web-services/`](future/06-web-services/) | Client-side web services library — long-term plan covering JSON (shipped), HTTP client (planned), and the broader scope toward "fully functioning": URL handling, auth helpers (Bearer / Basic / OAuth2 / cookies / signed requests), request/response refinements (streaming, multipart, content negotiation, conditional requests, retry/backoff, connection pooling), real-time push (SSE client, WebSocket client), transport/TLS (custom CAs, mTLS, proxies, DNS overrides), diagnostic tooling (mock client, recording, curl dump), and adjacent formats (form-urlencoded, YAML, MessagePack).  Three-file split: README.md (overview + scope + 8-step ship order), JSON.md (shipped reference + future extensions sketch), HTTP_CLIENT.md (locked-in `HttpResponse` + `http_*` design). | Mixed status (was `doc/claude/WEB_SERVICES.md`).  JSON shipped (`Type.parse` + `:j`); HTTP client is locked-in design deferred to 1.1+ per ROADMAP H4; future expansions sketched only.  Server-side (HTTP server, WebSockets, TLS termination, ACME, RBAC) is separate — see WEB_SERVER_LIB.md (still at doc root, awaiting promotion). |
| [`future/07-web-ide/`](future/07-web-ide/) | Fully serverless single-origin browser IDE for loft — zero-server, runs from `file://` or any static host.  Full Rust→WASM interpreter, CodeMirror 6 editor, problems panel, console, outline, go-to-definition / find-usages, multi-project IndexedDB storage, docs + examples browser, one-click ZIP export, PWA offline.  Roadmap series W1-W6.  Architecture: pre-bundled JS shell + `wasm-pack`-built loft binary; structured diagnostics in `src/diagnostics.rs`, thread-local output buffer in `src/fill.rs`, virtual filesystem in `src/parser/mod.rs`, JS bridge in `src/wasm.rs`. | Design (was `doc/claude/WEB_IDE.md`).  Deferred past 1.0 per ROADMAP § 1.0.0.  W2-W6 have `✓` design markers; LSP.1 is a prerequisite (the IDE consumes `loft-lsp` for diagnostics + symbol nav).  PACKAGES R1 workspace split is also a prerequisite (for the cdylib WASM target). |
| [`future/08-server/`](future/08-server/) | `server` library — fully featured HTTP server for loft programs.  Mostly written in loft itself with a thin native Rust layer for OS-level I/O + crypto.  ROADMAP series SRV.1-SRV.G: SRV.1 plain HTTP routing + middleware, SRV.2 HTTPS with static PEM certificates, SRV.3 WebSocket support, SRV.4 authentication (JWT / session / API key), SRV.5 ACME / Let's Encrypt automatic certificates, SRV.6 CORS / rate limiting / static files, SRV.G game loop (ws_poll, broadcast, ConnectionRegistry).  Companion to upcoming game-client library plan. | Design (was `doc/claude/WEB_SERVER_LIB.md`).  `lib/server/` already has 1234 lines of starting infrastructure (243 loft + 991 native Rust).  Full design covers feature set still to build.  Server-side game-loop additions co-design with plans/future/24-multiplayer-editor/ + plans/future/32-tic-tac-toe/ (active). |
| [`future/09-lsp/`](future/09-lsp/) | `loft-lsp` (LSP server) + `loft-dap` (DAP debug adapter) — protocol-agnostic editor integration that unlocks first-class support across every modern IDE (VSCode, Eclipse, JetBrains, Helix, Neovim, Sublime, the future browser IDE, Emacs).  ROADMAP series: LSP.1 (MVP, 0.8.6), LSP.2 (full editing surface, 0.9.0), LSP.3 (DAP interpreter-mode debug, 0.9.0), IDE.ECLIPSE / IDE.JETBRAINS / IDE.NEOVIM (1.0.0 — thin plugins). | Design (was `doc/claude/LSP.md`).  No `loft-lsp` / `loft-dap` binaries in tree yet — pure future plan.  Browser IDE (lib_plans/07-web-ide) consumes loft-lsp once it ships; native debugging (plans/25-native-debug) is the GDB/LLDB complement.  REPL plan (plans/08-repl-and-introspection) shares the introspection surface. |
| [`future/10-game-client/`](future/10-game-client/) | `game_client` library — multi-player game client.  ROADMAP series GC.1-GC.6: GC.1 WebSocket client + GameEnvelope protocol, GC.2 lobby + matchmaking, GC.3 fixed-timestep game loop, GC.4 client-side prediction + server reconciliation, GC.5 WASM script loading + Ed25519 verification (hot-swap untrusted scripts), GC.6 shared game logic with Tic-Tac-Toe demo.  Companion to lib_plans/08-server (server-side library). | Design (was `doc/claude/GAME_CLIENT_LIB.md`).  `lib/game_client/` doesn't exist yet — pure future plan.  Early `Dispatcher` / `run_game_loop` sketches in this design are SUPERSEDED by plans/23-event-loop's bidirectional handler model; the GC.x sub-features remain valid (they're features the EventLoop layer hosts).  Co-design with plans/24-multiplayer-editor + plans/future/32-tic-tac-toe (parked 2026-05-11 — which validate the protocol GC.6 demoes). |
| [`future/12-library-extraction/`](future/12-library-extraction/) | Library extraction — execution arc for PKG.EXTRACT.  Move `lib/*/` packages currently inside the main loft repo out into per-family external GitHub repositories, each consumable via the package registry.  Includes: per-library inventory + extraction priority (early/mid/late based on stability), per-library extraction template (8-step procedure), open-question register (naming, version policy, tagging, CI, BC window, transitive deps, etc.). | Blocked on PACKAGES.md § Open work PKG.REG landing first.  Substantial L-effort arc spanning multiple releases — each library extracted on its own validated schedule.  Sibling-coordinates with library plans (02-graphics, 05-game-infra, 08-server, 10-game-client) for the libraries those plans cover. |
| [`future/13-scriptable-scenes/`](future/13-scriptable-scenes/) | Scriptable scenes — users author loft scripts that drive scene behavior (hex enter/exit/interact in moros editor; analogous hooks elsewhere), edit them in an in-browser IDE, hot-reload without restart, share via scene JSON.  Consolidates 7 ROADMAP rows (SC.1-SC.6 + SC.P) that previously cross-cited each other with no doc home.  Includes 7-phase ship order, sandbox design (script-target build mode disables `use server` / `use file_io` etc.), 5 open design questions (script API style, hook signature evolution, script-to-script comm, resource limits, save state). | Plan-only; no implementation.  Scheduled for 1.0.0 (IDE + multiplayer block).  Depends on lib_plans/07-web-ide (W2 IDE editor shell) + moros editor MO.* milestones (which ship on demo-app cadence). |
| [`future/14-viewer-lsp-bridge/`](future/14-viewer-lsp-bridge/) | Viewer LSP bridge — sidecar Rust binary `loft-lsp-bridge` consuming rust-analyzer / loft-lsp / jdtls; gives `make view` real multi-language code intelligence (hover, jump-to-def, references, diagnostics).  Three IPC layers (browser↔viewer WebSocket, viewer↔bridge length-prefixed JSON over Unix socket, bridge↔servers stdio JSON-RPC).  Bridge intelligence (warm pool, multiplex, document cache, debounce, crash recovery, structured tracing) is the differentiator that justifies the Rust sidecar.  Designed 2026-05-13 in response to "tooling colleagues will judge by" framing.  CLIENT side; pairs with [`future/09-lsp/`](future/09-lsp/) (loft-lsp SERVER, consumed by phase 03). | Pure design.  Phase 0 scaffolds the binary + Layer-B socket protocol; phase 1 ships rust-analyzer end-to-end; phase 2 builds the bridge intelligence; phases 3-5 layer in loft-lsp / Java / browser polish; phase 6 closeout.  ~2 quarters of focused work.  Acceptance includes 5 quality metrics (cold start ≤ 2 s warm, hover P95 ≤ 50 ms, multi-language even-handedness, transparent crash recovery, log-surfacing UI). |

## Deferred library initiatives

Library plans well-described but intentionally paused — picked
up only when a concrete trigger arrives.  Distinct from
`future/`, which is "we will finish this, eventually" (real
intent to ship).  Deferred items are "we won't do this unless
something specific changes" — they may never run, and that is
acceptable.

| Dir | Initiative | Trigger to unpause |
|---|---|---|
| _(empty)_ |  |  |

## Finished library initiatives

| Dir | Initiative | Closed |
|---|---|---|
| _(empty)_ |  |  |
