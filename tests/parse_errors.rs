// Copyright (c) 2022-2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

extern crate loft;

use loft::data::Value;

mod testing;

#[test]
fn wrong_parameter() {
    code!("fn def(i: integer) { }\nfn test() { def(true); }")
        .error("boolean should be integer on call to def at wrong_parameter:1:40")
        .warning("Parameter i is never read at wrong_parameter:1:21");
}

#[test]
fn wrong_boolean() {
    code!("enum EType{ Val }\nfn def(t: EType) {}\nfn test() { def(true); }")
        .error("boolean should be EType on call to def at wrong_boolean:2:38")
        .warning("Parameter t is never read at wrong_boolean:2:19");
}

#[test]
fn unknown_var() {
    code!("fn test() { a == 1 }").error("Unknown variable 'a' at unknown_var:1:19");
}

/// S1: a misspelled variable name must produce a clear "Unknown variable" diagnostic
/// on the second pass without creating a ghost variable that could cause cascading errors.
#[test]
fn typo_var_name() {
    code!("fn test() { count = 0; cound + 1; }")
        .error("Unknown variable 'cound' — did you mean 'count'? at typo_var_name:1:33")
        .warning("Variable count is never read at typo_var_name:1:20");
}

#[test]
fn use_before_define() {
    code!("fn test() { if a == 1 { panic(); }; a = 1; }")
        .error("Unknown variable 'a' at use_before_define:1:22");
}

#[test]
fn wrong_text() {
    code!("fn rout(a: integer) -> integer {if a > 4 {return \"a\"} 2}\nfn test() {}")
        .error("text should be integer on return at wrong_text:1:53");
}

#[test]
fn empty_return() {
    code!("fn routine(a: integer) -> integer {if a > 4 {return} 1}\nfn test() {}")
        .error("Expect expression after return at empty_return:1:53");
}

#[test]
fn wrong_void() {
    code!("fn rout(a: integer) {if a > 4 {return 12}}\nfn test() {}")
        .error("Expect no expression after return at wrong_void:1:42");
}

#[test]
fn wrong_break() {
    code!("fn test() {break}").error("Cannot break outside a loop at wrong_break:1:18");
}

#[test]
fn wrong_continue() {
    code!("fn test() {continue}").error("Cannot continue outside a loop at wrong_continue:1:21");
}

#[test]
fn double_field_name() {
    code!("fn test(a: integer, b: integer, a: integer) { if a>b {} }")
        .error("Double attribute 'test.a' at double_field_name:1:35");
}

#[test]
fn incorrect_name() {
    code!("type something;\nfn something(a: integer) {}")
        .error("Cannot redefine 'something' (already defined at incorrect_name:1:16) at incorrect_name:2:27")
        .error("Expect type definitions to be in camel case style at incorrect_name:1:16");
}

#[test]
fn wrong_compare() {
    code!("enum EType{ V1 }\nenum Next{ V2 }\nfn test() { V1 == V2; }")
        .error("No matching operator '==' on 'EType' and 'Next' at wrong_compare:3:21");
}

#[test]
fn wrong_plus() {
    code!("fn test() {(1 + \"a\")}")
        .error("No matching operator '+' on 'integer' and 'text' at wrong_plus:1:20");
}

#[test]
fn wrong_if() {
    code!("fn test() {if 1 > 0 { 2 } else {\"a\"}\n}")
        .error("text should be integer on else at wrong_if:2:1");
}

#[test]
fn wrong_assign() {
    code!("enum EType { V1 }\nfn test() {a = 1; a = V1 }")
        .error("Variable 'a' cannot change type from integer to EType; use a new variable name or cast with 'as' at wrong_assign:2:27");
}

#[test]
fn mixed_enums() {
    code!("enum E1 { V1 }\nenum E2 { V2 }\nfn a(v: E2) -> E2 { v }\nfn test() { a(V1) }")
        .error("E1 should be E2 on call to a at mixed_enums:4:19");
}

#[test]
fn wrong_cast() {
    code!("enum E1 { V1 }\nfn test() { V1 as float }")
        .error("Unknown cast from E1 to float at wrong_cast:2:26");
}

#[test]
fn field_type() {
    code!("struct Rec { v: u8 }\nfn test() { r = Rec { v: \"a\" }; assert(\"{r}\" == \"{{v:\\\"a\\\"}}\", \"Object\"); }")
        .error("Cannot write integer(0, 255) on field Rec.v:text at field_type:2:31");
}

#[test]
fn key_field() {
    code!(
        "struct Rec { n: text, v: u16 }
struct Coll { d: vector<Rec>, h: hash<Rec[n]> }
fn test() {
  s = Coll { d:[Rec {n: \"a\", v:12} ] };
  s.d[0].v = 13;
  s.d[0].n = \"b\";
}"
    )
    .error("Cannot write to key field Rec.n create a record instead at key_field:6:18");
}

#[test]
fn undefined() {
    code!("fn test(v: V) -> V { v }").error("Undefined type V at undefined:1:14");
}

#[test]
fn undefined_return() {
    code!("fn test(v: integer) -> V { v }").error("Undefined type V at undefined_return:1:27");
}

#[test]
fn undefined_as() {
    code!("fn test(v: integer) -> long { v as V }").error("Undefined type V at undefined_as:1:39");
}

#[test]
fn undefined_enum() {
    code!("enum E1 { V1 }\nfn test(v: E1) -> boolean { v > V2 }")
        .error("Unknown variable 'V2' — did you mean 'v'? at undefined_enum:2:37");
}

#[test]
fn unknown_sizeof() {
    code!("fn test() { sizeof(C); }")
        .error("Expect a variable or type after sizeof at unknown_sizeof:1:22")
        .error("Unknown variable 'C' at unknown_sizeof:1:22");
}

#[test]
fn index_non_indexable() {
    code!("fn test() { v = 5; v[1]; }").error("Indexing a non vector — keyed collections (hash/sorted/index/spacial) have no generic-constructor expression; declare them as a struct field and initialise via a vector literal: `struct Db { h: hash<Row[id]> }; db = Db { h: [Row { id: 1 }] }` at index_non_indexable:1:23");
}

#[test]
fn fn_name_as_param_type() {
    code!("fn helper() {}\nfn test(v: helper) {}")
        .error("Undefined type helper at fn_name_as_param_type:2:19");
}

#[test]
fn fn_name_as_typedef() {
    code!("fn helper() {}\ntype Alias = helper;\nfn test() { 1 }")
        .error("Undefined type helper at fn_name_as_typedef:2:21");
}

#[test]
fn missing_variant_impl() {
    // area() is only defined for Circle; Rect has no area() — expect a warning at Rect's definition.
    code!(
        "enum Shape {\n    Circle { r: float },\n    Rect { w: float, h: float }\n}\nfn area(self: Circle) -> float { self.r * self.r }\nfn test() { 1 + 1; }"
    )
    .warning("no implementation of 'area' for variant 'Rect' at missing_variant_impl:3:11");
}

#[test]
fn stub_suppresses_missing_variant_warning() {
    // Rect has an empty-body stub — no warning should be emitted for either variant.
    code!(
        "enum Shape {\n    Circle { r: float },\n    Rect { w: float, h: float }\n}\nfn area(self: Circle) -> float { self.r * self.r }\nfn area(self: Rect) -> float { }\nfn test() { 1 + 1; }"
    );
    // no .warning() → assert_diagnostics expects an empty diagnostic set
}

// Direct call to stub (empty-body variant method) must not panic
#[test]
fn direct_call_to_stub() {
    // Calling r.area() where area is a stub for Rect must compile without panic.
    code!(
        "enum Shape { Circle { r: float }, Rect { w: float, h: float } }
fn area(self: Circle) -> float { self.r * self.r }
fn area(self: Rect) -> float { }
fn test() { r = Rect { w: 3.0, h: 4.0 }; r.area(); }"
    );
    // no .error() → compilation must succeed
}

// Direct call to a method that exists on the enum but has no implementation for the variant
#[test]
fn direct_call_unimplemented_variant() {
    // r.area() where Rect has no area method at all must give an error, not a panic.
    code!(
        "enum Shape { Circle { r: float }, Rect { w: float, h: float } }
fn area(self: Circle) -> float { self.r * self.r }
fn test() { r = Rect { w: 3.0, h: 4.0 }; r.area(); }"
    )
    .error("Unknown field Rect.area at direct_call_unimplemented_variant:3:49")
    .warning(
        "no implementation of 'area' for variant 'Rect' at direct_call_unimplemented_variant:1:41",
    );
}

// --- parallel_for: extra context-argument count validation ---

#[test]
fn parallel_for_missing_context_arg() {
    // Worker expects 1 extra context arg (m) but none is provided.
    code!(
        "struct Item { v: integer } \
         fn scale(r: const Item, m: integer) -> integer { r.v * m } \
         fn test() { items = [Item{v:1}]; parallel_for(scale, items, 1); }"
    )
    .error("parallel_for: wrong number of extra arguments: worker expects 1, got 0 at parallel_for_missing_context_arg:1:150");
}

#[test]
fn parallel_for_unexpected_context_arg() {
    // Worker expects 0 extra args but 1 is provided.
    code!(
        "struct Item { v: integer } \
         fn id(r: const Item) -> integer { r.v } \
         fn test() { items = [Item{v:1}]; mult = 3; parallel_for(id, items, 1, mult); }"
    )
    .error("parallel_for: wrong number of extra arguments: worker expects 0, got 1 at parallel_for_unexpected_context_arg:1:144");
}

#[test]
fn parallel_for_too_many_context_args() {
    // Worker expects 1 extra arg but 2 are provided.
    code!(
        "struct Item { v: integer } \
         fn scale(r: const Item, m: integer) -> integer { r.v * m } \
         fn test() { items = [Item{v:1}]; a = 2; b = 3; parallel_for(scale, items, 1, a, b); }"
    )
    .error("parallel_for: wrong number of extra arguments: worker expects 1, got 2 at parallel_for_too_many_context_args:1:170");
}

// --- For-loop mutation guards ---

#[test]
fn add_to_iterated_vector() {
    // `v += elem` where v is currently being iterated is unsound: get_vector re-reads
    // the length each step, so new elements are visited — risking an infinite loop.
    code!("fn test() { v = [1, 2, 3]; for e in v { v += [4]; } }")
        .warning("Variable e is never read at add_to_iterated_vector:1:40")
        .error("Cannot add elements to 'v' while it is being iterated — use a separate collection or add after the loop at add_to_iterated_vector:1:47");
}

#[test]
fn remove_from_iterated_vector_is_allowed() {
    // `e#remove` adjusts the iterator position after removal — it is the designed,
    // safe way to remove the current element during iteration.  No error expected.
    code!("fn test() { v = [1, 2, 3]; for e in v if e > 1 { e#remove; } }");
}

#[test]
fn add_to_outer_loop_iterated() {
    // The guard catches mutations of a collection iterated by an *outer* loop too.
    code!(
        "fn test() { v = [1, 2, 3]; for e in v { for n in 1..3 { v += [n]; } } }"
    )
    .warning("Variable e is never read at add_to_outer_loop_iterated:1:40")
    .error("Cannot add elements to 'v' while it is being iterated — use a separate collection or add after the loop at add_to_outer_loop_iterated:1:63");
}

// T1-10: unused loop variable warning
#[test]
fn unused_loop_var_range() {
    // Loop variable never read in body — should warn.
    code!("fn test() { total = 0; for i in 0..3 { total += 1; } assert(total == 3, \"t\"); }")
        .warning("Variable i is never read at unused_loop_var_range:1:39");
}

#[test]
fn unused_loop_var_int_vector() {
    // Integer-element vector loop — should warn when element never read.
    code!(
        "fn test() {
  items = [1, 2, 3];
  total = 0;
  for item in items { total += 1; }
  assert(total == 3, \"t\");
}"
    )
    .warning("Variable item is never read at unused_loop_var_int_vector:4:22");
}

#[test]
fn unused_loop_var_suppressed_by_underscore() {
    // _ prefix suppresses the warning — consistent with other unused-variable rules.
    code!(
        "fn test() {
  items = [1, 2, 3];
  total = 0;
  for _item in items { total += 1; }
  assert(total == 3, \"t\");
}"
    );
}

#[test]
fn unused_loop_var_used_is_silent() {
    // No warning when the loop variable is actually read.
    code!(
        "fn test() {
  items = [1, 2, 3];
  total = 0;
  for item in items { total += item; }
  assert(total == 6, \"t\");
}"
    );
}

/// Unreachable code after return.
#[test]
fn unreachable_after_return() {
    code!(
        "fn compute() -> integer { return 1; x = 2; x }
fn test() { assert(compute() == 1, \"ok\"); }"
    )
    .warning("Unreachable code after return at unreachable_after_return:1:38");
}

/// Unreachable code after break.
#[test]
fn unreachable_after_break() {
    code!(
        "fn test() {
    for i in 1..5 {
        break;
        assert(false, \"unreachable\");
    };
}"
    )
    .warning("Variable i is never read at unreachable_after_break:2:20")
    .warning("Unreachable code after break at unreachable_after_break:4:15");
}

/// Unreachable code after continue.
#[test]
fn unreachable_after_continue() {
    code!(
        "fn test() {
    for i in 1..5 {
        continue;
        assert(false, \"unreachable\");
    };
}"
    )
    .warning("Variable i is never read at unreachable_after_continue:2:20")
    .warning("Unreachable code after continue at unreachable_after_continue:4:15");
}

/// No warning: return inside an if branch does not terminate the block.
#[test]
fn no_unreachable_after_branch_return() {
    code!(
        "fn compute(x: integer) -> integer {
    if x > 0 { return x };
    0
}
fn test() { assert(compute(5) == 5, \"ok\"); }"
    );
}

#[test]
fn spacial_not_implemented() {
    // C7/P22: spacial<T> is a reserved keyword; its diagnostic now
    // surfaces the 1.1+ timeline so a user who typed it knows when
    // the feature ships and which substitute to reach for.
    code!("struct Point { x: integer, y: integer }\nstruct World { pts: spacial<Point, x, y> }\nfn test() {}")
        .error("spacial<T> is planned for 1.1+; until then use sorted<T> or index<T> for ordered lookups at spacial_not_implemented:2:43");
}

/// C7/P22 regression guard: the diagnostic also fires for a local
/// variable (not just a struct field) and carries the same hint.
#[test]
fn spacial_not_implemented_in_local() {
    code!("struct Point { x: integer, y: integer }\nfn test() { xs: spacial<Point, x, y> = []; }")
        .error("spacial<T> is planned for 1.1+; until then use sorted<T> or index<T> for ordered lookups at spacial_not_implemented_in_local:2:39");
}

/// F57: write_file on a struct with a collection-type field must produce a compile error.
#[test]
fn write_file_collection_field() {
    code!(
        "struct Item { x: integer }\n\
         struct Record { items: sorted<Item[x]> }\n\
         fn test() {\n\
           f = file(\"out.bin\");\n\
           f#format = LittleEndian;\n\
           r = Record{};\n\
           f += r;\n\
         }"
    )
    .error("write_file: 'Record' has collection field 'items'; use a plain struct for serialisation at write_file_collection_field:7:8");
}

/// F57: read_file with `as T` where T has a collection-type field must produce a compile error.
#[test]
fn read_file_collection_field() {
    code!(
        "struct Item { x: integer }\n\
         struct Record { items: sorted<Item[x]> }\n\
         fn test() {\n\
           f = file(\"out.bin\");\n\
           f#format = LittleEndian;\n\
           _ = f#read(8) as Record;\n\
         }"
    )
    .error("read_file: 'Record' has collection field 'items'; use a plain struct for serialisation at read_file_collection_field:6:25");
}

/// T1-22: function with `not null` return type that may fall through warns.
#[test]
fn missing_return_not_null() {
    code!(
        "fn classify(n: integer) -> text not null {
    if n > 0 { return \"pos\" };
}
fn test() { classify(1); }"
    )
    .warning(
        "Not all code paths return a value — function 'classify' may return null at missing_return_not_null:4:3",
    );
}

/// T1-22: if/else where both branches return — no error, no warning.
/// (This currently produces a false-positive "void should be integer" error.)
#[test]
fn all_paths_return_if_else() {
    code!(
        "fn classify(n: integer) -> integer {
    if n > 0 { return 1 } else { return -1 }
}
fn test() { assert(classify(5) == 1, \"ok\"); }"
    );
}

/// T1-22: if/else both return with `not null` — no warning.
#[test]
fn all_paths_return_not_null() {
    code!(
        "fn classify(n: integer) -> integer not null {
    if n > 0 { return 1 } else { return -1 }
}
fn test() { assert(classify(5) == 1, \"ok\"); }"
    );
}

/// T1-22: function with `not null` return ending in a direct return — no warning.
#[test]
fn direct_return_not_null() {
    code!(
        "fn always() -> integer not null {
    return 42
}
fn test() { assert(always() == 42, \"ok\"); }"
    );
}

/// T1-22: last expression in block is non-void — counts as definitely-returns, no warning.
#[test]
fn implicit_return_not_null() {
    code!(
        "fn double(n: integer) -> integer not null {
    n * 2
}
fn test() { assert(double(3) == 6, \"ok\"); }"
    );
}

#[test]
fn shadow_different_type() {
    // Error when a for-loop variable reuses a name with a different type.
    code!(
        "fn test() {
    x = 1.5;
    v = [1, 2, 3];
    for x in v { }
}"
    )
    .error("loop variable 'x' has type integer but was previously used as float at shadow_different_type:4:17")
    .warning("Variable x is never read at shadow_different_type:2:8");
}

#[test]
fn shadow_same_type_ok() {
    // C61.local: same-type shadow of an outer local is now rejected at
    // parse time — renaming the loop variable or dropping the outer
    // local are the two documented fixes.  Previously this test was
    // named "_ok" because the reuse silently succeeded; it now pins the
    // rejection to prevent regression.
    code!(
        "fn test() {
    x = 10;
    v = [1, 2, 3];
    for x in v { }
    println(\"{x}\");
}"
    )
    .error(
        "loop variable 'x' shadows a local named 'x' — rename the loop \
         variable (e.g. loop_x) or drop the outer `x` if it was a dead \
         placeholder; loft does not block-scope loop variables at \
         shadow_same_type_ok:4:17",
    );
}

#[test]
fn if_expr_without_else() {
    // Using if as a value expression without else is a compile error.
    code!(
        "fn test() {
    x = if true { 42 };
    println(\"{x}\");
}"
    )
    .error("If-expression produces a value but has no else clause; add an else branch or make the body a statement at if_expr_without_else:2:24");
}

#[test]
fn if_expr_with_else_ok() {
    // If-expression with else is fine.
    code!(
        "fn test() {
    x = if true { 42 } else { 0 };
    assert(x == 42, \"ok\");
}"
    );
}

#[test]
fn if_statement_without_else_ok() {
    // If-statement (void body) without else is fine — no error.
    code!(
        "fn test() {
    x = 10;
    if x > 5 {
        println(\"{x}\");
    }
}"
    );
}

#[test]
fn type_cycle_self() {
    // Self-referential struct is a compile error.
    code!("struct Node { val: integer, next: Node }\nfn test() { }")
        .error("Struct 'Node' contains itself (directly or indirectly) — use reference<Node> to break the cycle at type_cycle_self:1:14");
}

#[test]
fn type_cycle_indirect() {
    // Mutually recursive structs are a compile error.
    code!(
        "struct A { val: integer, b: B }
struct B { val: integer, a: A }
fn test() { }"
    )
    .error("Struct 'A' contains itself (directly or indirectly) — use reference<A> to break the cycle at type_cycle_indirect:1:11")
    .error("Struct 'B' contains itself (directly or indirectly) — use reference<B> to break the cycle at type_cycle_indirect:2:11");
}

#[test]
fn non_cyclic_nested_struct_ok() {
    // Non-cyclic struct nesting is fine.
    code!(
        "struct Inner { x: integer }
struct Outer { i: Inner, y: integer }
fn test() {
    o = Outer { i: Inner { x: 1 }, y: 2 };
    assert(o.i.x == 1, \"nested\");
}"
    );
}

#[test]
fn keyword_sizeof_as_fn() {
    code!("fn sizeof() {}\nfn test() {}")
        .error("Expect name in function definition at keyword_sizeof_as_fn:1:10")
        .error("Syntax error: unexpected 'sizeof' at keyword_sizeof_as_fn:1:10");
}

// A10: `fields` is no longer a keyword — it can be used as a function name.

#[test]
fn keyword_debug_assert_as_fn() {
    code!("fn debug_assert() {}\nfn test() {}")
        .error("Expect name in function definition at keyword_debug_assert_as_fn:1:16")
        .error("Syntax error: unexpected 'debug_assert' at keyword_debug_assert_as_fn:1:16");
}

#[test]
fn keyword_assert_as_fn() {
    code!("fn assert() {}\nfn test() {}")
        .error("Expect name in function definition at keyword_assert_as_fn:1:10")
        .error("Syntax error: unexpected 'assert' at keyword_assert_as_fn:1:10");
}

#[test]
fn keyword_panic_as_fn() {
    code!("fn panic() {}\nfn test() {}")
        .error("Expect name in function definition at keyword_panic_as_fn:1:9")
        .error("Syntax error: unexpected 'panic' at keyword_panic_as_fn:1:9");
}

/// P5.3: operator on generic type T produces a generic-specific error.
#[test]
fn generic_operator_error() {
    code!("fn bad<T>(x: T, y: T) -> T { x + y }\nfn test() {}").error(
        "generic type T: operator '+' requires a concrete type at generic_operator_error:1:36",
    );
}

/// P5.3: field access on generic type T produces a generic-specific error.
#[test]
fn generic_field_error() {
    code!("fn bad<T>(x: T) -> integer { x.name }\nfn test() {}")
        .error("generic type T: field access requires a concrete type at generic_field_error:1:38");
}

// ── A5.1 — Closure capture analysis ─────────────────────────────────────────

/// A5.1: lambda referencing an outer variable is detected as a capture.
#[test]
fn capture_detected() {
    code!("fn test() {\n  count = 0;\n  f = fn(x: integer) { count += x; };\n  f(1);\n}");
}

/// A5.1: lambda that does NOT reference outer variables has no capture error.
#[test]
fn no_capture_no_error() {
    code!("fn test() {\n  f = fn(x: integer) -> integer { x + 1 };\n  assert(f(1) == 2);\n}");
}

/// A5.1: variable defined inside the lambda is not flagged as captured.
#[test]
fn local_not_captured() {
    code!(
        "fn test() {\n  f = fn(x: integer) -> integer { y = x + 1; y };\n  assert(f(1) == 2);\n}"
    );
}

// ── A5.2 — Closure record layout ────────────────────────────────────────────

/// A5.2: closure record is synthesized with the correct captured variable.
#[test]
fn closure_record_single_capture() {
    code!("fn test() {\n  count = 0;\n  f = fn(x: integer) { count += x; };\n  f(1);\n}");
}

/// A5.2: multiple captures produce a record with multiple fields.
#[test]
fn closure_record_multi_capture() {
    // A5.3: multi-capture — captured reads redirect to closure record fields.
    // No more "Unknown variable" errors thanks to the pre-has_var redirect.
    code!(
        "fn test() {\n  a = 1;\n  b = 2.0;\n  f = fn(x: integer) -> float { (a + x) as float + b };\n  assert(f(3) == 6.0);\n}"
    );
}

// ── CO1.5c — e#remove rejection on generator iterators ──────────────────────

#[test]
fn generator_remove_rejected() {
    code!(
        "fn gen() -> iterator<integer> { yield 1; yield 2; }
         fn test() { for n in gen() { n#remove; } }"
    )
    .error("'n#remove' is only valid on a loop iteration variable (e.g. 'for n in collection { n#remove }') at generator_remove_rejected:2:48");
}

// ── Fix #91 — Circular init detection ────────────────────────────────────────

// ── S23 — reject generator functions as par() workers ────────────────────────

/// S23: a worker function whose return type is iterator<T> must be rejected at
/// compile time.  Worker threads run inside par() and cannot advance coroutines
/// from the main thread — calling coroutine_next on an out-of-range index panics.
#[test]
fn par_worker_returns_generator() {
    code!(
        "fn gen_worker(x: integer) -> iterator<integer> { yield x; }
         fn test() {
             items = [1, 2, 3];
             for a in items par(b = gen_worker(a), 1) { assert(b > 0); }
         }"
    )
    .error("parallel worker 'gen_worker' returns iterator(integer(-2147483647, 2147483647, false), null) — generator functions cannot be used as parallel workers at par_worker_returns_generator:4:51");
}

// ── T1.11 — Tuple type constraints ───────────────────────────────────────────

/// T1.11a: a tuple type in a struct field position must be rejected at compile
/// time because tuples are stack-only values that cannot be heap-allocated.
#[test]
fn tuple_in_struct_field_rejected() {
    code!("struct Foo { pair: (integer, integer) }\nfn test() {}")
        .error("struct field cannot have a tuple type — tuples are stack-only values at tuple_in_struct_field_rejected:1:40");
}

/// T1.11b: compound assignment on a tuple LHS must produce a clear diagnostic
/// instead of a generic internal error.
#[test]
fn tuple_compound_assign_rejected() {
    code!("fn test() { a = 1; b = 2; (a, b) += (1, 2); }")
        .error("compound assignment is not supported for tuple destructuring — use (a, b) = expr instead at tuple_compound_assign_rejected:1:36");
}

// ── I1/I3 — Interface declarations ───────────────────────────────────────────

/// I3: a minimal empty interface declaration parses without error.
#[test]
fn interface_empty_parses() {
    code!("interface Foo {}\nfn test() {}");
}

/// I3: an interface with method signatures parses without error.
#[test]
fn interface_with_method_parses() {
    code!("interface Showable { fn display(self: Self) -> text }\nfn test() {}");
}

/// I3: a duplicate interface name is rejected with a "Redefined interface" diagnostic.
#[test]
fn interface_duplicate_name_rejected() {
    code!("interface Foo {}\ninterface Foo {}\nfn test() {}")
        .error("Cannot redefine interface 'Foo' at interface_duplicate_name_rejected:2:16");
}

// ── I3.1 — op-sugar in interface bodies ──────────────────────────────────────

/// I3.1: `op < (self: Self, other: Self) -> boolean` in an interface body is
/// syntactic sugar for a method named `OpLt` and must parse without error.
#[test]
fn interface_op_sugar_lt_parses() {
    code!("interface Rankable { op >= (self: Self, other: Self) -> boolean }\nfn test() {}");
}

/// I3.1: a multi-operator interface with `op +` and `op ==` desugars correctly.
#[test]
fn interface_op_sugar_multi_parses() {
    code!(
        "interface Combinable { op & (self: Self, other: Self) -> Self\n\
                                op ^ (self: Self, other: Self) -> Self }\nfn test() {}"
    );
}

// ── I4 — <T: Bound> bound syntax ─────────────────────────────────────────────

/// I4: `fn foo<T: Ordered>(x: T) -> T` with a valid interface bound parses
/// without error and stores the bound for later satisfaction checking.
#[test]
fn generic_fn_with_bound_parses() {
    code!("fn identity<T: Ordered>(x: T) -> T { x }\nfn test() {}");
}

/// I4: a bound name that does not exist must produce a clear diagnostic.
#[test]
fn generic_fn_unknown_bound_errors() {
    code!("fn foo<T: NonExistent>(x: T) -> T { x }\nfn test() {}")
        .error("'NonExistent' is not a known interface at generic_fn_unknown_bound_errors:1:32");
}

/// I4: a struct name used as a type bound must be rejected — only interfaces are valid bounds.
#[test]
fn generic_fn_struct_as_bound_errors() {
    code!("struct Point { x: integer }\nfn foo<T: Point>(x: T) -> T { x }\nfn test() {}")
        .error("'Point' is not an interface — bounds must be interface names at generic_fn_struct_as_bound_errors:2:26");
}

// ── I5 — Factory-method restriction ──────────────────────────────────────────

/// I5: a method that returns `Self` without a leading `self: Self` parameter
/// is a factory method and must be rejected in phase 1.
#[test]
fn interface_factory_method_rejected() {
    code!("interface Creatable { fn create() -> Self }\nfn test() {}")
        .error("factory methods not yet supported: 'create' returns Self without a 'self: Self' parameter at interface_factory_method_rejected:1:44");
}

// ── I6/I10 — Satisfaction checking diagnostics ───────────────────────────────

/// I6/I10: calling a bounded generic function with a type that does NOT implement
/// the required interface method must produce a clear "does not satisfy" diagnostic.
#[test]
fn satisfaction_check_fails_missing_method() {
    code!(
        "struct Thing { x: integer }
         fn pick_first<T: Ordered>(a: T, _b: T) -> T { a }
         fn test() { pick_first(Thing{x:1}, Thing{x:2}) }"
    )
    .error("'Thing' does not satisfy interface 'Ordered': missing OpLt at satisfaction_check_fails_missing_method:3:57");
}

// ── fix-tvscope — Type variable namespace ────────────────────────────────────

/// fix-tvscope: defining a struct whose name clashes with a generic type variable
/// produces a clear diagnostic instead of the confusing "Redefined struct T".
#[test]
fn struct_name_clashes_with_type_variable() {
    code!("struct T { v: integer }\nfn test() {}")
        .error("'T' is reserved as a generic type variable \u{2014} choose a different struct name at struct_name_clashes_with_type_variable:1:11");
}

// ── Fix #91 — Circular init detection ────────────────────────────────────────

/// #91: two init fields referencing each other via $ should produce an error.
#[test]
fn circular_init_error() {
    code!("struct Bad {\n  a: integer init($.b),\n  b: integer init($.a),\n}\nfn test() {}")
        .error("circular init dependency: a -> b -> a at circular_init_error:5:3")
        .error("circular init dependency: b -> a -> b at circular_init_error:5:3");
}

// ── C42 — Unknown variable diagnostic ───────────────────────────────────────

/// C42: using an undefined variable name produces a clear error.
#[test]
fn unknown_variable_error() {
    code!("fn test() -> integer { reuslt = 42; result }")
        .error("Unknown variable 'result' — did you mean 'reuslt'? at unknown_variable_error:1:45")
        .warning("Variable reuslt is never read at unknown_variable_error:1:32");
}

// ── P128: File-scope constants accept type annotations (FIXED) ─────────────
//
// `parse_constant` (src/parser/definitions.rs:392) now consumes an optional
// `: type` annotation between the identifier and `=`. The annotation is
// parsed (so the parser accepts the form) but the literal's inferred type
// is the source of truth — a future enhancement could validate the two
// match after dep-list normalisation.
//
// Regression guard: the form must keep parsing without errors.
#[test]
fn p128_constant_with_type_annotation_parses() {
    code!("QUAD: vector<integer> = [1, 2, 3];\nfn test() {}");
    // No .error() calls — parses cleanly.
}

// ── P85b: User type/enum/struct shadowing a stdlib constant ─────────────────
//
// Defining a user type whose name collides with a stdlib constant (e.g.
// `enum E { ... }` collides with `pub E = OpMathEFloat()`) used to produce
// a compiler PANIC like `Cannot change returned type on [164]E to float
// twice was E`.  Both `enum` and `struct` now emit a clear, actionable
// diagnostic naming the conflicting definition's location.
#[test]
fn p85b_enum_shadowing_stdlib_constant_emits_diagnostic() {
    let s = loft::platform::sep_str();
    code!("enum E { Foo, Bar }\nfn test() {}").error(&format!(
        "enum 'E' conflicts with a constant of the same name already defined \
         at default{s}01_code.loft:385:24 — pick a different name \
         at p85b_enum_shadowing_stdlib_constant_emits_diagnostic:1:9"
    ));
}

#[test]
fn p85b_struct_shadowing_stdlib_constant_emits_diagnostic() {
    let s = loft::platform::sep_str();
    code!("struct E { n: integer }\nfn test() {}").error(&format!(
        "struct 'E' conflicts with a constant of the same name already defined \
         at default{s}01_code.loft:385:24 — pick a different name \
         at p85b_struct_shadowing_stdlib_constant_emits_diagnostic:1:11"
    ));
}

#[test]
fn p85b_type_shadowing_stdlib_constant_emits_diagnostic() {
    let s = loft::platform::sep_str();
    code!("type E = integer;\nfn test() {}").error(&format!(
        "type 'E' conflicts with a constant of the same name already defined \
         at default{s}01_code.loft:385:24 — pick a different name \
         at p85b_type_shadowing_stdlib_constant_emits_diagnostic:1:9"
    ));
}

#[test]
fn p85b_constant_shadowing_stdlib_constant_emits_diagnostic() {
    let s = loft::platform::sep_str();
    code!("E = 42;\nfn test() {}").error(&format!(
        "constant 'E' conflicts with a constant of the same name already defined \
         at default{s}01_code.loft:385:24 — pick a different name \
         at p85b_constant_shadowing_stdlib_constant_emits_diagnostic:1:8"
    ));
}

// ── P85c: file-scope-only declarations rejected with a clean diagnostic ─────
//
// Putting `struct`, `enum`, `type`, `interface`, `use`, `pub`, or a named
// `fn name(...)` inside a function body used to produce a cascade of
// confusing errors (`Expect token =`, `Expect constants to be in upper case`,
// `Syntax error: unexpected ...`).  parse_block now detects these keywords
// at the statement boundary and emits a single clear diagnostic.  Lambdas
// (`fn(args) { ... }`) are still allowed because they parse as expressions.
#[test]
fn p85c_struct_inside_fn_emits_diagnostic() {
    code!("fn test() {\n  struct Inner { v: integer }\n  x = 5;\n}").error(
        "'struct' definitions must be at file scope, not inside a function or block \
         at p85c_struct_inside_fn_emits_diagnostic:2:9",
    );
}

#[test]
fn p85c_enum_inside_fn_emits_diagnostic() {
    code!("fn test() {\n  enum Inner { A, B }\n  x = 5;\n}").error(
        "'enum' definitions must be at file scope, not inside a function or block \
         at p85c_enum_inside_fn_emits_diagnostic:2:7",
    );
}

#[test]
fn p85c_named_fn_inside_fn_emits_diagnostic() {
    code!("fn test() {\n  fn inner() -> integer { 5 }\n  x = 5;\n}").error(
        "'fn' definitions must be at file scope, not inside a function or block \
         at p85c_named_fn_inside_fn_emits_diagnostic:2:11",
    );
}

#[test]
fn c61_nested_same_name_loop_rejected() {
    // C61: nested `for i { for i { } }` silently aliases the outer
    // iterator's `#index` companion, causing wrong runtime results.
    // The parser now rejects it with an actionable rename hint.
    code!(
        "fn test() {\n  \
           for i in 0..3 {\n    \
             for i in 10..13 { }\n  \
           }\n\
         }"
    )
    .error(
        "loop variable 'i' shadows the enclosing loop's 'i' — \
         rename the inner loop variable (e.g. inner_i); loft does \
         not support nested same-name loops at c61_nested_same_name_loop_rejected:3:22",
    )
    .warning("Variable i is never read at c61_nested_same_name_loop_rejected:2:18");
}

#[test]
fn c61_local_shadow_rejected() {
    // C61.local: a for-loop variable that would silently clobber a
    // same-named outer local is rejected at parse time with a rename
    // hint.  Unblocked by PROBLEMS.md #139's OpReserveFrame fix, which
    // made it possible to rename stdlib docs without tripping the
    // slot-allocator TOS mismatch on layout changes.
    code!(
        "fn run() -> integer {\n  \
           x = 99;\n  \
           for x in 0..3 { }\n  \
           x\n\
         }"
    )
    .expr("run()")
    .error(
        "loop variable 'x' shadows a local named 'x' — rename the loop \
         variable (e.g. loop_x) or drop the outer `x` if it was a dead \
         placeholder; loft does not block-scope loop variables at \
         c61_local_shadow_rejected:3:18",
    );
}

#[test]
fn c61_local_shadow_renamed_ok() {
    // Regression guard: renaming the loop variable keeps the outer
    // local intact.
    code!(
        "fn run() -> integer {\n  \
           x = 99;\n  \
           for loop_x in 0..3 { x + loop_x; }\n  \
           x\n\
         }"
    )
    .expr("run()")
    .result(Value::Int(99));
}

#[test]
fn c61_local_dropped_outer_ok() {
    // Regression guard: dropping the dead outer placeholder is the
    // other documented fix.  `a` is live only inside the loop.
    code!(
        "fn run() -> integer {\n  \
           t = 0;\n  \
           for a in 1..6 { t += a; }\n  \
           t\n\
         }"
    )
    .expr("run()")
    .result(Value::Int(15));
}

#[test]
fn c61_nested_different_names_ok() {
    // Regression guard: nested loops with *different* names still parse.
    code!(
        "fn run() -> integer {\n  \
           total = 0;\n  \
           for i in 0..3 { for j in 10..13 { total += j + i; } }\n  \
           total\n\
         }"
    )
    .expr("run()")
    .result(Value::Int(108));
}

#[test]
fn c61_sequential_same_name_ok() {
    // Regression guard: sequential same-name loops (non-nested) remain
    // valid — only nested aliasing is rejected.
    code!(
        "fn run() -> integer {\n  \
           a = 0;\n  \
           for i in 0..3 { a += i; }\n  \
           b = 0;\n  \
           for i in 10..13 { b += i; }\n  \
           a + b\n\
         }"
    )
    .expr("run()")
    .result(Value::Int(36));
}

#[test]
fn p85c_lambda_inside_fn_still_works() {
    // Regression guard: lambda expressions (`fn(args) { ... }`) must not
    // trigger the file-scope-only diagnostic.
    code!(
        "fn test() {\n  f = fn(x: integer) -> integer { x * 2 };\n  \
         assert(f(5) == 10, \"lambda\");\n}"
    );
}

// ── L1: error recovery after token failures ─────────────────────────────────
//
// Missing `;` at end of a statement used to produce a cascade of four+
// errors ("Expect token ;", "Expect token }", "Expect constants to be in
// upper case style", "Syntax error: unexpected ..."). The parser now calls
// `Lexer::recover_to(&[";", "}"])` after a failed `token(";")` inside
// `parse_block`, resynchronising to the next statement boundary.
#[test]
fn l1_missing_semicolon_single_diagnostic() {
    // Missing `;` between `x = 1` and `y = 2;`. Should produce exactly one
    // error, not a cascade.
    code!("fn test() {\n  x = 1\n  y = 2;\n  assert(x + y == 3, \"\");\n}")
        .error("Expect token ; at l1_missing_semicolon_single_diagnostic:3:4");
}

#[test]
fn l1_missing_semicolon_in_body_single_diagnostic() {
    code!("fn foo(x: integer) -> integer {\n  y = x + 1\n  y * 2\n}\nfn test() {}")
        .error("Expect token ; at l1_missing_semicolon_in_body_single_diagnostic:3:4");
}

// ── P54 struct-enum blockers (BITING_PLAN § P54) ─────────────────────────
//
// Regression guards for the struct-enum compiler bugs surfaced while
// building JsonValue.  Each bug is tracked as B1..B7 in BITING_PLAN.md.
// Fixed bugs pin the diagnostic-or-success behaviour; open bugs land as
// `#[ignore]`'d with the expected future state so the test goes green
// automatically when the fix lands.

/// B2 was originally: `fn mk() -> Shade { Shade.N }` for a mixed-kind
/// enum (unit + struct-field variants) errored with 'Shade should be
/// Shade on return from block' because the unit variant's declared
/// `Type::Enum(d, false, _)` didn't unify with the struct-enum-upgraded
/// parent's `Type::Enum(d, true, _)`.
///
/// Full fix now lands (`parse_enum_values` post-pass syncs every
/// variant's type to the final parent type), so the compile passes
/// cleanly with no diagnostic.  Runtime use of the returned
/// struct-enum is still blocked by B3/B4 — tracked separately in
/// `tests/issues.rs::p54_b3_single_variant_return`.
#[test]
fn p54_b2_unit_variant_return_compiles() {
    // No .error() or .fatal() — the test passes when compilation
    // produces no diagnostics.
    code!(
        "pub enum Shade { N, V { v: integer } }
fn mk() -> Shade { Shade.N }
fn test() {}"
    );
}

/// `--features emit-repro` writes the assembled test source to
/// `/tmp/loft-repro/<name>.loft` before executing, with a thin
/// `fn main() { test(); }` tail appended so the file is directly
/// runnable via `target/release/loft <path>`.  Test name MUST match
/// the generated filename — `Test::drop` uses `stdext::function_name!()`.
#[cfg(feature = "emit-repro")]
#[test]
fn emit_repro_produces_runnable_loft_file() {
    let path = "/tmp/loft-repro/emit_repro_produces_runnable_loft_file.loft";
    let _ = std::fs::remove_file(path);

    code!(
        "fn run() -> integer {
    1 + 2
}"
    )
    .expr("run()")
    .result(Value::Int(3));

    let contents = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!("emit-repro: expected {path} to be written but read failed: {e}")
    });
    assert!(
        contents.contains("fn run() -> integer {"),
        "emit-repro: body missing from {path}:\n---\n{contents}"
    );
    assert!(
        contents.contains("pub fn test()"),
        "emit-repro: test() wrapper missing from {path}:\n---\n{contents}"
    );
    assert!(
        contents.contains("fn main() {") && contents.contains("test();"),
        "emit-repro: runnable `fn main() {{ test(); }}` tail missing from {path}:\n---\n{contents}"
    );
}
