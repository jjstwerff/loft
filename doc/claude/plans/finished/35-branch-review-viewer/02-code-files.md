<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 02 — Code-file rendering with `<pre>` + line numbers

**Status:** **Shipped 2026-05-13** (interp-mode; same native
blockers as phase 01).

## What actually shipped

- `GET /file/<path>` route renders any text file as
  line-numbered HTML.
- Each line wrapped in `<a id="L<n>" class="line">`; `#L<n>`
  fragment scrolls + highlights via CSS `:target` (yellow
  on light, dark purple on dark).
- HTML escape (`&`, `<`, `>`, `"`) plus tab → 4 spaces in a
  separate `escape_with_tabs()` helper.
- Light skip-list of binary file extensions (`.png` /
  `.jpg` / `.gif` / `.webp` / `.ico` / `.pdf` / `.zip` /
  `.gz` / `.tar` / `.wasm`) renders a "Binary file (N
  bytes)" stub with a download link instead of trying to
  read bytes as text.
- Markdown files (`.md`) render as code with a "Markdown
  rendering arrives in @PLAN35 phase 03" banner.
- Tree pages now link files to `/file/<path>` (rendered
  view); `/raw/<path>` still available from each file page.
- CSS extended with `pre.code-pre`, `a.line`, `.lineno`,
  `:target` — light + dark.

End-to-end on the current `demo_dev` branch:

```
$ curl -s http://localhost:8765/file/Cargo.toml | grep '<a id="L1"'
<a id="L1" class="line"><span class="lineno">1</span><span class="src">[package]</span></a>

# 192 KB PROBLEMS.md → 359 KB rendered HTML → 113 ms response.
```

Browser fragment test: navigating to
`http://localhost:8765/file/src/parser/vectors.rs#L1060`
scrolls to + highlights line 1060.

## Native blockers — same as phase 01

@P262 + @P263 still gate native compilation; phase 02 stays in
interpreter mode.  No new native-codegen quirks surfaced.

## Forward-looking — syntax highlighting (still not in v1)

Phase 02 ships unstyled `<pre><code>` lines.  Highlighting
is deferred per the original design — no new ground broken
here.  Either a loft-native lexer (drives a future
`lib/syntax/` library) or a pre-process via external tool
(needs the subprocess primitive loft doesn't have yet).
Either is a separate phase.

---

## Goal

Replace phase 01's "raw bytes" file view with a properly
rendered code page: HTML-escaped contents wrapped in `<pre>`,
line-numbered with `<a id="L<n>">` anchors so the user can
deep-link to a specific line.  No syntax highlighting in v1
(deferred to a later phase or post-shipping enhancement).

The output of this phase is a **review-quality code viewer**
for any file under the project root.

## What ships

### Routes

| Method | Path | Handler |
|---|---|---|
| GET | `/file/<path>` | Renders the file with line numbers + escape; supports `#L<n>` fragment for line scroll |
| GET | `/raw/<path>` | (Unchanged from phase 01) Raw bytes |

The `/file/` route becomes the default; the sidebar links and
tree-page entries route here.  `/raw/` is kept for "give me
the bytes" use cases (download, copy-paste).

### Rendering pipeline

1. Read the file as text via the `File` primitive.
2. Detect line endings (`\r\n`, `\n`, `\r`); normalize to `\n`.
3. Split on `\n` into lines.
4. For each line:
   - HTML-escape: `&` → `&amp;`, `<` → `&lt;`, `>` → `&gt;`,
     `"` → `&quot;`.  Tab handling: render as 4 spaces (or 8;
     pick one for v1, document).
   - Wrap in `<a id="L<n>" class="line"><span class="lineno">{n}</span><span class="src">{escaped}</span></a>`.
5. Wrap the lot in `<pre class="code-pre"><code>…</code></pre>`.
6. Embed in the standard layout (sidebar + breadcrumbs +
   header).

### HTML-escape helper

`tools/viewer/src/html.loft`:

```loft
pub fn escape(s: text) -> text {
    out = "";
    for c in s {
        if c == '&' {
            out += "&amp;";
        } else if c == '<' {
            out += "&lt;";
        } else if c == '>' {
            out += "&gt;";
        } else if c == '"' {
            out += "&quot;";
        } else {
            out += "{c}";
        }
    }
    out
}
```

Loft has no built-in `html.escape` (per the recon); this is
the smallest possible custom function.  Drives a future
`lib/text/html_escape` if patterns repeat.

### Line-anchor format

Format: `#L<lineno>` for a single line (`#L42` jumps to line
42).  Format: `#L<a>-L<b>` for a range (`#L42-L50`); range
support is v1.5 polish — single-line is enough for v1.

CSS adds `:target` highlighting:

```css
a.line:target { background: yellow; }
@media (prefers-color-scheme: dark) {
    a.line:target { background: #604; }
}
```

### Tab + long-line handling

- Tabs → 4 spaces.  Document the choice in the page header
  (`# loft-view tab-width: 4 spaces`).  Future env var
  `LOFT_VIEW_TABS=8` if anyone cares.
- Long lines: `<pre>` with `white-space: pre`; horizontal
  scroll on long lines.  Don't word-wrap (changes line
  numbers visually).

### File-extension detection

`tools/viewer/src/file_kind.loft` — small classifier:

```loft
enum FileKind {
    Markdown,           // .md → render in phase 03
    Code,               // .rs .loft .toml .py .sh .json → this phase
    PlainText,          // unknown extension → this phase as fallback
    Binary              // detected by null-byte scan; render as "binary file (1234 bytes)"
}

pub fn classify(path: text) -> FileKind { ... }
```

Phase 02 routes Markdown to a "stub" page that says "phase 03
will render this"; Code and PlainText use the new pretty
renderer; Binary shows the size + a download link.

## Critical files

| Path | Action |
|---|---|
| `tools/viewer/src/main.loft` | UPDATED: route `/file/<path>` |
| `tools/viewer/src/route.loft` | UPDATED: page_file() handler |
| `tools/viewer/src/html.loft` | NEW: escape() helper |
| `tools/viewer/src/file_kind.loft` | NEW: classify() |
| `tools/viewer/src/code_render.loft` | NEW: render_code(text, kind) -> text (HTML body) |
| `tools/viewer/src/style.loft` | UPDATED: add `.code-pre`, `.line`, `.lineno`, `:target` styles |

## Existing functions / tooling to reuse

- **`File` primitive** for reading file bytes.
- **`text` iteration** (`for c in text { … }`) for the escape
  loop.
- **No external deps** — fully loft-native.

## Test surface

- `curl -s http://localhost:8765/file/Cargo.toml | grep -c lineno` ≥ 50 (matches one per line in Cargo.toml).
- `curl -s http://localhost:8765/file/src/parser/vectors.rs#L1060`
  scrolls to line 1060 (verify visually in browser).
- `curl -s http://localhost:8765/file/src/parser/vectors.rs |
  grep -c '&lt;'` > 0 (proves HTML escape ran on `<` chars).
- `curl -s http://localhost:8765/file/some-binary.bin` returns
  the "binary file" message, not gigabytes of garbage HTML.
- `curl -s http://localhost:8765/file/doc/claude/PROBLEMS.md`
  returns the "phase 03 stub" page (PROBLEMS.md is a `.md`).

## Verification

```bash
$ curl -s http://localhost:8765/file/Cargo.toml > /tmp/page.html
$ grep '<a id="L1"' /tmp/page.html
<a id="L1" class="line"><span class="lineno">1</span><span class="src">[workspace]</span></a>

$ grep '<a id="L42"' /tmp/page.html | head -1
# verifies line 42 got an anchor

# Browser test: open http://localhost:8765/file/src/parser/vectors.rs#L1060
# Expected: page scrolls so line 1060 is visible AND highlighted (yellow on light, purple on dark)
```

## Risks

| Risk | Mitigation |
|---|---|
| Per-character HTML escape is slow on 192 KB files | Acceptable for v1; if benchmark shows >1 sec, write a chunked escape that uses `find` + `replace` on `&`, `<`, `>` separately |
| Binary detection (null-byte scan) is slow | Scan only first 512 bytes; if any nulls → binary |
| Newline normalisation loses CRLF info | Acceptable — viewing is read-only |
| Files outside the project root through symlinks | Same path-traversal guard as phase 01 |

## Forward-looking — syntax highlighting (NOT in v1)

Two paths if the user wants highlighting later:

1. **Loft-native lexer** — small per-language tokenisers
   (Rust, loft, TOML, Python, JSON).  ~200 lines per
   language.  Drives a future `lib/syntax/` library.
2. **Pre-process via external tool** — wrap pygments /
   `bat` in the refresh script; cache pre-rendered HTML.
   Requires subprocess primitive.

Either is a separate phase if it surfaces as needed.  The
plain `<pre>` view is enough for code review.

## Cross-references

- [Phase 01 — HTTP routes](01-http-routes.md) — feeds this
  phase the route handler infrastructure
- [Phase 03 — markdown subset](03-markdown-minimal.md) — what
  fires when `classify(path) == FileKind.Markdown`
- [`default/03_text.loft`](../../../../../default/03_text.loft) —
  text iteration / replace primitives
