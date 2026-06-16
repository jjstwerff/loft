<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 01 — rust-analyzer end-to-end

**Status:** Open (depends on phase 00)

## Goal

Land the first WORKING multi-language code-intelligence
experience in the viewer.  Click any identifier in a `.rs`
file → jump to its definition.  Hover → tooltip with type
signature + doc comment.  Sidebar → list of references
across the workspace.

This is the phase that proves the architecture.  If the
contract from phase 00 is right, phase 01 ships visible value.
If the contract is wrong, phase 01 surfaces the wrongness
under load and we revise before bolting on more languages.

## What ships

### Bridge: spawn rust-analyzer + bridge methods

The bridge gains its first language adapter:

```rust
// tools/loft-lsp-bridge/src/servers/rust_analyzer.rs (NEW)
pub struct RustAnalyzerServer {
    proc: Child,                      // rust-analyzer subprocess handle
    sender: lsp_server::Connection,   // send to server's stdin
    receiver: ...                     // recv from server's stdout
    workspace_root: PathBuf,
    initialised: bool,
    open_documents: HashMap<Url, OpenDocument>,
}

impl RustAnalyzerServer {
    pub async fn spawn(workspace_root: &Path) -> anyhow::Result<Self> { ... }
    pub async fn initialize(&mut self) -> anyhow::Result<()> { ... }
    pub async fn did_open(&mut self, uri: Url, text: String) -> anyhow::Result<()> { ... }
    pub async fn hover(&mut self, params: HoverParams) -> anyhow::Result<Option<Hover>> { ... }
    pub async fn definition(&mut self, params: GotoDefinitionParams) -> anyhow::Result<Option<GotoDefinitionResponse>> { ... }
    pub async fn references(&mut self, params: ReferenceParams) -> anyhow::Result<Option<Vec<Location>>> { ... }
    pub async fn shutdown(self) -> anyhow::Result<()> { ... }
}
```

Bridge gains four Layer-B methods (called from the viewer):

| Layer-B method | Bridge action | Layer-C calls |
|---|---|---|
| `lsp.open` | Open a document, route to the right server by extension | `textDocument/didOpen` |
| `lsp.hover` | Forward to the active server for the URI | `textDocument/hover` |
| `lsp.definition` | Forward to the active server | `textDocument/definition` |
| `lsp.references` | Forward to the active server | `textDocument/references` |
| `lsp.close` | Close the document | `textDocument/didClose` |

Each takes `{ uri, position }` (or `{ uri, text }` for `open`)
and returns the server's response unchanged (LSP types
serialised as JSON).  Phase 02 adds caching + multiplex; phase
01 is one-server, one-client, no caching.

### Server discovery + spawn

```rust
// tools/loft-lsp-bridge/src/servers/discover.rs (NEW)
pub fn find_rust_analyzer() -> anyhow::Result<PathBuf> {
    // 1. $LOFT_RUST_ANALYZER env var
    // 2. ~/.cargo/bin/rust-analyzer (rustup install)
    // 3. PATH lookup
    // 4. Clear actionable error: "rust-analyzer not found.
    //    Install with: rustup component add rust-analyzer"
}
```

`rust-analyzer` is invoked with no args (it auto-detects the
workspace from cwd).  Bridge sets cwd to the workspace root
the viewer is serving.

### `initialize` handshake

The bridge sends a complete `InitializeParams` matching
rust-analyzer's expectations:

```json
{
  "processId": <bridge_pid>,
  "rootUri": "file:///home/user/project",
  "capabilities": {
    "textDocument": {
      "hover": { "contentFormat": ["markdown", "plaintext"] },
      "definition": { "linkSupport": true },
      "references": {}
    }
  },
  "initializationOptions": {
    "checkOnSave": false,    // we don't want cargo-check noise on every save in phase 01
    "cargo": { "loadOutDirsFromCheck": true }
  }
}
```

Phase 01 uses CONSERVATIVE capabilities — declare only what
the bridge actually consumes.  Phase 02 expands as the bridge
intelligence layer arrives.

### Viewer-side: hover popup + jump-to-def + refs sidebar

The viewer's `/file/<path>` page (today: line-numbered code)
gains JavaScript overlay code that:

1. **Connects to the bridge** via the loft-side library on
   page load.  Sends `lsp.open` for the current file.
2. **Hover detection**: on `mouseover` over any `<a class="line">`,
   compute the (line, col) under the cursor and send
   `lsp.hover`.  Render the response in a floating tooltip.
   Debounced at 100 ms (phase 02 raises this to 50 ms with
   server-side debouncing).
3. **Click-to-def**: on `Ctrl+Click` (or `⌘+Click` on Mac)
   on any identifier, send `lsp.definition`.  Navigate to
   `/file/<resolved_path>#L<line>`.
4. **References sidebar**: a "Show references" button per
   identifier; opens a sidebar listing every `Location`
   returned, each linked to its `/file/...#L<line>`.

The JS is small (~300 lines) and lives in
`tools/viewer/static/lsp_overlay.js`, served from the existing
`/static/` route.  Phase 05 layers a real editor framework on
top; phase 01 uses plain DOM events on the existing render.

### URI translation

LSP uses `file://` URIs; the viewer uses repo-relative paths.
The bridge translates in both directions:

```
viewer "/file/src/parser/expressions.rs" 
   ←→ bridge canonicalises against workspace_root
   ←→ rust-analyzer "file:///abs/path/to/src/parser/expressions.rs"
```

Translation is a pure function (`fn uri_to_relpath(uri: &Url,
root: &Path) -> Option<PathBuf>` and inverse).  Pinned by
unit tests that catch path-traversal attempts (a hostile
`file:///etc/passwd` URI must NOT round-trip to a viewable
path).

### Position translation

LSP positions are 0-based (line, character) pairs in UTF-16
code units.  The viewer's `<a id="L42">` line anchors are
1-based line numbers in bytes.

Translation:

```rust
// tools/loft-lsp-bridge/src/positions.rs (NEW)
/// LSP (line=0-based, char=UTF-16-code-units)
/// → viewer (line=1-based, byte_col=UTF-8-bytes)
pub fn lsp_to_viewer(pos: lsp_types::Position, line_text: &str) -> ViewerPosition { ... }
pub fn viewer_to_lsp(pos: ViewerPosition, line_text: &str) -> lsp_types::Position { ... }
```

Pinned by tests that exercise multi-byte UTF-8 (CJK
characters, emoji) — the same shape that surfaced
[`@P264`](../../PROBLEMS.md#open-issues--quick-reference)
in the JSON parser.  Position translation is the BUG MAGNET
of every LSP client; tests upfront pay back many times.

## Acceptance

1. `make view` opens the browser; navigate to
   `/file/src/parser/expressions.rs`.
2. Hover over any identifier → tooltip appears within 200 ms
   showing the type signature.  Doc comments visible if
   present.
3. `Ctrl+Click` any function call → page navigates to the
   function's definition (`/file/<file>#L<line>`).
4. Click "References" button on any identifier → sidebar
   opens listing every reference; each clickable → navigates.
5. Hover on a Chinese-character identifier (or emoji-named
   variable in a test fixture) → position translates correctly,
   tooltip lands on the right symbol.  No off-by-one.
6. Bridge log (`tail -f /tmp/loft-lsp-bridge-<pid>.log`)
   shows each request as a `tracing` span with method, URI,
   duration.
7. Killing rust-analyzer externally (`kill -9 $(pgrep rust-analyzer)`)
   → bridge surfaces an error to the viewer; viewer shows
   "rust-analyzer crashed; refresh to retry" banner.  Phase
   02 makes this auto-recover.
8. Cold start (first hover after `make view` boot): ≤ 30 s
   (rust-analyzer's indexing is the bottleneck).  Subsequent
   hovers: ≤ 200 ms P95.
9. CI gate: bridge tests pass on Linux + macOS lanes
   (Windows is phase-00's scope; phase 01 follows whatever
   shape phase 00 settles on).
10. `tests/lsp_bridge_rust_analyzer.rs` integration test:
    spawn bridge, open `tests/fixtures/lsp/hello.rs`, request
    hover at a known position, assert the response contains
    the expected type signature.

## Risks

| Risk | Mitigation |
|---|---|
| rust-analyzer initialisation (~30 s indexing) makes first hover feel broken | Viewer shows a "Indexing… (~30 s for first request)" banner during the cold start.  Phase 02's warm pool removes this for restarts. |
| rust-analyzer crashes under load | Phase 01 surfaces the crash; phase 02 adds auto-restart.  Don't conflate the two. |
| Position translation off-by-one with multi-byte text | Test corpus includes ASCII, BMP-CJK, emoji, combining diacritics.  All pinned. |
| `Ctrl+Click` collides with browser default behaviour | Override `e.preventDefault()` on the keydown+click combo.  Document the keybinding (`?` shortcut shows the help overlay). |
| LSP server output streamed to bridge fills memory if viewer doesn't read | Bridge has a per-server bounded buffer (1 MB default); overflow drops the oldest non-response notifications with a `tracing::warn!`. |
| URI canonicalisation lets a hostile path escape the workspace | Translation rejects any URI whose canonical path is outside `workspace_root`.  Pinned by negative tests. |
| Hover response is large markdown that breaks the tooltip layout | Cap tooltip body at 4 KB (server-side, in the bridge); add a "Show more" link that opens a modal with the full content. |
| Workspace root detection fails (no `Cargo.toml`) | Bridge falls back to current directory; viewer shows a banner "no Cargo workspace detected; rust-analyzer features may be limited." |

## What phase 01 does NOT ship

- Multi-server support (phase 04 adds Java; phase 03 adds loft).
- Bridge intelligence — caching, warm pool, multi-client
  multiplex, debounce, crash recovery.  ALL phase 02.
- Editing — read-only nav UX only.
- Completion / signature help / inlay hints — phase 05 (E2).
- Diagnostics squiggles — phase 02 (R2 layer).

## Critical files

| Path | Action |
|---|---|
| `tools/loft-lsp-bridge/src/servers/mod.rs` | NEW — `mod rust_analyzer;` and the `LanguageServer` trait abstraction |
| `tools/loft-lsp-bridge/src/servers/rust_analyzer.rs` | NEW — rust-analyzer-specific spawn + initialize + capability config |
| `tools/loft-lsp-bridge/src/servers/discover.rs` | NEW — locate rust-analyzer binary |
| `tools/loft-lsp-bridge/src/positions.rs` | NEW — LSP ↔ viewer position translation; UTF-16 ↔ UTF-8 ↔ 1-based |
| `tools/loft-lsp-bridge/src/uri.rs` | NEW — URI canonicalisation against workspace root with traversal guard |
| `tools/loft-lsp-bridge/src/routing.rs` | EXTEND — `lsp.open`/`hover`/`definition`/`references`/`close` dispatch |
| `tools/viewer/static/lsp_overlay.js` | NEW — hover detection, ctrl-click handler, refs sidebar.  ~300 lines vanilla JS, no framework. |
| `tools/viewer/src/main.loft` | EXTEND — `/file/<path>` page emits `<script src="/static/lsp_overlay.js">`; new `/static/lsp_overlay.js` route serves the file |
| `tests/fixtures/lsp/hello.rs` | NEW — minimal Rust file with a hover-target function and a definition target |
| `tests/lsp_bridge_rust_analyzer.rs` | NEW — Rust integration test: end-to-end hover round-trip |
| `tests/lsp_bridge_positions.rs` | NEW — UTF-16/UTF-8 position translation property tests |

## Cross-references

- [Phase 00 — scaffold](00-scaffold.md) — phase 01 builds on
  the Layer-B protocol established there.
- [Phase 02 — bridge intelligence](02-bridge-intelligence.md)
  — capabilities phase 01 deliberately defers (warm pool,
  multiplex, cache) all live there.
- [`rust-analyzer` LSP capability list](https://rust-analyzer.github.io/manual.html#features)
  — the surface phase 01 consumes (hover/def/refs); deeper
  features (rename, code-action) come in phase 05.
- [`lsp-types` crate](https://docs.rs/lsp-types/) — typed
  request/response structs.
- [`lsp-server` crate](https://docs.rs/lsp-server/) — message
  loop + framing.
