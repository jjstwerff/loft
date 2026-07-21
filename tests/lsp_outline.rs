// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 S4 — the loft-side outline accessor (`loft::lsp::outline`) that drives
// `textDocument/documentSymbol`.  A fresh stdlib-loaded parser (the same recipe
// as `diagnose`) enumerates the buffer's OWN top-level definitions with their
// kind and source position; the LSP maps each `Symbol` to a `DocumentSymbol`.

use loft::lsp::{Symbol, outline};

/// Outline a buffer with the repo-relative stdlib (tests run from the repo root,
/// like the `diagnose` gate).
fn syms(buf: &str) -> Vec<Symbol> {
    outline(buf, "buf.loft", "default")
}

#[test]
fn lists_top_level_defs_with_kinds_in_source_order() {
    let src = "\
struct Point {
  x: integer,
  y: integer,
}
enum Shape {
  Circle { r: integer },
  Square { s: integer },
}
fn area(p: Point) -> integer {
  p.x * p.y
}
fn main() {
  print(\"hi\")
}
";
    let s = syms(src);
    let got: Vec<(&str, &str)> = s.iter().map(|x| (x.kind, x.name.as_str())).collect();
    assert_eq!(
        got,
        vec![
            ("struct", "Point"),
            ("enum", "Shape"),
            ("fn", "area"),
            ("fn", "main"),
        ],
        "top-level defs, kinds decoded, in source order"
    );
    // Positions are 1-based and strictly increasing by line (source order).
    assert_eq!(s[0].line, 1, "the first def is on line 1: {s:?}");
    assert!(
        s.iter().all(|x| x.line > 0 && x.col > 0),
        "every symbol carries a position: {s:?}"
    );
    assert!(
        s.windows(2).all(|w| w[0].line < w[1].line),
        "symbols are ordered by source line: {s:?}"
    );
}

#[test]
fn excludes_stdlib_symbols_enum_variants_and_synthetics() {
    // The buffer USES stdlib (`print`) — stdlib symbols must not appear.  Enum
    // variants (`Circle`/`Square`) belong to their enum's shape, not the
    // top-level list.  Compiler-synthetic defs (`__nullable<…>`, …) are excluded.
    let s = syms(
        "enum E {\n  A { v: integer },\n  B { w: integer },\n}\nfn main() {\n  print(\"x\")\n}\n",
    );
    let names: Vec<&str> = s.iter().map(|x| x.name.as_str()).collect();
    assert!(names.contains(&"E"), "the user's enum is listed: {names:?}");
    assert!(
        names.contains(&"main"),
        "the user's fn is listed: {names:?}"
    );
    assert!(
        !names.contains(&"A"),
        "an enum variant is not a top-level symbol: {names:?}"
    );
    assert!(
        !names.contains(&"print"),
        "a stdlib symbol is excluded: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.starts_with("__")),
        "no compiler-synthetic defs leak in: {names:?}"
    );
}

#[test]
fn empty_buffer_has_no_symbols() {
    assert!(syms("").is_empty(), "an empty buffer yields no symbols");
}
