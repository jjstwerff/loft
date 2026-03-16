// Copyright (c) 2021-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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
fn div_by_const_zero() {
    expr!("2 / 0")
        .tp(INTEGER)
        .warning("Division by constant zero — result is always null at div_by_const_zero:2:25");
}

#[test]
fn rem_by_const_zero() {
    expr!("5 % 0")
        .tp(INTEGER)
        .warning("Modulo by constant zero — result is always null at rem_by_const_zero:2:25");
}

#[test]
fn append_fn() {
    code!("fn append(ch: character) -> text { \"abc_de\" + ch }")
        .expr("append('x')")
        .result(Value::str("abc_dex"));
}

#[test]
fn append_str() {
    code!("fn append(ch: character) -> text { s=\"abc_de\"; s += ch; s }")
        .expr("append('x')")
        .result(Value::str("abc_dex"));
}

#[test]
fn continue_loop() {
    code!("fn routine() -> integer {b = 0; for a in 0..10 { if a == 2 {continue} if a > 5 {return b} b += a }; b}")
    .expr("routine()").result(Value::Int(13));
}

#[test]
fn call_void() {
    code!("fn routine(a: integer) {return;}")
        .expr("routine(5)")
        .warning("Parameter a is never read at call_void:1:25");
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

#[test]
fn inner_loop() {
    expr!(
        "
s = \"\";
for i in 0..10 {
    for j in 0..10 {
        if j > i {
            i#continue
        }
        s += \"{i}{j},\";
        if len(s) > 100 {
            i#break
        }
    }
}
s
"
    )
    .result(Value::str("00,10,11,20,21,22,30,31,32,33,40,41,42,43,44,50,51,52,53,54,55,60,61,62,63,64,65,66,70,71,72,73,74,75,"));
}

#[test]
fn recursion() {
    code!(
        "fn first(s: State, c: integer) -> integer {
	if s == Start {
		s = Ongoing
	} else if c > 10 {
		s = Halt
	}
	second(s, c)
}

fn second(s: State, c: integer) -> integer {
	if s != Halt {
		first(s, c + 1)
	} else {
		1 + c
	}
}

enum State {
	Start,
	Ongoing,
	Halt
}"
    )
    .expr("first(Start, 0)")
    .result(Value::Int(12));
}

// ── T1-7: Null-coalescing operator ?? ─────────────────────────────────────────

/// Non-null integer: ?? returns the LHS value.
#[test]
fn null_coerce_integer_non_null() {
    code!(
        "fn test() {
    x = 42;
    r = x ?? 0;
    assert(r == 42, \"r: {r}\");
}"
    );
}

/// Null integer: ?? returns the default.
#[test]
fn null_coerce_integer_null() {
    code!(
        "fn test() {
    x = null;
    r = x ?? 99;
    assert(r == 99, \"r: {r}\");
}"
    );
}

/// Non-null field access: ?? returns the field value.
#[test]
fn null_coerce_field_non_null() {
    code!(
        "struct Row { val: integer }
fn test() {
    r = Row { val: 7 };
    result = r.val ?? 0;
    assert(result == 7, \"result: {result}\");
}"
    );
}

/// Chaining: `a ?? b ?? c` — a is non-null, so result is a.
#[test]
fn null_coerce_chain_first_non_null() {
    code!(
        "fn test() {
    a = 1;
    b = 2;
    c = 3;
    r = a ?? b ?? c;
    assert(r == 1, \"r: {r}\");
}"
    );
}

/// Dead assignment: variable overwritten before first read.
#[test]
fn dead_assignment_basic() {
    code!(
        "fn test() {
    x = 1;
    x = 2;
    assert(x == 2, \"x\");
}"
    )
    .warning(
        "Dead assignment — 'x' is overwritten before being read at dead_assignment_basic:2:11",
    );
}

/// No dead assignment when variable is read between writes.
#[test]
fn dead_assignment_read_between() {
    code!(
        "fn test() {
    x = 1;
    assert(x == 1, \"x\");
    x = 2;
    assert(x == 2, \"x\");
}"
    );
}

/// Dead assignment: underscore-prefixed variable is exempt.
#[test]
fn dead_assignment_underscore_exempt() {
    code!(
        "fn test() {
    _x = 1;
    _x = 2;
    assert(_x == 2, \"x\");
}"
    );
}

/// Dead assignment: += is not a dead write (it reads the variable).
#[test]
fn dead_assignment_augmented_not_dead() {
    code!(
        "fn test() {
    x = 1;
    x += 2;
    assert(x == 3, \"x\");
}"
    );
}

/// Dead assignment: conditional update after initialization is not dead.
#[test]
fn dead_assignment_branch_not_dead() {
    code!(
        "fn test() {
    result = 0;
    if true {
        result = 42;
    };
    assert(result == 42, \"result\");
}"
    );
}

/// Dead assignment: writes in sibling branches do not warn each other.
#[test]
fn dead_assignment_sibling_branches() {
    code!(
        "fn test() {
    x = 0;
    if true {
        x = 1;
    } else {
        x = 2;
    };
    assert(x > 0, \"x\");
}"
    );
}

/// Dead assignment: parameter reassignment in branches does not warn.
#[test]
fn dead_assignment_param_branch() {
    code!(
        "enum State { Start, Ongoing, Halt }
fn step(s: State) -> State {
    if s == Start {
        s = Ongoing;
    } else {
        s = Halt;
    };
    s
}
fn test() {
    assert(step(Start) == Ongoing, \"step\");
}"
    );
}

/// Dead assignment: initialization before loop is not dead.
#[test]
fn dead_assignment_loop_init() {
    code!(
        "fn test() {
    sum = 0;
    for i in 1..4 {
        sum = sum + i;
    };
    assert(sum == 6, \"sum\");
}"
    );
}

/// Dead assignment: match arm writes do not trigger across arms.
#[test]
fn dead_assignment_match_arms() {
    code!(
        "enum Color { Red, Green, Blue }
fn label(c: Color) -> integer {
    result = 0;
    match c {
        Red => result = 1
        Green => result = 2
        Blue => result = 3
    };
    result
}
fn test() {
    assert(label(Red) == 1, \"red\");
}"
    );
}

/// Chaining: first two are null, third is non-null.
#[test]
fn null_coerce_chain_last_non_null() {
    code!(
        "fn test() {
    a = null;
    b = null;
    c = 3;
    r = a ?? b ?? c;
    assert(r == 3, \"r: {r}\");
}"
    );
}
