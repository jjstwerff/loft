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
    assert_readable(&root, &state, expr);
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
     enum Shape { Circle { radius: float }, \
                  Rect { width: float, height: float }, \
                  Tagged { name: text } }";

/// Calibration guard — the instrument must be able to SEE the value before a
/// cell may report anything.  A mis-specified case (bad syntax → a null or
/// out-of-range root) would otherwise round-trip garbage to identical garbage
/// and pass **vacuously**; that is exactly how the first draft of this matrix
/// green-lit two struct-enum cells that never built a value at all.
fn assert_readable(root: &DbRef, state: &State, expr: &str) {
    assert!(
        root.store_nr != u16::MAX
            && (root.store_nr as usize) < state.database.allocations.len()
            && root.rec != 0,
        "vacuous cell: `{expr}` did not leave a readable heap value on the stack \
         (root = {root:?}) — the case is mis-specified, not passing"
    );
}

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
        // struct-enum variants (heap-backed): the variant name constructs it
        ("Shape", "Shape", "Circle { radius: 1.5 }".into()),
        ("Shape", "Shape", "Rect { width: 3.0, height: 4.0 }".into()),
        (
            "Shape",
            "Shape",
            "Tagged { name: \"a\\\"b \\u{2603}\" }".into(),
        ),
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

/// Step 1 — the arc-B round-trip: `bind → materialize → observe`.  Materializes
/// the value into a dedicated session store, then **deep-frees the source**
/// (`remove_claims` + `free`, the proven inverse of the `copy_claims` walk
/// materialize is built on) before reading the copy back.  Surviving that is the
/// "exactly one owned home" property: the copy shares nothing with the source.
fn materialize_round_trip(
    defs: &[&str],
    ty: &str,
    type_name: &str,
    expr: &str,
) -> (String, String) {
    let (_p, mut state) = build(defs, ty, expr);
    let root = *state.get_stack::<DbRef>();
    assert_readable(&root, &state, expr);
    let tp = state.database.name(type_name);
    assert!(tp != u16::MAX, "unresolved type name `{type_name}`");
    let mut before = String::new();
    state.database.show_loft(&mut before, &root, tp);

    // The session store, and the value's own home inside it.
    let session = state.database.database(256);
    let copy = state.database.materialize(&root, tp, session.store_nr);
    assert_ne!(
        copy.store_nr, root.store_nr,
        "materialize returned a ref into the SOURCE store for `{expr}`"
    );

    // Deep-free the source: every nested allocation it owns, then its store.
    state.database.remove_claims(&root, tp);
    state.database.free(&root);

    let mut after = String::new();
    state.database.show_loft(&mut after, &copy, tp);
    (before, after)
}

#[test]
fn materialize_gives_each_value_its_own_home() {
    for (ty, type_name, expr) in heap_cases() {
        let (before, after) = materialize_round_trip(&[STRUCTS], ty, type_name, &expr);
        assert!(!before.is_empty(), "empty render for `{expr}`");
        assert_eq!(
            before, after,
            "materialized copy did not survive the source being deep-freed for \
             `{expr}` (type {type_name}) — it still shares storage with the source"
        );
    }
}

/// A null / empty source materializes as null: a faulting bind records no value
/// (no env entry, no poison) rather than a half-built record.
#[test]
fn materialize_of_null_is_null() {
    let (_p, mut state) = build(&[STRUCTS], "P", "P { x: 1, y: 2 }");
    let root = *state.get_stack::<DbRef>();
    let tp = state.database.name("P");
    let session = state.database.database(256);
    let null_src = DbRef {
        store_nr: u16::MAX,
        rec: 0,
        pos: 0,
    };
    let out = state.database.materialize(&null_src, tp, session.store_nr);
    assert_eq!(out.rec, 0, "a null source must materialize as null");
    let _ = root;
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
    assert_eq!(
        *state.get_stack::<i64>(),
        1,
        "b aliased a (expected a copy)"
    );
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

/// Positive control for the calibration guard: it must FIRE on each shape of
/// unreadable root (the null sentinel, an out-of-range store, record 0).  A
/// guard is only evidence once it is shown it can fail — otherwise the guard is
/// itself the vacuous thing.
#[test]
fn calibration_guard_catches_an_unreadable_root() {
    let (_p, state) = build(&[STRUCTS], "P", "P { x: 1, y: 2 }");
    let live = state.database.allocations.len() as u16;
    let unreadable = [
        DbRef {
            store_nr: u16::MAX,
            rec: 0,
            pos: 0,
        }, // null sentinel
        DbRef {
            store_nr: 65281,
            rec: 0,
            pos: 0,
        }, // out-of-range (garbage)
        DbRef {
            store_nr: live + 7,
            rec: 1,
            pos: 8,
        }, // past the store table
        DbRef {
            store_nr: 2,
            rec: 0,
            pos: 8,
        }, // in range but record 0
    ];
    for root in unreadable {
        let fired =
            std::panic::catch_unwind(|| assert_readable(&root, &state, "<synthetic>")).is_err();
        assert!(fired, "the calibration guard did NOT fire on root {root:?}");
    }
}

/// The property arcs A and F lean on: a materialized value is **entirely
/// self-contained in ONE store**.  `copy_claims` allocates every sub-record and
/// every text into the DESTINATION store, so after materialize we can free every
/// other user store and the copy still reads back identical.  That is what makes
/// the session store a single extractable / persistable unit (arc F persists one
/// store, not a graph of them).
#[test]
fn a_materialized_value_is_self_contained_in_one_store() {
    for (ty, type_name, expr) in heap_cases() {
        let (_p, mut state) = build(&[STRUCTS], ty, &expr);
        let root = *state.get_stack::<DbRef>();
        assert_readable(&root, &state, &expr);
        let tp = state.database.name(type_name);
        let mut before = String::new();
        state.database.show_loft(&mut before, &root, tp);

        let session = state.database.database(256);
        let copy = state.database.materialize(&root, tp, session.store_nr);

        // Free EVERY other user store (store 0 is the eval stack; 1 is reserved).
        let total = state.database.allocations.len() as u16;
        for nr in 2..total {
            if nr == session.store_nr {
                continue;
            }
            state.database.free(&DbRef {
                store_nr: nr,
                rec: 0,
                pos: 0,
            });
        }

        let mut after = String::new();
        state.database.show_loft(&mut after, &copy, tp);
        assert_eq!(
            before, after,
            "materialized value was NOT self-contained in its store for `{expr}` \
             (type {type_name}) — it still reads from a store we freed"
        );
    }
}

/// The Step-2 crux: can the session store be carried ACROSS runs?  A
/// materialized value is self-contained, but if its interior references were
/// absolute (`store_nr` = the slot it was materialized at) it would break the
/// moment the store is re-adopted at a different slot in the next eval's
/// throwaway `State`.  So: materialize, take the store OUT, adopt it into a
/// FRESH state at a deliberately different slot, and render from there.
///
/// Equal renders ⇒ interior refs are slot-independent ⇒ the env can key on
/// `(rec, pos)` and rebuild the `DbRef` from wherever the store currently sits.
#[test]
fn a_session_store_survives_re_adoption_at_a_different_slot() {
    for (ty, type_name, expr) in heap_cases() {
        let (p, mut state) = build(&[STRUCTS], ty, &expr);
        let root = *state.get_stack::<DbRef>();
        assert_readable(&root, &state, &expr);
        let tp = state.database.name(type_name);
        let mut before = String::new();
        state.database.show_loft(&mut before, &root, tp);

        let session = state.database.database(256);
        let copy = state.database.materialize(&root, tp, session.store_nr);
        let detached = state.database.take_store(session.store_nr);
        drop(state); // the whole run heap goes away, as it does between evals

        // A fresh run, with extra stores allocated first so the session store
        // is forced to land at a DIFFERENT slot than it had.
        let mut data = p.data.clone();
        let mut next = State::new(p.database.clone());
        compile::byte_code(&mut next, &mut data);
        for _ in 0..3 {
            let _ = next.database.database(64);
        }
        let new_nr = next.database.adopt_store(detached);
        assert_ne!(
            new_nr, session.store_nr,
            "probe is vacuous: the store landed at the SAME slot for `{expr}`"
        );

        let rebuilt = DbRef {
            store_nr: new_nr,
            rec: copy.rec,
            pos: copy.pos,
        };
        let tp2 = next.database.name(type_name);
        let mut after = String::new();
        next.database.show_loft(&mut after, &rebuilt, tp2);
        assert_eq!(
            before, after,
            "the session store did NOT survive re-adoption at slot {new_nr} \
             (was {}) for `{expr}` — interior refs are slot-dependent",
            session.store_nr
        );
    }
}

// ── Step 2 — the session store + env record, as a write-only shadow ──────────
//
// The env is written on every heap-backed bind and read ONLY by `env_value`.
// The replay model is still the source of truth, so these are the differential
// oracle: the shadow must already agree everywhere, which is what Step 4's
// frame-seed will be checked against.

use loft::repl::{Eval, ReplSession};

fn session() -> ReplSession {
    ReplSession::new("default").expect("load stdlib")
}

/// The type definitions the session-level tests need, one input per `eval`.
const REPL_DEFS: &[&str] = &[
    "struct P { x: integer, y: integer }",
    "struct Line { a: P, b: P, tag: text }",
    "struct Bag { items: vector<P>, name: text }",
    "struct Big { id: integer, note: text }",
    "enum Shape { Circle { radius: float }, Rect { width: float, height: float }, Tagged { name: text } }",
];

fn session_with_defs() -> ReplSession {
    let mut s = session();
    for def in REPL_DEFS {
        assert!(
            matches!(s.eval(def), Eval::Ran),
            "def failed to eval: {def}"
        );
    }
    s
}

/// The Step-2 differential: the store-resident shadow must render EXACTLY what a
/// fresh evaluation of the same expression renders.
///
/// The oracle is `value_of(<expr>)`, not `value_of(<name>)`: reading a bound
/// *vector* by name crashes on `main` (loft#618 — the fn-return copy of a
/// borrowed local), which is pre-existing and out of @PLN14's scope.  Evaluating
/// the expression afresh exercises the same render path without that hazard.
///
/// The exact claim is a BICONDITIONAL, which is what makes it a real
/// differential: the shadow holds a value **exactly when** the REPL.X snapshot
/// path captured one.  A binding the snapshot path declines (it falls back to
/// storing the RHS as source — e.g. a vector of >32-bit literals, pre-existing)
/// must have NO env entry, not a stale or half-built one.
#[test]
fn env_shadow_agrees_with_a_fresh_evaluation() {
    let mut checked = 0;
    for (_ty, type_name, expr) in heap_cases() {
        // Pre-existing, unrelated to @PLN14 (noted on loft#618): evaluating a
        // vector-of->32-bit-literals expression TWICE in one session re-registers
        // its element type and aborts with "Double structure type" from
        // `types.rs`.  The shadow's fidelity for >32-bit ints is proven at the
        // `Stores` level instead (`materialize_gives_each_value_its_own_home`).
        if expr.contains("9000000000") {
            continue;
        }
        let mut s = session_with_defs();
        assert!(
            matches!(s.eval(&format!("v = {expr}")), Eval::Ran),
            "bind failed for `{expr}`"
        );
        let shadow = s.env_value("v");
        let fresh = s.value_of(&expr);
        match (&shadow, &fresh) {
            (Some(shadow), Some(fresh)) => {
                assert_eq!(
                    shadow, fresh,
                    "session-store shadow diverged from a fresh evaluation of \
                     `{expr}` (type {type_name})"
                );
                checked += 1;
            }
            // The snapshot path declined this shape, so there is nothing to
            // materialize — and the env must say so rather than hold a stale value.
            (None, None) => {}
            _ => panic!(
                "shadow and snapshot disagree on WHETHER `{expr}` (type \
                 {type_name}) has a value: env={shadow:?} snapshot={fresh:?}"
            ),
        }
    }
    assert!(
        checked >= 8,
        "oracle covered only {checked} cells — too few to call this a differential"
    );
}

/// The store outlives each eval's throwaway `State`: several bindings coexist and
/// stay readable after later evals have come and gone.
#[test]
fn env_entries_survive_across_evals() {
    let mut s = session_with_defs();
    assert!(matches!(s.eval("a = [1, 2, 3]"), Eval::Ran));
    assert!(matches!(s.eval("b = P { x: 7, y: 9 }"), Eval::Ran));
    assert!(matches!(s.eval("c = [\"one\", \"two\"]"), Eval::Ran));
    // Unrelated evals in between — each builds and drops its own State.
    assert!(matches!(s.eval("d = [9, 8]"), Eval::Ran));
    assert!(matches!(s.eval("n = 41 + 1"), Eval::Ran));
    assert_eq!(s.env_names(), vec!["a", "b", "c", "d"]);
    assert_eq!(s.env_value("a").as_deref(), Some("[1,2,3]"));
    assert_eq!(s.env_value("b").as_deref(), Some("P{x:7,y:9}"));
    assert_eq!(s.env_value("c").as_deref(), Some("[\"one\",\"two\"]"));
    assert_eq!(s.env_value("d").as_deref(), Some("[9,8]"));
}

/// A nested / multi-store value round-trips through the session store too — the
/// case that motivated materializing with `copy_claims` rather than a store copy.
#[test]
fn a_nested_value_round_trips_through_the_session() {
    let mut s = session_with_defs();
    assert!(matches!(
        s.eval("v = Line { a: P{x:1,y:2}, b: P{x:3,y:4}, tag: \"seg\" }"),
        Eval::Ran
    ));
    assert_eq!(
        s.env_value("v").as_deref(),
        Some("Line{a:P{x:1,y:2},b:P{x:3,y:4},tag:\"seg\"}")
    );
}

/// A re-bind replaces the entry (the old record orphans in the session store
/// until arc G collects it).
#[test]
fn re_bind_replaces_the_env_entry() {
    let mut s = session_with_defs();
    assert!(matches!(s.eval("v = [1, 2, 3]"), Eval::Ran));
    assert_eq!(s.env_value("v").as_deref(), Some("[1,2,3]"));
    assert!(matches!(s.eval("v = [4, 5]"), Eval::Ran));
    assert_eq!(s.env_value("v").as_deref(), Some("[4,5]"));
}

/// Scope boundary, pinned so it is a decision and not a surprise: scalars and
/// top-level text stay inline-only — no env entry until arc C (Step 3) boxes
/// them.  Behaviour of the replay model is unchanged for them.
#[test]
fn scalars_and_text_have_no_env_entry_yet() {
    let mut s = session();
    assert!(matches!(s.eval("n = 5"), Eval::Ran));
    assert!(matches!(s.eval("t = \"hello\""), Eval::Ran));
    assert!(matches!(s.eval("f = 1.5"), Eval::Ran));
    assert!(matches!(s.eval("b = true"), Eval::Ran));
    assert!(
        s.env_names().is_empty(),
        "scalars/text are arc C, not Step 2: {:?}",
        s.env_names()
    );
    // The replay model still has them — the shadow changed nothing.
    assert_eq!(s.value_of("n").as_deref(), Some("5"));
    assert_eq!(s.value_of("t").as_deref(), Some("\"hello\""));
    assert_eq!(s.env_value("n"), None);
}

/// The shadow is WRITE-ONLY in Step 2: the replay model is still the source of
/// truth, so a session behaves exactly as before.  (Step 5 is where observing
/// switches to the env and the body replay goes away.)
#[test]
fn the_shadow_does_not_change_session_behaviour() {
    let mut s = session_with_defs();
    assert!(matches!(s.eval("a = [1, 2, 3]"), Eval::Ran));
    assert!(matches!(s.eval("n = 10"), Eval::Ran));
    assert!(matches!(s.eval("assert(n == 10, \"n\")"), Eval::Ran));
    assert!(matches!(s.eval("n = n + 5"), Eval::Ran));
    assert!(matches!(s.eval("assert(n == 15, \"n grew\")"), Eval::Ran));
    assert_eq!(s.value_of("n").as_deref(), Some("15"));
}
