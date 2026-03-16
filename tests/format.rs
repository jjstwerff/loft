// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Integration tests for T2-0: loft code formatter.

extern crate loft;

use loft::formatter;

/// Formatting an already-formatted file produces identical output (idempotent).
#[test]
#[ignore = "T2-0: not yet implemented"]
fn roundtrip_comments() {
    let input = include_str!("format/comments.loft");
    let result = formatter::format_source(input);
    assert_eq!(result, input, "comments.loft should be idempotent");
}

/// Formatting an already-formatted struct definition is idempotent.
#[test]
#[ignore = "T2-0: not yet implemented"]
fn roundtrip_struct_def() {
    let input = include_str!("format/struct_def.loft");
    let result = formatter::format_source(input);
    assert_eq!(result, input, "struct_def.loft should be idempotent");
}

/// A messy input file is normalised to the golden output.
#[test]
#[ignore = "T2-0: not yet implemented"]
fn normalize_messy() {
    let input = include_str!("format/messy.loft");
    let expected = include_str!("format/messy.loft.fmt");
    let result = formatter::format_source(input);
    assert_eq!(result, expected, "messy.loft normalisation mismatch");
}

/// check_source returns true for already-formatted source.
#[test]
#[ignore = "T2-0: not yet implemented"]
fn format_check_already_formatted() {
    let input = include_str!("format/comments.loft");
    assert!(
        formatter::check_source(input),
        "already-formatted source should pass check"
    );
}

/// check_source returns false for source that needs formatting.
#[test]
#[ignore = "T2-0: not yet implemented"]
fn format_check_needs_formatting() {
    let input = include_str!("format/messy.loft");
    assert!(
        !formatter::check_source(input),
        "messy source should fail check"
    );
}

// ─── Edge-case regression tests ──────────────────────────────────────────────

/// Unary minus and plus do not get a space after them (`-5`, not `- 5`),
/// but do get a space before them when context requires it (`a = -5`, not `a =-5`).
#[test]
#[ignore = "T2-0: not yet implemented"]
fn roundtrip_unary_minus() {
    let input = include_str!("format/unary_minus.loft");
    let result = formatter::format_source(input);
    assert_eq!(result, input, "unary_minus.loft should be idempotent");
}

/// Range operators `..` and `..=` are kept together without surrounding spaces.
#[test]
#[ignore = "T2-0: not yet implemented"]
fn roundtrip_range_ops() {
    let input = include_str!("format/range_ops.loft");
    let result = formatter::format_source(input);
    assert_eq!(result, input, "range_ops.loft should be idempotent");
}

/// Binary literals `0b...` are kept as a single token (not split at `b`).
#[test]
#[ignore = "T2-0: not yet implemented"]
fn roundtrip_binary_literals() {
    let input = include_str!("format/binary_literals.loft");
    let result = formatter::format_source(input);
    assert_eq!(result, input, "binary_literals.loft should be idempotent");
}

/// `if`, `for`, and `while` blocks open a Block context, not a StructLit.
/// Body statements are properly indented at depth+1.
#[test]
#[ignore = "T2-0: not yet implemented"]
fn roundtrip_if_for_blocks() {
    let input = include_str!("format/if_for_blocks.loft");
    let result = formatter::format_source(input);
    assert_eq!(result, input, "if_for_blocks.loft should be idempotent");
}

/// Two consecutive identifier/keyword tokens always get a space between them
/// (e.g. `boolean size(1)`, not `booleansize(1)`).
#[test]
#[ignore = "T2-0: not yet implemented"]
fn roundtrip_adjacent_words() {
    let input = include_str!("format/adjacent_words.loft");
    let result = formatter::format_source(input);
    assert_eq!(result, input, "adjacent_words.loft should be idempotent");
}

/// `else` on a separate line after `}` is pulled onto the same line: `} else {`.
#[test]
#[ignore = "T2-0: not yet implemented"]
fn normalize_else_same_line() {
    let input = include_str!("format/else_same_line.loft");
    let expected = include_str!("format/else_same_line.loft.fmt");
    let result = formatter::format_source(input);
    assert_eq!(result, expected, "else_same_line normalisation mismatch");
}
