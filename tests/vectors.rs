// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

extern crate loft;

mod testing;

use loft::data::Value;

// vectors, iter_vector: covered by 09-vectors.loft
// enum_vector: covered by 08-enums.loft

#[test]
fn append_vector() {
    code!("enum Val {A, B, C}")
        .expr("v=[A,A]; v += [B] + [C] + [A]; \"{v}\"")
        .result(Value::str("[A,A,B,C,A]"));
}

#[test]
fn format_vector() {
    expr!(
        // filtered iterators:  {v[$ < 3]}
        "v=[1, 2, 4, 8];
v += [16];
\"{v} {v.len()} {v[2]} {v[1..3]} {v[rev(1..=3)]} {for x in v if x >= 4 {x/2}}\""
    )
    .result(Value::str("[1,2,4,8,16] 5 4 [2,4] [8,4,2] [2,4,8]"));
}

#[test]
fn parse_vector() {
    expr!("a = \"[ 1.2, -10.3, 1.812e4, 1.001e-8 ]\" as vector<float>; a[2] + a[3]")
        .result(Value::Float(1.812e4 + 1.001e-8));
}

/*
#[test]
fn map_vector() {
    expr!("v=[0,1,2,3,4,5,6,7,8,9];
w=[for x in v if x < 4 {x * 3}];
\"{w}\"").result(Value::str("[0, 3, 6, 9]"));
}
*/
#[test]
fn loop_variables() {
    expr!("\"{for x in 0..10 if x != 0 && x % 3 == 0 {if x#first { x } else {x * 2}}}\"")
        .result(Value::str("[3,12,18]"));
}
/*
#[test]
fn store_iterator() {
    expr!("v=[1, 2, 3]; v += v[1..2]; v += [for x in v {x*2}]; \"{v}\"")
        .result(Value::Text("[1, 2, 3, 2, 3, 2, 4, 6, 4, 6]".to_string()));
}
*/
#[test]
fn format_object() {
    code!("struct Elm {a:integer, b:integer}")
        .expr(
            "v=[
    Elm{a:1, b:2},
    Elm{a:12, b:13},
    Elm{a:4, b:5}
];
v[2].b=6;
\"{v} sizeof {sizeof(Elm)}\"",
        )
        .result(Value::str("[{a:1,b:2},{a:12,b:13},{a:4,b:6}] sizeof 8"));
}

#[test]
fn parse_objects() {
    code!("struct Elm {n:text, c:integer}")
        .expr("v = \"[ {{n:'hi', c:10 }}, {{n:'world', c:2 }} ]\" as vector<Elm>; \"{v}\"")
        .result(Value::str("[{n:\"hi\",c:10},{n:\"world\",c:2}]"));
}

#[test]
fn sum_vector() {
    code!("fn sum(v: vector<integer>) -> integer { t = 0; for i in v { t += i }; t}")
        .expr("sum([1, 2, 3, 4, 5])")
        .result(Value::Int(15));
}

#[test]
fn growing_vector() {
    expr!(
        "a = [];
for v in 1..400 { a += [ v * 10 ] };
sum = 0;
for elm in a { sum += elm }
\"{sum}\""
    )
    .result(Value::str("798000"));
}

// sorted_vector: covered by 10-collections.loft

#[test]
fn sorted_iterator() {
    code!(
        "struct Elm {nr: integer, key: text, value: integer}
struct Db {map: sorted<Elm[-nr,key]>}"
    )
    .expr(
        "db=Db {map: [
  Elm {nr: 101, key: \"One\", value: 1},
  Elm {nr: 92, key: \"Two\", value: 2},
  Elm {nr: 83, key: \"Three\", value: 3},
  Elm {nr: 83, key: \"Four\", value: 4},
  Elm {nr: 83, key: \"Five\", value: 5},
  Elm {nr: 63, key: \"Six\", value: 6},
  Elm {nr: 61, key: \"Seven\", value: 7},
]};
sum = 0;
assert(db.map[83,\"Five\"].value == 5, \"Incorrect element {db.map[83,\"Five\"].value}\");
for v in db.map[84..=63,\"Six\"] {
  sum = sum * 10 + v.value;
};
sum",
    )
    .result(Value::Int(5436));
}

#[test]
fn fill_result() {
    code!(
        "pub fn fill() -> vector<text> {
    result = [];
    result += [\"aa\"];
    result += [\"bb\"];
    result
}"
    )
    .expr("t = fill(); \"{t}\"")
    .result(Value::str("[\"aa\",\"bb\"]"));
}

#[test]
fn combination_hash() {
    code!(
        "struct Count { t: text, v: integer};
struct Counting { v: vector<Count>, h: hash<Count[t]> };
fn fill(c: Counting) {
  c.v = [
    {t:\"One\", v:1},
    {t:\"Two\", v:2},
  ];
  c.v += [
    {t:\"Three\", v:3},
    {t:\"Four\", v:4},
    {t:\"Five\", v:5},
    {t:\"Six\", v:6},
    {t:\"Seven\", v:7},
    {t:\"Eight\", v:8},
    {t:\"Nine\", v:9},
    {t:\"Ten\", v:10},
    {t:\"Eleven\", v:11},
    {t:\"Twelve\", v:12},
    {t:\"Thirteen\", v:13}
  ];
}"
    )
    .expr(
        "c = Counting {};
  fill(c);
  assert(!c.h[\"None\"], \"No element\");
  add = 0;
  for v in c.v {
    add += v.v;
  }
  assert(add == 91, \"Incorrect sum\");
  c.h[\"Five\"].v + c.h[\"Seven\"].v",
    )
    .result(Value::Int(12));
}

// hash: covered by 10-collections.loft

#[test]
fn multi_hash() {
    code!(
        "enum Cat { A, B, C };
struct Count { c: Cat, t: text, v: integer};
struct Counting { v: sorted<Count[t,v]>, h: hash<Count[c,t]> };
fn fill(c: Counting) {
  c.v = [
    {c:A, t:\"One\", v:1},
    {c:B, t:\"Two\", v:2},
    {c:C, t:\"Two\", v:20},
    {c:A, t:\"Three\", v:3},
    {c:C, t:\"Four\", v:4}
  ]
}"
    )
    .expr("c = Counting {}; fill(c); c.h[A,\"Three\"].v + c.h[C,\"Two\"].v + c.v[\"Four\",4].v")
    .result(Value::Int(27));
}

#[test]
fn index_iterator() {
    code!(
        "struct Elm {nr: integer, key: text, value: integer}
struct Db {map: index<Elm[nr,-key]>}"
    )
    .expr(
        "db=Db {map: [
  Elm {nr: 101, key: \"One\", value: 1},
  Elm {nr: 92, key: \"Two\", value: 2},
  Elm {nr: 83, key: \"Three\", value: 3},
  Elm {nr: 83, key: \"Four\", value: 4},
  Elm {nr: 83, key: \"Five\", value: 5},
  Elm {nr: 63, key: \"Six\", value: 6},
]};
assert(db.map[101,\"One\"].value == 1 , \"Missing element\");
sum = 0;
for v in db.map[83..92,\"Two\"] {
  sum = sum * 10 + v.value;
};
total = 0;
for r in db.map {
    total += r.value;
}
assert(total == 21, \"Incorrect total {total}\");
assert(!db.map[12,\"\"], \"No element\");
assert(!db.map[83,\"One\"], \"No element\");
sum",
    )
    .result(Value::Int(345));
}

#[test]
fn fill_fn() {
    code!(
        "pub struct Data {
    name: character,
    number: integer
}

fn data(n: text) -> vector<Data> {
    res = [];
    nr = 0;
    for ch in n {
        res += [Data {name: ch, number: nr}];
        nr += 1;
    }
    res
}"
    )
    .expr("d = data(\"test\"); \"{d}\"")
    .result(Value::str(
        "[{name:'t',number:0},{name:'e',number:1},{name:'s',number:2},{name:'t',number:3}]",
    ));
}

#[test]
fn get_object_value() {
    code!(
        "struct T { n: text, v: u16 }
    struct N { d: vector<T>, h: hash<T[n]> }"
    )
    .expr("s = N { d:[T {n: \"a\", v:12} ] }; \"{s.d[0]} v={s.d[0].v}\"")
    .result(Value::str("{n:\"a\",v:12} v=12"));
}

#[test]
fn assign_text() {
    code!(
        "struct T { n: text, v: u16 }
    struct N { d: vector<T>, h: hash<T[v]> }"
    )
    .expr(
        "
s = N { d:[T {n: \"a\", v:12} ] };
s.d[0].n = \"bb\";
\"{s.d[0]} v={s.d[0].v}\"",
    )
    .result(Value::str("{n:\"bb\",v:12} v=12"));
}

#[test]
fn for_comprehension() {
    // Bug #16: [for n in range { expr }] vector comprehension
    expr!("v = [for n in 1..7 { n * 2 }]; \"{v}\"").result(Value::str("[2,4,6,8,10,12]"));
}

#[test]
fn for_comprehension_if() {
    // for comprehension with filter
    expr!("v = [for n in 1..10 if n % 2 == 0 { n }]; \"{v}\"").result(Value::str("[2,4,6,8]"));
}

// sorted_first_count: covered by 10-collections.loft (loop attrs on sorted)

#[test]
fn sorted_reverse_iterator() {
    // rev(sorted_collection) iterates elements in reverse key order
    code!(
        "struct Elm { key: integer, val: integer }
struct Db { s: sorted<Elm[key]> }
fn test() {
    db = Db { s: [Elm{key:1,val:10}, Elm{key:2,val:20}, Elm{key:3,val:30}] };
    // Forward: 1,2,3 → digits 1,2,3
    fwd = 0;
    for e in db.s { fwd = fwd * 10 + e.key }
    assert(fwd == 123, \"forward got {fwd}\");
    // Reverse: 3,2,1 → digits 3,2,1
    rev_sum = 0;
    for e in rev(db.s) { rev_sum = rev_sum * 10 + e.key }
    assert(rev_sum == 321, \"reverse got {rev_sum}\")
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn sorted_reverse_empty() {
    // rev() on an empty sorted collection completes without visiting any element
    code!(
        "struct Elm { key: integer }
struct Db { s: sorted<Elm[key]> }
fn test() {
    db = Db { s: [] };
    sum = 0;
    for e in rev(db.s) { sum += e.key }
    assert(sum == 0, \"empty rev got {sum}\")
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn sorted_remove() {
    // #remove on a sorted collection removes the current element while iterating
    code!(
        "struct Elm { key: integer, val: integer }
struct Db { s: sorted<Elm[key]> }
fn test() {
    db = Db { s: [Elm{key:1,val:10}, Elm{key:2,val:20}, Elm{key:3,val:30}, Elm{key:4,val:40}] };
    for e in db.s if e.key % 2 == 0 {
        e#remove
    }
    total = 0;
    for e in db.s { total += e.key }
    assert(total == 4, \"sum of remaining keys {total}\")
}"
    )
    .result(loft::data::Value::Null);
}

// hash_remove_key, index_remove_key: covered by issues.rs hash/index_key_null_removes_entry

#[test]
fn index_loop_remove_small() {
    // T0-2: loop #remove on an index with small N must empty the collection
    code!(
        "struct Elm { key: integer, val: integer }
struct Db { map: index<Elm[key]> }"
    )
    .expr(
        "db = Db { map: [
  Elm {key:1, val:10},
  Elm {key:2, val:20},
  Elm {key:3, val:30},
] };
for r in db.map { r#remove; }
cnt = 0;
for r in db.map { cnt += 1; }
cnt",
    )
    .warning("Variable r is never read at index_loop_remove_small:10:18")
    .result(Value::Int(0));
}

#[test]
fn index_loop_remove_large() {
    // T0-2: loop #remove on an index with N=20 elements must empty the collection
    code!(
        "struct Rec { id: integer, val: integer }
struct Db { rows: index<Rec[id]> }
fn test() {
    db = Db { rows: [] };
    for i in 0..20 { db.rows += [Rec { id: i, val: i * 2 }]; }
    cnt_before = 0;
    for r in db.rows { cnt_before += 1; }
    assert(cnt_before == 20, \"count before {cnt_before}\");
    for r in db.rows { r#remove; }
    cnt = 0;
    for r in db.rows { cnt += 1; }
    assert(cnt == 0, \"count after loop-remove {cnt}\");
}"
    )
    .warning("Variable r is never read at index_loop_remove_large:7:23")
    .result(Value::Null);
}

#[test]
fn index_key_null_removes_all() {
    // T0-3: key-null removal of all N elements must leave count == 0
    code!(
        "struct Elm { key: integer, val: integer }
struct Db { map: index<Elm[key]> }
fn check(n: integer) {
    db = Db { map: [] };
    for i in 0..n { db.map += [Elm { key: i, val: i }]; }
    for i in 0..n { db.map[i] = null; }
    cnt = 0;
    for r in db.map { cnt += 1; }
    assert(cnt == 0, \"key-null remove N={n} left {cnt}\");
}
fn test() {
    check(1);
    check(2);
    check(3);
    check(10);
    check(50);
    check(100);
}"
    )
    .warning("Variable r is never read at index_key_null_removes_all:8:22")
    .result(Value::Null);
}

#[test]
fn sorted_filtered_remove_large() {
    // T0-4: filtered loop #remove on a sorted with N=100 must give correct sum
    code!(
        "struct IntRec { id: integer, val: integer }
struct SDb { rows: sorted<IntRec[id]> }
fn test() {
    db = SDb { rows: [] };
    for i in 0..100 { db.rows += [IntRec { id: i, val: i }]; }
    for r in db.rows if r.id % 2 == 0 { r#remove; }
    odd_sum = 0;
    for r in db.rows { odd_sum += r.val; }
    // 1 + 3 + ... + 99 = 50^2 = 2500
    assert(odd_sum == 2500, \"sorted filtered remove odd sum {odd_sum}\");
}"
    )
    .result(Value::Null);
}

// sorted_remove_key: covered by issues.rs sorted_key_null_removes_entry
// sorted_loop_attrs_first_count, sorted_loop_attr_index, index_loop_attrs_first_count:
//   covered by 10-collections.loft (loop attrs section)

/// `#index` on index<T> is rejected at compile time (gives internal record number).
#[test]
fn index_loop_attr_index_rejected() {
    code!(
        "struct IE { id: integer, nm: text }
struct ID { rows: index<IE[id, nm]> }
fn test() {
    db = ID { rows: [] };
    for e in db.rows { _ = e#index; }
}"
    )
    .error(
        "#index is not supported on index<T> collections \
(it holds an internal record number, not a sequential counter); \
use #count instead at index_loop_attr_index_rejected:5:36",
    )
    .warning("Variable e is never read at index_loop_attr_index_rejected:5:23");
}

// Hash collections cannot be iterated directly at all; `fill_iter` emits a
// diagnostic and the for-loop fails to compile.  Behaviour is verified by the
// absence of hash-iteration code in 10-collections.loft and the error message
// produced at compile time when such code is written.
