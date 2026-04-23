// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Phase 0 fixture catalogue for the slot-assignment redesign (plan 04).
//!
//! Every fixture in this file locks the exact slot layout that today's
//! two-zone allocator assigns for one specific IR shape.  The layouts are
//! the ground truth that the single-pass V2 allocator (Phase 2) must
//! reproduce before the Phase 3 switchover.
//!
//! Each fixture carries a rationale in its doc comment describing which
//! V1 mechanism produced the placement and what invariant V2 must
//! preserve.  If a rationale only references a V1 mechanism
//! (e.g. "because `place_orphaned_vars` starts at `local_start`"), the
//! fixture is a candidate for rewrite into an invariant assertion
//! ("orphan locals must not overlap the argument region") — see
//! `doc/claude/plans/finished/04-slot-assignment-redesign/00-characterize.md`
//! step 3.
//!
//! Pattern coverage (drawn from [SLOTS.md] Known Patterns plus the open
//! regressions flagged by the Phase 0 audit):
//!
//! | # | Pattern | SLOTS.md row | Fixture |
//! |---|---------|--------------|---------|
//! | 1 | Zone 1 reuse across non-overlapping integers | — | `zone1_reuse_two_ints_same_block` |
//! | 2 | Loop-scope small vars placed sequentially | — | `loop_scope_small_vars_sequential` |
//! | 3 | Text block-return vs child text | 131 | `text_block_return_vs_child_text` |
//! | 4 | Insert preamble (P135 lift) | 128 | `insert_preamble_lift_ordering` |
//! | 5 | Sibling scope reuse | 132 | `sibling_scopes_share_frame_area` |
//! | 6 | Sequential lifted calls | 129 | `sequential_lifted_calls` |
//! | 7 | Comprehension then literal (P122p) | 133 | `p122p_comprehension_then_literal` |
//! | 8 | Sorted range comprehension (P122q) | 134 | `p122q_sorted_range_comprehension` |
//! | 9 | Par loop with inner for (P122r) | 135 | `p122r_par_loop_with_inner_for` (ignored — codegen panic) |
//! | 10 | Many parent refs + child loop index | 126 | `parent_refs_plus_child_loop_index` |
//! | 11 | Call with Block arg | 127 | `call_with_block_arg` |
//! | 12 | Parent var Set inside child scope | 130 | `parent_var_set_inside_child_scope` |
//! | 13 | P178 — is-capture in Insert-rooted body | — | `p178_is_capture_body` |
//! | 14 | P185 — late local after inner text-accum loop | — | `p185_late_local_after_inner_loop` (ignored — V2 needed) |
//! | 15 | Function with only args (no locals) | — | `fn_with_only_arguments` |
//! | 16 | Nested if with block branches (no overlap) | — | `nested_if_block_branches` |
//! | 17 | Large vector followed by small int (layout) | — | `large_vector_then_small_int` |
//! | 18 | Two sibling blocks with shared outer var | — | `two_sibling_blocks_shared_outer` |
//! | 19 | For-loop with two loop-scope locals | — | `for_loop_two_loop_locals` |
//! | 20 | Nested for in for (two loop scopes) | — | `nested_for_in_for` |
//! | 21 | Match with per-arm bindings | — | `match_with_arm_bindings` |
//! | 22 | Struct block-return (non-Text) | — | `struct_block_return_non_text` |
//! | 23 | Nested call chain `f(g(h(x)))` | — | `nested_call_chain` |
//! | 24 | Vector accumulator loop (`acc += [...]`) | — | `vector_accumulator_loop` |
//! | 25 | Early return from nested scope | — | `early_return_from_nested_scope` |
//! | 26 | Method-mutation extends var lifetime | — | `method_mutation_extends_lifetime` |
//! | 27 | Kind/size mismatch blocks RefSlot reuse (I5) | — | `kind_mismatch_no_reuse` |
//! | 28 | Block-return with early exit from enclosing fn | — | `block_return_with_early_exit` |
//! | 29 | Loop-carry of a parent scalar (minimal I6) | — | `loop_carry_parent_scalar_explicit` |
//!
//! ## Workflow for adding a fixture (Phase 2d onwards)
//!
//! 1. Write the `.loft` snippet inside `code!(...)`.
//! 2. Call `.invariants_pass()` to document intent — `validate_slots`
//!    during codegen is the actual check and panics with a distinct
//!    `[I1]` … `[I6]` prefix on any invariant violation.
//! 3. Add a runtime `assert(...)` inside the loft snippet if the
//!    fixture's pattern has observable behaviour (most do — e.g.
//!    the accumulator sum, the struct field values, …).
//!
//! Phase 2d retired the byte-exact `.slots("…layout…")` locks: the
//! invariant-based checks catch every class of placement bug the
//! layout locks caught, without forcing V2 to reproduce V1's
//! historical quirks.  Under `LOFT_SLOT_V2=validate` (Phase 2e)
//! `assign_slots_v2` runs alongside V1 and its output also passes
//! through `validate_slots`.

extern crate loft;

mod testing;

// ── Pattern 1 ───────────────────────────────────────────────────────────────
// Zone 1 reuse across non-overlapping small integers.
//
// Rationale (V1): two integers in the same Block with disjoint
// live-intervals are greedy-coloured onto the same Zone 1 slot.  The
// invariant V2 must preserve is that disjoint small-int intervals can
// share a slot (otherwise the frame grows unboundedly).

#[test]
fn zone1_reuse_two_ints_same_block() {
    code!(
        "fn test() {
    a = 1;
    _ = a;
    b = 2;
    _ = b;
}"
    )
    .invariants_pass();
}

// ── Pattern 2 ───────────────────────────────────────────────────────────────
// Loop-scope small vars placed sequentially, NOT greedy-coloured into
// zone 1.  A loop body re-enters its scope every iteration, so
// per-iteration OpFreeStack would corrupt still-live zone-1 slots from
// earlier iterations.
//
// Rationale (V1): `is_loop_scope(scope)` path in `slots.rs:216` bypasses
// zone 1 and places loop-scope small vars at TOS sequentially.  V2 must
// still avoid reusing a loop-scope slot across iterations.

#[test]
fn loop_scope_small_vars_sequential() {
    code!(
        "fn test() {
    s = 0;
    for i in 0..3 {
        x = i + 1;
        y = i + 2;
        s += x + y;
    }
    assert(s == 15, \"sum\");
}"
    )
    .invariants_pass();
}

// ── Pattern 3 ───────────────────────────────────────────────────────────────
// Text block-return vs child text.  When a Text variable is assigned
// from a Block expression, the child Block's own Text locals must not
// overlap the target: codegen emits OpText before entering the Block,
// so the target must be at TOS and the child's texts placed above it.
//
// Rationale (V1): `slots.rs:235–236` excludes Text from block-return
// frame-sharing.  V2 must preserve "parent Text and child Text cannot
// alias when both are live across the block boundary."

#[test]
fn text_block_return_vs_child_text() {
    code!(
        "fn test() {
    tv = { a = \"12\"; \"[\" + a + \"]\" };
    assert(tv == \"[12]\", \"got {tv}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 4 ───────────────────────────────────────────────────────────────
// Insert preamble (P135 lift).  An interpolated string `"{f(x)}"` is
// flattened into `Value::Insert([Set(__lift_N, f(x)), ...])` by
// `scopes.rs::scan_set`.  The lifted locals end up as orphans (the
// function root is Insert, not Block) and are placed by
// `place_orphaned_vars` above `local_start`.
//
// Rationale (V1): orphan placer with local_start floor.  V2 must place
// the lift targets disjoint from args / return-address.

#[test]
fn insert_preamble_lift_ordering() {
    code!(
        "fn lift(n: integer) -> integer { n + 1 }
fn test() {
    s = \"{lift(10)}-{lift(20)}\";
    assert(s == \"11-21\", \"got {s}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 5 ───────────────────────────────────────────────────────────────
// Sibling scope reuse.  Two Block arms of an If-expression have
// disjoint runtime lifetimes, so their locals can share slots.
//
// Rationale (V1): both arms call `process_scope` with the same starting
// `tos`, so sibling locals collapse onto the same frame area.  V2's
// liveness-based colouring must reproduce this collapse.

#[test]
fn sibling_scopes_share_frame_area() {
    code!(
        "fn test() {
    cond = true;
    v = if cond { a = 10; a * 2 } else { b = 5; b + 1 };
    assert(v == 20, \"got {v}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 6 ───────────────────────────────────────────────────────────────
// Sequential lifted calls.  Two consecutive `body += "{call()}"`
// statements both produce Insert preambles; V1 places each __work_N at
// a separate slot because their live intervals overlap the surrounding
// Text accumulator.
//
// Rationale (V1): `place_orphaned_vars` assigns in first_def order,
// with per-orphan conflict scanning.  V2's liveness-based approach
// must give each concurrent-lift its own slot.

#[test]
fn sequential_lifted_calls() {
    code!(
        "fn pad(n: integer) -> text { \"[{n}]\" }
fn test() {
    body = \"\";
    body += pad(1);
    body += pad(2);
    body += pad(3);
    assert(body == \"[1][2][3]\", \"got {body}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 7 ───────────────────────────────────────────────────────────────
// Comprehension then literal (P122p).  A vector comprehension
// `[for i in 0..N { f(i) }]` lifts into a Block that evaluates at an
// argument position; the following plain literal `[1, 2, 3]` must not
// alias the comprehension's iteration temporaries.
//
// Rationale (V1): `place_large_and_recurse` walks Call args before
// placing the outer result.  V2 must keep the comprehension's
// temporaries disjoint from the subsequent literal's.

#[test]
fn p122p_comprehension_then_literal() {
    code!(
        "fn test() {
    a =[for i in 0..3 { i * 10 }];
    b =[1, 2, 3];
    assert(a[0] == 0 and a[2] == 20, \"a\");
    assert(b[0] == 1 and b[2] == 3, \"b\");
}"
    )
    .invariants_pass();
}

// ── Pattern 8 ───────────────────────────────────────────────────────────────
// Sorted range comprehension (P122q).  A comprehension over a sorted
// range produces multiple intermediate locals during Insert lowering.
//
// Rationale (V1): Insert preamble + zone-1 / zone-2 interplay.  V2
// must place sorted-range temporaries without a shape-dispatched
// branch.

#[test]
fn p122q_sorted_range_comprehension() {
    code!(
        "fn test() {
    xs =[for i in 0..5 { i * i }];
    total = 0;
    for v in xs { total += v; }
    assert(total == 30, \"got {total}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 9 ───────────────────────────────────────────────────────────────
// Par-loop with inner for (P122r).  A `par` loop body that contains an
// inner sequential `for` creates nested loop scopes.  The inner loop's
// loop-scope vars must not overlap the par iterator's index.
//
// Rationale (V1): nested `is_loop_scope` handling plus per-thread
// frame allocation (see THREADING.md).  V2 must keep inner-loop
// locals isolated from outer-loop iteration state.

// The layout below is locked, but codegen currently panics on this
// P122r: `Incorrect var a[65535] versus N` codegen panic fixed
// 2026-04-23 by extending plan-04 B.3 follow-up v2's inline-alias
// treatment to the outer iterator `a` (not just the inner `b`).
// `parse_parallel_for_loop` / `build_parallel_for_ir` in
// `src/parser/collections.rs` now rewrite every `Value::Var(a)`
// in the body to `OpGetVector(items, elem_size, idx)` (plus
// `get_field` for non-Reference element types) so `a` never
// needs a slot.  The desugared par loop owns `idx` / `results`;
// `a` is purely syntactic sugar.
#[test]
fn p122r_par_loop_with_inner_for() {
    code!(
        "struct P04Item { iv: integer not null }
fn p04_double(p: const P04Item) -> integer { p.iv * 2 }
fn test() {
    items =[P04Item { iv: 1 }, P04Item { iv: 2 }, P04Item { iv: 3 }, P04Item { iv: 4 }];
    total = 0;
    for a in items par(b = p04_double(a), 2) {
        inner = 0;
        for j in 0..3 { inner += j * a.iv; }
        total += b + inner;
    }
    assert(total == 50, \"got {total}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 10 ──────────────────────────────────────────────────────────────
// Many parent refs + child loop index.  Multiple reference-valued
// parent locals that remain live across a child for-loop must not
// overlap the loop's index slot.
//
// Rationale (V1): zone 2 references in the parent + zone-1 index in
// the child must place the index above all still-live parent slots.
// V2 must give the same answer without dispatching on "parent refs."

#[test]
fn parent_refs_plus_child_loop_index() {
    code!(
        "struct P04Cell { val: integer not null }
fn test() {
    a = P04Cell { val: 1 };
    b = P04Cell { val: 2 };
    c = P04Cell { val: 3 };
    d = P04Cell { val: 4 };
    s = 0;
    for i in 0..3 { s += a.val + b.val + c.val + d.val + i; }
    assert(s == 33, \"got {s}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 11 ──────────────────────────────────────────────────────────────
// Call with Block arg.  `f([for i in 0..3 { i }])` evaluates the
// Block argument before placing the Call's result slot; the Block's
// internal temporaries must live below the Call's result.
//
// Rationale (V1): Call args walked first in `place_large_and_recurse`.
// V2's liveness-based ordering must reproduce "inner-Block locals
// precede outer-Call locals."

#[test]
fn call_with_block_arg() {
    code!(
        "fn sum_vec(v: vector<integer>) -> integer {
    t = 0;
    for x in v { t += x; }
    t
}
fn test() {
    total = sum_vec([for i in 0..5 { i + 1 }]);
    assert(total == 15, \"got {total}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 12 ──────────────────────────────────────────────────────────────
// Parent var Set inside child scope.  A parent-scope variable assigned
// from inside a child Block.  V1 collects it via `place_orphaned_vars`
// because the child-scope IR walk never sees the parent var.
//
// Rationale (V1): orphan path.  V2 must place parent vars written
// through child scopes without a separate orphan pass.

#[test]
fn parent_var_set_inside_child_scope() {
    code!(
        "fn test() {
    result = 1;
    {
        inner = 7;
        result = result + inner * 5;
    }
    assert(result == 36, \"got {result}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 13 ──────────────────────────────────────────────────────────────
// P178 — `is`-capture slot alias.  Captured binder `tb_id` from the
// `is`-pattern is defined inside the If-then branch of a function
// whose body root is a `Value::Insert` rather than a `Block`.
// `process_scope` returned early, the binder became an orphan, and
// `place_orphaned_vars` (pre-P178 patch) placed it starting at slot 0
// — overlapping the `tools` reference argument.  Fixed by
// threading `local_start` into the orphan placer.
//
// This fixture locks the POST-PATCH placement (binder above the
// argument region) so V2 reproduces the fix without the
// V1-specific mechanism.

#[test]
fn p178_is_capture_body() {
    code!(
        "enum P04Ui { P04UhButton { tb_id: integer not null } }
struct P04Tools { ft_cur: integer not null }
fn p04_hit() -> P04Ui { P04UhButton { tb_id: 2 } }
fn p04_router(dummy: integer, tools: &P04Tools) -> P04Ui {
    _ = dummy;
    rc = p04_hit();
    if rc is P04UhButton { tb_id } {
        tools.ft_cur = tb_id;
    }
    rc
}
fn test() {
    ft = P04Tools { ft_cur: 0 };
    p04_router(99, ft);
    assert(ft.ft_cur == 2, \"captured was {ft.ft_cur} (expected 2)\");
}"
    )
    .invariants_pass();
}

// ── Pattern 14 ──────────────────────────────────────────────────────────────
// P185 — late local after inner text-accumulator loop.  A local
// (`key`) declared AFTER an inner `body += "{...}"` accumulator loop,
// inside an outer `for _ in file(...).files()` with an inline
// temporary iterator source, is placed by V1's orphan pass onto a slot
// still used by a live text buffer.  Scope teardown's OpFreeText
// corrupts the aliased slot.
//
// V1 is BROKEN here — this fixture is the only one in the catalogue
// that currently ASSERTS the bug's symptom is observable.  Phase 3
// un-`#[ignore]`s it after V2 picks a non-overlapping slot.
//
// Until V2 lands we cannot lock a good layout, so the fixture is
// `#[ignore]`d and `slots("")` is a placeholder.

#[test]
fn p185_late_local_after_inner_loop() {
    code!(
        "fn test() {
    out = file(\"/tmp/p04_out.txt\");
    for f in file(\"tests/docs\").files() {
        path = \"{f.path}\";
        if !path.ends_with(\".loft\") or path.ends_with(\"/.loft\") { continue; }
        body = \"\";
        for i in 0..3 {
            body += \"{i}\";
        }
        key = path[path.find(\"/\") + 1..path.len() - 5];
        out += `
          {key}
        `;
        break;
    }
}"
    )
    .invariants_pass();
}

// ── Pattern 15 ──────────────────────────────────────────────────────────────
// Local placed after args-heavy function signature.  `test` takes
// nothing, but its single local `r` captures a helper call that
// itself has three integer args.  The fixture's role is to lock
// `test`'s local against any args-vs-locals regression (P178 class).

#[test]
fn fn_with_only_arguments() {
    code!(
        "fn add3(a: integer, b: integer, c: integer) -> integer { a + b + c }
fn test() {
    r = add3(1, 2, 3);
    assert(r == 6, \"sum\");
}"
    )
    .invariants_pass();
}

// ── Pattern 16 ──────────────────────────────────────────────────────────────
// Nested If with Block branches.  Each branch's locals share the
// sibling frame area, but outer locals assigned from the If result
// must not alias any branch-local slot.

#[test]
fn nested_if_block_branches() {
    code!(
        "fn test() {
    x = 3;
    r = if x > 0 {
        if x < 10 { a = x * 2; a + 1 } else { b = x; b - 1 }
    } else {
        c = 99; c
    };
    assert(r == 7, \"got {r}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 17 ──────────────────────────────────────────────────────────────
// Large vector followed by small int.  A vector local (zone 2, 16 B
// DbRef) followed by a plain integer; V1 places them in separate
// zones.  V2 must place them non-overlapping regardless of zone.

#[test]
fn large_vector_then_small_int() {
    code!(
        "fn test() {
    v =[1, 2, 3];
    n = v[0] + v[1] + v[2];
    assert(n == 6, \"got {n}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 18 ──────────────────────────────────────────────────────────────
// Two sibling Block statements that both assign to a shared outer
// variable.  The outer variable's slot must stay fixed across both
// blocks; each block's local temporaries may reuse the same area.

#[test]
fn two_sibling_blocks_shared_outer() {
    code!(
        "fn test() {
    result = 100;
    { a = 10; result += a; }
    { b = 20; result += b; }
    assert(result == 130, \"got {result}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 19 ──────────────────────────────────────────────────────────────
// For-loop with two loop-scope locals.  Both must place sequentially
// (no zone-1 reuse across iterations) and not overlap each other.

#[test]
fn for_loop_two_loop_locals() {
    code!(
        "fn test() {
    total = 0;
    for i in 0..4 {
        a = i * 2;
        b = a + 1;
        total += b;
    }
    assert(total == 16, \"got {total}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 20 ──────────────────────────────────────────────────────────────
// Nested for in for.  Two loop scopes, each with its own index and
// body locals.  Inner loop locals must not overlap outer loop
// locals that are live across iterations.

#[test]
fn nested_for_in_for() {
    code!(
        "fn test() {
    grid = 0;
    for i in 0..3 {
        row = 0;
        for j in 0..3 {
            row += i * 10 + j;
        }
        grid += row;
    }
    assert(grid == 99, \"got {grid}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 21 ──────────────────────────────────────────────────────────────
// Match with per-arm bindings.  Each arm introduces its own binder
// local; these binders share a lifetime window bounded by the arm but
// live at different scopes.  A uniform allocator must place them
// disjointly from the scrutinee and the match-wide result local,
// without a match-specific branch.

#[test]
fn match_with_arm_bindings() {
    code!(
        "enum P04Msg {
    P04Tag { tag_id: integer not null },
    P04Pair { pa: integer not null, pb: integer not null },
    P04Other
}
fn test() {
    m = P04Pair { pa: 3, pb: 4 };
    r = match m {
        P04Tag { tag_id } => tag_id + 1,
        P04Pair { pa, pb } => pa * 10 + pb,
        P04Other => 999
    };
    assert(r == 34, \"got {r}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 22 ──────────────────────────────────────────────────────────────
// Struct block-return (non-Text).  `Set(v, Block([..., struct_expr]))`
// where v is a struct-typed local.  V1's `slots.rs:235–236` gates
// frame-sharing on `!Type::Text`, so structs take the frame-sharing
// path — opposite of `text_block_return_vs_child_text`.  V2 must
// reproduce placement without a type-case branch on Text.
//
// Regression guard for P186 (`{ S { … } }` used to infer `void`);
// see `tests/issues.rs::p186_struct_typed_block_expressions` for the
// runtime correctness assertion.

#[test]
fn struct_block_return_non_text() {
    code!(
        "struct P04Box { bx_w: integer not null, bx_h: integer not null }
fn test() {
    b = { n = 3; P04Box { bx_w: n, bx_h: n * 2 } };
    assert(b.bx_w == 3 and b.bx_h == 6, \"got w={b.bx_w} h={b.bx_h}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 23 ──────────────────────────────────────────────────────────────
// Nested call chain `f(g(h(x)))`.  Each intermediate call's result
// becomes a `__work_N` local; their live intervals nest strictly
// (inner dies before outer can be consumed).  V1 places them in
// call-order; V2's liveness-based placement should allow slot reuse
// across intervals that happen to be adjacent but non-overlapping.

#[test]
fn nested_call_chain() {
    code!(
        "fn p04_h(n: integer) -> integer { n + 1 }
fn p04_g(n: integer) -> integer { n * 2 }
fn p04_f(n: integer) -> integer { n - 3 }
fn test() {
    r = p04_f(p04_g(p04_h(5)));
    assert(r == 9, \"got {r}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 24 ──────────────────────────────────────────────────────────────
// Vector accumulator loop.  `acc =[]; for x in src { acc +=[x * 2] }`.
// `acc` lives across every loop iteration and is mutated through a
// method-like append; V2 must keep `acc` placed outside the loop-scope
// reuse area, and the RHS literal `[x*2]` must not alias anything live.

#[test]
fn vector_accumulator_loop() {
    code!(
        "fn test() {
    src =[1, 2, 3, 4];
    acc =[];
    for x in src { acc +=[x * 2]; }
    sum = 0;
    for v in acc { sum += v; }
    assert(sum == 20, \"got {sum}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 25 ──────────────────────────────────────────────────────────────
// Early return from nested scope.  A helper function returns early
// from inside an If-branch; the caller's post-call locals are placed
// as though the helper returned normally.  This exercises allocator
// behaviour at the control-flow boundary — V2 must not confuse an
// early-return arm's locals with locals in the fall-through path.

#[test]
fn early_return_from_nested_scope() {
    code!(
        "fn p04_find(n: integer, limit: integer) -> integer {
    if n < 0 { return -1; }
    acc = 0;
    for i in 0..limit {
        acc += i;
        if acc > n { return i; }
    }
    -2
}
fn test() {
    a = p04_find(10, 20);
    b = p04_find(-5, 20);
    c = p04_find(10000, 5);
    assert(a == 5, \"a={a}\");
    assert(b == -1, \"b={b}\");
    assert(c == -2, \"c={c}\");
}"
    )
    .invariants_pass();
}

// ── Pattern 26 ──────────────────────────────────────────────────────────────
// Method-mutation extends var lifetime.  A vector `v` that would
// otherwise die after its last read is kept live by an intervening
// `v +=[item]` call.  V1's interval analysis should catch the extended
// liveness; V2 must agree, otherwise codegen places a later local into
// v's slot and corrupts the vector.

#[test]
fn method_mutation_extends_lifetime() {
    code!(
        "fn test() {
    v =[10, 20, 30];
    first = v[0];
    v +=[40];
    mid = v[2];
    v +=[50];
    last = v[4];
    sum = first + mid + last;
    assert(sum == 90, \"got {sum} (first {first} mid {mid} last {last})\");
}"
    )
    .invariants_pass();
}

// ── Pattern 27 ──────────────────────────────────────────────────────────────
// Kind-mismatch / size-mismatch no-reuse (invariant I5).  After a 24-B
// Text dies, a 12-B DbRef (vector) and an 8-B integer are created.
// V2 must NOT place the 12-B DbRef or the 8-B int at the 24-B Text's
// former slot: (a) cross-kind reuse is forbidden (Inline vs RefSlot)
// and (b) cross-size RefSlot reuse is forbidden (step 5b/5c of the
// algorithm).  Under V1 this is "accidentally correct" because V1's
// zone split keeps different sizes on separate slots anyway; under V2
// it is a direct invariant check.

#[test]
fn kind_mismatch_no_reuse() {
    code!(
        "fn test() {
    t = \"hello\";
    n = t.len() + 1;
    _ = t;
    v =[n, n + 1, n + 2];
    i = v[0];
    assert(n == 6, \"len+1\");
    assert(v[0] == 6, \"v[0]\");
    assert(i == 6, \"i\");
}"
    )
    .invariants_pass();
}

// ── Pattern 28 ──────────────────────────────────────────────────────────────
// Block-return with early exit from the enclosing function.  A block
// on the RHS of an assignment contains a conditional `return` that
// exits the whole function, not just the block.  When the return
// fires, the outer `Set(x, …)` never completes.  V2 must place `x`
// correctly even though its `first_def` may or may not execute at
// runtime (the allocator sees only `compute_intervals`' static
// record; control flow is irrelevant to placement).

#[test]
fn block_return_with_early_exit() {
    code!(
        "fn p04_blockret(c: boolean) -> integer {
    x = {
        n = 7;
        if c { return -1; }
        n * 2
    };
    x + 100
}
fn test() {
    a = p04_blockret(false);
    b = p04_blockret(true);
    assert(a == 114, \"normal path\");
    assert(b == -1, \"early-return path\");
}"
    )
    .invariants_pass();
}

// ── Pattern 29 ──────────────────────────────────────────────────────────────
// Minimal loop-iteration-safety (invariant I6).  A parent-scope
// accumulator defined BEFORE a loop and read AFTER must not share a
// slot with any loop-body local.  `intervals.rs::compute_intervals`
// extends the accumulator's `last_use` past the loop end; V2's
// colouring reads that extended range and places loop-body locals
// above it.  Simpler variant of fixture 2 — one body local, one
// carried var — that makes the I6 shape unambiguous.

#[test]
fn loop_carry_parent_scalar_explicit() {
    code!(
        "fn test() {
    acc = 0;
    for i in 0..5 {
        tmp = i * 2;
        acc += tmp;
    }
    assert(acc == 20, \"got {acc}\");
}"
    )
    .invariants_pass();
}
