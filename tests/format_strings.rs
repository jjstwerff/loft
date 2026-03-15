// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Tests for typos in format-string specifiers and nested literals inside `{...}`.
//
// Valid specifiers after `:` are:
//   Alignment:    <  ^  >   (left / centre / right)
//   Sign:         +          (always show sign)
//   Prefix flag:  #          (0x / 0b / 0o prefix for numeric radix)
//   Radix:        x X        (hex, case of the a-f digits differs)
//                 b          (binary)
//                 o          (octal)
//                 e          (scientific / exponent)
//                 j / json   (JSON notation for structs/vectors)
//   Zero pad:     0<width>   (fill with zeros, e.g. :03)
//   Width:        <integer>  (minimum field width)
//   Precision:    <float>    (width.precision, e.g. :6.2 for floats)
//
// Common mistakes tested below:
//   - Using a letter that is NOT a recognised radix identifier (bare → consumed as a
//     variable expression → "Unknown variable"; after a width → "Unexpected formatting
//     type").
//   - Uppercase letters for radix modes that require lowercase (B instead of b, O
//     instead of o).
//   - Letters from other languages' format mini-languages (d for decimal, f for float,
//     p for pointer — none of which exist in loft).
//   - Nested integer or float literals inside `{...}` with a bad specifier.
//
// Note: nested *string* literals inside `{...}` are supported (PROBLEMS #9 fixed 2026-03-14).

extern crate loft;

mod testing;

// ---------------------------------------------------------------------------
// Bare unknown identifier after `:` — consumed as a width expression, which
// then fails because the identifier names an unknown variable.
// ---------------------------------------------------------------------------

/// `:z` has no numeric width before it, so the parser tries to evaluate `z`
/// as a width expression.  `z` is not a defined variable.
#[test]
fn bare_specifier_z() {
    code!("fn test() { n = 42; s = \"{n:z}\"; }")
        .error("Unknown variable 'z' at bare_specifier_z:1:31");
}

/// `:d` is now accepted as an explicit decimal specifier (same as no specifier).
/// Developers coming from C/Python/Rust where `d` means "decimal integer" no longer
/// get a confusing "Unknown variable" error.
#[test]
fn bare_specifier_d_accepted() {
    code!("fn test() { n = 42; s = \"{n:d}\"; assert(s == \"42\", s); }");
}

// ---------------------------------------------------------------------------
// Width + unknown radix identifier — the integer width is consumed first, then
// `get_radix()` sees the identifier and emits "Unexpected formatting type".
// ---------------------------------------------------------------------------

/// `:5z` — width 5 is fine, but `z` is not a valid radix letter.
#[test]
fn width_then_unknown_specifier_z() {
    code!("fn test() { n = 42; s = \"{n:5z}\"; }")
        .error("Unexpected formatting type: z at width_then_unknown_specifier_z:1:33");
}

/// `:05z` — zero-pad + width 5, then unknown radix `z`.
#[test]
fn zero_pad_then_unknown_specifier() {
    code!("fn test() { n = 42; s = \"{n:05z}\"; }")
        .error("Unexpected formatting type: z at zero_pad_then_unknown_specifier:1:34");
}

/// `:5B` — binary is lowercase `b`; uppercase `B` is not recognised.
#[test]
fn uppercase_b_instead_of_lowercase_b() {
    code!("fn test() { n = 42; s = \"{n:5B}\"; }")
        .error("Unexpected formatting type: B at uppercase_b_instead_of_lowercase_b:1:33");
}

/// `:5O` — octal is lowercase `o`; uppercase `O` is not recognised.
#[test]
fn uppercase_o_instead_of_lowercase_o() {
    code!("fn test() { n = 42; s = \"{n:5O}\"; }")
        .error("Unexpected formatting type: O at uppercase_o_instead_of_lowercase_o:1:33");
}

/// `:5f` — accepted as fixed-point float format (same as no radix specifier).
/// Developers coming from Rust where `:f` means "Display as fixed-point" no longer
/// get "Unexpected formatting type".
#[test]
fn f_for_float_format_accepted() {
    code!("fn test() { n = 42; s = \"{n:5f}\"; assert(s == \"   42\", s); }");
}

/// `:5d` — accepted as explicit decimal with width 5.
#[test]
fn d_for_decimal_with_width_accepted() {
    code!("fn test() { n = 42; s = \"{n:5d}\"; assert(s == \"   42\", s); }");
}

// ---------------------------------------------------------------------------
// Nested literals — integer or float literals used directly as the format
// expression, combined with a bad specifier.
// ---------------------------------------------------------------------------

/// `{42:5z}` — integer literal as the format expression, unknown radix after width.
#[test]
fn int_literal_bad_specifier() {
    code!("fn test() { s = \"{42:5z}\"; }")
        .error("Unexpected formatting type: z at int_literal_bad_specifier:1:26");
}

/// `{3.14:5z}` — float literal as the format expression, unknown radix after width.
#[test]
fn float_literal_bad_specifier() {
    code!("fn test() { s = \"{3.14:5z}\"; }")
        .error("Unexpected formatting type: z at float_literal_bad_specifier:1:28");
}

/// Float *variable* with unknown radix — same error path, shown for completeness.
#[test]
fn float_var_bad_specifier() {
    code!("fn test() { f = 3.14; s = \"{f:5z}\"; }")
        .error("Unexpected formatting type: z at float_var_bad_specifier:1:35");
}

/// `{f:.2}` — bare precision without an explicit width.
/// `.` consumed by string_states sets float=true; `2` is then the width,
/// which append_data reinterprets as precision (width=0, precision=2).
#[test]
fn bare_precision_float() {
    code!("fn test() { f = 3.14159; s = \"{f:.2}\"; assert(s == \"3.14\", s); }");
}

// ---------------------------------------------------------------------------
// String literal nested inside `{...}` — supported since 2026-03-14 (PROBLEMS #9 fixed).
// The lexer handles `"` and `\"` inside format expressions via `string_nested()`,
// so nested string literals no longer crash or produce "Dual definition of" errors.
// ---------------------------------------------------------------------------

/// `{"hello":5z}` — string literal as the format expression, unknown radix after width.
#[test]
fn string_literal_bad_specifier_after_width() {
    code!(r#"fn test() { s = "{\"hello\":5z}"; }"#)
        .error("Unexpected formatting type: z at string_literal_bad_specifier_after_width:1:33");
}

/// `{"hello":z}` — string literal, bare unknown specifier (z consumed as width variable).
#[test]
fn string_literal_bare_bad_specifier() {
    code!(r#"fn test() { s = "{\"hello\":z}"; }"#)
        .error("Unknown variable 'z' at string_literal_bare_bad_specifier:1:31");
}

/// `{"hello"}` — string literal inside format expression with no specifier — valid.
#[test]
fn string_literal_no_specifier() {
    code!(r#"fn test() { s = "{\"hello\"}"; assert(s == "hello", s); }"#);
}

/// `{"hello":3}` — string literal with width specifier — right-padded.
#[test]
fn string_literal_with_width() {
    code!(r#"fn test() { s = "{\"hello\":3}"; assert(s == "hello", s); }"#);
}

// ---------------------------------------------------------------------------
// Use-before-assignment inside `{...}` — PROBLEMS #10 (fixed 2026-03-15).
// A variable that is assigned AFTER the format string used to panic in the
// byte-code generator ("Incorrect var cd[65535]").  Now a clean compile-time
// diagnostic is emitted and no panic occurs.
// ---------------------------------------------------------------------------

/// Variable used inside `{...}` before its first assignment — must produce a
/// "Unknown variable" diagnostic, not a panic.
#[test]
fn format_string_use_before_assign() {
    code!("fn test() { s = \"{cd}\"; cd = 5; }")
        .error("Unknown variable 'cd' at format_string_use_before_assign:1:22");
}

/// Variable used inside `{...}` AFTER its assignment — must compile and run.
#[test]
fn format_string_use_after_assign() {
    code!("fn test() { cd = 5; s = \"{cd}\"; assert(s == \"5\", s); }");
}

/// Loop counter `{e#count}` inside a format string — must compile and run correctly.
#[test]
fn format_string_loop_count() {
    code!(
        "fn test() {
    v = [10, 20, 30];
    r = \"\";
    for e in v {
        if !e#first { r += \",\" }
        r += \"{e#count}:{e}\"
    }
    assert(r == \"0:10,1:20,2:30\", r)
}"
    );
}
