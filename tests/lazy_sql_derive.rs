// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN129 arc B steps 2+3 — the descriptor carries enough to write the SELECT,
// and the declared mapping supplies the names it cannot.
//
// Driven by a REAL parse rather than a hand-built descriptor: the claim is about
// what `layout_descriptor` emits for a collection someone declared, so a
// descriptor assembled by the test would prove only that the builder agrees with
// itself. The two worked examples in QUERIES.md are here, and so are the cases
// that document does not spell — a composite hash key, a descending direction
// bit, a reserved word for a column name, and the shapes that must REFUSE rather
// than emit a query that runs and is wrong.

mod common;
extern crate loft;

use common::cached_default;
use loft::database::sql_query::{Bound, Mapping, Placeholder, QueryShape, Quoting, derive_select};
use loft::parser::Parser;

const CORPUS: &str = r#"
struct Person { id: integer, name: text }
struct Position { person_id: integer, company_id: integer, started: integer, ended: integer }
struct Pair { a: integer, b: integer, note: text }
struct Fall { k: integer, v: integer }
struct Nested { id: integer, who: Person }
// The plan's motivating history row, spelled the way an author would: `from` and
// `to` are ordinary loft field names and reserved words in every SQL engine.
struct Spell { person_id: integer, from: integer, to: integer }

struct Holder {
  persons: hash<Person[id]>,
  positions: index<Position[person_id, started]>,
  pairs: hash<Pair[a, b]>,
  falling: sorted<Fall[-k]>,
  nests: hash<Nested[id]>,
  spatial: spatial<Pair[a, b]>,
  spells: hash<Spell[person_id]>,
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

/// The parse, the collection's type-nr and its descriptor — every test starts
/// here.
fn schema(field: &str) -> (Parser, u16, loft::database::LayoutDesc) {
    let p = parse(CORPUS);
    let tp = collection(&p, field);
    let desc = p.database.layout_descriptor(&[tp]);
    (p, tp, desc)
}

// ── The two worked examples from QUERIES.md § the derivation ──────────────────

#[test]
fn hash_equality_is_the_persons_select() {
    let (_p, tp, desc) = schema("persons");

    let sql = derive_select(&desc, tp, QueryShape::Equality, &Mapping::default())
        .expect("hash<Person[id]> derives");
    assert_eq!(
        sql.text,
        "SELECT \"id\", \"name\" FROM \"person\" WHERE \"id\" = ?"
    );
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
    let (_p, tp, desc) = schema("positions");

    let sql = derive_select(&desc, tp, QueryShape::Range, &Mapping::default())
        .expect("index range derives");
    assert_eq!(
        sql.text,
        "SELECT \"person_id\", \"company_id\", \"started\", \"ended\" FROM \"position\" \
         WHERE \"person_id\" = ? AND \"started\" BETWEEN ? AND ? ORDER BY \"started\" ASC"
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
    let (_p, tp, desc) = schema("pairs");

    let sql = derive_select(&desc, tp, QueryShape::Equality, &Mapping::default())
        .expect("composite hash derives");
    assert_eq!(
        sql.text,
        "SELECT \"a\", \"b\", \"note\" FROM \"pair\" WHERE \"a\" = ? AND \"b\" = ?"
    );
    assert_eq!(sql.params.len(), 2);
}

#[test]
fn a_descending_key_orders_descending() {
    let (_p, tp, desc) = schema("falling");

    let sql = derive_select(&desc, tp, QueryShape::Range, &Mapping::default())
        .expect("sorted range derives");
    assert_eq!(
        sql.text,
        "SELECT \"k\", \"v\" FROM \"fall\" WHERE \"k\" BETWEEN ? AND ? ORDER BY \"k\" DESC",
        "the direction is the collection's own declaration, not a convention"
    );
}

// ── Step 3: the mapping, and the reserved word it exists for ──────────────────

#[test]
fn reserved_words_survive_the_default_quoting() {
    let (_p, tp, desc) = schema("spells");

    let sql = derive_select(&desc, tp, QueryShape::Equality, &Mapping::default())
        .expect("a history row derives");
    assert_eq!(
        sql.text, "SELECT \"person_id\", \"from\", \"to\" FROM \"spell\" WHERE \"person_id\" = ?",
        "`from`/`to` are ordinary loft field names; unquoted this query parses nowhere"
    );
}

#[test]
fn bare_quoting_writes_the_name_as_the_author_did() {
    let (_p, tp, desc) = schema("persons");
    let bare = Mapping::new(Quoting::Bare, Placeholder::Question);

    let sql = derive_select(&desc, tp, QueryShape::Equality, &bare).expect("bare derives");
    assert_eq!(sql.text, "SELECT id, name FROM person WHERE id = ?");
}

#[test]
fn a_declared_mapping_renames_the_table_and_the_column() {
    let (_p, tp, desc) = schema("persons");
    let mut map = Mapping::new(Quoting::Bare, Placeholder::Question);
    map.map_table(&desc, "Person", "persoon").expect("table");
    map.map_column(&desc, "Person", "name", "naam")
        .expect("column");

    let sql = derive_select(&desc, tp, QueryShape::Equality, &map).expect("mapped derives");
    assert_eq!(sql.text, "SELECT id, naam FROM persoon WHERE id = ?");
}

#[test]
fn a_mapped_key_column_is_renamed_in_the_where_too() {
    let (_p, tp, desc) = schema("persons");
    let mut map = Mapping::new(Quoting::Bare, Placeholder::Question);
    map.map_column(&desc, "Person", "id", "person_nr")
        .expect("column");

    let sql = derive_select(&desc, tp, QueryShape::Equality, &map).expect("mapped derives");
    assert_eq!(
        sql.text, "SELECT person_nr, name FROM person WHERE person_nr = ?",
        "a rename that reached the SELECT but not the WHERE would query a column \
         the table does not have"
    );
}

#[test]
fn backticks_and_numbered_placeholders_are_declared_not_guessed() {
    let (_p, tp, desc) = schema("positions");

    let mysql = Mapping::new(Quoting::Backtick, Placeholder::Question);
    let sql = derive_select(&desc, tp, QueryShape::Range, &mysql).expect("mysql derives");
    assert!(
        sql.text.starts_with("SELECT `person_id`, `company_id`"),
        "{}",
        sql.text
    );

    let pg = Mapping::new(Quoting::Double, Placeholder::Numbered);
    let sql = derive_select(&desc, tp, QueryShape::Range, &pg).expect("postgres derives");
    assert!(
        sql.text.ends_with(
            "WHERE \"person_id\" = $1 AND \"started\" BETWEEN $2 AND $3 ORDER BY \"started\" ASC"
        ),
        "{}",
        sql.text
    );
}

#[test]
fn a_mapping_naming_something_absent_is_refused_at_construction() {
    let (_p, _tp, desc) = schema("persons");
    let mut map = Mapping::default();
    assert!(
        map.map_column(&desc, "Person", "naam", "naam").is_err(),
        "a typo in a mapping is otherwise invisible — the derivation would use the \
         default and query a column nobody meant"
    );
    assert!(map.map_table(&desc, "Persoon", "persoon").is_err());
    assert!(map.map_column(&desc, "Persoon", "name", "naam").is_err());
}

// ── The refusals — each one a query that would otherwise run and be wrong ──────

#[test]
fn a_hash_refuses_a_range() {
    let (_p, tp, desc) = schema("persons");
    assert!(
        derive_select(&desc, tp, QueryShape::Range, &Mapping::default()).is_none(),
        "a hash has no order to range over"
    );
}

#[test]
fn a_spatial_collection_refuses_rather_than_scanning() {
    let (_p, tp, desc) = schema("spatial");
    assert!(derive_select(&desc, tp, QueryShape::Equality, &Mapping::default()).is_none());
    assert!(derive_select(&desc, tp, QueryShape::Range, &Mapping::default()).is_none());
}

#[test]
fn a_non_column_field_refuses_the_whole_query() {
    let (_p, tp, desc) = schema("nests");
    assert!(
        derive_select(&desc, tp, QueryShape::Equality, &Mapping::default()).is_none(),
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
    assert!(derive_select(&desc, tp, QueryShape::Equality, &Mapping::default()).is_none());
    // And a type-id the descriptor never heard of.
    assert!(derive_select(&desc, u16::MAX, QueryShape::Equality, &Mapping::default()).is_none());
}

#[test]
fn bare_quoting_refuses_a_name_it_cannot_write() {
    let (_p, tp, desc) = schema("persons");
    let mut map = Mapping::new(Quoting::Bare, Placeholder::Question);
    map.map_table(&desc, "Person", "my person").expect("table");
    assert!(
        derive_select(&desc, tp, QueryShape::Equality, &map).is_none(),
        "unquoted, a name with a space ends the identifier and the rest of the \
         query means something else"
    );
}
