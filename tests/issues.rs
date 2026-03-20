// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Minimal reproducing tests for known open issues in the loft runtime.
//! Each test isolates exactly the bug pattern described in doc/claude/PROBLEMS.md.
//! Broken tests are marked #[ignore] so they are tracked but do not break CI.

extern crate loft;

mod testing;

use loft::compile::byte_code;
use loft::data::Value;
use loft::logger::{Logger, RuntimeLogConfig};
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
use std::sync::{Arc, Mutex};

// ── Issue 3 ──────────────────────────────────────────────────────────────────
// Polymorphic text methods on struct-enum variants → stack overflow at state.rs:2070.
// `text_return` adds RefVar(Text) attributes to variant functions in the second pass,
// but enum_fn only runs in the first pass, so the Dynamic dispatch IR still calls
// with only [Var(0)] despite each variant now needing extra text-buffer arguments.

#[test]
fn polymorphic_text_method_on_enum() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect   { width: float, height: float }
}
fn describe(self: Circle) -> text { \"circle r={self.radius}\" }
fn describe(self: Rect)   -> text { \"rect {self.width}x{self.height}\" }
fn test() {
    c = Circle { radius: 3.0 };
    assert(c.describe() == \"circle r=3\", \"got: {c.describe()}\");
}"
    );
}

// ── Issue 5 ──────────────────────────────────────────────────────────────────
// Scalar `+=` on an empty (null) vector struct field has no effect.
// Expected: the scalar is appended and len == 1.

/// `b.items += [1]` (bracket form) on a null field — this is the WORKING baseline.
/// The bracket form goes through parse_vector with is_field=true and uses
/// OpNewRecord / OpFinishRecord to allocate the element in place.
#[test]
fn vec_field_append_bracket_scalar_works() {
    code!(
        "struct Box { items: vector<integer> }
fn test() {
    b = Box {};
    b.items += [1];
    assert(len(b.items) == 1, \"len after += [1]: {len(b.items)}\");
    assert(b.items[0] == 1,   \"value after += [1]: {b.items[0]}\");
}"
    );
}

/// `b.items += [3, 5]` on a null field — multiple elements with bracket form.
#[test]
fn vec_field_append_bracket_multi_works() {
    code!(
        "struct Box { items: vector<integer> }
fn test() {
    b = Box {};
    b.items += [3, 5];
    assert(len(b.items) == 2, \"len: {len(b.items)}\");
    assert(b.items[0] == 3, \"[0]: {b.items[0]}\");
    assert(b.items[1] == 5, \"[1]: {b.items[1]}\");
}"
    );
}

/// `b.items += 1` (bare scalar, no brackets) on a null field — FIXED.
/// Parser now routes through new_record so the field is allocated in place.
/// Was tracked as Issue 5 in doc/claude/PROBLEMS.md.
#[test]
fn vec_field_append_scalar() {
    code!(
        "struct Box { items: vector<integer> }
fn test() {
    b = Box {};
    b.items += 1;
    assert(len(b.items) == 1, \"len after += 1: {len(b.items)}\");
    assert(b.items[0] == 1,   \"value after += 1: {b.items[0]}\");
}"
    );
}

// ── Issue 1 ──────────────────────────────────────────────────────────────────
// A method whose return type is a NEW struct record crashes at database.rs:1494
// because the DbRef returned by the method has a garbage store_nr.

/// Minimal reproducer: `fn double(self: Color) -> Color { Color { r: self.r * 2 } }`
/// Calling `c.double()` crashes with "index out of bounds: the len is N but index is M".
/// Tracked as Issue 1 in doc/claude/PROBLEMS.md.
#[test]
fn method_returns_new_struct_record() {
    code!(
        "struct Color { r: integer not null }
fn double(self: Color) -> Color { Color { r: self.r * 2 } }
fn test() {
    c = Color { r: 3 };
    d = c.double();
    assert(d.r == 6, \"d.r after double: {d.r}\");
}"
    );
}

// ── Issue 2 ──────────────────────────────────────────────────────────────────
// A borrowed reference first assigned inside a branch gets a garbage store_nr=8
// DbRef at runtime, crashing at database.rs:1462.
// Owned references are correctly pre-initialized (Option A sub-3); borrowed refs are not.

/// Borrowed ref first assigned INSIDE an `if` branch — FIXED.
/// Was tracked as Issue 2 in doc/claude/PROBLEMS.md; now passes after
/// the Option A sub-3 pre-init work in scopes.rs.
#[test]
fn ref_inside_branch_borrowed() {
    code!(
        "struct Item { val: integer }
fn test() {
    items = [Item { val: 10 }, Item { val: 20 }];
    result = 0;
    if items[0].val > 0 {
        r = items[0];
        result = r.val;
    };
    assert(result == 10, \"result: {result}\");
}"
    );
}

// ── Issue 4 ──────────────────────────────────────────────────────────────────
// `v += items` inside a function that takes `v` as a `&vector<T>` ref-param
// has no visible effect on the caller's variable after the call returns.

/// Baseline: field mutation through a ref-param WORKS (e.g. `v[0].val = x`).
#[test]
fn ref_param_field_mutation_works() {
    code!(
        "struct Item { val: integer }
fn set_first(v: &vector<Item>, x: integer) { v[0].val = x; }
fn test() {
    buf = [Item { val: 1 }];
    set_first(buf, 42);
    assert(buf[0].val == 42, \"buf[0].val: {buf[0].val}\");
}"
    );
}

// ── Issue 44 ─────────────────────────────────────────────────────────────────
// Empty vector literal `[]` cannot be passed directly as a mutable vector argument.
// `parse_vector` else branch emits `Value::Insert([val])` with no temp var and no
// `vector_db` init ops; `generate_call` then fires a debug assert expecting a 12-byte
// DbRef but finding 0 bytes on the stack.

/// Bug: passing `[]` directly as a mutable `vector<text>` argument panics in debug builds.
/// Tracked as Issue 44 in doc/claude/PROBLEMS.md.
#[test]
fn empty_vector_as_mutable_arg() {
    code!(
        "fn test() {
    result = join([], \"-\");
    assert(result == \"\", \"join([]): {result}\");
}"
    );
}

// ── Issue 56 ─────────────────────────────────────────────────────────────────
// `v += extra` via ref-param panics in debug / silently fails in release.

/// Bug: `v += extra` via ref-param leaves the caller's vector unchanged.
/// Tracked as Issue 56 in doc/claude/PROBLEMS.md.
#[test]
fn ref_param_append_bug() {
    code!(
        "struct Item { name: text, value: integer }
fn fill(v: &vector<Item>, extra: vector<Item>) { v += extra; }
fn test() {
    buf = [Item { name: \"a\", value: 1 }];
    fill(buf, [Item { name: \"b\", value: 2 }]);
    assert(len(buf) == 2, \"len after fill: {len(buf)}\");
    assert(buf[1].value == 2, \"buf[1].value: {buf[1].value}\");
}"
    );
}

// ── Issue 11 ─────────────────────────────────────────────────────────────────
// Field-name overlap between two structs in the same file must NOT cause wrong
// field offsets in key lookups or tree traversal.
//
// Investigation: `determine_keys()` is type-scoped, so IdxElm.key is correctly
// resolved at offset 4 (after nr:integer), not at offset 0 (SortElm.key's position).
// Key lookups and full iteration both pass; Issue 11 was already fixed or never existed.
//
// Range-query note: `[10..20, "B"]` iterates everything up to but not including
// the element at (nr=20, key="B") in the descending ordering.  Since "C">"B"
// alphabetically and the key is sorted descending, (20,C) appears BEFORE (20,B) in
// the tree and IS therefore included → sum = 200+100+300 = 600.

/// Two structs share a field name `key` at different offsets:
/// `SortElm { key: text, value: integer }` (key is field 0, offset 0)
/// `IdxElm  { nr: integer, key: text, value: integer }` (key is field 1, offset 4)
/// Key lookups and iteration on `IdxElm` must use key's offset in IdxElm (4),
/// not in SortElm (0).  Confirmed working — field offsets are type-scoped.
#[test]
fn field_name_overlap_range_query() {
    code!(
        "struct SortElm { key: text, value: integer }
struct IdxElm  { nr: integer, key: text, value: integer }
struct Db {
    srt: sorted<SortElm[-key]>,
    idx: index<IdxElm[nr, -key]>
}
fn test() {
    db = Db {
        srt: [
            SortElm { key: \"One\",   value: 1 },
            SortElm { key: \"Two\",   value: 2 },
            SortElm { key: \"Three\", value: 3 }
        ],
        idx: [
            IdxElm { nr: 10, key: \"A\", value: 100 },
            IdxElm { nr: 10, key: \"B\", value: 200 },
            IdxElm { nr: 20, key: \"C\", value: 300 }
        ]
    };
    // Direct key lookup in sorted (must find correct field offset for SortElm)
    srt_val = db.srt[\"Two\"].value;
    assert(srt_val == 2, \"srt lookup: {srt_val}\");
    // Direct key lookup in index (must find correct field offsets for IdxElm)
    idx_val = db.idx[10, \"B\"].value;
    assert(idx_val == 200, \"idx lookup: {idx_val}\");
    // Range: [10..20, \"B\"] = up to (not including) the element at (nr=20, key=B).
    // In descending key order: (20,C) comes before (20,B), so it IS in range.
    // Correct sum = 200 + 100 + 300 = 600.
    sum = 0;
    for e in db.idx[10..20, \"B\"] { sum += e.value };
    assert(sum == 600, \"range sum: {sum}\");
}"
    );
}

// ── Issue 28 ─────────────────────────────────────────────────────────────────
// validate_slots could panic in debug builds when the same variable name is reused
// in sequential `{ }` blocks in the same function (both get the same slot but
// different live-interval entries).  Fixed: find_conflict() exempts same-name+same-slot pairs.

/// Same variable name in sequential blocks — the core Issue 28 case (fixed).
#[test]
fn sequential_blocks_same_varname_workaround() {
    code!(
        "fn test() {
    total = 0;
    { n = 1; total += n; }
    { n = 2; total += n; }
    { n = 3; total += n; }
    assert(total == 6, \"total: {total}\");
}"
    );
}

/// Different variable names in sequential blocks — validate_slots must not panic.
/// Each block is fully self-contained; variables don't escape their block.
#[test]
fn sequential_blocks_different_varnames() {
    code!(
        "fn test() {
    total = 0;
    { a = 10; total += a; }
    { b = 20; total += b; }
    assert(total == 30, \"total: {total}\");
}"
    );
}

// ── Issue 29 ─────────────────────────────────────────────────────────────────
// validate_slots false positive: two differently-named owned (Reference) variables
// that share a slot but have non-overlapping actual live ranges trigger a conflict
// because compute_intervals gives the first variable a last_use that reaches past
// the second variable's first_def.

/// Two differently-named struct variables in sequential blocks — each in its own
/// `{ }` scope so their lifetimes don't overlap.  validate_slots must not panic.
#[test]
fn sequential_blocks_different_ref_varnames() {
    code!(
        "struct Rec { x: integer }
fn test() {
    total = 0;
    { a = Rec { x: 10 }; total += a.x; }
    { b = Rec { x: 20 }; total += b.x; }
    assert(total == 30, \"total: {total}\");
}"
    );
}

/// The real issue 29 pattern: same variable name `f` reused across many sequential
/// blocks; a differently-named reference variable `c` is introduced between some of
/// those blocks.  validate_slots must not panic (c.first_def may fall between two
/// of f's live ranges, which are separate Variable entries sharing the same slot).
#[test]
fn issue_29_reused_refname_with_later_different_var() {
    code!(
        "struct Rec { x: integer }
fn test() {
    total = 0;
    { f = Rec { x: 1 }; total += f.x; }
    { f = Rec { x: 2 }; total += f.x; }
    c = Rec { x: 99 };
    { f = Rec { x: 3 }; total += f.x; }
    total += c.x;
    assert(total == 6 + 99, \"total: {total}\");
}"
    );
}

// ── T1-1: Non-zero exit code on runtime error (production mode) ───────────────
// In normal mode a failing assert/panic aborts via Rust panic!().
// In production mode (--production flag) the error is logged and execution
// continues — main.rs must exit(1) via had_fatal.  These tests verify that
// `Stores::had_fatal` is set correctly so the binary-level exit code is right.

/// Helper: compile loft code and return a State ready for execution.
fn compile_for_production(code: &str) -> (State, loft::data::Data) {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(code, "t1_1_test", false);
    assert!(
        p.diagnostics.lines().is_empty(),
        "Parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    (state, p.data)
}

/// Attach a production-mode logger (writes to /dev/null) to a State.
fn attach_production_logger(state: &mut State) {
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    let lg = Logger::new(config, None);
    state.database.logger = Some(Arc::new(Mutex::new(lg)));
}

/// No error: had_fatal stays false.
#[test]
fn production_mode_no_error_had_fatal_false() {
    let (mut state, data) = compile_for_production("fn test() { assert(1 == 1, \"ok\"); }");
    attach_production_logger(&mut state);
    state.execute("test", &data);
    assert!(
        !state.database.had_fatal,
        "had_fatal must stay false when assert passes"
    );
}

/// panic() in production mode: had_fatal becomes true, execution does NOT abort.
#[test]
fn production_mode_panic_sets_had_fatal() {
    let (mut state, data) = compile_for_production("fn test() { panic(\"deliberate\"); }");
    attach_production_logger(&mut state);
    state.execute("test", &data);
    assert!(
        state.database.had_fatal,
        "had_fatal must be true after panic() in production mode"
    );
}

/// assert(false, ...) in production mode: had_fatal becomes true.
#[test]
fn production_mode_assert_false_sets_had_fatal() {
    let (mut state, data) = compile_for_production("fn test() { assert(1 == 2, \"mismatch\"); }");
    attach_production_logger(&mut state);
    state.execute("test", &data);
    assert!(
        state.database.had_fatal,
        "had_fatal must be true after assert(false) in production mode"
    );
}

// ── T1-8: For-loop mutation guard extended to field access ────────────────────
// Appending to a collection that is actively being iterated can cause infinite
// loops (vector) or structural corruption (sorted/index).  The guard that
// catches `items += [x]` must also fire for `db.items += [x]`.

/// Direct variable form: existing guard must still work.
#[test]
fn for_loop_mutation_guard_simple_var() {
    code!(
        "fn test() {
    items = [1, 2, 3];
    for e in items { items += [e]; }
}"
    )
    .error(
        "Cannot add elements to 'items' while it is being iterated — \
use a separate collection or add after the loop \
at for_loop_mutation_guard_simple_var:3:32",
    );
}

/// Field-access form: `db.items += [x]` inside `for e in db.items { ... }`.
#[test]
fn for_loop_mutation_guard_field_access() {
    code!(
        "struct Container { items: vector<integer> }
fn test() {
    db = Container { items: [1, 2, 3] };
    for e in db.items { db.items += [e]; }
}"
    )
    .error(
        "Cannot add elements to a collection while it is being iterated — \
use a separate collection or add after the loop \
at for_loop_mutation_guard_field_access:4:38",
    );
}

/// Safe: appending to a DIFFERENT field than the one being iterated is allowed.
#[test]
fn for_loop_mutation_guard_different_field_ok() {
    code!(
        "struct Container { src: vector<integer>, dst: vector<integer> }
fn test() {
    db = Container { src: [1, 2, 3], dst: [] };
    for e in db.src { db.dst += [e * 2]; };
    assert(len(db.dst) == 3, \"len: {len(db.dst)}\");
    assert(db.dst[0] == 2, \"dst[0]: {db.dst[0]}\");
}"
    );
}

// ── T2-4  f#exists attribute ──────────────────────────────────────────────────

/// f#exists returns true for a known existing file.
#[test]
fn file_exists_true() {
    code!(
        "fn test() {
    f = file(\"tests/scripts/11-files.loft\");
    assert(f#exists, \"expected exists to be true\");
}"
    )
    .result(loft::data::Value::Null);
}

/// f#exists returns false for a path that does not exist.
#[test]
fn file_exists_false() {
    code!(
        "fn test() {
    f = file(\"tests/scripts/no-such-file.loft\");
    assert(!f#exists, \"expected exists to be false\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── T1-1 (Tier 2): Callable function references ───────────────────────────────
// `fn <name>` produces a Value::Int(d_nr) with Type::Function(args, ret).
// Calling `f(args)` where `f` is a local fn-ref variable emits OpCallRef.

/// Basic fn-ref: store `fn double` and call it through the reference.
#[test]
fn fn_ref_basic_call() {
    code!(
        "fn double(n: integer) -> integer { n * 2 }
fn test() {
    f = fn double;
    result = f(21);
    assert(result == 42, \"expected 42, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

/// Fn-ref with multiple arguments.
#[test]
fn fn_ref_two_args() {
    code!(
        "fn add(a: integer, b: integer) -> integer { a + b }
fn test() {
    f = fn add;
    result = f(10, 32);
    assert(result == 42, \"expected 42, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

/// Fn-ref assigned conditionally, then called.
#[test]
fn fn_ref_conditional_call() {
    code!(
        "fn inc(n: integer) -> integer { n + 1 }
fn dec(n: integer) -> integer { n - 1 }
fn test() {
    flag = true;
    f = if flag { fn inc } else { fn dec };
    result = f(41);
    assert(result == 42, \"expected 42, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

/// Fn-ref passed as a parameter and called inside the callee.
#[test]
fn fn_ref_as_parameter() {
    code!(
        "fn square(n: integer) -> integer { n * n }
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
fn test() {
    result = apply(fn square, 7);
    assert(result == 49, \"expected 49, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── map / filter / reduce ─────────────────────────────────────────────────────

#[test]
fn map_integers() {
    code!(
        "fn double(x: integer) -> integer { x * 2 }
fn test() {
    v = [1, 2, 3, 4, 5];
    r = map(v, fn double);
    s = 0;
    for x in r {
        s += x;
    }
    assert(s == 30, \"expected 30, got {s}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn filter_integers() {
    code!(
        "fn is_even(x: integer) -> boolean { x % 2 == 0 }
fn test() {
    v = [1, 2, 3, 4, 5, 6];
    r = filter(v, fn is_even);
    s = 0;
    for x in r {
        s += x;
    }
    assert(s == 12, \"expected 12, got {s}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn reduce_sum() {
    code!(
        "fn add(acc: integer, x: integer) -> integer { acc + x }
fn test() {
    v = [1, 2, 3, 4, 5];
    s = reduce(v, 0, fn add);
    assert(s == 15, \"expected 15, got {s}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── Issue 27 ─────────────────────────────────────────────────────────────────
// `self.field = null` in a method generated no bytecode for the null argument.
// `generate(Value::Null)` returned Type::Void with no emitted bytes, so OpSetInt
// read its `val` argument from the wrong stack location (return-address bytes),
// producing store_nr=60 → out-of-bounds panic in allocation.rs.
// Fix: `parse_assign_op` now calls `convert()` when s_type==Type::Null and op=="=",
// substituting OpConvIntFromNull (or the appropriate FromNull op) before towards_set.

/// Exact T0-1 reproduction: method sets an integer field to null via reference param.
/// Previously panicked with "store_nr=60" in `set_int`.
#[test]
fn set_int_field_null_via_ref() {
    code!(
        "struct S { cur: integer }
fn clear(self: S) { self.cur = null }
fn test() {
    s = S { cur: 42 };
    s.clear();
    assert(s.cur == null, \"expected null, got {s.cur}\");
}"
    )
    .result(loft::data::Value::Null);
}

/// Integer field set to null via direct struct access (not a method call).
#[test]
fn set_int_field_null_direct() {
    code!(
        "struct S { cur: integer }
fn test() {
    s = S { cur: 7 };
    s.cur = null;
    assert(s.cur == null, \"expected null after direct assignment\");
}"
    )
    .result(loft::data::Value::Null);
}

/// Long field set to null via reference parameter.
#[test]
fn set_long_field_null_via_ref() {
    code!(
        "struct S { val: long }
fn clear(self: S) { self.val = null }
fn test() {
    s = S { val: 1000000l };
    s.clear();
    assert(s.val == null, \"expected null, got {s.val}\");
}"
    )
    .result(loft::data::Value::Null);
}

/// Multiple scalar fields (integer, long) set to null in one method call.
#[test]
fn set_multiple_scalar_fields_null() {
    code!(
        "struct S { a: integer, b: long }
fn clear(self: S) {
    self.a = null;
    self.b = null;
}
fn test() {
    s = S { a: 1, b: 2l };
    s.clear();
    assert(s.a == null, \"a should be null\");
    assert(s.b == null, \"b should be null\");
}"
    )
    .result(loft::data::Value::Null);
}

/// Set field to null then restore a value — round-trip correctness.
#[test]
fn null_then_reassign_integer_field() {
    code!(
        "struct S { cur: integer }
fn clear(self: S) { self.cur = null }
fn test() {
    s = S { cur: 10 };
    s.clear();
    assert(s.cur == null, \"should be null after clear\");
    s.cur = 42;
    assert(s.cur == 42, \"should be 42 after reassign\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── PROBLEMS #37 (T0-2): LIFO store-free panic ───────────────────────────────
// scopes.rs::variables() was iterating var_scope (a BTreeMap) in ascending key
// order, causing get_free_vars() to emit OpFreeRef in forward declaration order.
// database::free() enforces LIFO: the most-recently-allocated store must be
// freed first.  Fix: track insertion order in var_order: Vec<u16> and iterate it
// in reverse so the last-inserted (last-allocated) variable is freed first.

/// Two owned struct refs in the same scope — minimal T0-2 reproducer.
#[test]
fn lifo_store_free_two_owned_refs() {
    code!(
        "struct S { val: integer }
fn double(self: S) -> S { S { val: self.val * 2 } }
fn test() {
    c = S { val: 3 };
    d = c.double();
    assert(d.val == 6, \"d.val after double: {d.val}\");
}"
    )
    .result(loft::data::Value::Null);
}

/// Three owned struct refs in the same scope — verifies LIFO holds for N > 2.
#[test]
fn lifo_store_free_three_owned_refs() {
    code!(
        "struct Point { x: integer, y: integer }
fn test() {
    a = Point { x: 1, y: 2 };
    b = Point { x: 3, y: 4 };
    c = Point { x: 5, y: 6 };
    assert(a.x + b.x + c.x == 9, \"sum x: {a.x + b.x + c.x}\");
    assert(a.y + b.y + c.y == 12, \"sum y: {a.y + b.y + c.y}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── PROBLEMS #38 (T0-3): T0-1 regression — key-null removal silently broken ──
// The T0-1 fix in parse_assign_op called self.convert(code, Null, f_type)
// unconditionally for all null assignments.  For reference-typed LHS (e.g. the
// element ref returned by sorted_coll[key]) convert() replaced Value::Null with
// Value::Call(OpConvRefFromNull, []).  towards_set_hash_remove checks
// *val == Value::Null to detect removal; after the substitution that check fails
// and the element is never removed.
// Fix: guard the convert() call so it only runs for scalar (non-reference,
// non-collection) types.

/// sorted[key] = null removes the entry.
#[test]
fn sorted_key_null_removes_entry() {
    code!(
        "struct Elm { key: integer, val: integer }
struct Db { s: sorted<Elm[key]> }"
    )
    .expr(
        "db = Db { s: [Elm{key:1,val:10}, Elm{key:2,val:20}] };
db.s[1] = null;
assert(!db.s[1], \"key 1 removed\");
assert(db.s[2].val == 20, \"key 2 still present: {db.s[2].val}\");",
    );
}

/// hash[key] = null removes the entry.
#[test]
fn hash_key_null_removes_entry() {
    code!(
        "struct Keyword { name: text }
struct Data { h: hash<Keyword[name]> }"
    )
    .expr(
        "c = Data {};
c.h = [{ name: \"one\" }, { name: \"two\" }];
c.h[\"one\"] = null;
assert(!c.h[\"one\"], \"one removed\");
assert(!!c.h[\"two\"], \"two still present\");",
    );
}

// ── PROBLEMS #39 (T0-4): `v += other_vec` shallow copy — text fields dangle ───
// vector_add() used a raw copy_block without calling copy_claims().  Both the
// source and destination vectors ended up with the same string-record indices;
// when the source was freed, remove_claims() deleted those records and the
// destination's text fields became dangling.  The fix: after copy_block, iterate
// each appended element and call copy_claims() to create independent copies of
// string records (and sub-structures) in the destination store.

/// Appending a vector<struct-with-text> to another vector must deep-copy string
/// records.  Without the fix both bags share the same records; at end-of-scope
/// LIFO frees the source first, then the destination tries to double-free the
/// same records → panic.
#[test]
fn vec_add_text_field_deep_copy() {
    code!(
        "struct Item { name: text, value: integer }
struct Bag { items: vector<Item> }
fn test() {
    a = Bag { items: [Item{name: \"hello\", value: 1}, Item{name: \"world\", value: 2}] };
    b = Bag {};
    b.items += a.items;
    assert(b.items[0].name == \"hello\", \"name[0]: {b.items[0].name}\");
    assert(b.items[1].name == \"world\", \"name[1]: {b.items[1].name}\");
    assert(b.items[0].value == 1, \"value[0]: {b.items[0].value}\");
    assert(b.items[1].value == 2, \"value[1]: {b.items[1].value}\");
}"
    )
    .result(loft::data::Value::Null);
}

/// Appending to a non-empty destination: pre-existing and new elements all have
/// independent text records.
#[test]
fn vec_add_text_field_non_empty_dest() {
    code!(
        "struct Tag { label: text }
struct Col { tags: vector<Tag> }
fn test() {
    src = Col { tags: [Tag{label: \"a\"}, Tag{label: \"b\"}] };
    dst = Col { tags: [Tag{label: \"x\"}] };
    dst.tags += src.tags;
    assert(dst.tags[0].label == \"x\", \"tag[0]: {dst.tags[0].label}\");
    assert(dst.tags[1].label == \"a\", \"tag[1]: {dst.tags[1].label}\");
    assert(dst.tags[2].label == \"b\", \"tag[2]: {dst.tags[2].label}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── PROBLEMS #40 (T0-5): copy_claims / remove_claims for Parts::Index ─────────
// Both copy_claims and remove_claims contained `Parts::Index => panic!("Not
// implemented")`.  Adding a struct-with-index to a vector triggers OpCopyRecord
// → copy_claims_index_body; freeing the struct after reassignment triggers the
// Parts::Index arm of remove_claims.

/// copy_claims path: a struct with an index<T[key]> field is added to a vector
/// (triggering OpCopyRecord → copy_claims → copy_claims_index_body).
/// Before the fix this panicked with "Not implemented".
#[test]
fn index_field_copy_claims_via_vector() {
    code!(
        "struct Item { key: integer, val: integer }
struct Container { items: index<Item[key]> }
fn test() {
    c = Container {
        items: [Item { key: 1, val: 10 }, Item { key: 2, val: 20 }]
    };
    bag = [c];
    assert(bag[0].items[1].val == 10, \"key 1 val: {bag[0].items[1].val}\");
    assert(bag[0].items[2].val == 20, \"key 2 val: {bag[0].items[2].val}\");
}"
    );
}

/// copy_claims on index with text fields: text records must be deep-copied so
/// source and destination are independent.
#[test]
fn index_field_copy_claims_text_elements() {
    code!(
        "struct Tag { key: integer, label: text }
struct Bag { tags: index<Tag[key]> }
fn test() {
    b = Bag {
        tags: [Tag { key: 1, label: \"alpha\" }, Tag { key: 2, label: \"beta\" }]
    };
    copy = [b];
    assert(copy[0].tags[1].label == \"alpha\", \"label 1: {copy[0].tags[1].label}\");
    assert(copy[0].tags[2].label == \"beta\",  \"label 2: {copy[0].tags[2].label}\");
}"
    );
}

/// remove_claims path for Parts::Index: reassigning a struct that holds an
/// index<T> field triggers database() → clear() → remove_claims on the index.
/// Before the fix this panicked with "Not implemented".
#[test]
fn index_field_remove_claims_on_reassign() {
    code!(
        "struct Node { key: integer, score: integer }
struct Graph { nodes: index<Node[key]> }
fn test() {
    g = Graph {
        nodes: [Node { key: 1, score: 100 }, Node { key: 2, score: 200 }]
    };
    assert(g.nodes[1].score == 100, \"score before: {g.nodes[1].score}\");
    g = Graph {
        nodes: [Node { key: 3, score: 300 }]
    };
    assert(g.nodes[3].score == 300, \"score after reassign: {g.nodes[3].score}\");
    assert(!g.nodes[1], \"key 1 gone after reassign\");
}"
    );
}

// ── PROBLEMS #41 (T0-6): inline ref-returning call leaks store → LIFO panic ───
// `p.method().field` where method() returns an owned ref must wrap the temporary
// in a work-ref variable so scopes.rs emits OpFreeRef at end-of-scope.

/// Single field access on an inline ref-returning call must not leak the store.
#[test]
fn inline_ref_call_field_access() {
    code!(
        "struct Point { x: float, y: float }
fn shifted(self: Point, dx: float, dy: float) -> Point {
    Point { x: self.x + dx, y: self.y + dy }
}
fn test() {
    p = Point { x: 1.0, y: 2.0 };
    assert(p.shifted(1.0, 0.0).x == 2.0, \"shifted x: {p.shifted(1.0, 0.0).x}\");
    assert(p.shifted(0.0, 2.0).y == 4.0, \"shifted y: {p.shifted(0.0, 2.0).y}\");
}"
    );
}

/// Two chained inline calls (shifted().shifted().x) must not leak either store.
#[test]
fn inline_ref_call_double_chain() {
    code!(
        "struct Point { x: float, y: float }
fn shifted(self: Point, dx: float, dy: float) -> Point {
    Point { x: self.x + dx, y: self.y + dy }
}
fn test() {
    p = Point { x: 1.0, y: 2.0 };
    assert(p.shifted(1.0, 0.0).shifted(0.0, 3.0).x == 2.0, \"double chain x\");
    assert(p.shifted(1.0, 0.0).shifted(0.0, 3.0).y == 5.0, \"double chain y\");
}"
    );
}

/// index[key] = null removes the entry.
#[test]
fn index_key_null_removes_entry() {
    code!(
        "struct Elm { nr: integer, key: text, val: integer }
struct Db { map: index<Elm[nr,-key]> }"
    )
    .expr(
        "db = Db { map: [Elm{nr:1,key:\"a\",val:10}, Elm{nr:2,key:\"b\",val:20}] };
db.map[1] = null;
assert(!db.map[1], \"nr 1 removed\");
assert(db.map[2].val == 20, \"nr 2 still present: {db.map[2].val}\");",
    );
}

/// T2-7: mkdir creates a directory, mkdir_all creates nested directories.
#[test]
fn mkdir_and_mkdir_all() {
    // Clean up from any previous failed run
    let _ = std::fs::remove_dir_all("tests/tmp_mkdir_test");
    code!(
        "fn test() {
    // mkdir_all creates nested path
    assert(mkdir_all(\"tests/tmp_mkdir_test/sub\"), \"mkdir_all\");
    // mkdir on existing directory returns false
    assert(!mkdir(\"tests/tmp_mkdir_test/sub\"), \"mkdir existing\");
    // mkdir on a new sibling
    assert(mkdir(\"tests/tmp_mkdir_test/other\"), \"mkdir sibling\");
}"
    );
    // Clean up after test
    let _ = std::fs::remove_dir_all("tests/tmp_mkdir_test");
}

// ── T0-11: Write to locked store must panic ───────────────────────────────────
// addr_mut() previously returned a thread-local DUMMY buffer in release builds
// (#[cfg(not(debug_assertions))]), silently discarding the write.  The fix
// removes the DUMMY and replaces it with an unconditional assert!(!self.locked)
// so any write to a locked store panics in both debug and release builds.
// The unit test lives in src/store.rs (tests::write_to_locked_store_panics)
// because Store is a private module.

// ── T0-12: vector self-append (`v += v`) must not corrupt data ────────────────
// vector_add() read o_rec before resizing the destination, but vector_append /
// vector_set_size may reallocate the backing store, making o_rec stale.
// The fix snapshots the source bytes into a Vec<u8> before any resize.

/// `v += v` on an integer vector: result must be a doubled vector with correct values.
#[test]
fn vector_self_append_integers() {
    code!(
        "fn test() {
    v = [1, 2, 3];
    v += v;
    assert(len(v) == 6, \"len: {len(v)}\");
    assert(v[0] == 1, \"v[0]: {v[0]}\");
    assert(v[1] == 2, \"v[1]: {v[1]}\");
    assert(v[2] == 3, \"v[2]: {v[2]}\");
    assert(v[3] == 1, \"v[3]: {v[3]}\");
    assert(v[4] == 2, \"v[4]: {v[4]}\");
    assert(v[5] == 3, \"v[5]: {v[5]}\");
}"
    );
}

/// `v += v` on a single-element vector: result must have two equal elements.
#[test]
fn vector_self_append_single() {
    code!(
        "fn test() {
    v = [42];
    v += v;
    assert(len(v) == 2, \"len: {len(v)}\");
    assert(v[0] == 42, \"v[0]: {v[0]}\");
    assert(v[1] == 42, \"v[1]: {v[1]}\");
}"
    );
}

// ── T1-32: File I/O errors are no longer silently discarded ──────────────────
// write_file/read_file/seek_file used unwrap_or_default() / unwrap_or(0),
// swallowing OS errors with no diagnostic output.  The fix logs to stderr via
// eprintln!.  The test below verifies that writing to a bad path does not panic
// or hang — the error is printed to stderr and execution continues normally.

/// Writing to an unwritable path must not panic; the program continues after the error.
#[test]
fn file_write_error_does_not_panic() {
    // Use a path inside a non-existent directory so File::create will fail.
    code!(
        "fn test() {
    f = file(\"/no_such_dir/output.txt\");
    f += \"hello\";
    // Execution must reach this assert without panicking.
    assert(true, \"should not panic\");
}"
    );
}

// ── N8 ───────────────────────────────────────────────────────────────────────
// Fix empty pre-eval bindings and `_pre{n}` → `_pre_{n}` naming in generation.rs.
// Root cause: (1) `generate_expr_buf` returns "" for some void/null expressions,
// causing `let _pre5 = ;` (invalid Rust) and corrupt substitution; (2) Rust
// edition 2021+ parses `_pre14` as a prefix token (like `b"…"`), producing
// parse errors in generated code.

/// N8-naming: generated code must use `_pre_N` names, not bare `_preN`.
/// A nested user-defined function call is enough to trigger pre-eval hoisting.
#[test]
fn n8_pre_eval_uses_underscore_separator() {
    // Two nested user-fn calls: the inner call is pre-eval-hoisted by generation.rs.
    code!("fn inc(v: integer) -> integer { v + 1 }")
        .expr("inc(inc(0))")
        .result(Value::Int(2));
    let src =
        std::fs::read_to_string("tests/generated/issues_n8_pre_eval_uses_underscore_separator.rs")
            .expect("generated file not found");
    // Every `let _pre…` line must use `_pre_N` (digit after underscore), not `_preN`.
    for line in src.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("let _pre") {
            assert!(
                rest.starts_with('_'),
                "Found bare `_preN` binding (should be `_pre_N`): {line}"
            );
        }
    }
}

/// N8-empty: generated code must not emit `let _preN = ;` (empty right-hand side).
/// The mutable-reference pattern (user fn with `&T = null` default) triggers this.
#[test]
fn n8_no_empty_pre_eval_binding() {
    code!(
        "struct Data { num: integer, values: vector<integer> }
fn add(r: &Data = null, val: integer) {
    if !r { r = Data { num: 0 }; }
    r.num += val;
    r.values += [val];
}"
    )
    .expr("v = Data { num: 1 }; add(v, 2); add(v, 3); \"{v}\"")
    .result(Value::str("{num:6,values:[2,3]}"));
    let src = std::fs::read_to_string("tests/generated/issues_n8_no_empty_pre_eval_binding.rs")
        .expect("generated file not found");
    for line in src.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("let _pre") && trimmed.trim_end().ends_with("= ;") {
            panic!("Found empty pre-eval binding: {line}");
        }
    }
}

/// N3: assigning a reference to another reference must emit OpCopyRecord for deep copy.
/// Without it, both variables alias the same heap record; mutating through one changes the other.
#[test]
fn n3_reference_assignment_emits_copy_record() {
    // Bytecode interpreter correctly deep-copies references already; test confirms behaviour.
    code!("struct T { name: text }")
        .expr(
            "a = T { name: \"hello\" };
b = a;
b.name += \" world\";
a.name",
        )
        .result(Value::str("hello"));
    let src = std::fs::read_to_string(
        "tests/generated/issues_n3_reference_assignment_emits_copy_record.rs",
    )
    .expect("generated file not found");
    assert!(
        src.contains("OpCopyRecord(stores,"),
        "generated code missing OpCopyRecord after reference assignment"
    );
}

/// N5: vector::clear_vector must not be called when the DbRef is null (rec == 0).
/// A function that initialises and returns a vector was panicking with
/// "Unknown record 2147483648" because clear_vector ran on stores.null() before allocation.
#[test]
fn n5_null_dbref_clear_vector_guard() {
    code!(
        "pub fn fill() -> vector<text> {
    result = [];
    result += [\"aa\", \"bb\"];
    result
}"
    )
    .expr("t = fill(); \"{t}\"")
    .result(Value::str("[\"aa\",\"bb\"]"));
    let src = std::fs::read_to_string("tests/generated/issues_n5_null_dbref_clear_vector_guard.rs")
        .expect("generated file not found");
    assert!(
        src.contains(".rec != 0"),
        "generated code missing null check before clear_vector"
    );
}

/// N4: struct-enum variants must show all fields when formatted with OpFormatDatabase.
/// The init function was registering every enum value with u16::MAX as the struct type,
/// so ShowDb could not dispatch to variant fields and only showed the variant name.
#[test]
fn n4_format_struct_enum_variant_shows_fields() {
    code!(
        "enum Op {
    Nop,
    Add { left: integer, right: integer }
}"
    )
    .expr("v = \"Add {{ left: 1, right: 2 }}\" as Op; \"{v}\"")
    .result(Value::str("Add {left:1,right:2}"));
    let src = std::fs::read_to_string(
        "tests/generated/issues_n4_format_struct_enum_variant_shows_fields.rs",
    )
    .expect("generated file not found");
    // The generated init must register the Add variant with its actual struct type (not u16::MAX).
    assert!(
        !src.contains("db.value(e, \"Add\", u16::MAX)"),
        "generated init still registers struct-enum variant Add with u16::MAX"
    );
}

/// N9a: the auto-generated tests/generated/fill.rs must contain `use crate::ops;`
/// so it can be compiled as a crate-internal file and eventually replace src/fill.rs.
#[test]
fn n9a_generated_fill_has_ops_import() {
    // generate_code() runs on every test via the testing harness — fill.rs exists.
    let src = std::fs::read_to_string("tests/generated/fill.rs")
        .expect("tests/generated/fill.rs not found — run any loft test first");
    assert!(
        src.contains("use crate::ops;"),
        "generated fill.rs missing `use crate::ops;`"
    );
}

/// N9 (N20b/N20c/N20d): auto-generated tests/generated/fill.rs must be byte-for-byte
/// identical to src/fill.rs once all #rust templates are present and rustfmt is applied.
/// Ignored until N20b (rustfmt call) and N20d (#rust templates) are both implemented.
#[test]
#[ignore = "N9: N20b (rustfmt) and N20d (#rust templates) not yet implemented"]
fn n9_generated_fill_matches_src() {
    let generated = std::fs::read_to_string("tests/generated/fill.rs")
        .expect("tests/generated/fill.rs not found — run any loft test first");
    let src =
        std::fs::read_to_string("src/fill.rs").expect("src/fill.rs not found");
    assert_eq!(
        generated,
        src,
        "tests/generated/fill.rs differs from src/fill.rs — \
         run create::generate_code() and copy the result"
    );
}

/// N8: Sort must work correctly in native-codegen mode.
/// The #rust template for OpSortVector is inlined directly (no OpSortVector runtime fn needed).
#[test]
fn n8_codegen_runtime_vector_ops_exist() {
    // Sorting a vector of integers must work in native-codegen mode.
    code!("fn sort_it() -> vector<integer> { v = [3, 1, 2]; sort(v); v }")
        .expr("\"{sort_it()}\"")
        .result(Value::str("[1,2,3]"));
    let src =
        std::fs::read_to_string("tests/generated/issues_n8_codegen_runtime_vector_ops_exist.rs")
            .expect("generated file not found");
    assert!(
        src.contains("vector::sort_vector("),
        "generated code missing inlined vector::sort_vector call"
    );
}

/// N10: ops::text_character returns char but loft represents character as i32.
/// Generated code assigns the char to an i32 variable without a cast, causing a compile error.
/// Also, i32 character variables used in method dispatch (is_alphanumeric etc.) need wrapping
/// with ops::to_char(...) since the method requires char, not i32.
#[test]
fn n10_char_cast_in_generated_code() {
    code!(
        "fn count_alpha(s: text) -> integer {
    n = 0;
    for c in s { if c.is_alphanumeric() { n += 1; } }
    n
}"
    )
    .expr("count_alpha(\"a1!\")")
    .result(Value::Int(2));
    let src = std::fs::read_to_string("tests/generated/issues_n10_char_cast_in_generated_code.rs")
        .expect("generated file not found");
    assert!(
        src.contains("as u32 as i32") || src.contains("ops::to_char("),
        "generated code missing char<->i32 cast"
    );
}

/// N2: output_init must register content types before the structs that reference them in
/// sorted/index/hash fields.  When a struct has a sorted<Foo> field and Foo has a higher
/// type-ID than the struct, the init function panicked because db.sorted(foo_type_id, ...)
/// was called before Foo was registered.
#[test]
fn n2_sorted_field_content_type_registered_first() {
    code!(
        "struct Sort { nr: integer }
struct Container { data: sorted<Sort[nr]> }"
    )
    .expr("c = Container {}; \"{c}\"")
    .result(Value::str("{data:[]}"));
    let src = std::fs::read_to_string(
        "tests/generated/issues_n2_sorted_field_content_type_registered_first.rs",
    )
    .expect("generated file not found");
    // Sort must appear in the init before Container (which contains the sorted<Sort> field).
    let sort_pos = src.find("\"Sort\"").expect("Sort not found in init");
    let container_pos = src
        .find("\"Container\"")
        .expect("Container not found in init");
    assert!(
        sort_pos < container_pos,
        "Sort (content type) must be registered before Container in generated init"
    );
}

/// N7: OpFormatFloat must generate ops::format_float(...), not OpFormatFloat(stores, ...).
/// OpFormatStackLong must generate ops::format_long(var_, ...) without stores or &mut.
#[test]
fn n7_format_ops_generate_correct_rust() {
    // Float formatting
    code!("struct Flt { v: float }")
        .expr("f = Flt { v: 3.14 }; \"{f.v}\"")
        .result(Value::str("3.14"));
    let src =
        std::fs::read_to_string("tests/generated/issues_n7_format_ops_generate_correct_rust.rs")
            .expect("generated file not found");
    assert!(
        !src.contains("OpFormatFloat("),
        "generated code still contains bare OpFormatFloat call"
    );
    assert!(
        src.contains("ops::format_float("),
        "generated code missing ops::format_float call"
    );
}
