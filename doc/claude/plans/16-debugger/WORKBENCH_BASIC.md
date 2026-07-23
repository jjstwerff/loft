<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Workbench — the basic foundation, step by step

> **Scope:** the four things that make it an *editor*, loft-only: **open a project · show its
> files · edit + save text files · LSP integration.** This is the concrete, code-pointed
> decomposition of [`WORKBENCH.md`](WORKBENCH.md) phases **WB0 + WB3 + WB1** for the single-
> language case. No debugger UI, no Rust/C, no bridge, no git, no game — those are later.
> **Status:** build plan. Steps **B1–B6**, each independently landable + testable, in order
> (B1 is the prerequisite for all).

## The one simplification that makes "basic" small

`loft-lsp` (the binary) is a thin stdio shell over the **`loft::lsp` library module**, and
that module is **stateless**: `diagnose(text, name, stdlib_dir)`, `symbol_at(…, line, col)`
(hover), `resolve_at(…, line, col)` (go-to-def), `complete(…, line, col)` — *text in,
structured result out* (`src/lsp.rs:62,163,265,1319`). **`src/serve.rs` already links the
`loft` rlib**, so the gateway can call these functions **in-process** — no child process, no
JSON-RPC framing, no `loft-lsp-bridge`. The browser holds the live buffer and sends it with
each request; the gateway computes and replies. (The sidecar bridge from @PLN66 is only needed
for the *external, stateful* servers — rust-analyzer, clangd — which arrive with multi-language
support, not here.)

Everything else is **generalising serve.rs's single-file assumptions to a workspace root** —
work M5e's [`IDE.md`](IDE.md) already names as the cheapest first move.

## Current state (what B-steps start from) — code map

| Seam | Where | Today | B-step |
|---|---|---|---|
| Server entry | `serve.rs:34` `run_serve(stdlib, libs, port, file)` | takes **one file** | B1 → a **root** |
| Write sandbox | `repl.rs:1718` `set_workspace_file` / `:1730` `write_file` | **one canonical file** (path-equality) | B1/B4 → **under-root** |
| Verb dispatch | `rpc.rs:149` `handle()` — `launch`/`writeFile`/`compile`/… | no file I/O verbs | B2/B4/B6 → `listFiles`/`readFile`/`lsp*` |
| Shell | `serve.rs:332` `render_shell` + `:339` `SHELL_TEMPLATE`; `<textarea id="src">` `:407`; `const FILE` `:426` | inline HTML, **one hardcoded file**, raw textarea | B3/B5 → tree + tabs + CM6 |
| LSP brain | `src/lsp.rs` (`diagnose`/`symbol_at`/`resolve_at`/`complete`) | used only by the `loft-lsp` binary | B6 → called **in-process** |

---

## B1 — Open a project (a workspace root, not one file)

**Goal.** `loft debug --serve <dir> [--lib …]` (and an `loft ide` alias) points the server at a
**directory**; the server knows the workspace root; the shell loads with no file open yet (an
empty editor + "select a file"). Existing single-file invocation still works (a file arg ⇒ root
= its parent, that file pre-opened).

**Code-points.**
- `src/main.rs` (the `--serve` dispatch, ~`3548`): accept a dir *or* a file; compute
  `workspace_root` = the dir, or the file's parent. Pass it to `run_serve`. Add a thin
  `loft ide <dir>` alias (optional; can defer — `debug --serve <dir>` suffices for B1).
- `src/serve.rs:34` `run_serve`: add a `root: &str` param; replace `set_workspace_file(file)`
  (`:48`) with `set_workspace_root(root)`; keep an optional `open: Option<&str>` (the pre-opened
  file). `render_shell` no longer bakes in a single file (B3 fills the tree).
- `src/repl.rs`: add `workspace_root: Option<PathBuf>` beside `workspace_file` (`:1194`); add
  `pub fn set_workspace_root(&mut self, dir: &str)` = `canonicalize(dir)`.

**Protocol.** none new (B1 is server plumbing).

**Gate.** `tests/serve.rs::serve_opens_workspace_root` — start `run_serve` on a temp dir, `GET /`
returns 200 + the shell; the session's root canonicalises to the temp dir. Single-file back-compat
test still green.

**Reuse ↔ build.** Reuse: all of `run_serve`/`ReplSession`. Build: root field + arg plumbing
(~30 lines). *This is IDE.md's "thread the workspace into `run_serve` + `ReplSession`" — the
cheapest first step, done before any UI.*

---

## B2 — `listFiles` verb (host directory walk, sandboxed)

**Goal.** The browser can enumerate the project: files and sub-directories under the root, one
directory level per call (lazy).

**Code-points.**
- `src/repl.rs`: `pub fn list_files(&self, rel: &str) -> Result<Vec<Entry>, String>` — resolve
  `root.join(rel)`, **verify it is under the root** (canonicalise + `starts_with`), `read_dir`,
  return `Entry { name, dir: bool, path: rel/name }`. Skip noise by default (`.git`, `target`,
  `.loft` caches) — a small deny-set. Refuse `..`/absolute/symlink escapes (same discipline as
  `write_file`).
- `src/rpc.rs:149` `handle()`: add arm
  `"listFiles" => { session.list_files(text(&parsed,"path").unwrap_or("")) … }` → reply
  `{ok, entries:[{name,dir,path}]}`. Add a small `entries_json` serialiser (mirror
  `diagnostics_event`).

**Protocol.** `listFiles {path?}` → `{ok, entries:[{name, dir, path}]}` (path omitted = root).

**Gate.** `tests/rpc.rs::rpc_list_files_walks_and_sandboxes` — a fixture tree: list root (see
top-level entries), list a sub-dir (see its children), `listFiles {path:"../.."}` ⇒ error,
absolute path ⇒ error.

**Reuse ↔ build.** Reuse: the `write_file` sandbox pattern (`repl.rs:1730`). Build: `list_files`
+ the verb (~40 lines). *This is IDE.md's specced `listFiles` = "the viewer's tree walk," now in
the gateway.*

---

## B3 — File-tree UI in the shell

**Goal.** A left sidebar renders the tree; clicking a directory expands it (lazy `listFiles`);
clicking a file selects it (loads in B4). Still the inline shell (CM6 is B5).

**Code-points.**
- `src/serve.rs:339` `SHELL_TEMPLATE`: add `<nav id="tree"></nav>` to the grid (a column left of
  the editor; the CSS grid already has named areas — add a `tree` track). Inline JS:
  `loadDir(path, node)` → send `listFiles` → render `<ul><li>` (dir = expandable ▸, file =
  clickable); `onFileClick(path)` sets `currentPath` (B4 wires the load). Send/await helpers
  already exist in the shell's WS plumbing (the request/reply-by-`id` pattern used by Run/Save).

**Protocol.** none new (uses B2's `listFiles`).

**Gate.** headless-chromium smoke (`check_html_bundle`-shape): the shell contains `id="tree"`;
on connect it lists the root; expanding a dir fetches children. The protocol itself is covered by
B2's test.

**Reuse ↔ build.** Reuse: the viewer's `page_tree`/`breadcrumbs` **as the UI reference**
(`tools/viewer/src/main.loft`); serve.rs's WS request helper. Build: the tree JS + CSS column.

---

## B4 — `readFile` + open in the editor + multi-file save (widen the sandbox)

**Goal.** Click a file → its text loads into the editor; edits **save back to disk**; the sandbox
now covers **every file under the root** (not one file). A minimal **tab bar** tracks open files +
per-file dirty state.

**Code-points.**
- `src/repl.rs`:
  - `pub fn read_file(&self, rel) -> Result<String, String>` — under-root check + `read_to_string`.
  - **widen `write_file` (`:1730`)**: instead of equality with a single `workspace_file`, resolve
    the target and require it be **under `workspace_root`**. For a *new* file (doesn't exist yet,
    so `canonicalize` fails) canonicalise the **parent** and join the name, then `starts_with(root)`.
    Keep refusing `..`/symlink/absolute escapes. (Leave `workspace_file` as an optional extra
    pin, or drop it once root is authoritative.)
- `src/rpc.rs:149`: add `"readFile" => …` → `{ok, content}`. `writeFile` (`:189`) is unchanged at
  the protocol layer — it now succeeds for any in-root path via the widened impl.
- `src/serve.rs` shell JS: `openFile(path)` → `readFile` → set editor value, push a tab, track
  `currentPath`; `save()` → `writeFile {path: currentPath, content}` (the dirty flag clears only
  on the `ok` reply — the existing reconciled-save pattern). **Run/compile** now target
  `currentPath` instead of the baked-in `FILE`.

**Protocol.** `readFile {path}` → `{ok, content}`; `writeFile {path, content}` → `{ok}` (existing,
now root-scoped).

**Gate.** `tests/rpc.rs::rpc_read_write_roundtrip_under_root` — read a fixture file; write it
back changed; read again = the change; `readFile`/`writeFile` on `../escape` and an absolute
outside path ⇒ error; **writing a brand-new in-root file succeeds** (the parent-canonicalise
path). `tests/serve.rs::serve_ws_open_edit_save_multifile`.

**Reuse ↔ build.** Reuse: the sandbox mechanism (widen, don't rewrite). Build: `read_file`, the
widened `write_file`, the open/tab/save JS. *This closes WORKBENCH.md seam 1 (single-file → root)
— which IDE.md flags as THE library-dev blocker, so the value lands beyond the editor.*

---

## B5 — Swap the textarea for CodeMirror 6 (served as a static asset)

**Goal.** The editor pane becomes **CM6** with loft syntax highlighting; the frontend moves from
an inline Rust string to a **bundled static asset** served by the gateway. Edit/save/tree/tabs
from B1–B4 keep working, now on CM6.

**Code-points.**
- `src/serve.rs:69` `serve_connection`: add a `GET /static/<name>` route (beside `GET /` at
  `:110`) that serves a bundled `ide.js` / `ide.css` with the right content-type — **embedded via
  `include_str!`** (the same tactic as `tools/fmt/whole.loft`), so there is no separate
  file-serving story and no CDN (offline posture).
- `SHELL_TEMPLATE` (`:339`): replace `<div id="editor"><textarea id="src">…` (`:407`) with
  `<div id="editor"></div>` + `<script type="module" src="/static/ide.js">`; drop the inline
  gutter JS (CM6 owns the gutter).
- New `editors/workbench/` (vendored bundle source): CM6 core + the **loft Lezer grammar** taken
  from @PLN62's `loft-language.js` (`lib_plans/62-web-ide/` design), plus the tree/tabs/console
  logic ported from the inline JS. A tiny build step emits the single `ide.js` that
  `include_str!` embeds (documented; run on change, like `make wasm`).

**Protocol.** none new.

**Gate.** smoke: CM6 mounts on `id="editor"`; a `.loft` buffer highlights keywords/strings;
open→edit→save (B4) still round-trips. `serve_static_serves_ide_bundle` asserts `GET /static/ide.js`
= 200 + `application/javascript`.

**Reuse ↔ build.** Reuse: @PLN62's loft grammar; serve.rs's GET routing. Build: the `/static`
route + the CM6 bundle + mount. *Closes WORKBENCH.md seam 5.* Independent of B6 — a better editor
even before LSP.

---

## B6 — LSP integration (in-process `loft::lsp`): diagnostics · hover · go-to-def · completion

**Goal.** The CM6 editor gets live **diagnostics** (squiggles), **hover** (signature + `///`
doc), **go-to-definition** (jump, cross-file), and **completion** — all computed **in-process**
by `loft::lsp`, on the *live buffer* (unsaved edits included).

**Code-points.**
- `src/rpc.rs:149` `handle()`: four new arms, each a thin call into `loft::lsp` with the buffer
  carried in the request (`text`), the file's rel path as `name`, and the session's stdlib dir:
  | verb | `loft::lsp` call (`src/lsp.rs`) | reply |
  |---|---|---|
  | `lspDiagnostics {file,text}` | `diagnose(text,name,stdlib)` `:62` | `{ok, items:[{line,col,level,message}]}` |
  | `lspHover {file,text,line,col}` | `symbol_at(text,name,stdlib,line,col)` `:163` | `{ok, hover:{markdown, range?}}` |
  | `lspDefinition {file,text,line,col}` | `resolve_at(text,stdlib,line,col)` `:265` | `{ok, location:{file,line,col}}` |
  | `lspComplete {file,text,line,col}` | `complete(text,name,stdlib,line,col)` `:1319` | `{ok, items:[{label,kind,detail}]}` |
  Add a `session.stdlib_dir()` accessor if absent (`ReplSession` already stores it). Optional
  extras, same shape, when wanted: `lspOutline` (`outline` `:107`), `lspFormat`
  (`Formatter::format` `:470`), `lspReferences` (`identifier_refs` `:902`).
- `editors/workbench/ide.js` (the B5 bundle): wire CM6 extensions to the verbs —
  - **linter** (debounced ~300 ms): on change, send `lspDiagnostics` with the current buffer →
    map to CM6 diagnostics → squiggles + gutter marks.
  - **hover tooltip**: `hoverTooltip` → `lspHover` → render the markdown.
  - **go-to-def**: a keymap/`Ctrl`-click command → `lspDefinition` → if another file, `openFile`
    (B4) then set the cursor; else jump in-buffer.
  - **autocomplete**: a CM6 completion source → `lspComplete`.

**Protocol.** the four `lsp*` verbs above (request carries `{file, text, line?, col?}`).

**The model — stateless, buffer-in-request.** Because `loft::lsp` is pure `(text,…) → result`,
the browser sends the live buffer with each request; **no server-side document store, no
didOpen/didChange**. This is correct for unsaved edits and is the whole reason basic-LSP is
small. (A server-side synced buffer + `didChange` is only needed when the *stateful, incremental*
external servers — rust-analyzer — arrive behind the @PLN66 bridge; that is multi-language work,
not B6.)

**Gate.** `tests/rpc.rs::rpc_lsp_basic` — drive each verb over the pipe:
`lspDiagnostics` on a broken buffer ⇒ an error item at the right line; on a clean buffer ⇒ none;
`lspHover` on a known symbol ⇒ non-empty markdown; `lspDefinition` on a call ⇒ the def's line;
`lspComplete` after a prefix ⇒ contains an expected symbol. Smoke: hover a symbol in-browser →
tooltip; Ctrl-click a call → jumps to its def.

**Reuse ↔ build.** Reuse: `loft::lsp::{diagnose, symbol_at, resolve_at, complete}` (shipped,
this session) — the *entire* language brain; CM6's linter/hover/completion extensions. Build: the
four thin verbs (~15 lines each) + the CM6 wiring. *Closes WORKBENCH.md seam 3 (loft leg) with
zero new language logic.*

---

## After B6 — where this sits

B1–B6 deliver a **usable single-language loft editor**: open a project, browse it, edit and save
any file, with live diagnostics/hover/def/completion. It is exactly WORKBENCH.md's **WB0 + WB3 +
the loft half of WB1**, minus the debugger UI (WB2, which re-skins serve.rs's *already-shipped*
breakpoint/step/variables panels onto CM6) and minus multi-language (WB4 adapter refactor →
WB5/WB6 Rust/C behind the bridge).

**Deliberately out of the basic scope** (each its own later step): the debugger panels (WB2), the
Markdown preview + git panel (WB8, both reuse the viewer), the live game + world editor (WB9), and
anything requiring an external language server (the bridge).

## Test + build discipline

- Every B-step lands with a `tests/rpc.rs` (drive the verb over a pipe, assert JSON) and/or
  `tests/serve.rs` (over a real socket) test **before** it is called done — the shipped pattern.
- The CM6 bundle (B5) has a documented build step; changing `ide.js` means rebuilding what
  `include_str!` embeds, then the smoke test — same "rebuild the binary to test embedded assets"
  rule as `tools/fmt/whole.loft`.
- `make ci` (fmt → clippy → test) gates each landing; both backends are irrelevant here (the IDE
  is interpreter-side tooling), but the LSP verbs must not regress the existing `loft-lsp` tests.

## Code-point index (quick reference)

- `src/serve.rs` — `run_serve:34`, `serve_connection:69` (GET `/`:110, 404:118 → add `/static`),
  `ws_protocol_loop:140` (dispatch `rpc::handle`:153), `render_shell:332`, `SHELL_TEMPLATE:339`,
  `<textarea>:407`, `const FILE:426`.
- `src/rpc.rs` — `handle:149` (verbs: `launch:158`, `writeFile:189`, `compile:199`); add
  `listFiles`/`readFile`/`lsp*` arms.
- `src/repl.rs` — `workspace_file:1194`, `set_workspace_file:1718`, `write_file:1730`; add
  `workspace_root`/`set_workspace_root`/`list_files`/`read_file`, widen `write_file`.
- `src/lsp.rs` — `diagnose:62`, `outline:107`, `symbol_at:163` (hover), `resolve_at:265` (def),
  `Formatter::format:470`, `identifier_refs:902`, `complete:1319`.
- Grammar donor — `lib_plans/62-web-ide/` `loft-language.js` (loft Lezer/CM6 grammar).
