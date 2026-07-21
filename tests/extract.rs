// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN63 extract-function ([EXTRACT.md]) — the intra-function data-flow engine.
//! E1: map a line selection to the enclosing function's contiguous TOP-LEVEL
//! statement slice, refusing anything that does not map to whole statements.

use loft::lsp::{extract_inputs, extract_outputs, extract_range};

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

// ── E2 — inputs (parameters): the upward-exposed uses of the slice ────────────

#[test]
fn e2_inputs_are_upward_exposed_uses() {
    // 1 fn f(x: integer) -> integer {
    // 2   y = x + 1;
    // 3   z = y * 2;
    // 4   return z;
    // 5 }
    // Selecting lines 2-3: `x` is read before any write in the slice (and is a
    // parameter → live-in) → an input; `y` is written then read → not an input;
    // `z` is written, never read → not an input.
    let f = "fn f(x: integer) -> integer {\n  y = x + 1;\n  z = y * 2;\n  return z;\n}\n";
    assert_eq!(
        extract_inputs(f, "buf.loft", "default", 2, 3),
        Some(vec!["x".to_string()]),
        "the only upward-exposed use is `x`"
    );
    // Selecting just line 3 (`z = y * 2`): now `y` is read before written → an input.
    assert_eq!(
        extract_inputs(f, "buf.loft", "default", 3, 3),
        Some(vec!["y".to_string()]),
        "narrowing to `z = y * 2` makes `y` the input"
    );
}

#[test]
fn e2_compound_assignment_reads_before_it_writes() {
    // `x = x + 1` is Set(x, Add(Var(x), 1)) — the RHS reads `x` BEFORE the write, so
    // `x` is upward-exposed (an input), even though it is also written in the slice.
    let f = "fn f(x: integer) -> integer {\n  x = x + 1;\n  return x;\n}\n";
    assert_eq!(
        extract_inputs(f, "buf.loft", "default", 2, 2),
        Some(vec!["x".to_string()]),
        "a compound write reads its target first → an input"
    );
}

#[test]
fn e2_a_slice_local_is_not_an_input() {
    // Selecting lines 2-3 where both `a` and `b` are written-first in the slice →
    // no inputs (nothing is read before being written).
    let f = "fn f() -> integer {\n  a = 1;\n  b = a + 2;\n  return b;\n}\n";
    assert_eq!(
        extract_inputs(f, "buf.loft", "default", 2, 3),
        Some(vec![]),
        "a, b are both produced in the slice → no inputs"
    );
}

// ── E3 — outputs (returns): writes in the slice that are live-out ─────────────

#[test]
fn e3_outputs_are_writes_that_are_live_out() {
    // 1 fn g(n: integer) -> integer {
    // 2   total = 0;
    // 3   for i in 0..n {
    // 4     total = total + i;
    // 5   }
    // 6   return total;
    // 7 }
    // Extracting the loop (lines 3-5): `total` is written in the slice and read
    // after it (`return total`) → an output; and it is also an input (read in the
    // body) → an IN-OUT parameter.
    let g = "fn g(n: integer) -> integer {\n  total = 0;\n  for i in 0..n {\n    total = total + i;\n  }\n  return total;\n}\n";
    let outs = extract_outputs(g, "buf.loft", "default", 3, 5).expect("the loop maps");
    assert!(
        outs.contains(&"total".to_string()),
        "total is live-out: {outs:?}"
    );
    // `total` is both an input and an output → in-out.
    let ins = extract_inputs(g, "buf.loft", "default", 3, 5).expect("the loop maps");
    assert!(
        ins.contains(&"total".to_string()) && outs.contains(&"total".to_string()),
        "total is an in-out param: ins={ins:?} outs={outs:?}"
    );
}

#[test]
fn e3_a_dead_write_is_not_an_output() {
    // 1 fn f(x: integer) -> integer {
    // 2   y = x + 1;    <- written in the slice, NEVER read after → not an output
    // 3   z = x + 2;    <- written and read after (return z) → an output
    // 4   return z;
    // 5 }
    let f = "fn f(x: integer) -> integer {\n  y = x + 1;\n  z = x + 2;\n  return z;\n}\n";
    let outs = extract_outputs(f, "buf.loft", "default", 2, 3).expect("the slice maps");
    assert_eq!(
        outs,
        vec!["z".to_string()],
        "only z is live-out (y is dead): {outs:?}"
    );
}

#[test]
fn e3_multiple_live_out_writes_become_multiple_outputs() {
    // Both `a` and `b` are written in the slice and read after it → two outputs.
    // 1 fn f() -> integer {
    // 2   a = 1;
    // 3   b = 2;
    // 4   return a + b;
    // 5 }
    let f = "fn f() -> integer {\n  a = 1;\n  b = 2;\n  return a + b;\n}\n";
    let outs = extract_outputs(f, "buf.loft", "default", 2, 3).expect("the slice maps");
    assert_eq!(outs.len(), 2, "a and b are both live-out: {outs:?}");
    assert!(
        outs.contains(&"a".to_string()) && outs.contains(&"b".to_string()),
        "{outs:?}"
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
