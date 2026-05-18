<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 03 — loft-lsp integration

**Status:** Open (depends on phase 02 + `lib_plans/future/09-lsp/` LSP.1)

## Goal

Light up `.loft` files in the viewer with the same hover /
jump-to-def / references UX rust-analyzer provides for `.rs`
files.  This is the moment loft becomes a FIRST-CLASS citizen
of its own tooling — not a special case, not a stub, the same
shape of UI as every other supported language.

## What ships

A new bridge adapter:

```rust
// tools/loft-lsp-bridge/src/servers/loft_lsp.rs (NEW)
pub struct LoftLspServer {
    proc: Child,                    // loft-lsp subprocess (built by lib_plans/09-lsp/)
    sender: lsp_server::Connection,
    receiver: ...
    workspace_root: PathBuf,
    initialised: bool,
    open_documents: HashMap<Url, OpenDocument>,
}
```

Implementation mirrors `RustAnalyzerServer` from phase 01 but
with loft-specific:

- **Server discovery**: `find_loft_lsp()` checks
  `$LOFT_LSP_BIN`, then `~/bin/loft-lsp` (@PLAN37 phase 08
  install), then `target/release/loft-lsp` (dev path), then
  `$PATH`.
- **Initialize options**: pass workspace root + path to
  `default/` stdlib so loft-lsp can resolve `use` imports.
- **Translator**: rewrite `loft://` URIs (loft-lsp uses these
  for synthesised stdlib symbols) to `/file/default/...`
  paths the viewer can navigate.

### Routing by extension

The bridge's request dispatcher already routes by URL
extension (added in phase 04 conceptually; phase 03 brings it
forward when the second server arrives):

```rust
fn server_for_uri(uri: &Url) -> Language {
    match Path::new(uri.path()).extension().and_then(|e| e.to_str()) {
        Some("rs") => Language::Rust,
        Some("loft") => Language::Loft,
        Some("java") => Language::Java,  // phase 04
        _ => Language::None,
    }
}
```

Files with no LSP backend fall through to the existing
read-only viewer behaviour (still browseable, just no hover/
def/refs).

### Acceptance

1. `make view` on the loft repo; navigate to
   `/file/default/01_code.loft`; hover over `to_uppercase` →
   tooltip shows the function signature + doc comment.
2. `Ctrl+Click` on a function call in `tools/viewer/src/main.loft`
   → page navigates to the function's definition.
3. References sidebar on a struct shows every `use` site
   across the workspace.
4. `loft-lsp` server is shared across browser tabs (phase 02
   multiplex still works).
5. Killing `loft-lsp` triggers auto-recovery (phase 02
   recovery still works).
6. UI is INDISTINGUISHABLE from the rust-analyzer experience
   (same tooltip styling, same sidebar shape, same keybinds).
7. CI: `tests/lsp_bridge_loft_lsp.rs` integration test.

## Risks

| Risk | Mitigation |
|---|---|
| loft-lsp from @PLAN09 isn't ready | Phase 03 depends on @PLAN09 LSP.1 explicitly.  If @PLAN14 phases 00-02 ship before @PLAN09 LSP.1, phase 03 waits. |
| loft-lsp positions / URIs differ from LSP spec | Translator module in the bridge handles divergences.  Coordinate with @PLAN09 to keep loft-lsp spec-compliant. |
| Loft files have no `Cargo.toml` analogue for workspace detection | Use repo root (`.git`) as the workspace boundary; same heuristic the indexer (@PLAN37) uses. |
| Stdlib symbols (`println`, `text.split`) need to render | loft-lsp synthesises documentation for stdlib via `default/*.loft` doc comments; bridge translator rewrites `loft://stdlib/...` URIs to `/file/default/...` paths. |

## Critical files

| Path | Action |
|---|---|
| `tools/loft-lsp-bridge/src/servers/loft_lsp.rs` | NEW — adapter |
| `tools/loft-lsp-bridge/src/servers/discover.rs` | EXTEND — `find_loft_lsp()` |
| `tools/loft-lsp-bridge/src/translator.rs` | EXTEND — `LoftLspTranslator` impl |
| `tools/loft-lsp-bridge/src/routing.rs` | EXTEND — dispatch `.loft` files to `Language::Loft` |
| `tests/lsp_bridge_loft_lsp.rs` | NEW — integration test |

## Cross-references

- [Phase 01 — rust-analyzer](01-rust-analyzer.md) — phase 03
  is the same shape.
- [Phase 02 — bridge intelligence](02-bridge-intelligence.md)
  — all the pool/multiplex/cache/recovery from phase 02
  applies to loft-lsp uniformly.
- [`lib_plans/future/09-lsp/`](../09-lsp/README.md) — the
  loft-lsp server this phase consumes.
