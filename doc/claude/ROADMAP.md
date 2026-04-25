<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Roadmap

## Roadmap vs. release plan

This file is the **wish list**: items we want to do, ordered by
when they fit best into the project's arc.  Not every roadmap
item blocks a release — many can slip from one milestone to the
next without holding up a ship.

The companion file [RELEASE.md](RELEASE.md) answers a narrower
question: "what MUST be true before we tag and publish?"  When a
roadmap item is also a release blocker, it gets echoed into
RELEASE.md's gate lists.

| File | Scope | Question it answers |
|---|---|---|
| **ROADMAP.md** (this file) | Things we'd like to do | "What's the arc of work, and in what order?" |
| **[RELEASE.md](RELEASE.md)** | Ship checklist | "What must be true before we can publish?" |
| **[PLANNING.md](PLANNING.md)** | Priority-ordered backlog | "What's the next best thing to pick up?" |

Items in expected implementation order, grouped by milestone.
Full descriptions and fix paths: [PLANNING.md](PLANNING.md).

**Project goal:** browser games that anyone can play via a shared link.
Native OpenGL is supported for desktop enthusiasts.  Server/multiplayer
comes after the single-player browser experience works.

## Milestone narrative

| Version | Headline                                       | Status |
|---------|------------------------------------------------|--------|
| 0.8.0–0.8.4 | Game-ready interpreter, web export, JSON / HTTP, Brick Buster | **Shipped** (latest 0.8.4 — 2026-04-25) |
| 0.8.5   | **loft is learnable** — syntax highlighting, VS Code extension, 30-minute tutorial, native-CI parity | Next |
| 0.8.6   | **loft is extensible** — `loft install <name>` + package registry + zero-boilerplate native extensions | Planned |
| 0.9.0   | **Fully working loft language** — REPL + error recovery + warnings + libraries extracted to their own repos | Planned |
| 1.0.0   | **Totally sure everything works** — IDE + multiplayer + stability contract | Planned |

**Demo applications** (Brick Buster, Moros editor, Web IDE, game-client / server
libraries) ship on their own cadence — not gated by language releases.  See
[Demo applications — independent lifecycles](#demo-applications--independent-lifecycles)
at the end of this file for their backlogs.

**Effort:** XS = Tiny · S = Small · M = Medium · MH = Med–High · H = High · VH = Very High

**Design:** ✓ = detailed design in place · ~ = partial/outline · — = needs design

**Maintenance rule:** When an item is completed, remove it from this file.
Completed work belongs in CHANGELOG.md and git history.

---

## Carried over from 0.8.4

| ID    | Title                                                  | E  | Notes |
|-------|--------------------------------------------------------|----|-------|
| G3    | Tilemap rendering (grid-based 2D, batched draw)        | M  | Partial — the brick grid + `level_brick(lv,r,c)` dispatcher in Brick Buster is the tilemap for that game; a generic `lib/tilemap` package is still open. |
| G7.P  | 🌐 **Playable Brick Buster on itch.io** — `--html` works and Pages already serves the build; a separate itch.io upload remains.  Demo-app deliverable; no language work attached. | S | Optional |

---

## 0.8.5 — loft is learnable

**Goal:** a first-time visitor installs loft, gets syntax highlighting
in their editor, works through a 30-minute tutorial, and can step
through a `--native` build under stock GDB or LLDB.  Closes the "on
your own" wall newcomers hit today.

**Advertising narrative:** "learnable" is the first of three
advertising-readiness ships (0.8.5 / 0.8.6 / 0.9.0).  0.8.6 adds
extensibility + first-class IDE support; 0.9.0 finishes the language
surface.  Each is a standalone tag.

### Tooling polish

| ID    | Title                                                  | E  | Design | Source           |
|-------|--------------------------------------------------------|----|--------|------------------|
| SH.1  | TextMate grammar for `.loft`                           | S  | ✓      | DX.md            |
| SH.2  | VS Code extension (grammar + snippets + run task)      | S  | ✓      | DX.md            |
| DX.1  | Quick-start `examples/` directory at repo root         | XS | ✓      | DX.md            |
| DX.3  | "Learn loft in 30 minutes" walkthrough page            | S  | ✓      | DX.md            |
| NDB.0 | `--native-debug` flag — DWARF in `--native` builds     | XS | ✓      | NATIVE_DEBUG.md  |

*(DX.4 native-CI parity already in place — `tests/native.rs::native_dir` /
`native_scripts` run inside `cargo nextest run --profile ci` with empty
NATIVE_SKIP / SCRIPTS_NATIVE_SKIP lists.)*

### Ship criteria

- Every item above merged to main with `make ci` green.
- One external programmer (outside the loft project) can install
  SH.2 from VS Code Marketplace, open `examples/10-2d-canvas.loft`,
  read DX.3 top-to-bottom, and run the demo within 30 minutes from
  zero prior exposure.  Hands-on feedback collected before tagging.
- `loft --native --native-debug hello.loft` produces a binary that
  steps cleanly under stock `gdb` / `lldb` — variable names are
  rust-internal but the basic motion works.

---

## 0.8.6 — loft is extensible + first-class editor support

**Goal:** `loft install <name>` works; a user can add external
libraries without cloning and wiring by hand; the native-extension
author experience is boilerplate-free.  In parallel, the Loft
Language Server lights up VSCode, Eclipse, JetBrains, and Neovim with
diagnostics + outline + hover via thin marketplace plugins.

### Ecosystem foundation

| ID      | Title                                                  | E  | Design | Source           |
|---------|--------------------------------------------------------|----|--------|------------------|
| FFI.1   | Generic type marshaller from `#native` signature       | MH | ✓      | GAME_INFRA.md    |
| FFI.2   | Generic cdylib loader — scan exports, HashMap          | S  | ✓      | GAME_INFRA.md    |
| FFI.3   | Eliminate per-function glue in native.rs               | M  | ✓      | GAME_INFRA.md    |
| FFI.4   | Docs: zero-boilerplate native function guide           | S  | ✓      | GAME_INFRA.md    |
| PKG.7   | Lock file (`loft.lock`) for reproducible builds        | S  | ✓      | manifest.rs      |
| PKG.REG | Central package registry MVP — `loft install <name>`   | M  | ✓      | PACKAGES.md      |

### Language server + IDE plugins

One LSP server unlocks first-class support across every editor that
speaks the protocol.  Per-IDE plugins are thin marketplace shims
(~200 lines each) on top of LSP4E / LSP4IJ / nvim-lspconfig.

| ID            | Title                                                  | E  | Design | Source           |
|---------------|--------------------------------------------------------|----|--------|------------------|
| LSP.1         | `loft-lsp` MVP — diagnostics + outline + hover         | M  | ✓      | LSP.md           |
| IDE.ECLIPSE   | Eclipse plugin via LSP4E (LSP.1 features)              | S  | ✓      | LSP.md           |
| IDE.JETBRAINS | JetBrains plugin via LSP4IJ (LSP.1 features)           | S  | ✓      | LSP.md           |
| IDE.NEOVIM    | Neovim docs + `nvim-lspconfig` snippet                 | XS | ✓      | LSP.md           |

### Ship criteria

- `loft install <name>` resolves and installs from the public
  registry for at least 3 libraries.
- FFI.1–4 land together; `lib/graphics/native/` has at most 3
  hand-written type-pun functions remaining (down from ~15 today).
- A worked example of a third-party library outside the `loft`
  repo registering to the registry and being `loft install`-able.
- All 0.8.5 tooling (SH.1 / SH.2 / DX.1 / DX.3) still green against
  the registry-resolved libraries — no tutorial drift.
- `loft-lsp` serves diagnostics + outline + hover under VSCode,
  Eclipse, JetBrains, and Neovim on a sample 1k-line program, with
  re-parse latency under 100 ms in release mode.
- Eclipse Marketplace listing live; JetBrains Marketplace listing
  live; `nvim-loft.lua` snippet shipped under `doc/`.

---

## 0.9.0 — Fully working loft language

**Goal:** the language itself is feature-complete and the library
ecosystem lives in its own GitHub repos, leaving the `loft` project
as a lean interpreter + compiler + stdlib core.  Building on 0.8.5
(learnability) and 0.8.6 (extensibility), 0.9.0 closes the remaining
language gaps — error recovery, REPL, developer warnings — that
made "fully working language" a premature label in the earlier
plan, and completes the repo split that lets the ecosystem scale
beyond the solo-maintainer monorepo.

**Advertising readiness** — the 0.8.5 / 0.8.6 / 0.9.0 sequence is
the three-ship progression:
- **0.8.5** — *learnable*: syntax highlighting, VS Code extension,
  30-minute tutorial, native-mode debugging in stock GDB / LLDB.
- **0.8.6** — *extensible + first-class IDE*: `loft install <name>`,
  package registry, zero-boilerplate FFI, language server with
  Eclipse / JetBrains / Neovim plugins.
- **0.9.0** — *fully working*: language polish (L1 + P2 + W-warn +
  C52 + C53), full LSP editing surface + DAP debugger, plus
  `PKG.EXTRACT` moving every library out of the interpreter repo.

Each ship is a standalone tag with its own CHANGELOG entry — users
don't wait for 0.9.0 to see loft graduate from "curious hobby
project" to "bettable scripting language".

PKG.EXTRACT is the last 0.9.0 item — it depends on 0.8.6's PKG.REG
+ FFI.1–4, so starting it earlier duplicates work.

### Language polish

| ID     | Title                                                  | E  | Design | Source           |
|--------|--------------------------------------------------------|----|--------|------------------|
| L1     | Error recovery after token failures                    | M  | ✓      | PLANNING.md      |
| P2     | REPL / interactive mode                                | M  | ✓      | PLANNING.md      |
| W-warn | Developer warnings (Clippy-inspired)                   | M  | ✓      | GAME_INFRA.md    |
| AOT    | Auto-compile libraries to native shared libs           | M  | ✓      | PLANNING.md      |
| C52    | Stdlib name clash: warning + `std::` prefix            | M  | ✓      | PLANNING.md      |
| C53    | Match arms: library enums + bare variant names         | M  | ✓      | PLANNING.md      |

### User-biting caveats — all ship in 0.9.0

Each is a commitment, not a maybe.  Deferring any makes the
"fully working language" label dishonest.  Step plans:
[QUALITY.md](QUALITY.md).

| ID   | Title                                                  | E  | Source                      |
|------|--------------------------------------------------------|----|-----------------------------|
| C54  | `integer` → i64; `long` becomes a historical alias     | L  | CAVEATS.md, QUALITY.md      |
| P54  | First-class `JsonValue` enum; old text-based JSON gone | MH | PROBLEMS.md #54, QUALITY.md |

### Language server — full editing surface

Builds on LSP.1 from 0.8.6.  Once these land, parity with JDT-for-Java
in Eclipse is achievable, modulo optional project-wizard / debug-
perspective polish.

| ID     | Title                                                  | E  | Design | Source           |
|--------|--------------------------------------------------------|----|--------|------------------|
| LSP.2  | `loft-lsp` editing — completion, def, refs, rename, semantic tokens, code actions | MH | ✓ | LSP.md |
| LSP.3  | `loft-dap` MVP — DAP server for interpreter-mode debug | MH | ✓ | LSP.md |
| NDB.1  | `.loft.map` source map + `loft-gdb.py` / `loft-lldb.py` plugins | M  | ✓ | NATIVE_DEBUG.md  |

### Compilation cache and constant store

The `.loftc` bytecode cache and `CONST_STORE` are implemented
(Phase A + D).  Remaining work must land in 0.9.0 to avoid stability
risk in later milestones.

| ID     | Title                                                  | E  | Design | Source           |
|--------|--------------------------------------------------------|----|--------|------------------|
| CS.B   | mmap cache loading (native)                            | S  | ✓      | CONST_STORE.md   |
| CS.C1  | Serialize `Data` struct to binary (prereq for CS.C2/C3) | MH | ~     | CONST_STORE.md   |
| CS.C2  | `build.rs` pre-compile stdlib to `.loftc`              | M  | ✓      | CONST_STORE.md   |
| CS.C3  | WASM: `include_bytes!` stdlib cache, skip re-parse     | S  | ✓      | CONST_STORE.md   |

### Developer experience

| ID    | Title                                                  | E  | Design | Source           |
|-------|--------------------------------------------------------|----|--------|------------------|
| DX.2  | CI: add package tests + native tests to workflow       | XS | ✓      | DX.md            |

### Library extraction

| ID          | Title                                                  | E  | Design | Source           |
|-------------|--------------------------------------------------------|----|--------|------------------|
| PKG.EXTRACT | Move `lib/*/` out into per-family GitHub repos via PKG.REG | L | ✓ | PACKAGES.md      |

---

## 1.0.0 — Totally sure everything works

**Goal:** the stability contract. Anyone can write, run, and share a
program — terminal or browser — and trust that it will keep working.
Ship the IDE, ship multiplayer, and prove the language is bulletproof
with hands-on testing on every supported platform.

### IDE + multiplayer must-haves

| ID    | Title                                                  | E  | Design | Source           |
|-------|--------------------------------------------------------|----|--------|------------------|
| W2    | Editor shell (CodeMirror 6 + Loft grammar)             | M  | ✓      | WEB_IDE.md       |
| W3    | Symbol navigation (go-to-def, find-usages)             | M  | ✓      | WEB_IDE.md       |
| W4    | Multi-file projects (IndexedDB)                        | M  | ✓      | WEB_IDE.md       |
| W5    | Docs & examples browser                                | M  | ✓      | WEB_IDE.md       |
| W6    | Export/import ZIP + PWA offline                        | M  | ✓      | WEB_IDE.md       |

*(Desktop IDE plugins — IDE.ECLIPSE, IDE.JETBRAINS, IDE.NEOVIM —
shipped in 0.8.6 alongside LSP.1.  Web IDE W2–W6 above uses the same
`loft-lsp` server compiled to WASM.)*

### Scene scripting

| ID    | Title                                                  | E  | Design | Depends on    |
|-------|--------------------------------------------------------|----|--------|---------------|
| SC.1  | Scene script API — hooks for hex enter/exit/interact   | M  | ✓      | MO.3, W2      |
| SC.2  | IDE panel in scene editor                              | M  | ✓      | W2, MO.E1     |
| SC.3  | In-browser compile + hot-reload                        | M  | ✓      | W1, SC.1      |
| SC.4  | Script sandbox — limited API                           | S  | ✓      | SC.3          |
| SC.5  | Built-in script templates                              | S  | ✓      | SC.1          |
| SC.6  | Script sharing via scene JSON                          | S  | ✓      | SC.3, MO.2    |
| SC.P  | 🌐 **Scriptable scenes** in browser                     | S  | ✓      | SC.3, MO.P    |

### Multiplayer

| ID    | Title                                                  | E  | Design | Source              |
|-------|--------------------------------------------------------|----|--------|---------------------|
| SRV.1 | Plain HTTP routing + middleware                        | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.2 | HTTPS with static PEM certificates                     | S  | ✓      | WEB_SERVER_LIB.md   |
| SRV.3 | WebSocket support                                      | S  | ✓      | WEB_SERVER_LIB.md   |
| SRV.4 | Authentication: JWT, session, API key                  | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.5 | ACME / Let's Encrypt automatic certs                   | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.6 | CORS, rate limiting, static files                      | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.G | Game loop: ws_poll, broadcast, ConnectionRegistry      | M  | ✓      | WEB_SERVER_LIB.md   |
| GC.1  | WebSocket client + GameEnvelope protocol               | M  | ✓      | GAME_CLIENT_LIB.md  |
| GC.2  | Lobby + matchmaking                                    | S  | ✓      | GAME_CLIENT_LIB.md  |
| GC.3  | Fixed-timestep game loop                               | S  | ✓      | GAME_CLIENT_LIB.md  |
| GC.4  | Client-side prediction + reconciliation                | M  | ✓      | GAME_CLIENT_LIB.md  |
| GC.5  | WASM script loading + Ed25519 verification             | M  | ✓      | GAME_CLIENT_LIB.md  |
| GC.6  | Shared game logic + Tic-Tac-Toe demo                   | M  | ✓      | GAME_CLIENT_LIB.md  |
| MP.P  | 🌐 **Moros multiplayer** — DM + players share live scene | S  | ✓      | hosted server       |

### Stability gate (no shortcuts)

The 1.0.0 stability contract requires every item below to be checked off
before tagging — no "appears fixed" exceptions.

- [ ] **PROBLEMS.md** has zero open `**High**` severity entries
- [ ] All `⚠️ Appears fixed but unverified` flags from 0.9.0 have been
      definitively closed via real-world testing (not just regression guards)
- [ ] **valgrind clean** on a debug build of `tests/scripts/50-tuples.loft`
      and the full brick-buster game (`25-brick-buster.loft`) for 5+ minutes of play
- [ ] `make ci` green on Linux, macOS Intel, macOS ARM, Windows
- [ ] All `~~Fixed~~` PROBLEMS.md entries removed (history lives in CHANGELOG.md)
- [ ] `doc/claude/INCONSISTENCIES.md` reviewed: each entry resolved or
      explicitly accepted in LOFT.md / CHANGELOG.md
- [ ] Pre-built binaries on the GitHub release for all four platforms
- [ ] HTML reference and PDF up to date and linked from the release page

---

## 1.1+ — Backlog

| ID     | Title                                                  | E  | Design | Source              |
|--------|--------------------------------------------------------|----|--------|---------------------|
| C57    | Route decorator syntax (`@get`, `@post`, `@ws`)       | H  | ✓      | SERVER_FEATURES.md  |
| W1.14  | WASM Tier 2: Web Worker pool; `par()` parallelism     | VH | ✓      | WASM.md             |
| I13    | Iterator protocol (`for msg in ws` via `fn next`)     | MH | ✓      | SERVER_FEATURES.md  |
| I12    | Interfaces: factory methods (`fn zero() -> Self`)     | S  | ✓      | INTERFACES.md       |
| A12    | Lazy work-variable initialization                      | M  | ✓      | PLANNING.md         |
| O2     | Stack raw pointer cache                                | M  | ✓      | PLANNING.md         |
| A4     | Spatial index operations                               | M  | ✓      | PLANNING.md         |
| O4     | Native: direct-emit local collections                  | M  | ✓      | PLANNING.md         |
| O5     | Native: omit `stores` from pure functions              | M  | ✓      | PLANNING.md         |
| NDB.2  | DWARF rewrite — point `.debug_line` / `.debug_info` directly at `.loft`; stock debuggers need no plugin | MH | ✓ | NATIVE_DEBUG.md |

---

## Deferred indefinitely

| ID    | Title                                              | Notes                                     |
|-------|----------------------------------------------------|-------------------------------------------|
| O1    | Superinstruction peephole rewriting                | Opcode table full (254/256)               |
| P4    | Bytecode cache (`.loftc`)                          | Superseded by native codegen              |

---

## Demo applications — independent lifecycles

Per [RELEASE.md § Explicitly out of scope here](RELEASE.md#explicitly-out-of-scope-here),
demo applications ship on their own cadence and do **not** gate
any language release.  They may ship before, during, or after any
of the language milestones above.

| Demo | State | Backlog location |
|---|---|---|
| **Brick Buster** (`lib/graphics/examples/25-brick-buster.loft`) | **Shipped 2026-04-25** to GH Pages via the v0.8.4 release workflow ([brick-buster.html](https://jjstwerff.github.io/loft/brick-buster.html)).  itch.io publication still optional. | Carried-over note above |
| **Moros hex RPG editor — native** | **Shipped 2026-04-22** via plan-03 (`plans/finished/03-native-moros-editor/`); `make editor-dist` builds a self-contained `dist/moros-editor/` runnable without `loft` on the machine.  Fullscreen, scroll-wheel + expanded key codes, panel UI overlay, `editor_click` routing. | Historical — see plan-03 README. |
| **Moros hex RPG editor — web** | Designed but not built (~20 open sprints: MO.1–MO.13 covering map data model, JS scene editor, WASM build, 3D renderer, GLB export).  Depends on the loft libraries that will be extracted per PKG.EXTRACT; once those ship independently, the web editor can iterate without touching the language repo. | `../moros/doc/claude/` + `PLANNING.md` MO.* entries |
| **Web IDE** (W1.1 HTML export is language-side and done; W2–W6 are IDE work) | Deferred past 1.0 per ROADMAP.md § 1.0.0.  Independent project. | ROADMAP.md § 1.0.0 IDE+multiplayer block |
| **Server library** (`lib/server`), **game-client library** (`lib/game_client`), **scene scripting** layer | 1.1+ — `WEB_SERVER_LIB.md`, `GAME_CLIENT_LIB.md`, `SERVER_FEATURES.md` | Own design docs |

If a demo's progress reveals a language-side bug or a missing
primitive, the fix lands under the relevant language milestone
(0.9.0 for language polish, 1.0.0 for stability).  But the demo's
own scope never blocks a language tag.

---

**Design documents:**

| Area | Document |
|---|---|
| Developer experience | [DX.md](DX.md) |
| Game infrastructure | [GAME_INFRA.md](GAME_INFRA.md) |
| Package system | [PACKAGES.md](PACKAGES.md) |
| WASM + frame yield | [WASM.md](WASM.md) |
| Web IDE | [WEB_IDE.md](WEB_IDE.md) |
| Server library | [WEB_SERVER_LIB.md](WEB_SERVER_LIB.md) |
| Game client library | [GAME_CLIENT_LIB.md](GAME_CLIENT_LIB.md) |
| Graphics | [OPENGL_IMPL.md](OPENGL_IMPL.md) |
| Renderer abstraction | [RENDERER.md](RENDERER.md) |
