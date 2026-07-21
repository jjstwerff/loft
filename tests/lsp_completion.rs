// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 step C — completion (`loft::lsp::complete`).  Reuses Data + classify for
// candidates, the per-fn variable tables for `expr.` member types, and the
// keyword list.  Kinds are LSP CompletionItemKind (Method 2, Function 3, Field 5,
// EnumMember 20, Keyword 14).

use loft::lsp::complete;

fn items(text: &str, line: u32, col: u32) -> Vec<(u32, String)> {
    complete(text, "b.loft", "default", line, col)
        .into_iter()
        .map(|c| (c.kind, c.label))
        .collect()
}

#[test]
fn identifier_prefix_completes_globals_and_keywords() {
    // `pri` → the stdlib `print` (Function). Cursor after the prefix (1-based col 6).
    let g = items("fn main() {\n  pri\n}\n", 2, 6);
    assert!(
        g.iter().any(|(k, l)| *k == 3 && l == "print"),
        "print (fn): {g:?}"
    );
    // `ret` → the `return` keyword.
    let k = items("fn main() {\n  ret\n}\n", 2, 6);
    assert!(
        k.iter().any(|(k, l)| *k == 14 && l == "return"),
        "return (keyword): {k:?}"
    );
}

#[test]
fn member_completion_after_a_variable() {
    let src = "struct Point {\n  x: integer,\n  y: integer,\n}\nfn area(self: Point) -> integer {\n  self.x\n}\nfn main() {\n  p = Point { x: 1, y: 2 };\n  p.\n}\n";
    // `p.` on line 10 (`  p.`), cursor after the dot (col 5).
    let m = items(src, 10, 5);
    assert!(m.iter().any(|(k, l)| *k == 5 && l == "x"), "field x: {m:?}");
    assert!(m.iter().any(|(k, l)| *k == 5 && l == "y"), "field y: {m:?}");
    assert!(
        m.iter().any(|(k, l)| *k == 2 && l == "area"),
        "method area: {m:?}"
    );
    assert_eq!(
        m.iter().filter(|(_, l)| l == "area").count(),
        1,
        "no method/virtual-field duplicate: {m:?}"
    );
}

#[test]
fn member_completion_lists_enum_variants() {
    let src = "enum Shape {\n  Circle { r: integer },\n  Square { s: integer },\n}\nfn main() {\n  x = Shape.\n}\n";
    // `Shape.` cursor after the dot (col 13).
    let m = items(src, 6, 13);
    assert!(
        m.iter().any(|(k, l)| *k == 20 && l == "Circle"),
        "Circle variant: {m:?}"
    );
    assert!(
        m.iter().any(|(k, l)| *k == 20 && l == "Square"),
        "Square variant: {m:?}"
    );
}
