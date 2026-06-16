// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN25 E2a.2 — Probe 1 (the core design claim): a nullable embedded struct
//! field, rewritten to the synthetic `__nullable<Row>` enum, is BYTE-IDENTICAL
//! to a hand-written `enum { Null, Some { … } }`.  If this holds, the existing
//! enum layout / construct / access / copy machinery carries null for free.
//!
//! Single `#[test]` so the `LOFT_E2_SYNTH` gate (read during `fill_all`) is set
//! with no concurrent parsing in this binary — the rewrite is scaffolding,
//! default-off, while the construct/access glue (E2a.3/4) is built.

extern crate loft;

use loft::data::Type;
use loft::parser::Parser;

const SRC: &str = r#"
struct Row { id: integer, tag: text }
struct Box { item: Row }

// Hand-written reference: the exact shape synthesis should reproduce.
enum HRow { HNull, HSome { id: integer, tag: text } }

fn test() {}
"#;

#[test]
fn nullable_struct_field_is_byte_identical_to_a_hand_enum() {
    // SAFETY: this binary holds a single test, so no other thread parses while
    // the gate is set.  Edition-2024 `set_var` is `unsafe` for exactly the
    // multi-threaded-mutation hazard we avoid here.
    unsafe {
        std::env::set_var("LOFT_E2_SYNTH", "1");
        // Embedded NON-vector struct-field nullability (`Box.item`) is gated on
        // its own opt-in now (more immature than the vector-element path).
        std::env::set_var("LOFT_E2_FIELDS", "1");
    }
    // Parse a real FILE (source = MAIN_SOURCE) rather than `parse_str` (which
    // leaves source at 0 = STD_SOURCE, which the scaffolding pass skips).
    let dir = std::env::temp_dir().join("loft_e2_probe");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let path = dir.join("e2.loft");
    std::fs::write(&path, SRC).expect("write probe");
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse(&path.to_string_lossy(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "probe must parse + lay out clean (synth on): {:?}",
        p.diagnostics.lines()
    );

    // (a) The rewrite happened: Box.item is now the synthetic enum, not Row.
    let box_d = p.data.def_nr("Box");
    let item = p.data.attr(box_d, "item");
    assert_ne!(item, usize::MAX, "Box has an `item` field");
    let item_ty = p.data.attr_type(box_d, item);
    let Type::Enum(syn, true, _) = item_ty else {
        panic!("Box.item must be a nullable enum, got {item_ty:?}");
    };
    assert_eq!(
        p.data.def(syn).name,
        "__nullable<Row>",
        "the synthetic enum is named for the struct"
    );

    // (b) Byte-identity: the synthetic `Some` variant vs the hand `HSome`
    // variant must have the same field offsets and overall size.
    let db = &p.database;
    assert!(db.has_type("__nullable<Row>"), "enum type registered");
    // The synthetic `Some` / `Null` variants register their DB structure under a
    // parent-enum-qualified key (`__nullable<Row>::Some`) so that two
    // `__nullable<S>` enums never collide on the bare name in the flat type table
    // (@PLN25 variant-name fix).  The qualifier does not change the layout.
    let some = db.name("__nullable<Row>::Some");
    let hsome = db.name("HSome");
    assert_ne!(some, u16::MAX, "synthetic Some variant registered");
    assert_ne!(hsome, u16::MAX, "hand HSome variant registered");

    // Discriminant at offset 0 (the whole representation hinges on this).
    assert_eq!(db.position(some, "enum"), 0, "synthetic discriminant @0");
    assert_eq!(db.position(hsome, "enum"), 0, "hand discriminant @0");

    // Payload offsets ride past the discriminant, identically on both.
    assert_eq!(
        db.position(some, "id"),
        db.position(hsome, "id"),
        "id offset matches the hand enum"
    );
    assert_eq!(
        db.position(some, "tag"),
        db.position(hsome, "tag"),
        "tag offset matches the hand enum"
    );
    assert_eq!(
        db.size(some),
        db.size(hsome),
        "Some variant size matches the hand enum"
    );

    unsafe {
        std::env::remove_var("LOFT_E2_SYNTH");
        std::env::remove_var("LOFT_E2_FIELDS");
    }
}
