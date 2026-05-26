// Copyright (c) 2021-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Tests that require Rust-level slot inspection (.slots()).
// All other string tests have moved to tests/scripts/31-strings.loft.

extern crate loft;

mod testing;

use loft::data::Value;

/// Byte-indexed `text[i]` returns the character that contains the
/// byte at position `i`.  `"♥😃"` has 7 bytes (3 for ♥, 4 for 😃),
/// so `a[0..2]` returns `♥` (each byte of ♥) and `a[3..6]` returns
/// `😃` (each byte of 😃).  Out-of-range indices (`a[7]` and up) are a
/// RECOVERABLE fault (@P356): they return the null char and execution
/// continues; the opt-in `LOFT_DEV_SOFT_HALT` mode surfaces the
/// `IndexOutOfBounds` for debugging — covered in
/// `tests/runtime_errors.rs::kind_index_out_of_bounds_text_returns_null_and_continues`.
#[test]
fn utf8_index() {
    expr!("a=\"♥😃\"; a[0] + a[1] + a[2] + a[3] + a[4] + a[5] + a[6]")
        .result(Value::str("♥♥♥😃😃😃😃"));
}

#[test]
fn string_scope() {
    expr!(
        "
  a=1;
  b=\"\";
  for n in 1..4 {
    t=\"1\";
    b+=\"n\" + \":{n}\" + \"=\";
    for _m in 1..n {
      t+=\"2\";
    };
    b += t+\" \";
    a += t as integer
  };
  \"{a} via {b}\"
"
    )
    // Slot expectation refreshed after P223 (work-text wrap on
    // self-referencing text assignments) added 2 extra `__work_*`
    // slots — the fix wraps `b += rhs-with-self-ref` and `t += "2"`
    // shapes in protective work-buffers so the interpreter's
    // clear-before-evaluate text-Set semantics don't destroy the
    // accumulator.  See `parser/expressions.rs:1273-1295`.
    .slots(
        "\
  block:1
  __work_5+24=4 [0..145]
  __work_4+24=28 [3..144]
  __work_3+24=52 [6..143]
  __work_2+24=76 [9..142]
  __work_1+24=100 [12..141]
  test_value+24=124 [15..140]
  │ block:2
  │ a+8=148 [17..102]
  │ b+24=156 [18..119]
  │ │ for:3
  │ │ n#index+8=180 [22..98]
  │ │ │ loop:4L [seq 23..99]
  │ │ │ n+8=188 [35..81]
  │ │ │ │ block:6
  │ │ │ │ t+24=196 [36..98]
  │ │ │ │ │ for:9
  │ │ │ │ │ _m#index+8=220 [65..81]
  │ │ │ │ │ │ loop:10L [seq 66..82]
  │ │ │ │ │ │ _m+8=228 [78..78]",
    )
    .result(Value::str("136 via n:1=1 n:2=12 n:3=122 "));
}

#[test]
fn loop_variable() {
    expr!("a = 0; for _t in 1..5 { b = \"123\"; a += b as integer; if a > 200 { break; }}; a")
        .slots(
            "\
  block:1
  test_value+8=4 [34..41]
  __work_1+24=12 [0..56]
  │ block:2
  │ a+8=36 [4..33]
  │ │ for:3
  │ │ _t#index+8=44 [6..32]
  │ │ │ loop:4L [seq 7..33]
  │ │ │ _t+8=52 [19..19]
  │ │ │ │ block:6
  │ │ │ │ b+24=60 [20..32]",
        )
        .result(Value::Int(246));
}
