// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 S2 — the loft-side diagnostics path for loft-lsp.
//
// loft ALREADY carries positioned, coded diagnostics (@I75; @PLN102 arc-E): a
// Parser with the stdlib loaded, then `parse_source(buffer)`, collects each error
// as a `DiagEntry { level, line, col, message, code }` into `p.diagnostics` — no
// stderr, no exit.  The LSP maps `DiagEntry` -> LSP `Diagnostic` at S3.
//
// Recipe: a FRESH stdlib-loaded parser per parse.  A parser cannot be re-parsed:
// loft registers every definition per source (that is how files read each other
// on `use`), so a second `parse_source` on the same parser re-registers and
// conflicts ("Cannot redefine 'main'").  A fresh `parse_dir("default") +
// parse_source(buf)` is the correct model — ~80 ms, within the LSP budget, and it
// cannot carry state across edits.

use loft::diagnostics::Level;
use loft::parser::Parser;

/// The loft-lsp diagnostics recipe: fresh stdlib-loaded parser -> parse buffer ->
/// the error/fatal entries as `(level, line, col, message)`.
fn diagnose(buf: &str) -> Vec<(Level, u32, u32, String)> {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("load stdlib");
    p.parse_source(buf, "buf.loft", false);
    p.diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= Level::Error)
        .map(|e| (e.level, e.line, e.col, e.message.clone()))
        .collect()
}

#[test]
fn clean_buffer_has_no_diagnostics() {
    let e = diagnose("fn main() {\n  print(\"hi\")\n}\n");
    assert!(e.is_empty(), "a valid program yields no error diagnostics, got {e:?}");
}

#[test]
fn syntax_error_is_correctly_positioned() {
    let e = diagnose("fn main() {\n  let x =\n}\n");
    assert!(!e.is_empty(), "an incomplete `let` must error");
    let (lvl, line, col, msg) = &e[0];
    assert!(*lvl >= Level::Error);
    assert_eq!(*line, 2, "the missing `;` is on line 2, got {e:?}");
    assert!(*col > 0, "carries a column");
    assert!(msg.contains("token"), "names the missing token: {msg}");
}

#[test]
fn unknown_symbol_is_reported_with_a_message() {
    let e = diagnose("fn main() {\n  nope(3)\n}\n");
    assert!(!e.is_empty(), "calling an undefined fn must error");
    let (lvl, line, _col, msg) = &e[0];
    assert!(*lvl >= Level::Error);
    assert!(*line > 0, "carries a source line");
    assert!(msg.contains("nope"), "names the offending symbol: {msg}");
    // KNOWN GAP (dogfood finding (b), S2): deferred/semantic errors report at the
    // resolution point (end of the enclosing item), not the reference site — so
    // this lands on line 3 (`}`), not line 2.  Syntax errors are exact.  The fix
    // is the next diagnostic-quality step: stamp the reference's own position.
}
