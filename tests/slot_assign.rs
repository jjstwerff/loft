// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Integration tests that exercise the slot-assignment pass for code patterns that were
//! historically prone to producing overlapping stack slots.  Each test runs the full
//! pipeline (parse → scope-check → codegen → execute) and must complete without a
//! `validate_slots` panic.

extern crate loft;

mod testing;

/// An integer accumulator (`best`) is alive across the whole function while a struct
/// reference loop element (`n`, a DbRef) is alive only inside the loop body.
/// The integer slot must not fall inside the 12-byte DbRef slot range.
#[test]
fn int_accumulator_and_struct_loop_element() {
    code!(
        "struct Node { val: integer }

fn find_max(nodes: vector<Node>) -> integer {
    best = nodes[0].val;
    for n in nodes {
        if n.val > best { best = n.val }
    };
    best
}

fn test() {
    nodes = [Node{val:3}, Node{val:10}, Node{val:7}];
    assert(find_max(nodes) == 10, \"Expected 10\");
}"
    );
}

/// A struct reference (`e`) from before the loop remains live while the loop element
/// (`el`, also a DbRef) is alive.  Both are DbRef-sized; they must occupy distinct slots.
#[test]
fn ref_before_loop_and_loop_element() {
    code!(
        "struct Elm { a: integer, b: integer }

fn sum_with_first(v: vector<Elm>) -> integer {
    e = v[0];
    t = 0;
    for el in v { t += el.a + el.b };
    t + e.a
}

fn test() {
    v = [Elm{a:1, b:2}, Elm{a:12, b:13}, Elm{a:4, b:5}];
    assert(sum_with_first(v) == 38, \"Expected 38\");
}"
    );
}

/// A long-lived integer accumulator stays alive while a struct record is appended to a
/// vector (which involves a CopyRecord).  After the CopyRecord, a reference into the
/// just-appended record is stored in a variable.  The reference variable must not be
/// allocated at a slot that overlaps the accumulator.
///
/// This is the pattern in `lib/code.loft::define` that triggers the `t_4Code_define`
/// slot conflict.  The `validate_slots` panic is fixed (Option A sub-3 pre-init), but
/// the test still fails at runtime: a borrowed Reference pre-init via `OpCreateStack`
/// produces a garbage DbRef (store_nr=8) that is read instead of the real value.
/// See `doc/claude/ASSIGNMENT.md` §"Option A sub-option 3" for the investigation path.
#[test]
fn long_lived_int_and_copy_record_followed_by_ref() {
    code!(
        "struct Item { val: integer }
struct Bag  { items: vector<Item>, extra: Item }

fn process(b: Bag) -> integer {
    result = 0;
    if b.items[0].val > 0 {
        result = b.items[0].val;
    } else {
        b.items += [b.extra];
        last = b.items[b.items.len() - 1];
        result = last.val;
    };
    result
}

fn test() {
    extra = Item { val: 42 };
    b = Bag { items: [], extra: extra };
    assert(process(b) == 42, \"Expected 42\");
}"
    );
}

/// Nested loops: the outer element (`p`) must not share a slot with the inner element (`q`)
/// while the inner loop executes, as both are DbRef references alive at the same time.
#[test]
fn nested_loop_struct_elements() {
    code!(
        "struct Pair { x: integer, y: integer }

fn sum_matching(pairs: vector<Pair>, xs: vector<Pair>) -> integer {
    total = 0;
    for p in pairs {
        for q in xs {
            if p.x == q.x { total = total + p.y }
        }
    };
    total
}

fn test() {
    pairs = [Pair{x:1, y:10}, Pair{x:2, y:20}];
    xs    = [Pair{x:1, y:0}];
    assert(sum_matching(pairs, xs) == 10, \"Expected 10\");
}"
    );
}

/// Post-loop variable reuses the dead loop-element slot — this must be ALLOWED.
///
/// After the loop ends, the loop element (`v`) is no longer live.  A subsequent variable
/// (`summary`) should be free to occupy the same stack slot.  This is the pattern exercised
/// by the `polymorph` enum test.
///
/// A naive "advance stack.position to max-assigned-slot" fix would incorrectly block this
/// reuse and break the `polymorph` test.  The slot assignment must rely on LIVE INTERVALS,
/// not just on which slots have ever been assigned.
#[test]
fn post_loop_variable_reuses_dead_loop_slot() {
    code!(
        "struct Point { x: integer, y: integer }

fn sum_points(pts: vector<Point>) -> integer {
    acc = 0;
    for p in pts {
        acc += p.x + p.y;
    };
    acc
}

fn test() {
    pts = [Point{x:1, y:2}, Point{x:3, y:4}];
    total = sum_points(pts);
    assert(total == 10, \"Expected 10\");
}"
    );
}

// ── A6.2 shadow-comparison tests ─────────────────────────────────────────────

/// Sequential primitives whose live intervals do not overlap.  `assign_slots` will
/// reuse slot 0 for all three; `claim()` assigns sequential slots 4/8/12.  The shadow
/// comparison logs the mismatch but must not panic — the function must still execute
/// correctly.
#[test]
fn shadow_comparison_sequential_dead_primitives() {
    code!(
        "fn add_chain() -> integer {
    x = 3;
    y = x + 1;
    z = y * 2;
    z
}

fn test() {
    assert(add_chain() == 8, \"Expected 8\");
}"
    );
}

/// A function with both alive-across-call integer accumulators and text variables.
/// Verifies assign_slots' shadow result is conflict-free (validate_slots does not panic).
#[test]
fn shadow_comparison_text_and_integer_mixed() {
    code!(
        "fn greet(name: text) -> text {
    msg = \"Hello \";
    msg += name;
    msg
}

fn test() {
    result = greet(\"world\");
    assert(result == \"Hello world\", \"Expected 'Hello world'\");
}"
    );
}

// ── A6.3 claim-free codegen tests ────────────────────────────────────────────

/// Sequential primitives must be placed at correct slots and produce correct results
/// with the new `is_stack_allocated` gate replacing the old `pos == u16::MAX` check.
#[test]
fn claim_free_sequential_primitives() {
    code!(
        "fn compute() -> integer {
    a = 10;
    b = a * 2;
    c = b - 5;
    c
}

fn test() {
    assert(compute() == 15, \"Expected 15\");
}"
    );
}

/// Text-variable first allocation via the `stack_allocated` flag must emit `OpText`
/// exactly once and allow subsequent appends.
#[test]
fn claim_free_text_variable_first_alloc() {
    code!(
        "fn build(prefix: text, n: integer) -> text {
    result = prefix;
    result += \" \";
    result += \"{n}\";
    result
}

fn test() {
    assert(build(\"count\", 42) == \"count 42\", \"Expected 'count 42'\");
}"
    );
}
