// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Layout-driven yield codec — the single source both ends of a native
//! coroutine's value channel derive from (`plans/16-coroutine-validation/`,
//! phase 02).
//!
//! A composite yield value (a tuple) is transported through the unified
//! `next_into(stores, &mut [i64])` channel as `T`'s slots flattened into
//! *transport* form: each scalar slot inline as one `i64`; each reference
//! slot as its full absolute `DbRef` packed across two `i64`s.  The producer
//! (`generation/coroutine.rs`) derives the flatten-walk from `T` directly; the
//! consumer (`generation/ops/coroutine.rs`) receives the same kind list as
//! extra `OpCoroutineNext` args (the interpreter ignores them — it reads only
//! `gen` + `value_size`).  Because *both* lists come from [`tuple_kinds`] over
//! the *same* `T`, the two ends agree by construction — no runtime shape tag,
//! no per-shape codec template.
//!
//! Text elements are the one excluded kind: a yielded `text` is a `&str` in
//! native code, not a buffer-native value, so riding the buffer requires
//! interning the string into a store (`codegen_runtime::db_from_text`) with
//! the lifetime question that entails — a separate slice, not this one.  A
//! tuple containing a text (or other unclassifiable) element returns `None`
//! from [`tuple_kinds`] and falls back to the legacy per-type channel.

use crate::data::Type;

/// One flattened transport slot of a yielded composite value.  Each variant
/// knows its transport width and is mapped to/from a small integer code so the
/// kind list can ride as `OpCoroutineNext` integer args.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YieldSlot {
    /// 64-bit integer (every loft `integer`/`long` in arg/var context is `i64`).
    Int,
    /// `character` — native repr is `i32`.
    CharI32,
    /// `boolean` — native repr is `bool`.
    Bool,
    /// `float` — native repr is `f64`, transported as its `to_bits()` image.
    F64,
    /// `single` — native repr is `f32`, transported as its `to_bits()` image.
    F32,
    /// `routine` — native repr is `u32`.
    Routine,
    /// Any `DbRef`-repr type (reference / vector / sorted / hash / index /
    /// boxed-enum / iterator).  Two slots: the full absolute `DbRef`.
    Ref,
}

impl YieldSlot {
    /// Classify a yield-element type into its transport slot, or `None` for a
    /// kind that cannot (yet) ride the unified buffer (`text`, `function`,
    /// nested tuple, …).  `None` anywhere in a tuple drops the whole tuple to
    /// the legacy channel.
    #[must_use]
    pub fn classify(tp: &Type) -> Option<YieldSlot> {
        match tp {
            Type::Integer(_) => Some(YieldSlot::Int),
            Type::Character | Type::Null => Some(YieldSlot::CharI32),
            Type::Boolean => Some(YieldSlot::Bool),
            Type::Float => Some(YieldSlot::F64),
            Type::Single => Some(YieldSlot::F32),
            Type::Routine(_) => Some(YieldSlot::Routine),
            Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Enum(_, true, _)
            | Type::Iterator(_, _) => Some(YieldSlot::Ref),
            // Text needs a store intern (lifetime); function is a (u32, DbRef)
            // pair still served by the dedicated fn-ref channel; a nested tuple
            // would need recursion the walk does not yet do.
            _ => None,
        }
    }

    /// Number of `i64` transport slots this kind occupies.
    #[must_use]
    pub fn width(self) -> usize {
        match self {
            YieldSlot::Ref => 2,
            _ => 1,
        }
    }

    /// Small integer code for transmission as an `OpCoroutineNext` arg.
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            YieldSlot::Int => 0,
            YieldSlot::CharI32 => 1,
            YieldSlot::Bool => 2,
            YieldSlot::F64 => 3,
            YieldSlot::F32 => 4,
            YieldSlot::Routine => 5,
            YieldSlot::Ref => 6,
        }
    }

    /// Inverse of [`code`](Self::code) — decode a transmitted arg back to its
    /// kind.  Unknown codes default to [`YieldSlot::Int`] (a plain `i64`
    /// passthrough) so a stale/garbled arg degrades to the identity slot
    /// rather than panicking codegen.
    #[must_use]
    pub fn from_code(code: i32) -> YieldSlot {
        match code {
            1 => YieldSlot::CharI32,
            2 => YieldSlot::Bool,
            3 => YieldSlot::F64,
            4 => YieldSlot::F32,
            5 => YieldSlot::Routine,
            6 => YieldSlot::Ref,
            _ => YieldSlot::Int,
        }
    }
}

/// The flatten-walk for a yielded type, or `None` when the type is not a
/// unified-channel composite (single scalars/refs/text keep their dedicated
/// channels; a tuple with any unclassifiable element falls back to legacy).
///
/// Only *tuples* take the unified walk today — single values already have
/// working dedicated channels (`next_i64` / `next_dbref` / `next_text`), and
/// fn-refs keep their `(u32, DbRef)` channel.  Returning `Some` here is the
/// single decision that both the producer's `is_tuple_into` and the consumer's
/// channel-1 selection share, so they never diverge.
#[must_use]
pub fn tuple_kinds(tp: &Type) -> Option<Vec<YieldSlot>> {
    let Type::Tuple(elems) = tp else {
        return None;
    };
    if elems.is_empty() {
        return None;
    }
    elems.iter().map(YieldSlot::classify).collect()
}

/// Total `i64` transport slots a kind list occupies (the `[i64; N]` buffer
/// size both ends allocate / write).
#[must_use]
pub fn slot_count(kinds: &[YieldSlot]) -> usize {
    kinds.iter().map(|k| k.width()).sum()
}
