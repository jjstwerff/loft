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

// ── TR1.2 — StackFrame + ArgValue type declarations ─────────────────────────
// Verify the types from default/04_stacktrace.loft can be constructed and used.

#[test]
#[ignore = "TR1.2: ArgValue/StackFrame type declarations not yet in default library"]
fn stacktrace_argvalue_construct() {
    code!(
        "fn test() -> integer {
            v = ArgValue.IntVal { n: 42 };
            match v {
                IntVal { n } => n,
                _ => 0,
            }
         }"
    )
    .result(Value::Int(42));
}

#[test]
#[ignore = "TR1.2: ArgValue/StackFrame type declarations not yet in default library"]
fn stacktrace_arginfo_construct() {
    code!(
        "fn test() -> text {
            info = ArgInfo { name: \"x\", type_name: \"integer\", value: ArgValue.IntVal { n: 7 } };
            info.name
         }"
    )
    .result(Value::str("x"));
}

#[test]
#[ignore = "TR1.2: ArgValue/StackFrame type declarations not yet in default library"]
fn stacktrace_frame_construct() {
    code!(
        "fn test() -> text {
            frame = StackFrame { function: \"main\", file: \"test.loft\", line: 1 };
            frame.function
         }"
    )
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
