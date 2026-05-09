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

## All open plans — index by priority

Comprehensive list of every open plan across `plans/` and `lib_plans/`.  Sorted by priority bucket (P0 active → P1 next → P2 within year → P3 longer → D deferred).  This is the single place to read for "what's open, what depends on what, when."

**Priority legend:**
- **P0** — actively in flight (max 2-3 by the active-plan discipline)
- **P1** — next 6-12 months; ready to start
- **P2** — within ~1 year; design ready, scheduled
- **P3** — longer horizon; design exists, not actively scheduled
- **D** — deferred (won't do absent a concrete trigger)

### P0 — active

| Plan | E | Depends on | Milestone | Status |
|---|---|---|---|---|
| [`plans/07-error-messages/`](plans/07-error-messages/) | M | — | cross-cuts | Phases 0-3 shipped (rustc-style renderer + caret + `--errors` CLI); phases 4-7 open |
| [`plans/14-tuple-validation/`](plans/14-tuple-validation/) | M | — | 0.9.0 | Phases 00-01 shipped; 02-06 open.  P212 closed in passing.  Hosts the cross-mode harness used by 15/16/17/18/19/20 |

### P1 — next 6-12 months (0.8.5 / 0.8.6)

| Plan | E | Depends on | Milestone | Status |
|---|---|---|---|---|
| [`plans/future/25-native-debug/`](plans/future/25-native-debug/) | XS (NDB.0) / M (NDB.1) | — | 0.8.5 (NDB.0) / 0.9.0 (NDB.1) | NDB.0 = `--native-debug` flag with DWARF; NDB.1 = `.loft.map` + GDB/LLDB plugins |
| [`plans/future/27-developer-experience/`](plans/future/27-developer-experience/) | XS-S per item | — | 0.8.5 (SH.1/2 done; DX.1/3 open) | SH/DX/NT items.  Per-item landing procedures in plan README |
| [`lib_plans/future/05-game-infra/`](lib_plans/future/05-game-infra/) | M-MH per item | — | 0.8.6 (FFI.1-4) / 0.9.0 (W-warn) | Kitchen sink: FFI marshaller, cdylib loader, sprites, tilemap, collision, audio, HTML export, warnings |
| [`lib_plans/future/09-lsp/`](lib_plans/future/09-lsp/) | M (LSP.1) / MH (LSP.2/3) | — | 0.8.6 (LSP.1, IDE plugins) / 0.9.0 (LSP.2/3) | `loft-lsp` + `loft-dap` + thin Eclipse / JetBrains / Neovim plugins |
| [`lib_plans/future/11-packages/`](lib_plans/future/11-packages/) | S (PKG.7) / M (PKG.REG) | — | 0.8.6 | Pointer-plan: lock file + central registry MVP.  Package format itself shipped (14 lib/* use loft.toml) |

### P2 — within ~1 year (0.9.0)

| Plan | E | Depends on | Milestone | Status |
|---|---|---|---|---|
| [`plans/future/08-repl-and-introspection/`](plans/future/08-repl-and-introspection/) | M | — | 0.9.0 (P2) | Phases 0-1 shipped; phases 2-6 open.  REPL + IR/Rust/slot-table dump CLI |
| [`plans/future/33-native-codegen-followups/`](plans/future/33-native-codegen-followups/) | XS-M per item | — | incremental | Pointer-plan: N8b.3 yield-from + N8c.1/2 generic text-return audit + N20a/b fill.rs auto-gen |
| [`plans/future/34-performance-followups/`](plans/future/34-performance-followups/) | S-MH per item | — | incremental (1.1+ for P1) | Pointer-plan: 7 optimization designs (P1-P3, N1-N3, W1).  P1 blocked on opcode-table capacity |
| [`plans/future/35-quality-followups/`](plans/future/35-quality-followups/) | MH (P54) / S-M (Q1-Q4) | — | 0.9.0 (P54) | Pointer-plan: P54 sprint (JsonValue enum) + Q1-Q4 JSON ecosystem + Dep-inference + B2-B7 audit |
| [`lib_plans/future/03-lazy-stdlib/`](lib_plans/future/03-lazy-stdlib/) | M | — | 0.9.0 (foundational) | Conditional stdlib loading; trigger-based.  REGEX is first downstream consumer |
| [`lib_plans/future/12-library-extraction/`](lib_plans/future/12-library-extraction/) | L | **lib_plans/11-packages PKG.REG** | 1.1+ | Move `lib/*/` into per-family GitHub repos.  Multi-release execution arc |

### P3 — longer horizon (1.0.0 / 1.1+)

| Plan | E | Depends on | Milestone | Status |
|---|---|---|---|---|
| [`plans/future/22-mutable-closures/`](plans/future/22-mutable-closures/) | M-MH | — | 1.1+ (no firm slot) | Locked-in spec.  Four-case closure-capture classification; evolves C38 |
| [`plans/future/23-event-loop/`](plans/future/23-event-loop/) | MH | **P213 v4** (compiler bug) | 1.0.0 (with multiplayer) | Design spec.  PROTOCOL v1 (text-mode) shipped; v2 binary-mode designed |
| [`plans/future/24-multiplayer-editor/`](plans/future/24-multiplayer-editor/) | M | **plans/32 v2 ground layer** | 1.0.0 | First real-game milestone — multi-client hex editor |
| [`plans/future/26-match-peg/`](plans/future/26-match-peg/) | MH | — | 1.1+ | L3 PEG-style match patterns.  Cooperates with `lib_plans/01-regex` |
| [`plans/future/29-server-features/`](plans/future/29-server-features/) | S-H per item | — | 1.1+ | C55/C56/A15/I13/C57 — language features for upcoming server / game-client work |
| [`plans/future/30-sorted-slice/`](plans/future/30-sorted-slice/) | M | — | 1.1+ | A8: slicing, open-ended ranges, partial-key match on sorted/index |
| [`plans/future/32-tic-tac-toe/`](plans/future/32-tic-tac-toe/) | M | — | 1.0.0 (with multiplayer) | Protocol-validation vehicle.  v1 shipped; v2/v3/v4 ground layers designed |
| [`plans/future/21-retire-scratch/`](plans/future/21-retire-scratch/) | M | cooperates with 33 N8c.x + 34 N1 | incremental cleanup | Eliminate `stores.scratch` lifetime hazard.  No firm milestone |
| [`lib_plans/future/01-regex/`](lib_plans/future/01-regex/) | M | **lib_plans/03-lazy-stdlib** | 1.1+ | Standalone regex library.  First lazy-loaded stdlib consumer |
| [`lib_plans/future/02-graphics/`](lib_plans/future/02-graphics/) | H (multi-arc) | — | 1.0.0 (with IDE) | Graphics library bundle: 2D canvas + GLB + OpenGL + WebGL.  Low-level `gl_*` API shipped |
| [`lib_plans/future/04-asset-pipeline/`](lib_plans/future/04-asset-pipeline/) | M | — | 1.0.0+ | Game asset workflow: prototype → artist polish → integration |
| [`lib_plans/future/06-web-services/`](lib_plans/future/06-web-services/) | M-H per arc | — | 1.1+ (HTTP client) | JSON shipped; HTTP client + auth + WebSocket / SSE clients designed |
| [`lib_plans/future/07-web-ide/`](lib_plans/future/07-web-ide/) | M per W item | **lib_plans/09-lsp LSP.1** + **lib_plans/11-packages R1 workspace split** | 1.0.0 (W2-W6) | Browser IDE: zero-server, full WASM interpreter, CodeMirror 6 |
| [`lib_plans/future/08-server/`](lib_plans/future/08-server/) | M-MH per SRV | — | 1.0.0 | `server` library: HTTP routing, HTTPS, WebSocket, JWT, ACME, CORS, game loop |
| [`lib_plans/future/10-game-client/`](lib_plans/future/10-game-client/) | M | **plans/23 EVENT_LOOP** + cooperates with 08-server / 32-tic-tac-toe | 1.0.0 | `game_client` library: WebSocket client, lobby, prediction, WASM script loading |
| [`lib_plans/future/13-scriptable-scenes/`](lib_plans/future/13-scriptable-scenes/) | M-S per SC | **lib_plans/07-web-ide W2** + moros editor MO.* + script-target build mode | 1.0.0 | Scene scripts authored in browser IDE; sandboxed; scene-JSON-shareable |
| [`plans/future/15-closure-validation/`](plans/future/15-closure-validation/) | M | **plans/14 cross-mode harness** | incremental | Pre-flight 50% bug yield expected.  Closure round-trip validation matrix |
| [`plans/future/16-coroutine-validation/`](plans/future/16-coroutine-validation/) | M | **plans/14 cross-mode harness** | incremental | Pre-flight 0/7 cells passing initially.  Coroutine round-trip validation |
| [`plans/future/18-match-validation/`](plans/future/18-match-validation/) | M | **plans/14 cross-mode harness** | incremental | Pre-flight 33% hang rate on or-patterns / `@`-bindings |
| [`plans/future/19-struct-enum-validation/`](plans/future/19-struct-enum-validation/) | M | **plans/14 cross-mode harness** | incremental | Pre-flight 20% bug rate.  Struct-enum dispatch validation |
| [`plans/future/20-collection-validation/`](plans/future/20-collection-validation/) | M | **plans/14 cross-mode harness** | incremental | Self-deferred at pre-flight (panic does not currently reproduce).  Trigger to unpause: any user report of `index out of bounds` at `src/database/structures.rs:609` |

### D — deferred (won't do absent trigger)

| Plan | Trigger to unpause |
|---|---|
| [`plans/deferred/10-scope-exit-emission/`](plans/deferred/10-scope-exit-emission/) | A bug in this gate's territory, dep-tracking maintenance, or contributor interest |
| [`plans/deferred/12-codegen-simplifications/`](plans/deferred/12-codegen-simplifications/) | Same trigger set as plan 13: 3+ template-path bugs OR ≥50 Op-annotation touches OR contributor appetite.  Tier 1 shipped on branch `plan-12-codegen-simplifications` |
| [`plans/deferred/13-rust-template-migration/`](plans/deferred/13-rust-template-migration/) | 3+ template-path bugs OR major codegen evolution touching ≥50 Op annotations OR contributor appetite for multi-week refactor |
| [`plans/deferred/28-const-store/`](plans/deferred/28-const-store/) | Phase B: Phase C lands large embedded stdlib cache.  Phase C: contributor appetite for multi-week `Data` serialization OR demonstrated WASM cold-start gap.  Phases A + D already shipped |

### Cross-tracker dependency chains worth noting

- **lib_plans/03-lazy-stdlib → lib_plans/01-regex** (registry mechanism → first consumer)
- **lib_plans/11-packages PKG.REG → lib_plans/12-library-extraction** (registry → execution of monorepo split)
- **lib_plans/11-packages R1 + lib_plans/09-lsp LSP.1 → lib_plans/07-web-ide** (workspace split + LSP server → browser IDE)
- **plans/23-event-loop → lib_plans/10-game-client** (protocol abstraction → client library)
- **plans/23-event-loop → plans/24-multiplayer-editor** (depends transitively via plans/32-tic-tac-toe v2 ground layer)
- **plans/14-tuple-validation cross-mode harness → plans/15/16/18/19/20** (the validation-matrix toolchain feeds 5 sibling validation plans)
- **plans/33 N8c.x + plans/34 N1 → plans/21-retire-scratch** (scratch consumers must close before scratch itself can retire)
- **plans/22-mutable-closures spec → lib_plans/13-scriptable-scenes script API** (closure semantics inform user-script ergonomics)
- **C57 / I13 (in plans/29-server-features) → lib_plans/08-server route decorators + iterator protocol** (language features prerequisite for server API ergonomics)

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
