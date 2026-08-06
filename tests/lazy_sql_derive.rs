// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN129 arc B step 2 — the descriptor carries enough to write the SELECT.
//
// Driven by a REAL parse rather than a hand-built descriptor: the claim is about
// what `layout_descriptor` emits for a collection someone declared, so a
// descriptor assembled by the test would prove only that the builder agrees with
// itself. The two worked examples in QUERIES.md are here verbatim, and so are
// the cases that document does not spell — a composite hash key, a descending
// direction bit, and the shapes that must REFUSE rather than emit a query that
// runs and is wrong.

mod common;
extern crate loft;

use common::cached_default;
use loft::database::sql_query::{Bound, QueryShape, derive_select};
use loft::parser::Parser;

const CORPUS: &str = r#"
struct Person { id: integer, name: text }
struct Position { person_id: integer, company_id: integer, moment: integer, ended: integer }
struct Pair { a: integer, b: integer, note: text }
struct Fall { k: integer, v: integer }
struct Nested { id: integer, who: Person }

struct Holder {
  persons: hash<Person[id]>,
  positions: index<Position[person_id, moment]>,
  pairs: hash<Pair[a, b]>,
  falling: sorted<Fall[-k]>,
  nests: hash<Nested[id]>,
  spatial: spatial<Pair[a, b]>,
}
"#;

fn parse(src: &str) -> Parser {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(src, "sql_derive_test", false);
    p
}

/// The type-nr of `Holder`'s field `name` — a collection type, which is what the
/// derivation takes.
fn collection(p: &Parser, field: &str) -> u16 {
    let kt = p.data.def(p.data.def_nr("Holder")).known_type;
    let loft::database::Parts::Struct(fields) = &p.database.types[kt as usize].parts else {
        panic!("Holder is not a struct — parse drifted");
    };
    fields
        .iter()
        .find(|f| f.name == field)
        .unwrap_or_else(|| panic!("Holder has no field `{field}`"))
        .content
}

// ── The two worked examples from QUERIES.md § the derivation ──────────────────

#[test]
fn hash_equality_is_the_persons_select() {
    let p = parse(CORPUS);
    let tp = collection(&p, "persons");
    let desc = p.database.layout_descriptor(&[tp]);

    let sql = derive_select(&desc, tp, QueryShape::Equality).expect("hash<Person[id]> derives");
    assert_eq!(sql.text, "SELECT id, name FROM person WHERE id = ?");
    assert_eq!(sql.params.len(), 1);
    assert_eq!(sql.params[0].bound, Bound::Eq);
    // The columns say where a row LANDS, and they must be the record's own field
    // order — a row written back in SELECT order into the wrong fields is the
    // silent corruption this pairing exists to prevent.
    assert_eq!(
        sql.columns.iter().map(|c| c.field).collect::<Vec<_>>(),
        vec![0, 1]
    );
}

#[test]
fn index_range_is_the_positions_select_with_its_own_direction() {
    let p = parse(CORPUS);
    let tp = collection(&p, "positions");
    let desc = p.database.layout_descriptor(&[tp]);

    let sql = derive_select(&desc, tp, QueryShape::Range).expect("index range derives");
    assert_eq!(
        sql.text,
        "SELECT person_id, company_id, moment, ended FROM position \
         WHERE person_id = ? AND moment BETWEEN ? AND ? ORDER BY moment ASC"
    );
    // One pinned key, then the two ends of the ranged one — the order a caller
    // binds them in.
    assert_eq!(
        sql.params.iter().map(|p| p.bound).collect::<Vec<_>>(),
        vec![Bound::Eq, Bound::Low, Bound::High]
    );
}

// ── What QUERIES.md does not spell ────────────────────────────────────────────

#[test]
fn composite_hash_key_pins_every_column() {
    let p = parse(CORPUS);
    let tp = collection(&p, "pairs");
    let desc = p.database.layout_descriptor(&[tp]);

    let sql = derive_select(&desc, tp, QueryShape::Equality).expect("composite hash derives");
    assert_eq!(
        sql.text,
        "SELECT a, b, note FROM pair WHERE a = ? AND b = ?"
    );
    assert_eq!(sql.params.len(), 2);
}

#[test]
fn a_descending_key_orders_descending() {
    let p = parse(CORPUS);
    let tp = collection(&p, "falling");
    let desc = p.database.layout_descriptor(&[tp]);

    let sql = derive_select(&desc, tp, QueryShape::Range).expect("sorted range derives");
    assert_eq!(
        sql.text, "SELECT k, v FROM fall WHERE k BETWEEN ? AND ? ORDER BY k DESC",
        "the direction is the collection's own declaration, not a convention"
    );
}

// ── The refusals — each one a query that would otherwise run and be wrong ──────

#[test]
fn a_hash_refuses_a_range() {
    let p = parse(CORPUS);
    let tp = collection(&p, "persons");
    let desc = p.database.layout_descriptor(&[tp]);
    assert!(
        derive_select(&desc, tp, QueryShape::Range).is_none(),
        "a hash has no order to range over"
    );
}

#[test]
fn a_spatial_collection_refuses_rather_than_scanning() {
    let p = parse(CORPUS);
    let tp = collection(&p, "spatial");
    let desc = p.database.layout_descriptor(&[tp]);
    assert!(derive_select(&desc, tp, QueryShape::Equality).is_none());
    assert!(derive_select(&desc, tp, QueryShape::Range).is_none());
}

#[test]
fn a_non_column_field_refuses_the_whole_query() {
    let p = parse(CORPUS);
    let tp = collection(&p, "nests");
    let desc = p.database.layout_descriptor(&[tp]);
    assert!(
        derive_select(&desc, tp, QueryShape::Equality).is_none(),
        "a nested struct is another table's row; omitting it would materialise a \
         record with a field nobody filled"
    );
}

#[test]
fn a_type_with_no_record_node_refuses() {
    let p = parse(CORPUS);
    // `Person` itself is a record, not a collection — nothing to derive from.
    let tp = p.data.def(p.data.def_nr("Person")).known_type;
    let desc = p.database.layout_descriptor(&[tp]);
    assert!(derive_select(&desc, tp, QueryShape::Equality).is_none());
    // And a type-id the descriptor never heard of.
    assert!(derive_select(&desc, u16::MAX, QueryShape::Equality).is_none());
}
