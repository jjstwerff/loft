<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 05 — Browser editor R1 + R2 (read-only nav + diagnostics)

**Status:** Open (depends on phase 02)

## Goal

Land the read-only browser-side experience that completes the
"review dashboard" framing the viewer was originally built
for.  Two visible jumps from phase 01:

- **R1** (read-only nav): hover popups, jump-to-def, refs
  sidebar — POLISHED.  Phase 01 shipped these in their first
  form; phase 05 makes them the production UX.
- **R2** (inline diagnostics): squiggle-underlines on errors
  + warnings; richer hover popup with multiple sections;
  references panel with grouping by file.

Editing (E1) is OUT of scope — that's a follow-up plan-14
phase 07+ once R1+R2 has been used in anger.  Read-only
covers ~80% of the review use case the viewer was built for.

## What ships

### R1 — Polished read-only nav

The phase 01 overlay JS is the v1; phase 05 ships the v2:

| v1 (phase 01) | v2 (phase 05) |
|---|---|
| Hover popup is a fixed div | Floating popup positioned via Popper.js (~10 KB); arrow points at the symbol |
| Refs sidebar is a flat list | Grouped by file with collapsible sections; current file expanded by default |
| Jump-to-def navigates the whole page | Jump-to-def updates URL hash + scrolls without full page reload (history API) |
| No back/forward navigation | Browser back button returns to prior position |
| No keyboard navigation | `?` shows keybind help; `g d` (go-to-def), `g r` (go-to-refs), `Esc` (close popup) |
| Symbol locations not highlighted | Hovered symbol gets a subtle background highlight while popup is open |

### R2 — Inline diagnostics + rich hover

```
src/parser/expressions.rs ──────────────────────
                                                
  fn parse_expr(&mut self) -> Value {
~~~~~~~~~~~~                                      ← squiggle underline (red)
    let x = self.parse_term();                    
    ~                                             ← squiggle (yellow warning)
    ...                                           
                                                
  Hover on `parse_term`:                          
  ┌─────────────────────────────────────────┐    
  │ fn parse_term(&mut self) -> Value       │   ← signature
  │                                         │    
  │ Parse a term expression. Returns the    │   ← doc comment
  │ parsed Value or `Value::Error` on fail. │    
  │                                         │    
  │ ► Show definition (g d)                 │   ← actions
  │ ► Find references (g r)                 │    
  │ ► View on GitHub                        │    
  │ ► Show 12 implementations               │    
  └─────────────────────────────────────────┘    
```

Implementation:

- **Diagnostics**: bridge forwards `publishDiagnostics`
  notifications to the viewer; overlay JS draws SVG
  squiggle paths under each diagnostic range.  Click a
  squiggle → side panel with the full message + related
  spans + suggested fix (if any).
- **Rich hover**: hover popup gains action buttons
  (Show definition, Find references, etc.) drawn from the
  LSP response's metadata.
- **Diagnostics panel**: persistent footer panel listing
  all diagnostics across the open file; click → scroll to
  the offending line.

### Frontend code organisation

```
tools/viewer/static/
├── lsp_overlay.js        (phase 01 — v1)
├── lsp_overlay/          (phase 05 — restructured)
│   ├── client.js         (bridge client + protocol types)
│   ├── hover.js          (hover detection + popup rendering)
│   ├── diagnostics.js    (squiggle drawing + panel)
│   ├── refs.js           (refs sidebar)
│   ├── nav.js            (jump-to-def + history API)
│   ├── keybinds.js       (g d, g r, ?, Esc)
│   └── popper.min.js     (vendored — ~10 KB)
└── lsp_overlay.css       (styling: tooltip, sidebar, squiggle, diagnostic panel)
```

Total JS bundle: ~2000 lines (excluding Popper).  No
framework — vanilla DOM + small modules.  Phase 07 (E1) is
where Monaco / CodeMirror enters; phase 05 deliberately
stays framework-free so the surface stays reviewable.

### Acceptance

1. Open any `.rs` / `.loft` / `.java` file in the viewer.
2. **Hover**: tooltip appears within 100 ms (P95) over any
   identifier.  Floats with the cursor; arrow points at
   the symbol.
3. **Jump-to-def**: `g d` (or `Ctrl+Click`) navigates without
   full page reload; URL updates; back button returns.
4. **Refs sidebar**: `g r` opens sidebar grouped by file;
   each entry clickable.
5. **Diagnostics**: errors + warnings rendered as squiggles;
   diagnostic panel shows a count badge in the footer; click
   → scroll to the diagnostic.
6. **Keybind help**: `?` opens an overlay listing every
   shortcut.
7. **Multi-language even-handedness**: same UX on `.rs`,
   `.loft`, `.java`.  Screenshot tests prove parity.
8. **Performance**: hover P95 ≤ 100 ms; squiggle render ≤
   16 ms (1 frame at 60 Hz); diagnostics-panel scroll
   smooth (no jank).
9. CI: `tests/scripts/lsp_browser_smoke.loft` (drives a
   headless browser via Playwright if the dep policy allows;
   otherwise a JS unit-test suite via the existing
   `tests/wasm.rs` shape).

## Risks

| Risk | Mitigation |
|---|---|
| 2000-line JS bundle drifts from the Rust bridge protocol | TypeScript type definitions auto-generated from `lsp-types` via `serde-typescript` / similar; bridge's `Translator` trait emits the schema at build time. |
| Squiggle rendering jank with many diagnostics (1000+) | Cap visible squiggles at 100 per viewport; render off-screen ones via virtualised overlay. |
| Hover-popup positioning breaks at viewport edges | Popper.js handles this; fall back to plain absolute-position if Popper fails. |
| Keybinds collide with browser defaults | `g d` / `g r` are vim-style sequences (no modifier); `Ctrl+Click` is intercepted only in the code-render area; `?` only when no input is focused. |
| Squiggle SVGs leak via XSS if diagnostic message has HTML | All LSP-returned text is escaped before rendering; CSP header (already in plan-35 phase 02) catches any leaks at the second layer. |
| Diagnostics from different files mix | Sidebar groups by file URI; each file panel is independent. |

## Critical files

| Path | Action |
|---|---|
| `tools/viewer/static/lsp_overlay/client.js` | NEW (or restructured from phase 01's `lsp_overlay.js`) |
| `tools/viewer/static/lsp_overlay/hover.js` | NEW |
| `tools/viewer/static/lsp_overlay/diagnostics.js` | NEW |
| `tools/viewer/static/lsp_overlay/refs.js` | NEW |
| `tools/viewer/static/lsp_overlay/nav.js` | NEW |
| `tools/viewer/static/lsp_overlay/keybinds.js` | NEW |
| `tools/viewer/static/lsp_overlay/popper.min.js` | VENDORED (~10 KB) |
| `tools/viewer/static/lsp_overlay.css` | NEW — tooltip + sidebar + squiggle + panel |
| `tools/viewer/src/main.loft` | EXTEND — `/file/<path>` page emits the new overlay bundle; `/static/lsp_overlay/<file>` route serves modules |
| `tests/scripts/lsp_browser_smoke.loft` | NEW — headless browser integration test (or JS-unit-suite alternative) |

## What phase 05 does NOT ship

- **E1 (single-file edit)** — adding a real editor framework
  (Monaco / CodeMirror); track as plan-14 phase 07.
- **E2 (completion + signature help)** — plan-14 phase 08.
- **E3 (refactoring + multi-file)** — plan-14 phase 09.
- **Workspace symbol search** (Ctrl+P / Cmd+P palette) —
  E2 territory.

These are LISTED in the plan-14 README's "Browser-side editor
— staged" table.  They're real follow-ups, not deferred-design;
promote to actual phase docs when R1+R2 has been used by the
target audience and the next bottleneck is editing.

## Cross-references

- [Phase 01 — rust-analyzer](01-rust-analyzer.md) — phase 05's
  R1 v2 supersedes phase 01's v1 overlay JS.
- [Phase 02 — bridge intelligence](02-bridge-intelligence.md)
  — diagnostics flow through the bridge's `publishDiagnostics`
  forwarding; cache populates the panel on tab join.
- [Plan-14 README — Quality bar](README.md#quality-bar-the-colleague-evaluator-framing)
  — the five metrics phase 05 is judged against.
- [Popper.js docs](https://popper.js.org/) — tooltip
  positioning library.
