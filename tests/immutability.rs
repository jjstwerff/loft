// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Tests for immutability checks:
//! - `&` parameter that is never modified → error
//! - `&` parameter that IS modified → ok
//! - `const` parameter that is modified → error
//! - `const` parameter that is only read → ok
//! - Transitive mutation via a called function counts as modified → ok (no error)

extern crate loft;

mod testing;

/// A `&` parameter that is actually modified: no compile error.
#[test]
fn ref_param_is_modified() {
    code!(
        "fn increment(a: &integer) { a += 1 }
fn test() { x = 5; increment(x); assert(x == 6, \"x={x}\") }"
    )
    .result(loft::data::Value::Null);
}

/// A `&` parameter that is never modified in the body: error.
#[test]
fn ref_param_never_modified() {
    code!(
        "fn read_only(a: &integer) -> integer { a }
fn test() {}"
    )
    .error(
        "Parameter 'a' has & but is never modified; remove the & at ref_param_never_modified:1:39",
    );
}

/// A `const` parameter that is only read: no error.
#[test]
fn const_param_read_only() {
    code!(
        "fn double(a: const integer) -> integer { a * 2 }
fn test() { assert(double(5) == 10, \"double\") }"
    )
    .result(loft::data::Value::Null);
}

/// A `const` parameter that is mutated: error.
/// A warning is also emitted because `a` is never READ (only assigned).
#[test]
fn const_param_mutated() {
    code!(
        "fn bad(a: const integer) { a = 42 }
fn test() {}"
    )
    .error("Cannot modify const parameter 'a'; remove 'const' or use a local copy at const_param_mutated:1:36")
    .warning("Parameter a is never read at const_param_mutated:1:27");
}

/// A `& const` parameter (const reference): mutation is an error.
#[test]
fn const_ref_param_mutated() {
    code!(
        "fn bad(a: & const integer) { a = 42 }
fn test() {}"
    )
    .error("Cannot modify const parameter 'a'; remove 'const' or use a local copy at const_ref_param_mutated:1:38");
}

/// A `&` parameter mutated through a called function is detected as modified (no error).
#[test]
fn ref_param_mutated_via_call() {
    code!(
        "fn add_one(v: &integer) { v += 1 }
fn wrapper(a: &integer) { add_one(a) }
fn test() {}"
    );
}

// ── Local `const` variable tests ────────────────────────────────────────────

/// A `const` local integer that is only read: no compile error.
#[test]
fn const_local_read_only() {
    code!("fn test() { const x = 42; assert(x == 42, \"ok\") }").result(loft::data::Value::Null);
}

/// A `const` local integer that is reassigned: compile error.
#[test]
fn const_local_int_reassigned() {
    code!("fn test() { const x = 5; x = 10 }")
        .error("Cannot modify const variable 'x'; remove 'const' or use a local copy at const_local_int_reassigned:1:34")
        .warning("Variable x is never read at const_local_int_reassigned:1:22");
}

/// A `const` local integer with `+=`: compile error.
#[test]
fn const_local_int_augmented() {
    code!("fn test() { const x = 5; x += 1 }")
        .error("Cannot modify const variable 'x'; remove 'const' or use a local copy at const_local_int_augmented:1:34")
        .warning("Variable x is never read at const_local_int_augmented:1:22");
}

/// A `const` local text that is reassigned: compile error.
#[test]
fn const_local_text_reassigned() {
    code!("fn test() { const t = \"hello\"; t = \"world\" }")
        .error("Cannot modify const variable 't'; remove 'const' or use a local copy at const_local_text_reassigned:1:45");
}

/// A `const` local text with `+=`: compile error.
#[test]
fn const_local_text_appended() {
    code!("fn test() { const t = \"hello\"; t += \"!\" }")
        .error("Cannot modify const variable 't'; remove 'const' or use a local copy at const_local_text_appended:1:42");
}

/// A `const` local vector that is read without modification: no error.
#[test]
fn const_local_vector_read_only() {
    code!(
        "struct P { x: integer }
fn test() { const v = [P { x: 1 }, P { x: 2 }]; assert(v[0].x == 1, \"ok\") }"
    )
    .result(loft::data::Value::Null);
}

/// A `const` local vector that is appended to: compile error.
#[test]
fn const_local_vector_appended() {
    code!(
        "struct P { x: integer }
fn test() { const v = [P { x: 1 }]; v += [P { x: 2 }] }"
    )
    .error("Cannot modify const variable 'v'; remove 'const' or use a local copy at const_local_vector_appended:2:56");
}

/// A `const` local reference that is read without modification: no error.
#[test]
fn const_local_ref_read_only() {
    code!(
        "struct Counter { value: integer }
fn test() { const c = Counter { value: 7 }; assert(c.value == 7, \"ok\") }"
    )
    .result(loft::data::Value::Null);
}

/// A `const` local reference that is directly reassigned: compile error.
#[test]
fn const_local_ref_reassigned() {
    code!(
        "struct Counter { value: integer }
fn test() { const c = Counter { value: 7 }; c = Counter { value: 9 } }"
    )
    .error("Cannot modify const variable 'c'; remove 'const' or use a local copy at const_local_ref_reassigned:2:71");
}
