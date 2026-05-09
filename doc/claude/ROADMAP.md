<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Roadmap

Open work items grouped by **value** (impact per effort), with explicit dependencies.  No time projections — those rot.  Order within each value tier is suggested by effort + dependency unblocking, not calendar.

| Companion file | Purpose |
|---|---|
| [`RELEASE.md`](RELEASE.md) | What MUST be true before tagging |
| [`PLANNING.md`](PLANNING.md) | Priority-ordered backlog (next-best pickup) |
| [`plans/README.md`](plans/README.md) | docs-vs-plans rule + plan workflow |

**Project goal:** browser games anyone can play via a shared link.  Native OpenGL is supported for desktop enthusiasts; server/multiplayer comes after the single-player browser experience works.

**Value legend:**
- **V1** — high impact: directly enables the core use case OR unblocks multiple downstream plans
- **V2** — medium impact: meaningful capability or quality improvement
- **V3** — niche / internal / cleanup: real value but not user-visible at the language surface

**Effort legend:** XS = Tiny · S = Small · M = Medium · MH = Med–High · H = High · VH = Very High · L = Large multi-arc

**Design legend:** ✓ = detailed design in place · ~ = partial/outline · — = needs design

**Maintenance rule:** When an item completes, remove it from this file.  Completed work belongs in CHANGELOG.md and git history.

**Features need plans.**  Every feature row below should cite a plan in its Source column (or be small enough for direct PROBLEMS.md + commit, like a bug).  Tiny deliverables (demo deploys, single-action items) can stay on ROADMAP without plans.

---

## V1 — high value

Directly enables the core use case (browser games, multiplayer, learnable language) OR unblocks multiple downstream plans.

### Foundation — unblocks ecosystem

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| PKG.REG | Central package registry MVP — `loft install <name>` | M | ✓ | lib_plans/future/11-packages/README.md |
| PKG.7 | Lock file (`loft.lock`) for reproducible builds | S | ✓ | lib_plans/future/11-packages/README.md |
| FFI.1 | Generic type marshaller from `#native` signature | MH | ✓ | lib_plans/future/05-game-infra/README.md |
| FFI.2 | Generic cdylib loader — scan exports, HashMap | S | ✓ | lib_plans/future/05-game-infra/README.md |
| FFI.3 | Eliminate per-function glue in native.rs | M | ✓ | lib_plans/future/05-game-infra/README.md |
| FFI.4 | Docs: zero-boilerplate native function guide | S | ✓ | lib_plans/future/05-game-infra/README.md |

### User-visible quality — first-time experience + day-to-day use

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Better error messages — `file:line:col` + caret + suggestion | M | ✓ | plans/07-error-messages/README.md |
| SH.1 | TextMate grammar for `.loft` | S | ✓ | plans/future/27-developer-experience/README.md |
| SH.2 | VS Code extension (grammar + snippets + run task) | S | ✓ | plans/future/27-developer-experience/README.md |
| DX.1 | Quick-start `examples/` directory at repo root | XS | ✓ | plans/future/27-developer-experience/README.md |
| DX.3 | "Learn loft in 30 minutes" walkthrough page | S | ✓ | plans/future/27-developer-experience/README.md |
| NDB.0 | `--native-debug` flag — DWARF in `--native` builds | XS | ✓ | plans/future/25-native-debug/README.md |
| LSP.1 | `loft-lsp` MVP — diagnostics + outline + hover | M | ✓ | lib_plans/future/09-lsp/README.md |
| IDE.ECLIPSE | Eclipse plugin via LSP4E (LSP.1 features) | S | ✓ | lib_plans/future/09-lsp/README.md |
| IDE.JETBRAINS | JetBrains plugin via LSP4IJ (LSP.1 features) | S | ✓ | lib_plans/future/09-lsp/README.md |
| IDE.NEOVIM | Neovim docs + `nvim-lspconfig` snippet | XS | ✓ | lib_plans/future/09-lsp/README.md |

### Language correctness — bug yield + JSON ergonomics

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Tuple validation across element × destination matrix | M | ✓ | plans/14-tuple-validation/README.md |
| P54 | First-class `JsonValue` enum; old text-based JSON gone | MH | ✓ | plans/future/35-quality-followups/README.md |

---

## V2 — medium value

Meaningful capability or quality improvement.  Not foundational; users see the benefit but project doesn't fall over without it.

### Language polish

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| P2 | REPL / interactive mode | M | ✓ | plans/future/08-repl-and-introspection/README.md |
| W-warn | Developer warnings (Clippy-inspired) | M | ✓ | lib_plans/future/05-game-infra/README.md |
| L1 | Error recovery after token failures | M | ✓ | (needs plan promotion from PLANNING.md) |
| C52 | Stdlib name clash: warning + `std::` prefix | M | ✓ | (needs plan promotion from PLANNING.md) |
| C53 | Match arms: library enums + bare variant names | M | ✓ | (needs plan promotion from PLANNING.md) |
| (cross) | Mutable-closure capture — novice-fit four-case classification | M-MH | ✓ | plans/future/22-mutable-closures/README.md |
| (cross) | L3 PEG-style match patterns (sequence / alternation / capture) | MH | ✓ | plans/future/26-match-peg/README.md |
| A8 | Slicing, open-ended ranges, partial-key match on sorted/index | M | ✓ | plans/future/30-sorted-slice/README.md |

### Language server + native debug — full editing surface

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| LSP.2 | `loft-lsp` editing — completion, def, refs, rename, semantic tokens, code actions | MH | ✓ | lib_plans/future/09-lsp/README.md |
| LSP.3 | `loft-dap` MVP — DAP server for interpreter-mode debug | MH | ✓ | lib_plans/future/09-lsp/README.md |
| NDB.1 | `.loft.map` source map + `loft-gdb.py` / `loft-lldb.py` plugins | M | ✓ | plans/future/25-native-debug/README.md |

### Multiplayer + protocol stack

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Event-loop abstraction (client + server protocol) | MH | ✓ | plans/future/23-event-loop/README.md |
| (cross) | First real-game milestone — multi-client hex editor | M | ✓ | plans/future/24-multiplayer-editor/README.md |
| (cross) | Protocol-validation vehicle (TIC_TAC_TOE v2/v3/v4 ground layers) | M | ✓ | plans/future/32-tic-tac-toe/README.md |
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

### Web IDE + scriptable scenes

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| W2 | Editor shell (CodeMirror 6 + Loft grammar) | M | ✓ | lib_plans/future/07-web-ide/README.md |
| W3 | Symbol navigation (go-to-def, find-usages) | M | ✓ | lib_plans/future/07-web-ide/README.md |
| W4 | Multi-file projects (IndexedDB) | M | ✓ | lib_plans/future/07-web-ide/README.md |
| W5 | Docs & examples browser | M | ✓ | lib_plans/future/07-web-ide/README.md |
| W6 | Export/import ZIP + PWA offline | M | ✓ | lib_plans/future/07-web-ide/README.md |
| SC.1 | Scene script API — hooks for hex enter/exit/interact | M | ✓ | lib_plans/future/13-scriptable-scenes/README.md |
| SC.2 | IDE panel in scene editor | M | ✓ | lib_plans/future/13-scriptable-scenes/README.md |
| SC.3 | In-browser compile + hot-reload | M | ✓ | lib_plans/future/13-scriptable-scenes/README.md |
| SC.4 | Script sandbox — limited API | S | ✓ | lib_plans/future/13-scriptable-scenes/README.md |
| SC.5 | Built-in script templates | S | ✓ | lib_plans/future/13-scriptable-scenes/README.md |
| SC.6 | Script sharing via scene JSON | S | ✓ | lib_plans/future/13-scriptable-scenes/README.md |

### Library capabilities

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Lazy stdlib loading — trigger-based pay-for-what-you-use | M | ✓ | lib_plans/future/03-lazy-stdlib/README.md |
| (cross) | Standalone regex library (first lazy consumer) | M | ✓ | lib_plans/future/01-regex/README.md |
| (cross) | Web services — HTTP client + URL handling + auth + SSE/WS | M-H | ✓ | lib_plans/future/06-web-services/README.md |
| (cross) | Graphics library bundle (2D canvas + GLB + OpenGL + WebGL) | H | ✓ | lib_plans/future/02-graphics/README.md |
| (cross) | Game asset pipeline (prototype → artist polish → integration) | M | ✓ | lib_plans/future/04-asset-pipeline/README.md |
| (cross) | Library extraction — `lib/*/` to per-family GitHub repos | L | ✓ | lib_plans/future/12-library-extraction/README.md |

### Internal performance + cleanup

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Native codegen follow-ups (yield-from + generic text-return + fill.rs auto-gen) | XS-M per item | ✓ | plans/future/33-native-codegen-followups/README.md |
| (cross) | Performance follow-ups (P1-P3 interpreter / N1-N3 native / W1 wasm) | S-MH per item | ✓ | plans/future/34-performance-followups/README.md |
| (cross) | Retire `stores.scratch` lifetime hazard | M | ✓ | plans/future/21-retire-scratch/README.md |

---

## V3 — niche / internal / cleanup

Real value but not user-visible at the language surface.  Validation backlog (catches latent bugs), small specific features, single-purpose optimizations.

### Validation matrix backlog (catches latent bugs across language axes)

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Closure validation — capture × storage matrix | M | ✓ | plans/future/15-closure-validation/README.md |
| (cross) | Coroutine validation — yielded type × drive context matrix | M | ✓ | plans/future/16-coroutine-validation/README.md |
| (cross) | Match validation — subject type × pattern shape matrix | M | ✓ | plans/future/18-match-validation/README.md |
| (cross) | Struct-enum validation — variant payload × dispatch context matrix | M | ✓ | plans/future/19-struct-enum-validation/README.md |
| (cross) | Keyed collection validation — collection × operation matrix | M | ✓ | plans/future/20-collection-validation/README.md |

### Small features

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| AOT | Auto-compile libraries to native shared libs | M | ✓ | (needs plan promotion from PLANNING.md) |
| I12 | Interfaces: factory methods (`fn zero() -> Self`) | S | ✓ | (needs plan promotion from INTERFACES.md) |
| A12 | Lazy work-variable initialization | M | ✓ | (could fold into 34-performance-followups) |
| O2 | Stack raw pointer cache | M | ✓ | (could fold into 34-performance-followups) |
| A4 | Spatial index operations | M | ✓ | (needs plan promotion from PLANNING.md) |
| O4 | Native: direct-emit local collections | M | ✓ | plans/future/34-performance-followups/README.md |
| O5 | Native: omit `stores` from pure functions | M | ✓ | plans/future/34-performance-followups/README.md |
| C57 | Route decorator syntax (`@get`, `@post`, `@ws`) | H | ✓ | plans/future/29-server-features/README.md |
| I13 | Iterator protocol (`for msg in ws` via `fn next`) | MH | ✓ | plans/future/29-server-features/README.md |
| NDB.2 | DWARF rewrite — point `.debug_line` / `.debug_info` directly at `.loft` | MH | ✓ | plans/future/25-native-debug/README.md |
| CS.B | mmap cache loading (native) | S | ✓ | plans/deferred/28-const-store/README.md |
| CS.C1 | Serialize `Data` struct to binary (prereq for CS.C2/C3) | MH | ~ | plans/deferred/28-const-store/README.md |
| CS.C2 | `build.rs` pre-compile stdlib to `.loftc` | M | ✓ | plans/deferred/28-const-store/README.md |
| CS.C3 | WASM: `include_bytes!` stdlib cache, skip re-parse | S | ✓ | plans/deferred/28-const-store/README.md |
| DX.2 | CI: add package tests + native tests to workflow | XS | ✓ | plans/future/27-developer-experience/README.md |

### Deliverables (no plan needed — single-action items)

| ID | Title | E | Notes |
|---|---|---|---|
| G3 | Tilemap rendering (grid-based 2D, batched draw) — generic `lib/tilemap` | M | Brick Buster has level_brick dispatcher as own tilemap; generic library still open |
| G7.P | 🌐 Brick Buster on itch.io | S | Optional demo deploy; `--html` works, GH Pages already serves |
| MP.P | 🌐 Moros multiplayer — DM + players share live scene | S | Hosted-server demo |

---

## Stability gate (stability contract)

Every item below must be checked off before the project claims its stability bar — no "appears fixed" exceptions.  Cross-references the safety gate in [`RELEASE.md`](RELEASE.md).

- [ ] **PROBLEMS.md** has zero open `**High**` severity entries
- [ ] All `⚠️ Appears fixed but unverified` flags definitively closed via real-world testing (not just regression guards)
- [ ] **valgrind clean** on a debug build of `tests/scripts/50-tuples.loft` and the full brick-buster game (`25-brick-buster.loft`) for 5+ minutes of play
- [ ] `make ci` green on Linux, macOS Intel, macOS ARM, Windows
- [ ] All `~~Fixed~~` PROBLEMS.md entries removed (history lives in CHANGELOG.md)
- [ ] `doc/claude/INCONSISTENCIES.md` reviewed: each entry resolved or explicitly accepted in LOFT.md / CHANGELOG.md
- [ ] Pre-built binaries on the GitHub release for all four platforms
- [ ] HTML reference and PDF up to date and linked from the release page

---

## Deferred indefinitely

| ID | Title | Notes |
|---|---|---|
| O1 | Superinstruction peephole rewriting | Opcode table full (254/256) |

---

## Demo applications — independent lifecycles

Demo apps ship on their own cadence and do **not** gate any language work.  Per [`RELEASE.md` § Explicitly out of scope here](RELEASE.md#explicitly-out-of-scope-here).  If a demo surfaces a language-side bug, the fix lands under the relevant plan — but the demo's own scope never blocks anything.

| Demo | State |
|---|---|
| **Brick Buster** | Shipped 2026-04-25 ([brick-buster.html](https://jjstwerff.github.io/loft/brick-buster.html)).  itch.io publication optional. |
| **Moros editor — native** | Shipped 2026-04-22 (plans/finished/03-native-moros-editor/).  `make editor-dist` builds a self-contained `dist/moros-editor/`. |
| **Moros editor — web** | Designed, not built.  ~20 sprints (MO.1–MO.13).  Lives in `../moros/doc/claude/` + PLANNING.md MO.* once PKG.EXTRACT lets the libraries iterate independently. |
| **Web IDE** (W2–W6) | V2 above (Multiplayer + protocol stack section). |
| **Server / game-client / scene scripting libraries** | V2 above (Multiplayer + protocol stack section). |

---

## All open plans — index by value

Comprehensive list of every open plan across `plans/` and `lib_plans/`.  Sorted by value bucket; within bucket by tracker then ID.  Single place to read for "what's open, what depends on what, how valuable."

### V1 — high value (foundation + user-visible quality + correctness)

| Plan | E | Depends on | Status |
|---|---|---|---|
| [`plans/07-error-messages/`](plans/07-error-messages/) | M | — | Active.  Phases 0-3 shipped (rustc-style renderer + caret + `--errors` CLI); phases 4-7 open |
| [`plans/14-tuple-validation/`](plans/14-tuple-validation/) | M | — | Active.  Phases 00-01 shipped; 02-06 open.  Hosts cross-mode harness used by 15/16/18/19/20 |
| [`plans/future/25-native-debug/`](plans/future/25-native-debug/) | XS-MH | — | Design.  NDB.0 = `--native-debug` flag with DWARF; NDB.1 = `.loft.map` + GDB/LLDB plugins; NDB.2 = full DWARF rewrite |
| [`plans/future/27-developer-experience/`](plans/future/27-developer-experience/) | XS-S per item | — | SH.1 + SH.2 shipped.  DX.1, DX.3, DX.2 open |
| [`plans/future/35-quality-followups/`](plans/future/35-quality-followups/) | MH (P54) / S-M (Q1-Q4) | — | Pointer-plan.  P54 active sprint (JsonValue) + Q1-Q4 ecosystem + Dep-inference + B2-B7 audit |
| [`lib_plans/future/05-game-infra/`](lib_plans/future/05-game-infra/) | M-MH per item | — | FFI.1-4 unblock third-party native extensions; W-warn + G/GL items also covered |
| [`lib_plans/future/09-lsp/`](lib_plans/future/09-lsp/) | M (LSP.1) / MH (LSP.2/3) | — | Pure future.  LSP.1 unblocks 4 IDE plugins + browser IDE |
| [`lib_plans/future/11-packages/`](lib_plans/future/11-packages/) | S-M | — | Pointer-plan.  PKG.7 + PKG.REG.  Format itself shipped (14 lib/* use loft.toml) |

### V2 — medium value (capability + polish)

| Plan | E | Depends on | Status |
|---|---|---|---|
| [`plans/future/08-repl-and-introspection/`](plans/future/08-repl-and-introspection/) | M | — | Phases 0-1 shipped; phases 2-6 open |
| [`plans/future/22-mutable-closures/`](plans/future/22-mutable-closures/) | M-MH | — | Locked-in spec; not yet implemented |
| [`plans/future/23-event-loop/`](plans/future/23-event-loop/) | MH | **P213 v4** (compiler bug) | Design spec.  PROTOCOL v1 (text-mode) shipped |
| [`plans/future/24-multiplayer-editor/`](plans/future/24-multiplayer-editor/) | M | **plans/32 TIC_TAC_TOE v2 ground layer** | Plan only |
| [`plans/future/26-match-peg/`](plans/future/26-match-peg/) | MH | — | Cooperates with `lib_plans/01-regex` |
| [`plans/future/30-sorted-slice/`](plans/future/30-sorted-slice/) | M | — | Runtime affordance present (`key_compare` zip-prefix); only parser changes needed |
| [`plans/future/32-tic-tac-toe/`](plans/future/32-tic-tac-toe/) | M | — | v1 shipped; v2/v3/v4 protocol-only ground layers designed |
| [`plans/future/21-retire-scratch/`](plans/future/21-retire-scratch/) | M | cooperates with 33 N8c.x + 34 N1 | Eliminate `stores.scratch` lifetime hazard |
| [`plans/future/33-native-codegen-followups/`](plans/future/33-native-codegen-followups/) | XS-M per item | — | Pointer-plan.  N8b.3 yield-from + N8c.1/2 generic text-return audit + N20a/b fill.rs auto-gen |
| [`plans/future/34-performance-followups/`](plans/future/34-performance-followups/) | S-MH per item | P1 blocked on opcode-table capacity | Pointer-plan.  7 optimization designs |
| [`lib_plans/future/01-regex/`](lib_plans/future/01-regex/) | M | **lib_plans/03-lazy-stdlib** | First lazy-loaded stdlib consumer |
| [`lib_plans/future/02-graphics/`](lib_plans/future/02-graphics/) | H (multi-arc) | — | Low-level `gl_*` API shipped; renderer abstraction designed |
| [`lib_plans/future/03-lazy-stdlib/`](lib_plans/future/03-lazy-stdlib/) | M | — | Foundational; REGEX is first downstream consumer |
| [`lib_plans/future/04-asset-pipeline/`](lib_plans/future/04-asset-pipeline/) | M | — | Game asset workflow |
| [`lib_plans/future/06-web-services/`](lib_plans/future/06-web-services/) | M-H per arc | — | JSON shipped; HTTP client + auth + WebSocket / SSE clients designed |
| [`lib_plans/future/07-web-ide/`](lib_plans/future/07-web-ide/) | M per W item | **lib_plans/09-lsp LSP.1** + **lib_plans/11-packages R1 workspace split** | W2-W6 |
| [`lib_plans/future/08-server/`](lib_plans/future/08-server/) | M-MH per SRV | — | `lib/server/` has 1234 lines of starting code; design covers full feature set |
| [`lib_plans/future/10-game-client/`](lib_plans/future/10-game-client/) | M | **plans/23 EVENT_LOOP** + cooperates with 08-server / 32-tic-tac-toe | `game_client` library design |
| [`lib_plans/future/12-library-extraction/`](lib_plans/future/12-library-extraction/) | L | **lib_plans/11-packages PKG.REG** | Multi-release execution arc |
| [`lib_plans/future/13-scriptable-scenes/`](lib_plans/future/13-scriptable-scenes/) | M-S per SC | **lib_plans/07-web-ide W2** + moros editor MO.* + script-target build mode | Plan-only |

### V3 — niche / internal / cleanup

| Plan | E | Depends on | Status |
|---|---|---|---|
| [`plans/future/15-closure-validation/`](plans/future/15-closure-validation/) | M | **plans/14 cross-mode harness** | Pre-flight 50% bug yield expected |
| [`plans/future/16-coroutine-validation/`](plans/future/16-coroutine-validation/) | M | **plans/14 cross-mode harness** | Pre-flight 0/7 cells initially |
| [`plans/future/18-match-validation/`](plans/future/18-match-validation/) | M | **plans/14 cross-mode harness** | Pre-flight 33% hang rate on or-patterns / `@`-bindings |
| [`plans/future/19-struct-enum-validation/`](plans/future/19-struct-enum-validation/) | M | **plans/14 cross-mode harness** | Pre-flight 20% bug rate |
| [`plans/future/20-collection-validation/`](plans/future/20-collection-validation/) | M | **plans/14 cross-mode harness** | Self-deferred; trigger to unpause: user report of `index out of bounds` at `src/database/structures.rs:609` |
| [`plans/future/29-server-features/`](plans/future/29-server-features/) | S-H per item | — | C55/C56/A15/I13/C57 — language features for server / game-client |

### Deferred (won't do absent trigger)

| Plan | Trigger to unpause |
|---|---|
| [`plans/deferred/10-scope-exit-emission/`](plans/deferred/10-scope-exit-emission/) | A bug in this gate's territory, dep-tracking maintenance, or contributor interest |
| [`plans/deferred/12-codegen-simplifications/`](plans/deferred/12-codegen-simplifications/) | Same trigger set as plan 13.  Tier 1 shipped on branch `plan-12-codegen-simplifications` |
| [`plans/deferred/13-rust-template-migration/`](plans/deferred/13-rust-template-migration/) | 3+ template-path bugs OR major codegen evolution touching ≥50 Op annotations OR contributor appetite |
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

### Features still needing plan promotion

The following ROADMAP rows still cite PLANNING.md or other reference docs as Source instead of having dedicated plans.  Per the docs-vs-plans rule + the "features need plan cadence" direction, each should be promoted to a plan before it ships:

- **L1** Error recovery after token failures (V2 polish)
- **AOT** Auto-compile libraries to native shared libs (V3 small)
- **C52** Stdlib name clash: warning + `std::` prefix (V2 polish)
- **C53** Match arms: library enums + bare variant names (V2 polish)
- **I12** Interfaces: factory methods (V3 small)
- **A12, O2** Performance items (V3 small) — could fold into `34-performance-followups` if their designs grow

Promote each at the moment it surfaces as next-up work.
