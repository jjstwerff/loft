// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN115 — the parse-time resolution index.  Each step's hook lands with a
//! probe here.  S2: local-variable occurrences resolve to their binding IDENTITY
//! `(fn_def, var_nr)`, so two same-named locals in different functions are distinct.
use loft::parser::Parser;
use loft::resolution::Resolution;

fn parse_with_resolutions(src: &str) -> Parser {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.set_record_resolutions(true);
    p.parse_source(src, "buf.loft", false);
    p
}

#[test]
fn s2_local_occurrences_resolve_to_distinct_bindings() {
    // Two functions, each with a local `s`; every occurrence (write target,
    // read, return) must record, and the two `s` bindings must not collide.
    let p = parse_with_resolutions(
        "fn aa() -> integer {\n  s = 1;\n  s = s + 1;\n  return s;\n}\n\
         fn bb() -> integer {\n  s = 9;\n  return s;\n}\n",
    );
    let mut aa = vec![];
    let mut bb = vec![];
    for o in p.resolutions() {
        if let Resolution::Local { fn_def, var_nr } = o.res {
            if o.line <= 4 {
                aa.push((fn_def, var_nr));
            } else {
                bb.push((fn_def, var_nr));
            }
        }
    }
    // aa: `s=1`, `s=…`, `…s…`, `return s` = 4 occurrences of one binding.
    assert_eq!(aa.len(), 4, "expected 4 occurrences of aa's `s`, got {aa:?}");
    assert_eq!(bb.len(), 2, "expected 2 occurrences of bb's `s`, got {bb:?}");
    assert!(aa.iter().all(|b| *b == aa[0]), "aa's `s` occurrences must share one binding: {aa:?}");
    assert!(bb.iter().all(|b| *b == bb[0]), "bb's `s` occurrences must share one binding: {bb:?}");
    assert_ne!(aa[0], bb[0], "same-named locals in different fns must be distinct bindings");
}

#[test]
fn s2_recording_off_by_default() {
    // The gate must be off unless explicitly enabled (the zero-cost compile path).
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse_source("fn cc() -> integer {\n  q = 5;\n  return q;\n}\n", "buf.loft", false);
    assert!(p.resolutions().is_empty(), "recording must be off by default");
}

#[test]
fn s3_lsp_parse_carries_buffer_occurrences() {
    // The LSP parse path enables recording and exposes the occurrences; every one
    // is a LOCAL positioned in the user buffer (recording is on only after the
    // stdlib load, so no stdlib occurrences leak in).
    let src = "fn dd() -> integer {\n  total = 5;\n  return total + total;\n}\n";
    let occ = loft::lsp::resolutions(src, "buf.loft", "default");
    assert!(!occ.is_empty(), "the LSP parse must carry occurrences");
    for o in &occ {
        assert!(
            matches!(o.res, Resolution::Local { .. }),
            "S2 records locals only: {:?}",
            o.res
        );
        // Positioned in the 4-line user buffer, never in the stdlib.
        assert!(o.line >= 1 && o.line <= 4, "occurrence in-buffer: line {}", o.line);
    }
    // `total` appears 3× (write + two reads); all share one binding.
    let totals: Vec<_> = occ
        .iter()
        .filter(|o| o.len == 5)
        .map(|o| o.res.clone())
        .collect();
    assert_eq!(totals.len(), 3, "three `total` occurrences: {totals:?}");
    assert!(totals.iter().all(|r| *r == totals[0]), "one binding: {totals:?}");
}

#[test]
fn s5_free_function_call_records_global() {
    // A free-function call resolves to a Global reference at its name; a param/local
    // in the same buffer stays a Local (the two resolution kinds coexist).
    let p = parse_with_resolutions(
        "fn helper(k: integer) -> integer {\n  return k + 1;\n}\n\
         fn main() {\n  n = helper(3);\n  print(\"{n}\");\n}\n",
    );
    let helper_def = p.data.def_nr("n_helper");
    assert_ne!(helper_def, u32::MAX, "helper must exist");
    let globals: Vec<_> = p
        .resolutions()
        .iter()
        .filter_map(|o| match o.res {
            Resolution::Global(d) => Some((o.line, o.col, o.len, d)),
            _ => None,
        })
        .collect();
    // The `helper(3)` call in main records Global(n_helper), name length 6.
    assert!(
        globals
            .iter()
            .any(|(_, _, len, d)| *len == 6 && *d == helper_def),
        "helper() call records Global(n_helper): {globals:?}"
    );
    // The param `k` inside helper is still a Local — Global recording is additive.
    assert!(
        p.resolutions()
            .iter()
            .any(|o| matches!(o.res, Resolution::Local { .. })),
        "locals still recorded alongside globals"
    );
}

#[test]
fn s4_assignment_local_refs_exclude_field_access() {
    // Local `x` is assigned then read alongside a field `p.x` of the SAME name.
    // The precise path must return the local's decl + read, and NOT the `p.x` field.
    // Layout (1-based):  L3 `  x = 5;`   L4 `  print("{p.x} {x}");`
    let src = "struct P { x: integer }\nfn h(p: P) {\n  x = 5;\n  print(\"{p.x} {x}\");\n}\n";
    // Cursor on the local `x` at its declaration (L3, col 3).
    let refs = loft::lsp::local_binding_refs(src, "default", "/buf.loft", 3, 3)
        .expect("assignment-local takes the precise path");
    let positions: Vec<(u32, u32)> = refs.iter().map(|r| (r.line, r.col)).collect();
    // Decl `x` at L3:3 and read `x` at L4 (inside the interpolation) — 2 refs.
    assert_eq!(refs.len(), 2, "local x: decl + one read, field p.x excluded: {positions:?}");
    assert!(positions.contains(&(3, 3)), "includes the declaration: {positions:?}");
    // The `p.x` field's `x` sits at L4 col 11; it must NOT be in the set.
    assert!(!positions.contains(&(4, 11)), "field p.x is excluded: {positions:?}");
}

#[test]
fn s4_parameter_falls_back_to_fv1() {
    // A parameter's signature declaration is not in the index, so the precise
    // path is unsound → None (the caller uses the F-v1 name-scan).
    let src = "fn f(w: integer) -> integer {\n  return w + w;\n}\n";
    // Cursor on a body use of `w` (L2, col 10).
    assert!(
        loft::lsp::local_binding_refs(src, "default", "/buf.loft", 2, 10).is_none(),
        "a parameter must fall back to F-v1"
    );
}

#[test]
fn s4_loop_binder_falls_back_to_fv1() {
    // A `for i` binder's declaration is not recorded → the earliest occurrence is
    // a use, not a `name =` write → None (fall back).
    let src = "fn g() {\n  total = 0;\n  for i in 0..3 {\n    total = total + i;\n  }\n}\n";
    // Cursor on the body use of `i` (L4, col 21).
    assert!(
        loft::lsp::local_binding_refs(src, "default", "/buf.loft", 4, 21).is_none(),
        "a loop binder must fall back to F-v1"
    );
    // The assignment-local `total` in the SAME function still takes the precise path.
    let refs = loft::lsp::local_binding_refs(src, "default", "/buf.loft", 2, 3)
        .expect("total is an assignment-local");
    assert_eq!(refs.len(), 3, "total: decl + two writes/reads");
}
