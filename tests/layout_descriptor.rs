// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 Phase 0 — the layout-descriptor emitter is a FAITHFUL and SUFFICIENT
// transcription of the store layout. Three independent oracles, no FFI:
//
//   * faithfulness — `LayoutDesc::render_dump` reproduces `Stores::layout_dump`
//     byte-for-byte over a rich corpus (so its hash IS `layout_algo_hash`): the
//     descriptor loses none of the layout facts the @PLN97 contract pins.
//   * sufficiency  — `read_via_descriptor`, driven ONLY by the descriptor, matches
//     `read_data` AND a hand-computed byte string for a nested live value.
//   * anti-vacuity — a deliberately corrupted descriptor DIVERGES from `read_data`
//     (the harness can fail), and the store-internal boundary (keyed collections)
//     is refused by the reader but present as an `Iterated` node in the descriptor.

mod common;
extern crate loft;

use common::cached_default;
use loft::database::{Iterated, LayoutNode, Parts};
use loft::parser::Parser;

/// A corpus spanning every user-writable storage kind: scalars, narrow ints,
/// inline & nested vectors, vector<text>, hash / index / sorted collections, a
/// nested struct (inline), a value enum, and a struct-enum. (Mirrors the @PLN97
/// layout-golden corpus so faithfulness is checked on the same shapes.)
const CORPUS: &str = r#"
struct Scalars { b: boolean, c: character, s: single, f: float, i: integer, t: text }
struct Narrow { a: i32, b: u8, c: u16 }
struct Vec1 { v: vector<integer> }
struct VecNest { vv: vector<vector<integer>> }
struct VecText { v: vector<text> }
struct Item { ik: integer }
struct Bag { items: hash<Item[ik]> }
struct SortedBag { items: sorted<Item[ik]> }
struct IndexBag { items: index<Item[ik]> }
struct RefHost { child: Scalars }
enum Color { Red, Green, Blue }
enum Shape { Circle { radius: integer }, Rect { width: integer, height: integer } }
"#;

const CORPUS_TYPES: &[&str] = &[
    "Scalars",
    "Narrow",
    "Vec1",
    "VecNest",
    "VecText",
    "Item",
    "Bag",
    "SortedBag",
    "IndexBag",
    "RefHost",
    "Color",
    "Shape",
];

fn parse(src: &str) -> Parser {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(src, "descriptor_test", false);
    p
}

fn known(p: &Parser, name: &str) -> u16 {
    let kt = p.data.def(p.data.def_nr(name)).known_type;
    assert!(
        kt != u16::MAX,
        "type `{name}` did not resolve — parse drifted"
    );
    kt
}

// ── Oracle 1: faithfulness — descriptor render == layout_dump (⇒ hash == F9) ────

#[test]
fn descriptor_render_reproduces_layout_dump() {
    let p = parse(CORPUS);
    let roots: Vec<u16> = CORPUS_TYPES.iter().map(|n| known(&p, n)).collect();

    let desc = p.database.layout_descriptor(&roots);

    assert_eq!(
        desc.render_dump(),
        p.database.layout_dump(&roots),
        "descriptor render must reproduce the layout dump byte-for-byte (a lost/altered \
         layout fact = a red diff here)"
    );
    assert_eq!(
        desc.layout_hash(),
        p.database.layout_algo_hash(&roots),
        "descriptor layout hash must equal the @PLN97 F9 layout-algo hash"
    );
}

// ── Oracle 1b: positive controls — keyed collections become Iterated, not panic ─

#[test]
fn keyed_collections_emit_iterated_nodes() {
    let p = parse(CORPUS);
    let roots: Vec<u16> = CORPUS_TYPES.iter().map(|n| known(&p, n)).collect();
    let desc = p.database.layout_descriptor(&roots);

    let has = |pred: &dyn Fn(&Iterated) -> bool| {
        desc.nodes
            .values()
            .any(|n| matches!(n, LayoutNode::Iterated(it) if pred(it)))
    };
    assert!(
        has(&|it| matches!(it, Iterated::Hash { .. })),
        "hash<> → Iterated::Hash"
    );
    assert!(
        has(&|it| matches!(it, Iterated::Index { .. })),
        "index<> → Iterated::Index"
    );
    assert!(
        has(&|it| matches!(it, Iterated::Ordered { .. })),
        "struct-field sorted<> is array-backed → Iterated::Ordered"
    );

    // The `Item` element type of the hash is reachable in the descriptor (a cursor
    // will yield records of it in a later phase).
    let item = known(&p, "Item");
    assert!(
        desc.nodes.contains_key(&item),
        "hash element type Item is in the descriptor"
    );
    assert!(
        matches!(desc.nodes.get(&item), Some(LayoutNode::Record(_))),
        "Item is a Record node"
    );
}

// ── Oracle 2: sufficiency — read_via_descriptor == read_data == hand-computed ───

/// A record with a nested (inline) struct field and an inline scalar vector — the
/// meaty `read_data` walk: Record → {Text, Record → {Integer, Single}, Vector →
/// Integer, Boolean}.
const NESTED: &str = r#"
struct Inner { a: integer, b: single }
struct Outer { label: text, inner: Inner, nums: vector<integer>, flag: boolean }
"#;

/// Byte position of a named field within a struct type (from its `Parts::Struct`).
fn field(p: &Parser, tp: u16, name: &str) -> (u32, u16) {
    if let Parts::Struct(fields) = &p.database.types[tp as usize].parts {
        let f = fields
            .iter()
            .find(|f| f.name == name)
            .expect("field exists");
        (u32::from(f.position), f.content)
    } else {
        panic!("type {tp} is not a struct");
    }
}

#[test]
fn read_via_descriptor_matches_read_data_and_truth() {
    let mut p = parse(NESTED);
    let outer_tp = known(&p, "Outer");
    let inner_tp = known(&p, "Inner");

    let (label_pos, _) = field(&p, outer_tp, "label");
    let (inner_pos, inner_content) = field(&p, outer_tp, "inner");
    let (nums_pos, _) = field(&p, outer_tp, "nums");
    let (flag_pos, _) = field(&p, outer_tp, "flag");
    let (a_pos, _) = field(&p, inner_tp, "a");
    let (b_pos, _) = field(&p, inner_tp, "b");
    assert_eq!(
        inner_content, inner_tp,
        "the `inner` field is the inline Inner struct"
    );

    // Build a live Outer value in one store.
    let outer = p.database.database(64);
    {
        let store = &mut p.database.allocations[outer.store_nr as usize];
        let sr = store.set_str("hi");
        store.set_u32_raw(outer.rec, outer.pos + label_pos, sr);
        store.set_int(outer.rec, outer.pos + inner_pos + a_pos, 7);
        store.set_single(outer.rec, outer.pos + inner_pos + b_pos, 1.5);
        let vec_rec = loft::vector::alloc_vector_from_bytes(store, 8, 3, &[]);
        for (i, v) in [10i64, 20, 30].iter().enumerate() {
            store.set_int(vec_rec, 8 + 8 * i as u32, *v);
        }
        store.set_u32_raw(outer.rec, outer.pos + nums_pos, vec_rec);
        store.set_byte(outer.rec, outer.pos + flag_pos, 0, 1); // flag = true
    }

    // Hand-computed truth, in declaration order (== Parts::Struct field order):
    //   label "hi" | inner.a=7 (i64) | inner.b=1.5 (f32) | nums 10,20,30 (i64) | flag=1
    let mut truth: Vec<u8> = Vec::new();
    truth.extend_from_slice(b"hi");
    truth.extend_from_slice(&7i64.to_le_bytes());
    truth.extend_from_slice(&1.5f32.to_le_bytes());
    for v in [10i64, 20, 30] {
        truth.extend_from_slice(&v.to_le_bytes());
    }
    truth.push(1);

    let mut via_read_data = Vec::new();
    p.database
        .read_data(&outer, outer_tp, true, &mut via_read_data);

    let desc = p.database.layout_descriptor(&[outer_tp]);
    let mut via_desc = Vec::new();
    p.database
        .read_via_descriptor(&desc, &outer, outer_tp, true, &mut via_desc)
        .expect("nested value is in the serializable subset");

    assert_eq!(
        via_read_data, truth,
        "read_data must produce the hand-computed bytes"
    );
    assert_eq!(
        via_desc, truth,
        "read_via_descriptor must produce the hand-computed bytes"
    );

    // ── anti-vacuity: a corrupted descriptor MUST diverge from read_data ────────
    let mut bad = desc.clone();
    if let Some(LayoutNode::Record(fields)) = bad.nodes.get_mut(&inner_tp) {
        // point Inner.a at Inner.b's byte offset → reads the wrong bytes
        let bpos = fields.iter().find(|f| f.name == "b").unwrap().position;
        fields.iter_mut().find(|f| f.name == "a").unwrap().position = bpos;
    }
    let mut via_bad = Vec::new();
    p.database
        .read_via_descriptor(&bad, &outer, outer_tp, true, &mut via_bad)
        .unwrap();
    assert_ne!(
        via_bad, via_read_data,
        "a corrupted descriptor must diverge from read_data (proves the round-trip can fail)"
    );
}

// ── Oracle 2b: the store-internal boundary is refused by the byte reader ────────

#[test]
fn keyed_collection_read_is_refused_not_panicked() {
    let mut p = parse(CORPUS);
    let bag_tp = known(&p, "Bag");
    let desc = p.database.layout_descriptor(&[bag_tp]);

    // A zeroed Bag record is enough: the reader reaches the hash field and returns
    // Err (the walkable-subset boundary) before dereferencing anything — never a
    // panic like read_data does.
    let bag = p.database.database(16);
    let err = p
        .database
        .read_via_descriptor(&desc, &bag, bag_tp, true, &mut Vec::new());
    assert!(
        err.is_err(),
        "a hash<> field is refused by the byte reader (store-internal)"
    );
    assert!(
        err.unwrap_err().contains("store-internal"),
        "the refusal names the store-internal boundary"
    );
}

// ── @PLN105 Phase 2: descriptor → JSON (the JS reader's contract) ────────────────

/// Balanced-brace / string-aware JSON well-formedness check — no parser dependency.
fn json_balanced(s: &str) -> bool {
    let (mut braces, mut brackets, mut in_str, mut esc) = (0i32, 0i32, false, false);
    for c in s.chars() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => braces += 1,
            '}' => braces -= 1,
            '[' => brackets += 1,
            ']' => brackets -= 1,
            _ => {}
        }
        if braces < 0 || brackets < 0 {
            return false;
        }
    }
    braces == 0 && brackets == 0 && !in_str
}

#[test]
fn descriptor_to_json_is_well_formed_and_faithful() {
    let p = parse(CORPUS);
    let roots: Vec<u16> = CORPUS_TYPES.iter().map(|n| known(&p, n)).collect();
    let js = p.database.layout_descriptor(&roots).to_json();

    // Well-formed (the JS side does `JSON.parse`), with the three top-level tables.
    assert!(json_balanced(&js), "descriptor JSON is not balanced:\n{js}");
    assert!(js.starts_with("{\"nodes\":{"), "top-level shape:\n{js}");
    for key in ["\"nodes\":{", "\"names\":{", "\"sizes\":{"] {
        assert!(js.contains(key), "missing top-level {key}\n{js}");
    }

    // Every node kind the JS reader (§2) dispatches on is present in the corpus.
    for kind in [
        "\"kind\":\"base\"",     // scalar leaves (text/char/…)
        "\"kind\":\"record\"",   // Scalars / Narrow / RefHost
        "\"kind\":\"vector\"",   // Vec1
        "\"kind\":\"enum\"",     // Color
        "\"kind\":\"iterated\"", // hash / index / sorted
    ] {
        assert!(js.contains(kind), "corpus JSON missing node {kind}\n{js}");
    }

    // Faithful details: a base carries its wire scalar name; the value enum keeps its variants
    // in discriminant order; a keyed collection is iterated with its sub-kind (never structural).
    assert!(
        js.contains("\"kind\":\"base\",\"base\":\"text\""),
        "text base wire name\n{js}"
    );
    for v in ["Red", "Green", "Blue"] {
        assert!(
            js.contains(&format!("\"name\":\"{v}\"")),
            "Color.{v} variant\n{js}"
        );
    }
    assert!(
        js.contains("\"kind\":\"iterated\",\"sub\":\"hash\""),
        "hash<> → iterated/hash\n{js}"
    );
}
