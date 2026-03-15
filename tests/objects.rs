// Copyright (c) 2022-2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Testing framework
extern crate loft;

mod testing;

use loft::data::Value;
#[test]
fn print_object() {
    code!(
        "struct Object{a: integer, bb: text, ccc: boolean}
fn obj() -> Object { Object {a: 12, bb: \"hi\", ccc: false } }"
    )
    .expr("o = obj(); o.bb += '!'; \"{o} pretty {o:#}\"")
    .result(Value::str(
        "{a:12,bb:\"hi!\",ccc:false} pretty { a: 12, bb: \"hi!\", ccc: false }",
    ));
}

#[test]
fn special_fields() {
    code!(
        "enum Gender { Male, Female, Fluid }
struct Object{a: vector<integer>, b: Gender}
fn sum(o: Object) -> integer {
  r = 0;
  for v in o.a { r += v; };
  r
}"
    )
    .expr("  o = Object {a: [1,4,3], b: Fluid};\n  o.a += [sum(o)];\n  \"{o}\"")
    .result(Value::str("{a:[1,4,3,8],b:Fluid}"));
}

#[test]
fn duplicate() {
    code!(
        "struct Point {
   r: integer,
   g: integer,
   r: integer
}"
    )
    .error("field `r` is already declared at duplicate:4:6");
}

#[test]
fn colours() {
    code!(
        "struct Point {
  r: integer limit(0, 255) not null,
  g: integer limit(0, 255) not null,
  b: integer limit(0, 255) not null
}

fn value(self: Point) -> integer {
  self.r * 0x10000 + self.g * 0x100 + self.b
}"
    )
    .expr(
        "  points = [ Point { r:128, b:128 }, Point { b:255 } ];
\"size:{sizeof(Point)} purple:{points[0]} value:{points[0].value():x} blue:{points[1]}\"",
    )
    .result(Value::str(
        "size:3 purple:{r:128,g:0,b:128} value:800080 blue:{r:0,g:0,b:255}",
    ));
}

#[test]
fn restrictions() {
    // was: val: integer limit(1, 256) check(val > byte),
    // this could not be properly parsed due to context switches for variables
    code!(
        "struct Data {
  byte: integer limit(0, 255) not null,
  val: integer limit(1, 256),
  signed: integer limit(-127, 127) default(1)
}

fn calc(self: Data) -> integer {
  self.val * 65536 + self.byte * 256 + self.signed
}
"
    )
    .expr("1")
    .result(Value::Int(1));
}

#[test]
fn mutable_reference() {
    code!(
        "struct Data {
  num: integer,
  values: vector<integer>
}

fn add(r: &Data = null, val: integer) {
    if !r {
       r = Data { num: 0 };
    }
    r.num += val;
    r.values += [val];
}"
    )
    .expr("v = Data { num: 1 }; add(v, 2); add(v, 3); \"{v}\"")
    .result(Value::str("{num:6,values:[2,3]}"));
}

#[test]
fn mutable_vector() {
    code!(
        "fn add(r: &vector<integer> = [], val: integer) {
    r += [val];
}"
    )
    .expr("v = [1]; add(v, 2); add(v, 3); \"{v}\"")
    .result(Value::str("[1,2,3]"));
}

#[test]
fn vector_argument() {
    code!(
        "fn sum(r: vector<integer>) -> integer {
  res = 0;
  for v in r {
    res += v;
  }
  res
}
    "
    )
    .expr("sum([1,2,3,4,5]) + 100 * sum([1,2,3] + [4,5])")
    .result(Value::Int(1515));
}

#[test]
fn return_text() {
    code!(
        "pub struct Data {
    name: text,
    number: integer
}

fn data(n: text) -> text {
    res = Data { name: n };
    res.name
}"
    )
    .expr("data(\"test\")")
    .result(Value::str("test"));
}

#[test]
fn assign_text() {
    code!("struct Object{a: text}")
        .expr(
            "o = Object {a: \"a\"};
        o.a = \"b\";
        o.a += \"c\";
        o.a += \"d\" + \"e\";
        o.a = \"{o.a}f\";
        \"{o}\"",
        )
        .result(Value::str("{a:\"bcdef\"}"));
}

#[test]
fn independent_strings() {
    code!("struct T { name: text }")
        .expr(
            "a = T { name: \"hello\" };
b = a;
b.name += \" world\";
a.name",
        )
        .result(Value::str("hello"));
}

#[test]
fn method_on_constructor() {
    // Bug #8: calling a method directly on a struct constructor expression.
    // Previously gave "MyStruct should be MyStruct on call to method" because
    // parse_object returned Type::Rewritten / Value::Insert instead of a proper Block.
    code!(
        "struct Pt { x: integer, y: integer }
fn dist2(self: Pt) -> integer { self.x * self.x + self.y * self.y }"
    )
    .expr("Pt { x: 3, y: 4 }.dist2()")
    .result(Value::Int(25));
}
