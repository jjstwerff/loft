// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 step D — semantic tokens (`loft::lsp::semantic_tokens`).  Classifies
// each identifier by its resolved def kind (reusing the references/completion
// name-lookup); a lexical first cut (globals + keywords), the editor's grammar
// covers locals/keywords the parser leaves untyped.

use loft::lsp::{semantic_token_types, semantic_tokens};

#[test]
fn classifies_structs_functions_and_types() {
    let src = "struct Point {\n  x: integer,\n}\nfn area(w: integer) -> integer {\n  w\n}\n";
    let toks = semantic_tokens(src, "b.loft", "default");
    let legend = semantic_token_types();
    let named: Vec<(u32, u32, &str)> = toks
        .iter()
        .map(|t| (t.line, t.col, legend[t.kind as usize]))
        .collect();
    assert!(
        named
            .iter()
            .any(|(l, c, k)| *l == 1 && *c == 8 && *k == "struct"),
        "`Point` is a struct at 1:8: {named:?}"
    );
    assert!(
        named.iter().any(|(l, _, k)| *l == 4 && *k == "function"),
        "`area` is a function on line 4: {named:?}"
    );
    assert!(
        named.iter().any(|(_, _, k)| *k == "type"),
        "`integer` is a type: {named:?}"
    );
    // Sorted by (line, col).
    assert!(
        toks.windows(2)
            .all(|w| (w[0].line, w[0].col) <= (w[1].line, w[1].col)),
        "tokens are sorted for delta-encoding"
    );
}

#[test]
fn index_classifies_locals_methods_and_fields() {
    // @PLN115: the resolution index types LOCALS (variable), METHODS (method), and
    // FIELDS (property) — the name-based cut could not.
    let src = "struct P { x: integer }\nfn main() {\n  p = P { x: 1 };\n  q = p.x;\n  s = \"hi\";\n  z = s.len();\n}\n";
    let toks = semantic_tokens(src, "b.loft", "default");
    let legend = semantic_token_types();
    let named: Vec<(u32, u32, &str)> = toks
        .iter()
        .map(|t| (t.line, t.col, legend[t.kind as usize]))
        .collect();
    // local `p` (L3 col 3) → variable
    assert!(
        named.iter().any(|(l, c, k)| *l == 3 && *c == 3 && *k == "variable"),
        "local p is a variable: {named:?}"
    );
    // field `p.x` (the `x` at L4 col 9) → property
    assert!(
        named.iter().any(|(l, c, k)| *l == 4 && *c == 9 && *k == "property"),
        "field x is a property: {named:?}"
    );
    // method `s.len` (the `len` at L6 col 9) → method
    assert!(
        named.iter().any(|(l, c, k)| *l == 6 && *c == 9 && *k == "method"),
        "method len is a method: {named:?}"
    );
}
