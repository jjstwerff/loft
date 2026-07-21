// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 step F (v1) — scope-aware resolution.  A LOCAL scopes to its enclosing
// function block; a GLOBAL / method stays workspace-wide.  Sharpens
// find-references and rename so a local is renamed only within its own function.

use loft::lsp::{RefScope, Reference, reference_scope, scoped_refs};

#[test]
fn locals_scope_to_their_function_globals_stay_global() {
    let src = "fn a() {\n  x = 1\n}\nfn b() {\n  x = 9\n}\nfn area(w: integer) -> integer { w }\n";
    // `x` inside fn a (line 2) → local, confined to a's block (lines 1..3).
    assert_eq!(
        reference_scope(src, "x", "default", 2),
        RefScope::Local {
            start_line: 1,
            end_line: 3
        }
    );
    // `x` inside fn b (line 5) → local, confined to b's block (lines 4..6).
    assert_eq!(
        reference_scope(src, "x", "default", 5),
        RefScope::Local {
            start_line: 4,
            end_line: 6
        }
    );
    // A global function and a stdlib function stay workspace-wide.
    assert_eq!(reference_scope(src, "area", "default", 7), RefScope::Global);
    assert_eq!(
        reference_scope(src, "print", "default", 2),
        RefScope::Global
    );
}

#[test]
fn scoped_refs_narrows_locals_by_file_and_range() {
    // Non-existent paths: `scoped_refs`'s canonicalize falls back to the path
    // as-is, so the comparison is exact on these literals.
    let refs = vec![
        Reference {
            file: "/w/m.loft".into(),
            line: 2,
            col: 3,
        }, // in range, right file
        Reference {
            file: "/w/m.loft".into(),
            line: 9,
            col: 3,
        }, // out of range
        Reference {
            file: "/w/other.loft".into(),
            line: 2,
            col: 3,
        }, // wrong file
    ];
    let local = RefScope::Local {
        start_line: 1,
        end_line: 3,
    };
    let kept = scoped_refs(refs.clone(), &local, "/w/m.loft");
    assert_eq!(
        kept.len(),
        1,
        "only the in-range same-file ref survives: {kept:?}"
    );
    assert_eq!(kept[0].line, 2);

    // A global keeps everything.
    assert_eq!(scoped_refs(refs, &RefScope::Global, "/w/m.loft").len(), 3);
}
