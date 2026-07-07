// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN97 Phase B — the golden layout-conformance test (the instrument).
//
// Pins the EXACT store layout of a corpus of loft structures: for each type,
// its record `size` and its `Parts` descriptor rendered structurally — struct
// field byte positions, narrow-int encodings (#399), and collection element
// record sizes (the nested-vector stride #477 changed). The store is ONE format
// (bit-identical in memory and on disk), and this layout is a deterministic
// function of the type table shared by BOTH backends — so pinning it here makes
// any layout change a red diff at commit time instead of silently-invalidated
// data (the gap @PLN97 exists to close).
//
// A layout change is not forbidden — it must be DELIBERATE: the golden diff
// shows exactly what moved, and `LAYOUT_ALGO_HASH` must be re-blessed in the
// same change (that constant is the seed of the plan's Phase-D handoff hash).
//
// Regenerate after an intentional layout change:
//   LOFT_BLESS_LAYOUT=1 cargo test --release --test layout_golden layout_golden
// then hand-verify the diff and update LAYOUT_ALGO_HASH to the printed value.

mod common;
extern crate loft;

use common::cached_default;
use loft::data::Data;
use loft::database::{Parts, Stores};
use loft::parser::Parser;
use std::path::PathBuf;

/// The corpus — one structure per representative layout cell. Expanded over
/// time (Phase A census); enum / sorted / index / tuple / closure are the known
/// gaps, to be added with the coverage self-audit (Phase B4).
const CORPUS: &str = r#"
struct Scalars { b: boolean, c: character, s: single, f: float, i: integer, t: text }
struct Narrow { a: i32, b: u8, c: u16 }
struct NotNull { i: integer not null, t: text not null }
struct Vec1 { v: vector<integer> }
struct VecNest { vv: vector<vector<integer>> }
struct VecText { v: vector<text> }
struct Item { ik: integer }
struct Bag { items: hash<Item[ik]> }
struct RefHost { child: Scalars }
"#;

/// The types to dump, in a fixed order (stable golden).
const TYPES: &[&str] = &[
    "Scalars", "Narrow", "NotNull", "Vec1", "VecNest", "VecText", "Item", "Bag", "RefHost",
];

/// A layout change flips this. Re-bless (with the golden) on an intentional change.
const LAYOUT_ALGO_HASH: u64 = 7_896_138_798_552_030_681;

fn type_name(db: &Stores, kt: u16) -> String {
    db.types
        .get(kt as usize)
        .map_or_else(|| format!("#{kt}"), |t| t.name.clone())
}

/// Render a type's `Parts` structurally — the load-bearing layout facts only
/// (positions, element sizes, narrow encodings), keyed by type NAME so the
/// golden is stable across unrelated known-type renumbering.
fn render_parts(db: &Stores, kt: u16) -> String {
    match &db.types[kt as usize].parts {
        Parts::Base => "base".to_string(),
        Parts::Struct(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|f| format!("{}@{}:{}", f.name, f.position, type_name(db, f.content)))
                .collect();
            format!("struct{{{}}}", inner.join(", "))
        }
        Parts::Enum(vs) => {
            let inner: Vec<String> = vs.iter().map(|(_, n)| n.clone()).collect();
            format!("enum{{{}}}", inner.join(", "))
        }
        Parts::EnumValue(tag, fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|f| format!("{}@{}:{}", f.name, f.position, type_name(db, f.content)))
                .collect();
            format!("enumvalue[{tag}]{{{}}}", inner.join(", "))
        }
        Parts::Byte(start, nul) => format!("byte(start={start},null={nul})"),
        Parts::Short(start, nul) => format!("short(start={start},null={nul})"),
        Parts::Int(start, nul) => format!("int4(start={start},null={nul})"),
        Parts::ShortRaw(start, nul) => format!("shortraw(start={start},null={nul})"),
        Parts::Vector(e) => {
            format!("vector<{}>(elem_size={})", type_name(db, *e), db.size(*e))
        }
        Parts::Array(e) => format!("array<{}>(elem_size={})", type_name(db, *e), db.size(*e)),
        Parts::Sorted(e, keys) => {
            format!(
                "sorted<{}>(keys={keys:?},elem_size={})",
                type_name(db, *e),
                db.size(*e)
            )
        }
        Parts::Ordered(e, keys) => {
            format!(
                "ordered<{}>(keys={keys:?},elem_size={})",
                type_name(db, *e),
                db.size(*e)
            )
        }
        Parts::Hash(e, keys) => {
            format!(
                "hash<{}>(keys={keys:?},elem_size={})",
                type_name(db, *e),
                db.size(*e)
            )
        }
        Parts::Index(e, keys, left) => {
            format!(
                "index<{}>(keys={keys:?},left={left},elem_size={})",
                type_name(db, *e),
                db.size(*e)
            )
        }
        Parts::Spacial(e, keys) => {
            format!(
                "spacial<{}>(keys={keys:?},elem_size={})",
                type_name(db, *e),
                db.size(*e)
            )
        }
        Parts::DbRef => "dbref12".to_string(),
        Parts::ChildRec(c) => format!("childrec<{}>", type_name(db, *c)),
    }
}

/// The types a type's layout references (fields, collection elements, childrecs) —
/// so the transitive closure pins the WHOLE layout graph, incl. nested-vector
/// element strides (#477), not just the root structs' field pointers.
fn referenced(db: &Stores, kt: u16, out: &mut Vec<u16>) {
    match &db.types[kt as usize].parts {
        Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
            out.extend(fields.iter().map(|f| f.content));
        }
        Parts::Vector(e) | Parts::Array(e) => out.push(*e),
        Parts::Sorted(e, _)
        | Parts::Ordered(e, _)
        | Parts::Hash(e, _)
        | Parts::Index(e, _, _)
        | Parts::Spacial(e, _) => out.push(*e),
        Parts::ChildRec(c) => out.push(*c),
        Parts::Enum(vs) => out.extend(vs.iter().map(|(t, _)| *t)),
        _ => {}
    }
}

fn dump_layout(db: &Stores, data: &Data) -> String {
    // Transitive closure from the corpus roots.
    let mut seen: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
    let mut stack: Vec<u16> = Vec::new();
    for name in TYPES {
        let kt = data.def(data.def_nr(name)).known_type;
        assert!(
            kt != u16::MAX && (kt as usize) < db.types.len(),
            "corpus type `{name}` did not resolve to a known_type — parse failed or syntax drifted"
        );
        stack.push(kt);
    }
    while let Some(kt) = stack.pop() {
        if !seen.insert(kt) {
            continue;
        }
        let mut refs = Vec::new();
        referenced(db, kt, &mut refs);
        for r in refs {
            if !seen.contains(&r) {
                stack.push(r);
            }
        }
    }
    // Render sorted by (name, known_type) so the golden is stable across
    // unrelated known-type renumbering.
    let mut items: Vec<u16> = seen.into_iter().collect();
    items.sort_by(|a, b| type_name(db, *a).cmp(&type_name(db, *b)).then(a.cmp(b)));
    let mut out = String::new();
    for kt in items {
        out.push_str(&format!(
            "{}\tsize={}\t{}\n",
            type_name(db, kt),
            db.size(kt),
            render_parts(db, kt)
        ));
    }
    out
}

/// Stable (FNV-1a) hash — NOT `DefaultHasher` (that is not stable across Rust
/// versions). This is the seed of @PLN97's layout-algo hash.
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/layout/corpus.txt")
}

#[test]
fn layout_golden() {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(CORPUS, "layout_corpus", false);

    let dump = dump_layout(&p.database, &p.data);
    let path = golden_path();

    if std::env::var("LOFT_BLESS_LAYOUT").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &dump).unwrap();
        eprintln!(
            "blessed {} ; LAYOUT_ALGO_HASH = {}",
            path.display(),
            fnv1a(&dump)
        );
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden {} — regenerate with LOFT_BLESS_LAYOUT=1",
            path.display()
        )
    });
    assert_eq!(
        dump, expected,
        "\nSTORE LAYOUT CHANGED. If intentional: re-bless (LOFT_BLESS_LAYOUT=1), \
         hand-verify the diff, and update LAYOUT_ALGO_HASH. If not: a layout \
         regression (the #477 class) just got caught.\n"
    );
    assert_eq!(
        fnv1a(&dump),
        LAYOUT_ALGO_HASH,
        "layout-algo hash drifted from the pinned constant"
    );
}
