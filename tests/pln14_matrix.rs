// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN14 Step 0 — the composition matrix (instrument, no product code).
//!
//! The load-bearing claim: *every value survives `bind → session-store →
//! observe` byte-for-byte equal.*  This file falsifies it across value-type ×
//! persistence-path, using ONLY the proven sibling (`Stores::snapshot_heap` /
//! `restore_heap`, @PLN63 RX) so the in-memory round-trip is pinned before any
//! arc-B product code exists.
//!
//! The strong form of the round-trip: snapshot the heap, then restore it into a
//! **fresh `State` that never built the value**, and render the value from
//! there.  It is non-vacuous by construction — the fresh heap does not even have
//! a store slot at the value's `store_nr` until the restore creates it, which
//! each case asserts.  Equality then proves the snapshot alone reconstructs the
//! value: the self-containment the session store needs.
//!
//! Findings recorded on the plan (2026-07-24): a loft value SPANS multiple
//! stores (a nested `struct` puts each `Reference` field in its own store), and
//! text in a struct field is stored IN-store (a `set_str` record, byte-copied by
//! the snapshot).  The on-disk (`save → restore`) cells wait on arc F's persist
//! primitive (Step 6).

use loft::compile;
use loft::keys::DbRef;
use loft::parser::Parser;
use loft::state::State;

fn stdlib() -> Parser {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("load stdlib");
    p
}

/// Parse `defs`, compile+run `fn probe() -> ty { expr }`, and return the live
/// `State` (value on the stack top) alongside the `Parser` (schema owner).
fn build(defs: &[&str], ty: &str, expr: &str) -> (Parser, State) {
    let mut p = stdlib();
    for def in defs {
        p.parse_str(&format!("{def}\n"), "<probe>", false);
    }
    let src = format!("fn probe() -> {ty} {{\n{expr}\n}}\n");
    p.parse_str(&src, "<probe>", false);
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data);
    compile::byte_code(&mut state, &mut p.data);
    state.execute_argv("probe", &p.data, &[]);
    assert!(
        state.database.runtime_error.is_none(),
        "probe `{expr}` faulted: {:?}",
        state.database.runtime_error
    );
    (p, state)
}

/// The in-memory round-trip via the proven sibling.  Returns
/// `(before, after, existed_before_restore)` — the matrix asserts the two
/// renders are equal and that the value's store did NOT exist pre-restore.
fn snapshot_round_trip(
    defs: &[&str],
    ty: &str,
    type_name: &str,
    expr: &str,
) -> (String, String, bool) {
    let (p, mut state) = build(defs, ty, expr);
    // `get_stack` CONSUMES — read the root DbRef exactly once.
    let root = *state.get_stack::<DbRef>();
    let tp = state.database.name(type_name);
    assert!(tp != u16::MAX, "unresolved type name `{type_name}`");
    let mut before = String::new();
    state.database.show_loft(&mut before, &root, tp);

    let snap = state
        .database
        .snapshot_heap()
        .expect("an all-in-memory heap snapshots");

    // A fresh state with the same schema but no knowledge of the value.
    let mut data = p.data.clone();
    let mut fresh = State::new(p.database.clone());
    compile::byte_code(&mut fresh, &mut data);
    // Non-vacuity: the value's root store does not exist in this heap yet.
    let existed = (root.store_nr as usize) < fresh.database.allocations.len();

    fresh.database.restore_heap(&snap);
    let tp2 = fresh.database.name(type_name);
    let mut after = String::new();
    fresh.database.show_loft(&mut after, &root, tp2);
    (before, after, existed)
}

/// The value-type axis for heap-resident values (scalars live on the stack, not
/// the heap — their store round-trip is arc C).  Each is a distinct store
/// layout the one snapshot rule must cover.
const STRUCTS: &str = "struct P { x: integer, y: integer }\n\
     struct Line { a: P, b: P, tag: text }\n\
     struct Bag { items: vector<P>, name: text }\n\
     struct Big { id: integer, note: text }\n\
     enum Shape { Circle(integer), Rect(integer, integer), Dot }";

fn heap_cases() -> Vec<(&'static str, &'static str, String)> {
    let big_text = "x".repeat(400); // > 256 B text
    vec![
        (
            "vector<integer>",
            "vector<integer>",
            "[1, 2, 3, 4, 5]".into(),
        ),
        (
            "vector<integer>",
            "vector<integer>",
            // >32-bit elements
            "[9000000000, -9000000001, 0]".into(),
        ),
        ("P", "P", "P { x: 7, y: 9 }".into()),
        (
            "Line",
            "Line",
            "Line { a: P{x:1,y:2}, b: P{x:3,y:4}, tag: \"seg\" }".into(),
        ),
        (
            "vector<P>",
            "vector<P>",
            "[P{x:1,y:2}, P{x:3,y:4}, P{x:5,y:6}]".into(),
        ),
        (
            "Bag",
            "Bag",
            "Bag { items: [P{x:1,y:2}, P{x:3,y:4}], name: \"sack\" }".into(),
        ),
        // text with embedded quote / newline / backslash / unicode
        (
            "Big",
            "Big",
            "Big { id: 1, note: \"a\\\"b\\nc\\\\d \\u{2603}\" }".into(),
        ),
        // > 256 B text in a struct field
        (
            "Big",
            "Big",
            format!("Big {{ id: 2, note: \"{big_text}\" }}"),
        ),
        (
            "vector<text>",
            "vector<text>",
            "[\"\", \"one\", \"two \\u{2603}\"]".into(),
        ),
        // struct-enum variants (heap-backed)
        ("Shape", "Shape", "Shape.Circle(5)".into()),
        ("Shape", "Shape", "Shape.Rect(3, 4)".into()),
    ]
}

#[test]
fn heap_values_survive_snapshot_round_trip() {
    for (ty, type_name, expr) in heap_cases() {
        let (before, after, existed) = snapshot_round_trip(&[STRUCTS], ty, type_name, &expr);
        assert!(
            !existed,
            "vacuous cell: the fresh heap already had a store at the value's \
             store_nr for `{expr}` — the equality below would not be carried by \
             the snapshot"
        );
        assert!(!before.is_empty(), "empty render for `{expr}`");
        assert_eq!(
            before, after,
            "snapshot round-trip diverged for `{expr}` (type {type_name})"
        );
    }
}

/// Aliasing / value-semantics control (arc B's Q4): `b = a` must COPY, so a
/// later mutation of `a` leaves `b` unchanged.  A store-copy materialize (not an
/// alias) is the only way this holds — expressed as a loft program so it pins
/// the language contract the session store must preserve.
#[test]
fn cross_binding_is_a_copy_not_an_alias() {
    let (_p, mut state) = build(
        &[STRUCTS],
        "integer",
        "a = [1, 2, 3];\n  b = a;\n  a[0] = 99;\n  b[0]",
    );
    assert_eq!(*state.get_stack::<i64>(), 1, "b aliased a (expected a copy)");
}

/// Negative control: a faulting RHS records a runtime error and NO value — the
/// structural form of the interim's `Capture::Failed` (no env entry, no poison).
#[test]
fn faulting_bind_records_a_fault_not_a_value() {
    let mut p = stdlib();
    let src = "fn probe() -> integer {\n  assert(false, \"boom\");\n  1\n}\n";
    p.parse_str(src, "<probe>", false);
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data);
    compile::byte_code(&mut state, &mut p.data);
    state.execute_argv("probe", &p.data, &[]);
    assert!(
        state.database.runtime_error.is_some(),
        "a faulting bind must record a runtime error"
    );
}

/// Store-span reading (the finding that shapes arc B): a nested `struct` puts
/// each `Reference` field in its OWN store, so a value is generally a
/// MULTI-store graph.  A per-binding materialize therefore needs a reachability
/// walk + store-number rebase, not a single `Store::snapshot_copy`.  Pinned as a
/// test so the assumption cannot drift silently.
#[test]
fn a_nested_struct_spans_several_stores() {
    let flat = build(&[STRUCTS], "P", "P { x: 7, y: 9 }")
        .1
        .database
        .allocations
        .len();
    let nested = build(
        &[STRUCTS],
        "Line",
        "Line { a: P{x:1,y:2}, b: P{x:3,y:4}, tag: \"seg\" }",
    )
    .1
    .database
    .allocations
    .len();
    assert!(
        nested > flat,
        "expected a nested struct to span more stores than a flat one \
         (flat={flat}, nested={nested})"
    );
}
