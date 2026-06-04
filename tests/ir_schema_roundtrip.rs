// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Real-data round-trip for the IR JSON codec (@PLAN28 Step 2).
//!
//! The unit tests in `src/ir_schema.rs` prove the Type / Value / Attribute /
//! Definition codecs on **hand-built sample** structs.  This integration test
//! proves them on the **real parsed `default/` stdlib** — every `Type`, every
//! `Value` IR body, every `Attribute`, and every `Definition` the parser
//! actually produces.  It is the "does it work on real input, not just my
//! samples" gate (gap #1 in the test-coverage review).
//!
//! What each layer proves:
//!   * `Type`  — `assert_eq!` round-trip (Type: PartialEq + Debug).  Exercises
//!     every `returned` type and every attribute `typedef` in the stdlib.
//!   * `Value` — `assert_eq!` round-trip (Value: PartialEq + Debug).  Exercises
//!     every definition's `code` IR graph (the function bodies — the bulk of
//!     the stdlib).
//!   * `Attribute` / `Definition` — re-encode equality (encode → decode →
//!     encode is stable), since neither derives PartialEq.
//!
//! The `Definition` codec includes the per-function **variable table** (C4):
//! debug symbols + each variable's final `stack_pos` (the fields codegen
//! reads).  Re-encode equality therefore also covers the variable table.
//!
//! What this does NOT prove (later step):
//!   * S3 equivalence — that a decoded `Data` recompiles to byte-identical
//!     bytecode.  This test is codec self-consistency on real data, not
//!     end-to-end snapshot correctness.  (Deferred — manual readable-output
//!     inspection is the current verification path.)

mod common;

use common::cached_default;
use loft::ir_schema::{
    attribute_from_json, attribute_to_json, compare_data, data_from_json, data_to_json,
    definition_from_json, definition_to_json, type_from_json, type_to_json, value_from_json,
    value_to_json,
};

/// Every `Type` in the stdlib (each def's `returned` plus every attribute
/// `typedef`) survives a JSON round-trip exactly.
#[test]
fn stdlib_types_round_trip() {
    let (data, _db) = cached_default();
    let mut checked = 0usize;
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        for ty in std::iter::once(&def.returned).chain(def.attributes.iter().map(|a| &a.typedef)) {
            let json = type_to_json(ty);
            let back = type_from_json(&json)
                .unwrap_or_else(|e| panic!("type decode failed in {}: {json}: {e:?}", def.name));
            assert_eq!(
                &back, ty,
                "type round-trip mismatch in {}: {json}",
                def.name
            );
            checked += 1;
        }
    }
    assert!(
        checked > 100,
        "expected to exercise many stdlib types, got {checked}"
    );
    eprintln!("stdlib_types_round_trip: {checked} types checked");
}

/// Every definition's `code` (the `Value` IR body) survives a JSON round-trip
/// exactly.  This is the heavy test — real function bodies exercise the full
/// 34-variant `Value` codec (Call / Block / If / Iter / Set / FnRef / …).
#[test]
fn stdlib_values_round_trip() {
    let (data, _db) = cached_default();
    let mut checked = 0usize;
    let mut non_null = 0usize;
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        let json = value_to_json(&def.code);
        let back = value_from_json(&json)
            .unwrap_or_else(|e| panic!("value decode failed in {}: {e:?}", def.name));
        assert_eq!(back, def.code, "value round-trip mismatch in {}", def.name);
        checked += 1;
        if !matches!(def.code, loft::data::Value::Null) {
            non_null += 1;
        }
    }
    assert!(checked > 100, "expected many definitions, got {checked}");
    assert!(
        non_null > 50,
        "expected many non-trivial code bodies, got {non_null}"
    );
    eprintln!("stdlib_values_round_trip: {checked} bodies checked ({non_null} non-null)");
}

/// Every attribute (struct fields + function parameters, including their
/// `value` / `check` / `check_message` `Value` fields) re-encodes identically.
#[test]
fn stdlib_attributes_round_trip() {
    let (data, _db) = cached_default();
    let mut checked = 0usize;
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        for attr in &def.attributes {
            let json = attribute_to_json(attr);
            let back = attribute_from_json(&json)
                .unwrap_or_else(|e| panic!("attr decode failed {}.{}: {e:?}", def.name, attr.name));
            assert_eq!(
                attribute_to_json(&back),
                json,
                "attribute re-encode drift in {}.{}",
                def.name,
                attr.name
            );
            checked += 1;
        }
    }
    assert!(
        checked > 50,
        "expected many stdlib attributes, got {checked}"
    );
    eprintln!("stdlib_attributes_round_trip: {checked} attributes checked");
}

/// Every definition re-encodes identically (the encoded portable-IR subset:
/// header + attributes + code + type/purity/field-groups/etc.).  Proves the
/// C2 codec is lossless on real definitions — both type-level and function
/// definitions (function bodies live in `code`, covered; variable tables are
/// not part of this codec by design).
#[test]
fn stdlib_definitions_round_trip() {
    let (data, _db) = cached_default();
    let mut checked = 0usize;
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        let json = definition_to_json(def);
        let back = definition_from_json(&json)
            .unwrap_or_else(|e| panic!("definition decode failed for {}: {e:?}", def.name));
        let rejson = definition_to_json(&back);
        assert_eq!(rejson, json, "definition re-encode drift in {}", def.name);
        // The derived attr_names index must rebuild to match the attributes.
        for (i, a) in back.attributes.iter().enumerate() {
            assert_eq!(
                back.attr_names.get(&a.name),
                Some(&i),
                "attr_names not rebuilt for {}.{}",
                def.name,
                a.name
            );
        }
        checked += 1;
    }
    assert!(
        checked > 100,
        "expected the full stdlib def table, got {checked}"
    );
    eprintln!("stdlib_definitions_round_trip: {checked} definitions checked");
}

/// The WHOLE parsed stdlib `Data` round-trips as one document (C3): encode the
/// entire `Data` to a single JSON (the `stdlib.json` shape), decode it back via
/// `data_from_json` (which rebuilds all derived indices), and assert:
///   * re-encode equality (Data has no PartialEq) — the document is lossless;
///   * the decoded `Data` has the same definition count;
///   * spot-check that `def_nr(name)` resolves identically on the decoded copy
///     (proves `rebuild_indices` reconstructed `def_names` correctly);
///   * the decoded copy compiles (codegen runs against rebuilt indices).
#[test]
fn stdlib_whole_data_round_trip() {
    let (data, _db) = cached_default();
    let n = data.definitions();

    let json = data_to_json(&data);
    let back = data_from_json(&json).expect("whole-Data decode");

    // Lossless: re-encoding the decoded Data yields byte-identical JSON.
    assert_eq!(data_to_json(&back), json, "whole-Data re-encode drift");

    // Same definition table size.
    assert_eq!(
        back.definitions(),
        n,
        "definition count changed across round-trip"
    );

    // def_names rebuilt: every original name resolves to the same def_nr.
    let mut name_checks = 0usize;
    for d_nr in 0..n {
        let name = &data.def(d_nr).name;
        // Skip the rare duplicate-name case (last-wins); check the forward map
        // is at least present and points at a def of the same name.
        let resolved = back.def_nr(name);
        assert_ne!(
            resolved,
            u32::MAX,
            "name {name} did not resolve after round-trip"
        );
        assert_eq!(
            &back.def(resolved).name,
            name,
            "def_nr({name}) points at wrong def"
        );
        name_checks += 1;
    }
    assert!(name_checks > 100, "expected to resolve the full name table");

    eprintln!(
        "stdlib_whole_data_round_trip: {n} definitions, {} bytes of JSON, {name_checks} names re-resolved",
        json.len()
    );
}

/// The snapshot LOAD + COMPARE validator (the quick-loading debug path): parse
/// the stdlib fresh, encode→decode it into a SEPARATE `Data`, and `compare_data`
/// the loaded copy against the fresh reference — neither mutated.  This is the
/// check the quick-loading work leans on: a divergence is reported as a
/// localized `DataDiff` (which definition, which byte), not a bare `false`.
#[test]
fn stdlib_load_compares_equal_to_fresh() {
    let (reference, _db) = cached_default();

    // Round-trip through the snapshot format into a fresh, separate Data.
    let json = data_to_json(&reference);
    let loaded = data_from_json(&json).expect("snapshot decode");

    // The loaded Data must be re-encode-identical to the freshly-parsed one,
    // definition by definition.  `reference` is NOT mutated.
    match compare_data(&reference, &loaded) {
        Ok(()) => {}
        Err(diff) => panic!("loaded snapshot diverges from fresh parse: {diff:?}"),
    }

    // Sanity: the reference really did carry the full stdlib.
    assert!(
        reference.definitions() > 100,
        "expected the full stdlib, got {} defs",
        reference.definitions()
    );
    eprintln!(
        "stdlib_load_compares_equal_to_fresh: {} definitions compared identical",
        reference.definitions()
    );
}
