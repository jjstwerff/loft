// Copyright (c) 2023-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

extern crate loft;

mod testing;

use loft::data::Value;

#[test]
fn expr_integer() {
    expr!("a = 1; sizeof(1+2+3) + sizeof(integer) + 10 * sizeof(a)").result(Value::Int(48));
}

#[test]
fn expr_float() {
    expr!("a = 1.1; sizeof(float) + 10 * sizeof(a)").result(Value::Int(88));
}

#[test]
fn expr_enum() {
    code!("enum En {V1, V2, V3}")
        .expr("sizeof(En) + 10 * sizeof(V1)")
        .result(Value::Int(11));
}

#[test]
fn sizeof_enum_structs() {
    code!(
        "enum Val {
    Small { n: u8 },
    Large { n: long }
}
fn get_size(v: Val) -> integer { sizeof(v) }"
    )
    .expr("if get_size(Small { n: 1 }) == get_size(Large { n: 42l }) { 1 } else { 0 }")
    .result(Value::Int(0)); // 0 = sizes differ (correct). With P12 bug: 1 (both same base size).
}

#[test]
fn sizeof_packed_integer_types() {
    // u8 = integer limit(0,255) size(1) → packed size 1
    // u16 = integer limit(0,65535) size(2) → packed size 2
    // i32 = integer → packed size 4
    expr!("sizeof(u8) + 10 * sizeof(u16) + 100 * sizeof(i32)").result(Value::Int(421));
}

#[test]
fn expr_struct() {
    code!(
        "struct S {a: integer, b: long, c: En}
enum En {V1, V2}"
    )
    .expr("sizeof(S)")
    .result(Value::Int(13));
}

#[test]
fn hash_member() {
    code!(
        "struct S {a: integer, b: long, c: integer}
struct Main { s:hash<S[b]> }"
    )
    .expr("sizeof(S) + 100 * sizeof(Main)")
    .result(Value::Int(416));
}

#[test]
fn index_member() {
    // Structure S will be a RB tree member so it is +9 size.
    // So it gains a left: reference, right: reference and black: boolean field.
    code!(
        "struct S {a: integer, b: long, c: integer};
struct Main { s: index<S[a, -c]> };"
    )
    .expr("m = Main {}; sizeof(S) + 100 * sizeof(m)")
    .result(Value::Int(425));
}

#[test]
fn reference_field() {
    // S is now a stand-alone object.
    // The vector holds references to S, the same as biggest.
    code!(
        "struct S {a: integer, b: integer, c:integer};
struct Main { s: vector<S>, biggest: reference<S> };"
    )
    .expr("sizeof(S) + 100 * sizeof(Main) + 10000 * sizeof(vector<S>)")
    .result(Value::Int(121612));
}

/*
TODO needs initialisation of fields instead of Main.biggest as reference.
#[test]
fn copy_field() {
    // S is now an Inner object that is the exact size of its fields.
    // biggest is an inner object that increases the size of Main.
    code!(
        "struct S {a: integer, b: integer, c:integer};
struct Main { s: vector<S>, biggest: S };"
    )
    .expr(
        "m = Main{};
sizeof(S) + 100 * sizeof(Main) + 10000 * sizeof(m) + 100000 * sizeof(vector<S>)",
    )
    .result(Value::Int(1242012));
}
*/

#[test]
fn vector_size() {
    // S is now an Inner object that is the exact size of its fields.
    // biggest is an inner object that increases the size of Main.
    code!(
        "struct S {a: integer, b: integer, c:integer};
struct Main { s: vector<S> };"
    )
    .expr(
        "m = Main{};
sizeof(S) + 100 * sizeof(Main) + 10000 * sizeof(m) + 100000 * sizeof(vector<S>)",
    )
    .result(Value::Int(1240412));
}
