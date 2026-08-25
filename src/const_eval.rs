// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR -> bytecode)

//! Compile-time constant folding for the O8 bulk initialisation optimisation.
//!
//! `const_eval` evaluates a pure expression at compile time, returning
//! `Some(literal)` when fully evaluable or `None` for runtime fallback.
//! Conservative: any unrecognised pattern returns `None`.

#![allow(clippy::cast_precision_loss)]
#![allow(dead_code)] // used by parser (lib) and will be used by O8.1 optimisation

use crate::data::{Data, Value};

/// Substitute `Var(var_nr)` with `replacement` throughout the IR tree,
/// then const-fold the result.  Used for O8.5 comprehension unrolling.
#[must_use]
pub fn const_eval_with_var(
    val: &Value,
    var_nr: u16,
    replacement: &Value,
    data: &Data,
) -> Option<Value> {
    let substituted = substitute_var(val, var_nr, replacement);
    const_eval(&substituted, data)
}

fn substitute_var(val: &Value, var_nr: u16, replacement: &Value) -> Value {
    match val {
        Value::Var(v) if *v == var_nr => replacement.clone(),
        Value::Call(op, args) => {
            let new_args: Vec<Value> = args
                .iter()
                .map(|a| substitute_var(a, var_nr, replacement))
                .collect();
            Value::Call(*op, new_args)
        }
        Value::If(cond, then_val, else_val) => Value::If(
            Box::new(substitute_var(cond, var_nr, replacement)),
            Box::new(substitute_var(then_val, var_nr, replacement)),
            Box::new(substitute_var(else_val, var_nr, replacement)),
        ),
        other => other.clone(),
    }
}

/// Try to evaluate `val` as a compile-time constant.
///
/// Safety invariants:
/// - Integer arithmetic uses wrapping ops to match interpreter overflow semantics
/// - Division/modulo by zero → `None` (runtime returns null)
/// - `i32::MIN / -1` and `i64::MIN / -1` → `None` (would panic in debug Rust)
/// - Float NaN propagation handled by IEEE 754
/// - Non-finite float → int cast → `None`
#[must_use]
pub fn const_eval(val: &Value, data: &Data) -> Option<Value> {
    // `Value::unspan`'s doc makes this an obligation: "every second-pass site that
    // pattern-matches a specific Value variant must call `code.unspan()` first".  The arms
    // below discriminate on `Call` / `If` / the literal variants, so a `Span` wrapper sends
    // the value to `_ => None` and the fold is skipped.  The parser callers already unspan
    // their own argument; what they cannot reach is the RECURSION here, and sub-expressions
    // carry their own spans.  Measured over the 858-program corpus: 1000 spanned values
    // arrive, 998 would not fold anyway, 2 are folds being lost.
    let val = val.unspan();
    match val {
        Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Single(_) | Value::Boolean(_) => {
            Some(val.clone())
        }
        Value::Call(op, args) => {
            let folded: Option<Vec<Value>> = args.iter().map(|a| const_eval(a, data)).collect();
            let args = folded?;
            let name = &data.def(*op).name;
            fold_op(name, &args)
        }
        Value::If(cond, then_val, else_val) => {
            if let Some(Value::Boolean(c)) = const_eval(cond, data) {
                const_eval(if c { then_val } else { else_val }, data)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn fold_op(name: &str, args: &[Value]) -> Option<Value> {
    match (name, args) {
        // --- integer arithmetic ---
        ("OpAddInt", [Value::Int(a), Value::Int(b)]) => Some(Value::Int(a.wrapping_add(*b))),
        ("OpMinInt", [Value::Int(a), Value::Int(b)]) => Some(Value::Int(a.wrapping_sub(*b))),
        ("OpMulInt", [Value::Int(a), Value::Int(b)]) => Some(Value::Int(a.wrapping_mul(*b))),
        ("OpDivInt", [Value::Int(a), Value::Int(b)])
            if *b != 0 && !(*a == i32::MIN && *b == -1) =>
        {
            Some(Value::Int(a / b))
        }
        ("OpModInt", [Value::Int(a), Value::Int(b)])
            if *b != 0 && !(*a == i32::MIN && *b == -1) =>
        {
            Some(Value::Int(a % b))
        }
        ("OpMinSingleInt", [Value::Int(a)]) => Some(Value::Int(a.wrapping_neg())),
        // --- integer bitwise ---
        ("OpAndInt", [Value::Int(a), Value::Int(b)]) => Some(Value::Int(a & b)),
        ("OpOrInt", [Value::Int(a), Value::Int(b)]) => Some(Value::Int(a | b)),
        ("OpXorInt", [Value::Int(a), Value::Int(b)]) => Some(Value::Int(a ^ b)),
        // --- integer comparison ---
        ("OpEqInt", [Value::Int(a), Value::Int(b)]) => Some(Value::Boolean(*a == *b)),
        ("OpNeInt", [Value::Int(a), Value::Int(b)]) => Some(Value::Boolean(*a != *b)),
        ("OpLtInt", [Value::Int(a), Value::Int(b)]) => Some(Value::Boolean(*a < *b)),
        ("OpLeInt", [Value::Int(a), Value::Int(b)]) => Some(Value::Boolean(*a <= *b)),
        // --- float arithmetic ---
        ("OpAddFloat", [Value::Float(a), Value::Float(b)]) => Some(Value::Float(a + b)),
        ("OpMinFloat", [Value::Float(a), Value::Float(b)]) => Some(Value::Float(a - b)),
        ("OpMulFloat", [Value::Float(a), Value::Float(b)]) => Some(Value::Float(a * b)),
        ("OpDivFloat", [Value::Float(a), Value::Float(b)]) => Some(Value::Float(a / b)),
        ("OpMinSingleFloat", [Value::Float(a)]) => Some(Value::Float(-a)),
        // --- nullary math constants (@PLN102 case-C residual) — same values as the runtime
        //     ops in `fill.rs`, so a `PI`/`E` divisor const-folds and is proven non-zero
        //     (`x / PI` non-null), and the domain lattice reads their sign. ---
        ("OpMathPiFloat", []) => Some(Value::Float(std::f64::consts::PI)),
        ("OpMathEFloat", []) => Some(Value::Float(std::f64::consts::E)),
        // --- single arithmetic ---
        ("OpAddSingle", [Value::Single(a), Value::Single(b)]) => Some(Value::Single(a + b)),
        ("OpMinSingle", [Value::Single(a), Value::Single(b)]) => Some(Value::Single(a - b)),
        ("OpMulSingle", [Value::Single(a), Value::Single(b)]) => Some(Value::Single(a * b)),
        ("OpDivSingle", [Value::Single(a), Value::Single(b)]) => Some(Value::Single(a / b)),
        ("OpMinSingleSingle", [Value::Single(a)]) => Some(Value::Single(-a)),
        // --- casts ---
        ("OpConvFloatFromInt", [Value::Int(a)]) => Some(Value::Float(f64::from(*a))),
        // @PLN88: a folded `float as integer` is a USER value — route through int_const so
        // a large float (`3e9 as integer`) becomes `Long`, not a silent `as i32` truncation.
        ("OpConvIntFromFloat", [Value::Float(a)]) if a.is_finite() => {
            Some(Value::int_const(*a as i64))
        }
        ("OpConvSingleFromInt", [Value::Int(a)]) => Some(Value::Single(*a as f32)),
        // --- boolean ---
        ("OpNot", [Value::Boolean(a)]) => Some(Value::Boolean(!a)),
        _ => None,
    }
}

/// Fold `val` to a literal, looking through the source-position markers the parser
/// leaves between the operands.
///
/// A `Span` records WHERE an operand was written; it is not part of the value, and an
/// expression straight out of the parser carries one around every sub-expression
/// (`(BASE + 1) * 2` is three spans deep).  [`const_eval`] declines a `Span` because
/// widening the shared folder is not free — teaching it about `Block` woke O8.5's
/// const-unroll in positions that lowering declines (loft#892) — so the span walk lives
/// here, for the callers that hold a raw parse tree rather than a lowered one.
#[must_use]
pub fn const_eval_through_spans(val: &Value, data: &Data) -> Option<Value> {
    const_eval(&strip_spans(val), data)
}

/// `val` with every position marker removed, so the folder sees only the expression.
fn strip_spans(val: &Value) -> Value {
    match val.unspan() {
        Value::Call(op, args) => Value::Call(*op, args.iter().map(strip_spans).collect()),
        Value::If(cond, then_val, else_val) => Value::If(
            Box::new(strip_spans(cond)),
            Box::new(strip_spans(then_val)),
            Box::new(strip_spans(else_val)),
        ),
        other => other.clone(),
    }
}

/// Fold an all-literal text expression to the string it spells, or `None` when some part
/// of it is only known at run time.
///
/// `"x" + "y"` lowers to a block that builds its value in a WORK BUFFER —
/// `{ OpClearText(w); OpAppendText(w, "x"); OpAppendText(w, "y"); w }` — so the value is
/// not in the tree as a literal even when every piece of it is one.  Callers that must
/// end up with a literal (a text constant, which is pasted at each use; a constant-store
/// element, which is written with `set_str`) fold it here.
#[must_use]
pub fn fold_text_block(val: &Value, data: &Data) -> Option<String> {
    let Value::Block(bl) = val.unspan() else {
        return None;
    };
    let clear = data.def_nr("OpClearText");
    let append = data.def_nr("OpAppendText");
    let mut out = String::new();
    for op in &bl.operators {
        match op.unspan() {
            // The buffer reset and the trailing read of it carry no text.
            Value::Call(d, args) if *d == clear && args.len() == 1 => {}
            Value::Var(_) => {}
            Value::Call(d, args) if *d == append && args.len() == 2 => match args[1].unspan() {
                Value::Text(t) => out.push_str(t),
                _ => return None,
            },
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test fold_op directly — constructing full Value::Call trees requires Data.
    #[test]
    fn fold_add_int() {
        assert_eq!(
            fold_op("OpAddInt", &[Value::Int(2), Value::Int(3)]),
            Some(Value::Int(5))
        );
    }

    #[test]
    fn fold_mul_int() {
        assert_eq!(
            fold_op("OpMulInt", &[Value::Int(6), Value::Int(7)]),
            Some(Value::Int(42))
        );
    }

    #[test]
    fn fold_div_by_zero() {
        assert_eq!(fold_op("OpDivInt", &[Value::Int(10), Value::Int(0)]), None);
    }

    #[test]
    fn fold_min_div_minus_one() {
        assert_eq!(
            fold_op("OpDivInt", &[Value::Int(i32::MIN), Value::Int(-1)]),
            None
        );
    }

    #[test]
    fn fold_wrapping_overflow() {
        assert_eq!(
            fold_op("OpAddInt", &[Value::Int(i32::MAX), Value::Int(1)]),
            Some(Value::Int(i32::MIN))
        );
    }

    #[test]
    fn fold_float_nan() {
        if let Some(Value::Float(v)) =
            fold_op("OpDivFloat", &[Value::Float(0.0), Value::Float(0.0)])
        {
            assert!(v.is_nan());
        } else {
            panic!("expected Float");
        }
    }

    #[test]
    fn fold_cast_nonfinite_float_to_int() {
        assert_eq!(
            fold_op("OpConvIntFromFloat", &[Value::Float(f64::INFINITY)]),
            None
        );
        assert_eq!(
            fold_op("OpConvIntFromFloat", &[Value::Float(f64::NAN)]),
            None
        );
    }

    #[test]
    fn fold_negation() {
        assert_eq!(
            fold_op("OpMinSingleInt", &[Value::Int(5)]),
            Some(Value::Int(-5))
        );
        assert_eq!(
            fold_op("OpMinSingleFloat", &[Value::Float(3.125)]),
            Some(Value::Float(-3.125))
        );
    }

    #[test]
    fn fold_unknown_op() {
        assert_eq!(fold_op("OpSqrt", &[Value::Float(4.0)]), None);
    }

    #[test]
    fn fold_nullary_math_consts() {
        // @PLN102 case-C residual — PI/E fold to their runtime values (fill.rs), so a `PI`
        // divisor / fault-op arg is proven non-zero / in-domain.
        assert_eq!(
            fold_op("OpMathPiFloat", &[]),
            Some(Value::Float(std::f64::consts::PI))
        );
        assert_eq!(
            fold_op("OpMathEFloat", &[]),
            Some(Value::Float(std::f64::consts::E))
        );
    }
}
