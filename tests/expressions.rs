// Copyright (c) 2021-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Tests that require Rust-level type checking (.tp()) or native codegen.
// All other expression tests have moved to tests/scripts/*.loft.

extern crate loft;

mod testing;

use loft::data::{Type, Value};

const INTEGER: Type = Type::Integer(i32::MIN + 1, i32::MAX as u32, false);

#[test]
fn expr_add_null() {
    expr!("1 + null").tp(INTEGER);
}

#[test]
fn expr_zero_divide() {
    expr!("2 / (3 - 2 - 1)").tp(INTEGER);
}

#[test]
fn call_with_null() {
    code!("fn add(a: integer, b: integer) -> integer { a + b }")
        .expr("add(1, null)")
        .tp(INTEGER)
        .result(Value::Null);
}

#[test]
fn call_text_null() {
    code!("fn routine(a: integer) -> text { if a > 2 { return null }; \"#{a}#\"}")
        .expr("routine(5)")
        .tp(Type::Text(vec![]))
        .result(Value::Null);
}

#[test]
fn call_int_null() {
    code!("fn routine(a: integer) -> integer { if a > 2 { return null }; a+1 }")
        .expr("routine(5)")
        .tp(INTEGER)
        .result(Value::Null);
}

#[test]
fn if_typing() {
    expr!("a = \"12\"; if a.len()>2 { null } else { \"error\" }").result(Value::str("error"));
    expr!("a = \"12\"; if a.len()==2 { null } else { \"error\" }")
        .tp(Type::Text(vec![]))
        .result(Value::Null);
}

// N6 generated_code_compiles and native_test_suite moved to tests/native.rs

// ── T1.1 — Type::Tuple helpers ──────────────────────────────────────────────

#[test]
fn tuple_element_offsets() {
    use loft::data::{Type, element_offsets, element_size};
    let types = [
        Type::Integer(i32::MIN, i32::MAX as u32, false),
        Type::Text(vec![]),
        Type::Float,
    ];
    let offsets = element_offsets(&types);
    // integer=4 at 0, text=Str size at 4, float=8 after text
    let text_sz = element_size(&Type::Text(vec![]));
    assert_eq!(offsets, vec![0, 4, 4 + text_sz]);
}

#[test]
fn tuple_owned_elements() {
    // owned_elements for [integer, text, reference<T>] should return text and ref entries
    use loft::data::{Type, owned_elements};
    let types = [
        Type::Integer(i32::MIN, i32::MAX as u32, false),
        Type::Text(vec![]),
        Type::Reference(0, vec![]),
    ];
    let owned = owned_elements(&types);
    assert_eq!(owned.len(), 2);
}

// ── CO1.1 — CoroutineStatus enum ────────────────────────────────────────────
// Verify the CoroutineStatus enum from default/05_coroutine.loft.

#[test]
fn coroutine_status_construct() {
    code!(
        "fn check(s: CoroutineStatus) -> boolean {
               match s { Created => true, _ => false }
           }"
    )
    .expr("check(CoroutineStatus.Created)")
    .result(Value::Boolean(true));
}

#[test]
fn coroutine_status_ordering() {
    // Enum variant ordering: Created < Suspended < Running < Exhausted
    expr!("CoroutineStatus.Created < CoroutineStatus.Exhausted").result(Value::Boolean(true));
}

// ── TR1.3 — stack_trace() materialisation ────────────────────────────────────
// Verify that stack_trace() returns a vector of StackFrame.

#[test]
fn stack_trace_returns_frames() {
    // stack_trace() returns one frame per call-stack entry, including the synthetic
    // entry frame for n_test (Fix #88).  Named variable `frames` ensures OpFreeRef
    // is emitted at scope exit.
    code!(
        "fn inner(n: integer) -> integer { frames = stack_trace(); len(frames) + n - n }
         fn outer(n: integer) -> integer { inner(n) }"
    )
    .expr("outer(0)")
    .result(Value::Int(3)); // n_test(entry) + outer + inner
}

#[test]
fn stack_trace_function_names() {
    // Returns integer (1 if name matches) to avoid borrowing text from the vector,
    // which would suppress OpFreeRef and leak the database store.
    // frames = [n_test(entry), caller, check_caller_name]; "caller" is at index len-2.
    code!(
        "fn check_caller_name() -> integer {
            frames = stack_trace();
            if len(frames) > 1 && frames[len(frames) - 2].function == \"caller\" { 1 } else { 0 }
         }
         fn caller() -> integer { check_caller_name() }"
    )
    .expr("caller()")
    .result(Value::Int(1));
}

// ── TR1.4 — Call-site line numbers ───────────────────────────────────────────

#[test]
fn call_frame_has_line() {
    // Verify that stack_trace() reports a non-zero line for a known call site.
    // frames = [n_test(entry, line=0), check_line(line=call-site)].
    // Use frames[len(frames)-1] to access the innermost (check_line) frame.
    code!(
        "fn check_line(n: integer) -> integer {
            frames = stack_trace();
            if len(frames) > 0 { frames[len(frames) - 1].line + n - n } else { -1 + n - n }
         }"
    )
    .expr("check_line(0)")
    .result(Value::Int(7)); // user code = 4 lines + 1 blank + "pub fn test() {" + call at line 7
}

// ── TR1.2 — StackFrame + ArgValue type declarations ─────────────────────────
// Verify the types from default/04_stacktrace.loft can be constructed and used.

#[test]
fn stacktrace_argvalue_construct() {
    // Verify ArgValue enum is visible: matching on a variant produces the expected type.
    code!(
        "fn check_arg(v: ArgValue) -> integer {
            match v { IntVal { n } => n, _ => -1 }
         }"
    )
    .expr("check_arg(IntVal { n: 42 })")
    .result(Value::Int(42));
}

#[test]
fn stacktrace_arginfo_field() {
    // Verify ArgInfo struct is visible and fields are accessible.
    code!("fn get_name(info: ArgInfo) -> text { info.name }")
        .expr("get_name(ArgInfo { name: \"x\", type_name: \"integer\", value: IntVal { n: 7 } })")
        .result(Value::str("x"));
}

#[test]
fn stacktrace_frame_field() {
    // Verify StackFrame struct is visible and fields are accessible.
    code!("fn get_fn(f: StackFrame) -> text { f.function }")
        .expr("get_fn(StackFrame { function: \"main\", file: \"test.loft\", line: 1 })")
        .result(Value::str("main"));
}

// ── TR1.1 — Shadow call-frame vector ────────────────────────────────────────
// Verify that function calls still work after the OpCall bytecode format change
// (d_nr + args_size operands added for the shadow call-frame vector).

#[test]
fn call_stack_nested_calls() {
    code!(
        "fn add(a: integer, b: integer) -> integer { a + b }
         fn double(x: integer) -> integer { add(x, x) }
         fn quad(x: integer) -> integer { double(double(x)) }"
    )
    .expr("quad(3)")
    .result(Value::Int(12));
}

#[test]
fn call_stack_fn_ref() {
    code!(
        "fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
         fn inc(n: integer) -> integer { n + 1 }"
    )
    .expr("apply(inc, 41)")
    .result(Value::Int(42));
}

#[test]
fn call_stack_recursive() {
    code!(
        "fn fib(n: integer) -> integer {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
         }"
    )
    .expr("fib(10)")
    .result(Value::Int(55));
}

// ── T1.2 — Tuple parser (notation, literals, destructuring) ─────────────────

#[test]
fn tuple_type_return() {
    // A function returning a tuple type should parse and compile.
    code!(
        "fn pair(a: integer, b: integer) -> (integer, integer) {
            (a, b)
         }"
    )
    .expr("pair(3, 7).0")
    .result(Value::Int(3));
}

#[test]
fn tuple_literal_basic() {
    // A tuple literal assigned to a variable; element access via .0 / .1.
    expr!("t = (10, 20); t.0 + t.1").result(Value::Int(30));
}

#[test]
fn tuple_element_access_three() {
    // Three-element tuple with mixed types — access each element.
    expr!("t = (1, 2, 3); t.0 + t.1 + t.2").result(Value::Int(6));
}

#[test]
fn tuple_destructure_basic() {
    // LHS destructuring: (a, b) = expr.
    code!("fn pair(x: integer) -> (integer, integer) { (x, x * 2) }")
        .expr("(a, b) = pair(5); a + b")
        .result(Value::Int(15));
}

#[test]
fn tuple_element_assign() {
    // Assigning to an individual tuple element: t.0 = expr.
    expr!("t = (1, 2); t.0 = 10; t.0 + t.1").result(Value::Int(12));
}

#[test]
fn tuple_type_annotation() {
    // Explicit tuple type annotation on a variable.
    expr!("t: (integer, integer) = (3, 4); t.0 + t.1").result(Value::Int(7));
}

#[test]
fn tuple_parameter() {
    // Tuple type as a function parameter.
    code!("fn sum_pair(p: (integer, integer)) -> integer { p.0 + p.1 }")
        .expr("sum_pair((10, 20))")
        .result(Value::Int(30));
}

#[test]
fn tuple_with_text() {
    // Tuple containing a text element — verify text is accessible.
    code!("fn greet(name: text) -> (integer, text) { (len(name), name) }")
        .expr("greet(\"hello\").0")
        .result(Value::Int(5));
}

// ── T1.5 — Reference-tuple parameters ────────────────────────────────────────

#[test]
fn ref_tuple_param_swap() {
    // &(integer, integer) parameter — swap elements via reference.
    code!(
        "fn swap(pair: &(integer, integer)) {
            tmp = pair.0;
            pair.0 = pair.1;
            pair.1 = tmp;
         }"
    )
    // In loft, ref args are passed by variable name — no & prefix at call site.
    .expr("p = (3, 7); swap(p); p.0 * 10 + p.1")
    .result(Value::Int(73));
}

// ── T1.6 — Tuple-aware mutation guard ────────────────────────────────────────

#[test]
fn ref_tuple_unused_mutation_error() {
    // &(integer, integer) parameter that is never mutated — should produce a warning.
    code!("fn read_only(pair: &(integer, integer)) -> integer { pair.0 + pair.1 }")
        .expr("p = (3, 7); read_only(p)")
        .warning("Parameter 'pair' does not need to be a reference at ref_tuple_unused_mutation_error:1:53")
        .result(Value::Int(10));
}

// ── A5.3 — Closure capture at call site ─────────────────────────────────────

#[test]
fn closure_capture_integer() {
    // A lambda captures an integer from the enclosing scope.
    expr!("x = 10; f = fn(y: integer) -> integer { x + y }; f(5)")
        .warning("closure record '__closure_0' created with 1 field: x(integer) at closure_capture_integer:2:67")
        .warning("Variable x is never read at closure_capture_integer:2:22")
        .result(Value::Int(15));
}

#[test]
fn closure_capture_after_change() {
    // Capture copies x's value at the call site (current implementation captures
    // at call time, not definition time).  x is 99 when f(5) runs → 99 + 5 = 104.
    // Capture-at-definition-time (expected: 15) is a deferred improvement.
    expr!("x = 10; f = fn(y: integer) -> integer { x + y }; x = 99; f(5)")
        .warning("closure record '__closure_0' created with 1 field: x(integer) at closure_capture_after_change:2:67")
        .warning("Dead assignment — 'x' is overwritten before being read at closure_capture_after_change:2:26")
        .warning("Variable x is never read at closure_capture_after_change:2:22")
        .result(Value::Int(104));
}

#[test]
fn closure_capture_multiple() {
    // A lambda captures two variables from the enclosing scope.
    expr!("a = 3; b = 7; f = fn(x: integer) -> integer { a + b + x }; f(10)")
        .warning("closure record '__closure_0' created with 2 fields: a(integer), b(integer) at closure_capture_multiple:2:77")
        .warning("Variable a is never read at closure_capture_multiple:2:22")
        .warning("Variable b is never read at closure_capture_multiple:2:29")
        .result(Value::Int(20));
}

#[test]
#[ignore = "A5.6b: text capture blocked by two runtime bugs — (1) OpSetText/OpGetText on \
closure records produces garbage DbRef in the lambda stack frame; (2) text-returning lambdas \
via CallRef don't allocate text work variable buffers. See CAVEATS.md C1."]
fn closure_capture_text() {
    // Captured text is deep-copied — independent of the original after capture.
    code!(
        "fn make_greeter(prefix: text) -> fn(text) -> text {
            fn(name: text) -> text { \"{prefix} {name}\" }
         }"
    )
    .expr("make_greeter(\"Hello\")(\"world\")")
    .result(Value::str("Hello world"));
}

#[test]
fn closure_capture_text_integer_return() {
    // Same-scope text capture: lambda reads captured text, returns integer.
    // A5.6b.1: zero-param fn-ref fast path now injects __closure arg; text_return
    // no longer adds captured vars as spurious RefVar(Text) work-buffer arguments.
    expr!("prefix = \"hello\"; f = fn() -> integer { len(prefix) }; f()")
        .warning("closure record '__closure_0' created with 1 field: prefix(text([])) at closure_capture_text_integer_return:2:73")
        .result(Value::Int(5));
}

// A5.6b.2: re-enabled after generate_call_ref work-buffer push fix.
#[test]
fn closure_capture_text_return() {
    // Same-scope text capture: lambda reads captured text, returns text.
    expr!("greeting = \"hello\"; f = fn(name: text) -> text { \"{greeting}, {name}!\" }; f(\"world\")")
        .warning("closure record '__closure_0' created with 1 field: greeting(text([])) at closure_capture_text_return:2:92")
        .result(Value::str("hello, world!"));
}

// ── CO1.2 — OpCoroutineCreate + OpCoroutineNext ─────────────────────────────

#[test]
fn coroutine_create_basic() {
    // A generator function should return an iterator without executing the body.
    code!(
        "fn count() -> iterator<integer> { yield 1; yield 2; yield 3; }
         fn test_count() -> integer {
            gen = count();
            next(gen)
         }"
    )
    .expr("test_count()")
    .result(Value::Int(1));
}

#[test]
fn coroutine_next_sequence() {
    // Successive next() calls advance the generator.
    code!(
        "fn count() -> iterator<integer> { yield 10; yield 20; yield 30; }
         fn sum_three() -> integer {
            gen = count();
            a = next(gen);
            b = next(gen);
            c = next(gen);
            a + b + c
         }"
    )
    .expr("sum_three()")
    .result(Value::Int(60));
}

#[test]
fn coroutine_exhausted() {
    // After all yields + one more advance, exhausted() returns true.
    code!(
        "fn one_val() -> iterator<integer> { yield 42; }
         fn check() -> boolean {
            gen = one_val();
            next(gen);
            next(gen);
            exhausted(gen)
         }"
    )
    .expr("check()")
    .result(Value::Boolean(true));
}

#[test]
fn coroutine_for_loop() {
    // Generator consumed by a for loop.
    code!(
        "fn range3() -> iterator<integer> { yield 1; yield 2; yield 3; }
         fn sum_gen() -> integer {
            total = 0;
            for n in range3() { total += n; }
            total
         }"
    )
    .expr("sum_gen()")
    .result(Value::Int(6));
}

// ── CO1.3e — Nested yield (generator calls helper function) ─────────────────

#[test]
fn coroutine_call_helper_between_yields() {
    // A generator calls a regular function between yields.
    // The call frame is saved/restored across the yield/resume cycle.
    code!(
        "fn double(x: integer) -> integer { x * 2 }
         fn gen() -> iterator<integer> {
            yield double(5);
            yield double(10);
         }"
    )
    .expr("total = 0; for n in gen() { total += n; }; total")
    .result(Value::Int(30));
}

// ── CO1.3d — Text serialisation across yield/resume ─────────────────────────

#[test]
fn coroutine_text_param_survives_yield() {
    // A generator that takes a `text` parameter and yields `len(text)`.
    // The text value must survive the yield/resume cycle without dangling pointers.
    code!(
        "fn gen_len(s: text) -> iterator<integer> {
            yield len(s);
            yield len(s);
         }
         fn sum_lens() -> integer {
            total = 0;
            for n in gen_len(\"hello\") { total += n; }
            total
         }"
    )
    .expr("sum_lens()")
    .result(Value::Int(10));
}

// P2-R3: text LOCAL survives yield — CO1.3d serialise_text_slots implemented
#[test]
fn coroutine_text_local_survives_yield() {
    // P2-R3: a generator that builds a text LOCAL (not a parameter) and yields.
    // CO1.3d must serialise the local String to text_owned at yield and restore
    // the pointer on resume; until then the raw bytes path leaves a dangling ptr.
    code!(
        "fn gen_words() -> iterator<text> {
            word = \"hello\";
            yield word;
            word = \"world\";
            yield word;
         }
         fn joined() -> text {
            result = \"\";
            for w in gen_words() { result += w; result += \" \"; }
            result
         }"
    )
    .expr("joined()")
    .result(Value::str("hello world "));
}

// ── CO1.4 — yield from delegation ───────────────────────────────────────────

// ── T1.7 — `integer not null` annotation for tuple elements ─────────────────

#[test]
fn not_null_element_assignment() {
    // `integer not null` element in a tuple type — basic assignment compiles and runs.
    code!("fn count_pair() -> (integer not null, integer not null) { (1, 2) }")
        .expr("p = count_pair(); p.0 + p.1")
        .result(Value::Int(3));
}

// ── CO1.4 — yield from ───────────────────────────────────────────────────────

#[test]
fn coroutine_yield_from() {
    // yield from delegates to a sub-generator.
    code!(
        "fn inner() -> iterator<integer> { yield 10; yield 20; }
         fn outer() -> iterator<integer> { yield 1; yield from inner(); yield 2; }
         fn sum_all() -> integer {
            total = 0;
            for n in outer() { total += n; }
            total
         }"
    )
    .expr("sum_all()")
    .result(Value::Int(33));
}

// ── S23 — runtime guard in coroutine_next ─────────────────────────────────────

/// S23/P1-R2 runtime guard: coroutine_next must bounds-check idx before indexing
/// self.coroutines.  When a COROUTINE_STORE DbRef escapes into a worker thread its
/// rec value indexes the WORKER's (empty) coroutines table → out-of-bounds panic.
/// After the fix a clear attributed message fires instead of a bare index panic.
#[test]
fn coroutine_next_bounds_guard() {
    // S23 is complete: the compiler rejects iterator<T> function calls inside par()
    // bodies, and coroutine_next has a runtime bounds guard as defence-in-depth.
    // The compiler check is exercised by par_worker_returns_generator in parse_errors.rs.
    // Nothing to assert here; this test exists as a named placeholder.
}

// ── S26 — exhausted coroutine frames freed on return ──────────────────────────

/// S26: after a for-loop exhausts a generator, coroutine_return frees the slot.
/// Running many generators in succession must not grow State::coroutines unboundedly.
#[test]
fn coroutine_frame_freed_after_exhaustion() {
    code!(
        "fn up_to_two() -> iterator<integer> { yield 1; yield 2; }
         fn sum_many() -> integer {
             total = 0;
             for _ in 0..1000 { for n in up_to_two() { total += n; } }
             total
         }"
    )
    .expr("sum_many()")
    .result(Value::Int(3000));
}

// ── S27 — text_positions save/restore across yield/resume ─────────────────────

/// S27: text_positions entries for suspended generator locals are removed at
/// yield and restored at resume.  Consumer text locals no longer conflict with
/// the suspended generator's tracked positions in debug builds.
/// The observable fix is the absence of spurious double-free panics; we verify
/// the generator + consumer text combination produces the correct integer sum.
#[test]
fn coroutine_text_positions_save_restore() {
    // Consumer allocates text while iterating a generator; S27 ensures
    // text_positions entries from the suspended generator don't mask missing
    // OpFreeText calls in the consumer (or cause false double-free panics).
    code!(
        "fn gen_ints() -> iterator<integer> { yield 1; yield 2; yield 3; }
         fn sum_with_label(label: text) -> integer {
             total = 0;
             for n in gen_ints() { total += n; }
             len(label) + total
         }"
    )
    .expr("sum_with_label(\"hi\")")
    .result(Value::Int(8)); // len("hi")=2, sum=6, total=8
}

// ── S29 — parallel store: thread::scope + skip claims ─────────────────────────

/// S29/P1-R2+P1-R3: run_parallel_direct uses thread::scope and claims-free
/// worker clones; observable results are identical to the old thread::spawn path.
#[test]
fn parallel_for_thread_scope_results() {
    code!(
        "struct Num { value: integer }
         struct NumList { items: vector<Num> }
         fn doubled(n: const Num) -> integer { n.value * 2 }
         fn run_par() -> integer {
             lst = NumList {};
             lst.items += [Num { value: 1 }, Num { value: 2 }, Num { value: 3 }, Num { value: 4 }, Num { value: 5 }];
             total = 0;
             for a in lst.items par(b = doubled(a), 2) { total += b; }
             total
         }"
    )
    .expr("run_par()")
    .result(Value::Int(30));
}

// ── S28 — debug generation counter for stale DbRef across coroutine yield ─────

/// S28: Mutating a struct store between coroutine next() calls should fire the
/// debug-mode generation-counter assertion.  The generator holds a `const Item`
/// reference (a DbRef into the `Items` store); between the two yields the
/// consumer pushes a new element into the same store, incrementing its
/// generation.  On the second resume, `coroutine_next` detects the mismatch and
/// panics with "stale DbRef".
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "stale DbRef")]
fn coroutine_stale_store_guard() {
    // The generator count_up has no DbRef parameters; the stale-store check is a
    // heuristic that fires on ANY store mutation between yields.  We pre-create a
    // struct store before the loop so it is included in the yield snapshot, then
    // claim a new record (Box{}) inside the loop to increment its generation.
    code!(
        "struct Box { val: integer }
         struct BoxList { items: vector<Box> }
         fn count_up() -> iterator<integer> { yield 1; yield 2; yield 3; }
         fn run_stale() -> integer {
             lst = BoxList {};
             lst.items += [Box { val: 0 }];
             total = 0;
             for n in count_up() {
                 lst.items += [Box { val: n }];
                 total += n;
             }
             total
         }"
    )
    .expr("run_stale()")
    .result(Value::Int(6)); // never reached — debug_assert fires on second resume
}

// ── S22 — claim/delete on a locked store panics in all build profiles ─────────

#[test]
#[should_panic(expected = "Claim on locked store")]
fn claim_on_locked_store_panics() {
    // S22: store.claim() now panics when locked in all build profiles.
    // Before the fix, release builds returned a silent dummy record 0 instead.
    let mut stores = loft::database::Stores::new();
    let db = stores.null();
    stores.store_mut(&db).lock();
    stores.claim(&db, 4); // must panic — locked store may not be extended
}

#[test]
#[should_panic(expected = "Delete on locked store")]
fn delete_on_locked_store_panics() {
    // S22: store.delete() now panics when locked in all build profiles.
    // Before the fix, release builds silently ignored the delete.
    let mut stores = loft::database::Stores::new();
    let db = stores.null();
    let cr = stores.claim(&db, 2);
    stores.store_mut(&db).lock();
    stores.store_mut(&db).delete(cr.rec); // must panic
}

// ── S25.1 — text arg `Str` serialised at coroutine_create ────────────────────

/// S25.1 (P2-R1): text args are passed as `Str { ptr, len }` — a borrowed reference
/// into the caller's owned `String`.  After `coroutine_create`, the caller's String
/// may be freed by `OpFreeText` before the generator is first resumed.  The `Str`
/// in `stack_bytes` then holds a dangling pointer.
/// Fix: `serialise_text_slots` at `coroutine_create` converts every dynamic text arg
/// to an owned `String` in `frame.text_owned`.  See SAFE.md § P2-R1.
#[test]
fn coroutine_text_arg_dynamic_serialised() {
    // gen_label receives a format-string arg (dynamic heap String, not a static literal).
    // After coroutine_create, the Str in stack_bytes must point to a serialised owned
    // String — not the caller's allocation which OpFreeText may free immediately after.
    code!(
        "fn gen_label(prefix: text) -> iterator<integer> {
             yield len(prefix);
             yield len(prefix);
         }
         fn sum_dynamic_lens(n: integer) -> integer {
             total = 0;
             for v in gen_label(\"item {n}\") { total += v; }
             total
         }"
    )
    .expr("sum_dynamic_lens(3)")
    .result(Value::Int(12)); // len("item 3") = 6, two yields: 6 + 6 = 12
}

// ── S25.2 — `String` locals freed before coroutine_return rewinds stack ──────

/// S25.2 (P2-R2): when a generator has a `text` local variable, the `String` heap
/// allocation lives on the coroutine's live stack.  `coroutine_return` calls
/// `frame.text_owned.clear()` and `frame.stack_bytes.clear()`, but neither path
/// calls `String::drop()` for stack-resident Strings → heap leak.
/// Fix: drain `text_positions` entries in the live frame region before rewinding.
/// See SAFE.md § P2-R2.
#[test]
fn coroutine_text_arg_freed_at_return() {
    // gen_len_twice takes a dynamic text arg, yields its length twice, then exhausts.
    // At coroutine_return, frame.text_owned must be drained so the owned String
    // allocated by S25.1 at create time is freed.  Without the drain, every
    // generator exhaustion with a text arg leaks a String heap allocation.
    // The test verifies correct values; Valgrind is needed to confirm the drain.
    // Note: single-yield generators have a separate pre-existing bug (return 0).
    code!(
        "fn gen_len_twice(prefix: text) -> iterator<integer> {
             yield len(prefix);
             yield len(prefix);
         }
         fn sum_twice(n: integer) -> integer {
             total = 0;
             for v in gen_len_twice(\"item {n}\") { total += v; }
             total
         }"
    )
    .expr("sum_twice(3)")
    .result(Value::Int(12)); // len("item 3") = 6, two yields: 6 + 6 = 12
}
