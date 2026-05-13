<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 01 — HTTP server + static + project tree

**Status:** Open

## Goal

Wire `lib/server` into the viewer.  Serve the project tree
over HTTP: a landing page lists top-level directories
(`doc/`, `lib/`, `src/`, `tests/`, `tools/`); clicking into
any directory shows its contents; clicking a file returns its
raw bytes (no rendering yet — that's phases 02 and 03).

The output of this phase is a **working HTTP file browser**
for the loft repo.  Not pretty, not git-aware — but the
server is alive, the routing works, and the user can navigate
the tree from a browser.

## What ships

### Routes

| Method | Path | Handler |
|---|---|---|
| GET | `/` | Landing page: branch name placeholder + tree of top-level dirs |
| GET | `/tree/<path>` | Directory listing (linked entries, with "../" parent link) |
| GET | `/raw/<path>` | Raw file bytes with `Content-Type: text/plain; charset=utf-8` |
| GET | `/static/style.css` | Embedded base CSS (single string constant) |
| GET | `*` | 404 with a link back to `/` |

### Project-root resolution

The binary looks for the loft repo root in this order:
1. `LOFT_VIEW_ROOT` env var (absolute path).
2. Current working directory containing a `Cargo.toml` with
   `name = "loft"` (sniff first 100 lines).
3. Fallback: refuse to start with a clear "set LOFT_VIEW_ROOT
   to the loft repo root" message.

### Tree walking

A small loft module under `tools/viewer/src/tree.loft`:

```loft
struct DirEntry {
    name: text,
    path: text,         // relative to project root
    is_dir: boolean,
    size: integer       // bytes; 0 for dirs
}

fn list_dir(rel_path: text) -> vector<DirEntry> { ... }
fn entry_html(e: DirEntry) -> text { ... }
```

`list_dir` uses the `File` / `files()` primitives from
`default/02_images.loft` (the canonical loft fs API).
Skip-list: hardcoded for v1 — `.git/`, `target/`, `node_modules/`,
`.cache/`, `tools/viewer/state/`.

### Path-traversal guard

Every incoming `/raw/<path>` and `/tree/<path>` resolves
against the project root.  If the resolved path escapes the
root (`../../../etc/passwd`), respond 403 with `forbidden:
path escapes project root`.

Loft's `File` type is sandboxed (per the Explore agent's
recon), so this is belt-and-braces — but cheap to add and
documents the invariant.

### Embedded CSS (v1 baseline)

`tools/viewer/src/style.loft` exports a single `BASE_CSS:
text` constant — handwritten ~120-line stylesheet:

- Two-column layout: sidebar (file tree) + main content.
- Light mode and dark mode via `@media prefers-color-scheme`.
- Monospace font for code; sans-serif for prose.
- Sticky header with branch name + page title.
- No JavaScript dependencies.

CSS is a string constant in the binary, served at
`/static/style.css` with `Content-Type: text/css`.  Cached
indefinitely (the CSS only changes when the binary is
rebuilt).

### Server entry point

`tools/viewer/src/main.loft` becomes:

```loft
use server;
use tree;
use style;

fn main() {
    port = 8765;          // TODO read from env in phase 04
    bind = "0.0.0.0";
    srv = server.listen(port);
    print("loft-view: listening on http://{bind}:{port}/\n");
    for req in srv {
        route(req);
    }
}

fn route(req: Request) {
    path = req.path;
    if path == "/" {
        respond_html(req, page_landing());
    } else if path.starts_with("/tree/") {
        rel = path.substring(6);
        respond_html(req, page_tree(rel));
    } else if path.starts_with("/raw/") {
        rel = path.substring(5);
        respond_raw(req, rel);
    } else if path == "/static/style.css" {
        respond_css(req, BASE_CSS);
    } else {
        respond_404(req, path);
    }
}
```

(Exact API names per `lib/server/src/server.loft`; this is the
shape, not verbatim.)

## Critical files

| Path | Action |
|---|---|
| `tools/viewer/src/main.loft` | UPDATED: server entry + routing |
| `tools/viewer/src/tree.loft` | NEW: dir walking + DirEntry rendering |
| `tools/viewer/src/style.loft` | NEW: BASE_CSS constant |
| `tools/viewer/src/route.loft` | NEW: per-route page builders (page_landing, page_tree, page_raw, page_404) |

## Existing functions / tooling to reuse

- **`lib/server/src/server.loft`** — `listen(port)`,
  `respond_html(req, body)`, `respond_css(req, body)`,
  `respond_typed(req, status, body, content_type)`, the
  `for req in srv` iterator pattern.
- **`default/02_images.loft`** — `File` type and
  `files(directory: File) -> vector<File>` for fs walking.
- **`default/03_text.loft`** — `find`, `replace`, `substring`,
  `starts_with`, `ends_with`, `trim`, `split`, `join` for
  path / URL manipulation.
- **No external deps required** for this phase.

## Test surface

- `make view-build && make view` starts the server, prints
  `loft-view: listening on http://0.0.0.0:8765/`.
- `curl -s http://localhost:8765/` returns HTML with a tree
  of top-level dirs.
- `curl -s http://localhost:8765/tree/doc/claude/` returns
  HTML listing `PROBLEMS.md`, `PLANNING.md`, etc.
- `curl -s http://localhost:8765/raw/Cargo.toml` returns the
  raw `Cargo.toml` text.
- `curl -s http://localhost:8765/raw/../../../etc/passwd`
  returns 403.
- `curl -s http://localhost:8765/static/style.css` returns
  the CSS with `Content-Type: text/css`.
- Browser at `http://localhost:8765/` (via SSH port-forward)
  shows the tree, clicking entries navigates correctly.

## Verification

End-to-end on the current `demo_dev` branch:

```bash
$ make view-build && make view &
$ sleep 1
$ curl -sI http://localhost:8765/ | head -3
HTTP/1.1 200 OK
Content-Type: text/html; charset=utf-8

$ curl -s http://localhost:8765/tree/doc/claude/ | grep -c PROBLEMS.md
1

$ curl -s http://localhost:8765/raw/Cargo.toml | head -1
[workspace]

$ kill %1
```

## Risks

| Risk | Mitigation |
|---|---|
| `lib/server` API doesn't expose what's needed (e.g. content-type header on raw response) | Surface as a viewer-driven enhancement — file as a row in `lib_plans/future/08-server/README.md` with this plan as the driver |
| File reading on huge files (192 KB PROBLEMS.md) is slow | Defer optimisation to phase 03 (markdown rendering); raw file serving is fine |
| Tree walk is recursive, could blow stack on deep dirs | Iterative walk; cap depth at 20 (well above any real loft tree depth) |
| Browser caches stale tree after the user `git pull`s | Send `Cache-Control: no-store` on tree pages; CSS cached aggressively, content not |

## Cross-references

- [`lib/server/src/server.loft`](../../../../lib/server/src/server.loft)
- [`default/02_images.loft`](../../../../default/02_images.loft) — File / files() API
- [Phase 02 — code-file rendering](02-code-files.md) — next: render `<pre>` + line numbers
- [Phase 03 — markdown subset](03-markdown-minimal.md) — render `.md` to HTML
