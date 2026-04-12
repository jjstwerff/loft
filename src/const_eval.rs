// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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
        // --- long arithmetic ---
        ("OpAddLong", [Value::Long(a), Value::Long(b)]) => Some(Value::Long(a.wrapping_add(*b))),
        ("OpMinLong", [Value::Long(a), Value::Long(b)]) => Some(Value::Long(a.wrapping_sub(*b))),
        ("OpMulLong", [Value::Long(a), Value::Long(b)]) => Some(Value::Long(a.wrapping_mul(*b))),
        ("OpDivLong", [Value::Long(a), Value::Long(b)])
            if *b != 0 && !(*a == i64::MIN && *b == -1) =>
        {
            Some(Value::Long(a / b))
        }
        ("OpMinSingleLong", [Value::Long(a)]) => Some(Value::Long(a.wrapping_neg())),
        // --- float arithmetic ---
        ("OpAddFloat", [Value::Float(a), Value::Float(b)]) => Some(Value::Float(a + b)),
        ("OpMinFloat", [Value::Float(a), Value::Float(b)]) => Some(Value::Float(a - b)),
        ("OpMulFloat", [Value::Float(a), Value::Float(b)]) => Some(Value::Float(a * b)),
        ("OpDivFloat", [Value::Float(a), Value::Float(b)]) => Some(Value::Float(a / b)),
        ("OpMinSingleFloat", [Value::Float(a)]) => Some(Value::Float(-a)),
        // --- single arithmetic ---
        ("OpAddSingle", [Value::Single(a), Value::Single(b)]) => Some(Value::Single(a + b)),
        ("OpMinSingle", [Value::Single(a), Value::Single(b)]) => Some(Value::Single(a - b)),
        ("OpMulSingle", [Value::Single(a), Value::Single(b)]) => Some(Value::Single(a * b)),
        ("OpDivSingle", [Value::Single(a), Value::Single(b)]) => Some(Value::Single(a / b)),
        ("OpMinSingleSingle", [Value::Single(a)]) => Some(Value::Single(-a)),
        // --- casts ---
        ("OpConvLongFromInt", [Value::Int(a)]) => Some(Value::Long(i64::from(*a))),
        ("OpConvFloatFromInt", [Value::Int(a)]) => Some(Value::Float(f64::from(*a))),
        ("OpConvIntFromLong", [Value::Long(a)]) => Some(Value::Int(*a as i32)),
        ("OpConvIntFromFloat", [Value::Float(a)]) if a.is_finite() => Some(Value::Int(*a as i32)),
        ("OpConvSingleFromInt", [Value::Int(a)]) => Some(Value::Single(*a as f32)),
        ("OpConvFloatFromLong", [Value::Long(a)]) => Some(Value::Float(*a as f64)),
        // --- boolean ---
        ("OpNot", [Value::Boolean(a)]) => Some(Value::Boolean(!a)),
        _ => None,
    }
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
    fn fold_cast_int_to_long() {
        assert_eq!(
            fold_op("OpConvLongFromInt", &[Value::Int(42)]),
            Some(Value::Long(42))
        );
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
}
