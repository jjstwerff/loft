<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Roadmap

Open work items grouped by value category, with explicit dependencies and effort estimates.

The methodology behind this file (categories, no-time-projections, features-need-plans) lives in [`plans/README.md` § Roadmap workflow](plans/README.md#roadmap-workflow).  This file holds only the work tables.

| Legend | Meaning |
|---|---|
| **S / R / G / F / U / C / Q / N** | Value category (silent-failure / regression / goal / foundation / UX / clean / quality / niche).  Default reading order — pick from the highest tier with open work. |
| **E** column | XS = Tiny · S = Small · M = Medium · MH = Med-High · H = High · VH = Very High · L = Large multi-arc |
| **Design** column | ✓ = detailed · ~ = partial · — = needs design |

| Companion file | Purpose |
|---|---|
| [`RELEASE.md`](RELEASE.md) | What MUST be true before tagging |
| [`PLANNING.md`](PLANNING.md) | Priority-ordered backlog (next-best pickup) |
| [`plans/README.md`](plans/README.md) | docs-vs-plans rule + plan workflow + roadmap workflow + value categories |

**Project goal:** browser games anyone can play via a shared link.  Native OpenGL is supported for desktop enthusiasts; server/multiplayer comes after the single-player browser experience works.

---

## S — Silent failure / data-loss prevention

Features that "appear to work" but don't, or that lose data without indication.  HIGHEST priority because invisible to users.  See [plans/README.md § Value categories](plans/README.md#value-categories--what-kind-of-value-not-just-how-much) for why S sits above R.

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Tuple validation across element × destination matrix (interp/native byte-identical) | M | ✓ | plans/14-tuple-validation/README.md |
| (cross) | Closure validation — capture × storage matrix | M | ✓ | plans/future/15-closure-validation/README.md |
| (cross) | Coroutine validation — yielded type × drive context matrix | M | ✓ | plans/future/16-coroutine-validation/README.md |
| (cross) | Match validation — subject type × pattern shape matrix | M | ✓ | plans/future/18-match-validation/README.md |
| (cross) | Struct-enum validation — variant payload × dispatch context matrix | M | ✓ | plans/future/19-struct-enum-validation/README.md |
| (cross) | Keyed collection validation — collection × operation matrix | M | ✓ | plans/future/20-collection-validation/README.md |
| Q* | JSON parse-error diagnostics (Q1) — parse currently fails silently in some shapes | S-M | ✓ | QUALITY.md#open-work--actionable-summary |
| (cross) | Closure-DbRef leak (LIFETIME.md "Type::Function — NOT YET HANDLED") | M | ~ | plans/future/15-closure-validation/ phase 03 (active risk) |

---

## R — Regression / release-blocker

Known broken behavior that gates the next release.  Currently empty — bug-fix workflow lands these as P-issues directly (PROBLEMS.md + test + commit), not as plans.  Items would surface here when a regression accumulates beyond a single fix.

*(no entries today)*

---

## G — Goal-enabling

Directly enables loft's core use case: browser games anyone can play via shared link, multiplayer, native-game debugging.

### Native game debugging

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| NDB.0 | `--native-debug` flag — DWARF in `--native` builds | XS | ✓ | plans/future/25-native-debug/README.md |
| NDB.1 | `.loft.map` source map + `loft-gdb.py` / `loft-lldb.py` plugins | M | ✓ | plans/future/25-native-debug/README.md |
| NDB.2 | DWARF rewrite — point `.debug_line` / `.debug_info` directly at `.loft` | MH | ✓ | plans/future/25-native-debug/README.md |

### Multiplayer + protocol stack

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Event-loop abstraction (client + server protocol) | MH | ✓ | plans/future/23-event-loop/README.md |
| (cross) | Protocol-validation vehicle (TIC_TAC_TOE v2/v3/v4 ground layers) | M | ✓ | plans/future/32-tic-tac-toe/README.md |
| (cross) | First real-game milestone — multi-client hex editor | M | ✓ | plans/future/24-multiplayer-editor/README.md |
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

### Browser game UI + scriptable scenes

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

### Game rendering

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Graphics library bundle (2D canvas + GLB + OpenGL + WebGL) — low-level `gl_*` shipped; high-level renderer designed | H | ✓ | lib_plans/future/02-graphics/README.md |

---

## F — Foundation

Unblocks 2+ downstream plans.  Lattice points in the dependency graph.

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| PKG.REG | Central package registry MVP — `loft install <name>` | M | ✓ | PACKAGES.md § Open work |
| PKG.7 | Lock file (`loft.lock`) for reproducible builds | S | ✓ | PACKAGES.md § Open work |
| PKG.EXTRACT | Move `lib/*/` out into per-family GitHub repos | L | ✓ | lib_plans/future/12-library-extraction/README.md |
| FFI.1 | Generic type marshaller from `#native` signature | MH | ✓ | lib_plans/future/05-game-infra/README.md |
| FFI.2 | Generic cdylib loader — scan exports, HashMap | S | ✓ | lib_plans/future/05-game-infra/README.md |
| FFI.3 | Eliminate per-function glue in native.rs | M | ✓ | lib_plans/future/05-game-infra/README.md |
| FFI.4 | Docs: zero-boilerplate native function guide | S | ✓ | lib_plans/future/05-game-infra/README.md |
| LSP.1 | `loft-lsp` MVP — diagnostics + outline + hover | M | ✓ | lib_plans/future/09-lsp/README.md |
| (cross) | Lazy stdlib loading — trigger-based pay-for-what-you-use | M | ✓ | lib_plans/future/03-lazy-stdlib/README.md |

---

## U — Ease of use

First-time-user experience, daily ergonomics, IDE polish.

### First-time experience + tutorial

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Better error messages — `file:line:col` + caret + suggestion | M | ✓ | plans/07-error-messages/README.md |
| SH.1 | TextMate grammar for `.loft` | S | ✓ | plans/future/27-developer-experience/README.md |
| SH.2 | VS Code extension (grammar + snippets + run task) | S | ✓ | plans/future/27-developer-experience/README.md |
| DX.1 | Quick-start `examples/` directory at repo root | XS | ✓ | plans/future/27-developer-experience/README.md |
| DX.3 | "Learn loft in 30 minutes" walkthrough page | S | ✓ | plans/future/27-developer-experience/README.md |
| DX.2 | CI: add package tests + native tests to workflow | XS | ✓ | plans/future/27-developer-experience/README.md |

### Day-to-day ergonomics

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| P2 | REPL / interactive mode | M | ✓ | plans/future/08-repl-and-introspection/README.md |
| W-warn | Developer warnings (Clippy-inspired) | M | ✓ | lib_plans/future/05-game-infra/README.md |
| L1 | Error recovery after token failures | M | ✓ | (needs plan promotion) |

### IDE editing surface

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| LSP.2 | `loft-lsp` editing — completion, def, refs, rename, semantic tokens, code actions | MH | ✓ | lib_plans/future/09-lsp/README.md |
| LSP.3 | `loft-dap` MVP — DAP server for interpreter-mode debug | MH | ✓ | lib_plans/future/09-lsp/README.md |
| IDE.ECLIPSE | Eclipse plugin via LSP4E (LSP.1 features) | S | ✓ | lib_plans/future/09-lsp/README.md |
| IDE.JETBRAINS | JetBrains plugin via LSP4IJ (LSP.1 features) | S | ✓ | lib_plans/future/09-lsp/README.md |
| IDE.NEOVIM | Neovim docs + `nvim-lspconfig` snippet | XS | ✓ | lib_plans/future/09-lsp/README.md |

---

## C — Clean features

Language correctness, removes special cases.  (Validation matrices that catch silent-failure variants live in S above; this section holds clean-feature work that doesn't primarily prevent silent failure.)

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| P54 | First-class `JsonValue` enum; old text-based JSON gone | MH | ✓ | QUALITY.md#open-work--actionable-summary |
| (cross) | Mutable-closure capture — novice-fit four-case classification | M-MH | ✓ | plans/22-mutable-closures/README.md |
| (cross) | L3 PEG-style match patterns (sequence / alternation / capture) | MH | ✓ | plans/future/26-match-peg/README.md |
| A8 | Slicing, open-ended ranges, partial-key match on sorted/index | M | ✓ | plans/future/30-sorted-slice/README.md |
| C52 | Stdlib name clash: warning + `std::` prefix | M | ✓ | (needs plan promotion) |
| C53 | Match arms: library enums + bare variant names | M | ✓ | (needs plan promotion) |
| I12 | Interfaces: factory methods (`fn zero() -> Self`) | S | ✓ | (needs plan promotion) |
| (cross) | Standalone regex library (cleaner text-pattern story) | M | ✓ | lib_plans/future/01-regex/README.md |

---

## Q — Internal quality

Performance, refactor, internal cleanup with clear payoff.

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Native codegen follow-ups (yield-from + generic text-return + fill.rs auto-gen) | XS-M per item | ✓ | NATIVE.md § Open work |
| (cross) | Performance follow-ups (P1-P3 interpreter / N1-N3 native / W1 wasm) | S-MH per item | ✓ | PERFORMANCE.md § Open work |
| (cross) | Retire `stores.scratch` lifetime hazard | M | ✓ | plans/future/21-retire-scratch/README.md |
| O4 | Native: direct-emit local collections | M | ✓ | PERFORMANCE.md § Open work (N1) |
| O5 | Native: omit `stores` from pure functions | M | ✓ | PERFORMANCE.md § Open work (N2) |
| A12 | Lazy work-variable initialization | M | ✓ | PLANNING.md (no PERFORMANCE.md design yet) |
| O2 | Stack raw pointer cache | M | ✓ | PLANNING.md (no PERFORMANCE.md design yet) |

### Constant store deferred-tail

The CS.B / CS.C1-C3 work items are deferred (Phase A + D shipped; Phase
B + C remain).  Trigger conditions and design content live in
[`plans/DEFERRED.md`](plans/DEFERRED.md) and the const-store plan in
`plans/deferred/`.  ROADMAP carries them only when the trigger fires.

---

## N — Niche / opportunistic

Small specific features, low-priority items.

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Game asset pipeline (prototype → artist polish → integration) | M | ✓ | lib_plans/future/04-asset-pipeline/README.md |
| (cross) | Web services — HTTP client + URL handling + auth + SSE/WS | M-H per arc | ✓ | lib_plans/future/06-web-services/README.md |
| C57 | Route decorator syntax (`@get`, `@post`, `@ws`) | H | ✓ | plans/future/29-server-features/README.md |
| I13 | Iterator protocol (`for msg in ws` via `fn next`) | MH | ✓ | plans/future/29-server-features/README.md |
| AOT | Auto-compile libraries to native shared libs | M | ✓ | (needs plan promotion) |
| A4 | Spatial index operations | M | ✓ | (needs plan promotion) |

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
| **Web IDE** (W2–W6) | G category above (Browser game UI section). |
| **Server / game-client / scene scripting libraries** | G category above (Multiplayer + protocol stack and Browser game UI sections). |

---

## All open plans — index by category

Comprehensive list of every open plan across `plans/` and `lib_plans/`, tagged by primary value category.

For per-phase status (what's shipped, what's in flight, what's blocked) **read the plan README directly** — it's the source of truth.  This index gives you the plan name, remaining effort, dependencies, and a one-line "what is this plan about" descriptor.

### S — Silent failure / data-loss prevention

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`plans/14-tuple-validation/`](plans/14-tuple-validation/) | M | — | Hosts cross-mode harness used by 15/16/18/19/20 |
| [`plans/future/15-closure-validation/`](plans/future/15-closure-validation/) | M | **plans/14 cross-mode harness** | Phase 03 also closes the closure-DbRef leak |
| [`plans/future/16-coroutine-validation/`](plans/future/16-coroutine-validation/) | M | **plans/14 cross-mode harness** | Yielded type × drive context matrix |
| [`plans/future/18-match-validation/`](plans/future/18-match-validation/) | M | **plans/14 cross-mode harness** | Subject type × pattern shape matrix |
| [`plans/future/19-struct-enum-validation/`](plans/future/19-struct-enum-validation/) | M | **plans/14 cross-mode harness** | Variant payload × dispatch context matrix |
| [`plans/future/20-collection-validation/`](plans/future/20-collection-validation/) | M | **plans/14 cross-mode harness** | Hash / sorted / index / spacial × operation matrix |

### G — Goal-enabling

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`plans/future/25-native-debug/`](plans/future/25-native-debug/) | XS-MH | — | NDB.0 / NDB.1 / NDB.2 — GDB / LLDB integration for `--native` |
| [`plans/future/23-event-loop/`](plans/future/23-event-loop/) | MH | **P213 v4** (compiler bug) | Bidirectional event-loop abstraction (client + server) |
| [`plans/future/24-multiplayer-editor/`](plans/future/24-multiplayer-editor/) | M | **plans/future/32 TIC_TAC_TOE v2 ground layer** | First real-game milestone |
| [`plans/future/32-tic-tac-toe/`](plans/future/32-tic-tac-toe/) | M | — | Protocol-validation vehicle (v1-v4 ground layers) |
| [`lib_plans/future/02-graphics/`](lib_plans/future/02-graphics/) | H (multi-arc) | — | Low-level GL + renderer abstraction |
| [`lib_plans/future/07-web-ide/`](lib_plans/future/07-web-ide/) | M per W item | **lib_plans/future/09-lsp LSP.1** + **PACKAGES.md § Open work R1 workspace split** | Browser IDE (W2-W6) |
| [`lib_plans/future/08-server/`](lib_plans/future/08-server/) | M-MH per SRV | — | HTTP / WS / static-file server library |
| [`lib_plans/future/10-game-client/`](lib_plans/future/10-game-client/) | M | **plans/future/23 EVENT_LOOP** + cooperates with 08-server / 32-tic-tac-toe | `game_client` library design |
| [`lib_plans/future/13-scriptable-scenes/`](lib_plans/future/13-scriptable-scenes/) | M-S per SC | **lib_plans/future/07-web-ide W2** + moros editor MO.* + script-target build mode | User-authored scene scripts (SC.1-SC.6 + SC.P) |
| [`plans/future/36-audience-generative-art/`](plans/future/36-audience-generative-art/) | M | — | Audience-driven plant/crystal growth demo via shared URL |

### F — Foundation

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`lib_plans/future/05-game-infra/`](lib_plans/future/05-game-infra/) | M-MH per item | — | FFI.1-4 — third-party native extensions |
| [`lib_plans/future/09-lsp/`](lib_plans/future/09-lsp/) | M (LSP.1) / MH (LSP.2/3) | — | LSP.1 unblocks 4 IDE plugins + browser IDE |
| [PACKAGES.md § Open work](PACKAGES.md#open-work) | S-M | — | PKG.7 + PKG.REG (format itself already shipped) |
| [`lib_plans/future/12-library-extraction/`](lib_plans/future/12-library-extraction/) | L | **PACKAGES.md § Open work PKG.REG** | Multi-release execution arc |
| [`lib_plans/future/03-lazy-stdlib/`](lib_plans/future/03-lazy-stdlib/) | M | — | Foundational — REGEX is first downstream consumer |

### U — Ease of use

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`plans/07-error-messages/`](plans/07-error-messages/) | M | — | `file:line:col` + caret + suggestions across parser / type / runtime / native |
| [`plans/future/27-developer-experience/`](plans/future/27-developer-experience/) | XS-S per item | — | SH.* / DX.* / NT.* — DX grab-bag (some shipped) |
| [`plans/future/08-repl-and-introspection/`](plans/future/08-repl-and-introspection/) | M | — | `loft>` interactive prompt + IR/Rust/slot-table CLI |

### C — Clean features

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`plans/22-mutable-closures/`](plans/22-mutable-closures/) | M-MH | — | Novice-fit closure capture (evolves C38).  Promoted to current 2026-05-10; drivers TTT v6 + plan-36 server retrofit |
| [`plans/future/26-match-peg/`](plans/future/26-match-peg/) | MH | — | L3 PEG-style match patterns (cooperates with regex lib) |
| [`plans/future/30-sorted-slice/`](plans/future/30-sorted-slice/) | M | — | A8 — slicing / open-ended ranges / partial-key match on sorted/index |
| [`lib_plans/future/01-regex/`](lib_plans/future/01-regex/) | M | **lib_plans/future/03-lazy-stdlib** | First lazy-loaded stdlib consumer |

### Q — Internal quality

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`plans/future/21-retire-scratch/`](plans/future/21-retire-scratch/) | M | cooperates with NATIVE § Open work N8c.x + PERFORMANCE § Open work N1 | Eliminate `stores.scratch` lifetime hazard |
| [NATIVE.md § Open work](NATIVE.md#open-work) | XS-M per item | — | N8b.3 yield-from + N8c.1/2 generic text-return audit + N20a/b fill.rs auto-gen |
| [PERFORMANCE.md § Open work](PERFORMANCE.md#open-work) | S-MH per item | P1 blocked on opcode-table capacity | 7 optimization designs (P1-P3 interpreter / N1-N3 native / W1 wasm) |

### N — Niche / opportunistic

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`plans/future/29-server-features/`](plans/future/29-server-features/) | S-H per item | — | C55/C56/A15/I13/C57 — language features for server / game-client |
| [`lib_plans/future/04-asset-pipeline/`](lib_plans/future/04-asset-pipeline/) | M | — | Game asset workflow |
| [`lib_plans/future/06-web-services/`](lib_plans/future/06-web-services/) | M-H per arc | — | JSON / HTTP client / auth / WebSocket / SSE clients |

### Deferred plans

Deferred plans don't appear on ROADMAP — their trigger index lives
in [`plans/DEFERRED.md`](plans/DEFERRED.md).  When a trigger fires,
the plan moves back to `future/` and ROADMAP gains a row.

### Cross-tracker dependency chains worth noting

- **lib_plans/future/03-lazy-stdlib → lib_plans/future/01-regex** (registry mechanism → first consumer)
- **PACKAGES.md § Open work PKG.REG → lib_plans/future/12-library-extraction** (registry → execution of monorepo split)
- **PACKAGES.md § Open work R1 + lib_plans/future/09-lsp LSP.1 → lib_plans/future/07-web-ide** (workspace split + LSP server → browser IDE)
- **plans/future/23-event-loop → lib_plans/future/10-game-client** (protocol abstraction → client library)
- **plans/future/23-event-loop → plans/future/24-multiplayer-editor** (depends transitively via plans/future/32-tic-tac-toe v2 ground layer)
- **plans/14-tuple-validation cross-mode harness → plans/future/15/16/18/19/20** (the validation-matrix toolchain feeds 5 sibling validation plans — all S category)
- **NATIVE.md § Open work N8c.x + PERFORMANCE.md § Open work N1 → plans/future/21-retire-scratch** (scratch consumers must close before scratch itself can retire)
- **plans/22-mutable-closures spec → lib_plans/future/13-scriptable-scenes script API** (closure semantics inform user-script ergonomics)
- **C57 / I13 (in plans/future/29-server-features) → lib_plans/future/08-server route decorators + iterator protocol** (language features prerequisite for server API ergonomics)

### Features still needing plan promotion

ROADMAP rows that still cite a flat reference doc as Source rather than a plan.  Promote when next-up work surfaces:

- **L1** Error recovery after token failures (U)
- **AOT** Auto-compile libraries to native shared libs (N)
- **C52** Stdlib name clash: warning + `std::` prefix (C)
- **C53** Match arms: library enums + bare variant names (C)
- **I12** Interfaces: factory methods (C)
- **A12, O2** Performance items (Q) — would fold into PERFORMANCE.md § Open work if their designs grow
- **A4** Spatial index operations (N)
