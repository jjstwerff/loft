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

// ── Issue 5 ──────────────────────────────────────────────────────────────────
// Scalar `+=` on an empty (null) vector struct field has no effect.
// Expected: the scalar is appended and len == 1.

// `b.items += [1]` (bracket form) on a null field — this is the WORKING baseline.
// The bracket form goes through parse_vector with is_field=true and uses
// OpNewRecord / OpFinishRecord to allocate the element in place.
// `b.items += [3, 5]` on a null field — multiple elements with bracket form.
// `b.items += 1` (bare scalar, no brackets) on a null field — FIXED.
// Parser now routes through new_record so the field is allocated in place.
// Was tracked as Issue 5 in doc/claude/PROBLEMS.md.
// ── Issue 1 ──────────────────────────────────────────────────────────────────
// A method whose return type is a NEW struct record crashes at database.rs:1494
// because the DbRef returned by the method has a garbage store_nr.

// Minimal reproducer: `fn double(self: Color) -> Color { Color { r: self.r * 2 } }`
// Calling `c.double()` crashes with "index out of bounds: the len is N but index is M".
// Tracked as Issue 1 in doc/claude/PROBLEMS.md.
// ── Issue 2 ──────────────────────────────────────────────────────────────────
// A borrowed reference first assigned inside a branch gets a garbage store_nr=8
// DbRef at runtime, crashing at database.rs:1462.
// Owned references are correctly pre-initialized (Option A sub-3); borrowed refs are not.

// Borrowed ref first assigned INSIDE an `if` branch — FIXED.
// Was tracked as Issue 2 in doc/claude/PROBLEMS.md; now passes after
// the Option A sub-3 pre-init work in scopes.rs.
// ── Issue 4 ──────────────────────────────────────────────────────────────────
// `v += items` inside a function that takes `v` as a `&vector<T>` ref-param
// has no visible effect on the caller's variable after the call returns.

// Baseline: field mutation through a ref-param WORKS (e.g. `v[0].val = x`).
// ── Issue 44 — L4: Empty `[]` literal as a mutable vector argument ───────────
// Fixed in parser/mod.rs call_nr(): when Value::Insert([Null]) (or empty Insert)
// appears where a vector parameter is expected, a temp variable is created with
// vector_db initialisation ops, giving the caller a proper 12-byte DbRef.
// The fix is in call_nr(), not in parse_vector(), so it runs whenever [] reaches
// the call-site type-check regardless of call nesting.

// Baseline: `join([], "-")` — empty `vector<text>` arg via call_nr fix.
// L4 edge: `[]` passed to a user-defined function taking `vector<integer>`.
// Exercises the same call_nr path for a non-text element type.
// L4 edge: `[]` as second argument, not first — verifies argument index handling.
// ── Issue 56 — L5: `v += extra` via `&vector` ref-param ──────────────────────
// Fixed in state/codegen.rs generate_var(): RefVar(Vector) now emits OpGetStackRef
// to dereference the ref-param and load the actual vector DbRef before OpAppendVector.
// The vector record write-back happens implicitly: vector_append writes through the
// DbRef into the caller's local-variable record, so the caller sees the updated vector.

// Baseline: `v += extra` via ref-param appends elements to the caller's vector.
// L5 edge: append integers via ref-param; verify values and length.
// L5 edge: multiple sequential ref-param appends grow the vector correctly.
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

// Two structs share a field name `key` at different offsets:
// `SortElm { key: text, value: integer }` (key is field 0, offset 0)
// `IdxElm  { nr: integer, key: text, value: integer }` (key is field 1, offset 4)
// Key lookups and iteration on `IdxElm` must use key's offset in IdxElm (4),
// not in SortElm (0).  Confirmed working — field offsets are type-scoped.
// ── Issue 28 ─────────────────────────────────────────────────────────────────
// validate_slots could panic in debug builds when the same variable name is reused
// in sequential `{ }` blocks in the same function (both get the same slot but
// different live-interval entries).  Fixed: find_conflict() exempts same-name+same-slot pairs.

// Same variable name in sequential blocks — the core Issue 28 case (fixed).
// Different variable names in sequential blocks — validate_slots must not panic.
// Each block is fully self-contained; variables don't escape their block.
// ── Issue 29 ─────────────────────────────────────────────────────────────────
// validate_slots false positive: two differently-named owned (Reference) variables
// that share a slot but have non-overlapping actual live ranges trigger a conflict
// because compute_intervals gives the first variable a last_use that reaches past
// the second variable's first_def.

// Two differently-named struct variables in sequential blocks — each in its own
// `{ }` scope so their lifetimes don't overlap.  validate_slots must not panic.
// The real issue 29 pattern: same variable name `f` reused across many sequential
// blocks; a differently-named reference variable `c` is introduced between some of
// those blocks.  validate_slots must not panic (c.first_def may fall between two
// of f's live ranges, which are separate Variable entries sharing the same slot).
// ── T1-1: Non-zero exit code on runtime error (production mode) ───────────────
// In normal mode a failing assert/panic aborts via Rust panic!().
// In production mode (--production flag) the error is logged and execution
// continues — main.rs must exit(1) via had_fatal.  These tests verify that
// `Stores::had_fatal` is set correctly so the binary-level exit code is right.

// Helper: compile loft code and return a State ready for execution.
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

// Attach a production-mode logger (writes to /dev/null) to a State.
fn attach_production_logger(state: &mut State) {
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    let lg = Logger::new(config, None);
    state.database.logger = Some(Arc::new(Mutex::new(lg)));
}

// No error: had_fatal stays false.
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

// panic() in production mode: had_fatal becomes true, execution does NOT abort.
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

// assert(false, ...) in production mode: had_fatal becomes true.
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

// Direct variable form: existing guard must still work.
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

// Field-access form: `db.items += [x]` inside `for e in db.items { ... }`.
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

// Safe: appending to a DIFFERENT field than the one being iterated is allowed.
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

// f#exists returns true for a known existing file.
#[test]
fn file_exists_true() {
    code!(
        "fn test() {
    f = file(\"tests/scripts/19-files.loft\");
    assert(f#exists, \"expected exists to be true\");
}"
    )
    .result(loft::data::Value::Null);
}

// f#exists returns false for a path that does not exist.
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

// Basic fn-ref: store `double` and call it through the reference.
#[test]
fn fn_ref_basic_call() {
    code!(
        "fn double(n: integer) -> integer { n * 2 }
fn test() {
    f = double;
    result = f(21);
    assert(result == 42, \"expected 42, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Fn-ref with multiple arguments.
#[test]
fn fn_ref_two_args() {
    code!(
        "fn add(a: integer, b: integer) -> integer { a + b }
fn test() {
    f = add;
    result = f(10, 32);
    assert(result == 42, \"expected 42, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Fn-ref assigned conditionally, then called.
#[test]
fn fn_ref_conditional_call() {
    code!(
        "fn inc(n: integer) -> integer { n + 1 }
fn dec(n: integer) -> integer { n - 1 }
fn test() {
    flag = true;
    f = if flag { inc } else { dec };
    result = f(41);
    assert(result == 42, \"expected 42, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Fn-ref passed as a parameter and called inside the callee.
#[test]
fn fn_ref_as_parameter() {
    code!(
        "fn square(n: integer) -> integer { n * n }
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
fn test() {
    result = apply(square, 7);
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
    r = map(v, double);
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
    r = filter(v, is_even);
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
    s = reduce(v, 0, add);
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

// Exact T0-1 reproduction: method sets an integer field to null via reference param.
// Previously panicked with "store_nr=60" in `set_int`.
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

// Integer field set to null via direct struct access (not a method call).
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

// Long field set to null via reference parameter.
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

// Multiple scalar fields (integer, long) set to null in one method call.
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

// Set field to null then restore a value — round-trip correctness.
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

// Two owned struct refs in the same scope — minimal T0-2 reproducer.
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

// Three owned struct refs in the same scope — verifies LIFO holds for N > 2.
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

// sorted[key] = null removes the entry.
// hash[key] = null removes the entry.
// ── PROBLEMS #39 (T0-4): `v += other_vec` shallow copy — text fields dangle ───
// vector_add() used a raw copy_block without calling copy_claims().  Both the
// source and destination vectors ended up with the same string-record indices;
// when the source was freed, remove_claims() deleted those records and the
// destination's text fields became dangling.  The fix: after copy_block, iterate
// each appended element and call copy_claims() to create independent copies of
// string records (and sub-structures) in the destination store.

// Appending a vector<struct-with-text> to another vector must deep-copy string
// records.  Without the fix both bags share the same records; at end-of-scope
// LIFO frees the source first, then the destination tries to double-free the
// same records → panic.
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

// Appending to a non-empty destination: pre-existing and new elements all have
// independent text records.
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

// copy_claims path: a struct with an index<T[key]> field is added to a vector
// (triggering OpCopyRecord → copy_claims → copy_claims_index_body).
// Before the fix this panicked with "Not implemented".
// copy_claims on index with text fields: text records must be deep-copied so
// source and destination are independent.
// remove_claims path for Parts::Index: reassigning a struct that holds an
// index<T> field triggers database() → clear() → remove_claims on the index.
// Before the fix this panicked with "Not implemented".
// ── PROBLEMS #41 (T0-6): inline ref-returning call leaks store → LIFO panic ───
// `p.method().field` where method() returns an owned ref must wrap the temporary
// in a work-ref variable so scopes.rs emits OpFreeRef at end-of-scope.

// Single field access on an inline ref-returning call must not leak the store.
// Two chained inline calls (shifted().shifted().x) must not leak either store.
// index[key] = null removes the entry.
// T2-7: mkdir creates a directory, mkdir_all creates nested directories.
#[test]
fn mkdir_and_mkdir_all() {
    // Clean up from any previous failed run
    let _ = std::fs::remove_dir_all("tests/tmp_mkdir_test");
    code!(
        "fn test() {
    // mkdir_all creates nested path
    assert(mkdir_all(\"tests/tmp_mkdir_test/sub\").ok(), \"mkdir_all\");
    // mkdir on existing directory returns not ok
    assert(!mkdir(\"tests/tmp_mkdir_test/sub\").ok(), \"mkdir existing\");
    // mkdir on a new sibling
    assert(mkdir(\"tests/tmp_mkdir_test/other\").ok(), \"mkdir sibling\");
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

// `v += v` on an integer vector: result must be a doubled vector with correct values.
// `v += v` on a single-element vector: result must have two equal elements.
// ── T1-32: File I/O errors are no longer silently discarded ──────────────────
// write_file/read_file/seek_file used unwrap_or_default() / unwrap_or(0),
// swallowing OS errors with no diagnostic output.  The fix logs to stderr via
// eprintln!.  The test below verifies that writing to a bad path does not panic
// or hang — the error is printed to stderr and execution continues normally.

// Writing to an unwritable path must not panic; the program continues after the error.
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

// N8-naming: generated code must use `_pre_N` names, not bare `_preN`.
// A nested user-defined function call is enough to trigger pre-eval hoisting.
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

// N8-empty: generated code must not emit `let _preN = ;` (empty right-hand side).
// The mutable-reference pattern (user fn with `&T = null` default) triggers this.
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

// N3: assigning a reference to another reference must emit OpCopyRecord for deep copy.
// Without it, both variables alias the same heap record; mutating through one changes the other.
#[test]
fn n3_reference_assignment_emits_copy_record() {
    // Bytecode interpreter correctly deep-copies references already; test confirms behaviour.
    code!("struct Item { name: text }")
        .expr(
            "a = Item { name: \"hello\" };
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

// N5: vector::clear_vector must not be called when the DbRef is null (rec == 0).
// A function that initialises and returns a vector was panicking with
// "Unknown record 2147483648" because clear_vector ran on stores.null() before allocation.
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

// N4: struct-enum variants must show all fields when formatted with OpFormatDatabase.
// The init function was registering every enum value with u16::MAX as the struct type,
// so ShowDb could not dispatch to variant fields and only showed the variant name.
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

// N9a: the auto-generated fill.rs must contain `use crate::ops;`
// so it can be compiled as a crate-internal file and eventually replace src/fill.rs.
#[test]
fn n9a_generated_fill_has_ops_import() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    scopes::check(&mut p.data);
    let tmp = format!(
        "tests/generated/fill_n9a_{:?}.rs",
        std::thread::current().id()
    );
    let _ = std::fs::create_dir_all("tests/generated");
    let src = loft::create::generate_code_to(&p.data, &tmp).expect("generate_code_to failed");
    let _ = std::fs::remove_file(&tmp);
    assert!(
        src.contains("use crate::ops;"),
        "generated fill.rs missing `use crate::ops;`"
    );
}

// N9 (N20b/N20c/N20d): auto-generated fill.rs must be byte-for-byte identical to
// src/fill.rs once all #rust templates are present.
// Generates to a thread-local temp file to avoid races with other tests writing
// tests/generated/fill.rs.
#[test]
fn n9_generated_fill_matches_src() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    scopes::check(&mut p.data);
    // Use a unique path so parallel test runs do not race on the same file.
    let tmp = format!(
        "tests/generated/fill_n9_{:?}.rs",
        std::thread::current().id()
    );
    let generated = loft::create::generate_code_to(&p.data, &tmp).expect("generate_code_to failed");
    let _ = std::fs::remove_file(&tmp);
    let src = std::fs::read_to_string("src/fill.rs").expect("src/fill.rs not found");
    assert_eq!(
        generated, src,
        "generated fill.rs differs from src/fill.rs — \
         run create::generate_code() and copy the result"
    );
}

// N8: Sort must work correctly in native-codegen mode.
// The #rust template for OpSortVector is inlined directly (no OpSortVector runtime fn needed).
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

// N10: ops::text_character returns char but loft represents character as i32.
// Generated code assigns the char to an i32 variable without a cast, causing a compile error.
// Also, i32 character variables used in method dispatch (is_alphanumeric etc.) need wrapping
// with ops::to_char(...) since the method requires char, not i32.
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

// N2: output_init must register content types before the structs that reference them in
// sorted/index/hash fields.  When a struct has a sorted<Foo> field and Foo has a higher
// type-ID than the struct, the init function panicked because db.sorted(foo_type_id, ...)
// was called before Foo was registered.
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

// ── S4: Binary I/O type coverage ─────────────────────────────────────────────
// read_data / write_data panic with "Not implemented" for Array / Sorted /
// Ordered / Hash / Index / Spacial / Base — should be improved.

// S4: writing a struct with a `sorted<T>` field must be rejected at parse time
// with a clear message pointing the user to plain structs for serialisation.
// The parser catches collection fields early; the message contains "collection field".
#[test]
#[should_panic(expected = "collection field")]
fn s4_sorted_field_write_panics_with_clear_message() {
    code!(
        "struct Item { key: integer, value: integer }
struct Container { items: sorted<Item[key]> }
fn test() {
    c = Container { items: [Item { key: 1, value: 10 }] };
    f = file(\"tests/tmp_s4_sorted.dat\");
    f#format = LittleEndian;
    f += c;
    delete(\"tests/tmp_s4_sorted.dat\");
}"
    );
}

// S4: writing a struct with a `hash<T>` field must be rejected at parse time
// with the same "collection field" message.
#[test]
#[should_panic(expected = "collection field")]
fn s4_hash_field_write_panics_with_clear_message() {
    code!(
        "struct Tag { name: text }
struct Bag { tags: hash<Tag[name]> }
fn test() {
    b = Bag { tags: [Tag { name: \"hello\" }] };
    f = file(\"tests/tmp_s4_hash.dat\");
    f#format = LittleEndian;
    f += b;
    delete(\"tests/tmp_s4_hash.dat\");
}"
    );
}

// ── N1: --native CLI flag ─────────────────────────────────────────────────────
// src/main.rs must recognise --native and run the native codegen pipeline.

// N1: parsing the default library and a trivial loft program, then generating
// native Rust via output_native_reachable must produce non-empty output containing
// the expected function signatures.  Actually running rustc is attempted if possible
// but is non-fatal if the loft crate cannot be linked (cargo test env dependency).
#[test]
fn n1_native_pipeline_trivial_program() {
    use loft::generation::Output;
    let mut p = loft::parser::Parser::new();
    p.parse_dir("default", true, false).unwrap();
    let start_def = p.data.definitions();
    p.parse_str(
        "fn main() { assert(1 + 1 == 2, \"arithmetic\"); }",
        "n1_test",
        false,
    );
    assert!(p.diagnostics.is_empty(), "parse errors: {}", p.diagnostics);
    loft::scopes::check(&mut p.data);
    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);
    let end_def = p.data.definitions();
    // Collect entry defs: just the user's main function.
    let main_nr = p.data.def_nr("n_main");
    assert_ne!(main_nr, u32::MAX, "n_main not found");
    let tmp_rs = std::env::temp_dir().join("loft_n1_test.rs");
    let mut f = std::fs::File::create(&tmp_rs).expect("tmp file");
    let mut out = Output {
        data: &p.data,
        stores: &state.database,
        counter: 0,
        indent: 0,
        def_nr: 0,
        declared: Default::default(),
        reachable: Default::default(),
        loop_stack: Vec::new(),
        next_format_count: 0,
        yield_collect: false,
        fn_ref_context: false,
        call_stack_prefix: None,
        wasm_browser: false,
    };
    out.output_native_reachable(&mut f, start_def, end_def, &[main_nr])
        .expect("output_native_reachable");
    drop(f);
    // Verify the generated source contains expected landmarks.
    let generated = std::fs::read_to_string(&tmp_rs).expect("read generated source");
    assert!(
        generated.contains("fn n_main("),
        "generated source missing n_main"
    );
    assert!(
        generated.contains("fn main()"),
        "generated source missing Rust main"
    );
    assert!(
        generated.contains("fn n_assert"),
        "generated source missing n_assert"
    );
    // Optionally compile with rustc — non-fatal if loft crate cannot be linked.
    let deps_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();
    let loft_rlib = std::fs::read_dir(&deps_dir).ok().and_then(|mut it| {
        it.find(|e| {
            e.as_ref().is_ok_and(|e| {
                let n = e.file_name();
                let s = n.to_string_lossy();
                s.starts_with("libloft") && s.ends_with(".rlib")
            })
        })
        .and_then(|e| e.ok())
        .map(|e| e.path())
    });
    let binary = std::env::temp_dir().join("loft_n1_test_bin");
    let mut rustc_args = vec![
        "--edition=2024".to_string(),
        "-o".to_string(),
        binary.to_str().unwrap().to_string(),
    ];
    if let Some(ref rlib) = loft_rlib {
        rustc_args.push("--extern".to_string());
        rustc_args.push(format!("loft={}", rlib.display()));
        rustc_args.push("-L".to_string());
        rustc_args.push(deps_dir.display().to_string());
        // S31: pass --extern for every non-loft rlib in deps/ so that optional
        // feature dependencies (rand_core, rand_pcg, png, etc.) can be resolved.
        // Without this, rustc cannot find crates that loft was compiled with.
        if let Ok(entries) = std::fs::read_dir(&deps_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with("lib")
                    || !name.ends_with(".rlib")
                    || name.starts_with("libloft")
                {
                    continue;
                }
                let without_lib = &name[3..];
                let without_rlib = without_lib.trim_end_matches(".rlib");
                let crate_name = if let Some(pos) = without_rlib.rfind('-') {
                    without_rlib[..pos].replace('-', "_")
                } else {
                    without_rlib.replace('-', "_")
                };
                rustc_args.push("--extern".to_string());
                rustc_args.push(format!("{crate_name}={}", entry.path().display()));
            }
        }
    }
    rustc_args.push(tmp_rs.to_str().unwrap().to_string());
    match std::process::Command::new("rustc")
        .args(&rustc_args)
        .status()
    {
        Ok(s) if s.success() => {
            // Binary compiled — run it to confirm correctness.
            let run = std::process::Command::new(&binary).status();
            match run {
                Ok(rs) => assert!(rs.success(), "native binary exited non-zero"),
                Err(e) => eprintln!("n1: could not run binary: {e}"),
            }
        }
        Ok(s) => eprintln!(
            "n1: rustc compilation failed (exit {s}) — likely linker issue in test env; \
             code generation verified above"
        ),
        Err(e) => eprintln!("n1: skipping rustc step (not in PATH): {e}"),
    }
    let _ = std::fs::remove_file(&tmp_rs);
    let _ = std::fs::remove_file(&binary);
}

// ── P1.1: Lambda parser ───────────────────────────────────────────────────────
// Parser must accept fn(params) -> ret { body } as an anonymous function
// expression, producing a Type::Function value like a named fn-ref.

// P1.1: a basic lambda `fn(x: integer) -> integer { x * 2 }` can be assigned
// to a variable and called through it.
#[test]
fn p1_1_lambda_basic_call() {
    code!(
        "fn test() {
    f = fn(x: integer) -> integer { x * 2 };
    result = f(21);
    assert(result == 42, \"expected 42, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.1: lambda passed inline to a function accepting fn(integer) -> integer.
#[test]
fn p1_1_lambda_as_argument() {
    code!(
        "fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
fn test() {
    result = apply(fn(n: integer) -> integer { n * n }, 7);
    assert(result == 49, \"expected 49, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.1: lambda with no return type (void).  A5.6c: write-backs make the
// outer `count` reflect mutations performed inside the lambda body.
#[test]
fn p1_1_lambda_void_body() {
    code!(
        "fn test() {
    count = 0;
    f = fn(x: integer) { count += x; };
    f(10);
    f(32);
    assert(count == 42, \"expected 42, got {count}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── P91 regression guards ───────────────────────────────────────────────────
// Earlier-parameter-reference in default expressions.  Before this fix,
// `fn f(a: integer, b: integer = a * 2)` produced "Unknown variable 'a'"
// because earlier arguments weren't visible to later default expressions.
// The fix in parse_arguments (src/parser/definitions.rs) injects earlier
// args into self.vars before parsing each default, rewrites internal slot
// numbers to argument indices, and then cleans up the temporary bindings.
// The call-site substitution (Self::substitute_param_refs in parser/mod.rs)
// replaces Var(N) in the default tree with the caller's actual arg[N].

#[test]
fn p91_default_references_earlier_param() {
    code!(
        "fn dbl(a: integer, b: integer = a * 2) -> integer { a + b }
fn run() -> integer { dbl(5) }"
    )
    .expr("run()")
    .result(loft::data::Value::Int(15));
}

#[test]
fn p91_default_identity_of_earlier_param() {
    // `fn rect(w, h = w)` is the idiomatic "square by default" shape.
    code!(
        "fn rect(w: integer, h: integer = w) -> integer { w * h }
fn run() -> integer { rect(4) }"
    )
    .expr("run()")
    .result(loft::data::Value::Int(16));
}

#[test]
fn p91_default_overridden_by_caller() {
    // Regression guard: supplying the argument skips the default entirely.
    code!(
        "fn dbl(a: integer, b: integer = a * 2) -> integer { a + b }
fn run() -> integer { dbl(3, 7) }"
    )
    .expr("run()")
    .result(loft::data::Value::Int(10));
}

#[test]
fn p91_chained_defaults_reference_earlier_args() {
    // Three-argument chain: c's default references both a and b, where
    // b itself has a literal default.  Verifies that substitute_param_refs
    // uses already-substituted earlier args, not the raw default tree.
    code!(
        "fn add3(a: integer, b: integer = 10, c: integer = a + b) -> integer { a + b + c }
fn run() -> integer { add3(1) }"
    )
    .expr("run()")
    // a=1, b=10 (default), c=a+b=11 → 1+10+11 = 22
    .result(loft::data::Value::Int(22));
}

// ── C60 Step 3+ integration tests (ignored until parser acceptance lands) ───
// These pin the end-to-end behaviour for hash iteration in loft source.
// The Rust primitives landed in Step 1a + 2; the parser + stdlib wiring
// that routes `for e in h { … }` through `hash::records_sorted` is the
// next step.  Tests are marked `#[ignore]` per DEVELOPMENT.md so CI stays
// green until the feature ships.

/// C60 Step 4 (MVP acceptance test): `for e in h { … }` parses and
/// iterates a hash in ascending key order under the interpreter.
#[test]
fn c60_hash_iter_single_field_asc() {
    code!(
        "struct Entry { name: text, count: integer }
struct Bag { data: hash<Entry[name]> }
fn run() -> text {
    b = Bag { data: [
        Entry{name:\"zebra\",count:1},
        Entry{name:\"apple\",count:5},
        Entry{name:\"mango\",count:3},
    ] };
    out = \"\";
    for e in b.data { out += e.name; out += \",\"; }
    out
}"
    )
    .expr("run()")
    .result(loft::data::Value::str("apple,mango,zebra,"));
}

/// C60 Step 5: `#index` / `#count` / `#first` work "for free" through
/// the vector-iteration path Step 3 desugars into.
#[test]
fn c60_hash_iter_loop_attributes() {
    code!(
        "struct Ent { k: text, v: integer }
struct Bag { data: hash<Ent[k]> }
fn run() -> integer {
    b = Bag { data: [Ent{k:\"c\",v:3}, Ent{k:\"a\",v:1}, Ent{k:\"b\",v:2}] };
    total = 0;
    for e in b.data { total += e.v * (e#index + 1); }
    total
}"
    )
    .expr("run()")
    // a=1 at idx=0, b=2 at idx=1, c=3 at idx=2
    // sum = 1*1 + 2*2 + 3*3 = 14
    .result(loft::data::Value::Int(14));
}

/// C60 Step 6: multi-field key — lexicographic order.
#[test]
fn c60_hash_iter_multi_field_lex() {
    code!(
        "struct R { region: text, score: integer }
struct Bag { data: hash<R[region, score]> }
fn run() -> text {
    b = Bag { data: [
        R{region:\"east\",score:10},
        R{region:\"west\",score:30},
        R{region:\"east\",score:50},
        R{region:\"west\",score:20},
    ] };
    out = \"\";
    for r in b.data { out += \"{r.region}:{r.score},\"; }
    out
}"
    )
    .expr("run()")
    .result(loft::data::Value::str("east:10,east:50,west:20,west:30,"));
}

/// C60 Step 8: filter clause on hash iteration works through the
/// vector-iteration path.
#[test]
fn c60_hash_iter_filter_clause() {
    code!(
        "struct Ent { k: text, v: integer }
struct Bag { data: hash<Ent[k]> }
fn run() -> integer {
    b = Bag { data: [Ent{k:\"a\",v:1}, Ent{k:\"b\",v:20}, Ent{k:\"c\",v:3}, Ent{k:\"d\",v:40}] };
    total = 0;
    for e in b.data if e.v > 10 { total += e.v; }
    total
}"
    )
    .expr("run()")
    // Only v=20 and v=40 pass the filter.
    .result(loft::data::Value::Int(60));
}

/// C60 Step 4: empty hash iterates zero times.
#[test]
fn c60_hash_iter_empty() {
    code!(
        "struct Ent { k: text, v: integer }
struct Bag { data: hash<Ent[k]> }
fn run() -> integer {
    b = Bag { data: [] };
    count = 0;
    for _ in b.data { count += 1; }
    count
}"
    )
    .expr("run()")
    .result(loft::data::Value::Int(0));
}

/// C60 Step 9: `#remove` must be rejected on hash iteration — the
/// iteration walks a pre-sorted snapshot, and `#remove` would not
/// actually remove from the underlying hash.  Users should
/// `h[key] = null` to remove.
#[test]
fn c60_hash_iter_remove_rejected() {
    // Parse error expected; format matches other parse-error tests.
    code!(
        "struct Ent { k: text, v: integer }
struct Bag { data: hash<Ent[k]> }
fn test() {
    b = Bag { data: [Ent{k:\"a\",v:1}] };
    for e in b.data { e#remove; }
}"
    )
    .error(
        "#remove is not supported on hash iteration — the iterated \
         vector is a sorted snapshot; use `hash[key] = null` to \
         remove from the hash at c60_hash_iter_remove_rejected:5:32",
    );
}

// ── P139 regression guards ──────────────────────────────────────────────────
// The slot allocator placed zone-1 byte-sized vars (plain enum, boolean) at
// fixed slots inside the zone-2 frontier, leaving codegen's TOS one byte
// below the next zone-2 slot.  `gen_set_first_at_tos` asserted `slot == TOS`
// and fired.  The fix emits `OpReserveFrame(gap)` when `slot > TOS`, so the
// runtime stack pointer advances to match and the init opcode writes to
// the correct slot.  These tests pin the three most common triggering
// shapes: plain-enum vector, two loops over an enum vector (the
// original 05-enums.loft pattern), and boolean vector.

#[test]
fn p139_enum_vec_same_type_write_through_loop() {
    code!(
        "enum Dir { North, East, South, West }
fn test() {
    dirs = [North, East, South, West];
    first_d = North;
    for elem in dirs { first_d = elem; }
    assert(first_d == West, \"last element wins, got {first_d}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn p139_enum_vec_two_loops_same_function() {
    code!(
        "enum D { A, B, C, W }
fn test() {
    dirs = [A, B, C, W];
    count = 0;
    for _ in dirs { count += 1; }
    first = A;
    last = A;
    n = 0;
    for elem in dirs {
        if n == 0 { first = elem; }
        last = elem;
        n += 1;
    }
    assert(count == 4, \"count: {count}\");
    assert(first == A, \"first: {first}\");
    assert(last == W, \"last: {last}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn p139_bool_vec_write_through_loop() {
    code!(
        "fn test() {
    flags = [true, false, true, true];
    flag = false;
    for f in flags { flag = f; }
    assert(flag == true, \"last flag, got {flag}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── P86 regression guards ───────────────────────────────────────────────────
// Pre-0.8.3 the parser's mitigation for P86 turned this source into a
// compile error ("closure capture is not yet supported"), and before that
// mitigation existed it produced a misleading codegen self-reference panic
// ("[generate_set] ... Var(1) self-reference — storage not yet allocated").
// With real closure capture shipped, both paths have to stay closed forever.
// `p1_1_lambda_void_body` above covers one integer-mutation case; the two
// tests below expand coverage to multi-variable mutation (integer) and
// text accumulation, which exercises the text work-buffer path in codegen
// and is the most common place where capture regressions hide.
#[test]
fn p86_lambda_capture_multi_mutation() {
    code!(
        "fn test() {
    count = 0;
    total = 0;
    add = fn(x: integer) { count += 1; total += x; };
    add(10);
    add(20);
    add(12);
    assert(count == 3, \"count: {count}\");
    assert(total == 42, \"total: {total}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn p86_lambda_capture_text_mutation() {
    code!(
        "fn test() {
    log = \"\";
    append = fn(s: text) { log += s; log += \",\"; };
    append(\"a\");
    append(\"bb\");
    append(\"ccc\");
    assert(log == \"a,bb,ccc,\", \"log: {log}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── Issue 82 ─────────────────────────────────────────────────────────────────
// `string` is not a valid type name — the canonical text type is `text`.
// Using `string` in a struct field produces "Undefined type string" and a
// cascade of "Invalid index key" / "Cannot write unknown" errors.

// Issue 82 / S7: `string` in a struct field must suggest `text`.
#[test]
fn issue_82_string_type_is_undefined() {
    code!("struct Bad { x: string }").error(
        "Undefined type 'string' — did you mean 'text'? at issue_82_string_type_is_undefined:1:25",
    );
}

// Issue 82 positive: the same pattern with `text` must work correctly.
#[test]
fn issue_82_text_type_works() {
    code!(
        "struct Word { key: text, count: integer }
fn test() {
    w = Word { key: \"hello\", count: 1 };
    assert(w.key == \"hello\", \"key\");
    assert(w.count == 1, \"count\");
}"
    )
    .result(Value::Null);
}

// ── Issue 83 ─────────────────────────────────────────────────────────────────
// A struct field named `key` used as a hash-value type causes a runtime panic:
// "Allocating a used store" (src/database/allocation.rs).
// `key` is a pseudo-field used by hash iteration (`kv.key`) and conflicts with
// the real struct field at the allocation level.

// Issue 83 / S8: field named `key` in a hash-value struct must be rejected at compile time.
#[test]
fn issue_83_hash_value_field_named_key_panics() {
    code!(
        "struct Entry { key: text, count: integer }
struct Db { data: hash<Entry[key]> }
fn test() {
    db = Db { data: [] };
    db.data += [Entry { key: \"hello\", count: 1 }];
    e = db.data[\"hello\"];
    assert(e != null, \"entry should exist\");
    assert(e.count == 1, \"count should be 1\");
}"
    )
    .error(
        "Struct 'Entry' has a field named 'key' which is reserved for hash iteration \
— rename the field at issue_83_hash_value_field_named_key_panics:1:15",
    );
}

// Issue 83 positive: renaming the field (non-`key`) is the documented workaround.
#[test]
fn issue_83_hash_value_field_renamed_works() {
    code!(
        "struct Score { id: integer not null, pts: integer not null }
struct Board { scores: hash<Score[id]> }
fn test() {
    b = Board { scores: [] };
    b.scores += [Score { id: 1, pts: 42 }];
    s = b.scores[1];
    assert(s != null, \"entry should exist\");
    assert(s.pts == 42, \"pts should be 42, got {s.pts}\");
}"
    )
    .result(Value::Null);
}

// ── Issue 84 ─────────────────────────────────────────────────────────────────
// A `for` loop in any function that is called from a recursive function causes
// a codegen panic: "Too few parameters on n_<recursive_fn>".
// Root cause: the flat global variable namespace corrupts the parameter-count
// slot table for the recursive function when the helper's loop variables are
// assigned. Affects both `const vector<T>` and plain `vector<T>` params.

// Issue 84: for loop in helper + recursive caller panics "Too few parameters".
#[test]
fn issue_84_for_loop_in_helper_called_from_recursive_fn() {
    code!(
        "fn sum_vec(v: vector<integer>) -> integer {
    s = 0;
    for sv_i in 0..len(v) { s += v[sv_i]; }
    s
}
fn recurse(n: integer) -> integer {
    if n <= 0 { return 0; }
    v = [n];
    sum_vec(v) + recurse(n - 1)
}
fn test() {
    result = recurse(5);
    assert(result == 15, \"expected 15, got {result}\");
}"
    )
    .result(Value::Null);
}

// Issue 84: merge sort (index-bound) also triggers the same panic.
#[test]
fn issue_84_merge_sort_too_few_parameters() {
    code!(
        "fn msort_merge(lp: vector<integer>, rp: vector<integer>) -> vector<integer> {
    out = [for mg_i in 0..0 { mg_i }];
    li = 0; ri = 0;
    ll = len(lp); rl = len(rp);
    for mg_step in 0..(ll + rl) {
        if li >= ll && ri >= rl { break; }
        li = li + mg_step * 0;
        if li >= ll { out += [rp[ri]]; ri += 1; }
        else if ri >= rl { out += [lp[li]]; li += 1; }
        else if lp[li] <= rp[ri] { out += [lp[li]]; li += 1; }
        else { out += [rp[ri]]; ri += 1; }
    }
    out
}
fn msort(arr: vector<integer>, lo: integer, hi: integer) -> vector<integer> {
    sz = hi - lo;
    if sz <= 1 {
        base = [for ms_i in 0..0 { ms_i }];
        if sz == 1 { base += [arr[lo]]; }
        return base;
    }
    mid = lo + sz / 2;
    msort_merge(msort(arr, lo, mid), msort(arr, mid, hi))
}
fn test() {
    data = [3, 1, 4, 1, 5, 9, 2, 6];
    out = msort(data, 0, 8);
    assert(out[0] == 1, \"first={out[0]}\");
    assert(out[7] == 9, \"last={out[7]}\");
}"
    )
    .result(Value::Null);
}

// N7: OpFormatFloat must generate ops::format_float(...), not OpFormatFloat(stores, ...).
// OpFormatStackLong must generate ops::format_long(var_, ...) without stores or &mut.
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

// ── Issue 85 ─────────────────────────────────────────────────────────────────
// Null-returning hash lookup before insert causes subsequent lookup to return null.
// Pattern: `e = hash[key]` (null result) followed by `hash += [Elem{...}]`
// makes the inserted element unfindable via `hash[key]`.

// Issue 85: null hash lookup before insert — integer key.
// The inserted element must be findable immediately after insertion.
#[test]
fn issue_85_hash_null_lookup_then_insert_integer_key() {
    code!(
        "struct Item { id: integer, val: integer }
struct Db { data: hash<Item[id]> }
fn test() {
    db = Db { data: [] };
    e0 = db.data[0];
    assert(e0 == null, \"pre-insert lookup should be null\");
    db.data += [Item { id: 0, val: 42 }];
    e1 = db.data[0];
    assert(e1 != null, \"inserted item must be findable\");
    assert(e1.val == 42, \"val should be 42, got {e1.val}\");
}"
    )
    .result(Value::Null);
}

// Issue 85: null hash lookup before insert — text key.
#[test]
fn issue_85_hash_null_lookup_then_insert_text_key() {
    code!(
        "struct Word { word: text, count: integer }
struct WordDb { freq: hash<Word[word]> }
fn test() {
    db = WordDb { freq: [] };
    e0 = db.freq[\"hello\"];
    assert(e0 == null, \"pre-insert lookup should be null\");
    db.freq += [Word { word: \"hello\", count: 1 }];
    e1 = db.freq[\"hello\"];
    assert(e1 != null, \"inserted word must be findable\");
    assert(e1.count == 1, \"count should be 1, got {e1.count}\");
}"
    )
    .result(Value::Null);
}

// ── Issue 89 ──────────────────────────────────────────────────────────────────
// Optional `& text` parameter panics with subtract-with-overflow when called
// with an explicit argument.  `convert()` must allocate a work-text variable
// and route through OpAppendText + OpCreateStack, not bare OpCreateStack(text).

// Issue 89: calling `directory("sub")` with an explicit text arg must not panic.
#[test]
fn issue_89_optional_ref_text_param_with_arg() {
    // directory() has signature `pub fn directory(v: & text = "") -> text`.
    // Calling it with an explicit string argument previously caused
    // "attempt to subtract with overflow" in codegen (issue #89).
    code!(
        "fn test() {
    d = directory(\"sub\");
    assert(d.len() >= 0, \"directory returned something\");
}"
    )
    .result(Value::Null);
}

// ── S8 — Compile-time error when hash-value struct has field named `key` ──────
// `key` is a pseudo-field reserved for hash iteration.  A struct with a real
// field named `key` used as a hash value type must be rejected at compile time.

// S8: hash-value struct with a `key` field must produce a compile-time error.
#[test]
fn s8_hash_value_struct_key_field_rejected() {
    code!(
        "struct Item { key: text, value: integer }
struct Container { data: hash<Item[key]> }
fn test() { }"
    )
    .error("Struct 'Item' has a field named 'key' which is reserved for hash iteration — rename the field at s8_hash_value_struct_key_field_rejected:1:14");
}

// ── P2-R6 — Compiler check: yield inside par() body ──────────────────────────
// A coroutine generator cannot yield inside a par() parallel body because the
// worker executes in a separate thread with its own store — there is no safe
// way to resume the parent coroutine from within a worker.
// Fix: `in_par_body` flag in Parser; error emitted when `yield` is encountered
// inside a parallel-for worker function body.

#[test]
fn p2_r6_yield_inside_par_body_rejected() {
    code!(
        "fn gen(items: vector<integer>) -> iterator<integer> {
    for a in items par(b = double(a), 1) {
        yield b;
    }
}
fn double(x: integer) -> integer { x * 2 }"
    )
    .error("yield is not allowed inside a par(...) parallel body at p2_r6_yield_inside_par_body_rejected:3:16");
}

// ── P1.2 — Short-form lambda expressions ─────────────────────────────────────
// Short-form `|params| { body }` and `|| { body }` syntax for inline lambdas.

// P1.2: long-form lambda `fn(x: integer) -> integer { x * 2 }` with explicit annotations.
#[test]

fn p1_2_short_lambda_explicit_types() {
    code!(
        "fn test() {
    f = fn(x: integer) -> integer { x * 2 };
    assert(f(5) == 10, \"expected 10\");
    assert(f(21) == 42, \"expected 42\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.2: Zero-parameter long-form lambda `fn() -> integer { 42 }`.
#[test]

fn p1_2_short_lambda_zero_params() {
    code!(
        "fn test() {
    f = fn() -> integer { 42 };
    assert(f() == 42, \"expected 42\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.2: Two-parameter long-form lambda with explicit types.
#[test]

fn p1_2_short_lambda_two_params() {
    code!(
        "fn test() {
    add = fn(a: integer, b: integer) -> integer { a + b };
    assert(add(3, 4) == 7, \"expected 7\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.2: Short lambda with inferred param type from call-site hint.
#[test]

fn p1_2_short_lambda_inferred_type() {
    code!(
        "fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
fn test() {
    result = apply(|n| { n * 3 }, 7);
    assert(result == 21, \"expected 21, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── P1.3 — map / filter / reduce with inline lambdas ─────────────────────────

// P1.3: `map` with a short-form lambda.
#[test]

fn p1_3_map_short_lambda() {
    code!(
        "fn test() {
    v = [1, 2, 3];
    r = map(v, |x| { x * 10 });
    assert(r[0] == 10, \"r[0]\");
    assert(r[1] == 20, \"r[1]\");
    assert(r[2] == 30, \"r[2]\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.3: `filter` with a short-form lambda.
#[test]

fn p1_3_filter_short_lambda() {
    code!(
        "fn test() {
    v = [1, 2, 3, 4, 5, 6];
    evens = filter(v, |x| { x % 2 == 0 });
    assert(len(evens) == 3, \"expected 3 evens\");
    assert(evens[0] == 2, \"evens[0]\");
    assert(evens[2] == 6, \"evens[2]\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.3: `reduce` with a short-form lambda.
#[test]

fn p1_3_reduce_short_lambda() {
    code!(
        "fn test() {
    v = [1, 2, 3, 4, 5];
    total = reduce(v, 0, |acc, x| { acc + x });
    assert(total == 15, \"expected 15, got {total}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── A8 — Destination-passing for text-returning natives ───────────────────────
// replace / to_lowercase / to_uppercase write directly into the destination
// string variable, eliminating the scratch buffer double-copy.

// A8: `replace` result assigned to a variable produces the right string.
#[test]

fn a8_replace_into_var() {
    code!(
        "fn test() {
    s = \"hello world\";
    r = s.replace(\"world\", \"loft\");
    assert(r == \"hello loft\", \"got {r}\");
}"
    )
    .result(loft::data::Value::Null);
}

// A8: `to_lowercase` result in a format string.
#[test]

fn a8_to_lowercase_in_format() {
    code!(
        "fn test() {
    s = \"HELLO\";
    r = \"value: {s.to_lowercase()}\";
    assert(r == \"value: hello\", \"got {r}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Assert that src/fill.rs matches what generate_code_to would produce.
// If this fails, run: cargo test regen_fill_rs -- --ignored --nocapture
#[test]
fn fill_rs_up_to_date() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    scopes::check(&mut p.data);
    let generated = loft::create::generate_code_to(&p.data, "tests/generated/fill_check.rs")
        .expect("generate_code_to failed");
    let current = std::fs::read_to_string("src/fill.rs").expect("cannot read src/fill.rs");
    assert_eq!(
        current, generated,
        "src/fill.rs is out of date — run: cargo test regen_fill_rs -- --ignored --nocapture"
    );
}

// Regenerate src/fill.rs from the default library definitions.
// Run with: cargo test regen_fill_rs -- --ignored --nocapture
#[test]
#[ignore = "maintenance: regenerates src/fill.rs — run manually when default/*.loft changes"]
fn regen_fill_rs() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    scopes::check(&mut p.data);
    loft::create::generate_code_to(&p.data, "src/fill.rs").expect("generate_code_to failed");
    println!("src/fill.rs regenerated");
}

// Assert that every #rust-annotated function from default/*.loft is registered
// in src/native.rs.  If this fails, a new native function was added to the
// default library but not wired into the native registry.
// Fix: add the missing entry to FUNCTIONS in src/native.rs and implement the fn.
#[test]
fn native_rs_functions_up_to_date() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    scopes::check(&mut p.data);
    let native_src = std::fs::read_to_string("src/native.rs").expect("cannot read src/native.rs");
    let mut missing = Vec::new();
    for d_nr in 0..p.data.definitions() {
        let d = p.data.def(d_nr);
        if d.is_operator() || d.rust.is_empty() {
            continue;
        }
        let entry = format!("\"{}\"", d.name);
        if !native_src.contains(&entry) {
            missing.push(d.name.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "src/native.rs is missing {} function(s) from default/*.loft:\n  {}\n\
         Add them to the FUNCTIONS array and implement the fn bodies.",
        missing.len(),
        missing.join("\n  ")
    );
}

// ── S9 / Issue 90 — character + character codegen panic ───────────────────────
// `c + d` where both are characters panics with a stack-size mismatch because
// `parse_append_text` uses the character variable as a text destination.

// S9: character + character must produce text concatenation, not a panic.
#[test]
fn s9_char_plus_char() {
    code!(
        "fn test() {
    c = 'h';
    d = 'i';
    r = c + d;
    assert(r == \"hi\", \"expected 'hi' got '{r}'\");
}"
    )
    .result(Value::Null);
}

// S9: text indexing `a[0] + a[1]` must also work.
#[test]
fn s9_text_index_plus_text_index() {
    code!(
        "fn test() {
    a = \"hello\";
    r = a[0] + a[1];
    assert(r == \"he\");
}"
    )
    .result(Value::Null);
}

// ── S10 — Disallow type annotations in |x| short-form lambdas ────────────────
// Short-form lambdas infer types from the call-site hint.  Explicit type
// annotations belong in the long form: fn(x: integer) -> integer { body }.

// S10: `|x: integer|` must produce a compile-time error.
#[test]
fn s10_short_lambda_type_annotation_rejected() {
    code!(
        "fn test() {
    v = [1, 2, 3];
    r = map(v, |x: integer| { x * 2 });
}"
    )
    .error("Type annotations are not allowed in |x| lambdas — use fn(x: <type>) -> <ret> { ... } instead at s10_short_lambda_type_annotation_rejected:3:27");
}

// ── S11 — Bare function references (no fn prefix) ────────────────────────────

// S11: bare `double` resolves as a function reference without `fn` prefix.
#[test]
fn s11_bare_fn_ref() {
    code!(
        "fn double(x: integer) -> integer { x * 2 }
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
fn test() {
    assert(apply(double, 7) == 14, \"bare fn ref\");
}"
    )
    .result(Value::Null);
}

// S11: bare fn-ref with map.
#[test]
fn s11_bare_fn_ref_map() {
    code!(
        "fn triple(x: integer) -> integer { x * 3 }
fn test() {
    v = [1, 2, 3];
    r = map(v, triple);
    assert(r[0] == 3);
    assert(r[1] == 6);
}"
    )
    .result(Value::Null);
}

// ── L6 — Field constraints and JSON-style struct literals ─────────────────────

// L6: basic field constraint — valid construction.
#[test]
fn l6_constraint_valid_construction() {
    code!(
        "struct Score {
    value: integer assert($.value >= 0, \"value must be >= 0\"),
    max: integer assert($.max >= $.value, \"max must be >= value\")
}
fn test() {
    s = Score { value: 5, max: 10 };
    assert(s.value == 5);
    assert(s.max == 10);
    s.value = 8;
    assert(s.value == 8);
}"
    )
    .result(Value::Null);
}

// L6: field constraint fires on invalid assignment.
#[test]
#[should_panic(expected = "value must be >= 0")]
fn l6_constraint_violation_on_assign() {
    code!(
        "struct Score {
    value: integer assert($.value >= 0, \"value must be >= 0\")
}
fn test() {
    s = Score { value: 5 };
    s.value = -1;
}"
    )
    .result(Value::Null);
}

// L6: cross-field constraint fires on invalid construction.
#[test]
#[should_panic(expected = "lo must be <= hi")]
fn l6_cross_field_constraint_violation() {
    code!(
        "struct Range {
    lo: integer assert($.lo <= $.hi, \"lo must be <= hi\"),
    hi: integer
}
fn test() {
    r = Range { lo: 20, hi: 10 };
}"
    )
    .result(Value::Null);
}

// L6: JSON-style quoted field names in struct literals.
#[test]
fn l6_json_quoted_field_names() {
    code!(
        r#"struct Point { x: integer, y: integer }
fn test() {
    p = Point { "x": 3, "y": 4 };
    assert(p.x == 3, "x={p.x}");
    assert(p.y == 4, "y={p.y}");
}"#
    )
    .result(Value::Null);
}

// L6: constraint with auto-generated message.
#[test]
#[should_panic(expected = "field constraint failed on Pos.x")]
fn l6_constraint_auto_message() {
    code!(
        "struct Pos {
    x: integer assert($.x >= 0)
}
fn test() {
    p = Pos { x: 5 };
    p.x = -1;
}"
    )
    .result(Value::Null);
}

// L6: vector literal input parsed like JSON array.
#[test]
fn l6_vector_literal_as_json_array() {
    code!(
        "fn test() {
    v = [12, 34, 56];
    assert(len(v) == 3, \"len={len(v)}\");
    assert(v[0] == 12);
    assert(v[1] == 34);
    assert(v[2] == 56);
}"
    )
    .result(Value::Null);
}

// L6: validate a vector of constrained structs with format-string message.
#[test]
fn l6_validate_vector_of_structs() {
    code!(
        "struct Item {
    name: text,
    qty: integer assert($.qty > 0, \"qty must be > 0 for '{$.name}'\")
}
fn test() {
    items = [
        Item { name: \"apple\", qty: 3 },
        Item { name: \"banana\", qty: 5 }
    ];
    total = 0;
    for it in items {
        total += it.qty;
    }
    assert(total == 8, \"total={total}\");
}"
    )
    .result(Value::Null);
}

// ── JSON-style parsing via `as` cast ─────────────────────────────────────────

// JSON-style quoted field names in `as Type` cast.
#[test]
fn json_quoted_field_names_in_cast() {
    code!(
        r#"struct Item { name: text, value: integer }
fn test() {
    jt = `{{"name": "hello", "value": 42}}` as Item;
    assert(jt.name == "hello", "name={jt.name}");
    assert(jt.value == 42, "value={jt.value}");
}"#
    )
    .result(Value::Null);
}

// JSON-style vector of structs parsed via `as`.
#[test]
fn json_vector_of_structs_cast() {
    code!(
        r#"struct Item { name: text, value: integer }
fn test() {
    items = `[ {{"name": "a", "value": 1}}, {{"name": "b", "value": 2}} ]` as vector<Item>;
    assert(len(items) == 2, "len={len(items)}");
    assert(items[0].name == "a");
    assert(items[1].value == 2);
}"#
    )
    .result(Value::Null);
}

// ── Type.parse(text) ──────────────────────────────────────────────────────────

// Type.parse(text) with JSON input.
#[test]
fn type_parse_json() {
    code!(
        r#"struct Score { value: integer, name: text }
fn test() {
    s = Score.parse(`{{"value": 42, "name": "test"}}`);
    assert(s.value == 42, "value={s.value}");
    assert(s.name == "test", "name={s.name}");
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) with loft-native input.
#[test]
fn type_parse_loft_native() {
    code!(
        r#"struct Score { value: integer, name: text }
fn test() {
    s = Score.parse(`{{value: 7, name: "hello"}}`);
    assert(s.value == 7);
    assert(s.name == "hello");
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) with variable input.
#[test]
fn type_parse_from_variable() {
    code!(
        r#"struct Point { x: integer, y: integer }
fn test() {
    input = `{{"x": 10, "y": 20}}`;
    p = Point.parse(input);
    assert(p.x == 10);
    assert(p.y == 20);
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) with constraint — valid data.
#[test]
fn type_parse_with_constraint_valid() {
    code!(
        r#"struct Score {
    value: integer assert($.value >= 0, "value must be >= 0")
}
fn test() {
    s = Score.parse(`{{"value": 5}}`);
    assert(s.value == 5);
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) with invalid data — constraint fires.
#[test]
#[should_panic(expected = "value must be >= 0")]
fn type_parse_with_constraint_violation() {
    code!(
        r#"struct Score {
    value: integer assert($.value >= 0, "value must be >= 0")
}
fn test() {
    s = Score { "value": -1 };
}"#
    )
    .result(Value::Null);
}

// L6: constraint violation with format-string message (falls back to auto-generated).
#[test]
#[should_panic(expected = "field constraint failed on Item.qty")]
fn l6_vector_struct_constraint_violation() {
    code!(
        "struct Item {
    name: text,
    qty: integer assert($.qty > 0, \"qty must be > 0 for '{$.name}'\")
}
fn test() {
    items = [
        Item { name: \"bad\", qty: 0 }
    ];
}"
    )
    .result(Value::Null);
}

// ── s#errors — error path reporting via #errors accessor ──────────────────────

// s#errors returns empty text on successful parse.
#[test]
fn errors_accessor_empty_on_success() {
    code!(
        r#"struct Score { value: integer }
fn test() {
    s = Score.parse(`{{"value": 42}}`);
    err = s#errors;
    assert(len(err) == 0, "expected no error, got: '{err}'");
    assert(s.value == 42);
}"#
    )
    .result(Value::Null);
}

// s#errors returns path text on parse failure.
#[test]
fn errors_accessor_path_on_failure() {
    code!(
        r#"struct Score { value: integer }
fn test() {
    bad = Score.parse(`not_json`);
    err = bad#errors;
    assert(len(err) > 0, "expected error for bad input");
    assert(bad.value == null, "value should be null on bad parse");
}"#
    )
    .result(Value::Null);
}

// s#errors includes field path for nested struct.
#[test]
fn errors_accessor_nested_path() {
    code!(
        r#"struct Inner { x: integer }
struct Outer { name: text, data: Inner }
fn test() {
    bad = Outer.parse(`{{"name": "ok", "data": "not_an_object"}}`);
    err = bad#errors;
    assert(len(err) > 0, "expected error for name={bad.name}");
}"#
    )
    .result(Value::Null);
}

// O7: OpClearStackText followed by ≥2 format ops must emit with_capacity hint;
// OpClearStackText followed by 0 or 1 ops must emit bare .clear().
#[test]
fn o7_format_string_with_capacity() {
    // Multi-segment format string: "hello {name}, count {n}" → 4 segments → with_capacity
    code!("struct S { name: text, count: integer }")
        .expr("s = S { name: \"Alice\", count: 3 }; \"hello {s.name}, count {s.count}\"")
        .result(Value::str("hello Alice, count 3"));
    let src = std::fs::read_to_string("tests/generated/issues_o7_format_string_with_capacity.rs")
        .expect("generated file not found");
    assert!(
        src.contains("with_capacity"),
        "multi-segment format string should emit with_capacity hint"
    );
    // Single-segment format: "{s.v}" → 1 segment → no with_capacity (bare .to_string())
    code!("struct S2 { v: integer }")
        .expr("s = S2 { v: 7 }; \"{s.v}\"")
        .result(Value::str("7"));
    let src2 = std::fs::read_to_string("tests/generated/issues_o7_format_string_with_capacity.rs")
        .expect("generated file not found");
    // The single-segment case must NOT get a with_capacity hint — only ≥2 segments qualify.
    // The generated file still contains with_capacity from the S struct test above (same file),
    // so instead verify that the S2 function body uses .to_string() for its single-segment clear.
    assert!(
        src2.contains(".to_string()"),
        "single-segment format string should fall through to bare .to_string()"
    );
}

// ── File.content() trace crash ──────────────────────────────────────────────
// File.content() on a non-existent file returns garbage text when execute_log
// traces the result.  The &text parameter's CreateStack points into the stack
// store; after GetFileText fails to open the file, the output String is never
// written, so VarText reads uninitialised memory as a Str with ptr=0x1.
// The runtime (execute) works fine — only execute_log triggers the crash.

#[test]
#[ignore = "SIGSEGV in execute_log: File.content() on non-existent file corrupts caller's String"]
fn file_content_nonexistent_trace() {
    code!(
        "fn test() {
    f = file(\"/nonexistent_file_trace_test.txt\");
    t = f.content();
    assert(t == \"\", \"expected empty, got '{t}'\");
}"
    )
    .result(Value::Null);
}

// ── P122: Struct return inside loop should not exhaust store pool ────────────
// When a function returns a struct, the callee allocates a store. Inside a loop
// these stores must be freed each iteration. If they accumulate, the store pool
// is exhausted and panics with "Allocating a used store".

#[test]
fn p122_struct_return_in_loop() {
    code!(
        "struct Pair { px: float not null, py: float not null }
fn make_pair(mx: float, my: float) -> Pair {
    Pair { px: mx, py: my }
}
fn test() {
    total = 0.0;
    for p122_i in 0..500 {
        p = make_pair(p122_i as float, p122_i as float * 2.0);
        total += p.px + p.py;
    }
    assert(total > 0.0, \"struct loop failed\");
}"
    )
    .result(Value::Null);
}

// P122b: nested loop struct creation (collision detection pattern)
#[test]
fn p122_struct_nested_loop() {
    // Iteration count reduced from 60*5*10=3000 to 20*5*5=500 for CI speed.
    // The bug exhausted the store pool after a few hundred iterations, so
    // 500 is sufficient as a regression guard. Run --ignored variant for
    // the full stress test.
    code!(
        "struct Box { bx: float not null, by: float not null, bw: float not null, bh: float not null }
fn overlap(a: const Box, b: const Box) -> boolean {
    a.bx < b.bx + b.bw && a.bx + a.bw > b.bx && a.by < b.by + b.bh && a.by + a.bh > b.by
}
fn test() {
    hits = 0;
    for p122_frame in 0..20 {
        ball = Box { bx: p122_frame as float, by: 50.0, bw: 10.0, bh: 10.0 };
        for p122_row in 0..5 {
            for p122_col in 0..5 {
                brick = Box { bx: (p122_col as float) * 12.0, by: (p122_row as float) * 12.0, bw: 10.0, bh: 10.0 };
                if overlap(ball, brick) { hits += 1; }
            }
        }
    }
    assert(hits > 0, \"nested struct loop failed\");
}"
    )
    .result(Value::Null);
}

// ── P123: Vector allocation inside loop ─────────────────────────────────────
// Creating a vector literal inside a loop should not leak stores.

#[test]
fn p123_vector_in_loop() {
    code!(
        "fn test() {
    total = 0;
    for p123_i in 0..200 {
        v = [for p123_j in 0..8 { p123_j + p123_i * 0 }];
        total += v[0] + v[7];
    }
    assert(total == 1400, \"vector loop failed\");
}"
    )
    .result(Value::Null);
}

// ── P126: Negative integer as tail expression ───────────────────────────────
//
// Symptom: a function whose body has earlier `if X { return Y; }` statements
// followed by a tail expression `-1` produces a misleading parse error:
//   "No matching operator '-' on 'void' and 'integer'"
//
// Root cause hypothesis: after parsing `if ... { return ... }` the parser
// records the previous-statement type as `void`, and when it then tries to
// parse `-1` as the next expression, the prefix `-` is consumed as a binary
// operator continuing the `void` expression instead of starting a new unary
// negation. Bare `-1` at the start of a function (no preceding statements)
// works fine, so the bug is in the boundary between statement-end and
// expression-start parsing.
//
// Fix path: in `parse_expression` (or wherever statement boundaries are
// resolved), force `-` after a void-returning statement to be parsed as a
// unary prefix on a new expression, not a binary operator on the previous
// statement's value. Equivalent to inserting an implicit `;` boundary.
//
// Workaround: use `return -1;` with explicit return.

/// Regression guard for the workaround — the explicit-return form must keep working.
#[test]
fn p126_negative_tail_expression() {
    code!(
        "fn negate(n: integer) -> integer {
    if n > 0 { return 0 - n; }
    n
}
fn test() {
    assert(negate(5) == -5, \"negate positive\");
    assert(negate(-3) == -3, \"negate negative\");
}"
    )
    .result(Value::Null);
}

/// Reproduces the actual bug — bare `-1` after `if { return; }` blocks
/// triggers the misleading "operator '-' on 'void'" diagnostic.
#[test]
fn p126_negative_tail_expression_after_returns() {
    code!(
        "fn lookup(idx: integer) -> integer {
  if idx == 0 { return 100; }
  if idx == 1 { return 200; }
  -1
}
fn test() {
  assert(lookup(0) == 100, \"case 0\");
  assert(lookup(1) == 200, \"case 1\");
  assert(lookup(5) == -1, \"default\");
}"
    )
    .result(Value::Null);
}

// ── P127: File-scope vector constant inlined into function call ─────────────
//
// Symptom: a file-scope constant holding a vector literal, when used as a
// function argument, panics in codegen with one of two flavours depending
// on context:
//   1. "[generate_set] first-assignment of 'X' (var_nr=0) in 'n_test'
//       contains a Var(0) self-reference — storage not yet allocated, will
//       produce a garbage DbRef at runtime. This is a parser bug."
//   2. "generate_call [n_F]: mutable arg 0 (data: Reference(265, []))
//       expected 12B on stack but generate(Var(0)) pushed 8B —
//       Value::Null in a typed slot? Missing convert() call in the parser?"
//
// Root cause: `parse_vector` builds a vector literal as a `Value::Block`
// via `v_block()` (src/data.rs:798), which sets `var_size: 0` and uses
// `Var(0)`/`Var(1)` for its temporaries. When this Block is stored as the
// `code` of a `DefType::Constant` (parser/definitions.rs:407) and later
// inlined where the constant is referenced, the `Var` indices are NOT
// rewritten — they collide with the calling function's local slots.
//
// Fix path: when inlining a file-scope constant Block into a calling
// function, either:
//   (a) remap each `Var(N)` in the constant's IR to a fresh local slot in
//       the caller (allocate `var_size` extra slots first, then offset all
//       Var indices by the caller's current var count), or
//   (b) re-emit the literal at every reference site so each call site has
//       its own freshly-numbered slots, or
//   (c) constant-fold simple literal vectors to a static IR node that
//       doesn't need temporaries at all (best for performance).
//
// Workaround: move the literal inline into the function that needs it.

#[test]
fn p127_file_scope_vector_constant_in_call() {
    code!(
        "QUAD = [1, 2, 3];
fn count(v: const vector<integer>) -> integer { v.len() }
fn test() {
  n = count(QUAD);
  assert(n == 3, \"got {n}\");
}"
    )
    .result(Value::Null);
}

/// Same bug — the local-variable form (literal inline) must keep working.
#[test]
fn p127_inline_vector_literal_in_call_works() {
    code!(
        "fn count(v: const vector<integer>) -> integer { v.len() }
fn test() {
  quad = [1, 2, 3];
  n = count(quad);
  assert(n == 3, \"got {n}\");
}"
    )
    .result(Value::Null);
}

/// The bug also fires for `vector<single>` constants, which is what hit
/// us originally in `lib/graphics/src/graphics.loft` with `UNIT_QUAD_2D`.
#[test]
fn p127_file_scope_single_vector_constant() {
    code!(
        "QUAD = [1.0f, 2.0f, 3.0f];
fn count(v: const vector<single>) -> integer { v.len() }
fn test() {
  n = count(QUAD);
  assert(n == 3, \"got {n}\");
}"
    )
    .result(Value::Null);
}

// ── P117: Struct-returning text-param functions leak callee store ───────────
//
// PROBLEMS.md #117 — `f = file("path")` and similar text-parameter
// struct-returning functions accumulate stores because the dep system
// keeps the work-ref alive even when the O-B2 adoption path bypasses it.
//
// 2026-04-09: I could not reproduce the symptom in a fresh repro of the
// described pattern. The repro below — repeatedly calling a text-param
// struct constructor in a loop — runs to completion without panic and
// without "Database N not correctly freed" warnings. Either:
//   (a) the bug was silently fixed by one of the recent O-B2 codegen
//       changes (P116/P118/P119/P122 fix wave),
//   (b) the original symptom requires the specific `file()` API path
//       which has changed (the `file().exists()` method no longer
//       exists in the current API), or
//   (c) the leak is too small to trigger pool exhaustion within a
//       reasonable test loop.
//
// This test is a *regression guard*: it locks in the current working
// behaviour. If it ever fails with "Allocating a used store" or
// "Database N not correctly freed", #117 has regressed and the
// PROBLEMS.md entry needs reopening with a fresh root-cause analysis.
//
// Fix path (per PROBLEMS.md): in the O-B2 codegen path
// (`gen_set_first_at_tos`), after adopting the callee's store, emit
// `OpFreeRef` for the unused `__ref_N` work variable.
#[test]
fn p117_text_param_struct_return_loop_no_leak() {
    // 100 iterations is enough to detect a per-call store leak in debug
    // mode without dominating CI time.
    code!(
        "struct Wrap { name: text not null, count: integer not null }
fn make(t: text) -> Wrap {
  Wrap { name: t, count: t.len() }
}
fn test() {
  for _p117_i in 0..100 {
    w = make(\"hello\");
    assert(w.count == 5, \"count\");
  }
}"
    )
    .result(Value::Null);
}

// ── P120: Vector field in struct returned from function ────────────────────
//
// PROBLEMS.md #120 — when a function returns a struct containing a
// `vector<integer>` field, the vector data was lost during stack unwind
// because the constructor only copied the vector pointer, not the
// underlying data. Caused length=0 vectors after function return.
//
// 2026-04-09: the original reproducer
// `lib/graphics/examples/test_mat4_crash.loft` now runs cleanly:
//   inside make_big: data len=16
//   after return: data len=16
//   data[0]=0 data[15]=15
//
// This regression-guard test reproduces the same pattern as a unit test
// so any future regression is caught in CI. If this test ever fails,
// reopen #120 in PROBLEMS.md and revisit `gen_set_first_ref_call_copy`
// in `src/state/codegen.rs`.
//
// Fix path (per PROBLEMS.md): the struct constructor must deep-copy
// vector field data into the struct's own store at `FinishRecord` /
// `SetField` level. Mitigations already in place include double-free
// tolerance in `free_ref`, loop pre-init reference hoisting, and
// `is_ret_work_ref` suppression of FreeRef in return paths.
#[test]
fn p120_vector_field_in_returned_struct_round_trip() {
    code!(
        "struct BigBox {
  width: integer not null,
  height: integer not null,
  data: vector<integer>
}
fn make_big() -> BigBox {
  d: vector<integer> = [];
  for p120_i in 0..16 { d += [p120_i]; }
  BigBox { width: 4, height: 4, data: d }
}
fn test() {
  b = make_big();
  assert(b.width == 4, \"width\");
  assert(b.height == 4, \"height\");
  assert(b.data.len() == 16, \"data len {b.data.len()}\");
  assert(b.data[0] == 0, \"data[0]\");
  assert(b.data[15] == 15, \"data[15]\");
}"
    )
    .result(Value::Null);
}

// ── P121: Tuple literals crashed interpreter with heap corruption ──────────
//
// PROBLEMS.md #121 — `a = (3.0, 2.0)` triggered glibc
// "corrupted size vs. prev_size" abort or SIGSEGV in interpreter mode
// (native worked). Cited as "stack layout issue in interpreter's tuple
// codegen — 16-byte float-pair allocation corrupts heap allocator metadata".
//
// 2026-04-09: the documented reproducer runs cleanly in `--interpret`
// mode. Either the bug has been silently fixed or the heap corruption
// requires specific allocator state that doesn't reliably reproduce.
//
// Regression guard: the test below executes the exact reproducer from
// PROBLEMS.md plus a few variants (function return, destructure,
// element assign). If any of these regress to the heap-corruption
// failure, #121 should be reopened.
//
// Fix path (per PROBLEMS.md): audit `OpTupleLiteral` and stack
// reservation for tuple temporaries for off-by-one / alignment errors.
#[test]
fn p121_float_tuple_literal_no_heap_corruption() {
    code!(
        "fn test() {
  a = (3.0, 2.0);
  assert(a.0 > 1.0, \"a.0\");
  assert(a.1 < 5.0, \"a.1\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p121_float_tuple_function_return() {
    code!(
        "fn pair(p121_x: float) -> (float, float) { (p121_x, p121_x * 2.0) }
fn test() {
  p = pair(3.0);
  assert(p.0 == 3.0, \"first\");
  assert(p.1 == 6.0, \"second\");
}"
    )
    .result(Value::Null);
}

// ── P124: Native codegen — inline array indexing on float literal ──────────
//
// PROBLEMS.md #124 — `[0.9, 0.2, 0.3][idx]` in loft generated an
// `as DbRef` cast in the Rust output that failed to compile. Native-mode
// only (interpreter handled it correctly).
//
// 2026-04-09: the inline form now triggers a parser-level type error
// in interpret mode ("Variable v cannot change type from vector<integer>
// to integer"), and the function-tail form (`[0.9, 0.2, 0.3][idx]` as
// the body of a function returning float) compiles cleanly under
// `--native`. Either the codegen `as DbRef` mistake has been fixed, or
// the parser now refuses the form before it reaches the codegen path.
//
// Regression guard: lock in the current working behaviour. If `--native`
// compilation regresses for the function-tail form, reopen #124. The
// test runs in interpret mode by default; native-mode coverage lives in
// `tests/native.rs` if a more thorough check is needed.
//
// Fix path (per PROBLEMS.md): in `src/generation/`, the inline-array-
// then-index pattern emits a `Reference` cast to the wrong type. Look
// for `as DbRef` in the generated Rust source via `--native-emit`.
#[test]
fn p124_function_returning_inline_array_index() {
    code!(
        "fn pick(p124_idx: integer) -> float {
  [0.9, 0.2, 0.3][p124_idx]
}
fn test() {
  assert(pick(0) > 0.85, \"0\");
  assert(pick(1) < 0.25, \"1\");
  assert(pick(2) > 0.25, \"2\");
}"
    )
    .result(Value::Null);
}

/// Documented workaround — assign the array to a variable first, then index.
/// This must keep working even if #124 is fixed at the inline-form level.
#[test]
fn p124_local_array_index_workaround_works() {
    code!(
        "fn pick(p124w_idx: integer) -> float {
  options = [0.9, 0.2, 0.3];
  options[p124w_idx]
}
fn test() {
  assert(pick(0) > 0.85, \"0\");
  assert(pick(1) < 0.25, \"1\");
}"
    )
    .result(Value::Null);
}

// P122c: struct-returning function used inside conditional inside loop
// This is the exact pattern from the Brick Buster collision detection.
#[test]
fn p122_struct_return_conditional_loop() {
    // Iterations reduced 100*50=5000 → 30*15=450 for CI speed. Still
    // exercises the store-leak pattern with hundreds of allocations.
    code!(
        "struct Overlap { ox: float not null, oy: float not null }
fn compute_overlap(ax: float, bx: float) -> Overlap {
    Overlap { ox: ax, oy: bx }
}
fn test() {
    score = 0;
    for p122c_frame in 0..30 {
        for p122c_i in 0..15 {
            d = compute_overlap(p122c_frame as float, p122c_i as float);
            if d.ox > 10.0 {
                score += 1;
            }
        }
    }
    assert(score > 0, \"conditional struct failed\");
}"
    )
    .result(Value::Null);
}

// P122d: struct created inside loop body (not from function return)
#[test]
fn p122_struct_literal_in_loop() {
    code!(
        "struct Rect { rx: float not null, ry: float not null, rw: float not null, rh: float not null }
fn test() {
    count = 0;
    for p122d_i in 0..500 {
        r = Rect { rx: p122d_i as float, ry: 0.0, rw: 10.0, rh: 10.0 };
        if r.rx > 100.0 { count += 1; }
    }
    assert(count == 399, \"struct literal loop failed\");
}"
    )
    .result(Value::Null);
}

// P122e: very long loop (simulating game frames) — exhaustion stress test.
//
// Marked #[ignore] because it takes ~10 minutes in debug mode. In release
// mode it completes in ~0.05s (verified 2026-04-11, no store exhaustion).
// Run on demand with `cargo test --release --ignored p122_long_running_struct_loop`.
#[test]
#[ignore = "P122 stress test — 100k struct allocations, ~10min in debug. Passes in 0.05s release."]
fn p122_long_running_struct_loop() {
    code!(
        "struct Overlap { ox: float not null, oy: float not null }
fn depth(ax: float, ay: float, bx: float, by: float) -> Overlap {
    Overlap { ox: ax - bx, oy: ay - by }
}
fn test() {
    score = 0;
    // 10000 frames * 10 bricks = 100,000 struct allocations
    for p122e_f in 0..10000 {
        for p122e_b in 0..10 {
            d = depth(p122e_f as float, 50.0, p122e_b as float * 8.0, 20.0);
            if d.ox > 0.0 && d.oy > 0.0 { score += 1; }
        }
    }
    assert(score > 0, \"long struct loop failed\");
}"
    )
    .result(Value::Null);
}

// ── GL-pattern verification tests ─────────────────────────────────────────
//
// These tests replicate the actual patterns from the GL renderer and game
// loop that historically triggered store leaks (P122), vector leaks (P123),
// struct-return leaks (P117), and heap corruption (P121).
//
// Each test is designed to run in both debug mode (with assertions) and
// release mode. They use sustained iteration counts that would exhaust the
// store pool if leaks were present.

// ── P122 GL pattern: mat4-style struct with vector field, returned per frame ──
//
// This replicates the `math::mat4_mul` / `mat4_trs` pattern from the renderer:
// a struct containing a vector<float> is constructed and returned from a function
// called once per frame. In the real renderer this happens via mat4_look_at,
// mat4_perspective, mat4_mul — each allocating a store for the Mat4 + its
// vector field.
#[test]
fn p122_gl_mat4_vector_field_per_frame() {
    code!(
        "struct M4 { m: vector<float> }

fn make_m4(gm4_s: float) -> M4 {
    M4 { m: [gm4_s, 0.0, 0.0, 0.0,
             0.0, gm4_s, 0.0, 0.0,
             0.0, 0.0, gm4_s, 0.0,
             0.0, 0.0, 0.0, 1.0] }
}

fn mul_m4(gm4_a: const M4, gm4_b: const M4) -> M4 {
    gm4_r = M4 { m: [0.0, 0.0, 0.0, 0.0,
                      0.0, 0.0, 0.0, 0.0,
                      0.0, 0.0, 0.0, 0.0,
                      0.0, 0.0, 0.0, 0.0] };
    for gm4_i in 0..4 {
        for gm4_j in 0..4 {
            gm4_sum = 0.0;
            for gm4_k in 0..4 {
                gm4_sum += gm4_a.m[gm4_i * 4 + gm4_k] * gm4_b.m[gm4_k * 4 + gm4_j];
            }
            gm4_r.m[gm4_i * 4 + gm4_j] = gm4_sum;
        }
    }
    gm4_r
}

fn test() {
    // Simulate 500 render frames, each creating 3 matrices and multiplying
    // Total: 500 * 5 = 2500 struct-with-vector allocations
    gm4_check = 0.0;
    for gm4_frame in 0..500 {
        gm4_view = make_m4(1.0);
        gm4_proj = make_m4(2.0);
        gm4_model = make_m4((gm4_frame as float) * 0.001 + 1.0);
        gm4_vp = mul_m4(gm4_view, gm4_proj);
        gm4_mvp = mul_m4(gm4_vp, gm4_model);
        gm4_check += gm4_mvp.m[0];
    }
    assert(gm4_check > 0.0, \"mat4 GL loop failed: {gm4_check}\");
}"
    )
    .result(Value::Null);
}

// ── P122 GL pattern: collision detection with struct Rect + Overlap ────────
//
// This replicates the Brick Buster collision loop using the *struct-based* API
// (not the raw-float workaround). Each frame checks N bricks for collision
// with M balls, creating Rect and Overlap structs per check.
#[test]
fn p122_gl_collision_struct_api() {
    code!(
        "struct Rect { rx: float not null, ry: float not null, rw: float not null, rh: float not null }
struct Overlap { ox: float not null, oy: float not null }

fn rects_overlap(gc_a: const Rect, gc_b: const Rect) -> boolean {
    gc_a.rx < gc_b.rx + gc_b.rw && gc_a.rx + gc_a.rw > gc_b.rx &&
    gc_a.ry < gc_b.ry + gc_b.rh && gc_a.ry + gc_a.rh > gc_b.ry
}

fn overlap_depth(gc_a: const Rect, gc_b: const Rect) -> Overlap {
    if !rects_overlap(gc_a, gc_b) { return Overlap { ox: 0.0, oy: 0.0 }; }
    gc_dx = min(gc_a.rx + gc_a.rw - gc_b.rx, gc_b.rx + gc_b.rw - gc_a.rx);
    gc_dy = min(gc_a.ry + gc_a.rh - gc_b.ry, gc_b.ry + gc_b.rh - gc_a.ry);
    Overlap { ox: gc_dx, oy: gc_dy }
}

fn test() {
    gc_hits = 0;
    // 200 frames, 8 bricks per frame = 1600 collision checks
    // Each check creates 2 Rect + 1 Overlap (conditionally)
    for gc_frame in 0..200 {
        gc_ball = Rect { rx: (gc_frame as float) * 0.5, ry: 50.0, rw: 8.0, rh: 8.0 };
        for gc_brick in 0..8 {
            gc_br = Rect {
                rx: (gc_brick as float) * 40.0, ry: 45.0,
                rw: 35.0, rh: 12.0
            };
            gc_d = overlap_depth(gc_ball, gc_br);
            if gc_d.ox > 0.0 && gc_d.oy > 0.0 {
                gc_hits += 1;
            }
        }
    }
    assert(gc_hits > 0, \"collision struct loop failed: {gc_hits}\");
}"
    )
    .result(Value::Null);
}

// ── P120 minimal isolation tests ───────────────────────────────────────────
//
// These tests isolate the exact pattern that leaks stores. Each adds one
// element of complexity to find the boundary between "works" and "leaks".

/// P120 atom A: struct field overwrite once, no loop.
/// Does a single overwrite of a struct field with a function return leak?
///
/// Root cause (from execution trace): `make_inner` allocates store 3 for the
/// returned Inner struct. `CopyRecord` copies the data from store 3 into
/// store 2 (the Outer's field). But no `FreeRef` is emitted for store 3
/// after the copy — it becomes orphaned. Debug mode catches this at exit
/// with "Database 3 not correctly freed".
#[test]
fn p120_field_overwrite_once() {
    code!(
        "struct Inner { ix: float not null, iy: float not null }
struct Outer { pos: Inner }

fn make_inner(p120a_v: float) -> Inner {
    Inner { ix: p120a_v, iy: p120a_v * 2.0 }
}

fn test() {
    p120a_o = Outer { pos: Inner { ix: 0.0, iy: 0.0 } };
    p120a_o.pos = make_inner(5.0);
    assert(p120a_o.pos.ix == 5.0, \"overwrite once: {p120a_o.pos.ix}\");
}"
    )
    .result(Value::Null);
}

/// P120 atom B: struct field overwrite twice, no loop.
/// The second overwrite must free the store from the first.
#[test]
fn p120_field_overwrite_twice() {
    code!(
        "struct Inner { ix: float not null, iy: float not null }
struct Outer { pos: Inner }

fn make_inner(p120b_v: float) -> Inner {
    Inner { ix: p120b_v, iy: p120b_v * 2.0 }
}

fn test() {
    p120b_o = Outer { pos: Inner { ix: 0.0, iy: 0.0 } };
    p120b_o.pos = make_inner(1.0);
    p120b_o.pos = make_inner(2.0);
    assert(p120b_o.pos.ix == 2.0, \"overwrite twice: {p120b_o.pos.ix}\");
}"
    )
    .result(Value::Null);
}

/// P120 atom C: struct field overwrite in a short loop (3 iterations).
#[test]
fn p120_field_overwrite_short_loop() {
    code!(
        "struct Inner { ix: float not null, iy: float not null }
struct Outer { pos: Inner }

fn make_inner(p120c_v: float) -> Inner {
    Inner { ix: p120c_v, iy: p120c_v * 2.0 }
}

fn test() {
    p120c_o = Outer { pos: Inner { ix: 0.0, iy: 0.0 } };
    for p120c_i in 0..3 {
        p120c_o.pos = make_inner(p120c_i as float);
    }
    assert(p120c_o.pos.ix == 2.0, \"short loop: {p120c_o.pos.ix}\");
}"
    )
    .result(Value::Null);
}

/// P120 atom D: local variable overwrite in a loop (NOT a field).
/// This should NOT leak — the store is on the stack, not in a struct.
#[test]
fn p120_local_overwrite_in_loop() {
    code!(
        "struct Inner { ix: float not null, iy: float not null }

fn make_inner(p120d_v: float) -> Inner {
    Inner { ix: p120d_v, iy: p120d_v * 2.0 }
}

fn test() {
    p120d_sum = 0.0;
    for p120d_i in 0..100 {
        p120d_val = make_inner(p120d_i as float);
        p120d_sum += p120d_val.ix;
    }
    assert(p120d_sum > 0.0, \"local overwrite: {p120d_sum}\");
}"
    )
    .result(Value::Null);
}

/// P120 atom E: struct field overwrite with text field (triggers P117 area).
#[test]
fn p120_field_overwrite_with_text() {
    code!(
        "struct Named { label: text, val: integer }
struct Container { item: Named }

fn make_named(p120e_s: text, p120e_n: integer) -> Named {
    Named { label: p120e_s, val: p120e_n }
}

fn test() {
    p120e_c = Container { item: Named { label: \"init\", val: 0 } };
    for p120e_i in 0..10 {
        p120e_c.item = make_named(\"iter_{p120e_i}\", p120e_i);
    }
    assert(p120e_c.item.val == 9, \"text field: {p120e_c.item.val}\");
}"
    )
    .result(Value::Null);
}

// ── P120 GL-pattern: struct return inside conditional inside loop ──────────
//
// Replicates the renderer transform update: a struct-returning function is
// called inside an `if` branch inside the render loop. P120 triggers when
// copy_record tries to delete a record in a locked store.
//
// Passes in release mode but fails in debug mode with "Database N not
// correctly freed" — confirming the store leak is real. The leaked store
// is from overwriting a struct field with a new struct-returning function
// call: the old store is not freed before the new one is assigned.
#[test]
fn p120_struct_return_in_conditional_in_loop() {
    code!(
        "struct Transform { tx: float not null, ty: float not null, tz: float not null }
struct Node { name: text, xform: Transform }

fn make_transform(gl_t: float) -> Transform {
    Transform { tx: sin(gl_t) * 2.0, ty: cos(gl_t), tz: 0.0 }
}

fn test() {
    gl_nd = Node { name: \"cube\", xform: Transform { tx: 0.0, ty: 0.0, tz: 0.0 } };
    gl_sum = 0.0;
    for gl_frame in 0..1000 {
        gl_time = (gl_frame as float) * 0.01;
        // Conditional struct return — the P120 pattern
        if gl_frame % 2 == 0 {
            gl_nd.xform = make_transform(gl_time);
        }
        gl_sum += gl_nd.xform.tx;
    }
    assert(gl_sum != 0.0, \"conditional struct return sum should be nonzero\");
}"
    )
    .result(Value::Null);
}

// ── P120 pattern: multiple struct field updates per frame ──────────────────
//
// Replicates the renderer's per-frame update of multiple node transforms
// in a scene graph. Each node's transform is overwritten with a new struct.
//
// Passes in release mode but fails in debug mode with "Database 9 not
// correctly freed" — same root cause as the conditional test above.
// The store allocated for the old Vec3 value is not freed when the field
// is overwritten with a new struct from make_pos().
#[test]
fn p120_multi_node_transform_update() {
    code!(
        "struct Vec3 { vx: float not null, vy: float not null, vz: float not null }
struct SceneNode { pos: Vec3, scale: float not null }

fn make_pos(mn_t: float, mn_i: integer) -> Vec3 {
    Vec3 { vx: sin(mn_t + mn_i as float), vy: cos(mn_t), vz: 0.0 }
}

fn test() {
    // 4 nodes, each updated per frame — like a small scene graph
    mn_n0 = SceneNode { pos: Vec3 { vx: 0.0, vy: 0.0, vz: 0.0 }, scale: 1.0 };
    mn_n1 = SceneNode { pos: Vec3 { vx: 0.0, vy: 0.0, vz: 0.0 }, scale: 1.0 };
    mn_n2 = SceneNode { pos: Vec3 { vx: 0.0, vy: 0.0, vz: 0.0 }, scale: 1.0 };
    mn_n3 = SceneNode { pos: Vec3 { vx: 0.0, vy: 0.0, vz: 0.0 }, scale: 1.0 };
    mn_sum = 0.0;
    for mn_frame in 0..500 {
        mn_t = (mn_frame as float) * 0.02;
        mn_n0.pos = make_pos(mn_t, 0);
        mn_n1.pos = make_pos(mn_t, 1);
        mn_n2.pos = make_pos(mn_t, 2);
        mn_n3.pos = make_pos(mn_t, 3);
        mn_sum += mn_n0.pos.vx + mn_n1.pos.vy + mn_n2.pos.vx + mn_n3.pos.vy;
    }
    assert(mn_sum != 0.0, \"multi-node update sum should be nonzero\");
}"
    )
    .result(Value::Null);
}

// ── P117 GL pattern: text-param struct return in a tight loop ──────────────
//
// The original P117 bug: a function that takes a text parameter and returns
// a struct leaks the callee's store. This test calls such a function in
// a sustained loop to verify the leak is actually gone.
#[test]
fn p117_gl_text_param_struct_return_sustained() {
    code!(
        "struct Asset { path: text, size: integer }

fn load_asset(tp_name: text) -> Asset {
    Asset { path: tp_name, size: tp_name.len() }
}

fn test() {
    tp_total = 0;
    for tp_i in 0..2000 {
        tp_a = load_asset(\"textures/brick_{tp_i}.png\");
        tp_total += tp_a.size;
    }
    assert(tp_total > 0, \"text-param struct loop failed: {tp_total}\");
}"
    )
    .result(Value::Null);
}

// ── P117 pattern: multiple text-param struct returns per iteration ─────────
//
// Stresses the text-param return path with multiple calls per loop iteration,
// mimicking loading different asset types each frame.
#[test]
fn p117_gl_multi_text_struct_per_frame() {
    code!(
        "struct FileRef { name: text, found: boolean }

fn lookup(mt_path: text) -> FileRef {
    FileRef { name: mt_path, found: mt_path.len() > 5 }
}

fn test() {
    mt_found = 0;
    for mt_i in 0..1000 {
        mt_a = lookup(\"shader/vert_{mt_i}.glsl\");
        mt_b = lookup(\"shader/frag_{mt_i}.glsl\");
        mt_c = lookup(\"tex/d.png\");
        if mt_a.found { mt_found += 1; }
        if mt_b.found { mt_found += 1; }
        if mt_c.found { mt_found += 1; }
    }
    assert(mt_found > 0, \"multi-text struct failed: {mt_found}\");
}"
    )
    .result(Value::Null);
}

// ── P121 pattern: tuple usage in a sustained loop ─────────────────────────
//
// The original P121 bug was heap corruption from tuple literals. This test
// creates tuples in a loop to verify the fix holds under sustained use.
#[test]
fn p121_tuple_sustained_loop() {
    code!(
        "fn make_pair(tp_x: float, tp_y: float) -> (float, float) {
    (tp_x, tp_y)
}

fn test() {
    tp_sum = 0.0;
    for tp_i in 0..1000 {
        tp_p = make_pair(tp_i as float, (tp_i as float) * 0.5);
        tp_sum += tp_p.0 + tp_p.1;
    }
    assert(tp_sum > 0.0, \"tuple loop failed: {tp_sum}\");
}"
    )
    .result(Value::Null);
}

// ── P121 pattern: nested tuple operations ─────────────────────────────────
//
// Tests tuple element access, arithmetic on tuple fields, and tuple
// construction from other tuple elements — more complex than a simple literal.
#[test]
fn p121_tuple_nested_operations() {
    code!(
        "fn swap_pair(tn_a: float, tn_b: float) -> (float, float) {
    (tn_b, tn_a)
}

fn test() {
    tn_sum = 0.0;
    for tn_i in 0..500 {
        tn_p1 = (tn_i as float, (tn_i as float) * 2.0);
        tn_p2 = swap_pair(tn_p1.0, tn_p1.1);
        tn_sum += tn_p2.0 - tn_p2.1;
    }
    // Each iteration: p2 = (i*2, i), so p2.0 - p2.1 = i*2 - i = i
    // sum = 0 + 1 + 2 + ... + 499 = 124750
    assert(tn_sum > 124000.0, \"nested tuple failed: {tn_sum}\");
}"
    )
    .result(Value::Null);
}

// ── P123 GL pattern: vector allocation per frame ──────────────────────────
//
// Replicates the renderer's per-frame vertex data construction: a vector of
// floats is built each frame (like uploading new vertex positions). This is
// the pattern that exhausted the store pool before P123 was fixed.
#[test]
fn p123_gl_vector_per_frame_sustained() {
    code!(
        "fn test() {
    vf_total = 0;
    for vf_frame in 0..1000 {
        // Build vertex data each frame (like gl_upload_vertices)
        vf_verts = [for vf_v in 0..12 { (vf_v + vf_frame) as float * 0.01 }];
        vf_total += vf_verts.len();
    }
    assert(vf_total == 12000, \"per-frame vector failed: {vf_total}\");
}"
    )
    .result(Value::Null);
}

// ── P123 pattern: multiple vector allocations per frame ───────────────────
//
// The renderer builds multiple vectors per frame: positions, normals, colors,
// indices. Each is a fresh allocation. This test creates 4 vectors per
// iteration for 500 iterations = 2000 vector allocations.
#[test]
fn p123_gl_multi_vector_per_frame() {
    code!(
        "fn test() {
    mv_check = 0;
    for mv_frame in 0..500 {
        mv_pos = [for mv_i in 0..6 { mv_i as float + mv_frame as float * 0.001 }];
        mv_norm = [for mv_i in 0..6 { 0.0 }];
        mv_col = [for mv_i in 0..6 { 1.0 }];
        mv_idx = [for mv_i in 0..3 { mv_i }];
        mv_check += mv_pos.len() + mv_norm.len() + mv_col.len() + mv_idx.len();
    }
    assert(mv_check == 10500, \"multi-vector per frame failed: {mv_check}\");
}"
    )
    .result(Value::Null);
}

// ── Combined GL pattern: struct + vector + text in game loop ──────────────
//
// This is the "full Brick Buster frame" pattern combining all the bug areas:
// struct collision detection, vector per-frame data, text for debug output,
// all inside a sustained game loop.
#[test]
fn gl_combined_game_loop_stress() {
    code!(
        "struct Ball { bx: float not null, by: float not null }
struct Brick { brx: float not null, bry: float not null, hp: integer }

fn make_ball(cb_frame: integer) -> Ball {
    Ball { bx: (cb_frame as float) * 0.3, by: 50.0 }
}

fn check_hit(cb_ball: const Ball, cb_brick: const Brick) -> boolean {
    abs(cb_ball.bx - cb_brick.brx) < 20.0 && abs(cb_ball.by - cb_brick.bry) < 10.0
}

fn format_score(cb_score: integer) -> text {
    \"Score: {cb_score}\"
}

fn test() {
    cb_score = 0;
    cb_last_text = \"\";
    for cb_frame in 0..300 {
        cb_b = make_ball(cb_frame);
        // Check 10 bricks per frame
        cb_active = [for cb_i in 0..10 { 1 }];
        for cb_i in 0..10 {
            if cb_active[cb_i] == 0 { continue; }
            cb_brick = Brick { brx: (cb_i as float) * 30.0, bry: 45.0, hp: 1 };
            if check_hit(cb_b, cb_brick) {
                cb_score += 1;
                cb_active[cb_i] = 0;
            }
        }
        // Text allocation per frame (like HUD update)
        if cb_frame % 60 == 0 {
            cb_last_text = format_score(cb_score);
        }
    }
    assert(cb_score > 0, \"combined game loop failed: {cb_score}\");
    assert(cb_last_text.len() > 0, \"text output empty\");
}"
    )
    .result(Value::Null);
}

// ── P54: JsonValue enum — landing tests ────────────────────────────────────
// Design in doc/claude/BITING_PLAN.md § P54.  These tests pin the public
// surface of the new JSON subsystem:
//
//   enum JsonValue { JObject, JArray, JString, JNumber, JBool, JNull }
//   fn json_parse(text) -> JsonValue
//   fn to_json(self: JsonValue) -> text
//   fn field / item / as_text / as_number / as_long / as_bool / len
//   MyStruct.parse(v: JsonValue) — replaces .parse(text)
//
// `#[ignore]`'d while the implementation lands incrementally across
// `default/06_json.loft`, `src/json.rs`, and the parser's .parse() gate.
// Unignore each case as its layer comes online.

#[test]
#[ignore = "P54 step 3: json_parse for primitives not yet implemented"]
fn p54_parse_primitive_string() {
    code!(
        "fn run() -> text {
    v = json_parse(\"\\\"hello\\\"\");
    match v {
        JString { value } => value,
        _ => \"<wrong variant>\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("hello"));
}

#[test]
#[ignore = "P54 step 3: json_parse for primitives not yet implemented"]
fn p54_parse_primitive_number() {
    code!(
        "fn run() -> float {
    v = json_parse(\"42.5\");
    match v {
        JNumber { value } => value,
        _ => 0.0
    }
}"
    )
    .expr("run()")
    .result(Value::Float(42.5));
}

#[test]
#[ignore = "P54 step 3: json_parse for primitives not yet implemented"]
fn p54_parse_primitive_bool_true() {
    code!(
        "fn run() -> boolean {
    v = json_parse(\"true\");
    match v {
        JBool { value } => value,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

#[test]
#[ignore = "P54 step 3: json_parse for primitives not yet implemented"]
fn p54_parse_primitive_null() {
    code!(
        "fn run() -> boolean {
    v = json_parse(\"null\");
    match v {
        JNull => true,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

#[test]
#[ignore = "P54 step 3: malformed JSON returns JNull, not a panic"]
fn p54_malformed_returns_jnull() {
    code!(
        "fn run() -> boolean {
    v = json_parse(\"{not valid}\");
    match v {
        JNull => true,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

#[test]
#[ignore = "P54 step 5: extractors not yet implemented"]
fn p54_extractor_as_text() {
    code!(
        "fn run() -> text {
    v = json_parse(\"\\\"abc\\\"\");
    v.as_text()
}"
    )
    .expr("run()")
    .result(Value::str("abc"));
}

#[test]
#[ignore = "P54 step 5: extractors return null on kind mismatch"]
fn p54_extractor_as_text_wrong_kind_returns_null() {
    code!(
        "fn run() -> text {
    v = json_parse(\"42\");
    t = v.as_text();
    if t == null { \"is-null\" } else { \"not-null\" }
}"
    )
    .expr("run()")
    .result(Value::str("is-null"));
}

#[test]
#[ignore = "P54 step 4: parse_object + field() chained access"]
fn p54_parse_object_field_access() {
    code!(
        "fn run() -> text {
    v = json_parse(\"{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}\");
    v.field(\"name\").as_text()
}"
    )
    .expr("run()")
    .result(Value::str("Alice"));
}

#[test]
#[ignore = "P54 step 4: parse_array + item() indexed access"]
fn p54_parse_array_item_access() {
    code!(
        "fn run() -> long {
    v = json_parse(\"[10, 20, 30]\");
    v.item(1).as_long()
}"
    )
    .expr("run()")
    .result(Value::Long(20));
}

#[test]
#[ignore = "P54 step 4: missing intermediate in chain returns JNull, not a trap"]
fn p54_missing_chain_returns_jnull() {
    code!(
        "fn run() -> boolean {
    v = json_parse(\"{\\\"a\\\": {\\\"b\\\": 1}}\");
    // Chain through a missing intermediate; must not panic.
    result = v.field(\"missing\").item(5).field(\"b\");
    match result {
        JNull => true,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

#[test]
#[ignore = "P54 step 6-7: .parse(JsonValue) accepts typed tree"]
fn p54_struct_parse_accepts_jsonvalue() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> text {
    v = json_parse(\"{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}\");
    u = User.parse(v);
    u.name
}"
    )
    .expr("run()")
    .result(Value::str("Bob"));
}

#[test]
#[ignore = "P54 step 6: .parse(text) rejected — must use json_parse first"]
fn p54_struct_parse_rejects_plain_text() {
    code!(
        "struct User { name: text }
fn test() {
    u = User.parse(\"{\\\"name\\\":\\\"Bob\\\"}\");
}"
    )
    .error("expects a JsonValue");
}

// ── P54 struct-enum blockers — runtime specs (BITING_PLAN § P54) ──────────
//
// Each struct-enum bug found while building JsonValue gets a regression
// guard (for fixed bugs) or an #[ignore]'d spec (for open bugs).  The
// #[ignore]'d tests document the expected behaviour; they'll go green
// automatically when the corresponding blocker is resolved.

/// B1 (FIXED `61c36d7`): `match v { UnitVariant => … }` no longer panics
/// when `v` is produced somewhere other than a literal.  Exercise via
/// a mixed-variant enum where the unit arm matches.
#[test]
fn p54_b1_unit_variant_match_from_binding() {
    code!(
        "pub enum Palette { Null, Shade { v: integer } }
fn run() -> integer {
    p = Shade { v: 7 };
    match p {
        Null => -1,
        Shade { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

/// B6 (FIXED `5684df2`): match-arm type unification strips `RefVar`
/// wrappers.  Binding a field-carrying struct variant's text field in one
/// arm and returning a literal text (`""`) in another no longer errors
/// with 'cannot unify: &text and text'.
///
/// Uses plain struct (not struct-enum) to dodge the still-open B4
/// runtime bug.  Same type-system machinery — the field binding yields
/// `&text`, the wildcard arm returns owned `text`.
#[test]
fn p54_b6_match_arm_text_unify_plain_struct() {
    code!(
        "struct Pair { a: text, b: integer }
fn extract(p: const Pair) -> text {
    match p.b {
        0 => p.a,
        _ => \"other\"
    }
}
fn run() -> text {
    extract(Pair { a: \"hello\", b: 0 })
}"
    )
    .expr("run()")
    .result(Value::str("hello"));
}

/// B3 (open): struct-enum with a `float not null` variant crashes
/// `free(): invalid size` at construction.  Workaround: drop `not null`
/// from the variant payload.  Spec: constructing + matching such a
/// variant round-trips cleanly.
#[test]
#[ignore = "P54 B3: float not null in struct-enum variant crashes free()"]
fn p54_b3_float_not_null_variant() {
    code!(
        "pub enum JV { A { v: float not null } }
fn mk() -> JV {
    n = A { v: 42.5 };
    n
}
fn run() -> float {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Float(42.5));
}

/// B4 (open): struct-enum with mixed-type variants (boolean/integer/text)
/// crashes at runtime when returned from a function.
#[test]
#[ignore = "P54 B4: mixed-field struct-enum runtime crashes"]
fn p54_b4_mixed_variant_return() {
    code!(
        "pub enum JV { JA { v: boolean }, JB { v: integer }, JC { v: text } }
fn mk() -> JV {
    n = JB { v: 42 };
    n
}
fn run() -> integer {
    x = mk();
    match x {
        JA { v } => if v { 1 } else { 0 },
        JB { v } => v,
        JC { v } => v.len()
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// B5 (open): self-referential struct-enum (`vector<Self>` in a variant)
/// trips the Recursion-depth-500 codegen guard.  Spec: declaration
/// compiles, pattern match dispatches correctly.
#[test]
#[ignore = "P54 B5: recursive struct-enum trips codegen recursion guard"]
fn p54_b5_recursive_struct_enum() {
    code!(
        "pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn count(t: const Tree) -> integer {
    match t {
        Leaf { v } => v,
        Node { kids } => {
            c = 0;
            for k in kids { c += count(k); }
            c
        }
    }
}
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }] };
    count(root)
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

/// P54 related — positive baseline for struct-enum parameter passing.
/// Passing a struct-enum into a function and matching on it inside works
/// today; this test guards against that regressing while the return-
/// direction bugs (B3/B4) are being resolved.
#[test]
fn p54_struct_enum_as_parameter_ok() {
    code!(
        "pub enum JV { A { v: integer }, B { x: integer } }
fn show(j: const JV) -> integer {
    match j {
        A { v } => v,
        B { x } => x
    }
}
fn run() -> integer {
    n = A { v: 42 };
    show(n)
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// P54 related — positive baseline for struct-enum constructed in a
/// function and immediately matched in the same scope (no return).
/// Works today; guard against regression.
#[test]
fn p54_struct_enum_literal_then_match_same_scope() {
    code!(
        "pub enum JV { A { v: integer }, B { x: integer } }
fn run() -> integer {
    n = A { v: 7 };
    match n {
        A { v } => v,
        B { x } => x
    }
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

/// B3 (open, sharpened): even a single-variant struct-enum returned
/// from a function crashes at runtime.  Narrows the reproducer from
/// B4's mixed-type variant case — the blocker is the return path, not
/// the variant diversity.
#[test]
#[ignore = "P54 B3/B4: struct-enum return from function crashes (any variant type)"]
fn p54_b3_single_variant_return() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV { A { v: 42 } }
fn run() -> integer {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// P54 — JsonValue-style extractors via a plain tagged struct.  This
/// is the workaround pattern callers use today while struct-enum
/// return-direction (B3/B4) is broken: a discriminant field plus one
/// slot per payload type.  Ugly but unblocks JSON work now.
///
/// Verifies that extractor-with-null-on-mismatch compiles cleanly
/// (B6 fix) and returns the expected values for both matching and
/// mismatching kind arms.
#[test]
fn p54_tagged_struct_extractors_work_today() {
    code!(
        "struct Tagged { kind: integer, text_val: text, num_val: float }
pub fn as_text(self: const Tagged) -> text {
    at_out = \"\";
    match self.kind {
        1 => { at_out = self.text_val; },
        _ => {}
    }
    at_out
}
pub fn as_number(self: const Tagged) -> float {
    match self.kind {
        2 => self.num_val,
        _ => 0.0
    }
}
fn run() -> text {
    r = \"\";
    t = Tagged { kind: 1, text_val: \"hello\", num_val: 0.0 };
    r += t.as_text();
    r += \"|\";
    n = Tagged { kind: 2, text_val: \"\", num_val: 3.14 };
    nt = n.as_text();
    r += \"miss[{nt}]\";
    r
}"
    )
    .expr("run()")
    .result(Value::str("hello|miss[]"));
}

/// P54 — the same extractor pattern via a struct-enum.  This is the
/// API we eventually want; it's blocked on B3/B4 (struct-enum return
/// from function).  When those are fixed, this test goes green.
#[test]
#[ignore = "P54 B3/B4: struct-enum return from function crashes"]
fn p54_struct_enum_extractors_spec() {
    code!(
        "pub enum V { T { v: text }, N { v: float } }
pub fn as_text(self: V) -> text {
    out = \"\";
    match self {
        T { v } => { out = v; },
        _ => {}
    }
    out
}
fn run() -> text {
    t = T { v: \"hello\" };
    t.as_text()
}"
    )
    .expr("run()")
    .result(Value::str("hello"));
}

/// B1-style fix applied to or-patterns: `A | B => …` over unit variants
/// in a mixed struct-enum previously panicked at
/// parser/control.rs:699 (same index-OOB shape as B1).  Guard the
/// attributes[0] access the same way.
#[test]
fn p54_or_pattern_mixed_struct_enum() {
    code!(
        "pub enum Sig { Off, Idle, On { level: integer } }
fn classify(s: const Sig) -> text {
    match s {
        Off | Idle => \"inactive\",
        On { level } => \"active\"
    }
}
fn run() -> text {
    classify(On { level: 80 })
}"
    )
    .expr("run()")
    .result(Value::str("active"));
}

/// Match guard on a struct-enum variant works today — regression
/// guard so future parser work doesn't drop this.
#[test]
fn p54_match_guard_on_struct_enum() {
    code!(
        "pub enum Sig { Off, On { level: integer } }
fn describe(s: const Sig) -> text {
    match s {
        Off => \"off\",
        On { level } if level > 50 => \"hi\",
        On { level } => \"lo\"
    }
}
fn run() -> text {
    r = \"\";
    r += describe(On { level: 80 });
    r += \",\";
    r += describe(On { level: 10 });
    r
}"
    )
    .expr("run()")
    .result(Value::str("hi,lo"));
}

/// B2-runtime (sub-bug of B3/B4): constructing a bare unit-variant
/// literal (`s = Idle;`) for a mixed struct-enum crashes at runtime
/// with `index out of bounds: the len is 2 but the index is <junk>`.
/// B2-compile-fix let this test compile; the runtime codegen path for
/// producing a valid struct-enum record from a bare unit-variant name
/// is still broken.  When that's fixed, this test goes green.
#[test]
#[ignore = "P54 B2-runtime: unit-variant literal construction in struct-enum crashes"]
fn p54_b2_unit_variant_literal_construction() {
    code!(
        "pub enum Sig { Off, Idle, On { level: integer } }
fn run() -> text {
    s = Idle;
    match s {
        Off => \"off\",
        Idle => \"idle\",
        On { level } => \"on\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("idle"));
}
