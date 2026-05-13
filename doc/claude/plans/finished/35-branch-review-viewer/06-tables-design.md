<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 06 — Proper GFM tables

**Status:** Open

## Goal

Ship table rendering that the user actually wants — column
alignment, multi-line cells, escaped pipes, inline formatting
within cells, wide-table handling.  This is the **marquee
phase** of the viewer: the user explicitly named bad table
rendering as the pain point that current tools fail on.

The output of this phase is **GFM-compatible table rendering**
(matching GitHub's behaviour) that handles every table in the
loft `doc/claude/` corpus correctly.

## Why this gets its own phase

Tables are the highest-value markdown feature for loft's docs:

- `PROBLEMS.md` is 90% tables (the open-issues quick-reference,
  the catalogue rows, etc.).
- Every plan README has a phase table.
- `ROADMAP.md` is dense tables.
- `STDLIB.md`, `LOFT.md`, `DEBUG.md` lean on tables for
  operator references and quick-reference grids.

Bad table rendering is the single biggest reason the user
finds existing tools insufficient.  Phase 03's "minimal
subset" deliberately deferred tables; this phase does them
right.

## What ships

### GFM table syntax (full support)

```markdown
| Header 1 | Header 2 | Header 3 |
|----------|:---------|---------:|
| Cell A   | Cell B   |        9 |
| `code`   | **bold** |   *em*   |
| Multi    | Cell C   | Cell D   |
| line\    |          |          |
| cell     |          |          |
```

Required behaviour:

| Feature | v1 (this phase) |
|---|---|
| Column count detection from header row | Yes |
| Alignment from separator row (`:---`, `---:`, `:---:`) | Yes |
| Inline formatting in cells (bold, italic, code, links) | Yes — reuses the inline renderer from phase 03 |
| Escaped pipes (`\|`) in cells | Yes |
| Trailing-pipe-optional (GitHub allows omitting trailing `\|`) | Yes |
| Multi-line cells via `<br>` (line continuation with `\`) | Yes — single-line cells with `<br>` for breaks; no rowspan |
| Empty cells | Yes — render `<td></td>` |
| Mismatched row column counts | Pad shorter rows with empty cells; don't error |
| Wide tables (>10 cols, >100 rows) | Render with horizontal scroll |
| Tables inside lists / blockquotes | Yes |

### Out of scope even for this phase

- Rowspan / colspan via `<td rowspan="2">` — not GFM, not used.
- HTML inside cells beyond what the inline renderer
  produces — defer.
- Sticky column headers on scroll — JS-required; v1 ships
  without.

### Architecture

`tools/viewer/src/markdown.loft`'s block scanner gains a
`Table(rows, alignments)` block variant.  Tables are detected
by the regex-free pattern:

1. Line N starts with `|` and contains another `|`.
2. Line N+1 matches the separator pattern: each cell is
   `:?-+:?` (optional colon + dashes + optional colon).
3. Lines N+2..M (until a blank line or non-`|` line) are
   data rows.

A new module `tools/viewer/src/table.loft` (~150 lines)
handles parsing + rendering:

```loft
struct Table {
    headers: vector<text>,           // raw cell content (post-pipe-split)
    alignments: vector<Alignment>,   // per column: Left | Center | Right | Default
    rows: vector<vector<text>>       // 2D: rows × cells
}

enum Alignment { Default, Left, Center, Right }

pub fn parse_table_block(lines: vector<text>) -> Table { ... }
pub fn render(t: Table) -> text { ... }   // returns <table>...</table>
```

`render` produces:

```html
<table>
  <thead>
    <tr><th style="text-align: left">Header 1</th>...</tr>
  </thead>
  <tbody>
    <tr><td>...</td>...</tr>
  </tbody>
</table>
```

Inline formatting in cells is applied by passing each cell's
text through the **inline-only** entry point of phase 03's
markdown renderer (`render_inline(s) -> text`).  This needs
phase 03 to expose `render_inline` as a public function.

### CSS

Tables get dedicated styling:

```css
table {
    border-collapse: collapse;
    margin: 1em 0;
    overflow-x: auto;       /* wide-table horizontal scroll */
    display: block;
    max-width: 100%;
}
th, td {
    border: 1px solid var(--border);
    padding: 0.4em 0.8em;
    vertical-align: top;
}
th {
    background: var(--header-bg);
    font-weight: 600;
}
tr:nth-child(even) td { background: var(--zebra); }
```

Light-mode + dark-mode variables; readable zebra striping.
Wide tables become horizontally scrollable inside the page
without breaking the layout.

### Test corpus

A regression matrix in `tools/viewer/tests/table_test.loft`
covering every shape from `doc/claude/`:

- `PROBLEMS.md` open-issues table (5 columns, ~20 rows).
- `ROADMAP.md` phase tables (4 columns each).
- Plan-22 README phase table (5 columns).
- `STDLIB.md` operator tables (3 columns, dense).
- A synthetic wide table (15 columns) for scroll behaviour.
- A synthetic table with code spans, bold, links, escaped
  pipes in cells.
- A table with mismatched row column counts.
- A table inside a `> blockquote`.

Each test asserts the rendered HTML against a golden — not
byte-perfect (CSS class names can drift), but structurally
(correct number of `<tr>`s, correct alignment attributes,
expected text in cells).

## Critical files

| Path | Action |
|---|---|
| `tools/viewer/src/markdown.loft` | UPDATED: detect `Table` block, expose `render_inline` |
| `tools/viewer/src/table.loft` | NEW (~150 lines) |
| `tools/viewer/src/style.loft` | UPDATED: full table CSS |
| `tools/viewer/tests/table_test.loft` | NEW (~10 cells) |

## Existing functions / tooling to reuse

- **Phase 03's `render_inline`** for cell content formatting.
- **`text.split('|')`** for cell separation (with escape
  handling).
- **`html.escape`** (phase 02) for cell text safety.

## Test surface

- All 10 cells in `tools/viewer/tests/table_test.loft` green.
- Browser smoke: open `http://localhost:8765/file/doc/claude/PROBLEMS.md`,
  the open-issues table renders with proper alignment, code
  in cells highlighted, links in cells clickable.
- Browser smoke: open `http://localhost:8765/file/doc/claude/STDLIB.md`,
  operator tables render dense + readable.
- Browser smoke: synthetic wide-table page scrolls
  horizontally without breaking sidebar layout.

## Verification

End-to-end against `doc/claude/`:

```bash
$ # Audit all tables across doc/claude/ render correctly
$ for f in $(find doc/claude -name "*.md"); do
    curl -s "http://localhost:8765/file/$f" > /tmp/page.html
    raw_table_count=$(grep -c '^|.*|' "$f")
    rendered_count=$(grep -c '<table>' /tmp/page.html)
    [ "$raw_table_count" -gt 0 ] && echo "$f: raw=$raw_table_count rendered=$rendered_count"
  done
# (counts won't match exactly because separator rows aren't tables,
#  but every file with raw tables should have ≥ 1 rendered table)
```

Visual: open the dashboard, click into PROBLEMS.md, scroll to
the open-issues quick-reference table — should look better
than `make serve`'s plain-text fallback (currently the user's
baseline pain point).

## Risks

| Risk | Mitigation |
|---|---|
| Inline parser invoked per-cell is slow on dense tables | Cache rendered HTML per (table, content-hash) within a render pass |
| Multi-line cell handling is fragile (continuation backslash + line breaks) | Document the supported subset; pin via tests; fall back to single-line for unsupported shapes |
| GFM compliance has edge cases (empty header, no separator row) | Document the strict-subset interpretation; surprising input renders as `<pre>` text, not crash |
| Wide tables overflow + sidebar layout fights | CSS `overflow-x: auto` on the table; don't propagate to the page-level scroll |
| Phase blocks viewer release | Phase 06 can split into `06a — basic GFM` (no multi-line cells, no inline-in-cells) and `06b — full polish` if shipping needs the basic version sooner |

## Forward-looking — what comes after tables

Once tables ship, the viewer's markdown coverage is "good
enough for loft docs."  Open follow-ups (none blocking
viewer release):

- **Promote markdown engine to `lib/markdown/`** if other loft
  programs want it.  This would be a separate plan.
- **Syntax highlighting in code blocks** — see phase 02's
  forward-looking section.
- **Mermaid / math** — only if loft docs start using them.

## Cross-references

- [Phase 03 — minimal markdown](03-markdown-minimal.md) —
  must expose `render_inline` for cell formatting
- [Phase 07 — closeout](07-closeout.md) — markdown coverage
  audit happens here
- [`doc/claude/PROBLEMS.md`](../../PROBLEMS.md) — the
  table-heavy worst-case file this phase must handle
- [GFM spec § Tables (extension)](https://github.github.com/gfm/#tables-extension-) — reference compliance target
