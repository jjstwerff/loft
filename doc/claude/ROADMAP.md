<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Roadmap

Work items grouped by milestone, in expected implementation order.
Every row's `Source` column points at the plan that owns the design,
phasing, acceptance criteria, and closure record.

| Companion file | Purpose |
|---|---|
| [`RELEASE.md`](RELEASE.md) | What MUST be true before tagging |
| [`PLANNING.md`](PLANNING.md) | Priority-ordered backlog (next-best pickup) |
| [`plans/README.md`](plans/README.md) | docs-vs-plans rule + plan workflow |

**Project goal:** browser games anyone can play via a shared link.
Native OpenGL is supported for desktop enthusiasts; server/multiplayer
comes after the single-player browser experience works.

## Milestone narrative

| Version | Headline | Status |
|---|---|---|
| 0.8.0–0.8.4 | Game-ready interpreter, web export, JSON / HTTP, Brick Buster | **Shipped** (latest 0.8.4 — 2026-04-25) |
| 0.8.5 | **loft is learnable** — syntax highlighting, VS Code extension, 30-minute tutorial, native-CI parity | Next |
| 0.8.6 | **loft is extensible** — `loft install <name>` + package registry + zero-boilerplate native extensions | Planned |
| 0.9.0 | **Fully working loft language** — REPL + error recovery + warnings + libraries extracted to their own repos | Planned |
| 1.0.0 | **Totally sure everything works** — IDE + multiplayer + stability contract | Planned |

**Effort legend:** XS = Tiny · S = Small · M = Medium · MH = Med–High · H = High · VH = Very High

**Design legend:** ✓ = detailed design in place · ~ = partial/outline · — = needs design

**Maintenance rule:** When an item completes, remove it from this file.  Completed work belongs in CHANGELOG.md and git history.

---

## Carried over from 0.8.4

| ID | Title | E | Source |
|---|---|---|---|
| G3 | Tilemap rendering (grid-based 2D, batched draw) — generic `lib/tilemap` package | M | (no plan; brick-buster has level_brick dispatcher as its own tilemap) |
| G7.P | 🌐 Brick Buster on itch.io — optional demo-app deliverable | S | (no plan; deliverable, not language work) |

---

## 0.8.5 — loft is learnable

**Goal:** a first-time visitor installs loft, gets syntax highlighting in their editor, works through a 30-minute tutorial, and can step through a `--native` build under stock GDB or LLDB.

### Tooling polish

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| SH.1 | TextMate grammar for `.loft` | S | ✓ | plans/future/27-developer-experience/README.md |
| SH.2 | VS Code extension (grammar + snippets + run task) | S | ✓ | plans/future/27-developer-experience/README.md |
| DX.1 | Quick-start `examples/` directory at repo root | XS | ✓ | plans/future/27-developer-experience/README.md |
| DX.3 | "Learn loft in 30 minutes" walkthrough page | S | ✓ | plans/future/27-developer-experience/README.md |
| NDB.0 | `--native-debug` flag — DWARF in `--native` builds | XS | ✓ | plans/future/25-native-debug/README.md |

### Ship criteria

- All items above merged with `make ci` green.
- One external programmer can install SH.2 from VS Code Marketplace, open an example, read DX.3, and run a demo within 30 minutes from zero prior exposure.  Hands-on feedback collected before tagging.
- `loft --native --native-debug hello.loft` produces a binary that steps cleanly under stock `gdb` / `lldb`.

---

## 0.8.6 — loft is extensible + first-class editor support

**Goal:** `loft install <name>` works; native-extension authoring is boilerplate-free; one LSP server lights up VSCode / Eclipse / JetBrains / Neovim.

### Ecosystem foundation

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| FFI.1 | Generic type marshaller from `#native` signature | MH | ✓ | lib_plans/future/05-game-infra/README.md |
| FFI.2 | Generic cdylib loader — scan exports, HashMap | S | ✓ | lib_plans/future/05-game-infra/README.md |
| FFI.3 | Eliminate per-function glue in native.rs | M | ✓ | lib_plans/future/05-game-infra/README.md |
| FFI.4 | Docs: zero-boilerplate native function guide | S | ✓ | lib_plans/future/05-game-infra/README.md |
| PKG.7 | Lock file (`loft.lock`) for reproducible builds | S | ✓ | lib_plans/future/11-packages/README.md |
| PKG.REG | Central package registry MVP — `loft install <name>` | M | ✓ | lib_plans/future/11-packages/README.md |

### Language server + IDE plugins

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| LSP.1 | `loft-lsp` MVP — diagnostics + outline + hover | M | ✓ | lib_plans/future/09-lsp/README.md |
| IDE.ECLIPSE | Eclipse plugin via LSP4E (LSP.1 features) | S | ✓ | lib_plans/future/09-lsp/README.md |
| IDE.JETBRAINS | JetBrains plugin via LSP4IJ (LSP.1 features) | S | ✓ | lib_plans/future/09-lsp/README.md |
| IDE.NEOVIM | Neovim docs + `nvim-lspconfig` snippet | XS | ✓ | lib_plans/future/09-lsp/README.md |

### Ship criteria

- `loft install <name>` resolves and installs from the public registry for ≥ 3 libraries.
- FFI.1–4 land together; `lib/graphics/native/` has ≤ 3 hand-written type-pun functions remaining (down from ~15).
- A worked example of a third-party library outside the `loft` repo registering to the registry and being `loft install`-able.
- All 0.8.5 tooling still green against registry-resolved libraries (no tutorial drift).
- `loft-lsp` serves diagnostics + outline + hover under VSCode / Eclipse / JetBrains / Neovim on a 1k-line program with re-parse latency < 100 ms in release.
- Eclipse / JetBrains marketplace listings live; `nvim-loft.lua` snippet shipped under `doc/`.

---

## 0.9.0 — Fully working loft language

**Goal:** language feature-complete; library ecosystem lives in its own GitHub repos; `loft` repo is a lean interpreter + compiler + stdlib core.

### Language polish

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| L1 | Error recovery after token failures | M | ✓ | PLANNING.md |
| P2 | REPL / interactive mode | M | ✓ | plans/future/08-repl-and-introspection/README.md |
| W-warn | Developer warnings (Clippy-inspired) | M | ✓ | lib_plans/future/05-game-infra/README.md |
| AOT | Auto-compile libraries to native shared libs | M | ✓ | PLANNING.md |
| C52 | Stdlib name clash: warning + `std::` prefix | M | ✓ | PLANNING.md |
| C53 | Match arms: library enums + bare variant names | M | ✓ | PLANNING.md |

### User-biting caveats

| ID | Title | E | Source |
|---|---|---|---|
| P54 | First-class `JsonValue` enum; old text-based JSON gone | MH | plans/future/35-quality-followups/README.md |

### Language server — full editing surface

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| LSP.2 | `loft-lsp` editing — completion, def, refs, rename, semantic tokens, code actions | MH | ✓ | lib_plans/future/09-lsp/README.md |
| LSP.3 | `loft-dap` MVP — DAP server for interpreter-mode debug | MH | ✓ | lib_plans/future/09-lsp/README.md |
| NDB.1 | `.loft.map` source map + `loft-gdb.py` / `loft-lldb.py` plugins | M | ✓ | plans/future/25-native-debug/README.md |

### Compilation cache and constant store

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| CS.B | mmap cache loading (native) | S | ✓ | plans/deferred/28-const-store/README.md |
| CS.C1 | Serialize `Data` struct to binary (prereq for CS.C2/C3) | MH | ~ | plans/deferred/28-const-store/README.md |
| CS.C2 | `build.rs` pre-compile stdlib to `.loftc` | M | ✓ | plans/deferred/28-const-store/README.md |
| CS.C3 | WASM: `include_bytes!` stdlib cache, skip re-parse | S | ✓ | plans/deferred/28-const-store/README.md |

### Developer experience

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| DX.2 | CI: add package tests + native tests to workflow | XS | ✓ | plans/future/27-developer-experience/README.md |

### Library extraction

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| PKG.EXTRACT | Move `lib/*/` out into per-family GitHub repos via PKG.REG | L | ✓ | lib_plans/future/12-library-extraction/README.md |

---

## 1.0.0 — Totally sure everything works

**Goal:** stability contract.  Anyone can write, run, and share a program — terminal or browser — and trust it will keep working.  Ship the IDE, ship multiplayer, prove the language is bulletproof.

### IDE + multiplayer must-haves

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| W2 | Editor shell (CodeMirror 6 + Loft grammar) | M | ✓ | lib_plans/future/07-web-ide/README.md |
| W3 | Symbol navigation (go-to-def, find-usages) | M | ✓ | lib_plans/future/07-web-ide/README.md |
| W4 | Multi-file projects (IndexedDB) | M | ✓ | lib_plans/future/07-web-ide/README.md |
| W5 | Docs & examples browser | M | ✓ | lib_plans/future/07-web-ide/README.md |
| W6 | Export/import ZIP + PWA offline | M | ✓ | lib_plans/future/07-web-ide/README.md |

### Scene scripting

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| SC.1 | Scene script API — hooks for hex enter/exit/interact | M | ✓ | lib_plans/future/13-scriptable-scenes/README.md |
| SC.2 | IDE panel in scene editor | M | ✓ | lib_plans/future/13-scriptable-scenes/README.md |
| SC.3 | In-browser compile + hot-reload | M | ✓ | lib_plans/future/13-scriptable-scenes/README.md |
| SC.4 | Script sandbox — limited API | S | ✓ | lib_plans/future/13-scriptable-scenes/README.md |
| SC.5 | Built-in script templates | S | ✓ | lib_plans/future/13-scriptable-scenes/README.md |
| SC.6 | Script sharing via scene JSON | S | ✓ | lib_plans/future/13-scriptable-scenes/README.md |
| SC.P | 🌐 Scriptable scenes in browser | S | ✓ | lib_plans/future/13-scriptable-scenes/README.md |

### Multiplayer

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| SRV.1 | Plain HTTP routing + middleware | M | ✓ | lib_plans/future/08-server/README.md |
| SRV.2 | HTTPS with static PEM certificates | S | ✓ | lib_plans/future/08-server/README.md |
| SRV.3 | WebSocket support | S | ✓ | lib_plans/future/08-server/README.md |
| SRV.4 | Authentication: JWT, session, API key | M | ✓ | lib_plans/future/08-server/README.md |
| SRV.5 | ACME / Let's Encrypt automatic certs | M | ✓ | lib_plans/future/08-server/README.md |
| SRV.6 | CORS, rate limiting, static files | M | ✓ | lib_plans/future/08-server/README.md |
| SRV.G | Game loop: ws_poll, broadcast, ConnectionRegistry | M | ✓ | lib_plans/future/08-server/README.md |
| GC.1 | WebSocket client + GameEnvelope protocol | M | ✓ | lib_plans/future/10-game-client/README.md |
| GC.2 | Lobby + matchmaking | S | ✓ | lib_plans/future/10-game-client/README.md |
| GC.3 | Fixed-timestep game loop | S | ✓ | lib_plans/future/10-game-client/README.md |
| GC.4 | Client-side prediction + reconciliation | M | ✓ | lib_plans/future/10-game-client/README.md |
| GC.5 | WASM script loading + Ed25519 verification | M | ✓ | lib_plans/future/10-game-client/README.md |
| GC.6 | Shared game logic + Tic-Tac-Toe demo | M | ✓ | lib_plans/future/10-game-client/README.md |
| MP.P | 🌐 Moros multiplayer — DM + players share live scene | S | ✓ | (no plan; demo deliverable) |

### Stability gate (no shortcuts)

Every item below must be checked off before tagging — no "appears fixed" exceptions.

- [ ] **PROBLEMS.md** has zero open `**High**` severity entries
- [ ] All `⚠️ Appears fixed but unverified` flags from 0.9.0 definitively closed via real-world testing (not just regression guards)
- [ ] **valgrind clean** on a debug build of `tests/scripts/50-tuples.loft` and the full brick-buster game (`25-brick-buster.loft`) for 5+ minutes of play
- [ ] `make ci` green on Linux, macOS Intel, macOS ARM, Windows
- [ ] All `~~Fixed~~` PROBLEMS.md entries removed (history lives in CHANGELOG.md)
- [ ] `doc/claude/INCONSISTENCIES.md` reviewed: each entry resolved or explicitly accepted in LOFT.md / CHANGELOG.md
- [ ] Pre-built binaries on the GitHub release for all four platforms
- [ ] HTML reference and PDF up to date and linked from the release page

---

## 1.1+ — Backlog

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| C57 | Route decorator syntax (`@get`, `@post`, `@ws`) | H | ✓ | plans/future/29-server-features/README.md |
| I13 | Iterator protocol (`for msg in ws` via `fn next`) | MH | ✓ | plans/future/29-server-features/README.md |
| I12 | Interfaces: factory methods (`fn zero() -> Self`) | S | ✓ | INTERFACES.md |
| A12 | Lazy work-variable initialization | M | ✓ | PLANNING.md |
| O2 | Stack raw pointer cache | M | ✓ | PLANNING.md |
| A4 | Spatial index operations | M | ✓ | PLANNING.md |
| O4 | Native: direct-emit local collections | M | ✓ | plans/future/34-performance-followups/README.md |
| O5 | Native: omit `stores` from pure functions | M | ✓ | plans/future/34-performance-followups/README.md |
| NDB.2 | DWARF rewrite — point `.debug_line` / `.debug_info` directly at `.loft` | MH | ✓ | plans/future/25-native-debug/README.md |

---

## Deferred indefinitely

| ID | Title | Notes |
|---|---|---|
| O1 | Superinstruction peephole rewriting | Opcode table full (254/256) |

---

## Demo applications — independent lifecycles

Demo apps ship on their own cadence and do **not** gate any language release.  Per [`RELEASE.md` § Explicitly out of scope here](RELEASE.md#explicitly-out-of-scope-here).  If a demo surfaces a language-side bug, the fix lands under the relevant language milestone — but the demo's own scope never blocks a tag.

| Demo | State |
|---|---|
| **Brick Buster** | Shipped 2026-04-25 ([brick-buster.html](https://jjstwerff.github.io/loft/brick-buster.html)).  itch.io publication optional. |
| **Moros editor — native** | Shipped 2026-04-22 (plans/finished/03-native-moros-editor/).  `make editor-dist` builds a self-contained `dist/moros-editor/`. |
| **Moros editor — web** | Designed, not built.  ~20 sprints (MO.1–MO.13).  Lives in `../moros/doc/claude/` + PLANNING.md MO.* once PKG.EXTRACT lets the libraries iterate independently. |
| **Web IDE** (W2–W6) | 1.0.0 milestone above. |
| **Server / game-client / scene scripting libraries** | 1.0.0–1.1+ milestones above (lib_plans/future/08, 10, 13). |
