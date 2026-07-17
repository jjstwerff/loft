//! Validation for the `loft::host` Rust→loft call API (P1).
//! The load-bearing check: a host `call(f, args)` equals an in-language call of `f`.

use loft::host::{LoftError, Program, Value};

const SRC: &str = r#"
fn add(a: integer, b: integer) -> integer { a + b }
fn greet(name: text) -> text { "hi {name}" }
fn is_big(n: integer) -> boolean { n > 100 }
fn dbl(x: single) -> single { x + x }
fn boom(n: integer) -> integer { assert(n > 0, "must be positive"); n }
"#;

fn prog() -> Program {
    Program::from_source(SRC).expect("program compiles")
}

#[test]
fn int_in_int_out() {
    let mut p = prog();
    assert_eq!(
        p.call("add", &[Value::Int(2), Value::Int(3)]).unwrap(),
        Value::Int(5)
    );
    // second call on the same program reuses the frame correctly
    assert_eq!(
        p.call("add", &[Value::Int(40), Value::Int(2)]).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn text_in_text_out() {
    let mut p = prog();
    // exercises the hidden text work-buffer path
    assert_eq!(
        p.call("greet", &[Value::Text("Ada".into())]).unwrap(),
        Value::Text("hi Ada".into())
    );
}

#[test]
fn boolean_and_single_returns() {
    let mut p = prog();
    assert_eq!(
        p.call("is_big", &[Value::Int(500)]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        p.call("is_big", &[Value::Int(5)]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        p.call("dbl", &[Value::Float(9.0)]).unwrap(),
        Value::Float(18.0)
    );
}

#[test]
fn unknown_function() {
    let mut p = prog();
    assert!(matches!(p.call("nope", &[]), Err(LoftError::UnknownFn(_))));
}

#[test]
fn arg_count_mismatch() {
    let mut p = prog();
    assert!(matches!(
        p.call("add", &[Value::Int(1)]),
        Err(LoftError::ArgCount {
            expected: 2,
            got: 1,
            ..
        })
    ));
}

#[test]
fn arg_type_mismatch() {
    let mut p = prog();
    assert!(matches!(
        p.call("add", &[Value::Text("x".into()), Value::Int(2)]),
        Err(LoftError::ArgType { index: 0, .. })
    ));
}

#[test]
fn runtime_error_surfaces() {
    let mut p = prog();
    // assert failure raises → Runtime error, not a panic
    assert!(matches!(
        p.call("boom", &[Value::Int(0)]),
        Err(LoftError::Runtime(_))
    ));
    // and the program is still usable afterwards (error was drained)
    assert_eq!(p.call("boom", &[Value::Int(7)]).unwrap(), Value::Int(7));
}

#[test]
fn formatter_dogfood() {
    // The driver: call the loft-written formatter's `format(text) -> text`.
    let src = std::fs::read_to_string("tools/fmt/whole.loft").expect("formatter source");
    let mut p = Program::from_source(&src).expect("formatter compiles");
    let input = "struct P{x:integer,y:integer}\n";
    let out = p
        .call("format", &[Value::Text(input.into())])
        .unwrap()
        .into_text()
        .unwrap();
    // definitions expand (the canonical rule); at minimum it changed + is non-empty
    assert!(out.contains("struct P {"), "got: {out:?}");
    assert!(out.contains("x: integer,"), "got: {out:?}");
}
