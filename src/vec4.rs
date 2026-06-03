// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! TEMPORARY investigation lever (@PLAN58 — nested-vector layout).
//!
//! A vector element that is itself a vector is stored as a 4-byte `u32`
//! rec-id handle.  Today the *compile-time* stride resolvers disagree on
//! that element's size — `type_def_nr(Vector)` resolves to a 16-byte
//! `FieldValue` alias (construction / pre-alloc), `type_elm(Vector<Vector>)`
//! resolves to the 8-byte bare-`vector` builtin (index reads), while the
//! *runtime* deep-copy / `OpCopyRecord` paths correctly use 4.  Four numbers
//! for one handle (16 / 8 / 4 / 4).
//!
//! This lever forces every *parser-computed* nested-vector element stride to
//! 4 (the true handle size) so a probe sweep can measure "does aligning the
//! strides close the class?" against a single toggle — BEFORE committing to a
//! resolver fix.  It is deliberately scoped:
//!
//!   * it only touches strides for elements whose type is itself a vector;
//!   * it is a no-op when the computed stride is already 4 ("leave if the
//!     same as the default") — so flat `vector<integer>` (8) /
//!     `vector<single>` (4) / struct-element vectors are untouched.
//!
//! Armed via `--vec4` on the CLI (`loft::vec4::set(true)`) or `LOFT_VEC4=1`
//! in the environment (so the test harness can arm it globally without
//! `@ARGS` plumbing).  **Remove this module when the resolver fix lands** —
//! the proper fix makes the resolvers return a size-4 type-id directly, at
//! which point clamping is dead weight.

use crate::data::Type;
use std::sync::atomic::{AtomicU8, Ordering};

// 0 = not yet resolved, 1 = off, 2 = on.
static STATE: AtomicU8 = AtomicU8::new(0);

/// Arm or disarm the lever explicitly (called from CLI `--vec4`).
pub fn set(on: bool) {
    STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
}

/// Whether the force-to-4 lever is active.  On first read with no explicit
/// `set`, falls back to the `LOFT_VEC4` environment variable.
pub fn enabled() -> bool {
    match STATE.load(Ordering::Relaxed) {
        2 => true,
        1 => false,
        _ => {
            let on = std::env::var("LOFT_VEC4").is_ok_and(|v| v == "1" || v == "on" || v == "true");
            STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Clamp a stride computed for an element of type `elem` to 4 when the lever
/// is armed and `elem` is itself a vector.  No-op otherwise.
pub fn clamp_elem(computed: i32, elem: &Type) -> i32 {
    if enabled() && computed != 4 && matches!(elem, Type::Vector(_, _)) {
        4
    } else {
        computed
    }
}

/// Clamp a stride computed for an element of the vector type `vecty` to 4 when
/// the lever is armed and that element is itself a vector (`vector<vector<…>>`).
/// `vecty` is the *vector* type (the element is its content); `RefVar` wrappers
/// are unwrapped.  No-op otherwise.
pub fn clamp_vec(computed: i32, vecty: &Type) -> i32 {
    if !enabled() || computed == 4 {
        return computed;
    }
    let inner = match vecty {
        Type::Vector(i, _) => Some(&**i),
        Type::RefVar(b) => match &**b {
            Type::Vector(i, _) => Some(&**i),
            _ => None,
        },
        _ => None,
    };
    if matches!(inner, Some(Type::Vector(_, _))) {
        4
    } else {
        computed
    }
}
