<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 03 — Minimal markdown subset

**Status:** Shipped 2026-05-13 (after the seven-bug native
arc @P262→@P269 cleared the underlying loft compiler issues
that blocked the first attempt).

## What shipped

A single-pass markdown renderer in
`tools/viewer/src/main.loft` (~250 lines added, no separate
module — kept inline to dodge multi-file complexity for v1):

- **Headings** `#` through `######` with GitHub-compatible
  ASCII slug ids (`<h2 id="open-work">…</h2>`).
- **Paragraphs** with blank-line separation; multi-line
  paragraphs concatenate with single spaces.
- **Fenced code blocks** ` ``` ` with optional language tag
  → `<pre><code class="language-…">…</code></pre>`,
  HTML-escaped, no syntax highlighting in v1.
- **Inline code** `` `text` `` → `<code>…</code>`.
- **Bold** `**text**` and **italic** `*text*` / `_text_`
  with the smart-`_` heuristic so identifiers like
  `snake_case` aren't treated as italic.  (Inner text
  rendered as HTML-escaped passthrough — no nested inline
  in v1; see § Risks.)
- **Links** `[text](url)` with relative-path resolution
  against the current file's directory.  Absolute URLs
  (`http`, `https`, `mailto`, `#anchor`) pass through.
  Relative `.md` links route to `/file/<resolved>`; `../`
  segments resolve correctly.  Anchor fragments preserved.
- **Horizontal rules** `---` / `***` / `___` → `<hr>`.
- **HTML comments** `<!-- … -->` (single-line) — stripped.

End-to-end verified on 2026-05-13:

- README.md: 2 H1 + 9 H2 + 1 H3 (matches structure).
- doc/claude/PROBLEMS.md (216 KB): 2 H1, 8 H2, 10 H3, 20 H4,
  3 H5 — full nested heading hierarchy preserved.
- Cross-doc links (`<a href="/file/...">`) resolve and route
  through the existing `/file/<path>` handler.

## Bugs surfaced + filed (dogfood-discovery)

Building this renderer surfaced two new loft compiler bugs,
filed in PROBLEMS.md per the bug-filing policy:

- **@P270** — Parser rejects `len(text_var)` in some in-context
  shapes ("Unknown function len" error pointing at wrong
  source position).  Workaround: method form `text_var.len()`.
  Minimal repro doesn't reproduce in isolation — surfacing
  needs the surrounding viewer context.
- **@P271** — Codegen panic "Too few parameters on n_<helper>
  (got 3, need 4)" when a text-returning helper is called
  from inside another text-returning fn.  Workaround: inline
  the helper's body at the call site.  Same shape works in
  many other places in the same file; minimal repro doesn't
  reproduce.

Both filed per the [Dogfood discovery](../../../../.claude/projects/-home-ubuntu-loft/memory/feedback_dogfood_discovery.md) <!--noindex-->
principle: real loft tools surface bugs synthetic tests miss.

## Original plan

## Goal (original)

Render `.md` files to readable HTML.  Cover the constructs
that loft's docs actually use; explicitly defer tables to
phase 06 (the user's stated pain point with current tools is
poor table rendering — fixing that properly belongs in its
own focused phase).

The output of this phase is a **navigable doc browser** for
`doc/claude/`: PROBLEMS.md, PLANNING.md, all plans, all
lib_plans render with cross-doc links resolved.

## Subset specification

### Block-level (in v1)

| Construct | Markdown syntax | HTML output |
|---|---|---|
| Heading 1-6 | `# H1` … `###### H6` | `<h1 id="<slug>">…</h1>` … `<h6 id="<slug>">…</h6>` |
| Paragraph | `text\n\ntext` | `<p>…</p>` |
| Unordered list | `- item` / `* item` | `<ul><li>…</li></ul>` |
| Ordered list | `1. item` | `<ol><li>…</li></ol>` |
| Fenced code block | ` ```lang\ncode\n``` ` | `<pre><code class="language-<lang>">…</code></pre>` (HTML-escaped, no syntax highlighting in v1) |
| Block quote | `> text` | `<blockquote>…</blockquote>` |
| Horizontal rule | `---` | `<hr>` |
| HTML comment | `<!-- … -->` | Stripped from output |

### Inline (in v1)

| Construct | Markdown syntax | HTML output |
|---|---|---|
| Bold | `**text**` | `<strong>text</strong>` |
| Italic | `*text*` / `_text_` | `<em>text</em>` |
| Inline code | `` `text` `` | `<code>text</code>` |
| Link | `[text](url)` | `<a href="<rewritten>">text</a>` |
| Image | `![alt](url)` | `<img src="<rewritten>" alt="alt">` |
| Auto-link | `<https://…>` | `<a href="https://…">https://…</a>` |
| Line break | trailing `  \n` | `<br>` |

### Explicitly NOT in v1 (deferred)

- **Tables** — the marquee feature; gets phase 06 to itself.
- **Nested lists** — defer; current loft docs don't nest more
  than one level (check before promoting to v2 if so).
- **Definition lists** — not used in loft docs.
- **Footnotes** — not used.
- **Mermaid / math** — not used.
- **Strikethrough (`~~text~~`)** — could land if cheap; ship
  if it's <10 lines of parser code.
- **Task lists (`- [x]`)** — same: ship if cheap.

## Architecture

### Module: `tools/viewer/src/markdown.loft`

Single-pass character scanner.  No regex (loft doesn't have
one yet).  ~250 lines.  Two phases internally:

1. **Block scanner** — split source into block tokens:
   `Heading(level, text)`, `Para(text)`, `List(items)`,
   `CodeBlock(lang, content)`, `BlockQuote(text)`, `Hr`,
   `Comment`.
2. **Inline renderer** — for each block's text content, apply
   inline rules to produce HTML.

Both phases consume `text` and produce `text`; no AST node
allocation.

### Cross-doc link rewriting

A `[text](path.md)` link where `path.md` resolves to a file <!--noindex-->
under the project root becomes `<a
href="/file/<resolved-path>">text</a>`.  Anchor fragments
(`path.md#section`) are preserved.

Rules:
- Absolute URLs (`https://`, `http://`, `mailto:`) pass
  through unchanged.
- Relative paths starting with `.` resolve against the
  current file's directory.
- Paths with `.md` suffix route to `/file/`.
- Paths without `.md` (e.g., `[link](src/parser/foo.rs)`)
  also route to `/file/` — works because phase 02 handles
  any file extension.
- Unresolvable paths render as plain text with a `class="broken-link"`
  CSS class for visual signalling (red underline).

### GitHub-compatible heading slugs

Phase 03 ships its own slugger to match GitHub's rules:

```loft
pub fn slugify(heading: text) -> text {
    out = "";
    for c in heading.lowercase() {
        if c == ' ' { out += "-"; }
        else if (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '-' || c == '_' {
            out += "{c}";
        }
        // skip everything else (punctuation, unicode)
    }
    out
}
```

Validation: hand-check 5 known headings against GitHub's
rendered output.  Examples:

- `## Open work` → `open-work`
- `## P259 (closed)` → `p259-closed`
- `## What ships` → `what-ships`
- `## Drivers — context` → `drivers--context`
- `## §1.2 — Foo` → `12--foo`

(GitHub's rules are slightly more nuanced for unicode; v1
gates on the ASCII subset, which covers all loft docs.)

### Code-block language tags

Lines like ` ```rust ` start a fenced code block tagged
`rust`; closing ` ``` ` ends it.  Content is HTML-escaped
and rendered with `<pre><code class="language-rust">…</code></pre>`.

Phase 03 doesn't highlight; the class hook lets a future
phase add highlighting via CSS-only or a JS library
(deliberately not added in v1).

### Render entry point

```loft
pub fn render(source: text, current_file_path: text) -> text {
    blocks = scan_blocks(source);
    body = "";
    for blk in blocks {
        body += render_block(blk, current_file_path);
    }
    body
}
```

The `current_file_path` is used for relative-link resolution.

## Critical files

| Path | Action |
|---|---|
| `tools/viewer/src/markdown.loft` | NEW (~250 lines) |
| `tools/viewer/src/route.loft` | UPDATED: page_file() routes `.md` to `markdown.render(...)` instead of phase 02's stub |
| `tools/viewer/src/style.loft` | UPDATED: add `.broken-link`, `<pre><code>`, `<blockquote>` styling |

## Existing functions / tooling to reuse

- **`html.escape`** from phase 02.
- **`text` iteration** for the character scanner.
- **`default/03_text.loft`** for `find`, `replace`,
  `starts_with`, `ends_with`, `trim`.

## Test surface

Acceptance is end-to-end against real loft docs.  Test files:

- `doc/claude/PROBLEMS.md` (192 KB, heavy use of headings,
  tables, inline code, links)
- `doc/claude/plans/finished/22-mutable-closures/README.md`
  (cross-doc link heavy)
- `doc/claude/plans/35-branch-review-viewer/README.md` (this
  plan's README — meta-test)
- `default/02_images.loft` — wait, that's `.loft`, skip;
  use `lib/server/README.md` instead.

Per-construct unit tests in `tools/viewer/tests/markdown_test.loft`:

```loft
fn test_heading() {
    h = markdown.render("# Hello\n\nWorld\n", "test.md");
    assert(h.contains("<h1 id=\"hello\">Hello</h1>"));
    assert(h.contains("<p>World</p>"));
}

fn test_link_rewrite() {
    h = markdown.render("[doc](other.md)\n", "doc/foo.md");
    assert(h.contains("<a href=\"/file/doc/other.md\">doc</a>"));
}

// ... ~15-20 cells covering each construct
```

## Verification

End-to-end smoke against `demo_dev` branch:

```bash
$ make view-build && make view &
$ curl -s http://localhost:8765/file/doc/claude/PROBLEMS.md > /tmp/p.html
$ grep -c '<h2 id=' /tmp/p.html       # ≥ 5 headings
$ grep -c '<a href="/file/' /tmp/p.html   # cross-doc links rewritten
$ grep -c '<pre><code class="language-' /tmp/p.html   # fenced code blocks
$ # No raw '##' or '**' should leak through:
$ grep -E '^##|\*\*[^<]' /tmp/p.html | head
# (should be empty or only inside <pre> blocks)
```

Visual: open `http://localhost:8765/file/doc/claude/PROBLEMS.md`
in browser.  Should be readable: clear headings, fenced code
blocks, links resolve, broken-link sentries highlighted.

## Risks

| Risk | Mitigation |
|---|---|
| Markdown subset misses something the loft docs actually use | Run the renderer against ALL `.md` files in `doc/claude/` once and audit the visual output; add missing constructs in a follow-up commit before phase 04 |
| Slug rule diverges from GitHub for unusual headings | Document the differences; if a real cross-doc anchor breaks, file as a viewer P-issue |
| 192 KB doc takes >1 sec to render | Cache rendered HTML by `(path, mtime)`; invalidate on next request when mtime changes.  Cache is in-memory only (no persistence across binary restarts) |
| Inline parser is fragile (nested `**` and `*`, code spans inside links) | v1 ships a deliberately-conservative parser; surprising input renders verbatim rather than crashing |
| Loft text iteration is slow per character | If benchmarks show this, switch to chunked `find`/`split` patterns; flag as a loft-text-API enhancement |

## Cross-references

- [Phase 02 — code files](02-code-files.md) — provides `escape()` and the file-route handler
- [Phase 06 — proper tables](06-tables-design.md) — the
  forward-looking phase that completes markdown coverage
- [`default/03_text.loft`](../../../../../default/03_text.loft) — text manipulation API
- [GitHub's slugger reference](https://github.com/Flet/github-slugger) — the rules our slugify() emulates
