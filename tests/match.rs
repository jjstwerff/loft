// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Integration tests for T1-4: match expression.

extern crate loft;

use loft::data::Value;

mod testing;

// ── Plain enum ────────────────────────────────────────────────────────────────

/// All four variants covered, no wildcard — exhaustiveness satisfied.
#[test]
#[ignore = "T1-4: not yet implemented"]
fn plain_all_arms() {
    code!(
        "enum Direction { North, East, South, West }"
    )
    .expr(
        "d = South;
match d {
    North => \"N\"
    East  => \"E\"
    South => \"S\"
    West  => \"W\"
}",
    )
    .result(Value::str("S"));
}

/// Partial coverage with a wildcard `_` arm.
#[test]
#[ignore = "T1-4: not yet implemented"]
fn plain_wildcard() {
    code!(
        "enum Direction { North, East, South, West }"
    )
    .expr(
        "d = West;
match d {
    North => \"north\"
    _     => \"other\"
}",
    )
    .result(Value::str("other"));
}

/// Wildcard arm catches the first variant too.
#[test]
#[ignore = "T1-4: not yet implemented"]
fn plain_wildcard_first() {
    code!(
        "enum Direction { North, East, South, West }"
    )
    .expr(
        "d = North;
match d {
    South => \"south\"
    _     => \"not south\"
}",
    )
    .result(Value::str("not south"));
}

/// Missing variant without wildcard — compile-time error.
#[test]
#[ignore = "T1-4: not yet implemented"]
fn plain_missing_arm() {
    code!(
        "enum Direction { North, East, South, West }

fn test() {
    d = North;
    match d {
        North => \"N\"
        East  => \"E\"
        South => \"S\"
    }
}"
    )
    .error("match on Direction is not exhaustive — missing: West");
}

/// match used as a void statement (result is dropped).
#[test]
#[ignore = "T1-4: not yet implemented"]
fn plain_as_statement() {
    code!(
        "enum Direction { North, East, South, West }

pub fn label(d: Direction) -> text {
    result = \"\";
    match d {
        North => result = \"N\"
        East  => result = \"E\"
        South => result = \"S\"
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
#[ignore = "T1-4: not yet implemented"]
fn plain_as_integer_value() {
    code!(
        "enum Priority { Low, Medium, High }"
    )
    .expr(
        "p = High;
v = match p {
    Low    => 1
    Medium => 5
    High   => 10
};
v * 2",
    )
    .result(Value::Int(20));
}

/// match used inside a function return position.
#[test]
#[ignore = "T1-4: not yet implemented"]
fn plain_in_function() {
    code!(
        "enum Direction { North, East, South, West }

pub fn label(d: Direction) -> text {
    match d {
        North => \"N\"
        East  => \"E\"
        South => \"S\"
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
#[ignore = "T1-4: not yet implemented"]
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
    Circle => true
    _      => false
}",
    )
    .result(Value::Boolean(true));
}

/// Struct enum — single field binding.
#[test]
#[ignore = "T1-4: not yet implemented"]
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
    Circle { radius }      => radius * radius
    Rect   { width, height } => width * height
    Square { side }        => side * side
}",
    )
    .result(Value::Float(4.0));
}

/// Struct enum — two-field binding on the matching arm.
#[test]
#[ignore = "T1-4: not yet implemented"]
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
    Circle { radius }        => radius * radius
    Rect   { width, height } => width * height
    Square { side }          => side * side
}",
    )
    .result(Value::Float(20.0));
}

/// All three Shape variants covered, no wildcard — exhaustiveness satisfied.
#[test]
#[ignore = "T1-4: not yet implemented"]
fn struct_all_variants() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect   { width: float, height: float },
    Square { side:  float }
}

pub fn area(s: Shape) -> float {
    match s {
        Circle { radius }        => PI * radius * radius
        Rect   { width, height } => width * height
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
#[ignore = "T1-4: not yet implemented"]
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
        Circle { radius } => radius
        Rect { width, height } => width * height
    }
}"
    )
    .error("match on Shape is not exhaustive — missing: Square");
}

// ── Nesting and composition ───────────────────────────────────────────────────

/// match expression nested inside another match arm.
#[test]
#[ignore = "T1-4: not yet implemented"]
fn match_nested() {
    code!(
        "enum X { A, B }
enum Y { P, Q }"
    )
    .expr(
        "x = A; y = Q;
match x {
    A => match y {
        P => 1
        Q => 2
    }
    B => 0
}",
    )
    .result(Value::Int(2));
}

/// match result fed directly into a function call.
#[test]
#[ignore = "T1-4: not yet implemented"]
fn match_in_call() {
    code!(
        "enum Flag { On, Off }"
    )
    .expr(
        "f = On;
len(match f {
    On  => \"enabled\"
    Off => \"disabled\"
})",
    )
    .result(Value::Int(7));
}

// ── Error cases ───────────────────────────────────────────────────────────────

/// match on a non-enum type is a compile-time error.
#[test]
#[ignore = "T1-4: not yet implemented"]
fn match_non_enum() {
    code!(
        "fn test() {
    n = 42;
    match n {
        _ => 0
    }
}"
    )
    .error("match requires an enum type");
}

/// Arms returning incompatible types — compile-time error.
#[test]
#[ignore = "T1-4: not yet implemented"]
fn match_type_mismatch() {
    code!(
        "enum Direction { North, East, South, West }

fn test() {
    d = North;
    match d {
        North => \"text\"
        _     => 42
    }
}"
    )
    .error("cannot unify");
}

/// Duplicate arm for the same variant — compile-time warning.
#[test]
#[ignore = "T1-4: not yet implemented"]
fn match_duplicate_arm() {
    code!(
        "enum Direction { North, East, South, West }

fn test() {
    d = North;
    match d {
        North => \"first\"
        North => \"second\"
        _     => \"other\"
    }
}"
    )
    .warning("unreachable arm: North already matched");
}
