<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 2 — Pretty renderer

Status: open

## Goal

Render every `DiagEntry` in a multi-line, source-aware format
modelled on rustc / clang:

```
error: cannot divide integer by integer-zero
  --> game.loft:88:14
   |
88 |     let damage = attack / armour
   |                         ^^^^^^^^ right-hand side evaluated to 0
   = note: integer division by zero produces a null sentinel; either
           guard with `if armour != 0` or use `attack / armour ?? 0`
```

Two-line compact rendering (today's
`DiagEntry::to_string_compact`) stays available for the test
harness; pretty rendering is the new default for `cargo run --bin
loft`.

## Decision 02.A — wire format

`Diagnostics` becomes a value type with two renderers:

| Renderer | Output | Used by |
|---|---|---|
| `to_string_compact` (existing) | `Error: msg at file:line:col` (one line per entry) | `tests/issues.rs`, `tests/dumps/*`, `LOFT_LOG=...` traces |
| `render_pretty` (new) | Multi-line with caret + note | `cargo run --bin loft`, `loft check`, future LSP |

A CLI flag `--errors=compact|pretty` overrides the default, and the
env var `LOFT_ERRORS=compact|pretty` mirrors it for tests.

## Steps

### 2a — `SourceLoader` trait

```rust
pub trait SourceLoader {
    fn line(&self, file: &str, line_1based: u32) -> Option<&str>;
    fn line_count(&self, file: &str) -> u32;
}
```

A `FileSourceLoader` reads each file once at first access, splits
on `\n`, caches the `Vec<String>` keyed by absolute path.  No
re-stat; no per-error file I/O.

The lexer already loads the source into `self.source: String`
during `add_file` (`src/lexer.rs:1029`).  Phase 2a refactors
loading so the lexer hands the loaded source to the
`FileSourceLoader` after parse, instead of dropping it.  Marginal
memory: ~50 KB per loft file kept resident.

### 2b — Renderer

New file `src/diagnostic_render.rs`:

```rust
pub fn render_pretty(
    entry: &DiagEntry,
    loader: &dyn SourceLoader,
    color: ColorMode,
) -> String { … }
```

Output structure (without colour, ASCII-only — colour added by ANSI
escapes when `color == Auto && stderr.is_terminal()`):

```
<level>: <message>            ← `error:`, `warning:`, `note:`
  --> <file>:<line>:<col>
   |
NN | <source line>
   |     <padding>^^^^...     ← caret + underline of length 1 (default)
   |
   = note: <optional secondary lines>
```

Caret length defaults to 1.  An entry can carry an optional
`span_len: u16` to underline the full token; phase 2 wires this in
the few sites where length is cheap (lexer's `Token` lengths,
operator tokens).

### 2c — Multi-entry layout

`Diagnostics` may hold multiple entries (a parser error often
cascades).  `render_pretty` for the whole `Diagnostics`:

1. Sort entries by `(file, line, col)`.
2. Render each entry separately with one blank line between them.
3. After all entries, render a summary line:
   `error: aborting due to <N> previous error(s)` (rustc style).

Cascade suppression: if two entries share `(file, line, col)` and
the second is `Level::Warning`, drop the warning — the error
already covered the location.

### 2d — Wire into `cargo run --bin loft`

In `src/main.rs`, after parse + compile:

```rust
if !diags.is_empty() {
    let loader = FileSourceLoader::from(&loaded_files);
    let mode = error_mode_from_env_and_args();
    let out = match mode {
        Mode::Pretty => render_pretty_all(&diags, &loader, color),
        Mode::Compact => format!("{}", diags),
    };
    eprintln!("{out}");
    if diags.level() >= Level::Error {
        std::process::exit(1);
    }
}
```

Note: phase 2 covers parse-time and type-check diagnostics only.
Runtime errors stay panics until phase 4 makes them
`RuntimeError`.

### 2e — Tests

- `tests/error_messages.rs` gains a second baseline directory:
  `tests/error_messages/baseline_pretty/` — same 40 cases, captured
  with `LOFT_ERRORS=pretty`.  These are the user-facing goldens.
- `tests/error_messages/baseline/` (compact) stays untouched — that
  is what `cargo test`'s harness sees (it sets `LOFT_ERRORS=compact`
  by default).
- A small unit test in `src/diagnostic_render.rs` covers:
  caret position correctness on tab-indented lines (tabs render as
  4 spaces in the gutter), multi-byte UTF-8 column counting (a
  caret under `ä` lands under `ä`, not its UTF-8 second byte), and
  out-of-range line numbers (graceful "<source unavailable>"
  rendering, no panic).

## Atomic landing sequence

| # | Step | Test |
|---|---|---|
| 2.1 | Add `SourceLoader` trait + `FileSourceLoader` (read once, split, cache) | Unit test: load a fixture file, assert `line(1)` returns first line, `line(huge)` returns `None`, `line_count` correct |
| 2.2 | Hand off lexer's `self.source` to `FileSourceLoader` instead of dropping it | Unit test on a parsed program: assert loader can return source line for the program's path |
| 2.3 | Implement `render_pretty(entry, loader, color=Off)` for a single-entry, single-line caret | Unit test on hand-crafted `DiagEntry`: byte-for-byte match against expected multi-line string |
| 2.4 | Add tab handling (tabs in source render as 4 spaces in the gutter; caret aligns) | Unit test: source line with leading `\t\tfoo`, caret at col 9, asserts caret lands under `f` |
| 2.5 | Add multi-byte UTF-8 column counting (use `unicode-width`) | Unit test: source line `let x = ä;`, caret at col-of-`ä`, asserts caret lands under `ä` not its UTF-8 second byte |
| 2.6 | Multi-entry render: sort by `(file, line, col)`, blank line between entries | Unit test: 3 entries (one in another file) render in stable order |
| 2.7 | Cascade dedup: warning at same position as preceding error is suppressed | Unit test: error+warning at same `(f,l,c)` renders once |
| 2.8 | Summary line: `error: aborting due to N previous error(s)` when entry count ≥ 1 | Unit test: 2-error diagnostics ends with `aborting due to 2 previous errors` |
| 2.9 | Add `LOFT_ERRORS` env var + `--errors=compact|pretty` CLI flag in `main.rs` | Integration test: `LOFT_ERRORS=pretty cargo run -- case.loft` produces multi-line output; `LOFT_ERRORS=compact` produces phase-0 single-line output |
| 2.10 | ANSI colour mode behind `is_terminal()` check | Unit test forces `ColorMode::Always` on a fixture, asserts ANSI escape bytes appear; forces `Off` and asserts none appear |
| 2.11 | Capture `tests/error_messages/baseline_pretty/*.expect` for all 40 cases | Golden test in `tests/error_messages.rs` runs each case under `LOFT_ERRORS=pretty`, asserts byte-for-byte match |

## Acceptance

- `--errors=pretty` and `LOFT_ERRORS=pretty` both work.
- Default for `cargo run --bin loft` is pretty; default for
  `cargo test` is compact.
- All 40 baseline cases have a `.expect` under
  `baseline_pretty/` and the new test asserts byte-for-byte match.
- The compact `.expect` files still match (renderer change is
  additive — compact is unchanged).
- Tab and multi-byte tests pass.
- `make ci` green.

## Risks

| Risk | Mitigation |
|---|---|
| `FileSourceLoader` keeps every loaded source resident → memory growth on large stdlibs | The cap is ~3 files in practice (user file + 2-3 default/*.loft).  If the count grows (e.g. plan-PACKAGES adds many imports), switch to LRU keyed by file path. |
| Coloured output on a redirected stderr corrupts logs | `ColorMode::Auto` only emits ANSI when `is_terminal()`; CI / file redirection gets uncoloured ASCII. |
| Multi-line diagnostics break test harnesses that grep stderr | Compact mode stays single-line; the harnesses use compact.  Pretty is opt-in for users. |
| Caret column miscounted under multi-byte UTF-8 | Use `unicode_width` (already in Cargo for `default/03_text.loft`) for the gutter padding; cover with a test in 2e. |
