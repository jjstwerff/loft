// Copyright (c) 2021-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

extern crate loft;

mod testing;

use loft::data::Value;

// str_index: covered by 03-text.loft (char indexing at byte offset)

#[test]
fn utf8_index() {
    expr!("a=\"♥😃\"; a[0] + a[1] + a[2] + a[3] + a[4] + a[5] + a[6] + \".\" + a[7]")
        .result(Value::str("♥♥♥😃😃😃😃."));
}

#[test]
fn string_fn() {
    code!(
        "fn to_text() -> text {
    res = \"aa \";
    for _i in 0..2 {
        res += \"b\";
    }
    res + \" cc\"
}"
    )
    .expr("\"1{to_text()}2\"")
    .result(Value::str("1aa bb cc2"));
}

#[test]
fn var_ref() {
    code!(
        "fn text_ref() -> text {
    a = \"12345\";
    a[0..4]
}"
    )
    .expr("text_ref()")
    .result(Value::str("1234"));
}

#[test]
fn return_ref() {
    code!(
        "fn return_ref() -> text {
    a = \"12345\";
    return a[0..4];
}"
    )
    .expr("return_ref()")
    .result(Value::str("1234"));
}

#[test]
fn string_scope() {
    expr!(
        "
  a=1l;
  b=\"\";
  for n in 1..4 {
    t=\"1\";
    b+=\"n\" + \":{n}\" + \"=\";
    for _m in 1..n {
      t+=\"2\";
    };
    b += t+\" \";
    a += t as long
  };
  \"{a} via {b}\"
"
    )
    .slots(
        "\
  block:1
  __work_3+24=4 [0..126]
  __work_2+24=28 [3..125]
  __work_1+24=52 [6..124]
  test_value+24=76 [9..123]
  │ block:2
  │ a+8=100 [11..86]
  │ b+24=108 [12..102]
  │ │ for:3
  │ │ n#index+4=132 [16..82]
  │ │ │ loop:4L [seq 17..83]
  │ │ │ n+4=136 [29..70]
  │ │ │ │ block:6
  │ │ │ │ t+24=140 [30..82]
  │ │ │ │ │ for:8
  │ │ │ │ │ _m#index+4=164 [54..70]
  │ │ │ │ │ │ loop:9L [seq 55..71]
  │ │ │ │ │ │ _m+4=168 [67..67]",
    )
    .result(Value::str("136 via n:1=1 n:2=12 n:3=122 "));
}

#[test]
fn reference() {
    code!(
        "fn add(a: &text, b: text=\" world!\") {
    a += b;
}"
    )
    .expr("v = \"Hello\"; add(v); v")
    .result(Value::str("Hello world!"));
}

#[test]
fn default_ref() {
    code!(
        "fn add(a: text, b: &text=\"var\") -> text {
    b += \"_\" + a;
    b
}"
    )
    .expr("add(\"1234\")")
    .result(Value::str("var_1234"));
}

#[test]
fn call() {
    code!("fn choice(a: text, b: text) -> text { if len(a) > len(b) { a } else { b } }")
        .expr("choice(\"{1:03}\", \"{2}1\") + choice(\"2\", \"\")")
        .result(Value::str("0012"));
}

#[test]
fn work_loop() {
    expr!("a = 0; for t in 1..4 { a += \"0{t}0\" as integer }; a").result(Value::Int(60));
}

#[test]
fn loop_variable() {
    expr!("a = 0; for _t in 1..5 { b = \"123\"; a += b as integer; if a > 200 { break; }}; a")
        .slots(
            "\
  block:1
  test_value+4=4 [34..41]
  __work_1+24=8 [0..56]
  │ block:2
  │ a+4=32 [4..33]
  │ │ for:3
  │ │ _t#index+4=36 [6..32]
  │ │ │ loop:4L [seq 7..33]
  │ │ │ _t+4=40 [19..19]
  │ │ │ │ block:6
  │ │ │ │ b+24=44 [20..32]",
        )
        .result(Value::Int(246));
}

#[test]
fn return_clear() {
    code!("fn res() -> integer { a = 0; for _t in 1..5 { b = \"123\"; a += b as integer; if a > 200 { return a; }}; 0}").expr("res()").result(Value::Int(246));
}

#[test]
fn optional_remove() {
    code!(
        "fn last(filename: text) -> text {
    v = filename.rfind(\"/\");
    if v {
        filename[v + 1..]
    } else {
        filename
    }
}"
    )
    .expr("last(\"a/b/c\") + last(\"d\")")
    .result(Value::str("cd"));
}

#[test]
fn strange_if() {
    code!(
        "
fn build() -> text {
    t = \"abcde\";
    if t.len() > 3 {
        t
    } else {
        \"\"
    }
}"
    )
    .expr("build()")
    .result(Value::str("abcde"));
}

#[test]
fn string_parse() {
    code!(
        "
fn parse(s: text) -> integer {
    for t in 0..300 {
        l = s[t];
        if !l.is_alphanumeric() and l != '_' {
            return t;
        }
    }
    return 0;
}"
    )
    .expr("parse(\"if_cond \")")
    .result(Value::Int(7));
}

// Only run this test locally, do not make it part of the release as it will log all kinds of
// data that is not for public consumption and not stable through multiple runs.
/*
#[test]
fn dirs() {
    code!(
        "fn test() {
  println(\"program {program_directory()}\");
  println(\"user {user_directory()}\");
  println(\"current {directory()}\");
  for v in env_variables() { println(\"{v}\"); }
  for a in arguments() { println(\"{a}\"); }
}"
    );
}
*/
