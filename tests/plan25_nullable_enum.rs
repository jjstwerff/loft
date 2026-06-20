// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN25 E2a.1 — `Data::nullable_enum_for` synthesises the 2-variant enum
//! `{ Null, Some<fields-of struct> }` that backs a nullable embedded struct
//! field / vector element.  These tests pin the def-level CONTRACT: the helper
//! builds an enum whose definition structure mirrors a hand-written
//! `enum { Null, Some { … } }` (so the existing layout pass produces a
//! byte-identical record).  The end-to-end byte-layout check (Probe 1) lands
//! with E2a.2, when the helper is wired into the field hook and `introspect`
//! can dump the synthesised type.

extern crate loft;

use loft::data::{DefType, Type, Value};
use loft::parser::Parser;

/// Parse the stdlib + a single `struct Row { id: integer, tag: text }`, then
/// synthesise its nullable-enum and hand the (parser, struct_d, enum_d) back.
fn synth() -> (Parser, u32, u32) {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse_str("struct Row { id: integer, tag: text }", "<probe>", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "probe source must parse clean: {:?}",
        p.diagnostics.lines()
    );
    let row_d = p.data.def_nr("Row");
    assert_ne!(row_d, u32::MAX, "Row must be defined");
    let enum_d = p.data.nullable_enum_for(&mut p.lexer, row_d);
    (p, row_d, enum_d)
}

#[test]
fn synthesises_a_two_variant_enum() {
    let (p, _row_d, e) = synth();
    assert_eq!(
        p.data.def_type(e),
        DefType::Enum,
        "synthetic type is an enum"
    );
    // Struct-enum (carries a payload) → discriminator type Enum(e, true).
    assert!(
        matches!(p.data.def(e).returned(), Type::Enum(d, true, _) if *d == e),
        "returned type must be Enum(self, true), got {:?}",
        p.data.def(e).returned()
    );
    // Parent carries one `constant` attribute per variant, in order, with the
    // discriminant values 1 (Null) and 2 (Some) — enum values start at 1.
    let attrs = &p.data.def(e).attributes;
    assert_eq!(attrs.len(), 2, "parent has exactly two variant attributes");
    assert_eq!(attrs[0].name, "Null");
    assert!(attrs[0].constant, "Null attribute is constant");
    assert_eq!(
        attrs[0].value,
        Value::Enum(1, u16::MAX),
        "Null discriminant = 1"
    );
    assert_eq!(attrs[1].name, "Some");
    assert!(attrs[1].constant, "Some attribute is constant");
    assert_eq!(
        attrs[1].value,
        Value::Enum(2, u16::MAX),
        "Some discriminant = 2"
    );
}

#[test]
fn null_variant_is_unit_some_carries_the_struct_payload() {
    let (p, _row_d, e) = synth();

    let nv = p.data.variant_of(e, "Null");
    let sv = p.data.variant_of(e, "Some");
    assert_ne!(nv, u32::MAX, "Null variant exists");
    assert_ne!(sv, u32::MAX, "Some variant exists");
    assert_eq!(p.data.def_type(nv), DefType::EnumValue);
    assert_eq!(p.data.def_type(sv), DefType::EnumValue);
    assert_eq!(p.data.def(nv).parent, e, "Null parent is the enum");
    assert_eq!(p.data.def(sv).parent, e, "Some parent is the enum");

    // Null is a unit variant — no attributes (matches a hand-written unit
    // variant; its discriminant rides Some's offset-0 slot).
    assert!(
        p.data.def(nv).attributes.is_empty(),
        "Null variant carries no fields"
    );

    // Single-payload form: discriminant `enum` at index 0, then ONE inline
    // `payload` field whose type IS the struct itself (dense `S`) — see
    // plans/25-nullable-sequences/single-payload-refactor.md.
    let sa = &p.data.def(sv).attributes;
    assert_eq!(sa.len(), 2, "Some = enum + payload");
    assert_eq!(sa[0].name, "enum", "discriminant must be the first field");
    assert_eq!(
        sa[0].value,
        Value::Enum(2, u16::MAX),
        "Some discriminant = 2"
    );
    assert_eq!(sa[1].name, "payload", "single inline payload field");
}

#[test]
fn payload_field_types_match_the_struct() {
    let (p, row_d, e) = synth();
    let sv = p.data.variant_of(e, "Some");
    // Single-payload form: the `Some` variant carries ONE inline `payload` field
    // whose type IS the struct (`Reference(Row)`), so its dense layout is preserved
    // and a payload sub-ref is a valid dense `S` reference (no copy at value boundaries).
    let payload = &p.data.def(sv).attributes[1]; // [0] is the discriminant
    assert_eq!(payload.name, "payload", "single inline payload field");
    assert!(
        matches!(payload.typedef, Type::Reference(d, _) if d == row_d),
        "payload type must be Reference(Row), got {:?}",
        payload.typedef
    );
}

#[test]
fn is_idempotent_and_marked_synthetic() {
    let (mut p, row_d, e) = synth();
    // A second call returns the SAME def (memoised on the struct name).
    let again = p.data.nullable_enum_for(&mut p.lexer, row_d);
    assert_eq!(again, e, "nullable_enum_for is memoised per struct");
    assert!(
        p.data.def(e).synthetic.is_some(),
        "synthetic enum is marked synthetic"
    );
}
