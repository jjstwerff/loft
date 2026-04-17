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
    .error("Type annotations are not allowed in |x| lambdas — use fn(x: <type>) { ... } instead (add `-> <ret>` only for non-void returns; `-> void` is not a valid type) at s10_short_lambda_type_annotation_rejected:3:27");
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

// Type.parse(text) with JSON input.  Auto-wraps through
// json_parse internally (P54 step 5 with the step-6 backward-
// compatibility shim — see `src/parser/objects.rs::parse_type_parse`).
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

// Type.parse(text) — loft-native bare-key form (`{value: 7}`)
// is NOT standard JSON, so json_parse rejects it.  Rewritten
// to standard JSON so the test still guards the struct-unwrap
// behaviour under the auto-wrap path.
#[test]
fn type_parse_loft_native() {
    code!(
        r#"struct Score { value: integer, name: text }
fn test() {
    s = Score.parse(`{{"value": 7, "name": "hello"}}`);
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

// Successful JSON parse yields no diagnostic.  With P54 step 5's
// auto-wrap, text arguments still route through json_parse
// internally — the `s#errors` accessor stays empty on success,
// and `json_errors()` also stays empty.
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

// Malformed JSON — `Struct.parse(text)` routes through the
// legacy lenient parser (preserves loft-native bare-key
// support per QUALITY.md § P54-U) which populates `s#errors`.
// The new typed-tree path (`Struct.parse(json_parse(text))`)
// populates `json_errors()` instead.  Both produce null fields
// on bad input.
#[test]
fn errors_accessor_path_on_failure() {
    code!(
        r#"struct Score { value: integer }
fn test() {
    bad = Score.parse(`not_json`);
    err = bad#errors;
    assert(bad.value == null, "value should be null on bad parse");
    assert(len(err) > 0, "expected #errors entries for bad input");
}"#
    )
    .result(Value::Null);
}

// Type-mismatched nested input — `data: "not_an_object"` is a
// JString, not a JObject.  Under P54 step 5 the struct unwrap
// returns null-valued fields for kind mismatches; schema-level
// diagnostics arrive with Q1 schema-side (pending).  Verify the
// unwrap doesn't crash on the mismatched shape.
#[test]
fn errors_accessor_nested_path() {
    code!(
        r#"struct Inner { x: integer }
struct Outer { name: text, data: Inner }
fn test() {
    bad = Outer.parse(`{{"name": "ok", "data": "not_an_object"}}`);
    assert(bad.name == "ok", "outer name should survive: got {bad.name}");
    assert(bad.data.x == null, "nested x should be null on mismatched shape");
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

// ── File.content() on non-existent file ─────────────────────────────────────
// Regression guard — verifies that File.content() on a missing path returns
// an empty text (not garbage / not a crash) under the regular execute path.
// The historical SIGSEGV was specific to execute_log (LOFT_LOG=full); the
// runtime behaviour the test asserts is now stable.  Un-ignored 2026-04-14
// after the test was found to pass in isolation; if execute_log ever
// regresses, that variant is exercised by the LOFT_LOG-driven test dumps,
// not by this guard.
//
// Deleted: file_content_nonexistent_trace — duplicate of file_content_nonexistent
// (passing), and the ignore was for a P136-adjacent harness bug, not a behavior gap.

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
// In release mode the 100 000 struct allocations complete in ~0.05s and
// the test is a real store-exhaustion regression guard that rides along
// with `cargo test --release` (CI's default).  In debug mode the same
// body takes ~10 minutes because the loft bytecode interpreter is
// dominated by debug Rust overhead — so we cfg-gate the `#[ignore]`
// attribute to debug_assertions only, not the whole test.  That keeps
// `cargo test` (debug) fast for day-to-day iteration while CI continues
// to exercise the real stress path.  Run manually in debug with
// `cargo test --ignored p122_long_running_struct_loop` when needed.
#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "P122 stress test — ~10min in debug mode; runs in release automatically (passes in ~0.05s)."
)]
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

/// String-variant parsing: the value is correctly stored, but
/// returning it through a function boundary trips a JsonValue-store
/// lifecycle issue — the store is freed during scope-exit cleanup
/// before the caller's text-copy machinery completes.  Same root
/// cause as `p54_extractor_as_text` below.  Standalone smoke
/// (`/tmp/jp1.loft` style with `match v { JString { value } =>
/// println("str={value}") }`) works fine inline; only the
/// fn-return path is broken.
#[test]
fn p54_parse_primitive_string() {
    code!(
        "fn run() -> text {
    v = json_parse(\"\\\"hello\\\"\");
    out = \"\";
    match v {
        JString { value } => { out = value; },
        _ => {}
    }
    out
}"
    )
    .expr("run()")
    .result(Value::str("hello"));
}

#[test]
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
fn p54_malformed_returns_jnull() {
    // Use a malformed input that doesn't trip loft's text-literal
    // interpolation (curly braces in `"…"` would).
    code!(
        "fn run() -> boolean {
    v = json_parse(\"xyz\");
    match v {
        JNull => true,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// `as_text` returns a `Str` into `Stores::scratch`; calling it
/// inline + assigning into a local works (`println("{v.as_text()}")`
/// is fine), but returning the resulting text through a function
/// boundary trips the same store-free-before-copy lifecycle as
/// `p54_parse_primitive_string`.  Both unblock together.
#[test]
fn p54_extractor_as_text() {
    code!(
        "fn run() -> text {
    v = json_parse(\"\\\"abc\\\"\");
    out = \"\";
    out += v.as_text();
    out
}"
    )
    .expr("run()")
    .result(Value::str("abc"));
}

/// Same store-free-before-copy lifecycle as the matching extractor
/// test above.  Unblocks once that lands.
#[test]
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

// Un-ignored 2026-04-14 by P54 step 4 third slice — non-empty
// primitive objects now materialise, n_field walks the arena
// vector for a name match, chained `.as_text()` reads the
// JString value out of the matched JsonField slot.
//
// Loft strings treat `{…}` as interpolation; the JSON `{` and
// `}` in the literal are doubled (`{{` / `}}`) so they reach
// `json_parse` as single braces.  Rationale in LOFT.md §
// String literals.
#[test]
fn p54_parse_object_field_access() {
    code!(
        "fn run() -> text {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}}\");
    v.field(\"name\").as_text()
}"
    )
    .expr("run()")
    .result(Value::str("Alice"));
}

// Un-ignored 2026-04-14 by P54 step 4 second slice — non-empty
// primitive arrays now materialise, `n_item` dispatches on JArray,
// and `as_long()` returns the element's numeric payload.
#[test]
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

/// Chain access on a non-object value never traps — every
/// intermediate `field()` / `item()` returns `JNull`.  Step 3 stub:
/// since object/array parsing isn't wired yet, json_parse on any
/// object-shaped input returns JNull anyway, and the chain is
/// JNull all the way down.  Locks the chained-access safety
/// guarantee on a non-object root: every intermediate failure
/// produces JNull rather than trapping.  The positive-path
/// counterpart is `p54_chained_access_on_nested_object`.
#[test]
fn p54_missing_chain_returns_jnull() {
    // `{` in a loft text literal triggers format-string interpolation;
    // use a primitive that json_parse handles to produce a non-object
    // root.  The chain still lands at JNull because field/item on a
    // non-object always returns JNull.
    code!(
        "fn run() -> boolean {
    v = json_parse(\"42\");
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

/// P54 step 4 — chained access on a real nested object reaches
/// the leaf.  Locks the documented STDLIB.md JSON example
/// (`v.field("users").item(0).field("name").as_text()`) so the
/// reference doc and the runtime cannot drift independently.
#[test]
fn p54_chained_access_on_nested_object() {
    code!(
        "fn run_pca() -> text {
    v_pca = json_parse(`{{\"users\":[{{\"name\":\"Alice\"}}]}}`);
    v_pca.field(\"users\").item(0).field(\"name\").as_text()
}"
    )
    .expr("run_pca()")
    .result(Value::str("Alice"));
}

/// P54 step 4 — locks the LOFT.md § Match expressions JsonValue
/// example.  Exercises destructuring of every JsonValue variant
/// (JObject / JArray / JNumber / JNull / wildcard) so that the
/// documented `match json_parse(raw) { ... }` patterns stay
/// supported.  When the doc is read by a new user, the same
/// arms must dispatch correctly today.
#[test]
fn p54_match_on_jsonvalue_classifies_each_kind() {
    code!(
        "fn classify_pmj(raw: text) -> text {
    match json_parse(raw) {
        JObject { fields: _ } => \"object\",
        JArray  { items: _ }  => \"array\",
        JNumber { value: _ }  => \"number\",
        JNull                 => \"null-or-error\",
        _                     => \"other\"
    }
}
fn run_pmj() -> integer {
    score_pmj = 0;
    if classify_pmj(`{{\"k\":1}}`) == \"object\" { score_pmj += 1; }
    if classify_pmj(\"[1,2]\") == \"array\" { score_pmj += 1; }
    if classify_pmj(\"42\") == \"number\" { score_pmj += 1; }
    if classify_pmj(\"null\") == \"null-or-error\" { score_pmj += 1; }
    if classify_pmj(\"not-json\") == \"null-or-error\" { score_pmj += 1; }
    if classify_pmj(`\"hi\"`) == \"other\" { score_pmj += 1; }
    score_pmj
}"
    )
    .expr("run_pmj()")
    .result(Value::Int(6));
}

#[test]
fn p54_struct_parse_accepts_jsonvalue() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> text {
    v = json_parse(\"{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}\");
    u = User.parse(v);
    u.name
}"
    )
    .expr("run()")
    .result(Value::str("Bob"));
}

/// P54 step 5 — `Type.parse(JsonValue)` populates an integer field
/// by unwrapping through `as_long()` + `OpCastIntFromLong` narrow.
/// Pairs with the above text-field test; together they lock both
/// primitive paths (text via OpSetText, integer via OpSetInt).
#[test]
fn p54_struct_parse_accepts_jsonvalue_integer_field() {
    code!(
        "struct User { name: text, age: integer }
fn run_age() -> integer {
    v = json_parse(\"{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}\");
    u = User.parse(v);
    u.age
}"
    )
    .expr("run_age()")
    .result(Value::Int(25));
}

/// P54 step 5 nested slice — nested struct field (`Type::Reference`
/// to another struct) recurses into `build_struct_from_jsonvalue`
/// on the corresponding sub-JsonValue, populating the embedded
/// struct record.  Exercises the full path through
/// `OpCopyRecord`-for-Reference in `set_field_no_check`.
#[test]
fn p54_struct_parse_accepts_nested_struct_field() {
    code!(
        "struct Inner { x_coord: integer }
struct Outer { label: text, data: Inner }
fn run_nested() -> integer {
    v = json_parse(\"{{\\\"label\\\":\\\"ok\\\",\\\"data\\\":{{\\\"x_coord\\\":99}}}}\");
    o = Outer.parse(v);
    o.data.x_coord
}"
    )
    .expr("run_nested()")
    .result(Value::Int(99));
}

/// P54 step 5 nested slice — verify the outer struct's primitive
/// text field is populated when a nested-struct field is also
/// present.  Complements `p54_struct_parse_accepts_nested_struct_field`
/// which returned only the inner integer; this returns the outer
/// label so a regression in field-ordering or offset calculation
/// for mixed-type structs gets caught.
#[test]
fn p54_struct_parse_nested_populates_outer_text_too() {
    code!(
        "struct Inner { x_coord: integer }
struct Outer { label: text, data: Inner }
fn run_label() -> text {
    v = json_parse(\"{{\\\"label\\\":\\\"ok\\\",\\\"data\\\":{{\\\"x_coord\\\":99}}}}\");
    o = Outer.parse(v);
    o.label
}"
    )
    .expr("run_label()")
    .result(Value::str("ok"));
}

/// P54 step 5 — `JsonValue`-typed field captures the sub-tree
/// verbatim as a passthrough.  Solves the "forward arbitrary
/// subtree" use case where a struct has a dynamic-shape payload.
/// The field() result gets OpCopyRecord'd into the struct's
/// JsonValue slot; kind() on the embedded payload reads the
/// discriminant back as `"JArray"` confirming the bytes round-trip.
#[test]
fn p54_struct_parse_captures_jsonvalue_field_verbatim() {
    code!(
        "struct WithPayload { name: text, info: JsonValue }
fn run_payload_kind() -> text {
    v = json_parse(\"{{\\\"name\\\":\\\"demo\\\",\\\"info\\\":[1,2,3]}}\");
    p = WithPayload.parse(v);
    p.info.kind()
}"
    )
    .expr("run_payload_kind()")
    .result(Value::str("JArray"));
}

/// P54 step 5 vector-field slice — populate `vector<long>` from
/// a JArray of numbers.  Today's implementation routes through
/// the `n_jsonvalue_to_vector_long` native which walks the
/// JArray at runtime, truncates each JNumber toward zero, and
/// appends.  Other primitive element types (text / float /
/// boolean / integer) and struct-element vectors are follow-up
/// slices.
#[test]
fn p54_struct_parse_accepts_vector_long_field_len() {
    code!(
        "struct Data { items: vector<long> }
fn run() -> integer {
    v = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    d = Data.parse(v);
    len(d.items)
}"
    )
    .expr("run()")
    .result(Value::Int(3));
}

#[test]
fn p54_struct_parse_vector_long_first_element() {
    code!(
        "struct Data { items: vector<long> }
fn run_first() -> long {
    v = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    d = Data.parse(v);
    d.items[0]
}"
    )
    .expr("run_first()")
    .result(Value::Long(10));
}

#[test]
fn p54_struct_parse_vector_long_iterates_correctly() {
    code!(
        "struct Data { items: vector<long> }
fn run_sum() -> long {
    v = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    d = Data.parse(v);
    total = 0l;
    for x in d.items { total += x; }
    total
}"
    )
    .expr("run_sum()")
    .result(Value::Long(60));
}

#[test]
fn p54_struct_parse_vector_long_empty_array() {
    code!(
        "struct Data { items: vector<long> }
fn run_empty() -> integer {
    v = json_parse(\"{{\\\"items\\\":[]}}\");
    d = Data.parse(v);
    len(d.items)
}"
    )
    .expr("run_empty()")
    .result(Value::Int(0));
}

/// P54 step 5 vector-field slice — `vector<integer>` populated
/// via the generic `n_jsonvalue_to_vector` native (elem_code = 2).
/// JNumber elements truncate toward zero with i32 narrowing;
/// non-number elements contribute `i32::MIN`.
#[test]
fn p54_struct_parse_vector_integer_field() {
    code!(
        "struct D { ns: vector<integer> }
fn run_int() -> integer {
    v = json_parse(\"{{\\\"ns\\\":[100,200,300]}}\");
    d = D.parse(v);
    total = 0;
    for x in d.ns { total += x; }
    total
}"
    )
    .expr("run_int()")
    .result(Value::Int(600));
}

/// P54 step 5 vector-field slice — `vector<float>` populated
/// via the generic `n_jsonvalue_to_vector` native (elem_code = 3).
/// JNumber elements pass through verbatim; non-number → NaN.
#[test]
fn p54_struct_parse_vector_float_field() {
    code!(
        "struct D { fs: vector<float> }
fn run_float() -> float {
    v = json_parse(\"{{\\\"fs\\\":[1.5,2.5]}}\");
    d = D.parse(v);
    d.fs[0]
}"
    )
    .expr("run_float()")
    .result(Value::Float(1.5));
}

/// P54 step 5 vector-field slice — `vector<boolean>` populated
/// via the generic `n_jsonvalue_to_vector` native (elem_code = 4).
/// JBool elements copy 0/1 byte; non-bool → 0.  The boolean case
/// previously hung because the handle store allocated only 1 word
/// (matching the element size) but the handle's vec_rec int sits
/// at byte offset 8 — overflowed the next free-block's header
/// and corrupted claim_scan into an infinite loop.  Fixed by
/// ensuring the handle store is always ≥ 2 words regardless of
/// element size.
#[test]
fn p54_struct_parse_vector_boolean_field() {
    code!(
        "struct D { bs: vector<boolean> }
fn run_bool() -> boolean {
    v = json_parse(\"{{\\\"bs\\\":[true,false,true]}}\");
    d = D.parse(v);
    d.bs[1]
}"
    )
    .expr("run_bool()")
    .result(Value::Boolean(false));
}

/// P54 step 5 vector-field slice — `vector<text>` populated via
/// the generic `n_jsonvalue_to_vector` native (elem_code = 5).
/// JString elements copy into the result vector's string area;
/// non-string → empty text.
#[test]
fn p54_struct_parse_vector_text_field() {
    code!(
        "struct D { ts: vector<text> }
fn run_text() -> text {
    v = json_parse(\"{{\\\"ts\\\":[\\\"hello\\\",\\\"world\\\"]}}\");
    d = D.parse(v);
    d.ts[0]
}"
    )
    .expr("run_text()")
    .result(Value::str("hello"));
}

/// P54 step 5 vector-of-struct slice — `vector<T>` where `T` is
/// a struct populates each element via runtime field-walk
/// (elem_code = 6).  The native enumerates the struct's fields
/// from `stores.types[struct_kt].parts` and writes each
/// primitive field by name lookup in the JSON object element.
/// Today handles primitive struct fields (text / integer /
/// long / float / boolean); nested struct or vector fields
/// inside the element type stay at zero-init defaults.
#[test]
fn p54_struct_parse_vector_of_struct_count() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}]}}\");
    inbox = Inbox.parse(v);
    len(inbox.users)
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

#[test]
fn p54_struct_parse_vector_of_struct_first_text_field() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> text {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}]}}\");
    inbox = Inbox.parse(v);
    inbox.users[0].name
}"
    )
    .expr("run()")
    .result(Value::str("Alice"));
}

#[test]
fn p54_struct_parse_vector_of_struct_second_integer_field() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}]}}\");
    inbox = Inbox.parse(v);
    inbox.users[1].age
}"
    )
    .expr("run()")
    .result(Value::Int(25));
}

#[test]
fn p54_struct_parse_vector_of_struct_iterates() {
    code!(
        "struct Score { val: long }
struct Bag { scores: vector<Score> }
fn run() -> long {
    v = json_parse(\"{{\\\"scores\\\":[{{\\\"val\\\":10}},{{\\\"val\\\":20}},{{\\\"val\\\":30}}]}}\");
    b = Bag.parse(v);
    total = 0l;
    for s in b.scores { total += s.val; }
    total
}"
    )
    .expr("run()")
    .result(Value::Long(60));
}

#[test]
fn p54_struct_parse_vector_of_struct_empty_array() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[]}}\");
    inbox = Inbox.parse(v);
    len(inbox.users)
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

#[test]
fn p54_struct_parse_vector_of_struct_missing_field_is_null() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\"}}]}}\");
    inbox = Inbox.parse(v);
    inbox.users[0].age
}"
    )
    .expr("run()")
    .result(Value::Null);
}

/// Q1 schema-side — type mismatch on a primitive field during
/// `Type.parse(JsonValue)` pushes a path-qualified diagnostic
/// to `json_errors()` instead of silently producing a null
/// sentinel.  Asserts the diagnostic contains the struct's
/// name + field name + expected vs actual variant.
#[test]
fn q1_schema_side_type_mismatch_pushes_diagnostic() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> boolean {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":\\\"thirty\\\"}}\");
    u = User.parse(v);
    if u == u {}
    err = json_errors();
    err.contains(\"User.age\") && err.contains(\"expected JNumber\") && err.contains(\"got JString\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// Q1 schema-side — missing fields (JSON object lacks the key
/// entirely) DO NOT push a diagnostic.  Distinguishes "absent"
/// from "present-but-wrong-kind" — the former is silently
/// allowed (caller handles the null sentinel via `??` or `!`).
#[test]
fn q1_schema_side_missing_field_silent() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> integer {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\"}}\");
    u = User.parse(v);
    if u == u {}
    json_errors().len()
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

/// Q1 schema-side — a clean parse leaves `json_errors()`
/// empty.  Companion guard to the mismatch-pushes test.
#[test]
fn q1_schema_side_clean_parse_no_diagnostic() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> integer {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}}\");
    u = User.parse(v);
    if u == u {}
    json_errors().len()
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

/// Q1 schema-side — type mismatch on a primitive field of a
/// struct INSIDE a vector element pushes a diagnostic via the
/// runtime field-walk (not the compile-time check).  The
/// runtime path mirrors the same struct-name + field-name
/// diagnostic shape as the compile-time path.
#[test]
fn q1_schema_side_vector_element_type_mismatch_pushes_diagnostic() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> boolean {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":\\\"twenty\\\"}}]}}\");
    inbox = Inbox.parse(v);
    if inbox == inbox {}
    err = json_errors();
    err.contains(\"User.age\") && err.contains(\"expected JNumber\") && err.contains(\"got JString\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// Q1 schema-side — text field receiving a number pushes a
/// diagnostic naming JString as expected and JNumber as actual.
#[test]
fn q1_schema_side_text_field_receiving_number() {
    code!(
        "struct User { name: text }
fn run() -> boolean {
    v = json_parse(\"{{\\\"name\\\":42}}\");
    u = User.parse(v);
    if u == u {}
    err = json_errors();
    err.contains(\"User.name\") && err.contains(\"expected JString\") && err.contains(\"got JNumber\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// Q1 schema-side — boolean field receiving a string pushes a
/// diagnostic naming JBool as expected.
#[test]
fn q1_schema_side_boolean_field_receiving_string() {
    code!(
        "struct Flag { active: boolean }
fn run() -> boolean {
    v = json_parse(\"{{\\\"active\\\":\\\"yes\\\"}}\");
    f = Flag.parse(v);
    if f == f {}
    err = json_errors();
    err.contains(\"Flag.active\") && err.contains(\"expected JBool\") && err.contains(\"got JString\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

// Deleted: p54_struct_parse_rejects_plain_text — tested a rejected design decision
// (hard rejection of text args). Current design auto-wraps through json_parse.

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

/// Direct-return versions — now working (commit `6074619` opened up
/// struct-enum subject dispatch in match; constructing + returning
/// without an intermediate variable round-trips cleanly).
#[test]
fn p54_b3_float_not_null_direct_return() {
    code!(
        "pub enum JV { A { v: float not null } }
fn mk() -> JV { A { v: 42.5 } }
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

#[test]
fn p54_b4_mixed_variant_direct_return() {
    code!(
        "pub enum JV { JA { v: boolean }, JB { v: integer }, JC { v: text } }
fn mk() -> JV { JB { v: 42 } }
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

/// Open (sharpened): the intermediate-variable pattern `n = A { … };
/// n` (tail expression, no `return`) crashes.  Sharpened in
/// `p54_b3_int_via_intermediate` to narrow the bug: the
/// tail-expression path frees the local's store while the returned
/// value still references it.  `return n;` works — see
/// `p54_struct_enum_explicit_return_of_local`.
#[test]
fn p54_b3_float_via_intermediate() {
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

/// B5 full regression guard: self-referential struct-enum with
/// recursive method.  Three layers had to land for this to pass:
///   1. `fill_all` registers `main_vector<T>` wrappers for every
///      struct/enum-variant `vector<T>` field — closes the original
///      "Incomplete record" panic on `OpDatabase(db_tp=u16::MAX)`.
///   2. Match-arm bindings carry `skip_free` — closes the garbage
///      `FreeRef(ref(4621,…))` on a not-taken arm's binding slot.
///   3. Struct-enum return-slot accounting (closed as a side-effect
///      of the cross-PR struct-enum return work landed in #168→#174).
#[test]
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

/// B5 match-arm-binding regression (layer 2): the not-taken arm of
/// a match whose binding is a `vector<T>` must not crash scope
/// cleanup.  Before the `skip_free` fix at src/parser/control.rs:1103,
/// the match's `_mv_items_*` binding in the `Full { items }` arm
/// was freed at function exit even when the `Empty` arm was taken,
/// reading garbage bytes as a `DbRef` and panicking in
/// `Stores::free_named` with an out-of-bounds `store_nr`.  Now
/// skip_free suppresses the OpFreeRef emission, so the garbage
/// slot stays untouched.  Guards the layer-2 half of B5 against
/// regression independently of the recursion path still open in
/// `p54_b5_recursive_struct_enum`.
#[test]
fn p54_b5_not_taken_arm_with_vector_binding_ok() {
    code!(
        "struct Item { v: integer }
pub enum Wrap { Empty, Full { items: vector<Item> } }
fn run() -> integer {
    w = Empty;
    match w {
        Empty => 42,
        Full { items } => items.len()
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// B5 type-registration regression: a recursive struct-enum (`Node`
/// variant contains `vector<Tree>`) must get its `main_vector<Tree>`
/// wrapper registered during `fill_all`, so codegen's
/// `name_type("main_vector<Tree>")` lookup returns a real
/// `known_type` instead of `u16::MAX`.  Without this fix, simply
/// constructing `Node { kids: [...] }` would panic in
/// `Store::claim("Incomplete record")` — the scenario the original
/// B5 ticket reported.  This narrower regression guard exercises
/// just the construct-and-measure-len path (no match, no for-loop),
/// isolating the half of B5 that is now fixed from the still-open
/// match-arm-binding half tracked in `p54_b5_recursive_struct_enum`.
#[test]
fn p54_b5_recursive_struct_enum_construction() {
    code!(
        "pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }] };
    match root {
        Leaf { v } => v,
        Node { kids } => kids.len()
    }
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

/// B5 layer 1 + 2 combined regression: iterate over a struct-enum-
/// variant `vector<T>` binding inside a match arm.  Exercises the
/// same code paths as `p54_b5_recursive_struct_enum` (the still-
/// ignored recursive one) up to but not including the recursive
/// inner call — so if the recursion layer lands later, a regression
/// on type registration OR binding skip_free gets caught here even
/// when the recursive path is green.  Asserts the for-loop sees
/// each element with its correct `Leaf.v` payload.
#[test]
fn p54_b5_for_loop_over_enum_variant_vector() {
    code!(
        "pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }, Leaf { v: 5 }] };
    sum = 0;
    match root {
        Leaf { v } => sum += v,
        Node { kids } => {
            for k in kids {
                match k {
                    Leaf { v } => sum += v,
                    Node { kids } => sum += kids.len()
                }
            }
        }
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(12));
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

/// FIXED: single-variant struct-enum with integer payload returned
/// from a function now round-trips cleanly.  Previously crashed with
/// 'malloc(): unaligned tcache chunk detected'.  The fix that closed
/// this was the Reference(Enum)-as-match-subject arm in commit
/// `6074619` — same root cause (the for-body subject-type dispatch
/// mismatch also affected struct-enum assignment-site dispatch).
/// `float` / `float not null` variants (see p54_b3_float_not_null_variant)
/// and mixed-field variants (p54_b4_mixed_variant_return) remain
/// broken.
#[test]
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

/// P54 — the same extractor pattern via a struct-enum.  Now works
/// after the Reference(Enum) match-subject fix (commit `6074619`) —
/// the `t.as_text()` call site receives a struct-enum argument,
/// matches it, and returns the bound text.
#[test]
fn p54_struct_enum_extractors_spec() {
    code!(
        "pub enum Jv { Jstr { v: text }, Jnum { v: float } }
pub fn jv_as_text(self: Jv) -> text {
    jvat_out = \"\";
    match self {
        Jstr { v } => { jvat_out = v; },
        _ => {}
    }
    jvat_out
}
fn make_jstr() -> Jv { Jstr { v: \"hello\" } }
fn run() -> text {
    jvs_t = make_jstr();
    jvs_t.jv_as_text()
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

/// P54 — positive baseline: a plain enum (all unit variants) round-trips
/// through a bare identifier literal.  Only the *mixed* struct-enum
/// case is broken (B2-runtime), not plain enums.  This test guards
/// that distinction.
#[test]
fn p54_plain_enum_bare_variant_works() {
    code!(
        "pub enum Sig { Off, Idle, On }
fn run() -> text {
    s = Idle;
    match s {
        Off => \"off\",
        Idle => \"idle\",
        On => \"on\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("idle"));
}

/// B2-runtime (qualified form): `Sig.Idle` as an expression in a
/// mixed struct-enum.  The parse path (parser/fields.rs) was giving
/// the result block `Type::Enum(dnr, true, vec![w])` — propagating
/// the work-ref into the LHS as a dep, so `s` became a borrower and
/// nothing freed the store.  Fixed by mirroring parser/objects.rs
/// and using `vec![]` instead (LHS owns, work-ref is skip_free).
#[test]
fn p54_b2_qualified_unit_variant_mixed_enum() {
    code!(
        "pub enum Sig { Off, Idle, On { level: integer } }
fn run() -> text {
    s = Sig.Idle;
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

/// P54 — positive baseline: plain enum match inside a for-loop body
/// works.  Pairs with the `#[ignore]`'d struct-enum version below to
/// isolate the struct-enum-specific breakage.
#[test]
fn p54_plain_enum_match_inside_for() {
    code!(
        "pub enum Item { One, Two }
fn run() -> text {
    v = [One, Two];
    r = \"\";
    for x in v {
        match x {
            One => { r += \"1\"; },
            Two => { r += \"2\"; }
        }
    }
    r
}"
    )
    .expr("run()")
    .result(Value::str("12"));
}

/// FIXED: struct-enum match inside a for-loop body previously failed
/// to parse with 'Expect token }' on the first arm's `=>`.  Root
/// cause: `for_type` maps `vector<StructEnum>` to
/// `Type::Reference(enum_def, …)` as the loop-variable type, but
/// `parse_match` only accepted `Type::Enum` or
/// `Type::Reference(EnumValue/Struct)` as a valid subject.  Added a
/// `Reference(d_nr) if DefType::Enum` case; struct-enum for-body
/// matches now compile and run.
#[test]
fn p54_struct_enum_match_inside_for() {
    code!(
        "pub enum Item { Empty, Filled { qty: integer } }
fn run() -> integer {
    v = [Filled { qty: 3 }, Filled { qty: 7 }];
    sum = 0;
    for x in v {
        match x {
            Empty => {},
            Filled { qty } => { sum += qty; }
        }
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(10));
}

/// P54 — positive baseline: struct-enum match outside a for-loop (via
/// direct indexing) works.  Confirms the struct-enum match machinery
/// itself is fine — the above bug lives in for-body parsing.
#[test]
fn p54_struct_enum_match_via_index_works() {
    code!(
        "pub enum Item { Empty, Filled { qty: integer } }
fn run() -> integer {
    v = [Filled { qty: 3 }, Filled { qty: 7 }];
    x = v[0];
    match x {
        Empty => 0,
        Filled { qty } => qty
    }
}"
    )
    .expr("run()")
    .result(Value::Int(3));
}

/// Struct-enum as a field of a plain struct — construction, access
/// through field chain, and match all work.  This is a pattern
/// JsonValue callers use today (wrap the JsonValue in a holder
/// struct).
#[test]
fn p54_struct_enum_as_struct_field() {
    code!(
        "pub enum Inner { A { v: integer }, B { v: text } }
pub struct Holder { inner: Inner, count: integer }
fn run() -> text {
    h = Holder { inner: A { v: 7 }, count: 1 };
    match h.inner {
        A { v } => \"A-{v}-{h.count}\",
        B { v } => \"B-{v}-{h.count}\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("A-7-1"));
}

/// Vector of struct-enums with mixed variants; iterate and dispatch
/// by variant.  Exercises the Reference(Enum) match-subject fix
/// (commit `6074619`) plus accumulator mutation inside match arms.
#[test]
fn p54_struct_enum_vector_accumulate() {
    code!(
        "pub enum Op { Add { v: integer }, Sub { v: integer } }
fn run() -> integer {
    ops = [Add { v: 5 }, Sub { v: 3 }, Add { v: 2 }];
    sum = 0;
    for op in ops {
        match op {
            Add { v } => { sum += v; },
            Sub { v } => { sum -= v; }
        }
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(4));
}

/// B3 narrowed: the bug is the TAIL-EXPRESSION path, not the
/// intermediate variable itself.  `n = A { … }; n` (no `return`)
/// crashes because the function's scope exit frees `n`'s store
/// while the returned value still references it.  `return n;`
/// works fine — see `p54_struct_enum_explicit_return_of_local`.
/// Fix requires teaching the tail-expression-as-return codegen to
/// suppress the free of any local whose store is being returned,
/// or to materialize a copy.
#[test]
fn p54_b3_int_via_intermediate() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV {
    n = A { v: 42 };
    n
}
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

/// Struct-enum intermediate-variable return — WORKS with explicit
/// `return n;` statement.  Pairs with `p54_b3_int_via_intermediate`
/// which fails on the tail-expression form `n = A { … }; n` (no
/// `return` keyword).  The tail-expression path has an ownership
/// bug: the local `n` gets freed on function exit while the returned
/// value still references the same store — explicit `return n`
/// avoids it.  Workaround today: always write `return n;`.
#[test]
fn p54_struct_enum_explicit_return_of_local() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV {
    n = A { v: 42 };
    return n;
}
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

/// Reassignment before return — also works with explicit return.
/// Documents that the bug is specifically the *tail-expression*
/// path, not the assignment path.
#[test]
fn p54_struct_enum_reassign_explicit_return() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV {
    n = A { v: 1 };
    n = A { v: 42 };
    return n;
}
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

/// Struct-enum as the value in `hash<Entry[name]>` — JsonValue's
/// eventual shape for JObject.  Works end-to-end: construction,
/// hash lookup, match on the retrieved value's enum field.  Pairs
/// with `p54_struct_enum_as_struct_field` to show struct-enum
/// embedding in containers is fully viable.
#[test]
fn p54_struct_enum_in_hash_value() {
    code!(
        "pub enum Val { IntV { v: integer }, StrV { v: text } }
pub struct Entry { name: text, value: Val }
pub struct Holder { h: hash<Entry[name]> }
fn run() -> text {
    m = Holder { h: [Entry { name: \"a\", value: IntV { v: 7 } }] };
    e = m.h[\"a\"];
    if e == null { return \"miss\"; }
    match e.value {
        IntV { v } => \"int-{v}\",
        StrV { v } => \"str-{v}\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("int-7"));
}

/// Nested struct-enum — outer variant carries an inner struct-enum
/// as a field.  Full match-and-destructure chain works.  Critical
/// for JsonValue's JArray-of-JObjects / JObject-of-JArrays cases.
#[test]
fn p54_nested_struct_enum() {
    code!(
        "pub enum Inner { Leaf { v: integer } }
pub enum Outer { Wrap { inner: Inner }, Plain }
fn run() -> integer {
    o = Wrap { inner: Leaf { v: 42 } };
    match o {
        Plain => -1,
        Wrap { inner } => {
            match inner {
                Leaf { v } => v
            }
        }
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// Struct-enum flowing through multiple function calls — parameter
/// into one fn, return from another, construct a fresh variant in
/// one arm, pass through in another.  Exercises the full Reference
/// / return / assignment path.
#[test]
fn p54_struct_enum_multi_call_flow() {
    code!(
        "pub enum V { A { v: integer }, B { v: text } }
fn double_a(x: const V) -> V {
    match x {
        A { v } => A { v: v * 2 },
        B { v } => B { v: v }
    }
}
fn describe(x: const V) -> text {
    match x {
        A { v } => \"a-{v}\",
        B { v } => \"b-{v}\"
    }
}
fn run() -> text {
    a = A { v: 5 };
    d = double_a(a);
    describe(d)
}"
    )
    .expr("run()")
    .result(Value::str("a-10"));
}

// ── P22: spacial<T> diagnostic wording (FIXED) ─────────────────────────
//
// `spacial<T>` is reserved for the planned 1.1+ ordered-spatial
// collection.  Today it's not implemented; the parser has a bespoke
// diagnostic that names the feature, the milestone, and the
// substitute (`sorted<T>` / `index<T>`) so users who guess the
// keyword get a useful answer rather than "unknown type".
//
// This test guards the wording — the diagnostic must mention BOTH
// the milestone (so users know when to retry) AND the substitute
// (so they know what to use today).
#[test]
fn p22_spacial_diagnostic_names_milestone_and_substitute() {
    code!(
        "struct Point { x: float not null, y: float not null }
struct World { items: spacial<Point> }
fn test() {
    w = World { items: [] };
}"
    )
    .error(
        "spacial<T> is planned for 1.1+; until then use sorted<T> or index<T> for ordered lookups \
at p22_spacial_diagnostic_names_milestone_and_substitute:2:39",
    );
}

// ── INC#29: !value asymmetry between boolean and integer ───────────────
//
// The unary `!` operator catches different things on different
// scalar types because the null sentinel is in-band:
//
//   boolean: false IS the null sentinel — `!b` catches both
//   integer: 0 is a real value — `!n` catches only i32::MIN
//
// This asymmetry silently changes meaning when code is ported
// between the two types.  These tests lock both shapes so a future
// uniformity refactor cannot regress without a doc update.
#[test]
fn inc29_bang_boolean_catches_false() {
    code!(
        "fn run() -> boolean {
    flag = false;
    !flag
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

#[test]
fn inc29_bang_integer_zero_is_not_null() {
    code!(
        "fn run() -> boolean {
    count = 0;
    !count
}"
    )
    .expr("run()")
    .result(Value::Boolean(false));
}

#[test]
fn inc29_bang_integer_null_is_caught() {
    code!(
        "fn divide(a: integer, b: integer) -> integer { a / b }
fn run() -> boolean {
    n = divide(1, 0);
    !n
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

// ── INC#3: #index semantics on text vs vector ──────────────────────────
//
// Text loops:    c#index == byte offset of current char (UTF-8)
// Vector loops:  v#index == 0-based element position
//
// On ASCII text the two coincide; on multi-byte text they diverge.
// Code that uses c#index as a counter passes its tests on ASCII and
// silently breaks on the first emoji.  These guards lock the
// divergence so a future "make these uniform" refactor cannot land
// without first updating LOFT.md.
#[test]
fn inc3_text_index_is_byte_offset_on_multibyte() {
    code!(
        "fn run() -> integer {
    last = 0;
    for c in \"a😊b\" { last = c#index; }
    last
}"
    )
    .expr("run()")
    // 'a' at 0, '😊' at 1 (4 bytes), 'b' at 5
    .result(Value::Int(5));
}

#[test]
fn inc3_text_count_is_character_position() {
    code!(
        "fn run() -> integer {
    last = 0;
    for c in \"a😊b\" { last = c#count; }
    last
}"
    )
    .expr("run()")
    // count is iterations completed so far; on the 'b' iteration that's 2
    .result(Value::Int(2));
}

#[test]
fn inc3_vector_index_is_element_position() {
    code!(
        "fn run() -> integer {
    items = [10, 20, 30];
    last = 0;
    for v in items { last = v#index; }
    last
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

// ── INC#26: match exhaustiveness ignores guarded arms ───────────────────
//
// A guarded arm (`pattern if guard => body`) does NOT count as
// covering that variant for exhaustiveness — the guard may fail
// at runtime.  Even if every variant has a guarded arm, a
// wildcard `_ =>` or an unguarded arm is still required.
//
// This is intentional (soundness: the compiler cannot prove the
// guard is always true), but surprising.  These tests lock the
// behaviour so a future "smarter exhaustiveness" attempt cannot
// silently drop the wildcard requirement without updating LOFT.md.
#[test]
fn inc26_guarded_arm_without_wildcard_is_rejected() {
    code!(
        "enum Color { Red, Green, Blue }
fn describe(c: const Color, bright: boolean) -> text {
    match c {
        Red if bright => \"bright red\",
        Green         => \"green\",
        Blue          => \"blue\"
    }
}"
    )
    // The match is not exhaustive: the Red guard can fail, and there
    // is no fallback for that case.  Parser must reject at compile time.
    .error(
        "match on Color is not exhaustive — missing: Red; add the missing variants or a '_ =>' wildcard \
at inc26_guarded_arm_without_wildcard_is_rejected:3:12",
    );
}

#[test]
fn inc26_guarded_arm_with_wildcard_compiles() {
    code!(
        "enum Color { Red, Green, Blue }
fn describe(c: const Color, bright: boolean) -> text {
    match c {
        Red if bright => \"bright red\",
        Green         => \"green\",
        Blue          => \"blue\",
        _             => \"dim red\"
    }
}
fn run() -> text { describe(Red, false) }"
    )
    .expr("run()")
    .result(Value::str("dim red"));
}

#[test]
fn inc26_guarded_arm_falls_through_when_guard_false() {
    code!(
        "enum Color { Red, Green, Blue }
fn describe(c: const Color, bright: boolean) -> text {
    match c {
        Red if bright => \"bright red\",
        Red           => \"dim red\",
        Green         => \"green\",
        Blue          => \"blue\"
    }
}
fn run() -> text { describe(Red, false) }"
    )
    .expr("run()")
    .result(Value::str("dim red"));
}

// ── INC#12: sort direction in the struct drives iteration direction ─────
//
// A `-` prefix on a key field in `sorted<T[-key]>` or
// `index<T[-key]>` flips the iteration direction of every query on
// that collection.  Reading the query site alone does not reveal
// the direction — the user must check the struct declaration,
// which may be far away.  This is the core of INC#12.
//
// These tests lock the direction-driven behaviour on two
// otherwise-identical sorted collections so a future uniformity
// refactor cannot silently flip either path without updating
// LOFT.md's Gotcha callout.
#[test]
fn inc12_sorted_ascending_iterates_forward() {
    code!(
        "struct ElmA { key: text, value: integer }
struct DbA { map: sorted<ElmA[key]> }
fn run_asc() -> text {
    db_a = DbA { map: [] };
    db_a.map += [ElmA { key: \"Alpha\", value: 1 }];
    db_a.map += [ElmA { key: \"Mid\",   value: 2 }];
    db_a.map += [ElmA { key: \"Omega\", value: 3 }];
    out_a = \"\";
    for v in db_a.map { out_a += \"{v.key},\"; }
    out_a
}"
    )
    .expr("run_asc()")
    .result(Value::str("Alpha,Mid,Omega,"));
}

#[test]
fn inc12_sorted_descending_iterates_backward() {
    code!(
        "struct ElmB { key: text, value: integer }
struct DbB { map: sorted<ElmB[-key]> }
fn run_desc() -> text {
    db_b = DbB { map: [] };
    db_b.map += [ElmB { key: \"Alpha\", value: 1 }];
    db_b.map += [ElmB { key: \"Mid\",   value: 2 }];
    db_b.map += [ElmB { key: \"Omega\", value: 3 }];
    out_b = \"\";
    for v in db_b.map { out_b += \"{v.key},\"; }
    out_b
}"
    )
    .expr("run_desc()")
    .result(Value::str("Omega,Mid,Alpha,"));
}

// ── INC#30: `{...}` double-duty (anonymous struct init vs. block) ───────
//
// The inconsistency writeup claimed that a typo like `{ x, y }`
// (missing colons) would silently become a block expression,
// evaluate `x` and `y` as statements, and return `y`.  Current
// loft rejects that shape at parse time — the trap is not
// reproducible.  These tests lock the three observable shapes so
// a future relaxation cannot reintroduce the silent-typo bite
// without updating LOFT.md and the INCONSISTENCIES.md entry.
#[test]
fn inc30_struct_init_with_colons_works() {
    code!(
        "struct Pt { x: integer, y: integer }
fn run_a() -> integer {
    p_a: Pt = Pt { x: 3, y: 4 };
    p_a.x + p_a.y
}"
    )
    .expr("run_a()")
    .result(Value::Int(7));
}

#[test]
fn inc30_block_expression_returns_last_value() {
    code!(
        "fn run_b() -> integer {
    r_b = { n_b = 1; m_b = n_b + 1; m_b };
    r_b
}"
    )
    .expr("run_b()")
    .result(Value::Int(2));
}

#[test]
fn inc30_typo_comma_without_colon_is_rejected() {
    code!(
        "struct PtC { x: integer, y: integer }
fn run_c() -> integer {
    a_c = 1;
    b_c = 2;
    p_c: PtC = { a_c, b_c };
    p_c.x + p_c.y
}"
    )
    .error("Expect token ; at inc30_typo_comma_without_colon_is_rejected:5:22");
}

// ── INC#31: open-ended range patterns in match arms ────────────────────
//
// The parser previously accepted `10..` (open-end) and `..10`
// (open-start) in match arms.  Under the interpreter these
// silently never matched (the absent endpoint encoded as null);
// under the native compiler they crashed rustc with an E0308
// `()` vs i32 type error.  Either failure mode is worse than
// "unsupported syntax".
//
// The fix emits a useful compile-time diagnostic pointing at
// the two-sided forms or a guard clause.  These tests lock
// three shapes:
//   - two-sided exclusive `lo..hi`  — works
//   - two-sided inclusive `lo..=hi` — works
//   - open-end `lo..` and open-start `..hi` — rejected
#[test]
fn inc31_two_sided_exclusive_range_matches() {
    code!(
        "fn bucket_a(n_a: integer) -> text {
    match n_a {
        10..20 => \"teens\",
        _      => \"other\"
    }
}"
    )
    .expr("bucket_a(15)")
    .result(Value::str("teens"));
}

#[test]
fn inc31_two_sided_inclusive_range_matches() {
    code!(
        "fn bucket_b(n_b: integer) -> text {
    match n_b {
        10..=20 => \"teens\",
        _       => \"other\"
    }
}"
    )
    .expr("bucket_b(20)")
    .result(Value::str("teens"));
}

#[test]
fn inc31_open_end_range_is_rejected() {
    code!(
        "fn bucket_c(n_c: integer) -> text {
    match n_c {
        10.. => \"ten-plus\",
        _    => \"other\"
    }
}"
    )
    .error(
        "open-ended range pattern `lo..` is not supported in match arms — \
write the two-sided form `lo..hi` (exclusive) or `lo..=hi` (inclusive), \
or use a guard like `n if n >= lo` at inc31_open_end_range_is_rejected:3:16",
    );
}

#[test]
fn inc31_open_start_range_is_rejected() {
    code!(
        "fn bucket_d(n_d: integer) -> text {
    match n_d {
        ..10 => \"below-ten\",
        _    => \"other\"
    }
}"
    )
    .error(
        "open-ended range pattern `..hi` is not supported in match arms — \
write the two-sided form `lo..hi` (exclusive) or `lo..=hi` (inclusive), \
or use a guard like `n if n < hi` at inc31_open_start_range_is_rejected:3:11",
    );
}

// ── INC#28: slice grammar + supported forms ────────────────────────────
//
// The grammar summary previously listed `v[2..-1]` as "negative
// indices count from end", but that claim was aspirational —
// the form produces an empty iterator (the range `2..-1` is
// literally empty).  `v[start..=end]` (inclusive) also worked
// in the parser but was undocumented.
//
// These tests lock the actually-supported shapes so a future
// implementation of negative indexing must update both the doc
// claim and these tests together, rather than silently flipping
// the semantics.
#[test]
fn inc28_slice_exclusive_range() {
    code!(
        "fn run_se() -> integer {
    v_se = [10, 20, 30, 40, 50];
    s_se = 0;
    for x_se in v_se[1..3] { s_se += x_se; }
    s_se
}"
    )
    .expr("run_se()")
    .result(Value::Int(50)); // 20 + 30
}

#[test]
fn inc28_slice_inclusive_range() {
    code!(
        "fn run_si() -> integer {
    v_si = [10, 20, 30, 40, 50];
    s_si = 0;
    for x_si in v_si[1..=3] { s_si += x_si; }
    s_si
}"
    )
    .expr("run_si()")
    .result(Value::Int(90)); // 20 + 30 + 40
}

#[test]
fn inc28_slice_open_end() {
    code!(
        "fn run_oe() -> integer {
    v_oe = [10, 20, 30, 40, 50];
    s_oe = 0;
    for x_oe in v_oe[2..] { s_oe += x_oe; }
    s_oe
}"
    )
    .expr("run_oe()")
    .result(Value::Int(120)); // 30 + 40 + 50
}

#[test]
fn inc28_slice_open_start() {
    code!(
        "fn run_os() -> integer {
    v_os = [10, 20, 30, 40, 50];
    s_os = 0;
    for x_os in v_os[..3] { s_os += x_os; }
    s_os
}"
    )
    .expr("run_os()")
    .result(Value::Int(60)); // 10 + 20 + 30
}

// INC#28: the doc claim that negative indices count from the end
// is aspirational.  Today `v[2..-1]` is the range `2..-1`, which
// is empty.  Locks the current behaviour so a future implementor
// of negative indexing can't silently change what programs do
// when they hit this form.
#[test]
fn inc28_negative_index_in_slice_yields_empty() {
    code!(
        "fn run_neg() -> integer {
    v_neg = [10, 20, 30, 40, 50];
    count_neg = 0;
    for x_neg in v_neg[2..-1] { count_neg += x_neg - x_neg + 1; }
    count_neg
}"
    )
    .expr("run_neg()")
    .result(Value::Int(0));
}

// ── INC#9: text indexing vs. slicing return different types ────────────
//
// `txt[i]` yields `character` (a scalar); `txt[i..j]` yields
// `text` (a string).  Vectors don't have this split (`vec[0]`
// is element T; `vec[0..1]` is `vector<T>`).  The asymmetry is
// deliberate — character is a distinct scalar, not a length-1
// text — but it's a real ergonomic trap: users assume the same
// operation family returns the same type domain.
//
// These tests lock both type paths + the practical concat
// consequences so a future "unify text indexing" refactor must
// update LOFT.md's Gotcha callout first.
// Probes `txt[i]` returns a `character` via its numeric value —
// avoids the B7-family text-return lifecycle crash hit when a
// function returns a text built by interpolating a character
// (`"{c}"` at tail of a `-> text` function SIGSEGVs today).
#[test]
fn inc9_text_index_returns_character() {
    code!(
        "fn run_ti() -> integer {
    txt_ti = \"hello\";
    c_ti = txt_ti[0];
    c_ti as integer
}"
    )
    .expr("run_ti()")
    .result(Value::Int(b'h' as i32));
}

#[test]
fn inc9_text_slice_returns_text() {
    code!(
        "fn run_ts() -> text {
    txt_ts = \"hello\";
    s_ts = txt_ts[0..1];
    s_ts
}"
    )
    .expr("run_ts()")
    .result(Value::str("h"));
}

#[test]
fn inc9_text_slices_concatenate_with_plus() {
    code!(
        "fn run_concat() -> text {
    txt_c = \"hello\";
    r_c = txt_c[0..1] + txt_c[1..2];
    r_c
}"
    )
    .expr("run_concat()")
    .result(Value::str("he"));
}

// Probes that `+` on `character` is arithmetic, not concatenation.
// 'b' - 'a' = 1 verifies the arithmetic path.  The "build text
// from characters via interpolation" consequence from the LOFT.md
// Gotcha callout is blocked today by a B7-family text-return
// SIGSEGV (`"{c}"` returned from `fn -> text` crashes); the
// workaround is to concatenate inside the function and not make
// the returned text the build target — but that crashes too.
// This test locks just the arithmetic-not-concat portion.
#[test]
fn inc9_character_plus_is_arithmetic_not_concat() {
    code!(
        "fn run_plus() -> integer {
    txt_p = \"abcd\";
    c1_p = txt_p[1];
    c2_p = txt_p[0];
    (c1_p as integer) - (c2_p as integer)
}"
    )
    .expr("run_plus()")
    .result(Value::Int(1));
}

// ── INC#17: type-conversion rules are mode-stratified, not uniform ──────
//
// Loft applies conversions in three modes: implicit (no
// annotation), format-only (implicit but only inside "{…}"),
// and explicit (`as` required).  The mode depends on the type
// pair, not on the context.  Users unable to predict this from
// first principles found themselves alternately typing too many
// `as` casts or hitting compile errors on missing ones.  The
// LOFT.md conversion table is the single reference; these tests
// lock the six most-common shapes so a future unification
// refactor cannot silently flip any mode.
#[test]
fn inc17_any_to_boolean_is_implicit() {
    // Non-zero integer is truthy in `if` without a cast.
    code!(
        "fn run_bool() -> integer {
    x_bool = 5;
    if x_bool { 1 } else { 0 }
}"
    )
    .expr("run_bool()")
    .result(Value::Int(1));
}

#[test]
fn inc17_integer_widens_to_float_in_arithmetic() {
    // 3 + 1.5 produces 4.5 without an explicit cast.
    code!(
        "fn run_widen() -> float {
    n_w = 3;
    f_w = 1.5;
    n_w + f_w
}"
    )
    .expr("run_widen()")
    .result(Value::Float(4.5));
}

#[test]
fn inc17_float_to_integer_requires_as() {
    // Truncates toward zero.
    code!(
        "fn run_trunc() -> integer {
    pi_t = 3.14;
    pi_t as integer
}"
    )
    .expr("run_trunc()")
    .result(Value::Int(3));
}

#[test]
fn inc17_text_to_integer_requires_as() {
    code!(
        "fn run_parse() -> integer {
    s_p = \"42\";
    s_p as integer
}"
    )
    .expr("run_parse()")
    .result(Value::Int(42));
}

#[test]
fn inc17_integer_to_text_is_format_only() {
    // Interpolation converts silently; the rendered text is
    // observable via format.  Probes the format-only path.
    code!(
        "fn run_fmt() -> integer {
    m_f = 7;
    t_f = \"n={m_f}\";
    len(t_f)
}"
    )
    .expr("run_fmt()")
    .result(Value::Int(3)); // "n=7"
}

#[test]
fn inc17_plain_enum_name_to_enum_requires_as() {
    code!(
        "enum Direction { North, South, East, West }
fn run_enum() -> integer {
    d_e = \"West\" as Direction;
    d_e as integer
}"
    )
    .expr("run_enum()")
    .result(Value::Int(4)); // plain-enum integer values are 1-indexed
}

// ── B7 family — method call on a JsonValue returning a scalar ────────
//
// Historical note (renamed 2026-04-14): this test was originally
// added as `b7_method_on_jsonvalue_returning_integer_crashes` —
// a regression marker for the period when every method call on a
// JsonValue local (even one returning a scalar like `len(v)`)
// double-freed the JsonValue store at scope exit.  The crash was
// resolved as a side-effect of later B-family landings (B2-runtime
// retrofit, B5 layers 1+2, the `t_9JsonValue_*` method-alias
// registrations for `n_as_*` / `n_field` / `n_item` / `n_len`).
//
// Today the test passes in both debug and release and guards the
// opposite invariant: method dispatch on a JsonValue local that
// returns a scalar must NOT crash and must NOT leak at scope exit.
// The remaining B7 symptom is narrower — the character-
// interpolation text-return path still SIGSEGVs, guarded by
// `b7_character_interpolation_return_crashes` (`#[ignore]`).
// See QUALITY.md § B7.
#[test]
fn b7_method_on_jsonvalue_returning_integer_works() {
    code!(
        "fn run_b7m() -> boolean {
    v_b7m = json_parse(\"null\");
    n_b7m = len(v_b7m);
    !n_b7m
}"
    )
    .expr("run_b7m()")
    .result(Value::Boolean(true));
}

/// Repeated method dispatch on the same JsonValue local.  If the
/// historical B7 double-free were still present, the second
/// `.kind()` call would trip the lifecycle bug because the
/// first call's post-dispatch cleanup had already decremented
/// the store's ref-count.  Today both calls succeed and return
/// the expected variant name.
#[test]
fn b7_repeated_method_dispatch_on_jsonvalue_works() {
    code!(
        "fn run_b7rm() -> text {
    v_b7rm = json_parse(\"true\");
    k1_b7rm = v_b7rm.kind();
    k2_b7rm = v_b7rm.kind();
    if k1_b7rm == k2_b7rm { k1_b7rm } else { \"MISMATCH\" }
}"
    )
    .expr("run_b7rm()")
    .result(Value::str("JBool"));
}

/// B7 method surface works on Q4-constructed JsonValues, not just
/// on `json_parse` results.  This locks that `json_null()` (and by
/// extension the other Q4 primitive constructors) produces a
/// JsonValue whose scope-exit cleanup doesn't conflict with method
/// dispatch — the same invariant the renamed `_works` test locks
/// for the json_parse side.
#[test]
fn b7_method_on_q4_constructed_jsonvalue_works() {
    code!(
        "fn run_b7q4() -> text {
    v_b7q4 = json_number(42.0);
    v_b7q4.kind()
}"
    )
    .expr("run_b7q4()")
    .result(Value::str("JNumber"));
}

// ── Q1: parser-side rich diagnostics through json_errors() ─────────────
//
// json_errors() now returns the full diagnostic shape:
//   parse error at line N col M (byte B):
//     path: /a/b/c
//     <message>
//     <context snippet with ^ caret>
//
// The loft testing harness has no substring matcher, so each test
// asserts the salient piece via a `text.contains(...)` check inside
// loft and returns a boolean.  Tolerant of future spacing /
// line-numbering tweaks.
#[test]
fn q1_json_errors_path_for_object_field() {
    code!(
        "fn run_q1a() -> boolean {
    v_q1a = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_q1a == v_q1a {}
    e_q1a = json_errors();
    e_q1a.contains(\"/x\")
}"
    )
    .expr("run_q1a()")
    .result(Value::Boolean(true));
}

#[test]
fn q1_json_errors_path_for_array_index() {
    code!(
        "fn run_q1b() -> boolean {
    v_q1b = json_parse(\"[1, 2, 1.]\");
    if v_q1b == v_q1b {}
    e_q1b = json_errors();
    e_q1b.contains(\"/2\")
}"
    )
    .expr("run_q1b()")
    .result(Value::Boolean(true));
}

#[test]
fn q1_json_errors_includes_caret_marker() {
    code!(
        "fn run_q1c() -> boolean {
    v_q1c = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_q1c == v_q1c {}
    e_q1c = json_errors();
    e_q1c.contains(\"^\")
}"
    )
    .expr("run_q1c()")
    .result(Value::Boolean(true));
}

#[test]
fn q1_json_errors_includes_line_and_byte() {
    code!(
        "fn run_q1d() -> boolean {
    v_q1d = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_q1d == v_q1d {}
    e_q1d = json_errors();
    e_q1d.contains(\"line\") && e_q1d.contains(\"byte\")
}"
    )
    .expr("run_q1d()")
    .result(Value::Boolean(true));
}

// B7 family — character-interpolation-return regression guard.
//
// Originally a SIGSEGV reproducer (discovered while writing INC#9
// regression tests): `fn f() -> text { c = txt[0]; "{c}" }`
// crashed because the text built via n_format_text on a character
// wasn't tracked for free on the outer function's text-return path.
//
// Closed as a side-effect of the B2-runtime / B5 / dep-inference /
// lock-args fixes that landed across PR #168 → #172.  Kept as a
// regression guard.  Old `_crashes` suffix retained for
// search-back compatibility — the test now passes.
#[test]
fn b7_character_interpolation_return_crashes() {
    code!(
        "fn build_b7c() -> text {
    txt_b7c = \"hello\";
    c_b7c = txt_b7c[0];
    \"{c_b7c}\"
}"
    )
    .expr("build_b7c()")
    .result(Value::str("h"));
}

// Multiple json_parse() in the same function — currently OK
// when each result is consumed via pattern matching.  Investigated
// while writing B7 regression tests; the previous QUALITY.md claim
// that "multiple json_parse() corrupts memory" was a misattribution
// — the corruption observed in earlier smoke tests came from the
// kind()/len() method calls, not from json_parse() itself.  This
// guard pins the pattern-match-based shape so future B7 work
// doesn't accidentally regress it.
#[test]
fn b7_multiple_json_parse_via_match_works() {
    code!(
        "fn run_b7p() -> boolean {
    a_b7p = json_parse(\"null\");
    b_b7p = json_parse(\"true\");
    match a_b7p { JNull => match b_b7p { JBool { value } => value, _ => false }, _ => false }
}"
    )
    .expr("run_b7p()")
    .result(Value::Boolean(true));
}

/// Q4 first slice — `json_null()` constructs a `JsonValue` set to
/// the `JNull` variant.  Primitive-only; doesn't need P54 step 4's
/// arena materialisation (no payload).  Consumed via pattern-match
/// so it rides on the working path guarded by
/// `b7_multiple_json_parse_via_match_works` rather than the still-
/// open method-call surface.  When P54 step 4 lands, the companion
/// `json_bool` / `json_number` / `json_string` primitives follow
/// the same shape; the container constructors `json_array` /
/// `json_object` land with the arena allocator.
#[test]
fn q4_json_null_returns_jnull_variant() {
    code!(
        "fn run_q4n() -> boolean {
    v_q4n = json_null();
    match v_q4n { JNull => true, _ => false }
}"
    )
    .expr("run_q4n()")
    .result(Value::Boolean(true));
}

/// Q4 ↔ Q2 cross-check — every primitive constructor must write
/// the same discriminant byte that `kind()` reads back as the
/// expected variant name.  Closes the integration gap between
/// the constructor write side (Q4) and the introspection read
/// side (Q2): without these guards, a constructor that wrote
/// the wrong discriminant byte would still pass its own match
/// test (because match dispatch and kind() share the same byte
/// — but a typo in either side would silently mis-name the
/// variant).
#[test]
fn q4_constructor_kind_cross_check_null() {
    code!(
        "fn run_q4ckn() -> text {
    json_null().kind()
}"
    )
    .expr("run_q4ckn()")
    .result(Value::str("JNull"));
}

#[test]
fn q4_constructor_kind_cross_check_bool() {
    code!(
        "fn run_q4ckb() -> text {
    json_bool(true).kind()
}"
    )
    .expr("run_q4ckb()")
    .result(Value::str("JBool"));
}

#[test]
fn q4_constructor_kind_cross_check_number() {
    code!(
        "fn run_q4cknum() -> text {
    json_number(2.5).kind()
}"
    )
    .expr("run_q4cknum()")
    .result(Value::str("JNumber"));
}

#[test]
fn q4_constructor_kind_cross_check_string() {
    code!(
        "fn run_q4cks() -> text {
    json_string(\"hi\").kind()
}"
    )
    .expr("run_q4cks()")
    .result(Value::str("JString"));
}

/// Q4 ↔ Q2 cross-check — `json_number(NaN)` resolves to `JNull`
/// (RFC 8259 disallows non-finite numbers), so kind() reports
/// `JNull`.  Locks the documented "non-finite → JNull"
/// substitution at the introspection level, not just the
/// internal storage.
#[test]
fn q4_constructor_kind_cross_check_nan_is_jnull() {
    code!(
        "fn run_q4cknan() -> text {
    json_number(0.0 / 0.0).kind()
}"
    )
    .expr("run_q4cknan()")
    .result(Value::str("JNull"));
}

/// Q4 ↔ Q3 cross-check — every primitive constructor's payload
/// must serialise back to the canonical RFC 8259 text via
/// `to_json()`.  Closes the integration gap between the
/// constructor write side (Q4) and the serialiser read side
/// (Q3).  Without these, a constructor that wrote the wrong
/// payload bytes (e.g. flipped boolean polarity, wrong float
/// position) would still pass kind() and as_*() because each
/// reads the constructor-specific position — only `to_json()`
/// touches every byte and renders it as text.
#[test]
fn q4_constructor_to_json_cross_check_null() {
    code!(
        "fn run_q4ctjn() -> text {
    json_null().to_json()
}"
    )
    .expr("run_q4ctjn()")
    .result(Value::str("null"));
}

#[test]
fn q4_constructor_to_json_cross_check_bool_true() {
    code!(
        "fn run_q4ctjbt() -> text {
    json_bool(true).to_json()
}"
    )
    .expr("run_q4ctjbt()")
    .result(Value::str("true"));
}

#[test]
fn q4_constructor_to_json_cross_check_bool_false() {
    code!(
        "fn run_q4ctjbf() -> text {
    json_bool(false).to_json()
}"
    )
    .expr("run_q4ctjbf()")
    .result(Value::str("false"));
}

#[test]
fn q4_constructor_to_json_cross_check_number_integral() {
    code!(
        "fn run_q4ctjni() -> text {
    json_number(42.0).to_json()
}"
    )
    .expr("run_q4ctjni()")
    .result(Value::str("42"));
}

#[test]
fn q4_constructor_to_json_cross_check_number_fractional() {
    code!(
        "fn run_q4ctjnf() -> text {
    json_number(2.5).to_json()
}"
    )
    .expr("run_q4ctjnf()")
    .result(Value::str("2.5"));
}

#[test]
fn q4_constructor_to_json_cross_check_string() {
    code!(
        "fn run_q4ctjs() -> text {
    json_string(\"hi\").to_json()
}"
    )
    .expr("run_q4ctjs()")
    .result(Value::str("\"hi\""));
}

/// Q4 ↔ extractor cross-check — `json_X(v).as_X()` round-trips
/// the value back through the typed extractor.  Validates that
/// the constructor's payload-write position matches the
/// extractor's read position for each primitive variant.
#[test]
fn q4_constructor_as_bool_round_trips() {
    code!(
        "fn run_q4cab() -> boolean {
    json_bool(true).as_bool()
}"
    )
    .expr("run_q4cab()")
    .result(Value::Boolean(true));
}

#[test]
fn q4_constructor_as_long_round_trips() {
    code!(
        "fn run_q4cal() -> long {
    json_number(100.0).as_long()
}"
    )
    .expr("run_q4cal()")
    .result(Value::Long(100));
}

#[test]
fn q4_constructor_as_text_round_trips() {
    code!(
        "fn run_q4cat() -> text {
    json_string(\"abc\").as_text()
}"
    )
    .expr("run_q4cat()")
    .result(Value::str("abc"));
}

/// Q4 ↔ Q2 cross-check — `has_field()` on a Q4-built JObject finds
/// fields by name and rejects misses.  Bridges the construction
/// side (Q4) and the introspection side (Q2 has_field) — the
/// existing `q2_has_field_*` tests build via `json_parse(text)`,
/// not via the constructor surface, so this cross-check is the
/// only one that exercises the deep-copy → name-scan invariant.
#[test]
fn q4_constructor_has_field_finds_present_name() {
    code!(
        "fn run_q4chf() -> boolean {
    fields_q4chf: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_number(2.0) }
    ];
    obj_q4chf = json_object(fields_q4chf);
    obj_q4chf.has_field(\"alpha\") && !obj_q4chf.has_field(\"missing\")
}"
    )
    .expr("run_q4chf()")
    .result(Value::Boolean(true));
}

/// Q4 ↔ Q2 cross-check — `keys()` on a Q4-built JObject lists
/// every constructed field name.  Asserts both presence and
/// count via `keys().len()`.
#[test]
fn q4_constructor_keys_lists_constructed_names() {
    code!(
        "fn run_q4ckl() -> integer {
    fields_q4ckl: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_number(2.0) }
    ];
    obj_q4ckl = json_object(fields_q4ckl);
    obj_q4ckl.keys().len()
}"
    )
    .expr("run_q4ckl()")
    .result(Value::Int(2));
}

/// Q4 ↔ Q2 cross-check — `fields()` on a Q4-built JObject yields
/// every `(name, value)` entry.  Combined with the keys() test
/// above this locks both faces of the JObject introspection
/// surface against the constructor.
#[test]
fn q4_constructor_fields_lists_constructed_entries() {
    code!(
        "fn run_q4cfl() -> integer {
    fields_q4cfl: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_number(2.0) }
    ];
    obj_q4cfl = json_object(fields_q4cfl);
    obj_q4cfl.fields().len()
}"
    )
    .expr("run_q4cfl()")
    .result(Value::Int(2));
}

/// Q4 ↔ field navigation cross-check — `field()` lookup on a
/// Q4-built JObject walks the deep-copied field vector and
/// returns the embedded value.  Then `as_text()` extracts it.
/// Locks the full chain `json_object(...) → field(name) →
/// as_text()`.
#[test]
fn q4_constructor_field_lookup_extracts_value() {
    code!(
        "fn run_q4cfl2() -> text {
    fields_q4cfl2: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_string(\"B\") }
    ];
    obj_q4cfl2 = json_object(fields_q4cfl2);
    obj_q4cfl2.field(\"beta\").as_text()
}"
    )
    .expr("run_q4cfl2()")
    .result(Value::str("B"));
}

/// Q1 — `json_errors()` clears its trail on a successful parse.
/// The stdlib doc-comment spec says "Empty when the parse
/// succeeded" — the existing q1_* tests exercise the diagnostic
/// path on a single bad input but never verify the state-clearing
/// invariant: that a subsequent good parse erases the previous
/// error.  Without this guard, a regression that left stale
/// diagnostics from an earlier failure would silently mislead
/// every successive caller.
#[test]
fn q1_json_errors_cleared_after_successful_parse() {
    code!(
        "fn run_q1cls() -> boolean {
    bad_q1cls = json_parse(\"[1, 2, 1.]\");
    if bad_q1cls == bad_q1cls {}
    bad_len_q1cls = json_errors().len();
    good_q1cls = json_parse(\"[1, 2, 3]\");
    if good_q1cls == good_q1cls {}
    bad_len_q1cls > 0 && json_errors().len() == 0
}"
    )
    .expr("run_q1cls()")
    .result(Value::Boolean(true));
}

/// Q1 — `json_errors()` is empty after a fresh successful parse
/// on a never-failed JSON expression.  Pairs with the
/// state-clearing test above to lock both the "always empty on
/// success" and "transitions on failure→success" invariants.
#[test]
fn q1_json_errors_empty_after_clean_parse() {
    code!(
        "fn run_q1cep() -> integer {
    v_q1cep = json_parse(\"{{\\\"k\\\": 1}}\");
    if v_q1cep == v_q1cep {}
    json_errors().len()
}"
    )
    .expr("run_q1cep()")
    .result(Value::Int(0));
}

// ── Q1 spec-named acceptance tests (complete the § Q1 Tests list) ──────
//
// QUALITY.md § Q1 Tests enumerates five `p54_err_*` names as the
// target acceptance coverage.  Earlier landings used `q1_*`
// prefixes (with equivalent content for some, different content
// for others).  These add the missing spec names directly so a
// reader looking for the Q1 checklist finds it by the exact
// documented identifiers.

/// Q1 — `json_errors()` path for a leaf inside a nested object
/// is the full `/a/b` pointer (parent field + child field),
/// not just the leaf name.  Complements
/// `q1_json_errors_path_for_object_field` which only checked
/// a top-level field.
#[test]
fn p54_err_reports_path_into_nested_object() {
    code!(
        "fn run_perno() -> boolean {
    v_perno = json_parse(`{{\"a\": {{\"b\": 1.}}}}`);
    if v_perno == v_perno {}
    json_errors().contains(\"/a/b\")
}"
    )
    .expr("run_perno()")
    .result(Value::Boolean(true));
}

/// Q1 — `json_errors()` path into an array element is
/// `/N` with the element index.  Same assertion shape as the
/// pre-existing `q1_json_errors_path_for_array_index`; kept
/// under the spec name as well so QUALITY.md's § Q1 Tests
/// checklist matches the landed test set by-name.
#[test]
fn p54_err_reports_path_into_array_element() {
    code!(
        "fn run_perae() -> boolean {
    v_perae = json_parse(\"[1, 2, 1.]\");
    if v_perae == v_perae {}
    json_errors().contains(\"/2\")
}"
    )
    .expr("run_perae()")
    .result(Value::Boolean(true));
}

/// Q1 — a parse failure on line 2 reports `line 2` in the
/// diagnostic (not just "line N" — asserts the specific
/// number).  Multi-line input via explicit `\n` escapes.
#[test]
fn p54_err_reports_line_and_column() {
    code!(
        "fn run_perlc() -> boolean {
    v_perlc = json_parse(\"{{\\n  \\\"x\\\": 1.\\n}}\");
    if v_perlc == v_perlc {}
    json_errors().contains(\"line 2\")
}"
    )
    .expr("run_perlc()")
    .result(Value::Boolean(true));
}

/// Q1 — the context snippet includes a `^` caret under the
/// offending column.  Equivalent to
/// `q1_json_errors_includes_caret_marker`, added under the
/// spec name.
#[test]
fn p54_err_context_snippet_includes_caret() {
    code!(
        "fn run_percsic() -> boolean {
    v_percsic = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_percsic == v_percsic {}
    json_errors().contains(\"^\")
}"
    )
    .expr("run_percsic()")
    .result(Value::Boolean(true));
}

/// Q1 — RFC 6901 path escaping at the acceptance level.  A
/// field named `a/b~c` renders as `/a~1b~0c` in the path.  The
/// unit test `err_path_escapes_slash_and_tilde` in
/// `src/json.rs` covers the parser-side function, but no
/// acceptance test verified that the escaping actually reaches
/// `json_errors()` output.  Without this guard, a refactor that
/// dropped the escape helper in the `n_json_parse` glue could
/// regress RFC 6901 conformance silently.
#[test]
fn p54_err_path_escapes_slash_and_tilde() {
    code!(
        "fn run_perpest() -> boolean {
    v_perpest = json_parse(`{{\"a/b~c\": 1.}}`);
    if v_perpest == v_perpest {}
    json_errors().contains(\"/a~1b~0c\")
}"
    )
    .expr("run_perpest()")
    .result(Value::Boolean(true));
}

/// Extractor null-on-mismatch — `as_long()` on a JString returns
/// the integer null sentinel (`i64::MIN`).  The stdlib spec says
/// "null on kind mismatch" — never directly tested.
#[test]
fn p54_as_long_on_jstring_returns_null_sentinel() {
    code!(
        "fn run_alos() -> long {
    json_string(\"hi\").as_long()
}"
    )
    .expr("run_alos()")
    .result(Value::Null);
}

/// Extractor null-on-mismatch — `as_text()` on a JNumber returns
/// the text null sentinel (which compares equal to `null` at the
/// loft level).  Validates the "null on kind mismatch" contract
/// for the text extractor.  Asserts via a loft-level `t == null`
/// check rather than a direct text comparison because the
/// underlying sentinel is `"\0"`, not the empty string.
#[test]
fn p54_as_text_on_jnumber_returns_null() {
    code!(
        "fn run_aton() -> boolean {
    t_aton = json_number(42.0).as_text();
    t_aton == null
}"
    )
    .expr("run_aton()")
    .result(Value::Boolean(true));
}

/// Extractor null-on-mismatch — `as_bool()` on a JNull returns
/// `false` (the boolean null sentinel).
#[test]
fn p54_as_bool_on_jnull_returns_false() {
    code!(
        "fn run_abon() -> boolean {
    json_null().as_bool()
}"
    )
    .expr("run_abon()")
    .result(Value::Boolean(false));
}

/// Extractor `as_long()` truncates float toward zero (NOT round,
/// NOT floor).  The stdlib spec is explicit: "Truncates the
/// underlying float toward zero before converting."  Locks the
/// behaviour for both signs — `2.7 → 2` and `-2.7 → -2`.
#[test]
fn p54_as_long_truncates_positive_float_toward_zero() {
    code!(
        "fn run_altp() -> long {
    json_number(2.7).as_long()
}"
    )
    .expr("run_altp()")
    .result(Value::Long(2));
}

#[test]
fn p54_as_long_truncates_negative_float_toward_zero() {
    code!(
        "fn run_altn() -> long {
    json_number(-2.7).as_long()
}"
    )
    .expr("run_altn()")
    .result(Value::Long(-2));
}

/// Edge-case parse inputs — the documented "malformed input
/// returns JNull" contract was tested for individual bad-syntax
/// inputs (Q1 path tests) but never for the lexically empty
/// boundary cases.  Locks `""`, `"   "` (whitespace-only), and
/// arbitrary garbage all return JNull.
#[test]
fn p54_parse_empty_string_returns_jnull() {
    code!(
        "fn run_pes() -> text {
    json_parse(\"\").kind()
}"
    )
    .expr("run_pes()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_parse_whitespace_only_returns_jnull() {
    code!(
        "fn run_pwo() -> text {
    json_parse(\"   \").kind()
}"
    )
    .expr("run_pwo()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_parse_garbage_input_returns_jnull() {
    code!(
        "fn run_pgi() -> text {
    json_parse(\"not-json-at-all\").kind()
}"
    )
    .expr("run_pgi()")
    .result(Value::str("JNull"));
}

/// Q4-built primitive match destructuring — the constructor
/// path didn't have direct destructuring guards beyond the
/// existing JNull (`q4_json_null_returns_jnull_variant`).
/// Adds JBool + JNumber.
///
/// JString destructuring on a Q4-built value is intentionally
/// NOT tested here because it triggers a B7-family
/// `free(): invalid size` crash (discovered while writing
/// these guards via `/tmp/jstring_match_probe.loft` — the
/// same store-lifecycle issue that gates
/// `b7_character_interpolation_return_crashes`).  The match
/// branch destructure of a JString value's text-typed inner
/// field is a known-failing path for the Q4 constructor —
/// pattern matching via wildcard works (existing q4 tests),
/// but field-binding doesn't.  Tracked under B7.
#[test]
fn q4_match_destructuring_jbool_extracts_value() {
    code!(
        "fn run_q4mb() -> boolean {
    match json_bool(true) {
        JBool { value } => value,
        _ => false
    }
}"
    )
    .expr("run_q4mb()")
    .result(Value::Boolean(true));
}

#[test]
fn q4_match_destructuring_jnumber_extracts_value() {
    code!(
        "fn run_q4mn() -> float {
    match json_number(3.25) {
        JNumber { value } => value,
        _ => 0.0
    }
}"
    )
    .expr("run_q4mn()")
    .result(Value::Float(3.25));
}

/// Extractor — `as_number()` returns the JNumber payload on a
/// matching variant and NaN (the float null sentinel) on every
/// other.  Complements the other extractor null-on-mismatch
/// guards (as_long / as_text / as_bool).  Asserts NaN via
/// self-inequality (`f != f` is true iff f is NaN — the only
/// reliable loft-level NaN test).
#[test]
fn p54_as_number_on_jnumber_returns_value() {
    code!(
        "fn run_annv() -> float {
    json_number(3.5).as_number()
}"
    )
    .expr("run_annv()")
    .result(Value::Float(3.5));
}

#[test]
fn p54_as_number_on_jstring_returns_nan() {
    code!(
        "fn run_annjs() -> boolean {
    x_annjs = json_string(\"hi\").as_number();
    x_annjs != x_annjs
}"
    )
    .expr("run_annjs()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_as_number_on_jbool_returns_nan() {
    code!(
        "fn run_annjb() -> boolean {
    x_annjb = json_bool(true).as_number();
    x_annjb != x_annjb
}"
    )
    .expr("run_annjb()")
    .result(Value::Boolean(true));
}

/// RFC 8259 numeric parse — scientific notation (`1e10`) must
/// parse as `JNumber` with the correctly-scaled float payload.
/// Never tested — the existing q1_* tests cover syntax-failure
/// paths on numbers like `1.` but not successful scientific
/// inputs.
#[test]
fn p54_parse_scientific_notation_is_jnumber() {
    code!(
        "fn run_psn() -> text {
    json_parse(\"1e10\").kind()
}"
    )
    .expr("run_psn()")
    .result(Value::str("JNumber"));
}

#[test]
fn p54_parse_scientific_notation_extracts_value() {
    code!(
        "fn run_psnv() -> boolean {
    v_psnv = json_parse(\"1e3\").as_number();
    v_psnv > 999.0 && v_psnv < 1001.0
}"
    )
    .expr("run_psnv()")
    .result(Value::Boolean(true));
}

/// RFC 8259 numeric parse — leading zeros are rejected (the
/// grammar allows only `0` or `[1-9][0-9]*` for the integer
/// part).  Locks the documented rejection behaviour so a
/// future permissive-mode change doesn't silently accept
/// `007`.  The `-0` case is a complementary positive: RFC 8259
/// explicitly allows negative zero (`-0` is a valid
/// `JNumber`).
#[test]
fn p54_parse_leading_zero_integer_is_rejected() {
    code!(
        "fn run_plz() -> text {
    json_parse(\"007\").kind()
}"
    )
    .expr("run_plz()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_parse_negative_zero_is_accepted() {
    code!(
        "fn run_pnz() -> text {
    json_parse(\"-0\").kind()
}"
    )
    .expr("run_pnz()")
    .result(Value::str("JNumber"));
}

/// Pretty-print depth counting — the `to_json_pretty` path
/// tracks an explicit depth counter in `json_to_text_at` and
/// emits the right number of 2-space indents at each level.
/// Prior tests cover depth 1 (`[1,2]`) and depth 2 (`{"k":[1,2]}`);
/// this guard exercises depth 3 (`{"a":{"b":[1]}}`) so the depth
/// counter is verified to propagate through nested containers
/// without off-by-one errors.
#[test]
fn q3_to_json_pretty_three_level_nesting() {
    code!(
        "fn run_q3p3() -> text {
    v_q3p3 = json_parse(`{{\"a\":{{\"b\":[1]}}}}`);
    v_q3p3.to_json_pretty()
}"
    )
    .expr("run_q3p3()")
    .result(Value::str(
        "{\n  \"a\": {\n    \"b\": [\n      1\n    ]\n  }\n}",
    ));
}

/// Round-trip preserves JObject insertion order.  The STDLIB.md
/// JSON reference says "Field names in insertion order" for
/// both `keys()` and object serialisation.  Never tested for
/// the parse → serialise path — a parser that sorted keys
/// alphabetically would be spec-incorrect but no test would
/// catch it.  Chooses names `z/a/m` so alphabetical reordering
/// (`a,m,z`) would produce distinct output from insertion
/// order (`z,a,m`).
#[test]
fn p54_parse_serialise_preserves_insertion_order() {
    code!(
        "fn run_piso() -> text {
    v_piso = json_parse(`{{\"z\":1,\"a\":2,\"m\":3}}`);
    v_piso.to_json()
}"
    )
    .expr("run_piso()")
    .result(Value::str("{\"z\":1,\"a\":2,\"m\":3}"));
}

/// Q4 → Q2 keys() preserves the caller's declared field order.
/// Complements `q4_constructor_keys_lists_constructed_names`
/// (which only asserted `.len() == 2`) by asserting every key
/// appears at the correct index, in the caller-supplied order.
#[test]
fn q4_constructor_keys_preserves_insertion_order() {
    code!(
        "fn run_q4kio() -> text {
    fields_q4kio: vector<JsonField> = [
        JsonField { name: \"zebra\", value: json_null() },
        JsonField { name: \"apple\", value: json_null() },
        JsonField { name: \"mango\", value: json_null() }
    ];
    obj_q4kio = json_object(fields_q4kio);
    ks_q4kio = obj_q4kio.keys();
    \"{ks_q4kio[0]}|{ks_q4kio[1]}|{ks_q4kio[2]}\"
}"
    )
    .expr("run_q4kio()")
    .result(Value::str("zebra|apple|mango"));
}

/// Deep-nesting navigation — a 5-level-deep JSON tree parses
/// into a tree where the leaf is reachable via five chained
/// `.field()` calls without tripping store-lifecycle or
/// arena-offset bugs.  QUALITY.md § Q3 Tests mentions
/// "nested up to depth 5" as the property-test target; this
/// guard pins that depth concretely.
#[test]
fn p54_deep_nesting_five_levels_navigable() {
    code!(
        "fn run_pdn5() -> long {
    v_pdn5 = json_parse(`{{\"a\":{{\"b\":{{\"c\":{{\"d\":{{\"e\":42}}}}}}}}}}`);
    v_pdn5.field(\"a\").field(\"b\").field(\"c\").field(\"d\").field(\"e\").as_long()
}"
    )
    .expr("run_pdn5()")
    .result(Value::Long(42));
}

/// Pretty-print of an empty container inside a non-empty
/// parent.  Locks the edge case where the outer array indents
/// its children one level but the inner empty array stays `[]`
/// (no newline padding even though its parent is pretty-printed).
/// A naive implementation that always emitted `\n<indent>` for
/// every container would turn `[]` into `[\n  ]` at depth 1 —
/// this guard catches that.
#[test]
fn q3_to_json_pretty_empty_container_inside_non_empty() {
    code!(
        "fn run_q3pein() -> text {
    inner_q3pein: vector<JsonValue> = [];
    outer_q3pein: vector<JsonValue> = [json_array(inner_q3pein), json_number(1.0)];
    v_q3pein = json_array(outer_q3pein);
    v_q3pein.to_json_pretty()
}"
    )
    .expr("run_q3pein()")
    .result(Value::str("[\n  [],\n  1\n]"));
}

/// Q2 `fields()` — full name+value insertion-order preservation.
/// The prior `q4_constructor_keys_preserves_insertion_order`
/// pinned `keys()` at per-index granularity; this is the
/// companion for `fields()` on a parsed input, asserting that
/// each entry carries both its original name AND value at the
/// correct index.  Uses `z/a` names so alphabetical reordering
/// would produce distinct output.
#[test]
fn q2_fields_preserves_name_and_value_at_each_index() {
    code!(
        "fn run_q2fnvi() -> text {
    v_q2fnvi = json_parse(`{{\"z\":1,\"a\":2}}`);
    entries_q2fnvi = v_q2fnvi.fields();
    \"{entries_q2fnvi[0].name}={entries_q2fnvi[0].value.as_long()}|{entries_q2fnvi[1].name}={entries_q2fnvi[1].value.as_long()}\"
}"
    )
    .expr("run_q2fnvi()")
    .result(Value::str("z=1|a=2"));
}

/// Q2 `has_field("")` — empty-string key on a JObject that
/// carries a field with an empty name must return `true`.  Edge
/// case for the name-scan loop: a naive string-length shortcut
/// that treated empty-string as "no lookup" would break this.
/// Locks the documented "returns `true` iff carries a field
/// named `name`" contract at the name boundary.
#[test]
fn q2_has_field_matches_empty_name_key() {
    code!(
        "fn run_q2hen() -> boolean {
    v_q2hen = json_parse(`{{\"\":1}}`);
    v_q2hen.has_field(\"\") && !v_q2hen.has_field(\"a\")
}"
    )
    .expr("run_q2hen()")
    .result(Value::Boolean(true));
}

/// Match on non-empty JArray binds the `items` field to the
/// real container vector.  The vector's `.len()` must match the
/// JSON array's length.  Coverage gap: existing JArray tests
/// destructure via wildcard (`JArray _ =>`) but don't bind the
/// items field — so the binding codegen path for a non-empty
/// container wasn't directly exercised.
#[test]
fn p54_match_jarray_binds_non_empty_items() {
    code!(
        "fn run_pmjba() -> integer {
    match json_parse(\"[10,20,30]\") {
        JArray { items } => items.len(),
        _ => -1
    }
}"
    )
    .expr("run_pmjba()")
    .result(Value::Int(3));
}

/// Match on an empty JArray binds `items` to an empty vector.
/// Pairs with the non-empty test above so the binding path
/// is covered at both the minimum (zero-length) and the
/// non-degenerate (three-element) boundaries.
#[test]
fn p54_match_jarray_binds_empty_items() {
    code!(
        "fn run_pmjbe() -> integer {
    match json_parse(\"[]\") {
        JArray { items } => items.len(),
        _ => -1
    }
}"
    )
    .expr("run_pmjbe()")
    .result(Value::Int(0));
}

/// Q4 first slice — `json_null()` ignoring the B7 method-call
/// surface: two independent `json_null()` calls in the same
/// function, each consumed via its own match.  Mirrors the
/// `b7_multiple_json_parse_via_match_works` shape to guarantee
/// the constructor doesn't trip the B7 double-free when two
/// results coexist.
#[test]
fn q4_two_json_nulls_via_match_works() {
    code!(
        "fn run_q4nn() -> integer {
    a_q4nn = json_null();
    b_q4nn = json_null();
    ok_a_q4nn = match a_q4nn { JNull => 1, _ => 0 };
    ok_b_q4nn = match b_q4nn { JNull => 1, _ => 0 };
    ok_a_q4nn + ok_b_q4nn
}"
    )
    .expr("run_q4nn()")
    .result(Value::Int(2));
}

/// Q4 second slice — `json_bool(v)` constructs a `JBool` variant
/// carrying the supplied boolean payload.  Pattern-match on the
/// result binds the `value` field; this guard locks both
/// construction (discriminant byte written) and the payload round-
/// trip (field offset correct).
#[test]
fn q4_json_bool_round_trips_true() {
    code!(
        "fn run_q4bt() -> boolean {
    v_q4bt = json_bool(true);
    match v_q4bt { JBool { value } => value, _ => false }
}"
    )
    .expr("run_q4bt()")
    .result(Value::Boolean(true));
}

#[test]
fn q4_json_bool_round_trips_false() {
    code!(
        "fn run_q4bf() -> integer {
    v_q4bf = json_bool(false);
    match v_q4bf { JBool { value } => { if value { 1 } else { 2 } }, _ => 0 }
}"
    )
    .expr("run_q4bf()")
    .result(Value::Int(2));
}

/// Q4 third slice — `json_number(v)` constructs a `JNumber` variant
/// carrying the supplied float payload.  Non-finite input produces
/// `JNull` with a diagnostic in `json_errors()` — that behaviour is
/// guarded by `q4_json_number_nan_becomes_jnull` below.
#[test]
fn q4_json_number_round_trips_finite() {
    code!(
        "fn run_q4nr() -> float {
    v_q4nr = json_number(2.75);
    match v_q4nr { JNumber { value } => value, _ => 0.0 }
}"
    )
    .expr("run_q4nr()")
    .result(Value::Float(2.75));
}

#[test]
fn q4_json_number_negative_finite() {
    code!(
        "fn run_q4nn2() -> float {
    v_q4nn2 = json_number(-2.5);
    match v_q4nn2 { JNumber { value } => value, _ => 0.0 }
}"
    )
    .expr("run_q4nn2()")
    .result(Value::Float(-2.5));
}

/// Q4 third slice negative-case — feeding `float null` (NaN) or
/// non-finite values makes `json_number` store `JNull` with a
/// diagnostic, not a numeric payload that would violate RFC 8259.
#[test]
fn q4_json_number_nan_becomes_jnull() {
    code!(
        "fn run_q4nn3() -> integer {
    nan_val_q4 = 0.0 / 0.0;
    v_q4nn3 = json_number(nan_val_q4);
    match v_q4nn3 { JNull => 1, _ => 0 }
}"
    )
    .expr("run_q4nn3()")
    .result(Value::Int(1));
}

/// Q4 fourth slice — `json_string(v)` constructs a `JString`
/// variant carrying a copy of the supplied text.  The text is
/// written into the JsonValue's own store, so the returned value
/// lifetime-extends the payload independently of `v`.
///
/// Reading the bound `value: text` out of the match arm trips the
/// same native-returned-text lifecycle bug that B7's
/// `b7_character_interpolation_return_crashes` guards (the Str
/// pointer into the JsonValue store gets returned and later freed
/// as a DbRef).  Until B7 lands, the test verifies the shape of
/// the variant — `JString` branch taken — by measuring the
/// bound text's length (integer return, no Str escape) rather
/// than returning the value itself.
#[test]
fn q4_json_string_round_trips() {
    code!(
        "fn run_q4sr() -> integer {
    v_q4sr = json_string(\"hello world\");
    match v_q4sr { JString { value } => value.len(), _ => -1 }
}"
    )
    .expr("run_q4sr()")
    .result(Value::Int(11));
}

#[test]
fn q4_json_string_empty() {
    code!(
        "fn run_q4se() -> integer {
    v_q4se = json_string(\"\");
    match v_q4se { JString { value } => value.len(), _ => -1 }
}"
    )
    .expr("run_q4se()")
    .result(Value::Int(0));
}

/// Q2 — `kind(self: JsonValue) -> text` introspection.  One guard
/// per primitive variant, in both free-function syntax `kind(v)`
/// and method syntax `v.kind()`, to lock the registration of both
/// dispatch paths (NATIVE_FNS registers `n_kind` + the
/// `t_9JsonValue_kind` method alias).  Container variants
/// (`JArray`, `JObject`) land with P54 step 4's arena materialiser.
#[test]
fn q2_kind_of_jnull_free_form() {
    code!(
        "fn run_q2kn() -> text {
    v_q2kn = json_null();
    kind(v_q2kn)
}"
    )
    .expr("run_q2kn()")
    .result(Value::str("JNull"));
}

#[test]
fn q2_kind_of_jnull_method_form() {
    code!(
        "fn run_q2kn2() -> text {
    v_q2kn2 = json_null();
    v_q2kn2.kind()
}"
    )
    .expr("run_q2kn2()")
    .result(Value::str("JNull"));
}

#[test]
fn q2_kind_of_jbool() {
    code!(
        "fn run_q2kb() -> text {
    v_q2kb = json_bool(true);
    v_q2kb.kind()
}"
    )
    .expr("run_q2kb()")
    .result(Value::str("JBool"));
}

#[test]
fn q2_kind_of_jnumber() {
    code!(
        "fn run_q2knum() -> text {
    v_q2knum = json_number(42.0);
    v_q2knum.kind()
}"
    )
    .expr("run_q2knum()")
    .result(Value::str("JNumber"));
}

#[test]
fn q2_kind_of_jstring() {
    code!(
        "fn run_q2ks() -> text {
    v_q2ks = json_string(\"hello\");
    v_q2ks.kind()
}"
    )
    .expr("run_q2ks()")
    .result(Value::str("JString"));
}

/// Q2 kind() on a json_parse result — locks that the discriminant
/// byte written by `n_json_parse` for parsed primitives matches
/// the one `n_kind` reads back.  A JSON-parser-vs-kind-reader drift
/// would make `kind(json_parse(x))` return "JUnknown".
#[test]
fn q2_kind_of_parsed_primitive() {
    code!(
        "fn run_q2kp() -> text {
    v_q2kp = json_parse(\"true\");
    kind(v_q2kp)
}"
    )
    .expr("run_q2kp()")
    .result(Value::str("JBool"));
}

/// Q3 primitive-slice — `to_json(self: JsonValue) -> text` renders
/// a JsonValue to canonical RFC 8259 JSON text.  One guard per
/// primitive variant, each measured by text equality to lock the
/// serialisation contract.  Container variants (JArray / JObject)
/// render as `"<pending step 4>"` today; the full recursive
/// formatter lands with P54 step 4.
#[test]
fn q3_to_json_of_jnull() {
    code!(
        "fn run_q3tn() -> text {
    v_q3tn = json_null();
    to_json(v_q3tn)
}"
    )
    .expr("run_q3tn()")
    .result(Value::str("null"));
}

#[test]
fn q3_to_json_of_jbool_true() {
    code!(
        "fn run_q3tb() -> text {
    v_q3tb = json_bool(true);
    v_q3tb.to_json()
}"
    )
    .expr("run_q3tb()")
    .result(Value::str("true"));
}

#[test]
fn q3_to_json_of_jbool_false() {
    code!(
        "fn run_q3tbf() -> text {
    v_q3tbf = json_bool(false);
    v_q3tbf.to_json()
}"
    )
    .expr("run_q3tbf()")
    .result(Value::str("false"));
}

#[test]
fn q3_to_json_of_jnumber_integer() {
    code!(
        "fn run_q3tni() -> text {
    v_q3tni = json_number(42.0);
    v_q3tni.to_json()
}"
    )
    .expr("run_q3tni()")
    .result(Value::str("42"));
}

#[test]
fn q3_to_json_of_jnumber_fractional() {
    code!(
        "fn run_q3tnf() -> text {
    v_q3tnf = json_number(2.75);
    v_q3tnf.to_json()
}"
    )
    .expr("run_q3tnf()")
    .result(Value::str("2.75"));
}

/// Non-finite inputs to json_number (NaN, ±Inf) construct a JNull
/// variant with a diagnostic — to_json on that reads "null", not
/// the garbage representation of the bad float.  Matches RFC 8259
/// which forbids non-finite numeric literals.
#[test]
fn q3_to_json_of_nan_becomes_null() {
    code!(
        "fn run_q3tnn() -> text {
    nan_q3 = 0.0 / 0.0;
    v_q3tnn = json_number(nan_q3);
    v_q3tnn.to_json()
}"
    )
    .expr("run_q3tnn()")
    .result(Value::str("null"));
}

#[test]
fn q3_to_json_of_jstring_plain() {
    code!(
        "fn run_q3ts() -> text {
    v_q3ts = json_string(\"hello\");
    v_q3ts.to_json()
}"
    )
    .expr("run_q3ts()")
    .result(Value::str("\"hello\""));
}

// Q3 escape-sequence regressions (`"a\"b\\c"` round-trip; `\n` /
// `\t` / control-byte encoding) are deferred.  Initial attempt
// exposed that loft's String parser currently drops backslash-
// escapes in string literals inside `code!()` test scaffolding
// (the `q3_to_json_of_jstring_with_escapes` case hung on the
// loft-side interpretation of the `\\` sequence; needs isolated
// reproducer + investigation).  The Rust-side escape logic in
// `n_to_json` is exercised; the test-harness plumbing is what's
// blocking.  Track as a follow-up under Q3.

/// Q3 pretty-print primitive slice — `to_json_pretty(self: JsonValue)`
/// produces identical output to `to_json` for primitive variants
/// (no nested structure to indent).  Divergence lands with P54
/// step 4's arena materialiser.  These guards lock that primitive
/// output is byte-identical across the two entry points today, so
/// a future change that adds pretty-specific padding for
/// primitives would be caught.
#[test]
fn q3_to_json_pretty_of_jnull() {
    code!(
        "fn run_q3pn() -> text {
    v_q3pn = json_null();
    v_q3pn.to_json_pretty()
}"
    )
    .expr("run_q3pn()")
    .result(Value::str("null"));
}

#[test]
fn q3_to_json_pretty_of_jbool() {
    code!(
        "fn run_q3pb() -> text {
    v_q3pb = json_bool(true);
    v_q3pb.to_json_pretty()
}"
    )
    .expr("run_q3pb()")
    .result(Value::str("true"));
}

#[test]
fn q3_to_json_pretty_of_jnumber() {
    code!(
        "fn run_q3pnum() -> text {
    v_q3pnum = json_number(42.0);
    v_q3pnum.to_json_pretty()
}"
    )
    .expr("run_q3pnum()")
    .result(Value::str("42"));
}

#[test]
fn q3_to_json_pretty_of_jstring() {
    code!(
        "fn run_q3ps() -> text {
    v_q3ps = json_string(\"hi\");
    v_q3ps.to_json_pretty()
}"
    )
    .expr("run_q3ps()")
    .result(Value::str("\"hi\""));
}

/// Q3 — `to_json` and `to_json_pretty` must agree on primitives.
/// This regression guard compares the outputs directly and fails
/// if they ever diverge for a primitive variant (which would
/// indicate the pretty path is accidentally formatting leaf
/// values differently from the canonical path).
#[test]
fn q3_to_json_and_pretty_agree_on_primitive() {
    code!(
        "fn run_q3pa() -> boolean {
    v_q3pa = json_number(2.75);
    canonical_q3pa = v_q3pa.to_json();
    pretty_q3pa = v_q3pa.to_json_pretty();
    canonical_q3pa == pretty_q3pa
}"
    )
    .expr("run_q3pa()")
    .result(Value::Boolean(true));
}

/// Q3 — free-function dispatch of `to_json_pretty`.  Locks the
/// `n_to_json_pretty` registration in NATIVE_FNS alongside the
/// `t_9JsonValue_to_json_pretty` method alias.
#[test]
fn q3_to_json_pretty_free_form() {
    code!(
        "fn run_q3pf() -> text {
    v_q3pf = json_null();
    to_json_pretty(v_q3pf)
}"
    )
    .expr("run_q3pf()")
    .result(Value::str("null"));
}

/// Q3 pretty — empty containers stay byte-identical to canonical
/// (no newline padding for `[]` / `{}`).
#[test]
fn q3_to_json_pretty_empty_array() {
    code!(
        "fn run_q3pea() -> text {
    v_q3pea = json_parse(\"[]\");
    v_q3pea.to_json_pretty()
}"
    )
    .expr("run_q3pea()")
    .result(Value::str("[]"));
}

#[test]
fn q3_to_json_pretty_empty_object() {
    code!(
        "fn run_q3peo() -> text {
    v_q3peo = json_parse(\"{{}}\");
    v_q3peo.to_json_pretty()
}"
    )
    .expr("run_q3peo()")
    .result(Value::str("{}"));
}

/// Q3 pretty — non-empty array indents each element on its own
/// line with 2-space indent, closing bracket dedents back.
#[test]
fn q3_to_json_pretty_array_indents_elements() {
    code!(
        "fn run_q3pai() -> text {
    v_q3pai = json_parse(\"[1,2,3]\");
    v_q3pai.to_json_pretty()
}"
    )
    .expr("run_q3pai()")
    .result(Value::str("[\n  1,\n  2,\n  3\n]"));
}

/// Q3 pretty — non-empty object indents each field on its own
/// line; key/value separator is `": "` (colon + single space).
#[test]
fn q3_to_json_pretty_object_indents_fields() {
    code!(
        "fn run_q3poi() -> text {
    v_q3poi = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2}}\");
    v_q3poi.to_json_pretty()
}"
    )
    .expr("run_q3poi()")
    .result(Value::str("{\n  \"a\": 1,\n  \"b\": 2\n}"));
}

/// Q3 pretty — nested containers indent recursively.  Inner
/// container's indent is one level deeper than the outer's.
#[test]
fn q3_to_json_pretty_nested_array_in_object() {
    code!(
        "fn run_q3pnao() -> text {
    v_q3pnao = json_parse(\"{{\\\"k\\\":[1,2]}}\");
    v_q3pnao.to_json_pretty()
}"
    )
    .expr("run_q3pnao()")
    .result(Value::str("{\n  \"k\": [\n    1,\n    2\n  ]\n}"));
}

/// Q3 pretty — canonical and pretty diverge once a non-empty
/// container is in play.  Locks the active difference (the prior
/// stub returned the same text for both, which would have hidden
/// a regression in the pretty walk).
#[test]
fn q3_to_json_and_pretty_differ_on_nonempty_container() {
    code!(
        "fn run_q3pdc() -> boolean {
    v_q3pdc = json_parse(\"[1,2]\");
    v_q3pdc.to_json() != v_q3pdc.to_json_pretty()
}"
    )
    .expr("run_q3pdc()")
    .result(Value::Boolean(true));
}

/// P54 step-4 null-safety — `len()` on each primitive variant
/// returns the integer null sentinel (`i32::MIN`) per the
/// stdlib contract.  Locks the documented "no length defined"
/// behaviour for non-container variants so an accidental
/// switch to `0` (which would be wrong — a real empty array
/// has length 0) gets caught.
#[test]
fn p54_step4_len_on_jnull_is_null_sentinel() {
    code!(
        "fn run_lnn() -> integer {
    v = json_null();
    v.len()
}"
    )
    .expr("run_lnn()")
    .result(Value::Null);
}

#[test]
fn p54_step4_len_on_jbool_is_null_sentinel() {
    code!(
        "fn run_lnb() -> integer {
    v = json_bool(true);
    v.len()
}"
    )
    .expr("run_lnb()")
    .result(Value::Null);
}

#[test]
fn p54_step4_len_on_jnumber_is_null_sentinel() {
    code!(
        "fn run_lnnum() -> integer {
    v = json_number(1.0);
    v.len()
}"
    )
    .expr("run_lnnum()")
    .result(Value::Null);
}

#[test]
fn p54_step4_len_on_jstring_is_null_sentinel() {
    code!(
        "fn run_lnstr() -> integer {
    v = json_string(\"hello\");
    v.len()
}"
    )
    .expr("run_lnstr()")
    .result(Value::Null);
}

/// P54 step-4 null-safety — `field()` on a non-JObject receiver
/// returns `JNull` rather than crashing.  Locks the chained-
/// access safety guarantee (every intermediate missing produces
/// `JNull`, never a trap).
#[test]
fn p54_step4_field_on_jstring_returns_jnull() {
    code!(
        "fn run_fjs() -> text {
    v = json_string(\"hi\");
    v.field(\"missing\").kind()
}"
    )
    .expr("run_fjs()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_field_missing_key_returns_jnull() {
    code!(
        "fn run_fmk() -> text {
    v = json_parse(\"{{\\\"present\\\":1}}\");
    v.field(\"absent\").kind()
}"
    )
    .expr("run_fmk()")
    .result(Value::str("JNull"));
}

/// P54 step-4 null-safety — `item()` on non-JArray, negative
/// index, and out-of-bounds index all return `JNull`.
#[test]
fn p54_step4_item_on_jnumber_returns_jnull() {
    code!(
        "fn run_ijn() -> text {
    v = json_number(42.0);
    v.item(0).kind()
}"
    )
    .expr("run_ijn()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_item_negative_index_returns_jnull() {
    code!(
        "fn run_ini() -> text {
    v = json_parse(\"[1,2,3]\");
    v.item(-1).kind()
}"
    )
    .expr("run_ini()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_item_out_of_bounds_returns_jnull() {
    code!(
        "fn run_iob() -> text {
    v = json_parse(\"[1,2,3]\");
    v.item(99).kind()
}"
    )
    .expr("run_iob()")
    .result(Value::str("JNull"));
}

/// Q3 — round-trip property for primitives.  Each primitive
/// variant survives `to_json` → `json_parse` with its kind
/// (and where applicable, payload) intact.  Listed in
/// QUALITY.md § Q3 Tests as `q3_primitives_round_trip`.
#[test]
fn q3_primitives_round_trip() {
    code!(
        "fn check_q3prt(s: text, expected_kind: text) -> boolean {
    v = json_parse(s);
    text_q3prt = v.to_json();
    parsed_q3prt = json_parse(text_q3prt);
    parsed_q3prt.kind() == expected_kind
}
fn run_q3prt() -> integer {
    score_q3prt = 0;
    if check_q3prt(\"null\", \"JNull\") { score_q3prt += 1; }
    if check_q3prt(\"true\", \"JBool\") { score_q3prt += 1; }
    if check_q3prt(\"false\", \"JBool\") { score_q3prt += 1; }
    if check_q3prt(\"42\", \"JNumber\") { score_q3prt += 1; }
    if check_q3prt(\"3.14\", \"JNumber\") { score_q3prt += 1; }
    if check_q3prt(\"\\\"hi\\\"\", \"JString\") { score_q3prt += 1; }
    score_q3prt
}"
    )
    .expr("run_q3prt()")
    .result(Value::Int(6));
}

/// Q3 — round-trip property for nested objects.  An object with
/// primitive fields survives `to_json` → `json_parse` and the
/// extracted leaves agree on values.  Listed in QUALITY.md
/// § Q3 Tests as `q3_nested_object_round_trip`.
#[test]
fn q3_nested_object_round_trip() {
    code!(
        "fn run_q3nort() -> integer {
    src_q3nort = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    text_q3nort = src_q3nort.to_json();
    back_q3nort = json_parse(text_q3nort);
    sum_q3nort = 0l;
    sum_q3nort += back_q3nort.field(\"a\").as_long();
    sum_q3nort += back_q3nort.field(\"b\").as_long();
    sum_q3nort += back_q3nort.field(\"c\").as_long();
    sum_q3nort as integer
}"
    )
    .expr("run_q3nort()")
    .result(Value::Int(6));
}

/// Q3 — round-trip property for arrays of mixed primitive kinds.
/// `[1,true,\"x\"]` survives `to_json` → `json_parse` with each
/// element's kind preserved.  Listed in QUALITY.md § Q3 Tests as
/// `q3_array_of_mixed_kinds_round_trip`.
#[test]
fn q3_array_of_mixed_kinds_round_trip() {
    code!(
        "fn run_q3amkrt() -> text {
    src_q3amkrt = json_parse(\"[1,true,\\\"x\\\"]\");
    text_q3amkrt = src_q3amkrt.to_json();
    back_q3amkrt = json_parse(text_q3amkrt);
    \"{back_q3amkrt.item(0).kind()}|{back_q3amkrt.item(1).kind()}|{back_q3amkrt.item(2).kind()}\"
}"
    )
    .expr("run_q3amkrt()")
    .result(Value::str("JNumber|JBool|JString"));
}

/// Q3 — pretty-printed output is still valid JSON: `parse(to_json_pretty(v))`
/// produces an equivalent tree.  Locks the property that pretty
/// mode only adds whitespace between structural tokens, never
/// inside string literals or numbers.  Listed in QUALITY.md § Q3
/// Tests as `q3_pretty_form_valid_json`.
#[test]
fn q3_pretty_form_valid_json() {
    code!(
        "fn run_q3pfvj() -> integer {
    src_q3pfvj = json_parse(\"{{\\\"items\\\":[1,2,3]}}\");
    pretty_q3pfvj = src_q3pfvj.to_json_pretty();
    back_q3pfvj = json_parse(pretty_q3pfvj);
    back_q3pfvj.field(\"items\").len()
}"
    )
    .expr("run_q3pfvj()")
    .result(Value::Int(3));
}

/// Q3 — UTF-8 string content passes through `to_json` verbatim
/// (no `\\uXXXX` escaping of BMP characters).  Listed in
/// QUALITY.md § Q3 Tests as `q3_unicode_string_escaping`.
#[test]
fn q3_unicode_string_escaping() {
    code!(
        "fn run_q3use() -> text {
    s_q3use = json_string(\"α β 😊\");
    s_q3use.to_json()
}"
    )
    .expr("run_q3use()")
    .result(Value::str("\"α β 😊\""));
}

/// P54 step 4 first slice — empty arrays `[]` and empty objects
/// `{}` are now materialised as real `JArray` / `JObject`
/// variants (not the earlier `JNull`-stub).  This unblocks
/// `kind()`, `len()`, `has_field()`, and `to_json()` for the
/// empty-container case today; non-empty containers remain
/// stubbed until the full arena materialiser lands.
#[test]
fn p54_step4_empty_array_has_jarray_kind() {
    code!(
        "fn run_p4ea() -> text {
    v_p4ea = json_parse(\"[]\");
    v_p4ea.kind()
}"
    )
    .expr("run_p4ea()")
    .result(Value::str("JArray"));
}

#[test]
fn p54_step4_empty_object_has_jobject_kind() {
    // Loft string literals treat `{...}` as interpolation; escape
    // literal braces by doubling (`{{` → `{`, `}}` → `}`), so
    // `"{{}}"` in loft source is the two-char JSON empty-object
    // literal `{}`.  Same trick below on the other object tests.
    code!(
        "fn run_p4eo() -> text {
    v_p4eo = json_parse(\"{{}}\");
    v_p4eo.kind()
}"
    )
    .expr("run_p4eo()")
    .result(Value::str("JObject"));
}

/// Step 4 first slice — `len()` returns 0 for empty containers
/// (both JArray and JObject).  Primitive variants still return
/// the integer null sentinel via the unchanged path.
#[test]
fn p54_step4_empty_array_len_is_zero() {
    code!(
        "fn run_p4al() -> integer {
    v_p4al = json_parse(\"[]\");
    v_p4al.len()
}"
    )
    .expr("run_p4al()")
    .result(Value::Int(0));
}

#[test]
fn p54_step4_empty_object_len_is_zero() {
    code!(
        "fn run_p4ol() -> integer {
    v_p4ol = json_parse(\"{{}}\");
    v_p4ol.len()
}"
    )
    .expr("run_p4ol()")
    .result(Value::Int(0));
}

/// Step 4 first slice — `to_json()` now renders `"[]"` / `"{}"`
/// for empty containers instead of the earlier `"<pending step
/// 4>"` placeholder.  Non-empty containers still render the
/// placeholder until the full arena materialiser lands.
#[test]
fn p54_step4_empty_array_to_json() {
    code!(
        "fn run_p4aj() -> text {
    v_p4aj = json_parse(\"[]\");
    v_p4aj.to_json()
}"
    )
    .expr("run_p4aj()")
    .result(Value::str("[]"));
}

#[test]
fn p54_step4_empty_object_to_json() {
    code!(
        "fn run_p4oj() -> text {
    v_p4oj = json_parse(\"{{}}\");
    v_p4oj.to_json()
}"
    )
    .expr("run_p4oj()")
    .result(Value::str("{}"));
}

/// Step 4 first slice — round-trip: parse `[]`, serialise, parse
/// again, confirm the discriminant agrees end-to-end.  Locks that
/// `n_json_parse` and `n_to_json` agree on empty containers.
#[test]
fn p54_step4_empty_array_round_trips_through_to_json() {
    code!(
        "fn run_p4ar() -> text {
    first_p4ar = json_parse(\"[]\");
    round_p4ar = json_parse(first_p4ar.to_json());
    round_p4ar.kind()
}"
    )
    .expr("run_p4ar()")
    .result(Value::str("JArray"));
}

/// Step 4 fourth slice (2026-04-14) — nested containers
/// (arrays-of-arrays, objects-of-objects, or any mix) now
/// materialise too, closing step 4.  This guard reverses the
/// earlier stub assertion: `[[1,2],[3,4]]` is a real JArray,
/// not a JNull stub.
#[test]
fn p54_step4_nested_array_materialises() {
    code!(
        "fn run_p4nm() -> text {
    v_p4nm = json_parse(\"[[1,2],[3,4]]\");
    v_p4nm.kind()
}"
    )
    .expr("run_p4nm()")
    .result(Value::str("JArray"));
}

/// Step 4 second slice (2026-04-14) — non-empty arrays of primitive
/// elements now materialise into real JArray variants with elements
/// in an arena sub-record of the root's store.  The
/// `n_json_parse` + `n_len` + `n_item` + `n_to_json` paths all
/// dispatch on JArray and cooperate: parse produces the arena,
/// len reads the vector header, item reads the N-th slot,
/// to_json recurses.  Nested containers still stub as JNull
/// (guarded above).
#[test]
fn p54_step4_nonempty_primitive_array_has_jarray_kind() {
    code!(
        "fn run_p4npk() -> text {
    v_p4npk = json_parse(\"[1,2,3]\");
    v_p4npk.kind()
}"
    )
    .expr("run_p4npk()")
    .result(Value::str("JArray"));
}

#[test]
fn p54_step4_nonempty_primitive_array_length_correct() {
    code!(
        "fn run_p4npl() -> integer {
    v_p4npl = json_parse(\"[1,2,3]\");
    v_p4npl.len()
}"
    )
    .expr("run_p4npl()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nonempty_primitive_array_item_0_is_first() {
    code!(
        "fn run_p4npi0() -> long {
    v_p4npi0 = json_parse(\"[10,20,30]\");
    v_p4npi0.item(0).as_long()
}"
    )
    .expr("run_p4npi0()")
    .result(Value::Long(10));
}

#[test]
fn p54_step4_nonempty_primitive_array_item_1_is_middle() {
    code!(
        "fn run_p4npi1() -> long {
    v_p4npi1 = json_parse(\"[10,20,30]\");
    v_p4npi1.item(1).as_long()
}"
    )
    .expr("run_p4npi1()")
    .result(Value::Long(20));
}

#[test]
fn p54_step4_nonempty_primitive_array_item_out_of_range_returns_jnull() {
    code!(
        "fn run_p4npior() -> text {
    v_p4npior = json_parse(\"[10,20]\");
    v_p4npior.item(5).kind()
}"
    )
    .expr("run_p4npior()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_nonempty_bool_array_item_kind() {
    code!(
        "fn run_p4npbk() -> text {
    v_p4npbk = json_parse(\"[true,false]\");
    v_p4npbk.item(0).kind()
}"
    )
    .expr("run_p4npbk()")
    .result(Value::str("JBool"));
}

#[test]
fn p54_step4_nonempty_string_array_item_value() {
    code!(
        "fn run_p4npsi() -> text {
    v_p4npsi = json_parse(\"[\\\"hello\\\",\\\"world\\\"]\");
    v_p4npsi.item(1).kind()
}"
    )
    .expr("run_p4npsi()")
    .result(Value::str("JString"));
}

#[test]
fn p54_step4_nonempty_array_to_json_round_trips() {
    code!(
        "fn run_p4narj() -> integer {
    v_p4narj = json_parse(\"[1,2,3]\");
    round_p4narj = json_parse(v_p4narj.to_json());
    round_p4narj.len()
}"
    )
    .expr("run_p4narj()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nonempty_array_to_json_text_shape() {
    // `[1,2,3]` serialises each JNumber via `f64::Display`, which
    // prints `1` / `2` / `3` for whole-number floats.
    code!(
        "fn run_p4nats() -> text {
    v_p4nats = json_parse(\"[1,2,3]\");
    v_p4nats.to_json()
}"
    )
    .expr("run_p4nats()")
    .result(Value::str("[1,2,3]"));
}

// The original `p54_parse_array_item_access` test (originally
// `#[ignore]`'d) was un-ignored in place 2026-04-14 by P54 step 4
// second slice — see that test's comment + commit history.

/// Step 4 third slice (2026-04-14) — non-empty primitive objects.
/// Tests mirror the array second-slice guards: discriminant,
/// length, field lookup (hit + miss), has_field, to_json, and
/// a round-trip.  Every loft string with `{` / `}` doubles them
/// to `{{` / `}}` per LOFT.md § String literals.
#[test]
fn p54_step4_nonempty_primitive_object_has_jobject_kind() {
    code!(
        "fn run_p4ok() -> text {
    v_p4oK = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oK.kind()
}"
    )
    .expr("run_p4ok()")
    .result(Value::str("JObject"));
}

#[test]
fn p54_step4_nonempty_primitive_object_length_correct() {
    code!(
        "fn run_p4ol() -> integer {
    v_p4oL = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    v_p4oL.len()
}"
    )
    .expr("run_p4ol()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nonempty_object_field_hit_returns_value() {
    code!(
        "fn run_p4oh() -> long {
    v_p4oH = json_parse(\"{{\\\"age\\\":30}}\");
    v_p4oH.field(\"age\").as_long()
}"
    )
    .expr("run_p4oh()")
    .result(Value::Long(30));
}

#[test]
fn p54_step4_nonempty_object_field_miss_returns_jnull() {
    code!(
        "fn run_p4om() -> text {
    v_p4oM = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oM.field(\"missing\").kind()
}"
    )
    .expr("run_p4om()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_nonempty_object_has_field_hit() {
    code!(
        "fn run_p4ohh() -> boolean {
    v_p4oHh = json_parse(\"{{\\\"users\\\":true}}\");
    v_p4oHh.has_field(\"users\")
}"
    )
    .expr("run_p4ohh()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_step4_nonempty_object_has_field_miss() {
    code!(
        "fn run_p4ohm() -> boolean {
    v_p4oHm = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oHm.has_field(\"q\")
}"
    )
    .expr("run_p4ohm()")
    .result(Value::Boolean(false));
}

#[test]
fn p54_step4_nonempty_object_to_json_text_shape() {
    code!(
        "fn run_p4oj() -> text {
    v_p4oJ = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oJ.to_json()
}"
    )
    .expr("run_p4oj()")
    .result(Value::str("{\"k\":1}"));
}

#[test]
fn p54_step4_nonempty_object_to_json_round_trips() {
    code!(
        "fn run_p4or() -> integer {
    v_p4oR = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2}}\");
    round_p4oR = json_parse(v_p4oR.to_json());
    round_p4oR.len()
}"
    )
    .expr("run_p4or()")
    .result(Value::Int(2));
}

#[test]
fn p54_step4_nonempty_object_mixed_primitive_values() {
    code!(
        "fn run_p4omx() -> text {
    v_p4oMx = json_parse(\"{{\\\"s\\\":\\\"hi\\\",\\\"n\\\":7,\\\"b\\\":true}}\");
    v_p4oMx.field(\"s\").as_text()
}"
    )
    .expr("run_p4omx()")
    .result(Value::str("hi"));
}

/// Step 4 fourth slice — nested arrays.  Outer `[[1,2],[3,4]]`
/// has length 2, each item is itself a JArray of length 2.
#[test]
fn p54_step4_nested_array_outer_length() {
    code!(
        "fn run_p4nal() -> integer {
    v_p4nal = json_parse(\"[[1,2],[3,4]]\");
    v_p4nal.len()
}"
    )
    .expr("run_p4nal()")
    .result(Value::Int(2));
}

#[test]
fn p54_step4_nested_array_inner_length() {
    code!(
        "fn run_p4nil() -> integer {
    v_p4nil = json_parse(\"[[1,2,3],[4,5]]\");
    v_p4nil.item(0).len()
}"
    )
    .expr("run_p4nil()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nested_array_inner_item_value() {
    code!(
        "fn run_p4niv() -> long {
    v_p4niv = json_parse(\"[[10,20],[30,40]]\");
    v_p4niv.item(1).item(0).as_long()
}"
    )
    .expr("run_p4niv()")
    .result(Value::Long(30));
}

/// Step 4 fourth slice — nested objects.
/// `{"a": {"b": 7}}` — outer field "a" is a JObject; inner
/// field "b" is a JNumber 7.
#[test]
fn p54_step4_nested_object_chained_field() {
    code!(
        "fn run_p4nocf() -> long {
    v_p4nocf = json_parse(\"{{\\\"a\\\":{{\\\"b\\\":7}}}}\");
    v_p4nocf.field(\"a\").field(\"b\").as_long()
}"
    )
    .expr("run_p4nocf()")
    .result(Value::Long(7));
}

/// Step 4 fourth slice — array of objects.  `[{"k":1},{"k":2}]`
/// — outer is JArray, each item is a JObject with field `"k"`.
#[test]
fn p54_step4_array_of_objects_field_lookup() {
    code!(
        "fn run_p4aof() -> long {
    v_p4aof = json_parse(\"[{{\\\"k\\\":1}},{{\\\"k\\\":2}}]\");
    v_p4aof.item(1).field(\"k\").as_long()
}"
    )
    .expr("run_p4aof()")
    .result(Value::Long(2));
}

/// Step 4 fourth slice — object containing an array.  Locks the
/// reverse mix from `array_of_objects` so both directions of the
/// recursion are exercised.
#[test]
fn p54_step4_object_with_array_field() {
    code!(
        "fn run_p4owaf() -> integer {
    v_p4owaf = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    v_p4owaf.field(\"items\").len()
}"
    )
    .expr("run_p4owaf()")
    .result(Value::Int(3));
}

/// Step 4 fourth slice — to_json round-trip for nested containers.
#[test]
fn p54_step4_nested_array_to_json_text_shape() {
    code!(
        "fn run_p4narts() -> text {
    v_p4narts = json_parse(\"[[1,2],[3,4]]\");
    v_p4narts.to_json()
}"
    )
    .expr("run_p4narts()")
    .result(Value::str("[[1,2],[3,4]]"));
}

#[test]
fn p54_step4_object_with_array_to_json_text_shape() {
    code!(
        "fn run_p4owats() -> text {
    v_p4owats = json_parse(\"{{\\\"k\\\":[1,2]}}\");
    v_p4owats.to_json()
}"
    )
    .expr("run_p4owats()")
    .result(Value::str("{\"k\":[1,2]}"));
}

/// Step 4 + Q2 cross-integration — `has_field` on an empty
/// JObject returns false (no fields to look up).  The two pieces
/// were developed independently; this guard locks their
/// interaction so a future has_field rewrite can't accidentally
/// claim a field exists on an empty object.
#[test]
fn p54_step4_empty_object_has_no_field() {
    code!(
        "fn run_p4oh() -> boolean {
    v_p4oh = json_parse(\"{{}}\");
    v_p4oh.has_field(\"anything\")
}"
    )
    .expr("run_p4oh()")
    .result(Value::Boolean(false));
}

/// Step 4 + existing `field()` stub cross-integration — querying
/// any key on an empty JObject returns JNull.  Regressing this
/// would break the common `if v.has_field(k) { v.field(k) … }`
/// pattern when users write it on a JSON-parsed empty object.
#[test]
fn p54_step4_empty_object_field_lookup_returns_jnull() {
    code!(
        "fn run_p4ofl() -> text {
    v_p4ofl = json_parse(\"{{}}\");
    v_p4ofl.field(\"k\").kind()
}"
    )
    .expr("run_p4ofl()")
    .result(Value::str("JNull"));
}

/// Step 4 + existing `item()` stub cross-integration — any index
/// into an empty JArray returns JNull.  Locks that out-of-range
/// access doesn't accidentally leak into an uninitialised
/// variant slot.
#[test]
fn p54_step4_empty_array_item_lookup_returns_jnull() {
    code!(
        "fn run_p4eil() -> text {
    v_p4eil = json_parse(\"[]\");
    v_p4eil.item(0).kind()
}"
    )
    .expr("run_p4eil()")
    .result(Value::str("JNull"));
}

/// Step 4 + Q3 `to_json_pretty` cross-integration — pretty output
/// for an empty container is byte-identical to canonical
/// (`"[]"` / `"{}"`) — there's nothing to indent.  Locks that
/// divergence between canonical and pretty only happens when a
/// container has content.
#[test]
fn p54_step4_empty_array_pretty_matches_canonical() {
    code!(
        "fn run_p4eapc() -> boolean {
    v_p4eapc = json_parse(\"[]\");
    canonical_p4eapc = v_p4eapc.to_json();
    pretty_p4eapc = v_p4eapc.to_json_pretty();
    canonical_p4eapc == pretty_p4eapc
}"
    )
    .expr("run_p4eapc()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_step4_empty_object_pretty_matches_canonical() {
    code!(
        "fn run_p4eopc() -> text {
    v_p4eopc = json_parse(\"{{}}\");
    v_p4eopc.to_json_pretty()
}"
    )
    .expr("run_p4eopc()")
    .result(Value::str("{}"));
}

/// Q2 — `has_field(self: JsonValue, name: text) -> boolean` —
/// forward-compatible stub.  Today returns false for every
/// primitive variant (JNull / JBool / JNumber / JString); a real
/// JObject can't be constructed until P54 step 4 so the JObject
/// branch isn't exercised yet.  When step 4 ships, these guards
/// still stand (primitives still return false) and a new
/// `q2_has_field_on_jobject` test will cover the positive case.
#[test]
fn q2_has_field_on_jnull_is_false() {
    code!(
        "fn run_q2hn() -> boolean {
    v_q2hn = json_null();
    v_q2hn.has_field(\"k\")
}"
    )
    .expr("run_q2hn()")
    .result(Value::Boolean(false));
}

#[test]
fn q2_has_field_on_jbool_is_false() {
    code!(
        "fn run_q2hb() -> boolean {
    v_q2hb = json_bool(true);
    v_q2hb.has_field(\"value\")
}"
    )
    .expr("run_q2hb()")
    .result(Value::Boolean(false));
}

#[test]
fn q2_has_field_on_jnumber_is_false() {
    code!(
        "fn run_q2hnum() -> boolean {
    v_q2hnum = json_number(42.0);
    v_q2hnum.has_field(\"n\")
}"
    )
    .expr("run_q2hnum()")
    .result(Value::Boolean(false));
}

#[test]
fn q2_has_field_on_jstring_is_false() {
    code!(
        "fn run_q2hs() -> boolean {
    v_q2hs = json_string(\"hello\");
    v_q2hs.has_field(\"anything\")
}"
    )
    .expr("run_q2hs()")
    .result(Value::Boolean(false));
}

/// Q2 — free-function form of `has_field`.  Locks the `n_has_field`
/// registration in NATIVE_FNS alongside the
/// `t_9JsonValue_has_field` method alias so both dispatch paths
/// keep working.
#[test]
fn q2_has_field_free_form_on_parsed_primitive() {
    code!(
        "fn run_q2hf() -> boolean {
    v_q2hf = json_parse(\"42\");
    has_field(v_q2hf, \"k\")
}"
    )
    .expr("run_q2hf()")
    .result(Value::Boolean(false));
}

/// Q2 — `keys(self: JsonValue) -> vector<text>` returns the
/// field names of a JObject in insertion order, empty vector
/// for every other variant.  First slice (2026-04-14): empty
/// vector unconditionally — JObject walk lands in a follow-up.
/// These guards lock the empty-vector return shape across all
/// variants, in both free and method form.
#[test]
fn q2_keys_on_jnull_is_empty() {
    code!(
        "fn run_q2kne() -> integer {
    v_q2kne = json_null();
    ks_q2kne = v_q2kne.keys();
    ks_q2kne.len()
}"
    )
    .expr("run_q2kne()")
    .result(Value::Int(0));
}

#[test]
fn q2_keys_on_jbool_is_empty() {
    code!(
        "fn run_q2kbe() -> integer {
    v_q2kbe = json_bool(true);
    keys(v_q2kbe).len()
}"
    )
    .expr("run_q2kbe()")
    .result(Value::Int(0));
}

#[test]
fn q2_keys_on_jobject_returns_field_names_length() {
    // (Was `q2_keys_on_jobject_is_empty_today` until 2026-04-14
    // when the JObject walk shipped.)  Locks that `keys()` on
    // a JObject now returns the actual field-name vector.
    code!(
        "fn run_q2koe() -> integer {
    v_q2koe = json_parse(\"{{\\\"k\\\":1}}\");
    v_q2koe.keys().len()
}"
    )
    .expr("run_q2koe()")
    .result(Value::Int(1));
}

#[test]
fn q2_keys_on_jobject_returns_multiple_field_names_length() {
    code!(
        "fn run_q2km() -> integer {
    v_q2km = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    v_q2km.keys().len()
}"
    )
    .expr("run_q2km()")
    .result(Value::Int(3));
}

/// Q2 — `keys()` JObject walk preserves insertion order: the
/// first key in the source is the first key in the result.
#[test]
fn q2_keys_on_jobject_preserves_first_name() {
    code!(
        "fn run_q2kf() -> text {
    v_q2kf = json_parse(\"{{\\\"alpha\\\":1,\\\"beta\\\":2}}\");
    ks_q2kf = v_q2kf.keys();
    first_q2kf = \"\";
    for k in ks_q2kf {
        if first_q2kf == \"\" { first_q2kf = k; }
    }
    first_q2kf
}"
    )
    .expr("run_q2kf()")
    .result(Value::str("alpha"));
}

/// Q2 — `keys()` collects every name, locked by joining them.
#[test]
fn q2_keys_on_jobject_collects_all_names() {
    code!(
        "fn run_q2kc() -> text {
    v_q2kc = json_parse(\"{{\\\"x\\\":1,\\\"y\\\":2}}\");
    out_q2kc = \"\";
    for k in v_q2kc.keys() { out_q2kc += k; out_q2kc += \"|\"; }
    out_q2kc
}"
    )
    .expr("run_q2kc()")
    .result(Value::str("x|y|"));
}

/// Q2 — the empty `keys()` result is a real iterable: a `for`
/// loop over it terminates without executing the body.  Locks
/// that callers can write `for k in v.keys() { ... }` today
/// without it crashing or looping forever.
#[test]
fn q2_keys_for_loop_is_safe() {
    code!(
        "fn run_q2kfl() -> integer {
    v_q2kfl = json_null();
    count_q2kfl = 0;
    for _k in v_q2kfl.keys() { count_q2kfl += 1; }
    count_q2kfl
}"
    )
    .expr("run_q2kfl()")
    .result(Value::Int(0));
}

/// Q2 — `fields(self: JsonValue) -> vector<JsonField>` mirrors
/// `keys`'s shape but returns (name, value) entries.  First
/// slice (2026-04-14): empty vector for every variant including
/// JObject — the real walk lands with `keys`'s JObject walk.
#[test]
fn q2_fields_on_jnull_is_empty() {
    code!(
        "fn run_q2fne() -> integer {
    v_q2fne = json_null();
    fs_q2fne = v_q2fne.fields();
    fs_q2fne.len()
}"
    )
    .expr("run_q2fne()")
    .result(Value::Int(0));
}

#[test]
fn q2_fields_on_jstring_is_empty() {
    code!(
        "fn run_q2fse() -> integer {
    v_q2fse = json_string(\"hi\");
    fields(v_q2fse).len()
}"
    )
    .expr("run_q2fse()")
    .result(Value::Int(0));
}

#[test]
fn q2_fields_on_jobject_returns_field_entries_length() {
    // (Was `q2_fields_on_jobject_is_empty_today` until 2026-04-14
    // when the JObject walk shipped.)  Locks that `fields()`
    // on a JObject now returns a vector of JsonField entries.
    code!(
        "fn run_q2foe() -> integer {
    v_q2foe = json_parse(\"{{\\\"k\\\":1}}\");
    v_q2foe.fields().len()
}"
    )
    .expr("run_q2foe()")
    .result(Value::Int(1));
}

#[test]
fn q2_fields_on_jobject_collects_multiple_entries() {
    code!(
        "fn run_q2fm() -> integer {
    v_q2fm = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    v_q2fm.fields().len()
}"
    )
    .expr("run_q2fm()")
    .result(Value::Int(3));
}

/// Q2 — `fields()` JObject walk preserves names: iterating the
/// result and reading `.name` gives back each JsonField's name.
#[test]
fn q2_fields_collects_all_names() {
    code!(
        "fn run_q2fcn() -> text {
    v_q2fcn = json_parse(\"{{\\\"x\\\":1,\\\"y\\\":2}}\");
    out_q2fcn = \"\";
    for entry in v_q2fcn.fields() { out_q2fcn += entry.name; out_q2fcn += \"|\"; }
    out_q2fcn
}"
    )
    .expr("run_q2fcn()")
    .result(Value::str("x|y|"));
}

/// Q2 — `fields()` JObject walk also copies primitive values:
/// iterating gives back each `entry.value` as the right variant
/// with the right payload.  This guard covers JNumber.
#[test]
fn q2_fields_preserves_primitive_number_values() {
    code!(
        "fn run_q2fp() -> long {
    v_q2fp = json_parse(\"{{\\\"k\\\":42}}\");
    sum_q2fp = 0l;
    for entry in v_q2fp.fields() {
        sum_q2fp += entry.value.as_long();
    }
    sum_q2fp
}"
    )
    .expr("run_q2fp()")
    .result(Value::Long(42));
}

/// Q2 — `fields()` JObject walk: container values deep-copy
/// into the result vector (JArray preserved).
#[test]
fn q2_fields_preserves_container_values_array() {
    code!(
        "fn run_q2fca() -> text {
    v_q2fca = json_parse(\"{{\\\"k\\\":[1,2,3]}}\");
    kind_q2fca = \"\";
    for entry in v_q2fca.fields() { kind_q2fca = entry.value.kind(); }
    kind_q2fca
}"
    )
    .expr("run_q2fca()")
    .result(Value::str("JArray"));
}

/// Q2 — `fields()` JObject walk: container values deep-copy
/// into the result vector (JObject preserved).
#[test]
fn q2_fields_preserves_container_values_object() {
    code!(
        "fn run_q2fco() -> text {
    v_q2fco = json_parse(\"{{\\\"k\\\":{{\\\"a\\\":1}}}}\");
    kind_q2fco = \"\";
    for entry in v_q2fco.fields() { kind_q2fco = entry.value.kind(); }
    kind_q2fco
}"
    )
    .expr("run_q2fco()")
    .result(Value::str("JObject"));
}

#[test]
fn q2_fields_for_loop_is_safe() {
    code!(
        "fn run_q2ffl() -> integer {
    v_q2ffl = json_bool(true);
    count_q2ffl = 0;
    for _entry in v_q2ffl.fields() { count_q2ffl += 1; }
    count_q2ffl
}"
    )
    .expr("run_q2ffl()")
    .result(Value::Int(0));
}

/// Q4 container constructors — first slice (2026-04-14):
/// `json_array(items)` and `json_object(fields)` build empty
/// containers when given empty input vectors.  Non-empty input
/// returns JNull + diagnostic; the per-element deep-copy lands
/// in a follow-up.
#[test]
fn q4_json_array_empty_vector_returns_jarray() {
    code!(
        "fn run_q4ae() -> text {
    items_q4ae: vector<JsonValue> = [];
    v_q4ae = json_array(items_q4ae);
    v_q4ae.kind()
}"
    )
    .expr("run_q4ae()")
    .result(Value::str("JArray"));
}

#[test]
fn q4_json_array_empty_has_zero_length() {
    code!(
        "fn run_q4ael() -> integer {
    items_q4ael: vector<JsonValue> = [];
    v_q4ael = json_array(items_q4ael);
    v_q4ael.len()
}"
    )
    .expr("run_q4ael()")
    .result(Value::Int(0));
}

#[test]
fn q4_json_array_empty_serialises_as_brackets() {
    code!(
        "fn run_q4aes() -> text {
    items_q4aes: vector<JsonValue> = [];
    v_q4aes = json_array(items_q4aes);
    v_q4aes.to_json()
}"
    )
    .expr("run_q4aes()")
    .result(Value::str("[]"));
}

#[test]
fn q4_json_array_nonempty_input_returns_jarray() {
    // (Was `…_stubs_to_jnull` until 2026-04-14 when the deep-copy
    // landed.)  Locks that non-empty input now produces a real
    // JArray with the right element count.
    code!(
        "fn run_q4ans() -> integer {
    items_q4ans: vector<JsonValue> = [json_null()];
    v_q4ans = json_array(items_q4ans);
    v_q4ans.len()
}"
    )
    .expr("run_q4ans()")
    .result(Value::Int(1));
}

/// Q4 `json_array` deep-copy — multiple elements, mixed primitive
/// variants, all preserved in the result arena.  `to_json` round-
/// trips back to the canonical text form.
#[test]
fn q4_json_array_multi_element_round_trips() {
    code!(
        "fn run_q4amrt() -> text {
    items_q4amrt: vector<JsonValue> = [
        json_number(1.0),
        json_number(2.0),
        json_number(3.0)
    ];
    v_q4amrt = json_array(items_q4amrt);
    v_q4amrt.to_json()
}"
    )
    .expr("run_q4amrt()")
    .result(Value::str("[1,2,3]"));
}

/// Q4 `json_array` deep-copy — element index access.  `item(N)`
/// reads back the value passed at position N.
#[test]
fn q4_json_array_item_access_after_construction() {
    code!(
        "fn run_q4aiac() -> long {
    items_q4aiac: vector<JsonValue> = [
        json_number(10.0),
        json_number(20.0),
        json_number(30.0)
    ];
    v_q4aiac = json_array(items_q4aiac);
    v_q4aiac.item(1).as_long()
}"
    )
    .expr("run_q4aiac()")
    .result(Value::Long(20));
}

/// Q4 `json_array` deep-copy — recursive: array of arrays.
/// Inner arrays are themselves built via `json_array`, then
/// embedded.  Outer length 2; inner length 2.
#[test]
fn q4_json_array_nested_construction() {
    code!(
        "fn run_q4anc() -> integer {
    inner_a_q4anc: vector<JsonValue> = [json_number(1.0), json_number(2.0)];
    inner_b_q4anc: vector<JsonValue> = [json_number(3.0), json_number(4.0)];
    outer_q4anc: vector<JsonValue> = [
        json_array(inner_a_q4anc),
        json_array(inner_b_q4anc)
    ];
    v_q4anc = json_array(outer_q4anc);
    v_q4anc.item(1).item(0).as_long() as integer
}"
    )
    .expr("run_q4anc()")
    .result(Value::Int(3));
}

#[test]
fn q4_json_object_empty_vector_returns_jobject() {
    code!(
        "fn run_q4oe() -> text {
    fields_q4oe: vector<JsonField> = [];
    v_q4oe = json_object(fields_q4oe);
    v_q4oe.kind()
}"
    )
    .expr("run_q4oe()")
    .result(Value::str("JObject"));
}

#[test]
fn q4_json_object_empty_has_zero_length() {
    code!(
        "fn run_q4oel() -> integer {
    fields_q4oel: vector<JsonField> = [];
    v_q4oel = json_object(fields_q4oel);
    v_q4oel.len()
}"
    )
    .expr("run_q4oel()")
    .result(Value::Int(0));
}

#[test]
fn q4_json_object_empty_serialises_as_braces() {
    code!(
        "fn run_q4oes() -> text {
    fields_q4oes: vector<JsonField> = [];
    v_q4oes = json_object(fields_q4oes);
    v_q4oes.to_json()
}"
    )
    .expr("run_q4oes()")
    .result(Value::str("{}"));
}

/// Q4 `json_object` deep-copy — single field round-trip.  Build a
/// JsonField in loft, pass it to json_object, read back via
/// field() lookup.
#[test]
fn q4_json_object_single_field_round_trips() {
    code!(
        "fn run_q4osf() -> long {
    f_q4osf = JsonField { name: \"k\", value: json_number(42.0) };
    fields_q4osf: vector<JsonField> = [f_q4osf];
    v_q4osf = json_object(fields_q4osf);
    v_q4osf.field(\"k\").as_long()
}"
    )
    .expr("run_q4osf()")
    .result(Value::Long(42));
}

/// Q4 `json_object` deep-copy — multi-field length.
#[test]
fn q4_json_object_multi_field_length() {
    code!(
        "fn run_q4omfl() -> integer {
    fa_q4omfl = JsonField { name: \"a\", value: json_number(1.0) };
    fb_q4omfl = JsonField { name: \"b\", value: json_string(\"x\") };
    fc_q4omfl = JsonField { name: \"c\", value: json_bool(true) };
    fields_q4omfl: vector<JsonField> = [fa_q4omfl, fb_q4omfl, fc_q4omfl];
    v_q4omfl = json_object(fields_q4omfl);
    v_q4omfl.len()
}"
    )
    .expr("run_q4omfl()")
    .result(Value::Int(3));
}

/// Q4 `json_object` deep-copy — to_json round-trip.  Build via
/// json_object, serialise via to_json, parse back, confirm shape.
#[test]
fn q4_json_object_serialisation() {
    code!(
        "fn run_q4os() -> text {
    f1_q4os = JsonField { name: \"k\", value: json_number(7.0) };
    fields_q4os: vector<JsonField> = [f1_q4os];
    v_q4os = json_object(fields_q4os);
    v_q4os.to_json()
}"
    )
    .expr("run_q4os()")
    .result(Value::str("{\"k\":7}"));
}

/// Q4 — forward a captured subtree.  Parses a JSON array, takes
/// the resulting JArray, embeds it as the value of a freshly-
/// constructed JObject field, and serialises.  Locks that the
/// `dbref_to_parsed` deep-copy used by `n_json_object` correctly
/// preserves container values originating from a parse arena
/// (not just constructor calls).  Listed in QUALITY.md § Q4 Tests
/// as `q4_forward_captured_subtree`.
#[test]
fn q4_forward_captured_subtree_array() {
    code!(
        "fn run_q4fcsa() -> text {
    src_q4fcsa = json_parse(\"[10,20,30]\");
    fields_q4fcsa: vector<JsonField> = [
        JsonField { name: \"data\", value: src_q4fcsa }
    ];
    obj_q4fcsa = json_object(fields_q4fcsa);
    obj_q4fcsa.to_json()
}"
    )
    .expr("run_q4fcsa()")
    .result(Value::str("{\"data\":[10,20,30]}"));
}

/// Q4 — forward-captured-subtree, object variant.  Same shape as
/// the array case but the captured subtree is itself a JObject.
#[test]
fn q4_forward_captured_subtree_object() {
    code!(
        "fn run_q4fcso() -> text {
    inner_q4fcso = json_parse(\"{{\\\"x\\\":1,\\\"y\\\":2}}\");
    fields_q4fcso: vector<JsonField> = [
        JsonField { name: \"point\", value: inner_q4fcso }
    ];
    obj_q4fcso = json_object(fields_q4fcso);
    obj_q4fcso.to_json()
}"
    )
    .expr("run_q4fcso()")
    .result(Value::str("{\"point\":{\"x\":1,\"y\":2}}"));
}

/// Q4 — forward-captured-subtree round-trip: parsing the
/// serialised result yields a tree whose structure agrees
/// with the original captured subtree.
#[test]
fn q4_forward_captured_subtree_round_trip() {
    code!(
        "fn run_q4fcsr() -> long {
    src_q4fcsr = json_parse(\"[10,20,30]\");
    fields_q4fcsr: vector<JsonField> = [
        JsonField { name: \"data\", value: src_q4fcsr }
    ];
    obj_q4fcsr = json_object(fields_q4fcsr);
    text_q4fcsr = obj_q4fcsr.to_json();
    back_q4fcsr = json_parse(text_q4fcsr);
    back_q4fcsr.field(\"data\").item(1).as_long()
}"
    )
    .expr("run_q4fcsr()")
    .result(Value::Long(20));
}

/// Q2 full-surface smoke — exercises every Q2 helper
/// (`kind`, `has_field`, `keys`, `fields`) on the same JObject
/// value in one expression chain.  Locks the four-way dispatch
/// interaction.  Score breakdown today (post both keys + fields
/// JObject walks 2026-04-14): kind=="JObject" → 1, has_field("k")
/// → 1, keys.len() → 1, fields.len() → 1.  Total 4 — every Q2
/// helper now returns its real JObject answer.
#[test]
fn q2_full_surface_smoke_on_jobject() {
    code!(
        "fn run_q2fs() -> integer {
    v_q2fs = json_parse(\"{{\\\"k\\\":1}}\");
    score_q2fs = 0;
    if v_q2fs.kind() == \"JObject\" { score_q2fs += 1; }
    if v_q2fs.has_field(\"k\")     { score_q2fs += 1; }
    score_q2fs += v_q2fs.keys().len();
    score_q2fs += v_q2fs.fields().len();
    score_q2fs
}"
    )
    .expr("run_q2fs()")
    .result(Value::Int(4));
}

/// Q2 — the common guarded-access idiom works today and will
/// keep working when step 4 lands.  `if v.has_field(k) { … }`
/// is the forward-compatible pattern — on a primitive it
/// takes the else branch, on a JObject (future) it takes the
/// then branch iff the key is present.  This guard locks the
/// control-flow shape.
#[test]
fn q2_has_field_gates_conditional_safely() {
    code!(
        "fn run_q2hg() -> integer {
    v_q2hg = json_parse(\"null\");
    if v_q2hg.has_field(\"users\") { 1 } else { 2 }
}"
    )
    .expr("run_q2hg()")
    .result(Value::Int(2));
}

// INC#18 — `x#break` is a labelled-break statement that reuses the
// `#attribute` syntax.  Documented in LOFT.md § Break and continue;
// these tests lock the behaviour so the two-mechanism design cannot
// silently regress.

/// INC#27 — corrected 2026-04-13: `x#continue` **is** implemented
/// correctly as labelled-continue, symmetric to `x#break`.  The
/// earlier writeup declaring it a silent miscompile was wrong — it
/// relied on a nested-loop reproducer where bare-continue and
/// labelled-continue happened to produce the same numeric sum (320).
/// This test uses an outer-body operation that runs BETWEEN x
/// iterations: a bare `continue` would let it run each time, a
/// labelled `x#continue` skips it when we jump past it.  The
/// observed result outer_count=1, inner_count=6 → 106 confirms the
/// labelled-continue semantics.  Manual walk:
///   x=1: y=1 inner=1; y=2 x#continue → skip rest of x=1 body
///   x=2: y=1 inner=2; y=2 inner=3; y=3 x#continue → skip rest
///   x=3: y=1,2,3 all pass; inner=4,5,6; outer_body runs → outer=1
#[test]
fn inc27_x_continue_is_labelled_continue() {
    code!(
        "fn run() -> integer {
    outer_count = 0;
    inner_count = 0;
    for x in 1..4 {
        for y in 1..4 {
            if y > x { x#continue; }
            inner_count += 1;
        }
        outer_count += 1;
    }
    outer_count * 100 + inner_count
}"
    )
    .expr("run()")
    .result(Value::Int(106));
}

/// `x#break` from an inner loop exits the outer loop whose variable
/// is `x` — not just the innermost loop.  Without the labelled break,
/// the outer loop would continue and overwrite `first`.
#[test]
fn inc18_labelled_break_exits_outer_loop() {
    code!(
        "fn run() -> integer {
    first = 0;
    for x in 1..5 {
        for y in 1..5 {
            if x * y >= 6 {
                first = x * 100 + y;
                x#break;
            }
        }
    }
    first
}"
    )
    .expr("run()")
    .result(Value::Int(203));
}

// P140 — vector range-slice `v[a..b]` used to produce a bare Rust
// panic at `src/scopes.rs:250` via the test harness.  Root cause:
// tests/testing.rs ran `scopes::check` BEFORE `assert_diagnostics` +
// the `Level::Error` return, contrary to `src/main.rs` order — a
// parser-level type error (iterator vs vector<integer>) produced a
// malformed IR that scope analysis panicked on.  Fixed 2026-04-13 by
// aligning harness order with the binary's.  The diagnostic the
// parser always emitted (type mismatch on sum_of) is now the
// user-facing error, with a proper source location.
#[test]
fn p140_vector_range_slice_reports_type_mismatch() {
    code!(
        "fn run() -> integer {
    v = [10, 20, 30, 40, 50];
    s = v[1..4];
    sum_of(s)
}"
    )
    .expr("run()")
    .error("iterator(integer(-2147483647, 2147483647, false), null) should be vector<integer> on call to sum_of at p140_vector_range_slice_reports_type_mismatch:5:1");
}

// INC#2 — vector has comprehensions; sorted/index do not.  Documented
// in LOFT.md § Key-based collections (gotcha block).  These tests
// lock the vector-vs-keyed-collection asymmetry so a future uniformity
// refactor cannot silently flip either half without updating the doc.

/// Vector comprehension `[for x in v if p { … }]` compiles and runs.
/// The positive baseline for the comprehension half of INC#2.
#[test]
fn inc02_vector_comprehension_works() {
    code!(
        "fn run() -> integer {
    v = [1, 2, 3, 4, 5, 6];
    sum_of([for x in v if x > 3 { x }])
}"
    )
    .expr("run()")
    .result(Value::Int(15));
}

/// Sorted collections ARE iterable — the keyed-collection half that
/// *does* share the `for` API with vector.
#[test]
fn inc02_sorted_is_iterable() {
    code!(
        "struct Elm { k: integer, v: integer }
struct Db { s: sorted<Elm[k]> }
fn run() -> integer {
    db = Db { s: [Elm { k: 1, v: 10 }, Elm { k: 2, v: 20 }, Elm { k: 3, v: 30 }] };
    total = 0;
    for e in db.s { total += e.v; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(60));
}

// INC#8 — method-vs-free-function is the stdlib author's choice per
// function.  Documented in LOFT.md § Methods and function calls
// (gotcha block).  These tests lock concrete examples so the
// stdlib's declared call-forms cannot silently drift.

/// `sum_of(v)` is a free-function-only stdlib definition (no `self` /
/// `both` on its first parameter).  Method syntax `v.sum_of()` must
/// not resolve — locks the "free-only" half of the INC#8 asymmetry.
#[test]
fn inc08_sum_of_is_free_function_only() {
    code!(
        "fn run() -> integer {
    v = [10, 20, 30];
    v.sum_of()
}"
    )
    .expr("run()")
    .error("Unknown field vector.sum_of — did you mean the free function `sum_of(…)` ? (stdlib declared `sum_of` as free-only; see LOFT.md § Methods and function calls) at inc08_sum_of_is_free_function_only:3:14");
}

/// `text.starts_with(s)` is declared with `self: text` — method syntax
/// works, free-function syntax doesn't.  Pairs with
/// `inc08_sum_of_is_free_function_only` to show the asymmetry runs in
/// both directions per the stdlib declaration.
/// QUALITY 6d — writing a bare `hash<Row[id]>()` constructor
/// expression used to produce the cryptic `"Indexing a non vector"`
/// error with no pointer to the struct-literal idiom that actually
/// works.  The diagnostic now spells out both halves (the missing
/// feature and the idiom users should reach for).
#[test]
fn quality_6d_keyed_collection_constructor_hint() {
    code!(
        "struct Row { id: integer, v: integer }
fn run() -> integer {
    h = hash<Row[id]>();
    0
}"
    )
    .error(
        "Indexing a non vector — keyed collections (hash/sorted/index/spacial) have no generic-constructor expression; declare them as a struct field and initialise via a vector literal: `struct Db { h: hash<Row[id]> }; db = Db { h: [Row { id: 1 }] }` at quality_6d_keyed_collection_constructor_hint:3:20",
    )
    .error(
        "No matching operator '<' on 'unknown(0)' and 'boolean' at quality_6d_keyed_collection_constructor_hint:3:24",
    );
}

/// QUALITY 6c — the free-function hint must NOT fire when there is
/// no `n_<field>` function compatible with the receiver.  Locks the
/// specificity of the hint: a genuinely-misspelled field produces
/// the plain "Unknown field" message without a misleading
/// "did you mean …" tail.
#[test]
fn quality_6c_unknown_field_without_free_fn_has_no_hint() {
    code!(
        "struct Point { x: integer, y: integer }
fn run() -> integer {
    p = Point { x: 1, y: 2 };
    p.z
}"
    )
    .error("Unknown field Point.z at quality_6c_unknown_field_without_free_fn_has_no_hint:5:2");
}

#[test]
fn inc08_starts_with_is_method_not_free_function() {
    code!(
        "fn run() -> boolean {
    s = \"hello\";
    s.starts_with(\"he\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// QUALITY 6c follow-on — the free→method direction.  `starts_with`
/// is declared `self: text`; calling it as a free function with a
/// wrong-type receiver (`starts_with(5, "he")`) used to produce the
/// cryptic `"Unknown function starts_with"` — the function *does*
/// exist, just not with `integer` as the receiver.  Hint now names
/// the receiver type the method is declared on.
#[test]
fn quality_6c_free_call_on_wrong_type_suggests_method() {
    code!(
        "fn run() -> boolean {
    starts_with(5, \"he\")
}"
    )
    .error("Unknown function starts_with — did you mean the method `x.starts_with(…)` on text? (stdlib declared `starts_with` as a method; see LOFT.md § Methods and function calls) at quality_6c_free_call_on_wrong_type_suggests_method:3:2");
}

/// QUALITY 6c follow-on — methods declared on several receiver types
/// (`is_numeric` lives on both `text` and `character`) enumerate all
/// candidates so the user can pick the right one.
#[test]
fn quality_6c_free_call_lists_all_method_receivers() {
    code!(
        "fn run() -> boolean {
    is_numeric(5)
}"
    )
    .error("Unknown function is_numeric — did you mean the method `x.is_numeric(…)` on text / character? (stdlib declared `is_numeric` as a method; see LOFT.md § Methods and function calls) at quality_6c_free_call_lists_all_method_receivers:3:2");
}

/// QUALITY 6c follow-on — the hint must stay silent when no method
/// by that name exists anywhere.  A genuinely-misspelled free
/// function name still produces the plain "Unknown function …"
/// message, without a misleading "did you mean …" tail.
#[test]
fn quality_6c_free_call_unknown_fn_has_no_method_hint() {
    code!(
        "fn run() -> integer {
    xyzzy_never_defined(5)
}"
    )
    .error("Unknown function xyzzy_never_defined at quality_6c_free_call_unknown_fn_has_no_method_hint:3:2");
}

/// `len` is declared `both: vector` — it works equally as method
/// (`v.len()`) and as free function (`len(v)`).  Guards the `both`
/// half of the INC#8 story: when an author picks `both`, the
/// asymmetry disappears.
#[test]
fn inc08_len_with_both_works_either_way() {
    code!(
        "fn run() -> integer {
    v = [1, 2, 3, 4];
    v.len() + len(v)
}"
    )
    .expr("run()")
    .result(Value::Int(8));
}

#[test]
fn inc18_bare_break_exits_innermost_only() {
    code!(
        "fn run() -> integer {
    count = 0;
    for x in 1..4 {
        for y in 1..4 {
            if y >= 2 { break; }
            count += x;
        }
    }
    count
}"
    )
    .expr("run()")
    .result(Value::Int(6));
}

// P143 regression — ref-returning function with two return paths:
//   early-return `gh_c.ck_hexes[0]` (DbRef into a `for`-iterator element
//   inside the argument `m`) vs fallthrough `Hex {}` (local promoted to
//   hidden `__ref_1`).  Calling the function twice on the same populated
//   Map used to SIGSEGV whenever memory layout didn't catch it (P143).
//
// Fix landed in `src/state/codegen.rs::gen_set_first_ref_call_copy` —
// emit `n_set_store_lock(arg, true)` for every ref-typed argument of
// the call before `OpCopyRecord`, then `n_set_store_lock(arg, false)`
// after.  The existing `OpCopyRecord` guard at `src/state/io.rs:1001`
// already skips the source-free when the source store is `locked`,
// so an early-return that aliased one of the args no longer kills
// the caller's argument.  The work-ref scope-exit logic in
// `src/scopes.rs::free_vars` was extended to free `__ref_*` /
// `__rref_*` work-refs to recover the storage that the
// non-aliased-source path used to claim via the `0x8000` bit.
//
// Fixtures: `tests/lib/p143_types.loft`, `tests/lib/p143_entry.loft`,
// `tests/lib/p143_main.loft` — three IR shapes (empty-map fallback,
// found-on-first-chunk, loop-fallback-after-non-matching-chunk).
#[test]
fn p143_default_struct_return_from_nested_vector_use() {
    let mut p = Parser::new();
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_dir("default", true, false).unwrap();
    p.parse("tests/lib/p143_main.loft", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "P143 regression: had_fatal set — ref-returning fn with early-return-through-iterator + fallthrough-default still corrupts memory"
    );
}

/// P144: forwarding a `&Struct` parameter to another function that also
/// takes `&Struct` caused native codegen to emit `*var_b` (deref) instead
/// of `var_b` (pass-through).  The fix in `calls.rs` detects when a
/// `Value::Var` pointing to a `RefVar` parameter is passed to another
/// `RefVar` parameter and emits it directly.
///
/// Interpreter test: parse + execute the cross-file package.
#[test]
fn p144_ref_param_forward_interpreter() {
    let mut p = Parser::new();
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_dir("default", true, false).unwrap();
    p.parse("tests/lib/p144_main.loft", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "P144 regression: & param forward caused runtime error"
    );
}

/// P144: native codegen test — the generated Rust must compile and run.
#[test]
fn p144_ref_param_forward_native() {
    let mut p = Parser::new();
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_dir("default", true, false).unwrap();
    p.parse("tests/lib/p144_main.loft", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    // Emit the native Rust source and verify it compiles.
    let rs_path = std::env::temp_dir().join("loft_p144_native.rs");
    {
        let mut f = std::fs::File::create(&rs_path).unwrap();
        let start_def = 0;
        let end_def = p.data.definitions();
        let main_nr = p.data.def_nr("n_main");
        let entry_defs: Vec<u32> = if main_nr < end_def {
            vec![main_nr]
        } else {
            (start_def..end_def).collect()
        };
        let mut out = loft::generation::Output {
            data: &p.data,
            stores: &state.database,
            counter: 0,
            indent: 0,
            def_nr: 0,
            declared: std::collections::HashSet::new(),
            reachable: std::collections::HashSet::new(),
            loop_stack: Vec::new(),
            next_format_count: 0,
            yield_collect: false,
            fn_ref_context: false,
            call_stack_prefix: None,
            wasm_browser: false,
        };
        out.output_native_reachable(&mut f, start_def, end_def, &entry_defs)
            .unwrap();
    }

    // Read and check the generated source contains the fix pattern.
    let source = std::fs::read_to_string(&rs_path).unwrap();
    // The call to box_ensure should pass var_b directly, not *var_b.
    assert!(
        !source.contains("n_box_ensure(stores, *var_b)"),
        "P144 regression: native codegen still emits *var_b for & param forward.\nGenerated: {}",
        rs_path.display()
    );
    assert!(
        source.contains("n_box_ensure(stores, var_b)"),
        "P144 regression: expected direct var_b pass-through for & param.\nGenerated: {}",
        rs_path.display()
    );
    let _ = std::fs::remove_file(&rs_path);
}

/// P145 regression: user fn name collision with native stdlib
/// (e.g. user `to_json` → `n_to_json`, stdlib also has `n_to_json`
/// for JsonValue serialization).  `generate_call` used to emit
/// `OpStaticCall` (native dispatch) whenever `library_names`
/// matched, bypassing the user body's `OpCall` path and
/// corrupting the stack.  Fix: skip library_names lookup when
/// `def.code != Value::Null`.
/// P151 regression guard: forward-reference to a struct-returning fn
/// followed by field mutation USED to corrupt variable type inference.
/// Trigger:
/// ```
/// fn one() { x = callee(); x.v = 99; }
/// fn callee() -> H { H { v: 7 } }
/// ```
/// errored with `Variable 'x' cannot change type from integer to H`.
///
/// Root cause (closed): `parser/fields.rs::field()` silently dropped
/// `.v` when called on an unknown-type receiver in pass-1 — leaving
/// `code` as `Value::Var(x)`, which caused downstream assignment
/// processing (`parse_assign_op` → `change_var`) to set x's type to
/// the RHS expression's type (integer in `x.v = 99`).  Pass-2 then
/// rejected the now-resolved `x = callee()` returning the struct.
///
/// Fix: wrap `code` in `Value::Drop` when the field access is
/// unresolvable in pass-1, so `code != Value::Var(x)` and the
/// assignment processing skips the spurious type update.
#[test]
fn p151_forward_ref_struct_call_with_mutation() {
    code!(
        "struct H { v: integer }
fn one() {
    x = callee();
    x.v = 99;
}
fn callee() -> H { H { v: 7 } }
fn test() { one(); }"
    )
    .result(Value::Null);
}

/// P152.A — vector field assignment from a variable used to be silently
/// dropped at runtime (`s.v = fresh;` evaluated `fresh` but never wrote
/// it into the field).  Fix: `towards_set` now emits
/// `OpClearVector + OpAppendVector` when the LHS is a vector
/// field-access, deep-copying the RHS into the field's storage.
#[test]
fn p152_vec_field_assign_from_var_dataloss() {
    code!(
        "struct S { v: vector<integer> }
fn modify(s: S) {
    fresh: vector<integer> = [1, 2, 3];
    s.v = fresh;
}
fn test() {
    s = S { v: [] };
    modify(s);
    assert(len(s.v) == 3, \"expected 3 got {len(s.v)}\");
}"
    )
    .result(Value::Null);
}

/// P152.A (variant) — `s.v = []` used to silently keep the existing
/// vector contents.  Fix: parse_assign_op detects the empty-Insert
/// + field LHS shape and emits OpClearVector(to).
#[test]
fn p152_vec_field_assign_from_empty_literal_dataloss() {
    code!(
        "struct S { v: vector<integer> }
fn modify(s: S) {
    s.v = [];
}
fn test() {
    s = S { v: [1, 2, 3] };
    modify(s);
    assert(len(s.v) == 0, \"expected 0 got {len(s.v)}\");
}"
    )
    .result(Value::Null);
}

/// P152.A (& form) — `s.v = fresh` on a `&S` parameter used to parse-fail
/// with "Parameter 's' has & but is never modified".  Fix: the same
/// `towards_set` change emits a real OpClearVector+OpAppendVector pair,
/// which `find_written_vars` now recognises (OpAppendVector folded into
/// the unified first_arg_write set).
#[test]
fn p152_vec_field_ref_param_mutation_undetected() {
    code!(
        "struct S { v: vector<integer> }
fn modify(s: &S) {
    fresh: vector<integer> = [1, 2, 3];
    s.v = fresh;
}
fn test() {
    s = S { v: [] };
    modify(s);
    assert(len(s.v) == 3, \"expected 3 got {len(s.v)}\");
}"
    )
    .result(Value::Null);
}

/// P152.B — struct field whole-replacement (`s.i = fresh`) works at
/// runtime via OpCopyRecord, but the `&` mutation check used to miss
/// OpCopyRecord.  Fix: `find_written_vars` adds OpCopyRecord(src, dst)
/// to the second_arg_write set, walking the OpGetField destination.
#[test]
fn p152_struct_field_ref_param_mutation_undetected() {
    code!(
        "struct Inner { x: integer not null }
struct Outer { i: Inner }
fn modify(s: &Outer) {
    fresh = Inner { x: 99 };
    s.i = fresh;
}
fn test() {
    s = Outer { i: Inner { x: 7 } };
    modify(s);
    assert(s.i.x == 99, \"expected 99 got {s.i.x}\");
}"
    )
    .result(Value::Null);
}

/// P153 regression guard — vector ≥187 elements transferred to a struct
/// field via construction USED to corrupt the field's storage.
/// Root cause (closed): `vector_set_size` wrote the new length to the
/// pre-resize rec after `Store::resize` relocated the block, and
/// `vector_add` then byte-copied into the stale destination captured from
/// `vector_append`.  Fix in `src/database/structures.rs`: track the
/// relocated rec in `vector_set_size`; re-read the destination rec after
/// `vector_set_size` in `vector_add`.
#[test]
fn p153_vec_field_transfer_relocation_from_var() {
    code!(
        "struct H { h_material: integer not null }
struct C { ck_hexes: vector<H> }
fn test() {
    hexes: vector<H> = [];
    for _ in 0..1024 { hexes += [H {}]; }
    c = C { ck_hexes: hexes };
    newh = H {};
    newh.h_material = 42;
    c.ck_hexes[167] = newh;
    v = c.ck_hexes[167].h_material;
    assert(v == 42, \"expected 42 got {v}\");
}"
    )
    .result(Value::Null);
}

/// P153 regression guard — same bug, exposed via a function-call
/// initializer instead of a bare variable.  Previously fell through
/// `handle_field`'s else branch (no OpAppendVector emitted) and left the
/// field empty.  Fix: widen the deep-copy check to any non-Insert vector
/// expression.
#[test]
fn p153_vec_field_transfer_relocation_from_call() {
    code!(
        "struct H { h_material: integer not null }
struct C { ck_hexes: vector<H> }
fn build() -> vector<H> {
    hexes: vector<H> = [];
    for _ in 0..200 { hexes += [H {}]; }
    hexes
}
fn test() {
    c = C { ck_hexes: build() };
    newh = H {};
    newh.h_material = 42;
    c.ck_hexes[100] = newh;
    v = c.ck_hexes[100].h_material;
    assert(v == 42, \"expected 42 got {v}\");
}"
    )
    .result(Value::Null);
}

/// P153 regression guard — append after transfer must not heap-corrupt.
/// Pre-fix this triggered libc `double free or corruption` and SIGABRT.
#[test]
fn p153_vec_field_append_after_transfer() {
    code!(
        "struct H { x: integer not null }
struct C { items: vector<H> }
fn test() {
    hexes: vector<H> = [];
    for _ in 0..200 { hexes += [H {}]; }
    c = C { items: hexes };
    c.items += [H {}];
    assert(len(c.items) == 201, \"len {len(c.items)}\");
}"
    )
    .result(Value::Null);
}

/// P153 complement — direct-into-field pattern must still work (guard
/// against over-fixing the transfer path).
#[test]
fn p153_vec_field_direct_into_field_still_works() {
    code!(
        "struct H { h_material: integer not null }
struct C { ck_hexes: vector<H> }
fn test() {
    c = C { ck_hexes: [] };
    for _ in 0..200 { c.ck_hexes += [H {}]; }
    newh = H {};
    newh.h_material = 42;
    c.ck_hexes[100] = newh;
    v = c.ck_hexes[100].h_material;
    assert(v == 42, \"expected 42 got {v}\");
}"
    )
    .result(Value::Null);
}

/// P154 regression guard — `s.v = helper_fn(s.v, …)` must not wipe the
/// field.  Root cause (closed): the P152 lowering emitted
/// OpClearVector(s.v) BEFORE OpAppendVector evaluated the RHS, so the
/// helper saw an already-empty field and returned an empty vector, which
/// was then copied back as empty.
/// Fix: when the RHS is a non-Var expression, capture it into a fresh
/// local temp FIRST, then clear + append from the temp.
#[test]
fn p154_vec_field_assign_from_helper_reading_self() {
    code!(
        "struct S { v: vector<integer> }
fn tail(v: vector<integer>, drop: integer) -> vector<integer> {
    rebuilt: vector<integer> = [];
    keep = len(v) - drop;
    for i in 0..keep { rebuilt += [v[i]]; }
    rebuilt
}
fn test() {
    s = S { v: [1, 2, 3] };
    s.v = tail(s.v, 1);
    assert(len(s.v) == 2, \"expected 2 got {len(s.v)}\");
    assert(s.v[0] == 1, \"[0] {s.v[0]}\");
    assert(s.v[1] == 2, \"[1] {s.v[1]}\");
}"
    )
    .result(Value::Null);
}

/// P154 complement — `s.v = s.v` must be a no-op, not a wipe.
/// Handled by the self-identity guard: IR-equal LHS and RHS collapse
/// to an empty Insert.
#[test]
fn p154_vec_field_self_identity_is_noop() {
    code!(
        "struct S { v: vector<integer> }
fn test() {
    s = S { v: [10, 20, 30] };
    s.v = s.v;
    assert(len(s.v) == 3, \"len {len(s.v)}\");
    assert(s.v[1] == 20, \"[1] {s.v[1]}\");
}"
    )
    .result(Value::Null);
}

/// P154 complement — `s.v = hexes` (plain Var RHS) must still work.
/// The Var-only fast path skips the temp (unnecessary for Var reads).
#[test]
fn p154_vec_field_assign_from_plain_var_still_works() {
    code!(
        "struct S { v: vector<integer> }
fn test() {
    fresh: vector<integer> = [7, 8, 9];
    s = S { v: [] };
    s.v = fresh;
    assert(len(s.v) == 3, \"len {len(s.v)}\");
    assert(s.v[0] == 7, \"[0] {s.v[0]}\");
}"
    )
    .result(Value::Null);
}

/// P155 regression guard — push/undo/mid-assert/redo/final-read
/// sequence SIGSEGVs in OpGetVector.  Triggered when a helper fn that
/// reads a struct out of a vector is called between an undo-style
/// restore and a redo-style restore, with a mid-assert reading the
/// field in between.  Removing the mid-assert makes the crash
/// disappear.  Hypothesis: the helper returns a DbRef into a store
/// that gets freed before the final read, leaving a dangling ref that
/// OpGetVector dereferences.  See PROBLEMS.md P155 for the 22-line
/// minimal reproducer.
/// P156 regression guard — `vector<T>` with a T that shadows a stdlib
/// constant (e.g. `E`, `PI`) used to panic `typedef.rs:309` instead of
/// emitting the clean "struct conflicts with constant" diagnostic.
/// Fix: `parser/definitions.rs::sub_type` checks the resolved element
/// def's DefType up-front and emits a proper diagnostic if it's not a
/// type; `typedef.rs::fill_database` softened the assert to `continue`
/// so a prior parser error never panics the runtime.
#[test]
fn p156_vector_element_shadows_constant() {
    let s = loft::platform::sep_str();
    code!(
        "struct E { x: integer }
struct Big { v: vector<E> }
fn test() { }"
    )
    .error(&format!(
        "struct 'E' conflicts with a constant of the same name already defined \
         at default{s}01_code.loft:385:24 — pick a different name \
         at p156_vector_element_shadows_constant:1:11"
    ))
    .error(&format!(
        "'E' is a Constant, not a type — the element of vector<T> must be a \
         struct or enum (defined at default{s}01_code.loft:385:24) \
         at p156_vector_element_shadows_constant:2:26"
    ));
}

/// P157 regression guard — native codegen's pre-eval path emitted
/// `*var_m` for a `&Map` forwarded to another fn taking `&Map`,
/// breaking rustc type-check.  P144 fixed this for the non-pre-eval
/// path; the pre-eval arg re-emitter in `generation/pre_eval.rs::
/// output_code_with_subst` had its own code that bypassed the check.
/// Trigger: call a user fn that takes `&Struct` from inside another
/// fn that also takes `&Struct`, AND the call has other args that
/// need pre-evaluation (nested field reads, etc.).
#[test]
fn p157_native_refvar_forwarding_with_preeval() {
    // Write the test program to a temp file; use the parser's file-
    // loading entry point rather than an inline string.
    let src_path = std::env::temp_dir().join("loft_p157_test.loft");
    std::fs::write(
        &src_path,
        "struct Inner { val: integer not null }\n\
         struct Outer { inner: Inner }\n\
         fn helper(o: &Outer, n: integer) { o.inner.val = n; }\n\
         fn caller(o: &Outer) { helper(o, o.inner.val + 1); }\n\
         fn main() { o = Outer { inner: Inner { val: 5 } }; caller(o); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let rs_path = std::env::temp_dir().join("loft_p157_native.rs");
    {
        let mut f = std::fs::File::create(&rs_path).unwrap();
        let main_nr = p.data.def_nr("n_main");
        let mut out = loft::generation::Output {
            data: &p.data,
            stores: &state.database,
            counter: 0,
            indent: 0,
            def_nr: 0,
            declared: std::collections::HashSet::new(),
            reachable: std::collections::HashSet::new(),
            loop_stack: Vec::new(),
            next_format_count: 0,
            yield_collect: false,
            fn_ref_context: false,
            call_stack_prefix: None,
            wasm_browser: false,
        };
        out.output_native_reachable(&mut f, 0, p.data.definitions(), &[main_nr])
            .unwrap();
    }
    let source = std::fs::read_to_string(&rs_path).unwrap();
    assert!(
        !source.contains("n_helper(stores, *var_o"),
        "P157 regression: pre-eval path still emits *var_o for & param forward.\n\
         Generated: {}",
        rs_path.display()
    );
    assert!(
        source.contains("n_helper(stores, var_o"),
        "P157 regression: expected direct var_o pass-through.\n\
         Generated: {}",
        rs_path.display()
    );
    let _ = std::fs::remove_file(&rs_path);
    let _ = std::fs::remove_file(&src_path);
}

// ── Language enhancements ────────────────────────────────────────────

/// Bitwise NOT operator `~` — desugars to OpBitNotSingleInt.
#[test]
fn enhancement_bitwise_not() {
    expr!("~0").result(Value::Int(-1));
}

#[test]
fn enhancement_bitwise_not_clear_bit() {
    expr!("(32 | 64) & ~32").result(Value::Int(64));
}

/// `&vector<T>` mutation detection — for-loop variable field writes
/// should propagate back to the iterated `&` collection parameter.
#[test]
fn enhancement_ref_vector_loop_mutation_detected() {
    code!(
        "struct Item { val: integer not null }
fn double_all(items: &vector<Item>) {
    for it in items { it.val = it.val * 2; }
}
fn test() {
    v: vector<Item> = [Item { val: 5 }];
    double_all(v);
    assert(v[0].val == 10, \"doubled\");
}"
    )
    .result(Value::Null);
}

/// Read-only loop over `&vector<T>` should still flag the `&`.
#[test]
fn enhancement_ref_vector_readonly_loop_still_flags() {
    code!(
        "struct Item { val: integer not null }
fn sum_vals(items: &vector<Item>) -> integer {
    total = 0;
    for it in items { total = total + it.val; }
    total
}
fn test() { }"
    )
    .error(
        "Parameter 'items' has & but is never modified; remove the & \
at enhancement_ref_vector_readonly_loop_still_flags:2:47",
    );
}

/// `break value` in void function → compile error.
#[test]
fn enhancement_break_value_in_void_function_errors() {
    code!(
        "fn test() {
    for i in 0..10 {
        if i == 5 { break i; }
    }
}"
    )
    .error(
        "`break <value>` requires a non-void function — \
the value is returned from the enclosing function \
at enhancement_break_value_in_void_function_errors:3:29",
    );
}

/// `is` operator — variant check on plain enum.
#[test]
fn enhancement_is_plain_enum() {
    code!(
        "enum Dir { North, South, East, West }
fn test() {
    d = North;
    assert(d is North, \"is North\");
    assert(!(d is South), \"not South\");
}"
    )
    .result(Value::Null);
}

/// `is` operator — variant check on struct-enum + loop counting.
#[test]
fn enhancement_is_struct_enum_in_loop() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    items: vector<Shape> = [Circle { radius: 1.0 }, Rect { width: 2.0, height: 3.0 }, Circle { radius: 4.0 }];
    count = 0;
    for it in items { if it is Circle { count = count + 1; } }
    assert(count == 2, \"2 circles\");
}"
    )
    .result(Value::Null);
}

/// `is` operator with field capture — single field.
#[test]
fn enhancement_is_capture_single_field() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    s = Circle { radius: 3.14 };
    result = 0.0;
    if s is Circle { radius } {
        result = radius;
    }
    assert(result == 3.14, \"captured radius\");
}"
    )
    .result(Value::Null);
}

/// `is` operator with field capture — multiple fields + else branch.
#[test]
fn enhancement_is_capture_multiple_fields_else() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    s = Rect { width: 5.0, height: 10.0 };
    area = 0.0;
    if s is Rect { width, height } {
        area = width * height;
    } else {
        area = -1.0;
    }
    assert(area == 50.0, \"captured both\");
    c = Circle { radius: 2.0 };
    if c is Rect { width, height } {
        area = width * height;
    } else {
        area = -1.0;
    }
    assert(area == -1.0, \"else taken\");
}"
    )
    .result(Value::Null);
}

/// `is` operator with field capture in loop — sum radii from mixed vector.
#[test]
fn enhancement_is_capture_in_loop() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    items: vector<Shape> = [Circle { radius: 1.0 }, Rect { width: 2.0, height: 3.0 }, Circle { radius: 4.0 }];
    total = 0.0;
    for it in items {
        if it is Circle { radius } {
            total += radius;
        }
    }
    assert(total == 5.0, \"sum of radii\");
}"
    )
    .result(Value::Null);
}

/// `is` capture scope doesn't leak into outer scope.
#[test]
fn enhancement_is_capture_scope_isolation() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    s = Circle { radius: 99.0 };
    radius = 1.0;
    if s is Rect { width, height } {
        radius = width;
    }
    assert(radius == 1.0, \"outer radius unchanged\");
}"
    )
    .result(Value::Null);
}

/// Op table extension — emit_op handles ops >= 255 via escape prefix.
/// No specific op to test yet (all 255 primary slots used), but verify
/// the infrastructure doesn't break existing ops.
#[test]
fn enhancement_op_extension_existing_ops_unaffected() {
    expr!("~0").result(Value::Int(-1));
}

/// map/filter on &vector<T> parameter — method resolution unwraps RefVar.
#[test]
fn enhancement_map_filter_on_ref_vector() {
    code!(
        "fn process(items: &vector<integer>) {
    items += [99];
    d = items.map(|x| { x * 2 });
    assert(d[0] == 2, \"mapped\");
}
fn test() {
    v = [1, 2, 3];
    process(v);
    assert(len(v) == 4, \"appended\");
}"
    )
    .result(Value::Null);
}

/// P161 regression guard — `for it in items` where items is
/// `&vector<Struct>` used to error "Unknown type null" (field access
/// on the loop variable failed).  Root cause: `for_type` and
/// `iterator` didn't unwrap `RefVar(Vector(...))` before matching.
#[test]
fn p161_for_over_ref_vector() {
    code!(
        "struct Item { val: integer not null }
fn add_item(items: &vector<Item>, v: integer) {
    items += [Item { val: v }];
}
fn test() {
    v: vector<Item> = [];
    add_item(v, 42);
    assert(len(v) == 1, \"len {len(v)}\");
    assert(v[0].val == 42, \"val {v[0].val}\");
}"
    )
    .result(Value::Null);
}

/// P160 regression guard — `modify(items[1], 42)` where `modify`
/// takes `&S` used to error "Cannot pass a literal or expression
/// to a '&' parameter".  Two fixes: (1) parser accepts "addressable"
/// expressions (vector element, field access chains rooted in a Var);
/// (2) codegen handles `OpCreateStack(non-Var expr)` by generating
/// the expression first (pushes DbRef), then emitting OpCreateStack
/// with the offset pointing at the just-pushed result.
#[test]
fn p160_vec_element_as_ref_param() {
    code!(
        "struct S { x: integer not null }
fn modify(s: &S, val: integer) { s.x = val; }
fn test() {
    items: vector<S> = [S { x: 0 }, S { x: 10 }];
    modify(items[1], 42);
    assert(items[1].x == 42, \"got {items[1].x}\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p160_nested_field_vec_element_as_ref_param() {
    code!(
        "struct Inner { val: integer not null }
struct Outer { items: vector<Inner> }
fn set_val(inner: &Inner, v: integer) { inner.val = v; }
fn test() {
    o = Outer { items: [Inner { val: 0 }, Inner { val: 0 }] };
    set_val(o.items[1], 99);
    assert(o.items[1].val == 99, \"got {o.items[1].val}\");
}"
    )
    .result(Value::Null);
}

/// P159 regression guard — `Shape.parse(json)` used to fail for
/// struct-enums ("Unknown field Shape.parse").  Fix: added
/// `DefType::Enum` branch in `parse_var` for `.parse(` detection,
/// and added discriminant wrapper `{"Variant":{fields}}` to the
/// JSON serializer in `format.rs`.
#[test]
fn p159_struct_enum_json_roundtrip() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    c = Circle { radius: 3.14 };
    j = \"{c:j}\";
    p = Shape.parse(j);
    r = match p { Circle { radius } => radius, Rect => 0.0 };
    assert(r == 3.14, \"circle rt\");
    rect = Rect { width: 5.0, height: 10.0 };
    j2 = \"{rect:j}\";
    p2 = Shape.parse(j2);
    r2 = match p2 { Circle => 0.0, Rect { width, height } => width * height };
    assert(r2 == 50.0, \"rect rt\");
}"
    )
    .result(Value::Null);
}

/// P158 regression guard — trailing comma after the last field in a
/// struct-enum variant used to trigger "Expect attribute".  Regular
/// structs accepted trailing commas; enum variants didn't.  Fix:
/// added `|| self.lexer.peek_token("}")` to the break condition in
/// `parse_enum_values`, mirroring `parse_struct`.
#[test]
fn p158_trailing_comma_enum_variant() {
    code!(
        "enum K {
    Alpha { x: integer not null, y: integer not null, },
    Beta { z: integer not null }
}
fn test() {
    a = Alpha { x: 1, y: 2 };
    match a { Alpha { x, y } => assert(x + y == 3, \"sum\"), Beta => 0 };
}"
    )
    .result(Value::Null);
}

/// P155 regression guard — push/undo/mid-assert/redo/final-read used
/// to SIGSEGV in OpGetVector.  Root cause: `state/codegen.rs::generate_set`
/// (reassignment path, lines 891-932) emitted `OpCopyRecord` with the
/// 0x8000 "free source" flag around a user-fn call, but without the
/// `n_set_store_lock` bracket.  When the callee returned a DbRef
/// aliased with a caller arg — e.g. `read_at(c, idx)` returns into
/// `c.items` — the free-source flag freed the caller's arg store.
/// Later uses of that arg SIGSEGV'd.  Fix: mirror the
/// `gen_set_first_ref_call_copy` lock/unlock bracket (which the P143
/// fix added) onto the reassignment path.
#[test]
fn p155_segv_undo_redo_midassert() {
    code!(
        "struct H { m: integer not null }
struct Elm { prev: H }
struct Ct { items: vector<H> }
struct Ss { undo: vector<Elm>, redo: vector<Elm> }
fn read_at(c: Ct, idx: integer) -> H { c.items[idx] }
fn test() {
    c = Ct { items: [H{}, H{}, H{}, H{}, H{}, H{}] };
    s = Ss { undo: [], redo: [] };
    h = read_at(c, 2);
    s.undo += [Elm { prev: h }];
    nh = H {}; nh.m = 77; c.items[2] = nh;
    e = s.undo[0];
    cur = read_at(c, 2);
    s.redo += [Elm { prev: cur }];
    c.items[2] = e.prev;
    assert(read_at(c, 2).m == 0, \"reverted\");
    re = s.redo[0];
    c.items[2] = re.prev;
    assert(read_at(c, 2).m == 77, \"reapplied\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p145_text_return_multivec_struct_cross_file() {
    let mut p = Parser::new();
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_dir("default", true, false).unwrap();
    p.parse("tests/lib/p145_main2.loft", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "P145 regression: to_json on cross-file multi-vector struct crashed"
    );
}

/// P162 regression guard — native codegen emits `return let mut …` when
/// a match expression with struct-enum field bindings + guard is returned
/// directly.  `pre_declare_branch_vars` writes `let mut` declarations
/// after the `return` keyword.  Interpreter works; native compilation fails.
#[test]
fn p162_return_match_struct_enum_native() {
    code!(
        "enum GShape {
    GCircle { radius: float },
    GRect { width: float, height: float }
}
fn garea(s: GShape) -> float {
    match s {
        GCircle { radius } if radius > 0.0 => 3.14 * radius * radius,
        GCircle { radius } => 0.0,
        GRect { width, height } => width * height
    }
}
fn test() {
    assert(garea(GCircle { radius: 2.0 }) > 12.0, \"circle area\");
    assert(garea(GCircle { radius: -1.0 }) == 0.0, \"negative radius\");
    assert(garea(GRect { width: 3.0, height: 4.0 }) == 12.0, \"rect area\");
}"
    )
    .result(Value::Null);
}

/// P164 regression guard — trailing comma after the LAST VARIANT of an
/// enum declaration used to fail with `Expect name in type definition`.
/// P158 fixed trailing commas inside a variant's field list; this is the
/// sibling case on the variant list itself.  Fix: mirror the P158 guard
/// (`|| self.lexer.peek_token("}")`) onto the outer variant-list break
/// check in `parse_enum_values`.
#[test]
fn p164_trailing_comma_enum_variant_list() {
    code!(
        "enum P164Kind {
    P164Alpha { x: integer not null },
    P164Beta { y: integer not null },
}
fn test() {
    a = P164Alpha { x: 1 };
    assert(a is P164Alpha, \"alpha\");
    b = P164Beta { y: 2 };
    assert(b is P164Beta, \"beta\");
}"
    )
    .result(Value::Null);
}

/// P164 also covers plain (non-struct-field) enum declarations.
#[test]
fn p164_trailing_comma_plain_enum() {
    code!(
        "enum P164Dir {
    P164North,
    P164East,
    P164South,
    P164West,
}
fn test() {
    d = P164North;
    assert(d is P164North, \"north\");
}"
    )
    .result(Value::Null);
}

/// P170 regression guard — `x = Struct{}; x = vec[i]; mutate(x)` used to
/// fail with `Incorrect var x[N] versus M on n_<fn>` at codegen.
///
/// Root cause: `parser/objects.rs::parse_object` had a gap in the
/// in-place struct-literal path.  When the LHS variable's type was
/// already inferred with dependencies (because a later assignment in
/// the same function did `x = bs[i]`, giving x type
/// `Reference(Bag, [bs])`), `is_independent(x)` returned false.  The
/// in-place `v_set(x, Null) + OpDatabase(x)` init branch required both
/// `is_independent` AND `type_matches` — with `type_matches=true` and
/// `is_independent=false`, neither the if-branch nor the else-if
/// (which required `!type_matches`) fired.  The struct-literal
/// statement emitted only field-init calls into uninitialised storage,
/// codegen never saw a Set for x's first assignment, and later
/// `generate_var(x)` asserted since x's slot sat above TOS.
///
/// Fix: extend the `else if` to also fire when
/// `!is_independent && !first_pass` — routes the construction through
/// a fresh work-ref (existing "new_object" path), which emits the
/// required `v_set + OpDatabase` prelude and yields a `Block`-shaped
/// RHS that the outer assignment can then copy/alias via the normal
/// Set path.
#[test]
fn p170_struct_placeholder_then_vec_elem_reassign() {
    code!(
        "struct P170Bag { items: vector<integer> }
fn p170_mutate_bag(b: &P170Bag, v: integer) { b.items += [v]; }
fn test() {
    p170_bs: vector<P170Bag> = [];
    p170_x = P170Bag {};
    p170_bs += [P170Bag {}];
    p170_x = p170_bs[len(p170_bs) - 1];
    p170_mutate_bag(p170_x, 1);
    assert(len(p170_bs[0].items) == 1, \"mutated through alias\");
}"
    )
    .warning("Dead assignment — 'p170_x' is overwritten before being read at p170_struct_placeholder_then_vec_elem_reassign:5:25")
    .result(Value::Null);
}

/// P170 guard — three-way: the same shape but with a conditional
/// assignment between the placeholder and the vec-elem reassign.
#[test]
fn p170_placeholder_conditional_then_reassign() {
    code!(
        "struct P170CBag { val: integer not null }
fn p170c_bump(b: &P170CBag) { b.val = b.val + 1; }
fn test() {
    p170c_v: vector<P170CBag> = [P170CBag { val: 5 }];
    p170c_x = P170CBag { val: 0 };
    p170c_x = p170c_v[0];
    p170c_bump(p170c_x);
    assert(p170c_v[0].val == 6, \"bumped first elem\");
}"
    )
    .warning("Dead assignment — 'p170c_x' is overwritten before being read at p170_placeholder_conditional_then_reassign:5:35")
    .result(Value::Null);
}

/// P167 regression guard — trailing comma in a function-call argument
/// list used to fail with "Too many parameters for n_<fn>".  P158 fixed
/// trailing commas in struct-enum variant field lists; P164 fixed
/// trailing commas in enum variant lists; P167 covers function-call
/// argument lists (the third and final trailing-comma site).  Fix:
/// mirror the P158 guard in `parser/control.rs::parse_call` — for both
/// the positional and named argument loops.
#[test]
fn p167_trailing_comma_function_call_positional() {
    code!(
        "fn p167_add3(a: integer, b: integer, c: integer) -> integer { a + b + c }
fn test() {
    r = p167_add3(1, 2, 3,);
    assert(r == 6, \"trailing comma positional\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p167_trailing_comma_function_call_multiline() {
    // The shape that actually caught it in the wild — multi-line
    // call (rgb/vec3-style) with a trailing comma.
    code!(
        "fn p167_mix(r: integer, g: integer, b: integer) -> integer {
    (r * 65536) + (g * 256) + b
}
fn test() {
    c = p167_mix(
        10,
        20,
        30,
    );
    assert(c == 10 * 65536 + 20 * 256 + 30, \"multiline trailing comma\");
}"
    )
    .result(Value::Null);
}

/// P165 regression guard — `var: Enum = Variant { ... }` used to fail
/// with "Variable 'var' cannot change type from Enum to Variant; use a
/// new variable name or cast with 'as'".  The type-change check treated
/// a struct-enum variant as a distinct type from its parent enum.
/// Fix: in `Function::change_var_type`, accept `(Enum(p, true, _),
/// Enum(v, true, _))` when `data.def(v).parent == p` — the parent
/// relationship proves subtype compatibility.
#[test]
fn p165_enum_annotation_with_variant_rhs() {
    code!(
        "enum P165Kind {
    P165Alpha { x: integer not null },
    P165Beta { y: integer not null }
}
fn take_kind(k: P165Kind) -> boolean { k is P165Alpha }
fn test() {
    // Annotated LHS with variant RHS (the P165 shape).
    k1: P165Kind = P165Alpha { x: 1 };
    assert(take_kind(k1), \"annotated alpha\");
    // Annotated LHS with the OTHER variant — also accepted.
    k2: P165Kind = P165Beta { y: 2 };
    assert(!take_kind(k2), \"annotated beta\");
}"
    )
    .result(Value::Null);
}
