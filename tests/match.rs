// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Integration tests for T1-4: match expression.

extern crate loft;

use loft::data::Value;

mod testing;

// ── Plain enum ────────────────────────────────────────────────────────────────

/// All four variants covered, no wildcard — exhaustiveness satisfied.
#[test]

fn plain_all_arms() {
    code!("enum Direction { North, East, South, West }")
        .expr(
            "d = South;
match d {
    North => \"N\",
    East  => \"E\",
    South => \"S\",
    West  => \"W\"
}",
        )
        .result(Value::str("S"));
}

/// Partial coverage with a wildcard `_` arm.
#[test]
fn plain_wildcard() {
    code!("enum Direction { North, East, South, West }")
        .expr(
            "d = West;
match d {
    North => \"north\",
    _     => \"other\"
}",
        )
        .result(Value::str("other"));
}

/// Wildcard arm catches the first variant too.
#[test]
fn plain_wildcard_first() {
    code!("enum Direction { North, East, South, West }")
        .expr(
            "d = North;
match d {
    South => \"south\",
    _     => \"not south\"
}",
        )
        .result(Value::str("not south"));
}

/// Missing variant without wildcard — compile-time error.
#[test]
fn plain_missing_arm() {
    code!(
        "enum Direction { North, East, South, West }

fn test() {
    d = North;
    match d {
        North => \"N\",
        East  => \"E\",
        South => \"S\"
    }
}"
    )
    .error("match on Direction is not exhaustive — missing: West at plain_missing_arm:10:2");
}

/// match used as a void statement (result is dropped).
#[test]
fn plain_as_statement() {
    code!(
        "enum Direction { North, East, South, West }

pub fn label(d: Direction) -> text {
    result = \"\";
    match d {
        North => result = \"N\",
        East  => result = \"E\",
        South => result = \"S\",
        West  => result = \"W\"
    }
    result
}"
    )
    .expr("label(East)")
    .result(Value::str("E"));
}

/// match produces an integer value used in arithmetic.
#[test]
fn plain_as_integer_value() {
    code!("enum Priority { Low, Medium, High }")
        .expr(
            "p = High;
v = match p {
    Low    => 1,
    Medium => 5,
    High   => 10
};
v * 2",
        )
        .result(Value::Int(20));
}

/// match used inside a function return position.
#[test]
fn plain_in_function() {
    code!(
        "enum Direction { North, East, South, West }

pub fn label(d: Direction) -> text {
    match d {
        North => \"N\",
        East  => \"E\",
        South => \"S\",
        West  => \"W\"
    }
}"
    )
    .expr("label(West)")
    .result(Value::str("W"));
}

// ── Struct enum ───────────────────────────────────────────────────────────────

/// Struct enum dispatch without field binding.
#[test]
fn struct_no_binding() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect   { width: float, height: float }
}"
    )
    .expr(
        "s = Circle { radius: 3.0 };
match s {
    Circle => true,
    _      => false
}",
    )
    .result(Value::Boolean(true));
}

/// Struct enum — single field binding.
#[test]
fn struct_single_field() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect   { width: float, height: float },
    Square { side:  float }
}"
    )
    .expr(
        "s = Circle { radius: 2.0 };
match s {
    Circle { radius }      => radius * radius,
    Rect   { width, height } => width * height,
    Square { side }        => side * side
}",
    )
    .result(Value::Float(4.0));
}

/// Struct enum — two-field binding on the matching arm.
#[test]
fn struct_multi_field() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect   { width: float, height: float },
    Square { side:  float }
}"
    )
    .expr(
        "s = Rect { width: 4.0, height: 5.0 };
match s {
    Circle { radius }        => radius * radius,
    Rect   { width, height } => width * height,
    Square { side }          => side * side
}",
    )
    .result(Value::Float(20.0));
}

/// All three Shape variants covered, no wildcard — exhaustiveness satisfied.
#[test]
fn struct_all_variants() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect   { width: float, height: float },
    Square { side:  float }
}

pub fn area(s: Shape) -> float {
    match s {
        Circle { radius }        => PI * radius * radius,
        Rect   { width, height } => width * height,
        Square { side }          => side * side
    }
}"
    )
    .expr(
        "c = area(Circle { radius: 1.0 });
r = area(Rect { width: 3.0, height: 4.0 });
s = area(Square { side: 5.0 });
\"{c} {r} {s}\"",
    )
    .result(Value::str("3.141592653589793 12 25"));
}

/// Struct enum missing variant without wildcard — compile-time error.
#[test]
fn struct_missing_arm() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect   { width: float, height: float },
    Square { side:  float }
}

fn test() {
    s = Circle { radius: 1.0 };
    match s {
        Circle { radius } => radius,
        Rect { width, height } => width * height
    }
}"
    )
    .error("match on Shape is not exhaustive — missing: Square at struct_missing_arm:13:2");
}

// ── Nesting and composition ───────────────────────────────────────────────────

/// match expression nested inside another match arm.
#[test]
fn match_nested() {
    code!(
        "enum X { A, B }
enum Y { P, Q }"
    )
    .expr(
        "x = A; y = Q;
match x {
    A => match y {
        P => 1,
        Q => 2
    },
    B => 0
}",
    )
    .result(Value::Int(2));
}

/// match result fed directly into a function call.
#[test]
fn match_in_call() {
    code!("enum Flag { On, Off }")
        .expr(
            "f = On;
len(match f {
    On  => \"enabled\",
    Off => \"disabled\"
})",
        )
        .result(Value::Int(7));
}

// ── Error cases ───────────────────────────────────────────────────────────────

/// match on an unsupported type (vector) is a compile-time error.
#[test]
fn match_non_enum() {
    code!(
        "fn test() {
    v = [1, 2, 3];
    match v {
        _ => 0
    }
}"
    )
    .error("match requires an enum, struct, or scalar type at match_non_enum:3:14");
}

/// Arms returning incompatible types — compile-time error.
#[test]
fn match_type_mismatch() {
    code!(
        "enum Direction { North, East, South, West }

fn test() {
    d = North;
    match d {
        North => \"text\",
        _     => 42
    }
}"
    )
    .error("cannot unify: text and integer at match_type_mismatch:8:6");
}

/// Duplicate arm for the same variant — compile-time warning.
#[test]
fn match_duplicate_arm() {
    code!(
        "enum Direction { North, East, South, West }

fn test() {
    d = North;
    match d {
        North => \"first\",
        North => \"second\",
        _     => \"other\"
    }
}"
    )
    .warning("unreachable arm: North already matched at match_duplicate_arm:7:17");
}

/// T1-18: match on a plain struct — bind fields.
#[test]
fn match_struct_destructure() {
    code!(
        "struct Point { x: float, y: float }

fn test() {
    p = Point { x: 3.0, y: 4.0 };
    r = match p {
        Point { x, y } => x + y
    };
    assert(r == 7.0, \"r: {r}\");
}"
    );
}

/// T1-18: match on a struct — no field bindings, just value.
#[test]
fn match_struct_no_fields() {
    code!(
        "struct Point { x: float, y: float }

fn test() {
    p = Point { x: 1.0, y: 2.0 };
    r = match p {
        Point => 42
    };
    assert(r == 42, \"r: {r}\");
}"
    );
}

/// T1-14: match on integer values with literal patterns.
#[test]
fn match_scalar_integer() {
    code!(
        "fn test() {
    x = 42;
    r = match x {
        1  => \"one\",
        42 => \"forty-two\",
        _  => \"other\"
    };
    assert(r == \"forty-two\", \"r\");
}"
    );
}

/// T1-14: match on text values.
#[test]
fn match_scalar_text() {
    code!(
        "fn test() {
    cmd = \"help\";
    r = match cmd {
        \"quit\" => 0,
        \"help\" => 1,
        _      => -1
    };
    assert(r == 1, \"r: {r}\");
}"
    );
}

/// T1-14: match on boolean — exhaustive (both true and false covered).
#[test]
fn match_scalar_boolean() {
    code!(
        "fn test() {
    flag = true;
    r = match flag {
        true  => \"on\",
        false => \"off\"
    };
    assert(r == \"on\", \"r\");
}"
    );
}

/// Commas between match arms are accepted and optional.
#[test]
fn match_with_commas() {
    code!("enum Direction { North, East, South, West }")
        .expr(
            "d = East;
match d {
    North => \"N\",
    East  => \"E\",
    South => \"S\",
    West  => \"W\",
}",
        )
        .result(Value::str("E"));
}

/// Scalar match with no matching arm and no wildcard returns null.
#[test]
fn match_scalar_no_match() {
    code!(
        "fn test() {
    x = 99;
    r = match x {
        1 => 10,
        2 => 20
    };
    assert(r == null, \"r should be null\");
}"
    );
}

/// Scalar match with a single arm.
#[test]
fn match_scalar_single_arm() {
    code!(
        "fn test() {
    x = 5;
    r = match x {
        5 => \"five\"
    };
    assert(r == \"five\", \"r\");
}"
    );
}

/// Negative integer literal in match arm.
#[test]
fn match_scalar_negative() {
    code!(
        "fn test() {
    x = -1;
    r = match x {
        -1 => \"neg\",
        0  => \"zero\",
        1  => \"pos\",
        _  => \"other\"
    };
    assert(r == \"neg\", \"r\");
}"
    );
}

/// P46: block expression as match arm body — was a segfault, now works.
#[test]
fn match_arm_block_body() {
    code!(
        "fn test() {
    x = 2;
    r = match x {
        1 => { 10 + 1 },
        2 => { 20 + 2 },
        _ => 0
    };
    assert(r == 22, \"r: {r}\");
}"
    );
}
