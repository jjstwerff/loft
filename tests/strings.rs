// Copyright (c) 2021-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Tests that require Rust-level slot inspection (.slots()).
// All other string tests have moved to tests/scripts/31-strings.loft.

extern crate loft;

mod testing;

use loft::data::Value;

#[test]
fn utf8_index() {
    expr!("a=\"♥😃\"; a[0] + a[1] + a[2] + a[3] + a[4] + a[5] + a[6] + \".\" + a[7]")
        .result(Value::str("♥♥♥😃😃😃😃."));
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
    .slots(
        "\
  block:1
  __work_3+24=4 [0..127]
  __work_2+24=28 [3..126]
  __work_1+24=52 [6..125]
  test_value+24=76 [9..124]
  │ block:2
  │ a+8=100 [11..86]
  │ b+24=108 [12..103]
  │ │ for:3
  │ │ n#index+8=132 [16..82]
  │ │ │ loop:4L [seq 17..83]
  │ │ │ n+8=140 [29..70]
  │ │ │ │ block:6
  │ │ │ │ t+24=148 [30..82]
  │ │ │ │ │ for:8
  │ │ │ │ │ _m#index+8=172 [54..70]
  │ │ │ │ │ │ loop:9L [seq 55..71]
  │ │ │ │ │ │ _m+8=180 [67..67]",
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
