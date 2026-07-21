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

#[test]
fn member_completion_is_scope_precise_to_the_enclosing_function() {
    // @PLN115: the same receiver name `v` has type A in `f` and type B in `g`;
    // completion after `v.` must offer THIS function's members, not the first `v`.
    let src = "struct A { a: integer }\nstruct B { b: text }\n\
               fn f() {\n  v = A { a: 1 };\n  v.\n}\n\
               fn g() {\n  v = B { b: \"x\" };\n  v.\n}\n";
    // In `f` (L5, `v.` cursor after the dot at col 5) → A's field `a`.
    let in_f = items(src, 5, 5);
    assert!(
        in_f.iter().any(|(k, l)| *k == 5 && l == "a"),
        "f sees A.a: {in_f:?}"
    );
    assert!(!in_f.iter().any(|(_, l)| l == "b"), "f does NOT see B.b: {in_f:?}");
    // In `g` (L9) → B's field `b`.
    let in_g = items(src, 9, 5);
    assert!(
        in_g.iter().any(|(k, l)| *k == 5 && l == "b"),
        "g sees B.b: {in_g:?}"
    );
    assert!(!in_g.iter().any(|(_, l)| l == "a"), "g does NOT see A.a: {in_g:?}");
}
