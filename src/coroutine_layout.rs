// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I84 — Coroutine runtime / yield codec

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

/// The native value-transport channel tag for a coroutine yield of `tp`.
///
/// ONE home for this decision: every coroutine *consumer* — the for-loop
/// (`parser/collections.rs`), manual `next()` (`parser/control.rs`) — and the
/// native producer/consumer emitters dispatch on it, so they must never
/// diverge (a duplicated copy that missed float/single/enum was #401's
/// manual-`next` E0308).  Packed into `value_size` as `(channel_tag << 8) |
/// byte_size`; the interpreter masks the tag off, native reads both bytes.
///   - `0` — legacy per-byte-size channel (i64 / i32 / bool / text / dbref)
///   - `1` — unified `next_into` (tuple)        - `2` — unified `next_into` (fn-ref)
///   - `3` — `f64::from_bits` (float)           - `4` — `f32::from_bits` (single)
///   - `5` — enum-as-`u{8·byte_size}` (NON-nullable only; a nullable enum is a
///     DbRef, so it stays on channel 0 → `next_dbref`)
///
/// 3/4/5 exist because these types' Rust value differs from the i64 transport,
/// which `byte_size` alone cannot distinguish (8 = i64 or f64, 4 = i32 or f32,
/// 1 = bool or u8 enum).
#[must_use]
pub fn channel_tag(tp: &Type) -> i32 {
    if tuple_kinds(tp).is_some() {
        1
    } else if matches!(tp, Type::Function(_, _, _)) {
        2
    } else if matches!(tp, Type::Float) {
        3
    } else if matches!(tp, Type::Single) {
        4
    } else if matches!(tp, Type::Enum(_, false, _)) {
        // Non-nullable enum = the variant index as uN.  A NULLABLE enum is a
        // DbRef (same null repr as Reference), so it must NOT come here — it
        // falls through to channel 0 / next_dbref.
        5
    } else {
        0
    }
}

/// The `OpCoroutineNext` operands for a yield type: the packed `value_size`
/// (channel tag in the high byte, byte size in the low) and the per-slot kind
/// codes a tuple channel carries as extra arguments.
///
/// One home, because the decision is made TWICE: once where a `for` over a
/// generator is lowered, and again per MONOMORPH, where a generic's
/// `iterator<T>` finally learns what `T` is (loft#1032).  A template bakes
/// these against the type VARIABLE — 12 bytes on the DbRef channel — and
/// substitution rewrites the loop variable's type without revisiting the
/// accessor it was paired with, so a scalar `T` read a 12-byte DbRef out of an
/// 8-byte slot and walked off the end of the store.  Deriving both ends from
/// this one function is what keeps the size and the channel from drifting apart
/// the way the producer and consumer lists already must not.
#[must_use]
pub fn next_operands(yield_tp: &Type) -> (i32, Vec<i32>) {
    let byte_size = i32::from(crate::variables::size(
        yield_tp,
        &crate::data::Context::Argument,
    ));
    let value_size = (channel_tag(yield_tp) << 8) | byte_size;
    let kinds = tuple_kinds(yield_tp)
        .map(|ks| ks.iter().map(|k| k.code()).collect())
        .unwrap_or_default();
    (value_size, kinds)
}

/// Total `i64` transport slots a kind list occupies (the `[i64; N]` buffer
/// size both ends allocate / write).
#[must_use]
pub fn slot_count(kinds: &[YieldSlot]) -> usize {
    kinds.iter().map(|k| k.width()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_tag_distinguishes_float_kinds_from_same_size_scalars() {
        // #401 — float/single get their own native channels (`from_bits`); the
        // other scalars must stay on channel 0.  This is exactly the distinction
        // `byte_size` alone cannot make (8 = i64 or f64, 1 = bool or u8 enum).
        assert_eq!(channel_tag(&Type::Float), 3);
        assert_eq!(channel_tag(&Type::Single), 4);
        assert_eq!(channel_tag(&Type::Boolean), 0);
        assert_eq!(channel_tag(&Type::Character), 0);
    }
}
