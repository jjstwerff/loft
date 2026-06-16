<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Scriptable scenes — user-authored scripts driving scene behavior

Users write loft scripts that drive scene behavior (hex
enter/exit/interact in the moros editor; analogous hooks in
other scene-based games), edit them in an in-browser IDE,
hot-reload them without restarting, share them via the scene's
JSON serialisation.

The 7 SC.* ROADMAP rows captured this cluster with no
dedicated design doc — they cross-cited each other and
upstream rows (W2 web IDE, MO.* moros editor).  This plan is
the **dedicated home** that consolidates the design + ship
order.

## Status

Designed only.  No implementation yet.  Scheduled in ROADMAP
1.0.0 milestone (IDE + multiplayer block).

## Sub-arcs (the 7 SC.* ROADMAP rows)

| Item | Title | Effort | Depends on | Notes |
|---|---|---|---|---|
| **SC.1** | Scene script API — hooks for hex enter / exit / interact | M | MO.3 (moros editor stable scene format), W2 (web IDE editor shell) | Defines the loft-side hook surface that scripts implement.  Each scene type has a known hook signature; user script subclasses or registers handlers. |
| **SC.2** | IDE panel in scene editor | M | W2 (web IDE editor shell), MO.E1 (moros editor extension point) | UI panel inside the moros editor for opening / editing / running the script attached to the current scene. |
| **SC.3** | In-browser compile + hot-reload | M | W1 (browser WASM target), SC.1 (script API) | The web-IDE's `wasm-pack`-built loft compiler accepts the user script source, recompiles it as the user edits, swaps the new bytecode into the running scene without restart. |
| **SC.4** | Script sandbox — limited API | S | SC.3 (compile + run path) | User scripts only see the documented Scene-Script API (no file I/O, no network, no shell, no `lib/server`).  Enforced at parse time by the loft compiler — reject `use server`, `use file_io`, etc. when the build target is `script`. |
| **SC.5** | Built-in script templates | S | SC.1 (script API) | Pre-canned starter scripts in the IDE's "new script" picker — empty hook, hello-world hook, NPC-walks-when-entered, door-opens-on-interact, etc. |
| **SC.6** | Script sharing via scene JSON | S | SC.3 (compile + run), MO.2 (moros editor scene JSON format) | The script source embeds in the scene's JSON file.  When the JSON loads, the embedded script auto-recompiles and binds.  Lets users share full playable scenes via a single file. |
| **SC.P** | 🌐 **Scriptable scenes** in browser (deliverable) | S | SC.3 (compile + run), MO.P (moros browser-playable) | Demo deliverable: a published scene with a non-trivial script, playable in the browser via a shared link.  Closes the user-facing story for "loft scripts a real game scene." |

## Cross-arc dependencies

This plan depends on:

- **W1 / W2 web IDE infrastructure** — see
  [`../07-web-ide/`](../62-web-ide).  SC.2 + SC.3 need the
  IDE editor shell + browser WASM target landed first.
- **MO.* moros editor** — moros is the first consumer.  The
  moros editor needs stable scene format (MO.3) + extension
  point (MO.E1) before SC.1 + SC.2 land.  Moros is a demo
  app shipping on its own cadence (per ROADMAP § Demo
  applications); MO.* rows are tracked there.
- **Compiler `script` build target** (for SC.4 sandbox) —
  not yet a documented build target.  Either extend
  `--target` with a `script` value that disables certain
  `use` paths, OR enforce sandbox at the embedding-host
  level by pre-validating the script source.  Decide at
  SC.4 design time.

## Phase ordering

Per the ROADMAP dependency arrows above, suggested sequence:

1. **SC.1** — scene script API (the hook surface).
   Depends on moros editor + web IDE landing.
2. **SC.3** — in-browser compile + hot-reload (the
   compile-and-run path).  Needs SC.1 to know what scripts
   look like.
3. **SC.2** — IDE panel (the UI on top of SC.1 + SC.3).
   Needs both to be useful.
4. **SC.4** — sandbox (security layer on top of SC.3).
   Pure restriction; lands once SC.3 is solid.
5. **SC.5** — templates (UX polish on top of SC.1 + SC.2).
   Pure content authoring; lands when SC.2 is solid.
6. **SC.6** — JSON embedding (data format extension on top
   of SC.3 + moros JSON).  Pure serialisation work.
7. **SC.P** — published deliverable (demo).  Closes the
   user-facing story; one published scene with a
   non-trivial script.

Each step is independent in implementation but the user-
facing story emerges when 1+3+2+4+6 are all done (the
sandbox + sharing closes the "real game scene anyone can
play in the browser" loop).

## Open design questions

These need decisions before SC.1 starts:

1. **Script API style** — pure functions registered by name
   (`fn on_enter(scene, hex) { … }`), or trait-style impls
   (`impl SceneScript for MyScene { fn on_enter(...) { … } }`)?
2. **Hook signature evolution** — when the engine adds a new
   hook (`on_save`?), do existing scripts break or do they
   silently lack the new hook?  Default-trait-impl approach
   would make this seamless.
3. **Script-to-script communication** — can scripts call
   functions in OTHER scripts?  Probably no for sandbox
   simplicity; scenes are the unit of script.
4. **Resource limits** — does the sandbox enforce CPU /
   memory limits per script?  WASM hosts can; do we?
   Probably defer to "not in MVP."
5. **Save state** — scripts may want to persist data
   between runs (NPC remembers it talked to the player
   yesterday).  Goes into the scene JSON, but format /
   API needs design.

## See also

- [`../07-web-ide/`](../62-web-ide) — browser IDE that
  hosts SC.2 (IDE panel) + SC.3 (in-browser compile)
- [`../09-lsp/`](../63-lsp) — LSP server that the IDE
  panel may consume for diagnostics + completion
- ROADMAP § 1.0.0 IDE + multiplayer block — milestone
  placement
- Moros editor (`lib/moros_editor/`) — the first
  consumer.  Demo app shipping on its own cadence
  (per ROADMAP § Demo applications).
