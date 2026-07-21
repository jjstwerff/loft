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
