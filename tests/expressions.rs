// Copyright (c) 2021-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Tests that require Rust-level type checking (.tp()) or native codegen.
// All other expression tests have moved to tests/scripts/*.loft.

extern crate loft;

mod testing;

use loft::data::{Type, Value};

const INTEGER: Type = Type::Integer(i32::MIN + 1, i32::MAX as u32);

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
        Type::Integer(i32::MIN, i32::MAX as u32),
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
        Type::Integer(i32::MIN, i32::MAX as u32),
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
#[ignore = "TR1.3: blocked by Problem #85 — struct-enum/reference local stack cleanup"]
fn stack_trace_returns_frames() {
    // stack_trace() returns one frame per fn_call (entry function excluded).
    code!(
        "fn inner(n: integer) -> integer { len(stack_trace()) + n - n }
         fn outer(n: integer) -> integer { inner(n) }"
    )
    .expr("outer(0)")
    .result(Value::Int(2)); // outer->inner (test is entry, not on call_stack)
}

#[test]
#[ignore = "TR1.3: blocked by Problem #85 — struct-enum/reference local stack cleanup"]
fn stack_trace_function_names() {
    code!(
        "fn get_caller_name() -> text {
            frames = stack_trace();
            if len(frames) > 0 { frames[len(frames) - 1].function } else { \"none\" }
         }
         fn caller() -> text { get_caller_name() }"
    )
    .expr("caller()")
    .result(Value::str("caller"));
}

// ── TR1.4 — Call-site line numbers ───────────────────────────────────────────

#[test]
#[ignore = "TR1.4: blocked by Problem #85 — struct-enum/reference local stack cleanup"]
fn call_frame_has_line() {
    // Verify that stack_trace() reports a non-zero line for a known call site.
    // Blocked by #85, but the diagnostic is correct.
    code!(
        "fn check_line(n: integer) -> integer {
            frames = stack_trace();
            if len(frames) > 0 { frames[0].line + n - n } else { -1 + n - n }
         }"
    )
    .expr("check_line(0)")
    .result(Value::Int(4)); // called from expr wrapper at line ~4
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
#[ignore = "T1.4: tuple return from function + temp element access needs work-var codegen"]
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
#[ignore = "T1.4: tuple destructuring codegen needs tuple-returning function support"]
fn tuple_destructure_basic() {
    // LHS destructuring: (a, b) = expr.
    code!("fn pair(x: integer) -> (integer, integer) { (x, x * 2) }")
        .expr("(a, b) = pair(5); a + b")
        .result(Value::Int(15));
}

#[test]
#[ignore = "T1.4: tuple element assignment codegen not yet implemented"]
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
#[ignore = "T1.4: tuple with text element needs text-return calling convention"]
fn tuple_with_text() {
    // Tuple containing a text element — verify text is accessible.
    code!("fn greet(name: text) -> (integer, text) { (len(name), name) }")
        .expr("greet(\"hello\").0")
        .result(Value::Int(5));
}

// ── T1.5 — Reference-tuple parameters ────────────────────────────────────────

#[test]
#[ignore = "T1.5: RefVar(Tuple) element access not yet wired in operators.rs"]
fn ref_tuple_param_swap() {
    // &(integer, integer) parameter — swap elements via reference.
    code!(
        "fn swap(pair: &(integer, integer)) {
            tmp = pair.0;
            pair.0 = pair.1;
            pair.1 = tmp;
         }"
    )
    .expr("p = (3, 7); swap(&p); p.0 * 10 + p.1")
    .result(Value::Int(73));
}

// ── T1.6 — Tuple-aware mutation guard ────────────────────────────────────────

#[test]
#[ignore = "T1.6: tuple mutation guard requires T1.5 ref-param element access"]
fn ref_tuple_unused_mutation_error() {
    // &(integer, integer) parameter that is never mutated — should produce a warning.
    code!("fn read_only(pair: &(integer, integer)) -> integer { pair.0 + pair.1 }")
        .expr("read_only(&(3, 7))")
        .warning("Parameter 'pair' does not need to be a reference")
        .result(Value::Int(10));
}

// ── A5.3 — Closure capture at call site ─────────────────────────────────────

#[test]
#[ignore = "A5.3: closure capture at call site not yet implemented"]
fn closure_capture_integer() {
    // A lambda captures an integer from the enclosing scope.
    expr!("x = 10; f = fn(y: integer) -> integer { x + y }; f(5)").result(Value::Int(15));
}

#[test]
#[ignore = "A5.3: closure capture at call site not yet implemented"]
fn closure_capture_after_change() {
    // Capture is by value at the point of lambda creation — changing original after
    // creation does not affect the captured value.
    expr!("x = 10; f = fn(y: integer) -> integer { x + y }; x = 99; f(5)").result(Value::Int(15));
}

#[test]
#[ignore = "A5.3: closure capture at call site not yet implemented"]
fn closure_capture_multiple() {
    // A lambda captures two variables from the enclosing scope.
    expr!("a = 3; b = 7; f = fn(x: integer) -> integer { a + b + x }; f(10)")
        .result(Value::Int(20));
}

#[test]
#[ignore = "A5.3: closure capture at call site not yet implemented"]
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
