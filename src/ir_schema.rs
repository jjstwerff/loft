// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! IR store schema — @PLAN28 startup-cache Step 2, rung S1.
//!
//! Registers the compiler's own IR types (`Value`, `Type`, `Definition`,
//! `Block`, `Function`, `Attribute`, and their payload shapes) as a
//! `Stores` struct-enum schema, the **same representation `loft --native`
//! emits for user struct-enums** (see NATIVE.md § `output_init`).  Once a
//! `Data` can be expressed as records under this schema, the existing
//! database JSON generator (`Stores::show_json` /
//! `populate_struct_from_jsonvalue`) can round-trip it without a bespoke
//! serializer — that is rungs S2–S4.
//!
//! ## Status: S1 — registration only, no reader/writer yet
//!
//! This module is **dead code today**: nothing in the live compile path
//! calls [`register_ir_schema`].  S1's contract is exactly that it
//! registers every IR type without colliding with the base types or with
//! each other.  The native↔store bridge (S2/S3) and the JSON wiring (S4)
//! land in later passes, each its own green PR.
//!
//! ## Why the names are prefixed
//!
//! IR type names are prefixed with [`IR_PREFIX`] (`$ir.`) so they can
//! never collide with a user program's `struct Value` / `struct Type` /
//! etc.  The prefix is not a legal loft identifier, so these schema
//! entries are reachable only through this module.
//!
//! ## Two-phase registration (required for recursion)
//!
//! `Value` embeds `Box<Value>` / `Vec<Value>` and `Block`; `Type` embeds
//! `Box<Type>` / `Vec<Type>`.  A field can only reference a type that
//! already has a number, so registration is two-phase:
//!   1. **Shells** — `enumerate()` / `structure()` every IR type so each
//!      has a `known_type` (this rung, S1).
//!   2. **Fields + variants** — `field()` / `value()` wiring that
//!      references those numbers (later within the S1 pass).
//!
//! S1 lands phase 1 (shells) first: it is the part that proves "registers
//! without collision", and every recursive field added in phase 2 depends
//! on the shells already existing.

use crate::database::Stores;

/// Prefix on every IR schema type name, so they cannot collide with a
/// user program's own `struct Value` etc.  Not a legal loft identifier.
pub const IR_PREFIX: &str = "$ir.";

/// The IR enum / struct type names registered as schema shells.  The two
/// recursive enums (`Value`, `Type`) plus the structs they and
/// `Definition` own.  Order is irrelevant for shell registration — every
/// name simply needs a slot before phase-2 field wiring references it.
const IR_ENUMS: &[&str] = &["Value", "Type"];

const IR_STRUCTS: &[&str] = &[
    // Top-level definition graph.
    "Data",
    "Definition",
    "Attribute",
    "Argument",
    "Block",
    "ParForBody",
    "Function",
    "Variable",
    "IntegerSpec",
    "Position",
    "Key",
    // Small payload shapes used by Type/Value variants that don't map to
    // a single scalar (e.g. the `(field-name, ascending)` pairs in
    // `Type::Sorted` / `Index`).
    "SortKey", // (name: text, ascending: boolean)
];

/// Register the IR schema shells into `stores`.  S1: shells only — each
/// IR type gets a `known_type`, no fields/variants wired yet.  Idempotent
/// per `Stores` only in the sense that it must be called once on a fresh
/// schema; calling twice panics on the duplicate-name guard in
/// `structure()` (by design — double registration is a bug).
///
/// Returns the number of IR types registered, for the collision test.
pub fn register_ir_schema(stores: &mut Stores) -> usize {
    let mut count = 0;
    for name in IR_ENUMS {
        let qualified = format!("{IR_PREFIX}{name}");
        debug_assert!(
            !stores.has_type(&qualified),
            "IR schema enum {qualified} already registered"
        );
        stores.enumerate(&qualified);
        count += 1;
    }
    for name in IR_STRUCTS {
        let qualified = format!("{IR_PREFIX}{name}");
        debug_assert!(
            !stores.has_type(&qualified),
            "IR schema struct {qualified} already registered"
        );
        stores.structure(&qualified, 0);
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Stores;

    #[test]
    fn shells_register_without_collision() {
        let mut stores = Stores::new();
        let before = stores.types.len();
        let registered = register_ir_schema(&mut stores);
        let after = stores.types.len();

        // Every IR shell got a distinct new slot — no name collided with a
        // base type or with another IR type (a collision would have
        // panicked in structure(), or silently merged in enumerate()).
        assert_eq!(
            after - before,
            registered,
            "every IR shell must register as a new type"
        );
        assert_eq!(registered, IR_ENUMS.len() + IR_STRUCTS.len());

        // Each registered name is present and prefixed (so it cannot clash
        // with a user type).
        for name in IR_ENUMS.iter().chain(IR_STRUCTS.iter()) {
            let qualified = format!("{IR_PREFIX}{name}");
            assert!(
                stores.has_type(&qualified),
                "IR shell {qualified} should be registered"
            );
        }
    }

    #[test]
    fn prefix_is_not_a_legal_identifier() {
        // The prefix must contain a char that can't start/continue a loft
        // identifier, guaranteeing user code can never name a colliding type.
        assert!(IR_PREFIX.contains('$') && IR_PREFIX.contains('.'));
    }
}
