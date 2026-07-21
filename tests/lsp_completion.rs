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
fn in_scope_locals_are_offered_as_variables() {
    // A local `count` (declared by `count = 5`) completes as a Variable (kind 6)
    // when typing its prefix inside the same function — the global scan can't see it.
    let src = "fn main() {\n  count = 5\n  co\n}\n";
    let m = items(src, 3, 5); // `  co`, cursor after the prefix
    assert!(
        m.iter().any(|(k, l)| *k == 6 && l == "count"),
        "local count (variable): {m:?}"
    );
}

#[test]
fn in_scope_locals_are_scope_precise() {
    // `total` lives in `f`, `amount` in `g`; each completes only inside its own
    // function (loft is flat-scoped per fn — the enclosing-fn resolution).
    let src = "fn f() {\n  total = 1\n  to\n}\nfn g() {\n  amount = 2\n  am\n}\n";
    let in_f = items(src, 3, 5); // `  to` in f
    assert!(
        in_f.iter().any(|(k, l)| *k == 6 && l == "total"),
        "f sees total: {in_f:?}"
    );
    assert!(
        !in_f.iter().any(|(_, l)| l == "amount"),
        "f does NOT see amount: {in_f:?}"
    );
    let in_g = items(src, 6, 5); // `  am` in g
    assert!(
        in_g.iter().any(|(k, l)| *k == 6 && l == "amount"),
        "g sees amount: {in_g:?}"
    );
    assert!(
        !in_g.iter().any(|(_, l)| l == "total"),
        "g does NOT see total: {in_g:?}"
    );
}

#[test]
fn a_typoed_prefix_fuzzy_matches() {
    // `prnt` (4 chars, no prefix match) is within edit distance 2 of `print`, so
    // the fuzzy fallback still offers it.  A short typo (< 4 chars) does not, to
    // avoid noise.
    let hit = items("fn main() {\n  prnt\n}\n", 2, 7);
    assert!(
        hit.iter().any(|(k, l)| *k == 3 && l == "print"),
        "prnt fuzzy-matches print: {hit:?}"
    );
    let miss = items("fn main() {\n  pn\n}\n", 2, 5);
    assert!(
        !miss.iter().any(|(_, l)| l == "print"),
        "a 2-char prefix does NOT fuzzy-match: {miss:?}"
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
    assert!(
        !in_f.iter().any(|(_, l)| l == "b"),
        "f does NOT see B.b: {in_f:?}"
    );
    // In `g` (L9) → B's field `b`.
    let in_g = items(src, 9, 5);
    assert!(
        in_g.iter().any(|(k, l)| *k == 5 && l == "b"),
        "g sees B.b: {in_g:?}"
    );
    assert!(
        !in_g.iter().any(|(_, l)| l == "a"),
        "g does NOT see A.a: {in_g:?}"
    );
}
