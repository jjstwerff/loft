// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN139 stage A — the drop cascade's one query, landed INERT.
//!
//! [`Data::owns_droppable`] answers *"does the death of a value of this type mean any work
//! at all"*: the type declares `OpDrop` itself, or it transitively OWNS a member that does.
//! It is deliberately a different fact from `drop_hook_nr`, which answers only *"does this
//! type run code of its own"* — a wrapper with no hook around a type that has one answers
//! `false` there and `true` here, and that combination IS loft#849.
//!
//! Nothing consumes the query yet. These tests are what makes it real before stage B routes
//! the cascade through it: a query with no caller and no test is a claim, not a fact.
//!
//! The cases are chosen so each one can only pass for the right reason — every `true` cell
//! has a `false` twin differing in ONE axis (the same shape without a hook anywhere), so a
//! query that simply answered `true` for every record type would fail half of them.

extern crate loft;

use loft::data::Data;
use loft::parser::Parser;

/// Parse `script` against the stdlib and hand back its `Data`.
///
/// No `scopes::check` and no `byte_code`: the query reads the DEFINITION table, which is
/// complete once parsing is. Running less also keeps a failure here pointing at the query
/// rather than at a later phase.
fn parse(script: &str) -> Data {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(script, "owns_droppable", false);
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    p.data
}

/// The verdict for a named user type.
fn owns(data: &Data, type_name: &str) -> bool {
    let d = data.def_nr(type_name);
    assert!(d != u32::MAX, "type `{type_name}` not found");
    data.owns_droppable(d)
}

/// Every case shares one droppable leaf `H` and one plain leaf `P`, so each pair below
/// differs from its twin in exactly one axis.
const PRELUDE: &str = "
struct H { id: integer }
fn OpDrop(self: H) { if self.id != 0 { } }
struct P { id: integer }
";

#[test]
fn the_droppable_itself_and_a_plain_twin() {
    let data = parse(&format!("{PRELUDE}fn main() {{ }}"));
    assert!(
        owns(&data, "H"),
        "a type with its own hook owns a droppable"
    );
    assert!(!owns(&data, "P"), "the same shape with no hook does not");
}

#[test]
fn a_struct_field_carries_it_and_a_plain_field_does_not() {
    let data = parse(&format!(
        "{PRELUDE}
struct WrapH {{ h: H }}
struct WrapP {{ p: P }}
fn main() {{ }}"
    ));
    assert!(owns(&data, "WrapH"), "a struct field carries it");
    assert!(!owns(&data, "WrapP"), "a plain field does not");
}

#[test]
fn nesting_carries_it_to_any_depth() {
    let data = parse(&format!(
        "{PRELUDE}
struct L1 {{ h: H }}
struct L2 {{ a: L1 }}
struct L3 {{ b: L2 }}
struct M1 {{ p: P }}
struct M2 {{ a: M1 }}
fn main() {{ }}"
    ));
    assert!(owns(&data, "L3"), "three levels down still reaches it");
    assert!(!owns(&data, "M2"), "no depth invents one");
}

#[test]
fn an_enum_payload_carries_it() {
    // The variant's fields are NOT the enum's attributes — they hang off the variant def,
    // which is a CHILD. A walk over attributes alone answers `false` here, which is the
    // whole reason this case exists (it is also @PLN138's registry shape).
    let data = parse(&format!(
        "{PRELUDE}
enum WithH {{ VH {{ h: H }}, VNone }}
enum WithP {{ VP {{ p: P }}, VNone2 }}
fn main() {{ }}"
    ));
    assert!(owns(&data, "WithH"), "an enum variant's payload carries it");
    assert!(!owns(&data, "WithP"), "a payload with no hook does not");
}

#[test]
fn a_unit_only_enum_owns_nothing() {
    let data = parse(&format!(
        "{PRELUDE}
enum Units {{ UA, UB, UC }}
fn main() {{ }}"
    ));
    assert!(!owns(&data, "Units"), "no payload, nothing to drop");
}

#[test]
fn a_collection_of_droppables_carries_it() {
    let data = parse(&format!(
        "{PRELUDE}
struct VecH {{ v: vector<H> }}
struct VecP {{ v: vector<P> }}
fn main() {{ }}"
    ));
    assert!(owns(&data, "VecH"), "owning a vector of them owns them");
    assert!(!owns(&data, "VecP"), "a vector of plain records does not");
}

#[test]
fn a_vector_of_wrappers_carries_it() {
    // Two constructors composed — the element type is itself only an owner by way of its
    // field. Each step alone is covered above; this is the one that proves they compose.
    let data = parse(&format!(
        "{PRELUDE}
struct WrapH {{ h: H }}
struct VecWrap {{ v: vector<WrapH> }}
fn main() {{ }}"
    ));
    assert!(owns(&data, "VecWrap"), "vector-of-wrapper composes");
}

#[test]
fn a_self_referential_type_terminates() {
    // A tree node holding its own children is ordinary loft, and a naive walk recurses
    // forever. The assertion that matters is that this test RETURNS at all; the verdicts
    // beside it check the cycle guard did not also swallow the answer.
    let data = parse(&format!(
        "{PRELUDE}
struct NodeH {{ h: H, kids: vector<NodeH> }}
struct NodeP {{ p: P, kids: vector<NodeP> }}
fn main() {{ }}"
    ));
    assert!(owns(&data, "NodeH"), "a cycle must not hide a real hook");
    assert!(!owns(&data, "NodeP"), "nor invent one");
}

#[test]
fn a_cycle_reaching_the_hook_only_through_the_back_edge() {
    // The sharper cycle: `A` owns nothing directly, `B` owns `H`, and they point at each
    // other. A guard that answered `false` for a revisited def AND cached that answer would
    // get this wrong depending on which end it was asked from — so both ends are asserted.
    let data = parse(&format!(
        "{PRELUDE}
struct A {{ b: vector<B> }}
struct B {{ h: H, a: vector<A> }}
fn main() {{ }}"
    ));
    assert!(owns(&data, "B"), "asked from the end that holds the hook");
    assert!(owns(&data, "A"), "and from the end that only reaches it");
}

#[test]
fn a_plain_program_answers_false_everywhere() {
    // The inertness anchor: a program with no `OpDrop` at all must answer `false` for every
    // one of its types, so stage B's cascade cannot fire where nothing asked for it.
    let data = parse(
        "struct Leaf { n: integer }
struct Holder { s: Leaf, v: vector<Leaf> }
enum Choice { CA { s: Leaf }, CNone }
fn main() { }",
    );
    for t in ["Leaf", "Holder", "Choice"] {
        assert!(
            !owns(&data, t),
            "`{t}` owns no droppable in a hook-free program"
        );
    }
}

#[test]
fn a_missing_type_is_not_an_error() {
    // The emitter asks about whatever type a binding carries, including `u32::MAX` for an
    // unresolved one. That must answer `false`, not panic.
    let data = parse("fn main() { }");
    assert!(!data.owns_droppable(u32::MAX), "the sentinel answers false");
    let past_end = data.definitions() + 1;
    assert!(
        !data.owns_droppable(past_end),
        "an out-of-range def answers false"
    );
}
