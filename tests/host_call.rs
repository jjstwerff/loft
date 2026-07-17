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

#[test]
fn formatter_enum_variant_if_body_is_a_block() {
    // Regression: `if x == Enum.Variant { <stmts> }` — the CamelCase variant name
    // (`Directory`) reached via `.` must NOT be mistaken for a `Directory { … }` struct
    // literal.  When it was, the if-body was rendered as a COMMA container and the last
    // statement got a stray trailing comma (`walk_into(cp, out);,`), which is invalid
    // loft.  The formatter mirrors the compiler here: a dotted name (`Format.Directory`)
    // is an enum-value access, never a struct construction (see objects.rs::parse_object,
    // gated on `Name` being a Struct/EnumValue type + the next token being `{`).
    let src = std::fs::read_to_string("tools/fmt/whole.loft").expect("formatter source");
    let mut p = Program::from_source(&src).expect("formatter compiles");
    let input =
        "enum E { A, B }\nfn f(m: E) {\n  if m == E.A {\n    x = 1;\n    print(\"{x}\");\n  }\n}\n";
    let out = p
        .call("format", &[Value::Text(input.into())])
        .unwrap()
        .into_text()
        .unwrap();
    // No stray comma anywhere (the exact bug symptom).
    assert!(
        !out.contains(";,"),
        "stray comma from struct-lit misclassification: {out:?}"
    );
    // The body is a real multi-statement BLOCK: each statement on its own line, not
    // comma-joined and not collapsed onto one line.
    assert!(
        out.contains("x = 1;\n"),
        "if-body statement 1 on its own line: {out:?}"
    );
    assert!(
        out.contains("print(\"{x}\");"),
        "if-body statement 2 preserved: {out:?}"
    );
    assert!(
        !out.contains("x = 1; print"),
        "multi-statement if-body must not be inlined as a container: {out:?}"
    );
}

#[test]
fn formatter_width_counts_characters_not_bytes() {
    // Regression (@PLN110): the formatter measures DISPLAY width, which is a CHARACTER
    // count — not a byte count. This assert line is 89 characters but 103 UTF-8 bytes
    // (em-dashes are 1 char / 3 bytes); it fits the 100-column budget and must stay on
    // one line. When width was measured with `size(text)` (which became the BYTE count
    // at the flip), the formatter over-measured multi-byte lines and wrapped ones that
    // visually fit — the fix uses `len` (characters) for column/width math.
    let src = std::fs::read_to_string("tools/fmt/whole.loft").expect("formatter source");
    let mut p = Program::from_source(&src).expect("formatter compiles");
    let input = "fn f(a: integer, b: integer) {\n  assert(a == b, \"mismatch —  —  —  —  —  —  — end padding to reach target length here\");\n}\n";
    let out = p
        .call("format", &[Value::Text(input.into())])
        .unwrap()
        .into_text()
        .unwrap();
    // the assert's args stay on ONE line (char-width 89 < 100); a byte-width measure
    // (103 > 100) would have wrapped them onto separate lines (`assert(\n a == b, …`).
    assert!(
        out.contains("assert(a == b, \"mismatch"),
        "multi-byte line under the char budget must stay one line: {out:?}"
    );
    assert!(
        !out.contains("assert(\n"),
        "args must not wrap — width is character count, not bytes: {out:?}"
    );
}
