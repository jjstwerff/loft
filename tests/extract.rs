// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN63 extract-function ([EXTRACT.md]) — the intra-function data-flow engine.
//! E1: map a line selection to the enclosing function's contiguous TOP-LEVEL
//! statement slice, refusing anything that does not map to whole statements.

use loft::lsp::extract_range;

// 1  fn f() -> integer {
// 2    a = 1;
// 3    b = a + 2;
// 4    c = a + b;
// 5    return c;
// 6  }
const F: &str = "fn f() -> integer {\n  a = 1;\n  b = a + 2;\n  c = a + b;\n  return c;\n}\n";

#[test]
fn e1_selection_maps_to_a_top_level_statement_slice() {
    // A single statement (line 2) → a slice; more statements → a wider op range.
    let one = extract_range(F, "buf.loft", "default", 2, 2).expect("line 2 = `a = 1`");
    let three = extract_range(F, "buf.loft", "default", 2, 4).expect("lines 2-4 = a, b, c");
    assert_eq!(three.0, one.0, "same enclosing function");
    assert!(one.2 >= one.1 && three.2 >= three.1, "non-empty op ranges");
    assert!(
        three.2 - three.1 > one.2 - one.1,
        "3 statements span more operators than 1: {one:?} vs {three:?}"
    );
    // The tail of the body (line 3 to the last statement) also maps.
    assert!(
        extract_range(F, "buf.loft", "default", 3, 5).is_some(),
        "lines 3-5 = b, c, return"
    );
}

#[test]
fn e1_refuses_non_statement_boundaries() {
    assert!(
        extract_range(F, "buf.loft", "default", 1, 1).is_none(),
        "the signature line is not a statement start"
    );
    assert!(
        extract_range(F, "buf.loft", "default", 6, 6).is_none(),
        "the closing brace is not a statement start"
    );
    assert!(
        extract_range(F, "buf.loft", "default", 4, 3).is_none(),
        "a reversed range"
    );
    assert!(
        extract_range(F, "buf.loft", "default", 99, 99).is_none(),
        "a line past EOF"
    );
}

#[test]
fn e1_refuses_a_selection_inside_a_nested_block() {
    // 1  fn g() {
    // 2    total = 0;
    // 3    for i in 0..3 {
    // 4      total = total + i;    <- inside the loop body (a nested block)
    // 5    }
    // 6  }
    let g = "fn g() {\n  total = 0;\n  for i in 0..3 {\n    total = total + i;\n  }\n}\n";
    // Line 4 is inside the loop body → NOT a top-level statement → refused.
    assert!(
        extract_range(g, "buf.loft", "default", 4, 4).is_none(),
        "a line inside a nested loop body is not a top-level statement"
    );
    // But the top-level statements DO map: `total = 0` (line 2), and the whole
    // `for` statement (lines 3-5) as one top-level operator group.
    assert!(
        extract_range(g, "buf.loft", "default", 2, 2).is_some(),
        "the top-level `total = 0`"
    );
    assert!(
        extract_range(g, "buf.loft", "default", 3, 5).is_some(),
        "the whole for statement"
    );
}
