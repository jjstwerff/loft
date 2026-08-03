// Copyright (c) 2022 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I62 — IR data model (Value/Type/Data)

//! Hold all definitions
//! Those are the combinations of types, records, and routines.
//! Many definitions can hold fields of their own, a routine
//! has parameters that behave very similarly to fields.

// These structures are rather inefficient right now, but they are the basis
// for a far more efficient database design later.
#![allow(dead_code)]

use crate::diagnostics::{Diagnostics, Level, diagnostic_format};
use crate::keys::Key;
use crate::lexer::Lexer;

// Re-export Position so external consumers (tests, integrations) can
// construct / pattern-match positions without depending on the private
// `lexer` module.
pub use crate::lexer::Position;
use crate::variables::Function;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::{Debug, Display, Formatter};
use std::io::{Result, Write};
use std::num::NonZeroU8;

static OPERATORS: &[&str] = &[
    "OpAdd", "OpMin", "OpMul", "OpDiv", "OpRem", "OpPow", "OpNot", "OpBitNot", "OpLand", "OpLor",
    "OpEor", "OpSLeft", "OpSRight", "OpEq", "OpNe", "OpLt", "OpLe", "OpGt", "OpGe", "OpAppend",
    "OpConv", "OpCast",
];

pub static I32: Type = Type::Integer(IntegerSpec::signed32());

/// Full-width integer (post-2c, 8 bytes, i64 range up to u32::MAX bound).
/// Produced by the parser when it sees `long`, `integer limit(..., > i32::MAX)`,
/// or an integer literal whose magnitude exceeds i32::MAX.  At rest: i64.
///
/// Phase 2c round 10c — replaces the former `Type::Long` variant.  The `max`
/// field can't hold full i64::MAX (it's u32), so u32::MAX is used as a
/// "wide" sentinel; all downstream code just observes "max - min >= 256"
/// and picks 8-byte storage.
pub static I64: Type = Type::Integer(IntegerSpec::wide());

/// @PLN22 Phase 2 — source numbering: `STD_SOURCE` (0) = the stdlib prelude AND
/// the home of program-global synthetic wrappers (`__tuple<…>`, `__fn_ref`,
/// `main_vector<…>`); `MAIN_SOURCE` (1) = the user's main program; 2.. = imported
/// libraries.  The main file gets its OWN source (not the prelude's) so a user
/// definition can shadow a prelude name — bare names resolve current-source-first
/// with a fallback to `STD_SOURCE`, while `std::Name` reaches the prelude.
pub const STD_SOURCE: u16 = 0;
/// See [`STD_SOURCE`] for the full source-numbering scheme.
pub const MAIN_SOURCE: u16 = 1;

/// One source's name-visibility summary, for `--show-resolution`.
pub struct SourceView {
    pub nr: u16,
    /// Display name: the file the source's definitions came from, or the library
    /// name a `use` bound it under.
    pub name: String,
    /// Definitions whose own source this is.
    pub defined: usize,
    /// Names REACHABLE from this source — its own plus every import alias.  The gap
    /// between the two columns is what an import buys, and an import fault shows up
    /// here as the gap closing.
    pub visible: usize,
}

/// One import alias: a name reachable from `into_source` whose definition lives in
/// `from_source`.  Derived from `def_names` rather than from the import log, so the
/// section reports what the table ACTUALLY holds — which is the thing that went
/// missing when a rebuild could not reproduce it.
pub struct AliasView {
    pub name: String,
    pub into_source: u16,
    pub from_source: u16,
    pub def_nr: u32,
}

/// One applied import, retained so a `def_names` rebuild can replay it.
/// `name` is `None` for a wildcard `use lib;` / `use lib::*`, or
/// `Some((name, bind))` for a selective `use lib::name [as bind]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedImport {
    pub lib_source: u16,
    pub into_source: u16,
    pub name: Option<(String, String)>,
}

/// Specification of an `integer`-family type — bounds, nullability,
/// and optional forced storage width.
///
/// `Debug` is implemented manually (instead of derived) so
/// `format!("{tp:?}")` on `Type::Integer(spec)` prints
/// `Integer(min, max, not_null)` — matching the tuple shape diagnostic
/// output was built around.  The optional `forced_size` is printed
/// only when present, as a trailing `, size(N)`.
///
/// `PartialEq` / `Eq` / `Hash` are implemented manually to ignore
/// `forced_size` — the annotation is a storage hint, not a value-type
/// difference, and the rest of the compiler uses `==` / `is_equal`
/// to match integer-valued types uniformly.  Code that cares about
/// the storage width reads `spec.forced_size` or calls
/// `spec.byte_width()` directly.
#[derive(Clone, Copy)]
pub struct IntegerSpec {
    /// Inclusive lower bound.  `i32::MIN` is reserved as the null
    /// sentinel; plain-integer templates use `i32::MIN + 1`.
    pub min: i32,
    /// Inclusive upper bound.  `u32` to allow the wide / former-`long`
    /// template to use `u32::MAX` as a "wider than i32" sentinel.
    pub max: u32,
    /// When true, the value cannot be null — frees the null sentinel
    /// and widens the usable range by 1 on narrow types.
    pub not_null: bool,
    /// When `Some(n)`, storage width is `n` bytes regardless of
    /// bounds.  Set by the parser from an integer alias's `size(N)`
    /// annotation (`i32` → `Some(4)`, `u8` → `Some(1)`).  `None`
    /// means "use the bounds-range heuristic in `byte_width()`".
    pub forced_size: Option<NonZeroU8>,
}

impl IntegerSpec {
    // ── Canonical templates (constructors) ──────────────────────────────

    /// Plain `integer` / former `long`: full i64 range, 8-byte storage
    /// via the default heuristic.  No forced size.
    pub const fn wide() -> Self {
        IntegerSpec {
            min: i32::MIN + 1,
            max: u32::MAX,
            not_null: false,
            forced_size: None,
        }
    }

    /// The I32 template — i32 bounds, nullable, no forced size.
    pub const fn signed32() -> Self {
        IntegerSpec {
            min: i32::MIN + 1,
            max: i32::MAX as u32,
            not_null: false,
            forced_size: None,
        }
    }

    /// `u8` alias — `0..=255`, forced 1-byte storage.
    pub fn u8() -> Self {
        IntegerSpec {
            min: 0,
            max: 255,
            not_null: false,
            forced_size: NonZeroU8::new(1),
        }
    }

    /// `i8` alias — `-128..=127`, forced 1-byte storage.
    pub fn i8() -> Self {
        IntegerSpec {
            min: -128,
            max: 127,
            not_null: false,
            forced_size: NonZeroU8::new(1),
        }
    }

    /// `u16` alias — `0..=65535`, forced 2-byte storage.
    pub fn u16() -> Self {
        IntegerSpec {
            min: 0,
            max: 65535,
            not_null: false,
            forced_size: NonZeroU8::new(2),
        }
    }

    /// `i16` alias — `-32768..=32767`, forced 2-byte storage.
    pub fn i16() -> Self {
        IntegerSpec {
            min: -32768,
            max: 32767,
            not_null: false,
            forced_size: NonZeroU8::new(2),
        }
    }

    /// `i32` alias — full i32 range, forced 4-byte storage.
    pub fn i32() -> Self {
        IntegerSpec {
            min: i32::MIN + 1,
            max: i32::MAX as u32,
            not_null: false,
            forced_size: NonZeroU8::new(4),
        }
    }

    /// `u32` alias — `0..=u32::MAX - 1`, wide storage.
    pub fn u32() -> Self {
        IntegerSpec {
            min: 0,
            max: u32::MAX - 1,
            not_null: false,
            forced_size: None,
        }
    }

    // ── Query methods (consolidate scattered bounds arithmetic) ─────────

    /// Storage width in bytes — honours `forced_size` first, falls back
    /// to the bounds-range heuristic otherwise.
    #[must_use]
    pub fn byte_width(&self, nullable: bool) -> u8 {
        if let Some(n) = self.forced_size {
            return n.get();
        }
        self.range_to_width(nullable)
    }

    /// Map the value RANGE to a narrow storage width (1/2/8 bytes), reserving
    /// one extra code for the null sentinel when `nullable`.
    ///
    /// The ONE home for the range→width fact (H6).  Both [`Self::byte_width`]
    /// (after `forced_size`) and `Type::size` derive from it, so a nullable
    /// narrow field's WRITE width (`Type::size`, used by `set_field_check`)
    /// cannot drift from its READ width (`byte_width`, used by `get_val`).
    /// That drift silently corrupted a nullable FULL-range narrow field's null:
    /// the two sites disagreed at `range == 256`/`257`, so the write stored the
    /// 1-byte `255` sentinel into a field the read decoded as a 2-byte Short →
    /// null read back as `max-1`.
    #[must_use]
    pub fn range_to_width(&self, nullable: bool) -> u8 {
        // #334: a nullable narrow field reserves one code as the null sentinel,
        // so it must hold `distinct + 1` codes; `range()` is the distinct count.
        let codes = self.range() + i64::from(nullable);
        if codes <= 256 {
            1
        } else if codes <= 65536 {
            2
        } else {
            8
        }
    }

    /// Usable LOWER bound for a field/element of this spec at the given
    /// nullability — the ONE home for the nullable-narrow range fact, so the read
    /// op's `min`, the write op's `min`, and the literal range-check cannot drift.
    ///
    /// Null is stored as the all-ones byte (`255`/`65535`), uniformly for every
    /// narrow type — one type-independent store/test in generated Rust.  A value
    /// decodes as `read + min`.  When a NULLABLE narrow field's range exactly
    /// FILLS its storage width, the all-ones byte would otherwise be a real value,
    /// so the field sacrifices ONE edge: SIGNED drops the BOTTOM (`min+1`, keeping
    /// the positive range — so a nullable `i8` is `-127..=127`); UNSIGNED keeps
    /// `min` and drops the top via [`Self::usable_max`].  Not-null specs, and any
    /// range that does not fill its width, use the full declared bounds.
    #[must_use]
    pub fn usable_min(&self, nullable: bool) -> i32 {
        if self.reserves_narrow_sentinel(nullable) && self.min < 0 {
            self.min + 1
        } else {
            self.min
        }
    }

    /// Usable UPPER bound — the [`Self::usable_min`] companion.  A nullable narrow
    /// UNSIGNED spec whose range fills its width drops the TOP code (`max-1`, so a
    /// nullable `u8` is `0..=254`); signed keeps `max`.
    #[must_use]
    pub fn usable_max(&self, nullable: bool) -> i64 {
        if self.reserves_narrow_sentinel(nullable) && self.min >= 0 {
            i64::from(self.max) - 1
        } else {
            i64::from(self.max)
        }
    }

    /// True when a nullable field of this spec must sacrifice one edge value: its
    /// value range exactly fills its 1- or 2-byte width, so the all-ones null byte
    /// would otherwise BE a usable value.  (Wider 4/8-byte ints reserve
    /// `i32::MIN`/`i64::MIN` for null, outside this narrow mechanism; an
    /// un-annotated `limit(...)` whose range does not fill the width already has a
    /// spare code and needs no sacrifice.)
    fn reserves_narrow_sentinel(&self, nullable: bool) -> bool {
        if !nullable {
            return false;
        }
        // Only a FIXED-width (`forced_size`) field can't widen to make room — an
        // un-annotated `limit(...)` instead widens via `range_to_width(+nullable)`,
        // so it never reaches here needing a sacrifice.  Reduce iff the range fills
        // the fixed 1- or 2-byte width (256 / 65536 codes).
        match self.forced_size.map(NonZeroU8::get) {
            Some(1) => self.range() == 256,
            Some(2) => self.range() == 65_536,
            _ => false,
        }
    }

    /// element stride for narrow vectors, matching
    /// what `typedef.rs::fill_database`'s Vector arm registers.
    /// Returns `Some(n)` for the direct-encoded widths:
    /// - 1 → `Parts::Byte` (u8 / i8)
    /// - 2 → `Parts::ShortRaw` (u16 / i16)
    /// - 4 → `Parts::Int` (i32)
    ///
    /// All three use direct raw encoding (no `+1` shift), so
    /// `vector_add`'s raw-byte copy path works across source literal
    /// vectors and destination fields without re-encoding.
    ///
    /// Returns `None` for any Type without a `forced_size` annotation
    /// (caller falls back to the default wide-integer stride) and for
    /// `forced_size` values outside the narrow gate.  `u16` struct
    /// fields continue to use `Parts::Short` (the legacy `+1` encoding)
    /// via the `alias != u32::MAX` path in `get_val` / `set_field_check`.
    ///
    /// Callers use this at compile time to emit matching `elm_size` in
    /// `OpGetVector` / `OpSetVector`, and in `get_val` to choose the
    /// right-width scalar-read opcode.  Keeping the predicate in one
    /// place avoids the narrow-read / wide-storage skew that bit the
    /// first Phase 3 attempt.
    #[must_use]
    pub fn vector_narrow_width(&self) -> Option<u8> {
        match self.forced_size?.get() {
            1 => Some(1),
            2 => Some(2),
            4 => Some(4),
            _ => None,
        }
    }

    /// True when the value range exceeds the signed-32-bit range.
    #[must_use]
    pub fn is_wide(&self) -> bool {
        self.max > i32::MAX as u32
    }

    /// Number of distinct representable values (inclusive range + 1).
    #[must_use]
    pub fn range(&self) -> i64 {
        i64::from(self.max) - i64::from(self.min) + 1
    }

    /// True when this is the I32 template (plain `integer` post-2c).
    #[must_use]
    pub fn is_signed32_template(&self) -> bool {
        self.min == i32::MIN + 1 && self.max == i32::MAX as u32
    }

    /// True when this is the wide I64 template.
    #[must_use]
    pub fn is_wide_template(&self) -> bool {
        self.min == i32::MIN + 1 && self.max == u32::MAX
    }

    /// The loft-SOURCE spelling of this spec, or `None` when it has no name of
    /// its own and must be written as the explicit `integer(min, max)` form.
    ///
    /// The ONE home for this fact: [`Type::name`] and `Display for Type` both
    /// render integers through it, so the two cannot drift.  They had — each
    /// named the signed32 template `integer` but let the WIDE template (equally
    /// `integer` in source, equally 8 bytes) fall through to the `integer(min,
    /// max)` debug form.  The REPL builds a capture function from that name, so
    /// it emitted `-> vector<integer(-2147483647, 4294967295)>`, which does not
    /// parse (#618).
    ///
    /// Narrow aliases (`u8`/`i16`/…) carry a `forced_size` and match no template
    /// here, so they keep a distinct name and a distinct storage width.
    #[must_use]
    pub fn source_name(&self) -> Option<&'static str> {
        if self.is_signed32_template() || self.is_wide_template() {
            Some("integer")
        } else if self.min == 0 && self.max == 256 {
            Some("byte")
        } else {
            None
        }
    }

    /// True when the declared range is non-negative AND runs past `i32::MAX` — a value
    /// no *signed* 4-byte slot can hold.  Such a type needs the unsigned 4-byte op pair
    /// (`OpGetInt4Raw` / `OpGetInt4Full`); the signed `OpGetInt4` sign-extends it on load
    /// and, worse, its `i32::MIN` sentinel is the legal value 2147483648.
    ///
    /// Both halves are load-bearing.  The `min >= 0` half is what excludes the WIDE
    /// (8-byte) template, which sets `max == u32::MAX` purely as a "wider than i32"
    /// marker while keeping a negative `min` — an `max > i32::MAX` test alone would
    /// misroute every plain `integer`.
    #[must_use]
    pub fn unsigned_wide(&self) -> bool {
        self.min >= 0 && self.max > i32::MAX as u32
    }
}

/// The narrow-integer storage op KIND a field of a resolved storage width uses —
/// the ONE home for the width→op decision (H4-medium).  Both the READ (`get_val`
/// → `OpGet*`) and the WRITE (`set_field_check` → `OpSet*`) emitters derive their
/// op from `NarrowIntKind::of`, so a field's read op and write op can't drift to
/// mismatched widths/encodings — the H6-class hazard one level up (a `Byte` write
/// decoded under a `Short` read).  Width itself comes from the single
/// `IntegerSpec::range_to_width`/`byte_width` home; this maps that width (+ the
/// nullable / narrow-vector context) to the matched `OpGet*`/`OpSet*` pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NarrowIntKind {
    /// 1-byte raw — `not null`, or a narrow-vector element.
    Byte,
    /// 1-byte with the reserved 255 null sentinel — a nullable struct field.
    ByteNullable,
    /// 2-byte direct encoding — a narrow-vector element (raw, no `+1`).
    ShortRaw,
    /// 2-byte `+1` sentinel encoding — a NULLABLE struct field (reserves `0`).
    Short,
    /// 2-byte direct encoding, NO sentinel — a NOT-NULL struct field, so the
    /// full 65536-value range round-trips (`Short`'s `+1` and `ShortRaw`'s
    /// `u16::MAX` both swallow the max as null).  Read via `OpGetShortFull`; the
    /// write reuses `OpSetShortRaw` (same `(val - min)` store).
    ShortFull,
    /// 4-byte raw, SIGNED — `i32` and any `size(4)` range inside `i32`'s bounds.
    /// `i32::MIN` is the null sentinel.
    Int4,
    /// 4-byte UNSIGNED with the reserved `u32::MAX` null sentinel — a nullable slot, or
    /// a narrow-vector element, of a type whose range runs past `i32::MAX` (`u32`).  The
    /// 4-byte twin of `ShortRaw`.
    Int4Raw,
    /// 4-byte UNSIGNED, NO sentinel — a NOT-NULL field of such a type, so the full 2^32
    /// round-trips.  Read via `OpGetInt4Full`; the write reuses `OpSetInt4Raw` (same
    /// unsigned store).  The 4-byte twin of `ShortFull`.
    Int4Full,
    /// 8-byte raw — the wide default.
    Int,
}

impl NarrowIntKind {
    /// Map a resolved storage `width` (1/2/4/8) to its op kind.  `nullable` = the
    /// slot reserves a null sentinel — a nullable struct field OR a nullable
    /// narrow-vector element (both encode null the same way).  `narrow_vec` = a
    /// direct-encoded narrow-vector element, which selects the RAW 2-byte read
    /// (`ShortRaw`) over the not-null field's full-range read (`ShortFull`) —
    /// but only when the slot is NOT nullable; a nullable slot always reserves a
    /// sentinel regardless of field-vs-vector.
    #[must_use]
    pub fn of(width: u8, nullable: bool, narrow_vec: bool, unsigned_wide: bool) -> Self {
        match width {
            1 if nullable => NarrowIntKind::ByteNullable,
            1 => NarrowIntKind::Byte,
            2 if nullable => NarrowIntKind::Short,
            2 if narrow_vec => NarrowIntKind::ShortRaw,
            2 => NarrowIntKind::ShortFull,
            // The 4-byte split mirrors the 2-byte one directly above, but applies ONLY
            // to a range that runs past `i32::MAX` (`unsigned_wide`).  Everything else
            // 4-byte — `i32` above all — stays on the signed `Int4` pair, whose stored
            // bytes are two's complement and are relied on beyond this module.
            4 if unsigned_wide && (nullable || narrow_vec) => NarrowIntKind::Int4Raw,
            4 if unsigned_wide => NarrowIntKind::Int4Full,
            4 => NarrowIntKind::Int4,
            _ => NarrowIntKind::Int,
        }
    }

    /// True for the kinds that reserve an in-band null sentinel (the `255` byte
    /// code / the `+1`-shifted short).  The read/write `min` derives from THIS —
    /// not a re-derived `nullable && !narrow_vec` — so a nullable narrow-vector
    /// element (also `ByteNullable`/`Short` now) decodes against the same shrunk
    /// `min` its write encodes with.
    #[must_use]
    pub fn reserves_sentinel(self) -> bool {
        matches!(self, NarrowIntKind::ByteNullable | NarrowIntKind::Short)
    }

    /// The `OpGet*` op name for this kind.
    #[must_use]
    pub fn get_op(self) -> &'static str {
        match self {
            NarrowIntKind::Byte => "OpGetByte",
            NarrowIntKind::ByteNullable => "OpGetByteNullable",
            NarrowIntKind::ShortRaw => "OpGetShortRaw",
            NarrowIntKind::Short => "OpGetShort",
            NarrowIntKind::ShortFull => "OpGetShortFull",
            NarrowIntKind::Int4 => "OpGetInt4",
            NarrowIntKind::Int4Raw => "OpGetInt4Raw",
            NarrowIntKind::Int4Full => "OpGetInt4Full",
            NarrowIntKind::Int => "OpGetInt",
        }
    }

    /// The `OpSet*` op name for this kind — the matched write twin of [`Self::get_op`].
    #[must_use]
    pub fn set_op(self) -> &'static str {
        match self {
            NarrowIntKind::Byte => "OpSetByte",
            NarrowIntKind::ByteNullable => "OpSetByteNullable",
            NarrowIntKind::ShortRaw => "OpSetShortRaw",
            NarrowIntKind::Short => "OpSetShort",
            // not-null 2-byte reuses the raw `(val - min)` store; only the READ
            // differs (`OpGetShortFull`, no sentinel decode).
            NarrowIntKind::ShortFull => "OpSetShortRaw",
            NarrowIntKind::Int4 => "OpSetInt4",
            // not-null 4-byte reuses the raw unsigned store; only the READ differs
            // (`OpGetInt4Full`, no sentinel decode) — as `ShortFull` does one width down.
            NarrowIntKind::Int4Raw | NarrowIntKind::Int4Full => "OpSetInt4Raw",
            NarrowIntKind::Int => "OpSetInt",
        }
    }

    /// True when the 1/2-byte ops take a trailing `min` arg; the 4/8-byte ops
    /// (`Int4`/`Int`) do not.
    #[must_use]
    pub fn takes_min(self) -> bool {
        matches!(
            self,
            NarrowIntKind::Byte
                | NarrowIntKind::ByteNullable
                | NarrowIntKind::ShortRaw
                | NarrowIntKind::Short
                | NarrowIntKind::ShortFull
        )
    }
}

impl Debug for IntegerSpec {
    /// Matches the `Integer(min, max, not_null)` tuple shape so the
    /// `Display for Type` fallback (`format!("{self:?}").to_lowercase()`)
    /// and any other `{:?}` consumer produces the expected output.
    /// A `forced_size` annotation, when present, is appended as `, size(N)`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}, {}, {}", self.min, self.max, self.not_null)?;
        if let Some(n) = self.forced_size {
            write!(f, ", size({})", n.get())?;
        }
        Ok(())
    }
}

impl PartialEq for IntegerSpec {
    /// Equality ignores `forced_size`: the annotation is a storage
    /// hint, not a semantic difference.  `vector<i32>` and
    /// `vector<integer limit(-2147483647, 2147483647)>` have the same
    /// value-type even though the former is stored in 4 bytes.  Code
    /// that needs the storage distinction reads `spec.forced_size`
    /// or calls `spec.byte_width()`.
    fn eq(&self, other: &Self) -> bool {
        self.min == other.min && self.max == other.max && self.not_null == other.not_null
    }
}

impl Eq for IntegerSpec {}

impl std::hash::Hash for IntegerSpec {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.min.hash(state);
        self.max.hash(state);
        self.not_null.hash(state);
        // forced_size intentionally omitted — see PartialEq.
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Block {
    pub name: &'static str,
    pub operators: Vec<Value>,
    pub result: Type,
    pub scope: u16,
    /// Bytes to pre-claim for small variables (≤ 8 B) at block entry via `OpReserveFrame`.
    /// Computed by `assign_slots`; 0 until then.
    pub var_size: u16,
}

/// A value that can be assigned to attributes on a definition of instance
#[derive(Debug, PartialEq, Clone)]
pub enum Value {
    Null,
    /// Line number inside the source file
    Line(u32),
    /// Source span wrapper (plan-07 phase 1, decision A).
    /// Wraps a single fault-prone IR node with its source `Position`.
    /// Walkers should treat this as a transparent passthrough that
    /// updates the walker's `current_span` so diagnostics raised
    /// while recursing into `inner` carry the correct file:line:col.
    /// Codegen mirrors the entry pc into `Definition.source_spans`
    /// so phase 3's pc→span lookup can run.
    ///
    /// Both `Position` and the inner `Value` are heap-boxed together
    /// to keep `size_of::<Value>()` at 32 bytes — `Position` carries a
    /// `String` and would otherwise grow the enum by 8 bytes,
    /// affecting every `Vec<Value>` allocation in the IR.
    Span(Box<(Position, Value)>),
    Int(i32),
    /// Enum value and database type
    Enum(u8, u16),
    Boolean(bool),
    /// A range
    Float(f64),
    Long(i64),
    Single(f32),
    Text(String),
    /// Call an outside routine with values.
    Call(u32, Vec<Value>),
    /// Call a function through a runtime function reference stored in a local variable.
    CallRef(u16, Vec<Value>),
    /// Call a closure function that allows access to the original stack
    // CCall(Box<Value>, Vec<Value>),
    /// Block with steps and last variable claimed before it.
    Block(Box<Block>),
    /// A block that will be inserted in the outer block and thus not form its own scope.
    /// A block that will be inserted in the outer block and thus not form its own scope.
    Insert(Vec<Value>),
    /// Read variable or parameter from stack (nr relative to current function start).
    Var(u16),
    /// Set a variable with an expressions
    Set(u16, Box<Value>),
    // / Read a variable from the closure stack instead of the current function
    // CVar(u32),
    // / Set a closure variable outside the current function
    // CSet(u32, Box<Value>),
    /// Return from a routine with optionally a Value
    Return(Box<Value>),
    /// Break out of the n-th loop
    Break(u16),
    /// Break out of the n-th loop with a value
    BreakWith(u16, Box<Value>),
    /// Continue the n-th loop
    Continue(u16),
    /// Conditional statement
    If(Box<Value>, Box<Value>, Box<Value>),
    /// Loop through the block till Break is encountered
    Loop(Box<Block>),
    // / Closure function value with a def-nr and
    // Closure(u32, u32),
    /// Drop the returned value of a call
    Drop(Box<Value>),
    /// An iterator (name, create, next, `extra_init`)
    /// `extra_init` is `Value::Null` for non-text loops, or `v_set(index_var`, 0) for text loops.
    Iter(u16, Box<Value>, Box<Value>, Box<Value>),
    /// Key structure
    Keys(Vec<Key>),
    /// T1.2: Tuple literal — elements are evaluated left-to-right onto contiguous stack slots.
    Tuple(Vec<Value>),
    // T1.4: Read element idx of tuple variable var_nr.
    TupleGet(u16, u16),
    // T1.4: Write value to element idx of tuple variable var_nr.
    TuplePut(u16, u16, Box<Value>),
    // CO1.3c: Yield a value from a generator function.
    Yield(Box<Value>), // @F34 — coroutines / generators (yield, yield from)
    // Construct a 16-byte fn-ref on the stack: push d_nr (4B via OpConstInt)
    // then push the closure DbRef (12B via OpVarRef of clos_var_nr). No new opcode.
    FnRef(i32, u16, Box<Type>), // @F23 — function references as first-class values
    // P215: project the d_nr (i64, first 8B of slot) from a fn-ref Var.
    // Used when writing a captured non-capturing fn-ref into a 4B-int
    // field (closure record) — the source's closure DbRef is the null
    // sentinel and is dropped, so only the d_nr needs writing.
    // - Interp codegen: `OpVarInt(var_pos)` — the dispatcher
    //   (`fill.rs::var_int`) reads 8B from the slot regardless of the
    //   variable's declared type.
    // - Native codegen: `(var_<name>.0 as i64)` — projects the u32
    //   d_nr from the (u32, DbRef) tuple and widens to i64.
    FnRefDnr(u16),
    /// Parallel { arm1; arm2; } — each arm runs concurrently.
    Parallel(Vec<Value>),
    /// Plan-06 PRIORITY.md spine step 3 — fused for-par IR shape
    /// (DESIGN.md D7).  Captures the streaming-only par construct
    /// `for x in input par(r=worker(x), threads) { body }` without
    /// materialising a result vector.  Heap-boxed because the
    /// variant carries 5 child `Value`s plus 3 small fields and
    /// would otherwise grow `size_of::<Value>()` past the 32-byte
    /// budget that every `Vec<Value>` allocation pays for.
    ///
    /// Field layout (mirrors the design doc's struct):
    ///   * `input`  — expression of type `vector<T>`
    ///   * `x_var`  — variable bound to each input element
    ///   * `r_var`  — bound to worker result; `u16::MAX` when the
    ///     stitch policy is Discard (no result name)
    ///   * `worker` — call expression evaluated by workers per row
    ///   * `threads` — expression of type integer
    ///   * `body`   — sequential block on the main thread
    ///   * `stitch_id` — `Stitch` policy: 0=Concat, 1=Discard,
    ///     2=Reduce, 3=Queue.  Numeric here so `data.rs` does not
    ///     need to import the typed `Stitch` enum from `parallel.rs`
    ///     (avoids a cross-module dependency); codegen converts to
    ///     the typed enum via the same id mapping.  Reduce/Queue
    ///     policy payloads (fold_fn, capacity) come from companion
    ///     fields added when their runtimes land in spine steps 4+9.
    ///
    /// Currently dead code — spine step 3a (this commit) lands the
    /// variant + walker arms only.  Steps 3b (codegen) and 3c
    /// (parser detection) follow.
    ParFor(Box<ParForBody>),
    /// Plan 09 phase 00 step 0.7 — codegen-internal "raw expression"
    /// passthrough.
    ///
    /// Holds a pre-emitted Rust expression string that emits verbatim
    /// when reached by `output_code_inner`.  Used exclusively by
    /// fn-ref dispatch in `src/generation/emit.rs` to inject
    /// pre-evaluated `let _farg_N` bindings as synthetic arg `Value`s
    /// into per-arm calls that route through `emit_op` /
    /// `output_call_user_fn`.
    ///
    /// **Not produced by the parser.**  Created only during native
    /// code generation, lives only on the codegen stack, and is
    /// consumed by `output_code_inner` which writes its string
    /// directly to the writer with no further transformation.  Other
    /// walkers (scopes.rs liveness, pre_eval.rs, parser passes) never
    /// see this variant — the catch-all `_ =>` arms in those walkers
    /// suffice as a defensive default.
    RawExpr(String),
}

/// Plan-06 PRIORITY.md spine step 3 — payload for `Value::ParFor`.
/// Heap-boxed so the variant fits the 32-byte budget.
#[derive(Debug, PartialEq, Clone)]
pub struct ParForBody {
    pub input: Value,
    pub x_var: u16,
    pub r_var: u16,
    pub worker: Value,
    pub threads: Value,
    pub body: Value,
    pub stitch_id: u8,
}

#[allow(dead_code)]
impl Value {
    #[must_use]
    pub fn str(s: &str) -> Value {
        Value::Text(s.to_string())
    }

    /// @PLN88 keystone — turn a user integer VALUE into an IR constant with the
    /// compact-or-wide encoding: a value that fits i32 stays the compact 4-byte
    /// `Value::Int`, a larger one routes to `Value::Long` (i64) instead of silently
    /// truncating via `as i32`.  This is "i32 in the IR, outside the i64": the runtime
    /// is already i64 and every reader widens `Int` via `i64::from`, so only the
    /// CONSTRUCTION of a user value can truncate — use this at every such site
    /// (const-fold results, `float as integer`, computed bounds/defaults).
    ///
    /// Do NOT use it for compiler METADATA emitted as `Value::Int` (def / type / field
    /// numbers, sizes, line numbers, char codes): those are inherently small and stay
    /// the compact `Int` — widening them is the IR bloat @PLN88 deliberately avoids.
    /// Mirrors the lexer's `ret_number` literal selection (`src/lexer.rs`).
    #[must_use]
    pub fn int_const(v: i64) -> Value {
        match i32::try_from(v) {
            Ok(n) => Value::Int(n),
            Err(_) => Value::Long(v),
        }
    }

    #[must_use]
    pub fn is_op(&self, op: u32) -> bool {
        if let Value::Call(func, _) = self {
            return *func == op;
        }
        false
    }

    /// Plan-07 phase 1 — wrap a fault-prone IR construction with its
    /// source position.  Walkers see `Span(box (pos, inner))` and
    /// recurse into `inner` while remembering `pos` as the
    /// `current_span` for any diagnostic raised inside.  Codegen
    /// records the entry pc → pos mapping in `Definition.source_spans`
    /// (phase-1 step 1.D), which phase 3's pc→span table consumes.
    ///
    /// Caller must capture `pos` from the lexer at the *exact* token
    /// the diagnostic should point to (e.g. the `/` of a binary
    /// division), not at whatever the lexer drifted to while parsing
    /// the inner construct.
    #[must_use]
    pub fn with_span(pos: Position, inner: Value) -> Value {
        Value::Span(Box::new((pos, inner)))
    }

    /// Read a `Value::Span`'s `Position`.  Returns `None` for any
    /// other variant.  Convenience for codegen / renderer that asks
    /// "what's the span of the IR node I'm about to lower?".
    #[must_use]
    pub fn span_pos(&self) -> Option<&Position> {
        if let Value::Span(b) = self {
            Some(&b.0)
        } else {
            None
        }
    }

    /// Plan-07 phase 1, step 1.B.0 — see through any number of nested
    /// `Value::Span` wrappers and return the inner non-Span node.
    ///
    /// Every second-pass site that pattern-matches a specific Value
    /// variant (`if let Value::Call(...) = code`, etc.) must call
    /// `code.unspan()` first.  Without this, the per-site wraps in
    /// 1.B.1+ silently break optimisations that rely on the unwrapped
    /// shape.  See the plan doc for the audit list.
    ///
    /// `Span` may nest in principle (e.g. a nested struct field
    /// access wrapped at multiple parser layers); the recursion
    /// flattens any depth.  In practice depth is 1.
    #[must_use]
    pub fn unspan(&self) -> &Value {
        if let Value::Span(b) = self {
            b.1.unspan()
        } else {
            self
        }
    }

    /// Mutable counterpart of `unspan()` — returns `&mut` to the inner
    /// non-Span node, for sites that need to rewrite the wrapped value
    /// in place (e.g. compound-assignment LHS rewrites).
    #[must_use]
    pub fn unspan_mut(&mut self) -> &mut Value {
        if let Value::Span(b) = self {
            b.1.unspan_mut()
        } else {
            self
        }
    }

    /// Pass-2 keystone (STABILITY_PASS2.md): the ONE place that knows
    /// `Value`'s tree shape.  Calls `f` once per direct child expression.
    /// Every traversal derives from this — the match is exhaustive on
    /// purpose (no wildcard), so a new `Value` variant forces a decision
    /// here and every walker inherits the edge.
    pub fn for_each_child(&self, f: &mut impl FnMut(&Value)) {
        match self {
            Value::Span(b) => f(&b.1),
            Value::Call(_, items)
            | Value::CallRef(_, items)
            | Value::Insert(items)
            | Value::Tuple(items)
            | Value::Parallel(items) => items.iter().for_each(&mut *f),
            Value::Block(bl) | Value::Loop(bl) => bl.operators.iter().for_each(&mut *f),
            Value::Set(_, inner)
            | Value::Return(inner)
            | Value::BreakWith(_, inner)
            | Value::Drop(inner)
            | Value::Yield(inner)
            | Value::TuplePut(_, _, inner) => f(inner),
            Value::If(c, t, e) => {
                f(c);
                f(t);
                f(e);
            }
            Value::Iter(_, a, b, c) => {
                f(a);
                f(b);
                f(c);
            }
            Value::ParFor(b) => {
                f(&b.input);
                f(&b.worker);
                f(&b.threads);
                f(&b.body);
            }
            // Leaves — no child expressions.
            Value::RawExpr(_)
            | Value::Null
            | Value::Line(_)
            | Value::Int(_)
            | Value::Enum(_, _)
            | Value::Boolean(_)
            | Value::Float(_)
            | Value::Long(_)
            | Value::Single(_)
            | Value::Text(_)
            | Value::Var(_)
            | Value::Break(_)
            | Value::Continue(_)
            | Value::Keys(_)
            | Value::TupleGet(_, _)
            | Value::FnRef(_, _, _)
            | Value::FnRefDnr(_) => {}
        }
    }

    /// Pre-order search: does `pred` hold on this node or any descendant?
    /// `Span` wrappers are transparent — `pred` never sees them, so node
    /// predicates match on the bare variants.
    pub fn any_node(&self, pred: &mut impl FnMut(&Value) -> bool) -> bool {
        if let Value::Span(b) = self {
            return b.1.any_node(pred);
        }
        if pred(self) {
            return true;
        }
        let mut found = false;
        self.for_each_child(&mut |c| {
            if !found && c.any_node(pred) {
                found = true;
            }
        });
        found
    }

    /// Mutable twin of [`Value::for_each_child`] — the same exhaustive
    /// child-edge enumeration, kept adjacent so the two matches cannot
    /// drift (a new variant breaks both).
    pub fn for_each_child_mut(&mut self, f: &mut impl FnMut(&mut Value)) {
        match self {
            Value::Span(b) => f(&mut b.1),
            Value::Call(_, items)
            | Value::CallRef(_, items)
            | Value::Insert(items)
            | Value::Tuple(items)
            | Value::Parallel(items) => items.iter_mut().for_each(&mut *f),
            Value::Block(bl) | Value::Loop(bl) => bl.operators.iter_mut().for_each(&mut *f),
            Value::Set(_, inner)
            | Value::Return(inner)
            | Value::BreakWith(_, inner)
            | Value::Drop(inner)
            | Value::Yield(inner)
            | Value::TuplePut(_, _, inner) => f(inner),
            Value::If(c, t, e) => {
                f(c);
                f(t);
                f(e);
            }
            Value::Iter(_, a, b, c) => {
                f(a);
                f(b);
                f(c);
            }
            Value::ParFor(b) => {
                f(&mut b.input);
                f(&mut b.worker);
                f(&mut b.threads);
                f(&mut b.body);
            }
            // Leaves — no child expressions.
            Value::RawExpr(_)
            | Value::Null
            | Value::Line(_)
            | Value::Int(_)
            | Value::Enum(_, _)
            | Value::Boolean(_)
            | Value::Float(_)
            | Value::Long(_)
            | Value::Single(_)
            | Value::Text(_)
            | Value::Var(_)
            | Value::Break(_)
            | Value::Continue(_)
            | Value::Keys(_)
            | Value::TupleGet(_, _)
            | Value::FnRef(_, _, _)
            | Value::FnRefDnr(_) => {}
        }
    }

    /// Pre-order mutable visitor: calls `f` on this node, then on every
    /// descendant of whatever the node is AFTER `f` ran (so a node `f`
    /// replaces wholesale gets its replacement's children visited).
    /// Unlike the read-side walkers, `f` SEES `Span` nodes (it may want to
    /// replace them); descent still enters the wrapped value.
    pub fn map_nodes(&mut self, f: &mut impl FnMut(&mut Value)) {
        f(self);
        self.for_each_child_mut(&mut |c| c.map_nodes(f));
    }

    /// Pre-order visitor: calls `f` on this node and every descendant.
    /// `Span` wrappers are transparent, matching [`Value::any_node`].
    pub fn walk(&self, f: &mut impl FnMut(&Value)) {
        if let Value::Span(b) = self {
            return b.1.walk(f);
        }
        f(self);
        self.for_each_child(&mut |c| c.walk(f));
    }

    /// Does this expression read (or name) variable `v` anywhere?  The ONE
    /// reads-var predicate — it replaced `scopes::value_reads_var` and two
    /// `value_mentions_var` copies whose hand-rolled descents had drifted
    /// apart (#330's predicate hole).  Deliberately conservative: `Set` /
    /// `TuplePut` targets count, a `FnRef`'s closure work var counts, a
    /// `CallRef`'s callee var counts — every consumer fails safe on a
    /// too-wide answer (a suppressed free), never on a too-narrow one (a
    /// premature free of a store the RHS still reads).
    pub fn reads_var(&self, v: u16) -> bool {
        self.any_node(&mut |n| match n {
            Value::Var(x)
            | Value::Set(x, _)
            | Value::TupleGet(x, _)
            | Value::TuplePut(x, _, _)
            | Value::FnRefDnr(x)
            | Value::CallRef(x, _)
            | Value::Iter(x, _, _, _) => *x == v,
            Value::FnRef(_, w, _) => *w == v,
            Value::ParFor(b) => b.x_var == v || b.r_var == v,
            _ => false,
        })
    }

    /// The TAIL expression of this value: descends `Span` wrappers and the
    /// last operator of `Block` / `Insert` sequences (skipping trailing
    /// `Line` position markers, which are never values).  Stops at `If` —
    /// branch policy belongs to the caller.  Pass-3 keystone: the tail
    /// walkers in scopes / emit / control each hand-rolled this descent
    /// with drifted arm sets (one missed `Insert`, two missed `Span`, none
    /// skipped `Line`).
    pub fn tail(&self) -> &Value {
        match self {
            Value::Span(b) => b.1.tail(),
            Value::Block(bl) => bl
                .operators
                .iter()
                .rev()
                .find(|v| !matches!(v.unspan(), Value::Line(_)))
                .map_or(self, Value::tail),
            Value::Insert(ops) => ops
                .iter()
                .rev()
                .find(|v| !matches!(v.unspan(), Value::Line(_)))
                .map_or(self, Value::tail),
            _ => self,
        }
    }

    /// The frame variable an accessor expression is rooted at: `Var(h)`
    /// itself, or the first argument of an accessor chain
    /// (`OpGetField(OpGetField(h, …), …)`).  `None` for shapes with no
    /// single var root (literals, fresh allocations inside blocks).
    pub fn base_var(&self) -> Option<u16> {
        match self.unspan() {
            Value::Var(v) => Some(*v),
            Value::Call(_, args) => args.first().and_then(Value::base_var),
            _ => None,
        }
    }

    /// Is this a PLACE that produces a heap `DbRef` — a variable, or an
    /// accessor chain (`OpGetField` / `OpGetVector` / `OpVectorRef`) rooted at
    /// one?  Such an expression is pure address arithmetic over a store
    /// somebody else owns: it allocates nothing and delivers into no return
    /// buffer, so its value lives only on the evaluation stack.  The `&`
    /// argument check and the loft#754 tail hoist both ask this question.
    pub fn is_place_read(&self, data: &Data) -> bool {
        match self.unspan() {
            Value::Var(_) => true,
            Value::Call(d_nr, args) => {
                let name = data.def(*d_nr).name();
                (name == "OpGetField" || name == "OpGetVector" || name == "OpVectorRef")
                    && args.first().is_some_and(|a| a.is_place_read(data))
            }
            _ => false,
        }
    }

    /// @PLN87 P2.2 — the terminal/result variable of a value expression: a bare
    /// `Var`, the target of a tail `Set`, or the last non-`Line` operator of a
    /// `Block`/`Insert` (recursively, unspanning at each level).  Used to
    /// recognise a TRANSFERRED construction temp — `o = Obj{..}` lowers to a
    /// `Block` whose result is the owned `__ref_N` — at the RefVar-set site.
    /// `u16::MAX` when there is no single terminal var (the SAFE default: the
    /// caller then skips the displaced-free, never frees the wrong store).
    #[must_use]
    pub fn result_var(&self) -> u16 {
        let last_non_line = |ops: &[Value]| -> u16 {
            ops.iter()
                .rev()
                .find(|o| !matches!(o.unspan(), Value::Line(_)))
                .map_or(u16::MAX, Value::result_var)
        };
        match self.unspan() {
            Value::Var(v) => *v,
            Value::Set(v, _) => *v,
            Value::Block(bl) => last_non_line(&bl.operators),
            Value::Insert(ops) => last_non_line(ops),
            Value::Return(inner) | Value::Drop(inner) => inner.result_var(),
            _ => u16::MAX,
        }
    }
}

#[must_use]
pub fn to_default(tp: &Type, data: &Data) -> Value {
    match tp {
        Type::Boolean => Value::Boolean(false),
        Type::Enum(tp, _, _) => Value::Enum(0, data.def(*tp).known_type),
        Type::Integer(_)
        | Type::Vector(_, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _) => Value::Int(0),
        Type::Single => Value::Single(0.0),
        Type::Float => Value::Float(0.0),
        // @PLN116 — `character`'s zero is the NUL codepoint `'\0'`, stored (like every
        // character value) as a 4-byte `Value::Int(0)`.  Previously `character` fell to
        // the `_ => Value::Null` arm, so an omitted `character` field defaulted to `null`
        // (interpret) / `()` (native `() as u32` E0605) instead of `'\0'` — the runtime
        // `set_default_value` (content-type 6 → codepoint 0) already produced `'\0'`, so
        // this aligns the compile-time builder with the store-init path.
        Type::Character => Value::Int(0),
        Type::Text(_) => Value::Text(String::new()),
        // Plan-06 phase 4d (P193): null fn-ref defaults are needed
        // when a struct with a fn-ref field is default-initialised.
        // `Value::FnRef(0, u16::MAX, …)` produces 8B `OpConstInt(0)`
        // + 12B null DbRef in the interpreter, and `(0_u32,
        // null_DbRef)` natively — both shapes the downstream
        // `set_field_check::Type::Function` arm reduces to a 4-byte
        // d_nr=0 storage write.
        Type::Function(_, _, _) => Value::FnRef(0, u16::MAX, Box::new(tp.clone())),
        // Plan-06 phase 4d (P193): tuple struct fields default to
        // per-element defaults so `Pair {}` with `v: (text,
        // integer)` lands as `("", 0)`.  Recurses through nested
        // tuples and other compound element types.
        Type::Tuple(elems) => Value::Tuple(elems.iter().map(|e| to_default(e, data)).collect()),
        // @PLN25 — an `Optional(τ)` FIELD with no explicit default takes its base type's
        // default (base-zero: `integer? → 0`, `bool? → false`, `text? → ""`), NOT a bare
        // `null`. `null` would fall to the `_` arm below and render as native `(())` (unit)
        // into the scalar's slot (E0308) — and base-zero is the settled design call (the
        // nullable field is still writable to null via an explicit `= null`). Inert gate-OFF
        // (no `Optional` is constructed there).
        Type::Optional(inner) => to_default(inner, data),
        _ => Value::Null,
    }
}

/// The debug-only address-space tag carried by [`Deps`] — which index
/// space its entries live in.  See [DEPS_INVENTORY](../doc/claude/DEPS_INVENTORY.md).
#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DepSpace {
    /// Not yet classified (codec round-trips, tests, legacy plumbing).
    /// Asserts nothing.
    #[default]
    Unknown,
    /// Entries are caller FRAME variable numbers (variable-table and
    /// expression types).
    Frame,
    /// Entries are callee ATTRIBUTE indices (`Definition.returned`,
    /// attribute typedefs).
    Attr,
}

/// A decoded def-space dep entry (H2 step 5,
/// [DEPS_INVENTORY](../doc/claude/DEPS_INVENTORY.md)): an attribute index,
/// or a callee-internal FRAME-var note tagged in-band with
/// [`Deps::CALLEE_FRAME_BIT`].  Being a VALUE tag (not a debug-only field)
/// it survives the IR codecs and the startup cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepEntry {
    /// Callee attribute index — "the result borrows from parameter N".
    Attr(u16),
    /// Callee frame var — "the result carries this callee-LOCAL's store"
    /// (the closure work var of a returned fn-ref).  Meaningless to
    /// callers (`resolve_deps` skips it); read only inside the defining
    /// function (`scopes` keeps the local alive through the return).
    CalleeFrame(u16),
}

impl DepEntry {
    /// Decode one raw def-space dep value.  THE single decoder — used by
    /// [`Deps::entries`] and by readers that only have flattened values
    /// (e.g. tuple-union deps from [`Type::depend`]).  The `u16::MAX`
    /// markers decode as `Attr(u16::MAX)` (their readers check
    /// [`Deps::is_pointer_marker`] before iterating).
    #[must_use]
    pub fn decode(raw: u16) -> DepEntry {
        if raw != u16::MAX && raw & Deps::CALLEE_FRAME_BIT != 0 {
            DepEntry::CalleeFrame(raw & !Deps::CALLEE_FRAME_BIT)
        } else {
            DepEntry::Attr(raw)
        }
    }
}

/// H2 ([STABILITY_HOTSPOTS](../doc/claude/STABILITY_HOTSPOTS.md) /
/// [DEPS_INVENTORY](../doc/claude/DEPS_INVENTORY.md)): the dependency list
/// carried by heap-backed `Type` variants.  An entry is an index into one
/// of TWO address spaces depending on where the `Type` lives — caller
/// frame VAR numbers (variable-table / expression types) or callee ATTR
/// indices (`Definition.returned` / attr typedefs) — plus marker
/// overloads (`u16::MAX` pointer/share markers, `[self]` ownership,
/// empty = owned).  `resolve_deps` (def→frame, at call sites) and
/// `ref_return` (frame→def, at promotion) are the only legitimate space
/// converters.
///
/// Construction goes through the NAMED constructors so every creation
/// site states its meaning; reads go through `Deref<[Vec<u16>]>` (the
/// space-agnostic ones) or the space-asserting accessors.  In debug
/// builds each value carries its [`DepSpace`]; the tag is excluded from
/// equality and absent in release builds.
#[derive(Clone, Debug, Default)]
pub struct Deps {
    items: Vec<u16>,
    #[cfg(debug_assertions)]
    space: DepSpace,
}

impl PartialEq for Deps {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl std::ops::Deref for Deps {
    type Target = Vec<u16>;
    fn deref(&self) -> &Vec<u16> {
        &self.items
    }
}

impl std::ops::DerefMut for Deps {
    /// Mutation inherits the construction-site space tag.
    fn deref_mut(&mut self) -> &mut Vec<u16> {
        &mut self.items
    }
}

impl<'a> IntoIterator for &'a Deps {
    type Item = &'a u16;
    type IntoIter = std::slice::Iter<'a, u16>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl IntoIterator for Deps {
    type Item = u16;
    type IntoIter = std::vec::IntoIter<u16>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

#[cfg(debug_assertions)]
macro_rules! deps_new {
    ($items:expr, $space:expr) => {
        Deps {
            items: $items,
            space: $space,
        }
    };
}
#[cfg(not(debug_assertions))]
macro_rules! deps_new {
    ($items:expr, $space:expr) => {{
        let _ = $space; // tag only exists in debug builds
        Deps { items: $items }
    }};
}

#[allow(dead_code)]
impl Deps {
    /// OWNED — no borrow, either space.  The most load-bearing convention
    /// in the codebase (`is_empty()` gates the free logic).
    #[must_use]
    pub fn none() -> Deps {
        #[cfg(debug_assertions)]
        {
            deps_new!(Vec::new(), DepSpace::Unknown)
        }
        #[cfg(not(debug_assertions))]
        {
            deps_new!(Vec::new(), ())
        }
    }

    /// Caller FRAME variable numbers.
    #[must_use]
    pub fn frame(items: Vec<u16>) -> Deps {
        #[cfg(debug_assertions)]
        {
            deps_new!(items, DepSpace::Frame)
        }
        #[cfg(not(debug_assertions))]
        {
            deps_new!(items, ())
        }
    }

    /// One caller frame variable.
    #[must_use]
    pub fn frame1(v: u16) -> Deps {
        Self::frame(vec![v])
    }

    /// Callee ATTRIBUTE indices (`Definition.returned` / attr typedefs).
    #[must_use]
    pub fn attrs(items: Vec<u16>) -> Deps {
        #[cfg(debug_assertions)]
        {
            deps_new!(items, DepSpace::Attr)
        }
        #[cfg(not(debug_assertions))]
        {
            deps_new!(items, ())
        }
    }

    /// #328 — the `reference<T>` POINTER-field marker (struct-field
    /// typedefs; def space).
    #[must_use]
    pub fn pointer_marker() -> Deps {
        Self::attrs(vec![u16::MAX])
    }

    /// Closure auto-Reference share-sentinel (`vectors.rs` — a stand-in
    /// for a not-yet-known OUTER var; frame space).  Says TWO things: the
    /// attribute stores a 12-byte `DbRef` rather than inline bytes, AND the
    /// closure record ADOPTS the captured store — `free_named`'s cascade
    /// reclaims it when the record dies (which is what lets an escaping
    /// factory closure outlive the frame that minted the capture, #323).
    #[must_use]
    pub fn share_sentinel() -> Deps {
        Self::frame(vec![u16::MAX])
    }

    /// The BORROWED half of [`Deps::share_sentinel`] (#682): identical 12-byte
    /// `DbRef` storage, but the captured store stays owned by whoever owned it
    /// — a parameter's caller, or the vector a projection local views into — so
    /// the record's cascade must NOT reclaim it.  Adoption is only available
    /// when the defining frame owned the capture in the first place; without
    /// this second marker the one sentinel had to mean both, and a captured
    /// parameter was freed out from under its caller.
    #[must_use]
    pub fn borrowed_share_sentinel() -> Deps {
        Self::frame(vec![u16::MAX, u16::MAX])
    }

    /// Does this closure-record attribute carry the BORROWED share marker
    /// ([`Deps::borrowed_share_sentinel`]) rather than the adopting one?  The
    /// two layout producers — `typedef::fill_database` (interpreter) and
    /// `generation` (native) — ask this to pick the storage shape, so both
    /// backends agree on which captures the cascade may free.
    #[must_use]
    pub fn is_borrowed_share(&self) -> bool {
        self.items == [u16::MAX, u16::MAX]
    }

    /// In-band per-entry tag (H2 step 5): marks a callee-internal
    /// FRAME-var note inside a def-space list.  No definition has 0x8000
    /// attributes, so a tagged value can never read as a real attr index;
    /// the constructor rejects var 0x7FFF so the `u16::MAX`
    /// pointer/share markers stay unambiguous.
    const CALLEE_FRAME_BIT: u16 = 0x8000;

    /// One callee-internal frame-var note for a def-space list — the
    /// closure-factory shape: a returned fn-ref carries its closure work
    /// var so the defining function's scope analysis keeps the record
    /// alive through the return (the ONLY writer is the lambda
    /// propagation in `parser/vectors.rs`).  Decode with [`Deps::entries`].
    ///
    /// # Panics
    /// When `v >= 0x7FFF` — the tagged value would collide with the
    /// `u16::MAX` pointer/share markers (no real frame ever holds 32 767
    /// variables).
    #[must_use]
    pub fn callee_frame1(v: u16) -> Deps {
        assert!(
            v < Self::CALLEE_FRAME_BIT - 1,
            "callee-frame dep var {v} would collide with the u16::MAX markers"
        );
        let items = vec![Self::CALLEE_FRAME_BIT | v];
        #[cfg(debug_assertions)]
        {
            deps_new!(items, DepSpace::Attr)
        }
        #[cfg(not(debug_assertions))]
        {
            deps_new!(items, ())
        }
    }

    /// Decode a DEF-space list per entry — attr indices vs tagged
    /// callee-frame notes (see [`DepEntry::decode`]).
    pub fn entries(&self) -> impl Iterator<Item = DepEntry> + '_ {
        self.items.iter().map(|&it| DepEntry::decode(it))
    }

    /// Unclassified — codec round-trips, tests, plumbing that merely
    /// copies entries through.  Asserts nothing in debug builds.
    #[must_use]
    pub fn unknown(items: Vec<u16>) -> Deps {
        #[cfg(debug_assertions)]
        {
            deps_new!(items, DepSpace::Unknown)
        }
        #[cfg(not(debug_assertions))]
        {
            deps_new!(items, ())
        }
    }

    /// The entries read as caller frame variable numbers.
    /// Debug builds panic when the value is tagged as attr indices —
    /// the cross-space read #306 was made of.
    #[must_use]
    pub fn frame_vars(&self) -> &[u16] {
        #[cfg(debug_assertions)]
        debug_assert_ne!(
            self.space,
            DepSpace::Attr,
            "dep-space violation: attr-index deps read as frame vars ({:?})",
            self.items
        );
        &self.items
    }

    /// The entries read as callee attribute indices.
    /// Debug builds panic when the value is tagged as frame vars.
    #[must_use]
    pub fn as_attr_indices(&self) -> &[u16] {
        #[cfg(debug_assertions)]
        debug_assert_ne!(
            self.space,
            DepSpace::Frame,
            "dep-space violation: frame-var deps read as attr indices ({:?})",
            self.items
        );
        &self.items
    }

    /// #328: is this the pointer-field marker?
    #[must_use]
    pub fn is_pointer_marker(&self) -> bool {
        self.items.contains(&u16::MAX)
    }

    /// @PLN104 — renumber FRAME-variable entries in place for a variable swap:
    /// every plain entry equal to `from` becomes `to`.  Skips the `u16::MAX`
    /// pointer/share markers (they name no frame var).  FRAME-space only: a
    /// variable swap never relocates attribute indices, so applying this to an
    /// attr-space list would corrupt it (debug-asserted).  Used by the text-return
    /// retbuf renumber (`Type::renumber_frame_deps`).
    pub fn renumber_frame(&mut self, from: u16, to: u16) {
        #[cfg(debug_assertions)]
        debug_assert_ne!(
            self.space,
            DepSpace::Attr,
            "renumber_frame on attr-space deps ({:?})",
            self.items
        );
        for e in &mut self.items {
            if *e != u16::MAX && *e == from {
                *e = to;
            }
        }
    }

    /// A copy extended with `on` at the front (the `depending()` shape);
    /// inherits this value's space.
    #[must_use]
    pub fn extended(&self, on: u16) -> Deps {
        let mut v = vec![on];
        if !self.items.contains(&on) {
            v.append(&mut self.items.clone());
        }
        let items = v;
        #[cfg(debug_assertions)]
        {
            Deps {
                items,
                space: self.space,
            }
        }
        #[cfg(not(debug_assertions))]
        {
            Deps { items }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
/// Static type of a parsed expression or variable.
///
/// Several variants carry a `Vec<u16>` **dependency list** (`dep`):
/// - **Empty** → the value is *owned* — freed by `OpFreeRef` at scope exit.
/// - **Non-empty** → the value *borrows* from the parameters listed by
///   attribute index — NOT freed (the caller owns the store).
///
/// This governs the freeing logic in [`crate::scopes`].  See also
/// [`Function::depend`](crate::variables::Function) which adds entries.
// @F3 — primitive scalar types (integer/float/single/boolean/character) — the static Type model
pub enum Type {
    /// The type of this parse result is unknown, possibly linked to a yet unknown type (if != 0).
    Unknown(u32),
    /// The type of this result is specifically undefined.
    Null,
    /// Result of a function without return type.
    Void,
    /// Divergent expression (return/break/continue) — compatible with any type.
    Never,
    /// Integer type carrying bounds, null-flag, and optional forced
    /// storage width.  See [`IntegerSpec`].
    Integer(IntegerSpec),
    /// A store with the given base record type. (nullable)
    Boolean,
    Float,
    Single,
    Character,
    /// A text with the linked variables.
    Text(Deps),
    /// Description of the possible keys on a structure (hash, index, spatial, sorted)
    Keys,
    /// An enum value. With definition with enum type itself. With value true it is a reference.
    Enum(u32, bool, Deps),
    /// A readonly reference to a record instance in a store.
    Reference(u32, Deps),
    /// A reference to a variable on stack.
    RefVar(Box<Type>),
    /// A dynamic vector of a specific type
    Vector(Box<Type>, Deps), // @F6 — vector<T> (dynamic array + comprehensions + aggregates)
    /// A dynamic routine, from a routine definition without code.
    /// The actual code is a routine with this routine as a parent or just a Block for a lambda function.
    Routine(u32),
    /// Iterator with a certain result, the first type is the result per step.
    /// The second is the internal iterator value or `Type::Null` for structure iterator: `(i32,i32)`
    Iterator(Box<Type>, Box<Type>), // @F10 — iterator<T> values
    /// An ordered vector on a record, second is the key [field name, ascending]
    Sorted(u32, Vec<(String, bool)>, Deps), // @F8 — sorted<T[keys]> collection
    /// An index towards other records. The key is [field name, ascending]
    Index(u32, Vec<(String, bool)>, Deps), // @F9 — index<T[keys]> B-tree (asc/desc, multi-key)
    /// An index towards other records. The second is [field name]
    Radix(u32, Vec<String>, Deps),
    /// A hash table towards other records. The second is the hash function per [field name].
    Hash(u32, Vec<String>, Deps), // @F7 — hash<T[keys]> keyed collection
    /// A function reference allowing for closures. Argument types, result, and deps.
    /// The dep list tracks ownership of the closure record embedded in the fn-ref slot.
    Function(Vec<Type>, Box<Type>, Deps),
    /// A rewritten type into append statements (mostly Text or structures)
    Rewritten(Box<Type>),
    /// T1.1: stack-allocated fixed-arity compound type, e.g. `(integer, text)`.
    Tuple(Vec<Type>), // @F11 — tuples (anonymous fixed-arity)
    /// @PLN25 — a nullable wrapper over any base type (`τ?`). **Compile-time only:**
    /// `Optional(τ)` and `τ` share the same sentinel-based runtime layout (no wrapper
    /// alloc, no `__nullable` synth for scalars). Build it with [`Type::optional`] (kept
    /// idempotent — no `Optional(Optional)` — and normalising `Optional(Never|Null)`); read
    /// the bit with [`Type::peel_optional`]. A new variant — not a `nullable: bool` flag — so
    /// every exhaustive `match Type` is a COMPILE ERROR until it handles nullability (loud
    /// omission). See plans/25-nullable-sequences/scalar-optional-representation.md.
    Optional(Box<Type>),
}

impl Type {
    /// @PLN104 — renumber a FRAME-variable index (`from` → `to`) through every
    /// [`Deps`] this type carries, recursing into nested element / argument / return
    /// types.  Frame-space only (expression + variable-table types); the
    /// attribute-space `Definition.returned` must NOT be passed here.  Companion to
    /// the `Value`-tree walker (`Parser::renumber_frame_var`): together they move a
    /// variable's every reference — IR nodes AND embedded type deps — in tandem, which
    /// a `Function::swap_variables` needs so the body and the variable table stay in
    /// sync (loft-lang/loft#568; the `remap_var_deep`-only swap desynced the cref
    /// buffer's `Block.result` dep).
    pub fn renumber_frame_deps(&mut self, from: u16, to: u16) {
        match self {
            Type::Text(d)
            | Type::Reference(_, d)
            | Type::Enum(_, _, d)
            | Type::Sorted(_, _, d)
            | Type::Index(_, _, d)
            | Type::Radix(_, _, d)
            | Type::Hash(_, _, d) => d.renumber_frame(from, to),
            Type::Vector(inner, d) => {
                inner.renumber_frame_deps(from, to);
                d.renumber_frame(from, to);
            }
            Type::Function(args, ret, d) => {
                for a in args {
                    a.renumber_frame_deps(from, to);
                }
                ret.renumber_frame_deps(from, to);
                d.renumber_frame(from, to);
            }
            Type::RefVar(inner) | Type::Rewritten(inner) | Type::Optional(inner) => {
                inner.renumber_frame_deps(from, to);
            }
            Type::Iterator(step, state) => {
                step.renumber_frame_deps(from, to);
                state.renumber_frame_deps(from, to);
            }
            Type::Tuple(items) => {
                for t in items {
                    t.renumber_frame_deps(from, to);
                }
            }
            // scalar / dep-free variants
            Type::Unknown(_)
            | Type::Null
            | Type::Void
            | Type::Never
            | Type::Integer(_)
            | Type::Boolean
            | Type::Float
            | Type::Single
            | Type::Character
            | Type::Keys
            | Type::Routine(_) => {}
        }
    }

    /// @PLN25 — the idempotent `τ?` former. `Optional(Optional(τ)) → Optional(τ)`
    /// (N-Idem); `Optional(Never|Null) → Never|Null` (no junk optional over a non-value).
    /// Everything else becomes `Optional(Box::new(inner))`.
    pub fn optional(inner: Type) -> Type {
        match inner {
            Type::Optional(_) | Type::Never | Type::Null => inner,
            other => Type::Optional(Box::new(other)),
        }
    }

    /// @PLN25 — split a type into its base and whether it was `Optional`. The
    /// nullability-agnostic majority of `match Type` sites peel through this; only the
    /// discharge / store / cast checks (N-Store/N-Decl/N-Coal/N-Match) read the bool.
    pub fn peel_optional(&self) -> (&Type, bool) {
        match self {
            Type::Optional(inner) => (inner, true),
            other => (other, false),
        }
    }

    /// @PLN25 — the base type with any `Optional` wrapper removed (the agnostic peel).
    pub fn base(&self) -> &Type {
        self.peel_optional().0
    }

    /// Pass-2 keystone, the `Type` twin of `Value::for_each_child`
    /// (STABILITY_PASS2.md): the ONE place that knows which `Type`
    /// variants carry child types.  Exhaustive on purpose — a new
    /// variant forces a decision here and every walker inherits it.
    pub fn for_each_child(&self, f: &mut impl FnMut(&Type)) {
        match self {
            Type::RefVar(t) | Type::Vector(t, _) | Type::Rewritten(t) | Type::Optional(t) => f(t),
            Type::Iterator(a, b) => {
                f(a);
                f(b);
            }
            Type::Function(args, ret, _) => {
                args.iter().for_each(&mut *f);
                f(ret);
            }
            Type::Tuple(ts) => ts.iter().for_each(&mut *f),
            // Leaves — def-nr heads carry no child `Type`.
            Type::Unknown(_)
            | Type::Null
            | Type::Void
            | Type::Never
            | Type::Integer(_)
            | Type::Boolean
            | Type::Float
            | Type::Single
            | Type::Character
            | Type::Text(_)
            | Type::Keys
            | Type::Enum(_, _, _)
            | Type::Reference(_, _)
            | Type::Routine(_)
            | Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Hash(_, _, _) => {}
        }
    }

    /// Pre-order search: does `pred` hold on this type or any nested type?
    pub fn any_node(&self, pred: &mut impl FnMut(&Type) -> bool) -> bool {
        if pred(self) {
            return true;
        }
        let mut found = false;
        self.for_each_child(&mut |c| {
            if !found && c.any_node(pred) {
                found = true;
            }
        });
        found
    }

    /// Does this type mention definition `d_nr` anywhere — as a struct /
    /// enum reference, routine, keyed-collection record, or unresolved
    /// forward ref?  Unified from the parser's `type_contains_def` /
    /// `type_contains_tv` twins, whose hand-rolled descents missed the
    /// Tuple / Function / Iterator children that `substitute_type` DOES
    /// rewrite (the GET-side predicate had drifted behind the SET side).
    pub fn contains_def(&self, d_nr: u32) -> bool {
        self.any_node(&mut |t| {
            matches!(t,
                Type::Unknown(d)
                | Type::Enum(d, _, _)
                | Type::Reference(d, _)
                | Type::Routine(d)
                | Type::Sorted(d, _, _)
                | Type::Index(d, _, _)
                | Type::Radix(d, _, _)
                | Type::Hash(d, _, _) if *d == d_nr)
        })
    }

    /// Returns the dep list if this is a heap-allocated, store-backed type
    /// (Reference, Vector, struct-enum with is_ref=true, or any keyed
    /// collection: Sorted/Hash/Index/Radix — each `gen_set_first_keyed_null`
    /// allocates a fresh store via `OpDatabase` that needs scope-exit
    /// `OpFreeRef` cleanup).
    /// Use this instead of manual pattern matches to avoid forgetting an arm.
    #[must_use]
    pub fn heap_dep(&self) -> Option<&Vec<u16>> {
        match self {
            Type::Reference(_, dep)
            | Type::Vector(_, dep)
            | Type::Enum(_, true, dep)
            | Type::Sorted(_, _, dep)
            | Type::Hash(_, _, dep)
            | Type::Index(_, _, dep)
            | Type::Radix(_, _, dep) => Some(dep),
            _ => None,
        }
    }

    /// True if this type owns a heap store (heap_dep is Some and dep is empty).
    #[must_use]
    pub fn is_heap_owned(&self) -> bool {
        self.heap_dep().is_some_and(Vec::is_empty)
    }

    /// The definition number for struct-like heap types (Reference or struct-enum).
    #[must_use]
    pub fn heap_def_nr(&self) -> Option<u32> {
        match self {
            Type::Reference(d, _) | Type::Enum(d, true, _) => Some(*d),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_unknown(&self) -> bool {
        if let Type::Vector(tp, _) = self {
            return tp.is_unknown();
        }
        matches!(self, Type::Unknown(_)) || matches!(self, Type::Reference(0, _))
    }

    /// The same type with every dep list emptied — an OWNED reading of the shape.
    ///
    /// For a type used as a HINT (what shape is expected here?) rather than as a
    /// place. A hint is copied out of `Definition.returned` or an attribute typedef,
    /// so its deps are ATTRIBUTE indices; any caller that reads them as caller frame
    /// variables silently borrows from an unrelated local (loft#666). The shape is
    /// the only part of a hint that means anything, so this keeps exactly that.
    #[must_use]
    pub fn without_deps(&self) -> Type {
        match self {
            Type::Text(_) => Type::Text(Deps::none()),
            Type::Reference(t, _) => Type::Reference(*t, Deps::none()),
            Type::Enum(t, is_ref, _) => Type::Enum(*t, *is_ref, Deps::none()),
            Type::Index(t, keys, _) => Type::Index(*t, keys.clone(), Deps::none()),
            Type::Radix(t, keys, _) => Type::Radix(*t, keys.clone(), Deps::none()),
            Type::Hash(t, keys, _) => Type::Hash(*t, keys.clone(), Deps::none()),
            Type::Sorted(t, keys, _) => Type::Sorted(*t, keys.clone(), Deps::none()),
            Type::Vector(t, _) => Type::Vector(Box::new(t.without_deps()), Deps::none()),
            Type::Function(params, ret, _) => {
                Type::Function(params.clone(), ret.clone(), Deps::none())
            }
            Type::RefVar(tp) => Type::RefVar(Box::new(tp.without_deps())),
            Type::Optional(tp) => Type::optional(tp.without_deps()),
            Type::Tuple(elems) => Type::Tuple(elems.iter().map(Type::without_deps).collect()),
            _ => self.clone(),
        }
    }

    /**
    Return the same type but with an additional variable in the dependency list.
    # Panics
    When this extra variable doesn't exist.
    */
    #[must_use]
    /// Rebase this type's borrow onto frame var `on`.
    ///
    /// H2 finding (DEPS_INVENTORY § findings): the original per-arm bodies
    /// carried a dead merge guard — `let mut v = vec![on]; if !v.contains(&on)
    /// { v.append(&mut dep.clone()) }` can never append (`v` always contains
    /// `on`), so `depending()` has ALWAYS meant "deps := [on]" (replace, not
    /// extend); the `Index` arm would even have double-appended had it ever
    /// run.  This rewrite keeps the replace semantics and deletes the fossil.
    /// Every caller passes a frame var (the assert has always rejected the
    /// `u16::MAX` markers).
    pub fn depending(&self, on: u16) -> Type {
        assert_ne!(on, u16::MAX, "Unknown depended on variable");
        let v = Deps::frame1(on);
        match self {
            Type::Text(_) => Type::Text(v),
            Type::Reference(t, _) => Type::Reference(*t, v),
            Type::Enum(t, is_ref, _) => Type::Enum(*t, *is_ref, v),
            Type::Index(t, keys, _) => Type::Index(*t, keys.clone(), v),
            Type::Radix(t, keys, _) => Type::Radix(*t, keys.clone(), v),
            Type::Hash(t, keys, _) => Type::Hash(*t, keys.clone(), v),
            Type::Sorted(t, keys, _) => Type::Sorted(*t, keys.clone(), v),
            Type::Vector(t, _) => Type::Vector(Box::new(*t.clone()), v),
            Type::Function(params, ret, _) => Type::Function(params.clone(), ret.clone(), v),
            Type::RefVar(tp) => Type::RefVar(Box::new(tp.depending(on))),
            // @PLN25 — `Optional` is a compile-time nullability marker over the base's
            // runtime representation; deps are a lifetime property, agnostic to it, so a
            // borrow attaches to the base and the wrapper is transparent (like `RefVar`).
            // Without this an `Optional`-typed borrow (`e = v[i]` under DN1) silently loses
            // its dep, and the deps pass treats it as OWNING → wrong store alloc / corruption.
            Type::Optional(tp) => Type::optional(tp.depending(on)),
            Type::Tuple(elems) => Type::Tuple(elems.iter().map(|e| e.depending(on)).collect()),
            _ => self.clone(),
        }
    }

    #[must_use]
    /// The dependency list AS TAGGED (`Deps`) — `None` for dep-less
    /// variants; recurses through `RefVar`.  A `Tuple` has no single list
    /// (its deps are the union of its elements'); use [`Type::depend`]
    /// for the flattened, tag-erased union.
    pub fn deps_ref(&self) -> Option<&Deps> {
        match self {
            Type::Text(dep)
            | Type::Reference(_, dep)
            | Type::Index(_, _, dep)
            | Type::Radix(_, _, dep)
            | Type::Hash(_, _, dep)
            | Type::Sorted(_, _, dep)
            | Type::Enum(_, _, dep)
            | Type::Vector(_, dep)
            | Type::Function(_, _, dep) => Some(dep),
            // @PLN25 — `Optional` is dep-transparent (see `depending`).
            Type::RefVar(tp) | Type::Optional(tp) => tp.deps_ref(),
            _ => None,
        }
    }

    #[must_use]
    pub fn depend(&self) -> Vec<u16> {
        let mut v = Vec::new();
        match self {
            Type::Text(dep)
            | Type::Reference(_, dep)
            | Type::Index(_, _, dep)
            | Type::Radix(_, _, dep)
            | Type::Hash(_, _, dep)
            | Type::Sorted(_, _, dep)
            | Type::Enum(_, _, dep)
            | Type::Vector(_, dep)
            | Type::Function(_, _, dep) => v.append(&mut dep.clone()),
            // @PLN25 — `Optional` is dep-transparent (see `depending`).
            Type::RefVar(tp) | Type::Optional(tp) => return tp.depend(),
            // P197: a tuple's effective dependencies are the union of
            // its elements'.  Dedup to keep the vector compact.
            Type::Tuple(elems) => {
                for e in elems {
                    for d in e.depend() {
                        if !v.contains(&d) {
                            v.push(d);
                        }
                    }
                }
            }
            _ => {}
        }
        v
    }

    #[must_use]
    pub fn content(&self) -> Type {
        match self {
            Type::Index(tp, _, dep)
            | Type::Radix(tp, _, dep)
            | Type::Hash(tp, _, dep)
            | Type::Sorted(tp, _, dep) => Type::Reference(*tp, dep.clone()),
            Type::Vector(tp, _) => *tp.clone(),
            Type::RefVar(tp) => tp.content(),
            _ => Type::Unknown(0),
        }
    }

    /// Do these two types belong to the same KIND — *not* are they the same type.
    ///
    /// Any two `Reference`s are "same" here whatever struct they name, and likewise any
    /// two `Enum`s or `Vector`s; only the outer constructor is compared. That is what
    /// callers asking "is this the same shape of thing" want, and it is a trap for
    /// callers asking "may this binding hold that value" — loft#690: the loop-variable
    /// reuse check asked with `is_same`, so `for r in as_a { … } for r in as_b { … }`
    /// over two different structs passed, and the second loop then read B's records
    /// through A's layout with no diagnostic and no crash.
    ///
    /// **Use [`Type::is_equal`] for type IDENTITY** — it compares the struct a
    /// `Reference` names and the element of a `Vector`, while still ignoring the
    /// differences that are not type differences (integer ranges, text deps, fn-ref
    /// capture lists).
    #[must_use]
    pub fn is_same(&self, other: &Type) -> bool {
        // @P352: two `Type::Function`s compare by SHAPE (params + return),
        // ignoring the dep list — a fn-ref's dep list records which closure
        // vars it captured, which differs per binding site, so a raw `==`
        // wrongly reports two structurally-identical fn-refs as different
        // (e.g. the @P344 loop-var reuse check fired on `for f in a {…}` then
        // `for f in b {…}` even though both are `fn(integer)->integer`).
        if let (Type::Function(sp, sr, _), Type::Function(op, or, _)) = (self, other) {
            return sp.len() == op.len()
                && sp.iter().zip(op.iter()).all(|(a, b)| a.is_equal(b))
                && sr.is_equal(or);
        }
        self == other
            || (matches!(self, Type::Enum(_, _, _)) && matches!(other, Type::Enum(_, _, _)))
            || (matches!(self, Type::Reference(_, _)) && matches!(other, Type::Reference(_, _)))
            || (matches!(self, Type::Vector(_, _)) && matches!(other, Type::Vector(_, _)))
            || (matches!(self, Type::Integer(_)) && matches!(other, Type::Integer(_)))
            || (matches!(self, Type::Text(_)) && matches!(other, Type::Text(_)))
    }

    #[must_use]
    pub fn is_equal(&self, other: &Type) -> bool {
        // `Rewritten(T)` is a marker on a variable that has been rewritten
        // into append statements; semantically the type is still T.  Compare
        // through the wrapper so a `vector<T>` LHS matches a `vector<T>` RHS
        // even when only one side carries the marker.
        if let Type::Rewritten(inner) = self {
            return inner.is_equal(other);
        }
        if let Type::Rewritten(inner) = other {
            return self.is_equal(inner);
        }
        match (self, other) {
            (Type::RefVar(s), Type::RefVar(o)) => return s.is_equal(o),
            (Type::Enum(s, s_tp, _), Type::Enum(o, o_tp, _)) => return *s == *o && *s_tp == *o_tp,
            (Type::Reference(r, _), Type::Reference(o, _)) => return r == o,
            (Type::Vector(r, _), Type::Vector(o, _)) => {
                return r.is_equal(o) && r.same_element_storage(o);
            }
            (Type::Hash(r, rf, _), Type::Hash(o, of, _))
            | (Type::Radix(r, rf, _), Type::Radix(o, of, _)) => return r == o && rf == of,
            (Type::Sorted(r, rf, _), Type::Sorted(o, of, _))
            | (Type::Index(r, rf, _), Type::Index(o, of, _)) => return r == o && rf == of,
            (Type::Function(sp, sr, _), Type::Function(op, or, _)) => {
                return sp.len() == op.len()
                    && sp.iter().zip(op.iter()).all(|(a, b)| a.is_equal(b))
                    && sr.is_equal(or);
            }
            // T1.7: tuple equality ignores `not_null` on Integer elements (runtime type is same).
            (Type::Tuple(se), Type::Tuple(oe)) => {
                return se.len() == oe.len()
                    && se.iter().zip(oe.iter()).all(|(a, b)| a.is_equal(b));
            }
            _ => {}
        }
        self == other
            || (matches!(self, Type::Integer(_)) && matches!(other, Type::Integer(_)))
            || (matches!(self, Type::Text(_)) && matches!(other, Type::Text(_)))
    }

    /// loft#751 — do two types occupy the SAME bytes when they are the ELEMENT
    /// of a vector?  In a register an `integer` and a `u8` are one type, which
    /// is why [`Self::is_equal`] compares two scalar integers by kind alone.
    /// In a vector they are not: the element width IS the stride, so handing a
    /// `vector<integer>` to a `vector<u8>` parameter re-reads each 8-byte
    /// element as eight 1-byte ones — silently, since the element COUNT is
    /// stored and still agrees.  Width comes from the canonical
    /// [`IntegerSpec::byte_width`] (so a range-typed element and its alias —
    /// `integer(0,100)` and `u8` — are correctly the same layout), and the sign
    /// of the lower bound separates `i8` from `u8`, which share a width but not
    /// a reading.
    #[must_use]
    pub fn same_element_storage(&self, other: &Type) -> bool {
        match (self.base(), other.base()) {
            (Type::Integer(a), Type::Integer(b)) => {
                a.byte_width(!a.not_null) == b.byte_width(!b.not_null) && (a.min < 0) == (b.min < 0)
            }
            (Type::Vector(a, _), Type::Vector(b, _)) => a.same_element_storage(b),
            _ => true,
        }
    }

    #[must_use]
    pub fn size(&self, nullable: bool) -> u8 {
        // @PLN25 slice (b): `Optional(τ)` shares its base's storage width — peel the marker.
        if let Type::Integer(spec) = self.base() {
            // H6: derive from the ONE range→width home so the field WRITE width
            // (this, via `set_field_check`) cannot drift from the READ width
            // (`IntegerSpec::byte_width`, via `get_val`).  Honours the value
            // range only — `forced_size` is handled by the callers (they check
            // the alias size before reaching here), matching the prior contract.
            spec.range_to_width(nullable)
        } else {
            0
        }
    }

    #[must_use]
    pub fn name(&self, data: &Data) -> String {
        match self {
            Type::Optional(tp) => format!("{}?", tp.name(data)),
            Type::Rewritten(tp) => tp.name(data),
            Type::RefVar(tp) => format!("&{}", tp.name(data)),
            Type::Enum(t, _, _) | Type::Reference(t, _) => data.def(*t).name.clone(),
            Type::Text(_) => "text".to_string(),
            Type::Vector(tp, _) if matches!(tp as &Type, Type::Unknown(_)) => "vector".to_string(),
            Type::Vector(tp, _) => format!("vector<{}>", tp.name(data)),
            Type::Sorted(tp, key, _) => {
                format!("sorted<{},{key:?}>", data.def(*tp).name)
            }
            Type::Hash(tp, key, _) => format!("hash<{},{key:?}>", data.def(*tp).name),
            Type::Index(tp, key, _) => format!("index<{},{key:?}>", data.def(*tp).name),
            Type::Radix(tp, key, _) => {
                format!("spatial<{},{key:?}>", data.def(*tp).name)
            }
            Type::Routine(tp) => format!("fn {}[{tp}]", data.def(*tp).name),
            // Plan-07 phase 6.1 — explicit user-facing rendering for the
            // remaining variants.  Pre-fix these fell through to the
            // Display impl which lower-cases the debug format
            // (e.g. `tuple([integer(...), text([])])`); user-visible
            // error messages now render proper loft-surface syntax.
            Type::Unknown(_) => "unknown".to_string(),
            Type::Null => "null".to_string(),
            Type::Void => "void".to_string(),
            Type::Never => "never".to_string(),
            Type::Boolean => "boolean".to_string(),
            Type::Float => "float".to_string(),
            Type::Single => "single".to_string(),
            Type::Character => "character".to_string(),
            Type::Integer(spec) if spec.source_name().is_some() => {
                spec.source_name().unwrap_or("integer").to_string()
            }
            Type::Integer(spec) => format!("integer({}, {})", spec.min, spec.max),
            Type::Keys => "keys".to_string(),
            Type::Iterator(elem, _) => format!("iterator<{}>", elem.name(data)),
            Type::Tuple(elems) => {
                let inner = elems
                    .iter()
                    .map(|e| e.name(data))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({inner})")
            }
            Type::Function(params, ret, _) => {
                let p = params
                    .iter()
                    .map(|t| t.name(data))
                    .collect::<Vec<_>>()
                    .join(", ");
                if matches!(ret.as_ref(), Type::Void) {
                    format!("fn({p})")
                } else {
                    format!("fn({p}) -> {}", ret.name(data))
                }
            }
        }
    }

    #[must_use]
    pub fn show(&self, data: &Data, vars: &Function) -> String {
        match self {
            Type::RefVar(tp) => format!("&{}", tp.show(data, vars)),
            Type::Enum(t, false, _) => data.def(*t).name.clone(),
            Type::Reference(t, dep) | Type::Enum(t, true, dep) => {
                format!("ref({}){}", data.def(*t).name, Self::dep_var(dep, vars))
            }
            Type::Vector(tp, dep) if matches!(tp as &Type, Type::Unknown(_)) => {
                format!("vector{}", Self::dep_var(dep, vars))
            }
            Type::Vector(tp, dep) => format!(
                "vector<{}>{}",
                tp.show(data, vars),
                Self::dep_var(dep, vars)
            ),
            Type::Sorted(tp, key, dep) => {
                format!(
                    "sorted<{},{key:?}>{}",
                    data.def(*tp).name,
                    Self::dep_var(dep, vars)
                )
            }
            Type::Hash(tp, key, dep) => format!(
                "hash<{},{key:?}>{}",
                data.def(*tp).name,
                Self::dep_var(dep, vars)
            ),
            Type::Index(tp, key, dep) => format!(
                "index<{},{key:?}>{}",
                data.def(*tp).name,
                Self::dep_var(dep, vars)
            ),
            Type::Radix(tp, key, dep) => {
                format!(
                    "spatial<{},{key:?}>{}",
                    data.def(*tp).name,
                    Self::dep_var(dep, vars)
                )
            }
            Type::Routine(tp) => format!("fn {}[{tp}]", data.def(*tp).name),
            Type::Text(dep) if dep.is_empty() => "text".to_string(),
            Type::Text(dep) => format!("text{}", Self::dep_var(dep, vars)),
            Type::Tuple(elems) => {
                let inner = elems
                    .iter()
                    .map(|e| e.show(data, vars))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({inner})")
            }
            _ => self.to_string(),
        }
    }

    fn dep_var(dep: &Vec<u16>, vars: &Function) -> String {
        let mut ls = BTreeSet::new();
        for d in dep {
            ls.insert(vars.name(*d).to_string());
        }
        let mut res = Vec::new();
        for v in ls {
            res.push(v);
        }
        if res.is_empty() {
            String::new()
        } else {
            format!("{res:?}")
        }
    }

    #[must_use]
    pub fn argument(&self, data: &Data, d_nr: u32) -> String {
        match self {
            Type::Reference(t, link) if link.is_empty() => data.def(*t).name.clone(),
            Type::Reference(t, link) => {
                format!("{}{:?}", data.def(*t).name, Self::dep_att(data, d_nr, link))
            }
            Type::Text(dep) if dep.is_empty() => "text".to_string(),
            Type::Text(dep) => format!("text{:?}", Self::dep_att(data, d_nr, dep)),
            _ => {
                let d = data.def(d_nr);
                self.show(data, &Function::new(&d.name, &d.position.file))
            }
        }
    }

    fn dep_att(data: &Data, d_nr: u32, dep: &Vec<u16>) -> Vec<String> {
        let mut ls = BTreeSet::new();
        for d in dep {
            // A dep list is attribute-indexed only at parse time; after scopes
            // runs it holds FRAME var numbers, which this display helper cannot
            // resolve (no Function at hand).  Render those positionally instead
            // of indexing out of bounds (par-synthesized defs hit this).
            match data.def(d_nr).attributes.get(*d as usize) {
                Some(a) => ls.insert(a.name.clone()),
                None => ls.insert(format!("#{d}")),
            };
        }
        let mut res = Vec::new();
        for v in ls {
            res.push(v);
        }
        res
    }
}

// ── T1.1 — Tuple element layout helpers ─────────────────────────────────────

/// Natural-alignment of a single element type, in bytes.
/// Plan-14 phase 07 (P234 runtime): does this type carry a lifetime
/// concern that requires store-side ownership tracking?  Used by the
/// function-return rewrite in `parser/definitions.rs` to decide
/// whether a `Type::Tuple(elems)` return must be re-routed through
/// the synthetic `__tuple<…>` struct (so the existing struct-return
/// `ref_return` / `text_return` ownership-transfer machinery applies).
///
/// Lifetime-bearing = goes through `text_return` / `ref_return` as a
/// direct function return today: Text, Reference, Vector, Enum-struct,
/// Sorted / Hash / Index / Radix keyed collections, RefVar.  Tuples
/// recursively inherit the concern from any element.
///
/// Pure-value tuples (every element is a scalar value type — Integer,
/// Float, Single, Boolean, Character, Enum-no-payload, Function fn-ref)
/// continue to use Rust's tuple ABI under `--native` (the T1.8a path
/// for `(integer, integer)` and similar shapes).
#[must_use]
pub fn has_lifetime_concern(t: &Type) -> bool {
    matches!(
        t,
        Type::Text(_)
            | Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Enum(_, true, _)
            | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Radix(_, _, _)
            | Type::RefVar(_)
    ) || matches!(t, Type::Tuple(elems) if elems.iter().any(has_lifetime_concern))
}

///
/// Used by `element_offsets` to pad tuple-element offsets so each
/// element lands on its natural-alignment boundary — a tuple
/// `(byte, integer)` has `_0` at offset 0 and `_1` at offset 8 (not 1).
/// Mirrors the alignment table in `LinkedFieldGroup::group_alignment`'s
/// caller (`Data::tuple_def`) and the database `align` field set by
/// `database::types`.
#[must_use]
pub fn element_stack_align(t: &Type) -> u8 {
    match t {
        // @PLN25 slice (b): `Optional(τ)` aligns like its base (same storage).
        Type::Optional(inner) => element_stack_align(inner),
        Type::Boolean | Type::Enum(_, false, _) => 1,
        Type::Single | Type::Character => 4,
        // P249 — fn-ref slot layout per `variables::size` and
        // `OpVarFnRef`'s `[u8; 20]` read: 8 B d_nr (i64) + 12 B
        // closure DbRef.  The d_nr's i64 alignment dictates the
        // overall slot alignment.
        Type::Function(_, _, _) => 8,
        Type::Integer(_) | Type::Float => 8,
        Type::Text(_) => 4,
        Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::RefVar(_)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _)
        | Type::Enum(_, true, _) => 4,
        Type::Tuple(elems) => element_offsets_alignment_max(elems),
        _ => 1,
    }
}

/// Internal: max alignment across a tuple's elements.  Recursive
/// because nested tuples contribute their own max alignment.
fn element_offsets_alignment_max(types: &[Type]) -> u8 {
    types.iter().map(element_stack_align).max().unwrap_or(1)
}

/// Stack width in bytes of a single element type.
/// Uses the same sizing as `variables::size(tp, &Context::Argument)`.
///
/// For tuples this returns the **atomic group size** —
/// alignment-padded so each element lands on its natural-alignment
/// boundary.  Matches `LinkedFieldGroup::group_size`'s packing so
/// the runtime read path (`element_offsets`) and the storage layout
/// (`calculate_positions_with_groups`) agree on every byte offset.
#[must_use]
pub fn element_stack_size(t: &Type) -> usize {
    match t {
        // @PLN25 slice (b): `Optional(τ)` shares its base's sentinel storage size.
        Type::Optional(inner) => element_stack_size(inner),
        Type::Boolean | Type::Enum(_, false, _) => 1,
        Type::Single | Type::Character => 4,
        // P249 — fn-ref slot is 20 bytes (8 B d_nr + 12 B closure DbRef);
        // matches `variables::size(Type::Function, _) = 20` and
        // `OpVarFnRef`'s `[u8; 20]` read.  Pre-fix returned 4, which
        // truncated tuple-stored closures and produced garbage on
        // call.
        Type::Function(_, _, _) => 20,
        Type::Integer(_) | Type::Float => 8,
        Type::Text(_) => std::mem::size_of::<crate::keys::Str>(),
        Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _)
        | Type::Enum(_, true, _) => std::mem::size_of::<crate::keys::DbRef>(),
        Type::Tuple(elems) => {
            // @PLN114 — the STACK view: one `aligned_stack_step` slot per element,
            // because that is what a push actually advances.  The natural-alignment
            // packing this used to do described neither side — the callee's frame was
            // sized from it while the caller's pushes stepped by 8, so `(P,P)`
            // reserved 24 bytes for a 32-byte push and the callee read its second
            // element 4 bytes early.  Storage is `element_storage_size`'s job now, so
            // widening here no longer costs a byte on the heap.
            elems
                .iter()
                .map(|t| {
                    crate::variables::aligned_stack_step(element_stack_size(t) as u32) as usize
                })
                .sum()
        }
        _ => 0,
    }
}

/// The **STORAGE** width of one tuple element, in the record's terms (@PLN114).
///
/// Sibling of [`element_stack_size`]. A tuple has two legitimate layouts and the
/// names must say which is which — they genuinely differ: `(u8, u16)` is **3 bytes
/// stored** and **16 on the stack**. Consulting the wrong one silently is the whole
/// bug class this pair exists to end.
///
/// [`element_size`] reports the **eval-stack** width: `Integer` is 8 bytes flat,
/// because that is what a push occupies regardless of the alias's `size(N)`.  A
/// tuple element stored in a record or a vector is a *field*, so it must be sized
/// the way a field is — `IntegerSpec::byte_width`, the one home for the storage
/// width (`forced_size` first, else the range).  Using the stack width for storage
/// is what makes `(u8, u16)` occupy 16 bytes where `struct { a: u8, b: u16 }`
/// occupies 3.
///
/// Records pack TIGHTLY — `struct { a: u8, b: u32, c: u16 }` is 1+4+2 = 7 bytes with
/// no padding, because store access is unaligned-tolerant — so there is no alignment
/// term here.
#[must_use]
pub fn element_storage_size(t: &Type) -> usize {
    match t.base() {
        Type::Integer(spec) => spec.byte_width(true) as usize,
        Type::Tuple(elems) => elems.iter().map(element_storage_size).sum(),
        // Measured against the record oracle: `(u8, text)` is 5 bytes, so a stored
        // `text` element is the 4-byte heap pointer, NOT the 16-byte stack `Str`.
        // `read_tuple_at_wide` says the same ("text: 4-byte heap-pointer") and
        // inflates it to a `Str` for the worker slot.
        Type::Text(_) => 4,
        // A stored fn-ref is EIGHT bytes: `parser/mod.rs`'s `get_val` documents the
        // shape — "storage is two database fields per loft attribute: `<attr>` 4B
        // i32 d_nr, `<attr>__closure_rec` 4B vector header at pos+4".  The 20-byte
        // figure is the STACK slot (8B d_nr + 12B closure DbRef), which `get_val`
        // reconstructs from these two halves.  Reserving only 4 truncates the
        // closure half and the fn-ref reads back wrong.
        Type::Function(_, _, _) => 8,
        other => element_stack_size(other),
    }
}

/// Element offsets in the **STORAGE** (record) layout: cumulative
/// [`element_storage_size`], packed tight. Sibling of [`element_stack_offsets`].
#[must_use]
pub fn element_storage_offsets(types: &[Type]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(types.len());
    let mut pos = 0usize;
    for t in types {
        offsets.push(pos);
        pos += element_storage_size(t);
    }
    offsets
}

/// Byte offset of each element in a tuple-like layout.
/// Element *i* starts at `offsets[i]`; total size is `element_stack_size(&Type::Tuple(types))`.
///
/// **Alignment-aware**: each element is placed at the next position
/// that satisfies its natural alignment.  For `(byte, integer)`,
/// returns `[0, 8]` (not `[0, 1]`) — `_1` (integer, align 8) needs
/// an 8-byte boundary, so 7 bytes of padding follow `_0`.
///
/// This matches `LinkedFieldGroup::group_member_offsets` exactly,
/// so storage layout (synthetic `__tuple<…>` struct via
/// `calculate_positions_with_groups`) and runtime reads (via
/// codegen / `read_tuple_at_wide`) compute the same offsets.
#[must_use]
pub fn element_stack_offsets(types: &[Type]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(types.len());
    let mut pos: usize = 0;
    for t in types {
        offsets.push(pos);
        // @PLN114 — one stepped slot per element; see `element_stack_size`.
        pos += crate::variables::aligned_stack_step(element_stack_size(t) as u32) as usize;
    }
    offsets
}

/// Resolve per-element byte offsets for a STORED tuple via the synthetic
/// `__tuple<…>` struct's post-finish field positions.
///
/// Returns `Some(offsets)` when:
/// - `tuple_def` has registered the synthetic struct, AND
/// - `Stores::finish_type` has assigned `position` to every field
///   (i.e. layout has been finalised).
///
/// Returns `None` in any other situation (struct not yet registered,
/// not yet finished, or wrong arity) so callers can fall back to
/// `element_offsets` for early-parse paths.
///
/// **Why this exists**: storage reads / writes for tuple elements MUST
/// use the same field offsets that ordinary struct fields use via
/// `OpGetInt` / `OpSetInt`.  Routing through the synthetic struct's
/// finished layout (rather than recomputing via `element_offsets`)
/// means any divergence between the two paths is detected immediately
/// — and keeps a single source of truth for stored-tuple field
/// offsets.
#[must_use]
pub fn stored_tuple_offsets(
    data: &Data,
    database: &crate::database::Stores,
    elems: &[Type],
) -> Option<Vec<u16>> {
    let inner_names: Vec<String> = elems.iter().map(|t| t.name(data)).collect();
    let name = format!("__tuple<{}>", inner_names.join(","));
    let def_nr = data.def_nr(&name);
    if def_nr == u32::MAX {
        return None;
    }
    stored_tuple_offsets_for_def(data, database, def_nr, elems.len())
}

/// Same as [`stored_tuple_offsets`] but with the synthetic struct's
/// `def_nr` already resolved.  Use when the caller already has the
/// def_nr (e.g. parser sites that called `tuple_def` directly).
#[must_use]
pub fn stored_tuple_offsets_for_def(
    data: &Data,
    database: &crate::database::Stores,
    def_nr: u32,
    expected_arity: usize,
) -> Option<Vec<u16>> {
    if def_nr == u32::MAX {
        return None;
    }
    let known_type = data.def(def_nr).known_type;
    if (known_type as usize) >= database.types.len() {
        return None;
    }
    let parts = &database.types[known_type as usize].parts;
    let fields = match parts {
        crate::database::Parts::Struct(f) | crate::database::Parts::EnumValue(_, f) => f,
        _ => return None,
    };
    if fields.len() != expected_arity {
        return None;
    }
    let mut out = Vec::with_capacity(fields.len());
    for f in fields {
        if f.position == u16::MAX {
            return None;
        }
        out.push(f.position);
    }
    Some(out)
}

#[cfg(test)]
mod renumber_frame_deps_tests {
    //! @PLN104 — the type-dep half of a variable renumber (the piece the
    //! `remap_var_deep`-only retbuf swap was missing, loft-lang/loft#568).
    use super::{Deps, Type};

    fn text(dep: Vec<u16>) -> Type {
        Type::Text(Deps::frame(dep))
    }

    #[test]
    fn plain_frame_dep_moves() {
        let mut t = text(vec![2]);
        t.renumber_frame_deps(2, 99);
        assert_eq!(t.depend(), vec![99]);
    }

    #[test]
    fn non_matching_dep_untouched() {
        let mut t = text(vec![5]);
        t.renumber_frame_deps(2, 99);
        assert_eq!(t.depend(), vec![5]);
    }

    #[test]
    fn u16_max_marker_preserved() {
        let mut t = text(vec![u16::MAX, 2]);
        t.renumber_frame_deps(2, 99);
        assert_eq!(t.depend(), vec![u16::MAX, 99]);
    }

    #[test]
    fn recurses_into_vector_and_function() {
        let mut v = Type::Vector(Box::new(text(vec![2])), Deps::frame(vec![2]));
        v.renumber_frame_deps(2, 99);
        let Type::Vector(inner, d) = &v else { panic!() };
        assert_eq!(inner.depend(), vec![99]);
        assert_eq!(&d.items, &vec![99]);

        let mut f = Type::Function(
            vec![text(vec![2])],
            Box::new(text(vec![2])),
            Deps::frame(vec![2]),
        );
        f.renumber_frame_deps(2, 99);
        let Type::Function(args, ret, d) = &f else {
            panic!()
        };
        assert_eq!(args[0].depend(), vec![99]);
        assert_eq!(ret.depend(), vec![99]);
        assert_eq!(&d.items, &vec![99]);
    }

    #[test]
    fn three_way_swap_exchanges_two_indices() {
        // swap frame vars 2 and 3 via the temp-index dance the caller uses.
        let mut t = Type::Tuple(vec![text(vec![2]), text(vec![3]), text(vec![2, 3])]);
        let tmp = 100u16;
        t.renumber_frame_deps(2, tmp);
        t.renumber_frame_deps(3, 2);
        t.renumber_frame_deps(tmp, 3);
        let Type::Tuple(items) = &t else { panic!() };
        assert_eq!(items[0].depend(), vec![3]); // was 2
        assert_eq!(items[1].depend(), vec![2]); // was 3
        assert_eq!(items[2].depend(), vec![3, 2]); // was [2,3]
    }
}

#[cfg(test)]
mod tuple_stack_layout_tests {
    //! Stack-level tuple layout tests.  `element_size` and
    //! `element_offsets` operate at stack widths — every
    //! `Type::Integer` reports 8 bytes regardless of `forced_size`,
    //! `Type::Text` reports `size_of::<Str>()` (16 bytes), etc.
    //! Used by codegen for STACK tuples (`Value::TupleGet` on
    //! `Type::Tuple` variables) and the par worker's tuple-arg
    //! inflation in `read_tuple_at_wide`.
    //!
    //! **Storage** (`Store` heap) tuple element access does NOT use
    //! these helpers — it goes through the synthetic `__tuple<…>`
    //! struct's post-finish field positions via
    //! `state::codegen::stored_tuple_field_offset`.
    use super::{
        IntegerSpec, Type, element_stack_align, element_stack_offsets, element_stack_size,
    };

    fn integer() -> Type {
        Type::Integer(IntegerSpec {
            min: i32::MIN + 1,
            max: i32::MAX as u32,
            not_null: false,
            forced_size: None,
        })
    }

    fn boolean() -> Type {
        Type::Boolean
    }

    #[test]
    fn stack_offsets_two_integers() {
        // (int, int): both 8B aligned 8.  Stack layout: [0, 8].
        let elems = vec![integer(), integer()];
        assert_eq!(element_stack_offsets(&elems), vec![0, 8]);
        assert_eq!(element_stack_size(&Type::Tuple(elems)), 16);
    }

    #[test]
    fn stack_offsets_three_bools() {
        // (bool, bool, bool) on the STACK: one stepped slot each, so [0, 8, 16] and
        // 24 bytes — NOT the packed [0, 1, 2] this asserted before @PLN114.
        //
        // The old expectation described the storage layout while naming itself a
        // stack test, and it was wrong about the stack in a way that mattered: a
        // bool push advances `aligned_stack_step(1)` = 8, so a callee reading the
        // second element at +1 read the first element's padding.  `probes/bool3`
        // (a `(boolean,boolean,boolean)` argument) is the runtime witness.
        // Storage still packs these into 3 bytes — see `storage_view_packs_like_a_record`.
        let elems = vec![boolean(), boolean(), boolean()];
        assert_eq!(element_stack_offsets(&elems), vec![0, 8, 16]);
        assert_eq!(element_stack_size(&Type::Tuple(elems)), 24);
    }

    fn narrow(bits: u8) -> Type {
        // u8 / u16 / u32 shaped: a range-limited integer carrying `size(N)`.
        Type::Integer(IntegerSpec {
            min: 0,
            max: match bits {
                1 => 255,
                2 => 65535,
                _ => 4_294_967_294,
            },
            not_null: false,
            forced_size: std::num::NonZeroU8::new(bits),
        })
    }

    /// @PLN114 D1 — the storage view sizes elements as record FIELDS.
    ///
    /// Hand-computed against the record oracle: `struct { a: u8, b: u32, c: u16 }`
    /// measures 7 bytes per record on this build, so the tuple of the same three
    /// element types must compute 7 too.
    #[test]
    fn storage_view_packs_like_a_record() {
        use super::{element_storage_offsets, element_storage_size};
        let elems = vec![narrow(1), narrow(4), narrow(2)];
        assert_eq!(element_storage_offsets(&elems), vec![0, 1, 5]);
        assert_eq!(
            element_storage_size(&Type::Tuple(elems)),
            7,
            "u8 + u32 + u16 packs to 7 bytes, as `struct M` does"
        );

        let pair = vec![narrow(1), narrow(2)];
        assert_eq!(element_storage_offsets(&pair), vec![0, 1]);
        assert_eq!(element_storage_size(&Type::Tuple(pair)), 3);
    }

    /// The stack view is unchanged and deliberately WIDER — a push occupies a whole
    /// slot whatever the alias says.  The two views must not be confused, so pin the
    /// difference rather than leaving it implicit.
    #[test]
    fn stack_view_stays_wide_where_storage_narrows() {
        use super::{element_stack_size, element_storage_size};
        let elems = vec![narrow(1), narrow(4), narrow(2)];
        assert_eq!(
            element_stack_size(&Type::Tuple(elems.clone())),
            24,
            "stack: three 8-byte integer slots"
        );
        assert_eq!(
            element_storage_size(&Type::Tuple(elems)),
            7,
            "storage: 1 + 4 + 2"
        );
    }

    /// Plain `integer` has no `forced_size`, so both views agree at 8 — which is why
    /// `(integer, integer)` tuples have never shown either defect.
    #[test]
    fn storage_and_stack_agree_for_plain_integers() {
        use super::{element_stack_size, element_storage_size};
        let elems = vec![integer(), integer()];
        assert_eq!(element_stack_size(&Type::Tuple(elems.clone())), 16);
        assert_eq!(element_storage_size(&Type::Tuple(elems)), 16);
    }

    #[test]
    fn stack_alignment_max_member() {
        // Max alignment among elements drives the tuple's alignment.
        let elems = vec![boolean(), integer()];
        assert_eq!(element_stack_align(&Type::Tuple(elems)), 8);
    }
}

/// `(offset, index)` pairs for elements that need cleanup on scope exit
/// (text, reference, vector, collection, struct-enum).
#[must_use]
pub fn owned_elements(types: &[Type]) -> Vec<(usize, usize)> {
    let offsets = element_stack_offsets(types);
    let mut result = Vec::new();
    for (i, t) in types.iter().enumerate() {
        match t {
            Type::Text(_)
            | Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Enum(_, true, _) => {
                result.push((offsets[i], i));
            }
            _ => {}
        }
    }
    result
}

/// Plan-06 phase 5b' (DESIGN.md D12) — recursive walk of a `Value`
/// tree collecting every `Value::Call(callee, _)` edge.  Pushes
/// `(callee, caller_d_nr)` pairs onto `edges`.
///
/// Skips `Value::CallRef` (runtime function reference) — phase 5e
/// pessimises CallRef-routed callers because the actual callee is
/// not statically known.
#[allow(clippy::similar_names)]
fn collect_callees(value: &Value, caller: u32, edges: &mut Vec<(u32, u32)>) {
    match value {
        Value::Call(callee, args) => {
            edges.push((*callee, caller));
            for a in args {
                collect_callees(a, caller, edges);
            }
        }
        Value::CallRef(_, args) => {
            // The actual callee is a runtime value; skip the edge,
            // but still walk arg expressions for nested Call edges.
            for a in args {
                collect_callees(a, caller, edges);
            }
        }
        Value::Block(b) => {
            for v in &b.operators {
                collect_callees(v, caller, edges);
            }
        }
        Value::Insert(vs) => {
            for v in vs {
                collect_callees(v, caller, edges);
            }
        }
        Value::If(c, t, e) => {
            collect_callees(c, caller, edges);
            collect_callees(t, caller, edges);
            collect_callees(e, caller, edges);
        }
        Value::Loop(body) => {
            for v in &body.operators {
                collect_callees(v, caller, edges);
            }
        }
        Value::Set(_, rhs) => {
            collect_callees(rhs, caller, edges);
        }
        // Leaves and Value variants without nested expressions —
        // nothing to walk.  Conservative: any future Value variant
        // not enumerated here is treated as a leaf, missing any
        // nested calls inside it.  Phase 5e's tests cover this so
        // a missed variant surfaces as a "callers_of returns
        // empty" regression.
        _ => {}
    }
}

impl Display for Type {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Integer(s) if s.source_name().is_some() => {
                f.write_str(s.source_name().unwrap_or("integer"))
            }
            Type::Integer(IntegerSpec { min, max, .. }) => {
                f.write_str(&format!("integer({min}, {max})"))
            }
            Type::Vector(tp, link) if matches!(tp as &Type, Type::Unknown(_)) => {
                f.write_str(&format!("vector#{link:?}"))
            }
            _ => f.write_str(&format!("{self:?}").to_lowercase()),
        }
    }
}

#[derive(Debug)]
pub struct Argument {
    pub name: String,
    pub typedef: Type,
    pub default: Value, // @F17 — default parameter value
    pub constant: bool, // @F18 — const parameters (compile-time mutation check)
}

#[derive(Clone)]
#[allow(clippy::struct_excessive_bools)] // independent property flags (mutable/constant/const_field/nullable/primary); an enum would add indirection without clarity
pub struct Attribute {
    /// Name of the attribute for this definition
    pub name: String,
    pub typedef: Type,
    /// This attribute is mutable.
    pub mutable: bool,
    /// Only return the default on this field.
    pub constant: bool,
    /// This field is `const` — write-once at construction; a later reassignment
    /// is rejected. See doc/claude/plans/40-const-fields/.
    pub const_field: bool,
    /// This field is VALUE-const (`v: const T` — `const` before the TYPE): the value it
    /// holds is a read-only borrow, so every mutation THROUGH it (`s.v.x=`, `s.v[i]=`,
    /// `s.v+=`) is rejected, while a rebind `s.v = other` is allowed.  Composes with
    /// `const_field`: `const v: const T` is fully frozen.  @PLN40 Phase 2 (deep-frozen
    /// records); enforced by `validate_write`'s LHS chain-walk.
    pub value_const: bool,
    /// L7: init(expr) field — stored at creation, writable after. `$` allowed.
    pub init: bool,
    /// This attribute is allowed to be null in the substructure.
    pub nullable: bool,
    /// This attribute is holding the primary reference of its records.
    pub(crate) primary: bool,
    /// Hidden return-mechanism parameter added by `text_return` or `ref_return`.
    /// Not a user-declared parameter — should be excluded from dep propagation.
    pub hidden: bool,
    /// The initial value of this attribute if it is not given.
    pub value: Value,
    /// A constraint expression checked on every field write.
    /// Parsed from `assert(expr)` or `assert(expr, message)` in field definitions.
    pub check: Value,
    /// Optional message for a failed constraint check.
    pub check_message: Value,
    /// Post-2c: when the field's declared type was an integer alias with a
    /// `size(N)` annotation (e.g. `i32`), this holds the alias def_nr so
    /// `fill_database` / codegen can consult `forced_size(alias_nr)`.  `0`
    /// means "no alias" — fall back to the limit()-based heuristic.
    pub alias_d_nr: u32,
    /// P213: for fn-ref struct fields, the def_nr of the lambda assigned
    /// at the (single) construction site.  Used by `fill_database`'s
    /// `Type::Function` arm to look up the lambda's `closure_record`
    /// schema and register `cb__closure_rec` as `Parts::Vector(closure_kt)`.
    /// `u32::MAX` = no capturing-lambda assignment seen yet (or never
    /// will be — non-capturing case).  Heterogeneous shapes (different
    /// lambdas with different capture schemas) are rejected at the
    /// second assignment site by `set_field_check`.
    pub assigned_lambda_d_nr: u32,
    /// @PLN86 P6.8 (F8b) — the `group#right` capability links the host put on this
    /// member: a struct field's read/update/append rights, or a function parameter's
    /// `…#default` lock.  The single home for both (one fact, one place), so they
    /// round-trip through the IR store (`ATTR_LINKS`) — a warm-cached host type / library
    /// keeps its links instead of losing them with the parser side-maps.  Empty = an
    /// unlinked member.
    pub links: Vec<String>,
    /// @PLN35 — the field marked `#lexeme` on a token-enum variant carries the token's surface
    /// text, so a bare literal in a slice pattern (`[ "fn", … ]`) matches against it (see
    /// `parse_vector_match`'s literal-element path).  A PARSE-TIME marker only — set while
    /// parsing the enum, consumed when a pattern desugars; it is not written to the IR store
    /// (a round-trip defaults it to `false`, which is fine: patterns compile in-session, after
    /// the enum is parsed and before any store handoff).
    pub lexeme: bool,
}

impl Debug for Attribute {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("{}:{}", self.name, self.typedef))
    }
}

/// Plan-06 phase 5a (DESIGN.md D8.1) — purity classification of a
/// function definition for the `is_par_safe` analyser.
///
/// Set by the `#pure` and `#impure(category)` annotations parsed
/// from `default/*.loft` (and user code, future).  Phase 5b's
/// analyser uses this to short-circuit: a worker fn that calls a
/// `Pure` or `Impure(HostIo|Prng|Io)` stdlib fn is itself still
/// par-safe; calls into `Impure(ParentWrite|ParCall)` are rejected.
///
/// `Unknown` (the default) means "no annotation provided" — phase
/// 5b conservatively treats this as `Impure(ParentWrite)` to avoid
/// false-positive par-safety classifications.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Purity {
    /// No annotation — conservatively treated as parent-write
    /// impure by the analyser to avoid false positives.
    #[default]
    Unknown,
    /// `#pure` — no observable side effects, no parent-store writes.
    /// Always par-safe.
    Pure,
    /// `#impure(category)` — observable side effect, classified by
    /// category for the par-safety analyser's per-call check.
    Impure(ImpureCategory),
}

/// Plan-06 phase 5a (DESIGN.md D8.1) — sub-classification of
/// observable side effects.  Distinguishes "writes parent state"
/// (forbidden in par workers) from "has side effects but doesn't
/// write parent state" (allowed; host bridges serialise).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImpureCategory {
    /// Host I/O — log_*, print, file read/write through host
    /// bridges.  Allowed in par workers; the host serialises.
    HostIo,
    /// PRNG state mutation — random_int, random_seed, etc.  Allowed
    /// in par workers (non-deterministic across runs but correct).
    Prng,
    /// Filesystem / network I/O beyond host_io — write_file,
    /// delete_file, network ops.  Allowed in par workers; user
    /// accepts that I/O happens in parallel.
    Io,
    /// Writes to a parent-side store via its first argument
    /// (vector_add, hash_set, vector_insert, vector_remove, etc.).
    /// Compile error in par workers when first arg is non-local.
    ParentWrite,
    /// Spawns parallel workers — par, par_fold, parallel_for.
    /// Allowed if inner worker fn is par-safe (DESIGN.md D8 R2);
    /// the analyser recurses.
    ParCall,
}

#[derive(Clone, PartialEq, Debug)]
pub enum DefType {
    // Not yet known, must be filled in after the first parse pass.
    Unknown,
    // A normal function cannot be defined twice.
    Function,
    // Dynamic function, where all arguments hold references to multiple implementations we can choose
    Dynamic,
    // The possible values are EnumValue definitions in the childs.
    Enum,
    // The parent is the Enum.
    EnumValue,
    // A structure, with possibly conditional fields in the childs.
    Struct,
    // A vector with a unique content (can be a base Type, Struct, Enum or Vector)
    Vector,
    // A type definition, for now only the base types.
    Type,
    // A static constant.
    Constant,
    // A generic function template parameterised by a single type variable.
    // Not compiled until instantiated at a concrete call site.
    Generic,
    // I2: an interface declaration — a named set of required method signatures.
    // Method stubs are stored as attributes on this definition.
    // Used by bounded generics (<T: InterfaceName>) for satisfaction checking (I6).
    Interface,
}

impl Display for DefType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("{self:?}"))
    }
}

/// A group of fields on a struct/type that belong together as a single
/// logical unit.  Used **exclusively** for two patterns — tuple
/// elements and index bookkeeping triples.  Do not extend for other
/// uses without explicit user direction.
///
/// `Tuple` — synthetic `__tuple<T1,T2,…>` struct's element fields
/// `_0`, `_1`, ….  One group per tuple-shape struct; `field_indices`
/// lists every attribute in element order.  Registered by
/// [`Data::tuple_def`].
///
/// `Index` — bookkeeping triple appended to a content struct when
/// `index<T[key]>` is registered.  One group per index instance on the
/// struct (multiple indexes → multiple groups); `field_indices`
/// lists exactly `[left, right, color]` in that order.  Registered by
/// [`Stores::index`].
#[derive(Clone, Debug, PartialEq)]
pub enum LinkedFieldKind {
    Tuple,
    Index,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkedFieldGroup {
    pub kind: LinkedFieldKind,
    /// 0 for Tuple (only one tuple group per synthetic struct).
    /// 1-based index instance counter for Index (matches the `_N`
    /// suffix on `#left_N` / `#right_N` / `#color_N`).
    pub instance: u16,
    /// Indices into the host struct's field/attribute list.
    /// For Tuple: element order, length = arity.
    /// For Index: exactly `[left, right, color]`, length = 3.
    pub field_indices: Vec<u16>,
    /// Alignment the GROUP must be placed at — the MAX of every
    /// member field's alignment.  An index triple
    /// `[int4 (align 4), int4 (align 4), bool (align 1)]` has group
    /// alignment 4.  A tuple `(integer (align 8), byte (align 1))` has
    /// group alignment 8.  Pass this to the layout routine as the
    /// alignment requirement when reserving the group's contiguous
    /// block — the routine must honour it so the int/integer members
    /// land on their natural-alignment boundaries.
    pub alignment: u8,
    /// Total bytes the group occupies when laid out atomically — the
    /// sum of every member's size, **plus** any internal padding
    /// needed to align larger members after smaller ones in element
    /// order.  An index triple is `4 + 4 + 1 = 9` (no internal padding,
    /// 1-byte bool last).  A tuple `(byte, integer)` requires 7 bytes
    /// of internal padding between them: `1 + 7 + 8 = 16`.
    pub size: u16,
}

impl LinkedFieldGroup {
    /// Compute group alignment as the max of every member's
    /// alignment.  `member_aligns` lists each member's natural
    /// alignment in element order — e.g. for an index triple
    /// `[4, 4, 1]`, returns 4.
    #[must_use]
    pub fn group_alignment(member_aligns: &[u8]) -> u8 {
        member_aligns.iter().copied().max().unwrap_or(1)
    }

    /// Compute the group's total atomic size: each member at its
    /// natural-alignment offset within the group, ending at the
    /// natural-alignment-padded total.  `member_sizes_aligns` lists
    /// `(size, alignment)` per member in element order.
    ///
    /// For an index triple `[(4,4), (4,4), (1,1)]` returns 9.
    /// For a tuple `[(byte=1,1), (integer=8,8)]` returns 16 (the
    /// 1-byte byte then 7 bytes padding then 8-byte integer).
    #[must_use]
    pub fn group_size(member_sizes_aligns: &[(u16, u8)]) -> u16 {
        let mut pos: u16 = 0;
        for &(size, align) in member_sizes_aligns {
            // Pad up to the member's natural alignment.
            let align_u16 = u16::from(align.max(1));
            let rem = pos % align_u16;
            if rem != 0 {
                pos += align_u16 - rem;
            }
            pos += size;
        }
        pos
    }

    /// Number of fields in this group.  For Tuple = arity, for Index
    /// = 3 (always `[left, right, color]`).
    #[must_use]
    pub fn arity(&self) -> usize {
        self.field_indices.len()
    }

    /// Index into the host struct's field/attribute list of the
    /// `member_idx`-th group member.  Returns `None` if `member_idx`
    /// is out of range.  Safe entry point — replaces ad-hoc
    /// `group.field_indices[i]` with bounds-checked access.
    #[must_use]
    pub fn member_field_index(&self, member_idx: usize) -> Option<u16> {
        self.field_indices.get(member_idx).copied()
    }

    /// Per-member offsets inside the group, in element order.  Used
    /// by the layout routine (post-group-atomic placement) to assign
    /// each member's position relative to the group's anchor.
    /// Mirrors `group_size`'s internal packing — first member at 0,
    /// each subsequent member at the next natural-alignment offset.
    #[must_use]
    pub fn group_member_offsets(member_sizes_aligns: &[(u16, u8)]) -> Vec<u16> {
        let mut offsets = Vec::with_capacity(member_sizes_aligns.len());
        let mut pos: u16 = 0;
        for &(size, align) in member_sizes_aligns {
            let align_u16 = u16::from(align.max(1));
            let rem = pos % align_u16;
            if rem != 0 {
                pos += align_u16 - rem;
            }
            offsets.push(pos);
            pos += size;
        }
        offsets
    }
}

/// Game definition, the data cannot be changed, there can be instances with differences
#[derive(Clone)]
pub struct Definition {
    pub name: String,
    pub source: u16,
    /// Type of definition.
    pub def_type: DefType,
    /// Parent definition for `EnumValue` or `StructPart`. Initial `u32::MAX`.
    pub parent: u32,
    /// The source file position where this is defined, only allow redefinitions within the same file.
    /// This might eventually also limit access to protected internals.
    pub position: Position,
    /// Allowed attributes
    pub attributes: Vec<Attribute>,
    /// Allowed attributes on name
    pub attr_names: HashMap<String, usize>,
    /// Possible code associated with this definition. The attributes are parameters.
    pub code: Value,
    /// Related type for fields, and the return type for functions
    pub returned: Type,
    /// Whether the return type was declared `not null` (only meaningful for functions)
    pub returned_not_null: bool,
    /// Rust code
    pub rust: String,
    /// Native symbol name for `#native "symbol"` extern dispatch; empty if not native.
    pub native: String,
    /// The function's call-gate capability link — a `group#right` token written in
    /// the signature (@PLN86, e.g. `fs#read`); empty if unlinked.  The sandbox
    /// admission walk gates a TRUSTED symbol against the active profile's grants; a
    /// sandboxed def's own link is ignored (its capabilities derive from what it
    /// reaches, not a self-label).
    ///
    /// Persisted through the IR store (`DEF_CAP`) so a `#cap`-tagged stdlib loaded
    /// from the `LOFT_STDLIB_CACHE` bundle still gates correctly; mirrored in
    /// `tools/ir_schema/ir.loft` (`Definition.cap`).
    pub cap: String,
    /// Interpreter operator code
    pub op_code: u16,
    /// Position inside the generated code
    pub code_position: u32,
    /// Code length for this function
    pub code_length: u32,
    /// Entry in the known types for the database
    pub known_type: u16,
    /// Known variables inside this definition
    pub variables: Function,
    /// Whether this definition was declared with `pub`.
    pub pub_visible: bool,
    /// @PLN46 W2 — `#null_safe`: the author asserts every nullable parameter
    /// tolerates null and yields a defined result, so the undefended-fault warning
    /// is suppressed for a fault-prone expression passed DIRECTLY as an argument.
    /// Persisted through the IR store (`DEF_NULL_SAFE`) so a `#null_safe`-annotated
    /// stdlib helper loaded from the `LOFT_STDLIB_CACHE` bundle still suppresses;
    /// mirrored in `tools/ir_schema/ir.loft` (`Definition.null_safe`).
    pub null_safe: bool,
    /// @PLN102 arc C — `#superseded "Y"`: the bare name of the successor symbol
    /// this callable is superseded by (e.g. `"write_through"`).  Empty = not
    /// superseded.  Set by the `#superseded` attribute; step 1 only parses +
    /// stores it (nothing reads it yet, so it is inert).  A later step reads it
    /// at the call chokepoint to steer an OWNED-source caller toward `Y`, and a
    /// `make ci` lint checks `Y` resolves and this body is a shim over it.
    /// Persisted through the IR store (`DEF_SUPERSEDED`) so a `#superseded`
    /// stdlib symbol loaded from the `LOFT_STDLIB_CACHE` bundle keeps its mark;
    /// mirrored in `tools/ir_schema/ir.loft` (`Definition.superseded`).
    pub superseded: String,
    /// @PLN24 arc A — the C symbol a `#c` binding names, e.g. `"PQstatus"`.
    /// Empty = not a C binding.  Deliberately NOT stored in `native`: that
    /// field means "dispatch this as a Rust native symbol", and a `#c` function
    /// is not one — reusing it would route the call into the Rust bridge
    /// registry instead of leaving arc A inert.
    ///
    /// Persisted through the IR store (`DEF_C_SYMBOL`) so a cached program does
    /// not come back with the binding silently unbound; mirrored in
    /// `tools/ir_schema/ir.loft` (`Definition.c_symbol`).
    pub c_symbol: String,
    /// @PLN24 arc A — the C signature exactly as declared, e.g. `"int(void*)"`.
    /// Empty = not a C binding.
    ///
    /// Kept as the author's own spelling rather than a parsed structure,
    /// because the widths it resolves to depend on the TARGET (`long` is 64
    /// bits on Linux, 32 on Windows) — so the portable thing to persist is the
    /// text, and `c_signature::of` is the single place that turns it into
    /// widths.  Nothing may re-derive those from the loft types: `integer` is
    /// i64 whatever the C function takes, which is the whole point.
    pub c_sig: String,
    /// Definition number of the closure record struct for capturing lambdas.
    /// `u32::MAX` if this function does not capture.
    pub closure_record: u32,
    /// Plan-22 phase 01 — names of captured bindings whose value is
    /// mutated inside the lambda body.  Empty for non-capturing
    /// lambdas, for read-only captures (case A), and for non-lambda
    /// definitions.  Populated by `Parser::collect_mutated_captures`
    /// after the lambda body is parsed.  Phases 02-05 consume this
    /// to drive case B/C/D classification + lowering.
    pub mutated_captures: Vec<String>,
    /// Plan-22 phase 02d-i — names of LOCAL bindings in THIS
    /// function's scope that are captured-and-mutated by some
    /// inner lambda, where the local's type is a scalar
    /// (Integer / Text / Float / Single / Boolean / Character /
    /// plain Enum).  Populated by the accumulator pass when an
    /// inner lambda's `mutated_captures` includes scalar-typed
    /// names: we push those names onto the parent function's
    /// `scalars_to_box` so a future 02d-iii pass can rewrite the
    /// outer binding to a hidden cell.
    ///
    /// The accumulator runs in pass 1 (right after each lambda's
    /// mutation walker), so by pass 2 the parent function knows
    /// which of its locals need boxing — required because the
    /// outer-binding decision happens at variable-init time
    /// (BEFORE the lambda literal is parsed).
    ///
    /// Detection-only at phase 02d-i: the field is populated but
    /// not yet consumed.  Phase 02d-iii does the actual
    /// outer-binding rewrite.
    pub scalars_to_box: Vec<String>,
    /// I2: for generic functions — the `def_nr`s of all required interface bounds.
    /// Empty for non-generic or unbounded generic functions.  Multiple bounds (`<T: A + B>`)
    /// are stored as multiple entries; checked for conflicting method signatures at I6.
    pub bounds: Vec<u32>,
    /// DbRef into CONST_STORE for pre-built vector constants.
    /// `None` for non-constant definitions or constants that couldn't be pre-built.
    pub const_ref: Option<crate::keys::DbRef>,
    /// Post-2c: explicit `size(N)` annotation on an integer subtype
    /// (e.g. `pub type i32 = integer size(4);`).  `None` means use the
    /// limit()-based heuristic; `Some(n)` forces the stored-width to n
    /// bytes (n ∈ {1, 2, 4, 8}).
    pub forced_size: Option<u8>,
    /// Plan-06 phase 5a (DESIGN.md D8.1) — purity classification set
    /// by `#pure` / `#impure(category)` annotations.  Default
    /// `Purity::Unknown` (no annotation provided); phase 5b's
    /// `is_par_safe` analyser treats unknown as ParentWrite-impure
    /// for safety.
    pub purity: Purity,
    /// Linked-field groups on this definition's attributes.
    /// Used **exclusively** for tuple elements (one group covering all
    /// `_0`, `_1`, …) and index bookkeeping triples (one group per
    /// index instance, covering `#left_N` / `#right_N` / `#color_N`).
    /// Empty for ordinary user-defined structs.
    pub field_groups: Vec<LinkedFieldGroup>,
    /// Origin tag for definitions the compiler synthesises rather than
    /// the user declaring directly.  `None` for user-written defs
    /// (the common case); `Some(reason)` identifies the synthesis
    /// site (e.g. `"enum_dispatcher"` for the polymorphic stub
    /// `create_enum_dispatch_fn` builds when only per-variant impls
    /// exist).  Used by parser fallbacks (e.g. method-on-parent-enum
    /// dispatch in `parser/fields.rs`) that must distinguish
    /// user-declared methods from auto-generated stubs to avoid
    /// silently bypassing intentional compile-time errors.  The
    /// reason string is `&'static` for zero-cost comparison and easy
    /// grep.
    pub synthetic: Option<&'static str>,
}

impl Definition {
    // ─── @PLN11 arc C — store-backed-field read seam ───────────────────────
    //
    // Read accessors for the `Definition` fields that live in the store schema
    // (`src/ir_schema_gen.rs`).  Routing reads through these methods (rather than
    // touching the `pub` fields directly) is the precondition for swapping the
    // representation to store-backed per subsystem (§ Incremental migration):
    // when the swap lands, the method body reads the store instead of `self`,
    // and every call site is already correct.  Today they are thin field reads —
    // no behaviour change.  The codegen-DERIVED fields (`code_position` /
    // `code_length`) are intentionally NOT seamed: they are recomputed on load,
    // never stored, so they stay native.

    /// Definition name (`n_<fn>` / type name / …).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `#native "symbol"` extern symbol, or empty.
    #[must_use]
    pub fn native(&self) -> &str {
        &self.native
    }

    /// The function's `group#right` call-gate link (@PLN86), or empty if unlinked.
    #[must_use]
    pub fn cap(&self) -> &str {
        &self.cap
    }

    /// `#null_safe` (@PLN46 W2): the author asserts nullable params tolerate null.
    #[must_use]
    pub fn null_safe(&self) -> bool {
        self.null_safe
    }

    /// `#superseded "Y"` (@PLN102 arc C): the bare successor-symbol name this
    /// callable is superseded by, or empty if not superseded.  Inert in step 1
    /// (parsed + stored only); a later step reads it to steer owned-source callers.
    #[must_use]
    pub fn superseded(&self) -> &str {
        &self.superseded
    }

    /// If this is a method — stored `t_<LEN><Type>_<method>` (LEN = chars in the
    /// receiver type name) — the `t_<LEN><Type>_` prefix (e.g. `t_4text_` for a
    /// `text` method); `None` for a free fn (`n_…`) or operator (`Op…`).
    #[must_use]
    pub fn method_type_prefix(&self) -> Option<&str> {
        let rest = self.name.strip_prefix("t_")?;
        let nd = rest.chars().take_while(char::is_ascii_digit).count();
        let len: usize = rest.get(..nd)?.parse().ok()?;
        // `t_`(2) + digits(nd) + type(len) + `_`(1) + method. The byte AFTER the type name
        // must be the `_` separator — the other four manglers (`api_surface::method_name`,
        // `generation::is_t_param_stub`, `parser::h5_names_a_generic_template`) require it; a
        // longer `rest` alone is not enough, so `t_4textX…` must NOT be read as a method.
        (rest.as_bytes().get(nd + len) == Some(&b'_')).then_some(&self.name[..=2 + nd + len])
    }

    /// The user-facing name of this definition — the internal `n_` (free fn) or
    /// `t_<LEN><Type>_` (method) prefix stripped, so `t_4text_contains` reads as
    /// `contains`.  Used by diagnostics (the @PLN102 arc-C steer + fold lint).
    #[must_use]
    pub fn display_name(&self) -> &str {
        match self.method_type_prefix() {
            Some(p) => &self.name[p.len()..],
            None => self.name.strip_prefix("n_").unwrap_or(&self.name),
        }
    }

    /// Source-file id this definition was parsed from.
    #[must_use]
    pub fn source(&self) -> u16 {
        self.source
    }

    /// Source position where this definition is declared.
    #[must_use]
    pub fn position(&self) -> &Position {
        &self.position
    }

    /// The definition's attributes (struct fields / function parameters).
    #[must_use]
    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    /// The code body (function body / field default / …).
    #[must_use]
    pub fn code(&self) -> &Value {
        &self.code
    }

    /// The related/return type.
    #[must_use]
    pub fn returned(&self) -> &Type {
        &self.returned
    }

    /// Cluster-A (return/bind ownership) — THE one return-ownership query,
    /// shared by both backends.  Answers: *does this function's heap return
    /// BORROW a visible parameter's store (caller must copy / must not free
    /// the source), or is it OWNED/fresh (caller adopts; the callee's source
    /// store is freed via the `0x8000` source-free bit)?*
    ///
    /// Reads `returned().depend()` against the parameter list:
    /// - a dep naming a VISIBLE (non-hidden) attribute → borrowed view (the
    ///   return aliases that parameter; freeing the source would corrupt the
    ///   caller's arg).
    /// - an empty dep → not borrowed (owned/fresh → the source-free bit is set
    ///   and the caller adopts).
    /// - a HIDDEN-only dep (a `ref_return`-promoted `__retbuf`/work-ref attr,
    ///   or the `["??"]` one-buffer marker) → NOT a borrow: the callee minted a
    ///   fresh store into its own buffer param, so the source-free bit stays set.
    /// - an OUT-OF-RANGE dep (def-space list contaminated with a frame var — the
    ///   DEPS_INVENTORY corpus probe found zero of these) → conservatively
    ///   borrowed (never free a maybe-borrowed source), with a debug scream.
    ///
    /// This collapses the three structurally-identical derivations that used to
    /// live separately (interp `state/codegen.rs` ×2, native
    /// `generation/dispatch.rs` ×1) into one fact — see
    /// [STABILITY_REDFLAGS](../doc/claude/STABILITY_REDFLAGS.md) Cluster A (A.4).
    /// Both backends translate the SAME answer, so they cannot diverge on the
    /// out-of-range / hidden-only edge.
    #[must_use]
    pub fn returns_borrowed_view(&self) -> bool {
        let attrs = self.attributes();
        self.returned().depend().iter().any(|&d| {
            // A `CALLEE_FRAME_BIT`-tagged note is a closure-internal frame var,
            // never a real attr index; such deps live only on `Type::Function`
            // returns, which the heap-return callers gate out before reaching
            // here.  Assert that invariant so a future caller that violates it
            // is caught rather than silently mis-reading the tag as out-of-range.
            debug_assert!(
                d == u16::MAX || d & 0x8000 == 0,
                "returns_borrowed_view: callee-frame-tagged return dep {d} on \
                 '{}' (closure-internal note reached a heap-return ownership read)",
                self.name()
            );
            // Out-of-range (`None`) = a def-space dep list contaminated with a
            // frame var (the DEPS_INVENTORY corpus probe found zero); scream in
            // debug, then answer conservatively borrowed (never free a
            // maybe-borrowed source).  A visible attr → borrows it; a hidden
            // attr → not a borrow.
            debug_assert!(
                attrs.get(d as usize).is_some(),
                "dep-space violation: returned dep {d} outside attr range of '{}'",
                self.name()
            );
            attrs.get(d as usize).is_none_or(|a| !a.hidden)
        })
    }

    /// Cluster-A.3 (OWNERSHIP_MODEL row 102) — THE adopt-vs-copy answer for a
    /// heap binding from a struct/Reference-returning call: *may the caller
    /// ADOPT the callee's returned store directly (no deep copy), or must it
    /// COPY into a fresh store the binding owns?*
    ///
    /// ADOPT (returns `true`) iff the return is a genuinely FRESH store the
    /// callee minted with no tie to any passed buffer:
    /// - an **empty** return dep (`fn mk() -> Box { Box { … } }`), or
    /// - the **`["??"]` one-buffer marker** (`Deps::pointer_marker()`, a single
    ///   `u16::MAX`): an NRVO'd by-value return whose hidden `__ref`/`__vdb`
    ///   buffer the caller already frees, so adopting it is safe.
    ///
    /// COPY (returns `false`) iff the return dep names a REAL attribute index —
    /// whether a VISIBLE parameter (`fn idb(b) -> Box { b }`, dep `["b"]` — the
    /// return aliases the arg) OR a HIDDEN `ref_return`-promoted work-ref attr
    /// (`fn render(p) -> Canvas { cv = …; cv }`, dep `["cv"]` — the return IS
    /// the caller-passed `__ref_N` buffer, which the caller REUSES across loop
    /// iterations; adopting it without a copy aliases every iteration onto the
    /// recycled buffer).
    ///
    /// This is a STRICTLY BROADER copy condition than
    /// [`Self::returns_borrowed_view`]: that method answers the *source-free-bit*
    /// question (does the return borrow a VISIBLE param?) and treats both hidden
    /// cases (`["??"]` and `["cv"]`) alike as "not a borrow".  The adopt-vs-copy
    /// decision must split them — `["??"]` adopts, `["cv"]` copies — so it reads
    /// THIS predicate, not `returns_borrowed_view`.
    #[must_use]
    pub fn return_adopts_fresh_store(&self) -> bool {
        let deps = self.returned().depend();
        // Empty → owned/fresh → adopt.
        if deps.is_empty() {
            return true;
        }
        // The `["??"]` one-buffer marker (a lone `u16::MAX`, the
        // `Deps::pointer_marker()` shape) is an NRVO'd hidden-buffer return the
        // caller already frees — adopt.  Any OTHER non-empty dep names a real
        // attr (visible param OR hidden work-ref) the return is tied to — copy.
        deps.len() == 1 && deps[0] == u16::MAX
    }

    /// Interpreter operator code.
    #[must_use]
    pub fn op_code(&self) -> u16 {
        self.op_code
    }

    /// Index into the database's known-types schema.
    #[must_use]
    pub fn known_type(&self) -> u16 {
        self.known_type
    }

    /// The per-function variable table.
    #[must_use]
    pub fn variables(&self) -> &Function {
        &self.variables
    }

    /// The kind of definition (function / struct field / enum value / …).
    /// Returned by value (cheap — a unit-variant enum): a store read decodes an
    /// integer discriminant into a fresh `DefType`, never a borrow.
    #[must_use]
    pub fn def_type(&self) -> DefType {
        self.def_type.clone()
    }

    /// Inline Rust body (`#rust "…"`), or empty for a non-native definition.
    #[must_use]
    pub fn rust(&self) -> &str {
        &self.rust
    }

    /// Parent definition (`EnumValue` / `StructPart`), or `u32::MAX` if none.
    #[must_use]
    pub fn parent(&self) -> u32 {
        self.parent
    }

    /// Closure-record def_nr for a capturing lambda, or `u32::MAX`.
    #[must_use]
    pub fn closure_record(&self) -> u32 {
        self.closure_record
    }

    /// @PLAN22 — names of captured bindings mutated inside this lambda body.
    #[must_use]
    pub fn mutated_captures(&self) -> &[String] {
        &self.mutated_captures
    }

    /// @PLAN22 — local scalar bindings captured-and-mutated by an inner lambda.
    #[must_use]
    pub fn scalars_to_box(&self) -> &[String] {
        &self.scalars_to_box
    }

    /// Synthesis origin tag for compiler-generated defs; `None` for user code.
    #[must_use]
    pub fn synthetic(&self) -> Option<&'static str> {
        self.synthetic
    }

    #[must_use]
    pub fn is_operator(&self) -> bool {
        matches!(self.def_type, DefType::Function)
            && self.name.len() > 2
            && self.name.starts_with("Op")
            && self.name[2..3]
                .chars()
                .next()
                .unwrap_or_default()
                .is_uppercase()
    }

    /// Tuple-element field group on this synthetic `__tuple<…>` struct,
    /// or `None` for non-tuple defs.  Reuses the `LinkedFieldGroup`
    /// infrastructure rather than parsing the `__tuple<` name prefix
    /// or scanning attributes named `_0`, `_1`, ….
    #[must_use]
    pub fn tuple_group(&self) -> Option<&LinkedFieldGroup> {
        self.field_groups
            .iter()
            .find(|g| matches!(g.kind, LinkedFieldKind::Tuple))
    }

    #[must_use]
    pub fn original_name(&self) -> String {
        if self.def_type == DefType::Function {
            if self.name.starts_with("t_") {
                if let Ok(nr) = self.name[2..4].parse::<u8>() {
                    self.name[5 + nr as usize..].to_string()
                } else if let Ok(nr) = self.name[2..3].parse::<u8>() {
                    self.name[4 + nr as usize..].to_string()
                } else {
                    self.name[2..].to_string()
                }
            } else {
                self.name[2..].to_string()
            }
        } else {
            self.name.clone()
        }
    }

    #[must_use]
    pub fn header(&self, data: &Data, d_nr: u32) -> String {
        let mut res = "fn ".to_string();
        res += &self.name;
        res += "(";
        for (a_nr, a) in self.attributes.iter().enumerate() {
            if a_nr > 0 {
                res += ", ";
            }
            res += &a.name;
            res += ":";
            res += &a.typedef.argument(data, d_nr);
        }
        res += ")";
        if self.returned != Type::Void {
            res += " -> ";
            res += &self.returned.argument(data, d_nr);
        }
        res
    }
}

#[derive(PartialEq, Debug)]
pub enum Context {
    Argument,
    Reference,
    Result,
    Constant,
    Variable,
}

/// @PLN24 arc D/G — one C library a package declared, as every consumer of the
/// declaration needs to see it.
///
/// A named struct rather than a tuple because four sites read it and each reads
/// a different field for a different purpose — the interpreter's load, the
/// `--native` link line, the `--native` emission, and the availability query.
/// A positional `bool` in a tuple is read wrong at one of them eventually, and
/// this plan's counted risk is that such a mistake is SILENT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CLibrary {
    /// The soname as the dynamic linker knows it (`libpq.so.5`), or a path
    /// relative to `pkg_dir` for a library the package ships itself.
    pub name: String,
    /// The declaring package's directory. It travels with the name because a
    /// library may ship its own `.so` beside its `.loft`, so the name alone
    /// cannot be resolved.
    pub pkg_dir: String,
    /// `[c] optional-libs` rather than `[c] libs`: the package works without
    /// it, so it is NOT linked and NOT opened until a symbol needs it.
    ///
    /// The author's claim about their own library, never inferred from whether
    /// a `dlopen` happened to succeed — that would make one package required on
    /// one machine and optional on the next.
    pub optional: bool,
}

#[allow(dead_code)]
#[derive(Clone)]
/// The immutable data of a parsed loft program
pub struct Data {
    pub definitions: Vec<Definition>,
    /// Index on definitions on name
    def_names: HashMap<(String, u16), u32>,
    use_names: HashMap<String, u16>,
    /// Every import that has been applied, so [`rebuild_indices`](Self::rebuild_indices)
    /// can replay it.
    ///
    /// An import is an ALIAS in `def_names` — `(name, importing_source) → def_nr` — and
    /// therefore NOT derivable from `definitions`, each of which knows only its own
    /// source.  A rebuild that reads `definitions` alone silently drops every one, and
    /// the REPL rebuilds on each `savepoint`/`rewind`, so a single rolled-back probe
    /// used to make every `use`d library name unresolvable for the rest of the session
    /// (moros H13's residual: a library call in `eval` answered "couldn't evaluate"
    /// while a same-file call worked).
    ///
    /// Deliberately NOT cleared by [`reset`](Self::reset): a REPL/debugger expression
    /// is parsed as a fresh source that re-declares no `use`, and inheriting the
    /// program's imports is exactly what lets it name the frame's own vocabulary.
    applied: Vec<AppliedImport>,
    /// Current source file
    pub source: u16,
    /// @PLN101 — struct def_nrs declared `value struct`: a value (copy) type stored inline
    /// wherever it lives (record field / vector element already inline out-of-the-box), never
    /// aliased via a DbRef, non-null. A thin marker (a set, not a Definition field — those
    /// serialize) consulted by the few value-semantics chokepoints.
    pub value_structs: HashSet<u32>,
    used_definitions: HashSet<u32>,
    used_attributes: HashSet<(u32, usize)>,
    /// This definition is referenced by a specific definition, the code is used to update this
    referenced: HashMap<u32, (u32, Value)>,
    /// Static data
    statics: Vec<u8>,
    pub(crate) op_codes: u16,
    possible: HashMap<String, Vec<u32>>,
    pub(crate) operators: HashMap<u16, u32>,
    /// PKG.4: native function symbols — loft function name → Rust symbol path.
    /// Populated when packages with `[native.functions]` are loaded.
    /// Keys are the user-facing loft names (e.g. `save_png`), not the internal
    /// `n_save_png` or `t_8graphics_save_png` forms.
    pub native_symbols: HashMap<String, String>,
    /// PKG.4: native package crate directories — (`crate_name`, `pkg_dir`).
    /// Used to construct `--extern` flags for `rustc`.
    pub native_packages: Vec<(String, String)>,
    /// @PLN24 arc D — every C library a loaded package declares, and whether it
    /// is required or optional.
    ///
    /// Read by the interpreter (which `dlopen`s them so `#c` symbols resolve),
    /// by `--native` (which links or lazily resolves them for the same symbols),
    /// and by the availability query. One parse, one flag, every reader — the
    /// plan counts `N × silence` at exactly these re-assertion sites.
    pub c_libraries: Vec<CLibrary>,
    /// Map from `#native "symbol"` names to the Rust crate that provides them.
    /// Populated when a package declares `[native] crate` in loft.toml.
    /// Used by native codegen to emit `crate::symbol(args)` calls.
    pub native_symbol_crates: HashMap<String, String>,
    /// lib_plan-29 W1c: WASM bridge package directories — (`crate_name`,
    /// `pkg_dir`).  Populated from each loaded package's `[wasm.bridge].crate`.
    /// The `--html` driver builds `<pkg_dir>/wasm/` to a
    /// `wasm32-unknown-unknown` rlib and links it via `--extern`.
    pub wasm_bridge_packages: Vec<(String, String)>,
    /// lib_plan-29 W1c: WASM bridge routes — loft `#native` symbol
    /// (e.g. `n_load_png`) → `(crate_name, bridge_fn)`.  Read by
    /// `src/generation/mod.rs::output_native_direct_call` for the
    /// `wasm_browser=true` path; replaces the hard-coded `WASM_BRIDGE_FNS`
    /// const.
    pub wasm_bridge_routes: HashMap<String, (String, String)>,
    /// lib_plan-29 W2: absolute paths to per-library JS host-imports
    /// files (resolved from `[wasm.bridge].host_js` relative to each
    /// package root).  The `--html` driver reads each file and
    /// concatenates it into the HTML preamble; the bundled JS pushes
    /// a registration callback onto `globalThis.LOFT_WASM_EXTENSIONS`
    /// which the preamble dispatches after `buildLoftImports` returns.
    pub wasm_bridge_host_js_files: Vec<String>,
    /// Plan-06 phase 5b' (DESIGN.md D12) — lazy caller-graph cache.
    /// Maps callee def_nr → list of caller def_nrs.  Built once on
    /// first `callers_of` call by walking every user fn's body and
    /// collecting `Value::Call` edges.  `OnceLock` (not RefCell) so
    /// `Data` stays `Sync` — required because tests park `Data` in
    /// a process-wide `OnceLock<(Data, Stores)>` and parallel
    /// workers read from a `&Data` across threads.
    caller_index: std::sync::OnceLock<HashMap<u32, Vec<u32>>>,
}

#[must_use]
pub fn v_if(test: Value, t: Value, f: Value) -> Value {
    Value::If(Box::new(test), Box::new(t), Box::new(f))
}

#[must_use]
pub fn v_set(var: u16, value: Value) -> Value {
    Value::Set(var, Box::new(value))
}

#[must_use]
pub fn v_block(operators: Vec<Value>, result: Type, name: &'static str) -> Value {
    Value::Block(Box::new(Block {
        name,
        operators,
        result,
        scope: u16::MAX,
        var_size: 0,
    }))
}

#[must_use]
pub fn v_loop(operators: Vec<Value>, name: &'static str) -> Value {
    Value::Loop(Box::new(Block {
        name,
        operators,
        result: Type::Void,
        scope: u16::MAX,
        var_size: 0,
    }))
}

impl Display for Definition {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.name, self.def_type)
    }
}

impl Default for Data {
    fn default() -> Self {
        Self::new()
    }
}

struct Into {
    str: String,
}

impl Write for Into {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.str += &String::from_utf8_lossy(buf);
        Ok(self.str.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.write(buf)?;
        Ok(())
    }
}

#[allow(dead_code)]
/// @T0.3 — the loft type a newcomer's habit from another language means.
///
/// Suggestion only: these names stay **undefined**, they are not aliases you can
/// write (making `int` legal is a language question, not a diagnostic one — see
/// `DESIGN_DECISIONS.md`).  Every entry is a name verified to be UNDEFINED in
/// loft; the width types that ARE legal (`i8`/`i16`/`i32`/`u8`/`u16`/`u32`) are
/// deliberately absent, since a legal name never reaches an unknown-type error.
pub(crate) fn builtin_type_alias(name: &str) -> Option<&'static str> {
    Some(match name {
        // Rust / C / Java / Go habits for the 64-bit integer loft calls `integer`.
        "int" | "i64" | "u64" | "long" => "integer",
        // `float` is 64-bit and `single` is 32-bit, so f64→float and f32→single.
        "f64" | "double" => "float",
        "f32" => "single",
        "str" | "string" => "text",
        "bool" => "boolean",
        "char" => "character",
        _ => return None,
    })
}

impl Data {
    /// @PLN11 arc D — serialize this `Data` to a file-backed IR store at
    /// `path` (zero-copy-loadable via [`Data::open`]).  Thin wrapper over
    /// [`crate::ir_store::save_data`].
    ///
    /// # Errors
    /// Propagates file I/O errors from the store writer.
    #[cfg(feature = "mmap")]
    pub fn save(&self, path: &str) -> std::io::Result<()> {
        crate::ir_store::save_data(self, path)
    }

    /// @PLN11 arc D — load a `Data` from a file-backed IR store written by
    /// [`Data::save`], by `mmap`-ing the file and rebuilding the native graph —
    /// no re-parse.  Thin wrapper over [`crate::ir_read::open_data`].
    ///
    /// # Errors
    /// Returns `NotFound` if `path` does not exist (cache miss → caller parses).
    #[cfg(feature = "mmap")]
    pub fn open(path: &str) -> std::io::Result<Data> {
        crate::ir_read::open_data(path)
    }

    /// map a vector's content `Type` to a narrow
    /// database element type-nr when the content is a `Type::Integer`
    /// with a `forced_size` annotation that [`IntegerSpec::vector_narrow_width`]
    /// accepts (currently 1 and 4 bytes; 2 opens in Phase 4b).
    ///
    /// Returns `None` for:
    /// - non-Integer content (structs, enums, nested vectors, …);
    /// - `Type::Integer` without `forced_size` (plain `integer`,
    ///   `integer limit(...)`);
    /// - `forced_size` values outside the narrow gate (today `Some(2)`
    ///   and larger).
    ///
    /// The caller falls back to the default wide storage (the
    /// content's own `known_type`, or the plain-`integer` slot) when
    /// this returns `None`.  Single source of truth for narrow
    /// detection — invoked by `typedef.rs::fill_database`'s Vector
    /// arm for struct fields AND by `Parser::vector_of` for locals,
    /// parameters, return types, and literals.
    // `self` is not currently read but the helper belongs on `Data`
    // semantically — future refactors (e.g. looking up an alias's
    // captured forced_size via a Data-side registry) will need it.
    #[allow(clippy::unused_self)]
    pub fn narrow_vector_content(
        &self,
        content: &Type,
        database: &mut crate::database::Stores,
    ) -> Option<u16> {
        // A nullable narrow element (`vector<u8?>`) reserves a null sentinel the
        // same way a nullable FIELD does — peel the `Optional` and register the
        // NULLABLE narrow Parts so the element can hold null (@PLN25 item 2).  A
        // non-nullable `vector<u8>` element stays raw (full range, no sentinel).
        let narrow = match content {
            Type::Integer(spec) => Some((spec, false)),
            Type::Optional(inner) => match &**inner {
                Type::Integer(spec) => Some((spec, true)),
                _ => None,
            },
            _ => None,
        };
        if let Some((spec, nullable)) = narrow {
            let n = spec.vector_narrow_width()?;
            return match n {
                1 => Some(database.byte(spec.min, nullable)),
                // a nullable 2-byte element uses the `+1` sentinel encoding
                // (`Parts::Short`), matching the nullable field; the non-null
                // element stays direct (`Parts::ShortRaw`, full 65536 range).
                2 if nullable => Some(database.short(spec.min, true)),
                2 => Some(database.short_raw(spec.min, false)),
                4 => Some(database.int(spec.min, nullable)),
                _ => None,
            };
        }
        // Plan-06 ARC.md A6.c — fn-ref vector elements are 4-byte
        // d_nrs (`element_stack_size(Type::Function) = 4`).  The previous
        // routing via `vector_of` → `type_elm(Function)` →
        // `def_nr("i32").known_type` lands on a placeholder type
        // with `size = 0`, so `vector_append`'s stride is 0 and
        // every literal element overwrites offset 8 — yielding a
        // `length=3` vector whose elements after the last write are
        // all the SAME d_nr (the last one written) at offset 8 with
        // zeros at offsets 12 and 16.  Routing through
        // `database.int(0, false)` (Parts::Int with `size = 4`)
        // makes `vector_append` step through the storage in 4-byte
        // increments, matching `OpSetInt4`'s narrow writes.
        if matches!(content, Type::Function(_, _, _)) {
            return Some(database.int(0, false));
        }
        None
    }

    /// Use this for the schema type id of a vector ELEMENT whose loft type is
    /// `content` — the ONE derivation of that fact.
    ///
    /// Three rules, in order: a narrow numeric (or fn-ref) leaf resolves through
    /// [`Data::narrow_vector_content`]; a NESTED vector recurses, so the inner
    /// element's width survives; anything else uses the leaf's own registered
    /// `known_type`.
    ///
    /// `None` means "not derivable yet" — the leaf has no registered type id
    /// (a forward reference, a generic type variable, an unresolved content
    /// type).  Each caller keeps its own recovery for that case, because the
    /// options differ: `typedef.rs::fill_database` can fill the missing type on
    /// the spot, while `Parser::vector_of` must bake a sentinel and let the
    /// later fill pass re-derive.
    ///
    /// Every writer AND reader of a vector element type routes here, which is
    /// the point: the three independent derivations this replaces agreed only
    /// when the element happened to be 8 bytes wide, so a declared
    /// `vector<vector<u16>>` REGISTERED as `vector<vector<integer>>` and the
    /// renderer read two 2-byte elements as one 8-byte slot (loft#624 nested,
    /// the named remainder of the plan-58 / loft#437 / #457 / #483 family —
    /// `doc/claude/plans/nested-narrow-width/`).
    pub fn vector_element_type(
        &self,
        content: &Type,
        database: &mut crate::database::Stores,
    ) -> Option<u16> {
        if let Some(narrow) = self.narrow_vector_content(content, database) {
            return Some(narrow);
        }
        // A nested vector element is itself a vector.  `type_elm` collapses a
        // level here (`Vector(inner)` → `type_def_nr(inner)`), which loses the
        // inner width; recurse instead so the registered outer content type is
        // a real `vector<<inner storage>>`.
        if let Type::Vector(inner, _) = content.base() {
            let elem = self.vector_element_type(inner, database)?;
            return Some(database.vector(elem));
        }
        let c_nr = self.type_elm(content);
        if c_nr == u32::MAX {
            return None;
        }
        let c_tp = self.def(c_nr).known_type();
        if c_tp == u16::MAX { None } else { Some(c_tp) }
    }

    #[must_use]
    pub fn new() -> Data {
        Data {
            definitions: Vec::new(),
            def_names: HashMap::new(),
            use_names: HashMap::new(),
            applied: Vec::new(),
            source: STD_SOURCE,
            value_structs: HashSet::new(),
            used_definitions: HashSet::new(),
            used_attributes: HashSet::new(),
            referenced: HashMap::new(),
            statics: Vec::new(),
            op_codes: 0,
            possible: HashMap::new(),
            operators: HashMap::new(),
            native_symbols: HashMap::new(),
            native_packages: Vec::new(),
            c_libraries: Vec::new(),
            native_symbol_crates: HashMap::new(),
            wasm_bridge_packages: Vec::new(),
            wasm_bridge_routes: HashMap::new(),
            wasm_bridge_host_js_files: Vec::new(),
            caller_index: std::sync::OnceLock::new(),
        }
    }

    pub fn reset(&mut self) {
        self.use_names.clear();
        self.source = STD_SOURCE;
        self.use_names.insert("std".to_string(), STD_SOURCE);
    }

    /// @PLN12 phase 02 — transactional rollback for the REPL statement parser.
    ///
    /// Drop every definition added since index `keep` and rebuild the derived
    /// lookup tables (`def_names`, operators, …) from what remains, so a
    /// statement that failed to parse leaves `Data` exactly as it was before
    /// the attempt.  `keep` is the `definitions()` count captured before the
    /// `parse_statement` attempt ran.
    pub(crate) fn rollback_to(&mut self, keep: u32) {
        if (keep as usize) < self.definitions.len() {
            // @PLN120 E.4 guard — the import aliases that MUST survive this rollback:
            // a `def_names` entry whose definition lives in another source and is not
            // itself being truncated away.  Captured before the rebuild, checked after.
            //
            // NOT `#[cfg(debug_assertions)]`: `[profile.dev.package.loft]` turns those
            // off inside this library (they cost ~270x on the store hot paths), so a
            // cfg-gated check here would be dead in `target/debug/loft` AND under
            // `cargo test` — it would only ever run in a release-DA build.  The
            // project's convention for a load-bearing invariant is a plain `assert!`,
            // and this one is load-bearing: a dropped alias makes every `use`d name
            // silently unresolvable for the rest of the session.  Cost is bounded by
            // the `applied.is_empty()` gate — a program with no `use` pays one check.
            //
            // Narrower than `derived_indices_diff` (the cache round-trip oracle) on
            // purpose: a rollback legitimately drops definitions, so a whole-`Data`
            // comparison would report every truncated def as a divergence and drown
            // the one thing that matters.  The property here is only "the rebuild
            // reproduced what it could still reproduce".
            let expected: Vec<(String, u16, u32)> = if self.applied.is_empty() {
                Vec::new()
            } else {
                self.def_names
                    .iter()
                    .filter(|((_, src), nr)| {
                        **nr < keep
                            && self
                                .definitions
                                .get(**nr as usize)
                                .is_some_and(|d| d.source != *src)
                    })
                    .map(|((n, s), &nr)| (n.clone(), *s, nr))
                    .collect()
            };
            self.definitions.truncate(keep as usize);
            self.rebuild_indices();
            for (name, src, nr) in expected {
                assert_eq!(
                    self.def_names.get(&(name.clone(), src)).copied(),
                    Some(nr),
                    "a rollback dropped the import alias `{name}` visible from source                      {src} (definition #{nr}).  `rebuild_indices` reconstructs                      `def_names` from `definitions`, which know only their own source,                      so any cross-source binding has to be replayed — see                      `Data::replay_imports`.  A dropped alias makes every `use`d name                      unresolvable for the rest of the session."
                );
            }
        }
    }

    /// @PLAN28 Step 2 (C3) — rebuild the derived lookup indices from
    /// `definitions` alone, for the startup-cache load path.
    ///
    /// These indices are pure functions of `definitions` and are never
    /// stored in the JSON snapshot; after a decoder restores `definitions`
    /// (and each `Definition.op_code`, serialised by the Definition codec)
    /// this re-derives them so the loaded `Data` matches a fresh parse:
    ///
    /// * `def_names` — `(name, source) -> def_nr`, inserted in definition
    ///   order.  Non-variant kinds are unique per source; an enum VARIANT
    ///   keeps a FIRST-wins key (@PLN22 Phase 1 — reachable as a
    ///   type/constructor, while a bare variant VALUE resolves via context),
    ///   mirroring `add_def`.
    /// * `operators` — `op_code -> def_nr`, and `op_codes` — the
    ///   next-free counter (`max assigned op_code + 1`).  `op_code` values
    ///   themselves are restored from each `Definition` (not recomputed).
    /// * `possible` — operator overload sets, mirroring `add_op`'s fill
    ///   (operator defs only, walked in definition order).
    ///
    /// `caller_index` is intentionally left as its lazy `OnceLock` — it
    /// rebuilds on first `callers_of`.  Cross-source import bindings (the
    /// `insert_or_replace_stub` path) are NOT reproduced here: a
    /// whole-stdlib / whole-bundle snapshot is single-pass and uniform, so
    /// the `add_def`-level inserts are sufficient.  Multi-library import
    /// reconciliation is a later extension if per-library snapshots land.
    /// Rebuild the derived lookup indices from `definitions`.
    ///
    /// Ends by replaying the retained imports ([`applied`](Self::applied)): a
    /// definition knows only its own source, so the loop below can restore
    /// `(name, own_source)` but never the `(name, importing_source)` alias an import
    /// creates.  Without the replay a rollback drops every `use`d name.
    pub(crate) fn rebuild_indices(&mut self) {
        self.def_names.clear();
        self.operators.clear();
        self.possible.clear();
        let mut max_op: i32 = -1;
        for d_nr in 0..self.definitions.len() as u32 {
            let def = &self.definitions[d_nr as usize];
            // @PLN22 Phase 1 — mirror add_def: an enum variant keeps a FIRST-wins
            // flat key (reachable as a type/constructor); a later same-key variant
            // does not overwrite it.  Every other def kind is unique per source.
            if def.def_type == DefType::EnumValue {
                self.def_names
                    .entry((def.name.clone(), def.source))
                    .or_insert(d_nr);
            } else {
                self.def_names.insert((def.name.clone(), def.source), d_nr);
            }
            if def.is_operator() {
                if def.op_code != u16::MAX {
                    self.operators.insert(def.op_code, d_nr);
                    max_op = max_op.max(i32::from(def.op_code));
                }
                for op in OPERATORS {
                    if def.name.starts_with(op) {
                        self.possible
                            .entry((*op).to_string())
                            .or_default()
                            .push(d_nr);
                    }
                }
            }
        }
        self.op_codes = (max_op + 1) as u16;
        if self.use_names.is_empty() {
            self.use_names.insert("std".to_string(), STD_SOURCE);
        }
        // The loop above restored each definition under its own source only; an
        // import's cross-source alias has to be re-applied.
        self.replay_imports();
    }

    /// Cache-verify oracle: compare the DERIVED indices (rebuilt by
    /// `rebuild_indices` from the serialized `definitions`, and NOT themselves
    /// serialized) against `other`.  A warm cache load rebuilds these, so any
    /// binding the fresh parse holds that the rebuild can't reproduce — a
    /// cross-source `def_names` entry, the `use_names` module map — is a silent
    /// round-trip gap that `compare_data` (definitions + header only) misses.
    /// Returns one line per divergence; empty = identical.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn derived_indices_diff(&self, other: &Self) -> Vec<String> {
        let mut out = Vec::new();
        let name_of = |d: &Self, v: u32| {
            d.definitions
                .get(v as usize)
                .map_or("<oob>", |x| x.name.as_str())
                .to_string()
        };
        for (k, v) in &self.def_names {
            match other.def_names.get(k) {
                None => out.push(format!(
                    "def_names: (\"{}\", src {}) -> #{} '{}' present in FRESH, MISSING in loaded",
                    k.0,
                    k.1,
                    v,
                    name_of(self, *v)
                )),
                Some(ov) if ov != v => out.push(format!(
                    "def_names: (\"{}\", src {}) fresh=#{} loaded=#{}",
                    k.0, k.1, v, ov
                )),
                _ => {}
            }
        }
        for (k, v) in &other.def_names {
            if !self.def_names.contains_key(k) {
                out.push(format!(
                    "def_names: (\"{}\", src {}) -> #{} EXTRA in loaded (not in fresh)",
                    k.0, k.1, v
                ));
            }
        }
        if self.operators != other.operators {
            out.push(format!(
                "operators map differs: fresh {} entries, loaded {} entries",
                self.operators.len(),
                other.operators.len()
            ));
        }
        if self.possible != other.possible {
            out.push(format!(
                "possible map differs: fresh {} keys, loaded {} keys",
                self.possible.len(),
                other.possible.len()
            ));
        }
        if self.use_names != other.use_names {
            out.push(format!(
                "use_names module map differs: fresh {:?}, loaded {:?}",
                self.use_names, other.use_names
            ));
        }
        out
    }

    #[must_use]
    pub fn get_source(&self, name: &str) -> u16 {
        if let Some(nr) = self.use_names.get(name) {
            *nr
        } else {
            u16::MAX
        }
    }

    /// @P379 — a library-qualified database name for a struct/enum-value
    /// definition, e.g. `"moros_map::Chunk"`.  Used by database type
    /// registration to disambiguate two libraries that each define a
    /// struct of the same bare name (function access is already namespaced
    /// per library; this gives the flat database type table the same
    /// namespacing).  Falls back to `"src<N>::<name>"` if the source id
    /// has no recorded library short-name (keeps the key unique either way).
    #[must_use]
    pub fn qualified_type_name(&self, d_nr: u32) -> String {
        let def = self.def(d_nr);
        for (lib, &id) in &self.use_names {
            if id == def.source && !lib.is_empty() && lib != "std" {
                return format!("{lib}::{}", def.name);
            }
        }
        format!("src{}::{}", def.source, def.name)
    }

    /// @P379 — the native backend emits each loft function as a flat
    /// `n_<name>` Rust symbol.  Two libraries that each define a function of
    /// the same name would produce duplicate-definition Rust (`E0428`).
    /// Rename the higher-source duplicates to a source-qualified symbol
    /// (`n_s<N>_<name>`) so generated code has unique names.  Calls resolve
    /// by `d_nr` → `def.name`, so renaming the definition keeps every call
    /// site consistent automatically.
    ///
    /// No-op unless two distinct sources define the same function name, so
    /// single-library / non-colliding programs are byte-identical.  The
    /// lowest-source definer keeps the bare name (stdlib is source 0, so a
    /// user fn never displaces a stdlib symbol).  Must run AFTER the two-pass
    /// parse and BEFORE native emit.  Idempotent (a renamed `n_s<N>_…` symbol
    /// no longer collides, so a second call is a no-op).
    pub fn namespace_colliding_native_fns(&mut self) {
        let mut by_name: HashMap<String, Vec<(u32, u16)>> = HashMap::new();
        for d in 0..self.definitions() {
            let def = self.def(d);
            // user loft function: has a body and the `n_` user-fn prefix.
            if def.code != Value::Null && def.name.starts_with("n_") {
                by_name
                    .entry(def.name.clone())
                    .or_default()
                    .push((d, def.source));
            }
        }
        for (name, mut defs) in by_name {
            let mut srcs: Vec<u16> = defs.iter().map(|(_, s)| *s).collect();
            srcs.sort_unstable();
            srcs.dedup();
            if srcs.len() < 2 {
                continue; // no cross-source collision — nothing to rename
            }
            defs.sort_by_key(|(_, s)| *s);
            let keep_src = defs[0].1; // lowest source keeps the bare name
            let rest = name.strip_prefix("n_").unwrap_or(&name).to_string();
            for (d, src) in &defs {
                if *src != keep_src {
                    self.definitions[*d as usize].name = format!("n_s{src}_{rest}");
                }
            }
        }
    }

    /// @PLN26 phase 1 — body-less `#native` symbols declared by 2+ distinct
    /// sources (libraries).  Unlike a wrapper *name* (which
    /// `namespace_colliding_native_fns` renames), a `#native` *symbol* lives in
    /// the package's cdylib and cannot be renamed by the consumer.  The C-ABI
    /// native link puts every package's exports in one flat namespace (and the
    /// interpreter's `BRIDGE_REGISTRY` is keyed by symbol), so two packages
    /// exporting the same symbol resolve first-`.so`-wins / last-loaded-wins —
    /// silently the wrong fn.  Native codegen turns each collision into a
    /// `compile_error!`.  Returns `(symbol, sorted distinct sources)` per
    /// collision (deterministic order).
    #[must_use]
    pub fn native_symbol_collisions(&self) -> Vec<(String, Vec<u16>)> {
        let mut by_sym: HashMap<String, Vec<u16>> = HashMap::new();
        for d in 0..self.definitions() {
            let def = self.def(d);
            if def.code == Value::Null && !def.native().is_empty() {
                by_sym
                    .entry(def.native().to_string())
                    .or_default()
                    .push(def.source);
            }
        }
        let mut out: Vec<(String, Vec<u16>)> = by_sym
            .into_iter()
            .filter_map(|(sym, mut srcs)| {
                srcs.sort_unstable();
                srcs.dedup();
                (srcs.len() > 1).then_some((sym, srcs))
            })
            .collect();
        out.sort();
        out
    }

    #[must_use]
    pub fn use_exists(&self, file: &str) -> bool {
        self.use_names.contains_key(file)
    }

    pub fn use_add(&mut self, short: &str) {
        // @PLN22 Phase 2 — source 0 = stdlib prelude, source 1 = the main program
        // (MAIN_SOURCE, reserved so a user def can shadow a prelude name without a
        // `(name, source)` collision); imported libraries number up from 2.
        // `use_names` holds std + the libs (never the main file), so `len() + 1`
        // is the next free source after the reserved main slot.
        let n = self.use_names.len() as u16 + 1;
        self.use_names.insert(short.to_string(), n);
        self.source = n;
    }

    /// @PLN22 Phase 3 — register an additional name (`use lib as alias`) for an
    /// ALREADY-loaded library source, so `alias::fn` resolves the same source as
    /// `lib::fn`.  Unlike [`use_add`] this allocates no new source and does not
    /// switch `self.source`; it only adds a qualifier alias to `use_names`.
    pub fn use_alias(&mut self, alias: &str, source: u16) {
        self.use_names.insert(alias.to_string(), source);
    }

    /// Allow a new attribute on a definition with a specified type.
    pub fn add_attribute(
        &mut self,
        lexer: &mut Lexer,
        on_def: u32,
        name: &str,
        typedef: Type,
    ) -> usize {
        if self.def(on_def).attr_names.contains_key(name) {
            let orig_attr = self.def(on_def).attr_names[name];
            let attr = &self.def(on_def).attributes[orig_attr];
            if attr.typedef.is_unknown() {
                if attr.typedef == typedef {
                    diagnostic!(
                        lexer,
                        Level::Error,
                        "Double attribute '{}.{name}'",
                        self.def(on_def).name
                    );
                } else {
                    diagnostic!(
                        lexer,
                        Level::Error,
                        "Cannot change the type of attribute: {}.{name}",
                        self.def(on_def).name
                    );
                }
            }
            return orig_attr;
        }
        let attr = Attribute {
            name: name.to_string(),
            typedef,
            mutable: true,
            constant: false,
            const_field: false,
            value_const: false,
            init: false,
            nullable: true,
            primary: false,
            hidden: false,
            value: Value::Null,
            check: Value::Null,
            check_message: Value::Null,
            alias_d_nr: u32::MAX,
            assigned_lambda_d_nr: u32::MAX,
            links: Vec::new(),
            lexeme: false,
        };
        let next_attr = self.def(on_def).attributes.len();
        let def = &mut self.definitions[on_def as usize];
        def.attr_names.insert(name.to_string(), next_attr);
        def.attributes.push(attr);
        next_attr
    }

    /**
        Add a definitions.
        # Panics
        Will panic if a definition with the same name already exists.
    */
    pub fn add_def(&mut self, name: &str, position: &Position, def_type: DefType) -> u32 {
        let rec = self.definitions();
        // @PLN22 Phase 1 — an enum variant keeps a flat `(name, source)` key
        // (FIRST-wins, no panic) so it is reachable as a TYPE / constructor
        // (`Circle { … }`, `s: Circle`, `fn f(self: Circle)`); two enums may
        // therefore share a variant name.  But a bare variant used as a VALUE
        // resolves ONLY via context (match subject, typed decl, typed
        // reassignment / `rec.field`, parameter, return, `==` LHS, `Enum::`/`lib::`
        // qualifier — the variant_of chokepoints), NEVER via this flat key, so a
        // no-context `s = Red` is an error even when the name is currently unique
        // (see parse_constant_value).  That way adding a second enum with the same
        // variant name can never silently break an existing bare assignment.
        // Every OTHER def kind keeps the flat key + the hard dual-definition guard.
        if def_type == DefType::EnumValue {
            self.def_names
                .entry((name.to_string(), self.source))
                .or_insert(rec);
        } else {
            assert!(
                !self
                    .def_names
                    .contains_key(&(name.to_string(), self.source)),
                "Dual definition of {name} at {position}"
            );
            self.def_names.insert((name.to_string(), self.source), rec);
        }
        let new_def = Definition {
            name: name.to_string(),
            source: self.source,
            position: position.clone(),
            def_type,
            parent: u32::MAX,
            attributes: Vec::default(),
            attr_names: HashMap::default(),
            code: Value::Null,
            returned: Type::Unknown(rec),
            returned_not_null: false,
            rust: String::new(),
            native: String::new(),
            cap: String::new(),
            op_code: u16::MAX,
            known_type: u16::MAX,
            code_position: 0,
            code_length: 0,
            variables: Function::new(name, &position.file),
            pub_visible: false,
            null_safe: false,
            superseded: String::new(),
            c_symbol: String::new(),
            c_sig: String::new(),
            closure_record: u32::MAX,
            mutated_captures: Vec::new(),
            scalars_to_box: Vec::new(),
            bounds: Vec::new(),
            const_ref: None,
            forced_size: None,
            purity: Purity::Unknown,
            field_groups: Vec::new(),
            synthetic: None,
        };
        self.definitions.push(new_def);
        rec
    }

    /// Mark a definition as synthesised by the compiler with a
    /// `&'static` reason string.  Used by fallback dispatch paths
    /// (e.g. `parser/fields.rs::field`'s parent-enum lookup) to skip
    /// auto-generated stubs that look identical to user decls.
    pub fn mark_synthetic(&mut self, d_nr: u32, reason: &'static str) {
        self.definitions[d_nr as usize].synthetic = Some(reason);
    }

    /// Assign a sequential op_code to an operator definition.
    ///
    /// Op_codes 0..N map to `fill::OPERATORS[0..N]`.  Bytecode encoding is
    /// transparent via `fill::emit_op`: codes < 255 use 1 byte, codes >= 255
    /// use 2 bytes (255 + offset).
    ///
    /// Op_code assignment runs at parse time and may legitimately exceed
    /// `fill::OPERATORS.len()` when a new opcode has just been added to
    /// `default/*.loft` and `src/fill.rs` has not yet been regenerated.  The
    /// staleness checks (`n9_generated_fill_matches_src` and
    /// `fill_rs_up_to_date` in `tests/issues.rs`) catch the drift; the
    /// runtime would index-OOB on dispatch if a stale `fill.rs` were
    /// actually executed.
    pub fn op_code(&mut self, def_nr: u32) {
        if !self.def(def_nr).is_operator() || self.def(def_nr).op_code != u16::MAX {
            return;
        }
        self.definitions[def_nr as usize].op_code = self.op_codes;
        self.operators.insert(self.op_codes, def_nr);
        self.op_codes += 1;
    }

    #[must_use]
    /// # Panics
    /// When an operator is searched that is currently not known.
    pub fn get_possible(&self, start: &str, lexer: &Lexer) -> &Vec<u32> {
        assert!(
            self.possible.contains_key(start),
            "Unknown operator {start} at {}",
            lexer.pos()
        );
        &self.possible[start]
    }

    /// @PLN99 Arc C — register `d_nr` into the `possible[prefix]` operator map.
    /// A user-defined conversion (`fn OpConvXFromY`) is a global stored `n_OpConv…`,
    /// so it skips `add_op`'s name-gated registration and never entered `possible` —
    /// which is the list `convert`'s type-matching OpConv loop searches. Registering
    /// it here (deduped) lets `x as T` and implicit conversions find a user `S → T`.
    pub fn register_possible(&mut self, prefix: &str, d_nr: u32) {
        let slot = self.possible.entry(prefix.to_string()).or_default();
        if !slot.contains(&d_nr) {
            slot.push(d_nr);
        }
    }

    #[must_use]
    pub fn definitions(&self) -> u32 {
        self.definitions.len() as u32
    }

    #[must_use]
    pub fn def_referenced(&self, d_nr: u32) -> bool {
        self.referenced.contains_key(&d_nr)
    }

    pub fn set_referenced(&mut self, d_nr: u32, t_nr: u32, change: Value) {
        if d_nr != u32::MAX {
            self.referenced.insert(d_nr, (t_nr, change));
        }
    }

    #[must_use]
    pub fn def_type(&self, d_nr: u32) -> DefType {
        if d_nr == u32::MAX {
            DefType::Unknown
        } else {
            self.def(d_nr).def_type.clone()
        }
    }

    /**
    Set the return type on a definition.
    # Panics
    When the return type was already set before.
    */
    pub fn set_returned(&mut self, d_nr: u32, tp: Type) {
        assert!(
            self.def(d_nr).returned.is_unknown(),
            "Cannot change returned type on [{d_nr}]{} to {} twice was {} at {:?}",
            self.def(d_nr).name,
            self.def(d_nr).returned.name(self),
            tp.name(self),
            self.def(d_nr).position
        );
        self.definitions[d_nr as usize].returned = tp;
    }

    #[must_use]
    pub fn attributes(&self, d_nr: u32) -> usize {
        self.def(d_nr).attributes.len()
    }

    #[must_use]
    pub fn attr(&self, d_nr: u32, name: &str) -> usize {
        if let Some(nr) = self.def(d_nr).attr_names.get(name) {
            *nr
        } else {
            usize::MAX
        }
    }

    #[must_use]
    pub fn attr_name(&self, d_nr: u32, a_nr: usize) -> String {
        if a_nr == usize::MAX {
            "Undefined".to_string()
        } else {
            self.def(d_nr).attributes[a_nr].name.clone()
        }
    }

    #[must_use]
    pub fn attr_type(&self, d_nr: u32, a_nr: usize) -> Type {
        if a_nr == usize::MAX {
            self.def(d_nr).returned.clone()
        } else {
            self.def(d_nr).attributes[a_nr].typedef.clone()
        }
    }

    /// Check if struct `d_nr` contains itself as a value type (not reference)
    /// field, directly or through other structs.  (Moved from `typedef.rs` —
    /// pass-2: a walk over `Data`'s definition graph lives with `Data`.)
    pub fn has_value_cycle(
        &self,
        d_nr: u32,
        visiting: &mut std::collections::HashSet<u32>,
    ) -> bool {
        if !visiting.insert(d_nr) {
            return true; // Already visiting this type — cycle found.
        }
        for a_nr in 0..self.attributes(d_nr) {
            let a_type = self.attr_type(d_nr, a_nr);
            // Only recurse into value-typed struct fields.  A `reference<T>`
            // field (the `u16::MAX` share-marker dep, #328) is a 12-byte
            // pointer, not inline bytes — it cannot cause an infinite-size
            // cycle, and skipping it here is exactly what makes
            // `reference<Self>` legal.
            if let Type::Reference(child_nr, deps) = &a_type
                && !deps.contains(&u16::MAX)
                && self.def_type(*child_nr) == DefType::Struct
                && !self.def_referenced(*child_nr)
                && self.has_value_cycle(*child_nr, visiting)
            {
                visiting.remove(&d_nr);
                return true;
            }
        }
        visiting.remove(&d_nr);
        false
    }

    /**
    Write the type on an attribute of a definition.
    # Panics
    When the type was already set before.
    */
    pub fn set_attr_type(&mut self, d_nr: u32, a_nr: usize, tp: Type) {
        if a_nr == usize::MAX || !self.attr_type(d_nr, a_nr).is_unknown() {
            panic!(
                "Cannot set attribute type {}.{} twice was {} to {}",
                self.def(d_nr).name,
                self.attr_name(d_nr, a_nr),
                self.attr_type(d_nr, a_nr).name(self),
                tp.name(self)
            );
        } else {
            self.definitions[d_nr as usize].attributes[a_nr].typedef = tp;
        }
    }

    /// #682 — downgrade a closure-record capture attribute from the ADOPTING
    /// share marker ([`Deps::share_sentinel`]) to the BORROWED one, once scope
    /// analysis has settled who really owns the captured store.
    ///
    /// Separate from [`Data::set_attr_type`] (which refuses to overwrite a type
    /// that is already set) because this rewrites only the dep MARKER of an
    /// already-typed `Reference` attribute.  The storage shape — 12 bytes,
    /// align 4 — is the same either way, which is what makes rewriting it after
    /// the record has been laid out safe: no position, size or alignment moves,
    /// only the cascade's free decision.
    pub fn mark_capture_borrowed(&mut self, record: u32, a_nr: usize) {
        if let Type::Reference(_, deps) =
            &mut self.definitions[record as usize].attributes[a_nr].typedef
        {
            *deps = Deps::borrowed_share_sentinel();
        }
    }

    /// #687 — retype a closure-record capture attribute whose storage the parent's
    /// pass-1 body end has just settled.
    ///
    /// Separate from [`Data::set_attr_type`] (which refuses to overwrite a set type)
    /// because the lambda's own epilogue has to write SOMETHING before the parent's
    /// body finishes, and only the body end knows whether the captured binding ended
    /// up with its own indirection.  Safe to run there: the record's db layout is
    /// built by `fill_all` at the end of the pass, after this.
    pub fn retype_capture_attr(&mut self, record: u32, a_nr: usize, tp: Type) {
        self.definitions[record as usize].attributes[a_nr].typedef = tp;
    }

    #[must_use]
    pub fn attr_value(&self, d_nr: u32, a_nr: usize) -> Value {
        self.def(d_nr).attributes[a_nr].value.clone()
    }

    /// @PLN116 — does `tp` have a well-defined default value?  This is the single
    /// predicate that both the `x?` default-fallback operator and (the `S{}` zero
    /// value, in time) consult — there must be exactly one notion of "T's default".
    ///
    /// `Ok(())` when a default exists; `Err(reason)` names the first field / reason a
    /// record has none, ready to drop into the compile error.  Scalars, text,
    /// collections, enums (first/marked variant), nullables (`null`), tuples of
    /// defaulted elements, and fn-refs all have a default.  A **bare reference /
    /// non-null pointer** has none.  A **record** has one iff every field declares
    /// `= expr`, is nullable, or its own type has a default — recursively.  Cycles
    /// self-resolve: value-recursion is illegal (infinite size) and ref-recursion
    /// bottoms out at a reference, which has no default.
    ///
    /// # Errors
    /// Returns `Err(reason)` — a message naming the culprit field/type — when `tp`
    /// has no well-defined default (a bare reference, or a record with a non-null
    /// field whose type has none).
    pub fn has_default(&self, tp: &Type) -> std::result::Result<(), String> {
        match tp {
            Type::Integer(_)
            | Type::Float
            | Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Text(_)
            | Type::Vector(_, _)
            | Type::Hash(_, _, _)
            | Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Optional(_)
            | Type::Null
            | Type::Function(_, _, _)
            | Type::Enum(_, _, _) => Ok(()),
            Type::Tuple(elems) => {
                for e in elems {
                    self.has_default(e)?;
                }
                Ok(())
            }
            Type::Reference(d_nr, deps) => {
                // A shared POINTER field (`&T`, the u16::MAX share marker) is a bare
                // reference — it has no default value of its own.
                if deps.is_pointer_marker() {
                    return Err(format!(
                        "a `&{}` reference has no default",
                        self.def(*d_nr).name()
                    ));
                }
                if self.def_type(*d_nr) != DefType::Struct {
                    // An enum-value reference and other non-struct refs default fine.
                    return Ok(());
                }
                let rec = self.def(*d_nr).name().to_string();
                for a in 0..self.attributes(*d_nr) {
                    let at = &self.def(*d_nr).attributes()[a];
                    let ftp = self.attr_type(*d_nr, a);
                    // An explicit `= expr` default, a const-default / hidden field, and a
                    // computed (routine) field all supply their own value.
                    if at.value != Value::Null
                        || at.constant
                        || at.hidden
                        || matches!(ftp, Type::Routine(_))
                    {
                        continue;
                    }
                    // @PLN116 — a BARE (non-`Optional`) enum field needs an EXPLICIT choice.
                    // The enum's first-defined variant is a valid default for a bare enum
                    // (`x?` on `E?`), but choosing a variant AS a record's silent default is a
                    // real semantic decision the author must make — otherwise declaration
                    // order becomes a hidden default.  A bare enum's `nullable` flag is only
                    // the artifact that its 0 IS its null, so it does NOT excuse the field —
                    // only a genuinely `Optional`-typed field (below) defaults to null.  The
                    // synthetic `__nullable<…>` enum (a nullable struct field's inline rep) is
                    // excluded — it is handled as a real nullable.
                    if let Type::Enum(e, _, _) = &ftp
                        && !self.def(*e).name.starts_with("__")
                    {
                        let fname = self.attr_name(*d_nr, a);
                        let tn = ftp.name(self);
                        return Err(format!(
                            "record `{rec}` has no default: field `{fname}: {tn}` is an enum with \
                             no explicit choice — add `{fname}: {tn} = <variant>`, or make it \
                             `{tn}?` (defaults null)"
                        ));
                    }
                    // A genuinely nullable field defaults to `null`.
                    if at.nullable || matches!(ftp, Type::Optional(_)) {
                        continue;
                    }
                    if self.has_default(&ftp).is_err() {
                        let fname = self.attr_name(*d_nr, a);
                        let tn = ftp.name(self);
                        return Err(format!(
                            "record `{rec}` has no default: field `{fname}: {tn}` has none — add \
                             `{fname}: {tn} = <expr>`, make it `{tn}?` (defaults null), or give \
                             `{tn}` a default"
                        ));
                    }
                }
                Ok(())
            }
            _ => Err(format!("`{}` has no default", tp.name(self))),
        }
    }

    /**
    Write the default value of an attribute in a definition.
    # Panics
    When the value was already set before.
    */
    pub fn set_attr_value(&mut self, d_nr: u32, a_nr: usize, val: Value) {
        self.definitions[d_nr as usize].attributes[a_nr].value = val;
    }

    #[must_use]
    pub fn attr_check(&self, d_nr: u32, a_nr: u16) -> Value {
        self.def(d_nr).attributes[a_nr as usize].check.clone()
    }

    /**
    Write the check value of an attribute in a definition.
    # Panics
    When the value was already set before.
    */
    pub fn set_attr_check(&mut self, d_nr: u32, a_nr: usize, check: Value) {
        assert_eq!(
            self.def(d_nr).attributes[a_nr].value,
            Value::Null,
            "Cannot set attribute value twice"
        );
        self.definitions[d_nr as usize].attributes[a_nr].check = check;
    }

    #[must_use]
    pub fn attr_nullable(&self, d_nr: u32, a_nr: usize) -> bool {
        if a_nr == usize::MAX {
            return false;
        }
        self.definitions[d_nr as usize].attributes[a_nr].nullable
    }

    pub fn set_attr_nullable(&mut self, d_nr: u32, a_nr: usize, nullable: bool) {
        self.definitions[d_nr as usize].attributes[a_nr].nullable = nullable;
    }

    /**
    Add a new function to the definitions.
    # Panics
    When the return type cannot be parsed.
    */
    pub fn add_fn(&mut self, lexer: &mut Lexer, fn_name: &str, arguments: &[Argument]) -> u32 {
        let mut name = String::new();
        let is_self = !arguments.is_empty() && arguments[0].name == "self";
        let is_both = !arguments.is_empty() && arguments[0].name == "both";
        if is_self || is_both {
            let type_nr = self.type_def_nr(&arguments[0].typedef);
            if type_nr == u32::MAX {
                diagnostic!(
                    lexer,
                    Level::Error,
                    "Unknown type on fn '{fn_name}' argument '{}'",
                    arguments[0].name
                );
            } else {
                // @PLN25 — the signature key is NULLABILITY-AWARE: a `τ?` receiver/`both`
                // param appends `?` so `min(τ)` and `min(τ?)` are DISTINCT overloads. The
                // `type_def_nr` peel still governs LAYOUT (Optional shares the base's
                // storage); this only distinguishes the def KEY. Gate-OFF no `Optional`
                // exists, so the name is the base — byte-identical.
                let sig = Self::sig_type_name(&self.def(type_nr).name, &arguments[0].typedef);
                name = format!("t_{}{}_{fn_name}", sig.len(), sig);
            }
        } else {
            name = format!("n_{fn_name}");
        }
        // @PLN102 C97 — a LIBRARY (source ≥ 2, i.e. not the stdlib prelude and not the user's
        // MAIN program) defines its public symbols MODULE-SCOPED: they live under the library's
        // own source and are reached as `lib::name`, never injected into the global namespace.
        // So a library name that exists only in the STDLIB is NOT a redefinition — the two
        // coexist (`shapes::clamp` beside the stdlib `clamp`), which is what lets the stdlib grow
        // without breaking a shipped lib.  Only a clash within the library's OWN source is a real
        // redefinition.  The stdlib (STD_SOURCE) and the user's MAIN program keep the global-scope
        // check (a MAIN top-level def that a stdlib method would silently shadow is a C95 error).
        let scoped = self.source != STD_SOURCE && self.source != MAIN_SOURCE;
        let own = |data: &Self, nm: &str| -> u32 {
            if scoped {
                data.source_nr(data.source, nm)
            } else {
                data.def_nr(nm)
            }
        };
        let o_nr = own(self, fn_name);
        // A `Dynamic` def under the bare name is the DISPATCHER a `both:`/`self` function registers
        // its type-overloads against — so a same-named def is not always a redefinition. It is when
        // the bare name is a concrete def (struct/enum/plain fn), or when a plain FREE fn whose
        // first-arg type already HAS a method would be silently shadowed by it: a call `name(x, …)`
        // resolves to the method `t_<sig>_<name>` (the fn's canonical internal name), never the free
        // `n_<name>`. Turn that silent shadow into a clear error (owner 2026-07-15). It is NOT a
        // redefinition when a new `both:`/`self` overload registers on the dispatcher
        // (`abs(integer)`/`abs(single)`/`abs(float)`; a same-type duplicate is caught by the mangled
        // check below), nor when a free fn merely shares a name with a method on another receiver
        // type (`scale(integer,…)` beside `scale(self: Vec,…)`) — arg-type dispatch keeps it live.
        let shadows_a_method = !(is_both || is_self)
            && arguments.first().is_some_and(|a| {
                let tn = self.type_def_nr(&a.typedef);
                tn != u32::MAX && {
                    let sig = Self::sig_type_name(&self.def(tn).name, &a.typedef);
                    own(self, &format!("t_{}{}_{fn_name}", sig.len(), sig)) != u32::MAX
                }
            });
        if o_nr != u32::MAX && (self.def(o_nr).def_type != DefType::Dynamic || shadows_a_method) {
            diagnostic!(
                lexer,
                Level::Error,
                "Cannot redefine '{}' (already defined at {})",
                fn_name.strip_prefix("n_").unwrap_or(fn_name),
                self.def(o_nr).position
            );
        }
        let mut d_nr = own(self, &name); // C97: a library's mangled name is scoped to its own source
        if d_nr != u32::MAX {
            diagnostic!(
                lexer,
                Level::Error,
                "Cannot redefine '{}'",
                fn_name.strip_prefix("n_").unwrap_or(fn_name)
            );
            return u32::MAX;
        }
        d_nr = self.add_def(&name, lexer.pos(), DefType::Function);
        for a in arguments {
            let a_nr = self.add_attribute(lexer, d_nr, &a.name, a.typedef.clone());
            self.set_attr_value(d_nr, a_nr, a.default.clone());
            // Note: Argument.constant (the `const` keyword on a parameter) is enforced at the
            // parser level via Variable.const_param — NOT by setting Attribute.mutable = false
            // here. Setting mutable = false for a user-defined function parameter would cause
            // the bytecode generator to skip pushing the argument value onto the stack, breaking
            // all calls to the function. Attribute.constant/mutable semantics are only correct
            // for operator definitions (add_op), where non-mutable params are bytecode constants.
        }
        if is_self || is_both {
            let type_nr = self.type_def_nr(&arguments[0].typedef);
            let existing = self.attr(type_nr, fn_name) != usize::MAX;
            // @PLN25 — a `τ?` overload peels to the base type here, so its type attribute
            // collides with the `τ` base's (true duplicates were already caught by the
            // mangled-name check above). The base overload owns the single type attribute;
            // the `τ?` overload is reachable via its distinct mangled key + the `Dynamic`
            // dispatcher, so skip re-adding it. (Define the non-null overload first.)
            if existing && matches!(&arguments[0].typedef, Type::Optional(_)) {
                // nullability overload — the base owns the type attribute; nothing to add.
            } else if existing {
                // The receiver type already carries a member of this name.  Unlike a free
                // function (which C97 module-scopes to its library), a method lives in the
                // type's SHARED, global attribute table, and `x.name(…)` can resolve to only
                // one thing — so a colliding method can't be module-scoped (the C97 residual).
                // Name the type and point at the fix; when the clash is with the stdlib, say so.
                let tname = self.def(type_nr).name.clone();
                let attr_idx = self.attr(type_nr, fn_name);
                let existing_rt = match &self.def(type_nr).attributes[attr_idx].typedef {
                    Type::Routine(nr) => *nr,
                    _ => u32::MAX,
                };
                if existing_rt != u32::MAX && self.def(existing_rt).source == STD_SOURCE {
                    diagnostic!(
                        lexer,
                        Level::Error,
                        "`{fn_name}` is a stdlib method on `{tname}` — a type's methods are global, so `x.{fn_name}(…)` can't be two things; rename yours, or drop it (the stdlib already provides it)",
                    );
                } else if existing_rt != u32::MAX {
                    diagnostic!(
                        lexer,
                        Level::Error,
                        "cannot redefine method `{fn_name}` on `{tname}` (already defined at {})",
                        self.def(existing_rt).position
                    );
                } else {
                    diagnostic!(
                        lexer,
                        Level::Error,
                        "cannot redefine field `{fn_name}` on `{tname}`"
                    );
                }
                return u32::MAX;
            } else {
                let a_nr = self.add_attribute(lexer, type_nr, fn_name, Type::Routine(d_nr));
                self.definitions[type_nr as usize].attributes[a_nr].mutable = false;
                self.definitions[type_nr as usize].attributes[a_nr].constant = true;
            }
        }
        if is_both {
            let mut main = self.def_nr(fn_name);
            if main == u32::MAX {
                main = self.add_def(fn_name, lexer.pos(), DefType::Dynamic);
            }
            let type_nr = self.type_def_nr(&arguments[0].typedef);
            assert_ne!(
                type_nr,
                u32::MAX,
                "Unknown type {}: {:?} at {}",
                arguments[0].name,
                arguments[0].typedef,
                lexer.pos()
            );
            // @PLN25 — key the dispatcher attribute by the nullability-aware sig name so a
            // `min(τ?)` overload lives beside `min(τ)` on the `Dynamic` def.
            let base = self.def(type_nr).name.clone();
            let sig = Self::sig_type_name(&base, &arguments[0].typedef);
            let a_nr = self.add_attribute(lexer, main, &sig, Type::Routine(d_nr));
            self.definitions[main as usize].attributes[a_nr].mutable = false;
            self.definitions[main as usize].attributes[a_nr].constant = true;
        }
        d_nr
    }

    /// @PLN25 — the nullability-aware signature type-name used to KEY an overload: a `τ?`
    /// receiver / `both` param appends `?` (so `min(τ)` and `min(τ?)` are distinct def keys),
    /// else the base type name. The `type_def_nr` peel still governs LAYOUT (Optional shares
    /// the base's storage); this only distinguishes the def KEY. Gate-OFF no `Optional` is
    /// constructed, so the result is always the base name — byte-identical.
    #[must_use]
    fn sig_type_name(base: &str, typedef: &Type) -> String {
        if matches!(typedef, Type::Optional(_)) {
            format!("{base}?")
        } else {
            base.to_string()
        }
    }

    #[must_use]
    pub fn get_fn(&self, fn_name: &str, arguments: &[Argument]) -> u32 {
        let is_self = !arguments.is_empty() && arguments[0].name == "self";
        let is_both = !arguments.is_empty() && arguments[0].name == "both";
        if is_self || is_both {
            let type_nr = self.type_def_nr(&arguments[0].typedef);
            let base = self.def(type_nr).name.clone();
            let sig = Self::sig_type_name(&base, &arguments[0].typedef);
            let struct_source = self.definitions[type_nr as usize].source;
            let lookup = |nm: &str| {
                let d = self.source_nr(struct_source, nm);
                // Method defined outside the struct's source file (e.g., user extends a
                // library type). Fall back to the current parse source.
                if d == u32::MAX {
                    self.source_nr(self.source, nm)
                } else {
                    d
                }
            };
            let d_nr = lookup(&format!("t_{}{}_{fn_name}", sig.len(), sig));
            // @PLN25 — a `τ?` receiver falls back to the base (non-null) overload when no
            // `τ?` overload exists, so a nullable value still reaches the plain method
            // (preserves the pre-nullability-key dispatch; inert when sig == base).
            if d_nr == u32::MAX && sig != base {
                lookup(&format!("t_{}{}_{fn_name}", base.len(), base))
            } else {
                d_nr
            }
        } else {
            self.def_nr(&format!("n_{fn_name}"))
        }
    }

    #[must_use]
    pub fn find_fn(&self, source: u16, fn_name: &str, tp: &Type) -> u32 {
        if matches!(tp, Type::Unknown(_)) {
            return self.source_nr(source, &format!("n_{fn_name}"));
        }
        let type_nr = self.type_def_nr(tp);
        if type_nr == u32::MAX {
            // No method dispatch for types like Function; fall back to n_ global.
            return self.source_nr(source, &format!("n_{fn_name}"));
        }
        let base = self.def(type_nr).name.clone();
        let sig = Self::sig_type_name(&base, tp);
        let d_nr = self.source_nr(source, &format!("t_{}{}_{fn_name}", sig.len(), sig));
        if d_nr != u32::MAX {
            return d_nr;
        }
        // @PLN25 — a `τ?` receiver falls back to the base (non-null) overload (inert when
        // sig == base, i.e. gate-OFF or a non-nullable receiver).
        if sig != base {
            let d_nr = self.source_nr(source, &format!("t_{}{}_{fn_name}", base.len(), base));
            if d_nr != u32::MAX {
                return d_nr;
            }
        }
        let d_nr = self.source_nr(source, &format!("n_{fn_name}"));
        if d_nr != u32::MAX {
            return d_nr;
        }
        // I9-prim: fall back to the `possible` operator map for built-in types.
        // Built-in operators use `add_op` (e.g. `OpLtInt`) rather than the method-style
        // `t_7integer_OpLt` convention.  Search `possible[fn_name]` for an operator whose
        // first parameter matches `tp`.
        if let Some(ops) = self.possible.get(fn_name) {
            for &op_nr in ops {
                if !self.def(op_nr).attributes.is_empty() && self.attr_type(op_nr, 0).is_equal(tp) {
                    return op_nr;
                }
            }
        }
        u32::MAX
    }

    /// @PLN101 — is this struct def declared `value struct`? A value (copy) type stored
    /// inline (record field / vector element already inline out-of-the-box), never aliased,
    /// non-null. Consulted by the value-semantics chokepoints.
    #[must_use]
    pub fn is_value_struct(&self, d_nr: u32) -> bool {
        self.value_structs.contains(&d_nr)
    }

    /// @PLN99 Arc A completion — resolve ONLY a user-defined operator METHOD
    /// (`t_<len><Type>_<fn>`, with the `τ?`→base fallback), and NOTHING ELSE: no
    /// `n_<fn>` global, no `possible`-map / built-in fallback.  `call_op` calls this
    /// BEFORE its built-in `possible` loop so a first-grade struct's OWN operator wins
    /// over built-in reference-identity (`==`) and conversion-coercion (`a - b` when a
    /// `T → integer` conversion exists).  Returns `u32::MAX` when the type has no such
    /// method — the caller then runs the built-in loop unchanged (integer/float/text
    /// operators live in `possible`, never as `t_` methods, so they are unaffected).
    #[must_use]
    pub fn find_op_method(&self, source: u16, fn_name: &str, tp: &Type) -> u32 {
        let type_nr = self.type_def_nr(tp);
        if type_nr == u32::MAX {
            return u32::MAX;
        }
        let base = self.def(type_nr).name.clone();
        let sig = Self::sig_type_name(&base, tp);
        let d_nr = self.source_nr(source, &format!("t_{}{}_{fn_name}", sig.len(), sig));
        if d_nr != u32::MAX {
            return d_nr;
        }
        if sig != base {
            let d_nr = self.source_nr(source, &format!("t_{}{}_{fn_name}", base.len(), base));
            if d_nr != u32::MAX {
                return d_nr;
            }
        }
        u32::MAX
    }

    /**
    Add a new operator
    # Panics
    When operators are not scanned correctly.
    */
    pub fn add_op(&mut self, lexer: &mut Lexer, fn_name: &str, arguments: &[Argument]) -> u32 {
        let d_nr = self.add_def(fn_name, lexer.pos(), DefType::Function);
        for a in arguments {
            let a_nr = self.add_attribute(lexer, d_nr, &a.name, a.typedef.clone());
            self.definitions[d_nr as usize].attributes[a_nr].mutable = !a.constant;
            self.definitions[d_nr as usize].attributes[a_nr].constant = a.constant;
            self.set_attr_value(d_nr, a_nr, a.default.clone());
        }
        if self.def(d_nr).is_operator() {
            for op in OPERATORS {
                if self.def(d_nr).name.starts_with(op) {
                    if !self.possible.contains_key(*op) {
                        self.possible.insert((*op).to_string(), Vec::new());
                    }
                    self.possible.get_mut(*op).unwrap().push(d_nr);
                }
            }
        }
        d_nr
    }

    /// Get a vector definition. This is a record with a single field pointing towards this vector.
    /// We need this definition as the primary record of a database holding a vector and its child records/vectors.
    pub fn vector_def(&mut self, lexer: &mut Lexer, tp: &Type) -> u32 {
        let fld_tp = Type::Vector(Box::new(tp.clone()), Deps::none());
        let fld = fld_tp.name(self);
        if self.def_nr(&fld) == u32::MAX {
            let d = self.add_def(&fld, lexer.pos(), DefType::Vector);
            self.definitions[d as usize].returned = fld_tp;
            self.definitions[d as usize].parent = self.type_def_nr(tp);
        }
        let name = format!("main_vector<{}>", tp.name(self));
        let d_nr = self.def_nr(&name);
        if d_nr == u32::MAX {
            let vd = self.add_def(&name, lexer.pos(), DefType::Struct);
            // Also register globally (source=0) so other files can find it.
            self.def_names
                .entry((name.clone(), STD_SOURCE))
                .or_insert(vd);
            // This synthetic wrapper is global, not owned by the file that
            // happened to first request it.  Stamp `source = 0` so a cache
            // reload's `rebuild_indices` (which keys `def_names` on each def's
            // own `source`) reproduces the global `(name, 0)` binding — without
            // it, `name_type(name, other_source)` returns `u16::MAX` after a
            // warm start and codegen emits `OpDatabase(db_tp=u16::MAX)`.
            self.definitions[vd as usize].source = 0;
            self.add_attribute(
                lexer,
                vd,
                "vector",
                Type::Vector(Box::new(tp.clone()), Deps::none()),
            );
            vd
        } else {
            d_nr
        }
    }

    /// P189 — register a synthetic struct definition for a tuple type
    /// shape `(T1, T2, ...)`.  Each tuple shape resolves to a single
    /// struct with attributes named `_0`, `_1`, ... so the rest of the
    /// type system (vector storage, field access, fill_database) can
    /// treat tuple-as-vector-element identically to struct-as-vector-element.
    ///
    /// Idempotent: returns the existing def_nr on subsequent calls
    /// for the same tuple shape.
    pub fn tuple_def(&mut self, lexer: &mut Lexer, types: &[Type]) -> u32 {
        let inner_names: Vec<String> = types.iter().map(|t| t.name(self)).collect();
        let name = format!("__tuple<{}>", inner_names.join(","));
        if let Some(&nr) = self.def_names.get(&(name.clone(), STD_SOURCE)) {
            return nr;
        }
        if let Some(&nr) = self.def_names.get(&(name.clone(), self.source)) {
            return nr;
        }
        let d = self.add_def(&name, lexer.pos(), DefType::Struct);
        // Register globally (source 0) so other files referencing the
        // same tuple shape resolve to the same def.  Stamp `source = 0` too so
        // a cache reload's `rebuild_indices` reproduces the global binding (see
        // `vector_def`).
        self.def_names
            .entry((name.clone(), STD_SOURCE))
            .or_insert(d);
        self.definitions[d as usize].source = STD_SOURCE;
        self.definitions[d as usize].returned = Type::Reference(d, Deps::none());
        let mut indices: Vec<u16> = Vec::with_capacity(types.len());
        let mut sizes_aligns: Vec<(u16, u8)> = Vec::with_capacity(types.len());
        for (i, t) in types.iter().enumerate() {
            let aname = format!("_{i}");
            let attr_idx = self.add_attribute(lexer, d, &aname, t.clone());
            // @PLN114 — a tuple element is nullable only if its TYPE says so, exactly
            // like a declared struct field.  `add_attribute` defaults `nullable:
            // true`, and a declared field overrides it from the declaration (`a: u8`
            // is not-null, `a: u8?` is not); the synthetic tuple attributes never
            // did, so every element resolved as nullable.  That split the op pair:
            // `NarrowIntKind::of` gave the WRITE `Short`/`ByteNullable` (the `+1`
            // sentinel encodings) while the READ resolved `ShortRaw`/`Byte`, so a
            // narrow element was written shifted and read raw — `(1,2)` read back
            // `(1,3)`.  The record picks the not-null pair on both sides; now so does
            // the tuple.
            self.set_attr_nullable(d, attr_idx, matches!(t, Type::Optional(_)));
            indices.push(attr_idx as u16);
            // For tuple element-size we use `data::element_size` (the
            // vector-storage width).  Natural alignment of an integer-
            // width field equals its size.  Text is 4 bytes via
            // interned heap pointer.  References are 12 bytes (DbRef);
            // alignment is 4.  Functions are 4 bytes (i32 d_nr); the
            // stack-slot inflation (to 20B) happens at read-back, not
            // here — the GROUP's storage view is what matters.
            let sz = element_stack_size(t) as u16;
            // @PLN114 A4 — one alignment table.  This used to carry an inline copy
            // of `element_align`'s rules, which had already drifted: it said
            // `Function => 4` where `element_align` says 8 (P249: the fn-ref slot's
            // 8-byte d_nr dictates the slot alignment).  Nothing compared them, so
            // nothing caught it.  `element_align` peels `Optional(τ)` itself.
            let align = element_stack_align(t);
            sizes_aligns.push((sz, align));
        }
        let alignment = LinkedFieldGroup::group_alignment(
            &sizes_aligns.iter().map(|&(_, a)| a).collect::<Vec<_>>(),
        );
        let size = LinkedFieldGroup::group_size(&sizes_aligns);
        // Register the tuple element field group — single Tuple-kind
        // group covering all `_0`, `_1`, … attributes in element order.
        // Used by codegen / runtime to identify tuple structs without
        // string-prefix matching on attribute names.  Alignment + size
        // are pre-computed so the layout routine can honour the
        // group's atomic placement.
        self.definitions[d as usize]
            .field_groups
            .push(LinkedFieldGroup {
                kind: LinkedFieldKind::Tuple,
                instance: 0,
                field_indices: indices,
                alignment,
                size,
            });
        d
    }

    /// @PLN25 E2a.1 — synthesize (once per struct) the nullable-enum type that
    /// backs a nullable embedded struct field / vector element: a 2-variant
    /// enum `{ Null, Some<fields-of struct_d> }`.  A nullable inline `Row` is
    /// byte-identical to this enum (discriminant at offset 0, `0` = absent), so
    /// the existing enum layout / construct / access / copy machinery carries
    /// null for free — see
    /// doc/claude/plans/25-nullable-sequences/embedded-record-null.md.
    ///
    /// Mirrors `parse_enum_values`' first-pass calls exactly — the parent's
    /// per-variant `constant` attributes, each variant's `enum` discriminant at
    /// offset 0, and the `Some` payload copied from `struct_d` — so the
    /// synthesized layout equals a hand-written `enum { Null, Some { … } }` by
    /// construction.  Idempotent + globally registered (source 0) like
    /// [`tuple_def`].
    pub fn nullable_enum_for(&mut self, lexer: &mut Lexer, struct_d: u32) -> u32 {
        let struct_name = self.def(struct_d).name.clone();
        let name = format!("__nullable<{struct_name}>");
        // @PLN25 + @PLN22 (p379 `two_libs_same_struct_name`): key the synth enum on the
        // STRUCT's OWN source — NOT a single global `STD_SOURCE` entry.  Two libraries may
        // each define a struct of the same name (`Chunk`); a global `__nullable<Chunk>` binds
        // EVERY `vector<Chunk>` to whichever `Chunk` synth'd first, so the OTHER lib's element
        // resolves to the wrong payload struct — its fields are "not found", and a field WRITE
        // (`c.field[i] = v`) then collapses to a base-var reassign → "cannot change type from
        // __nullable<Chunk> to <field-type>".  `(name, struct_source)` uniquely identifies the
        // struct (a lib cannot define two structs of one name); a consumer that references the
        // struct (a different `self.source`) resolves to the same synth via the struct's source,
        // because deps parse before dependents.
        let struct_source = self.definitions[struct_d as usize].source;
        if let Some(&nr) = self.def_names.get(&(name.clone(), struct_source)) {
            return nr;
        }
        let pos = lexer.pos().clone();
        // Create + register the synth under the STRUCT's source (not the current parse source),
        // so `add_def`'s `(name, source)` registration + dual-definition guard match the lookup
        // key above, and a `rebuild_indices` (cache-load path) re-derives the SAME key from the
        // def's `.source`.  Temporarily retarget `self.source` for the one `add_def` call.
        let saved_source = self.source;
        self.source = struct_source;
        let e = self.add_def(&name, &pos, DefType::Enum);
        self.source = saved_source;
        self.definitions[e as usize].source = struct_source;
        // Struct-enum (carries payload) → discriminator type is Enum(e, true).
        // Set directly (not via set_returned): the `Some` variant below would
        // otherwise set it a second time and trip set_returned's once-only guard.
        self.definitions[e as usize].returned = Type::Enum(e, true, Deps::none());
        let enumerate = self.def_nr("enumerate");

        // Variant 0 — `Null` (unit, no payload).  nr = 0 ⇒ discriminant value 1
        // (enum values start at 1; `0` = absent).  A unit variant carries no
        // `enum` attribute of its own (matches `parse_enum_values`); its
        // discriminant rides the `Some` variant's offset-0 slot.
        let nv = self.add_def("Null", &pos, DefType::EnumValue);
        self.definitions[nv as usize].parent = e;
        self.set_returned(nv, Type::Enum(e, true, Deps::none()));
        let null_attr = self.add_attribute(lexer, e, "Null", Type::Enum(e, true, Deps::none()));
        self.definitions[e as usize].attributes[null_attr].constant = true;
        self.set_attr_value(e, null_attr, Value::Enum(1, u16::MAX));

        // Variant 1 — `Some` carrying struct_d's fields.  nr = 1 ⇒ discriminant 2.
        let sv = self.add_def("Some", &pos, DefType::EnumValue);
        self.definitions[sv as usize].parent = e;
        self.set_returned(sv, Type::Enum(e, true, Deps::none()));
        let some_attr = self.add_attribute(lexer, e, "Some", Type::Enum(e, true, Deps::none()));
        self.definitions[e as usize].attributes[some_attr].constant = true;
        self.set_attr_value(e, some_attr, Value::Enum(2, u16::MAX));
        // Discriminant at offset 0 — the layout pass keys on `fields[0].name ==
        // "enum"` to reserve offset 0 and pack the payload after it.
        let e_attr = self.add_attribute(
            lexer,
            sv,
            "enum",
            Type::Enum(enumerate, false, Deps::none()),
        );
        self.set_attr_value(sv, e_attr, Value::Enum(2, u16::MAX));
        // Single-payload form (see plans/25-nullable-sequences/single-payload-refactor.md):
        // the `Some` variant carries ONE inline `payload: S` field whose TYPE is `struct_d`
        // itself — the same struct definition as a standalone `S`.  The layout pass embeds
        // it after the discriminant, so `payload`'s region keeps S's exact dense field-offset
        // table (no reorder/gap-fill of S's own fields) and a sub-ref at the payload offset
        // IS a valid dense `S` reference.  Field access, key resolution, args/returns and
        // `??` therefore reuse the dense-`S` machinery with no copy and no per-field encode.
        // A method `fn m(self: S)` is a `Type::Routine` ATTRIBUTE on `S`, not a data field;
        // it is reached transparently through the payload, so it need not be re-copied here.
        self.add_attribute(
            lexer,
            sv,
            "payload",
            Type::Reference(struct_d, Deps::none()),
        );

        self.mark_synthetic(e, "@PLN25 nullable-enum for embedded struct field");
        self.mark_synthetic(nv, "@PLN25 nullable-enum Null variant");
        self.mark_synthetic(sv, "@PLN25 nullable-enum Some variant");
        e
    }

    /// Plan-06 phase 4d.C step 1 — register the synthetic struct
    /// definition that backs `Type::Function` storage.  Mirrors
    /// [`tuple_def`]: idempotent, globally registered (source 0),
    /// returns the existing def_nr on subsequent calls.
    ///
    /// Storage layout (16 bytes total, finalised by `fill_database`
    /// + `Stores::finish_type` in later phases):
    /// - `_d_nr`: `i32` at offset 0 (4 bytes; `i32::MIN` = null fn-ref).
    /// - `_closure`: 12-byte stored DbRef at offset 4 (store_nr +
    ///   rec + pos pointing at the closure record in the same store).
    ///
    /// Phase 1 wires the **data-side definition only**; no database
    /// type-id is assigned yet, no fill_database arm routes through
    /// it, no codegen reads/writes it.  The function exists so phase
    /// 2 (opcode addition + Parts::DbRef + database routing) and
    /// phase 3 (codegen rework) have a single place to call.
    ///
    /// All fn-refs share the same storage shape regardless of
    /// signature, so the synthetic struct's name carries no
    /// argument-list suffix — `__fn_ref` is canonical and reused
    /// across every `Type::Function(...)` value in the program.
    pub fn fn_ref_def(&mut self, lexer: &mut Lexer) -> u32 {
        let name = "__fn_ref".to_string();
        if let Some(&nr) = self.def_names.get(&(name.clone(), STD_SOURCE)) {
            return nr;
        }
        if let Some(&nr) = self.def_names.get(&(name.clone(), self.source)) {
            return nr;
        }
        let d = self.add_def(&name, lexer.pos(), DefType::Struct);
        // Register globally (source 0) so every reference to
        // `Type::Function` across all source files resolves to the
        // same synthetic struct.  Stamp `source = 0` too so a cache reload's
        // `rebuild_indices` reproduces the global binding (see `vector_def`).
        self.def_names
            .entry((name.clone(), STD_SOURCE))
            .or_insert(d);
        self.definitions[d as usize].source = STD_SOURCE;
        self.definitions[d as usize].returned = Type::Reference(d, Deps::none());
        // `_d_nr`: 4-byte signed integer holding the function's
        // def-nr.  The integer alias `i32` carries the `size(4)`
        // annotation that fill_database honours via
        // `Data::forced_size(alias_d_nr)`.  When phase 2 routes
        // `Type::Function` through this synthetic struct,
        // fill_database registers `_d_nr` as `Parts::Int` (4-byte
        // storage) automatically.
        let i32_d_nr = self.def_nr("i32");
        let d_nr_attr_idx = self.add_attribute(
            lexer,
            d,
            "_d_nr",
            Type::Integer(IntegerSpec {
                min: i32::MIN + 1,
                max: i32::MAX as u32,
                not_null: false,
                forced_size: NonZeroU8::new(4),
            }),
        );
        if i32_d_nr != u32::MAX {
            self.definitions[d as usize].attributes[d_nr_attr_idx].alias_d_nr = i32_d_nr;
        }
        // `_closure`: 12-byte stored DbRef pointing at the captured-
        // state record in the host's store.  Phase 2 introduces
        // `Parts::DbRef` (12B) and a `Type` shape that fill_database
        // routes through it — for now we leave the attribute typed
        // as `Type::Reference(d, _)` (self-reference is a benign
        // placeholder that fill_database will overwrite once the
        // real DbRef Parts variant lands).  The data-side definition
        // is the load-bearing piece; the runtime layout is deferred.
        self.add_attribute(lexer, d, "_closure", Type::Reference(d, Deps::none()));
        d
    }

    pub fn check_vector(&mut self, d_nr: u32, vec_tp: u16, pos: &Position) -> u32 {
        let vec_name = format!("vector<{}>", self.def(d_nr).name);
        let mut v_nr = self.def_nr(&vec_name);
        if v_nr == u32::MAX {
            v_nr = self.add_def(&vec_name, pos, DefType::Vector);
            self.definitions[v_nr as usize].parent = d_nr;
        }
        self.definitions[v_nr as usize].known_type = vec_tp;
        v_nr
    }

    /// A generic type-variable placeholder: the attribute-less, self-referential
    /// `Struct` the parser registers for a `<T>` type parameter (e.g. stdlib
    /// `min_of<T>`) so the template body's types resolve.  It has store size 0 and
    /// is an INTERNAL construct — it must never resolve as a real type outside the
    /// default files that declare it.
    #[must_use]
    pub fn is_type_var_placeholder(&self, d_nr: u32) -> bool {
        let d = &self.definitions[d_nr as usize];
        d.def_type == DefType::Struct
            && d.attributes.is_empty()
            && matches!(&d.returned, Type::Reference(r, _) if *r == d_nr)
    }

    /// Get the corresponding number from a definition on name.
    /// This will test both the own source file or the standard library data.
    #[must_use]
    pub fn def_nr(&self, name: &str) -> u32 {
        if let Some(nr) = self.def_names.get(&(name.to_string(), self.source)) {
            *nr
        } else if let Some(nr) = self.def_names.get(&(name.to_string(), STD_SOURCE)) {
            *nr
        } else {
            u32::MAX
        }
    }

    /// @PLN102 arc C — is `source` the compilation's OWNED entry project (vs a
    /// resolved dependency or the stdlib)?  loft numbers sources `STD_SOURCE` (0)
    /// = stdlib, `MAIN_SOURCE` (1) = the entry being compiled, `2..` = imported
    /// dependencies — so a source is owned exactly when it is the entry.  This is
    /// RELATIVE to what is being built: a library's own source is owned when its
    /// author compiles it, and a dependency when a consumer imports it (the
    /// consumer re-parses the library at a `2..` source, never `MAIN_SOURCE`).
    /// The arc-C steer gate (a later step) reads this so a `#superseded` warning
    /// reaches only whoever can act on it.
    ///
    /// Boundary: loft's entry is a single `MAIN_SOURCE` file plus `use`d
    /// libraries; were an entry package ever spread across several sources this
    /// would need the package-path-prefix map, but until then owned == the entry.
    #[must_use]
    pub fn source_is_owned(&self, source: u16) -> bool {
        source == MAIN_SOURCE
    }

    /// @PLN24 arc G — the package that owns `file`, among those that declared C
    /// libraries: the longest declared `pkg_dir` that is a prefix of it.
    ///
    /// The longest-prefix rule is the one the repo already uses to attribute a
    /// definition to its package (`native_symbol_crates`), so a nested package
    /// wins over the parent that contains it.
    #[must_use]
    pub fn c_owner_pkg(&self, file: &str) -> Option<&str> {
        self.c_libraries
            .iter()
            .filter(|c| !c.pkg_dir.is_empty() && file.starts_with(c.pkg_dir.as_str()))
            .max_by_key(|c| c.pkg_dir.len())
            .map(|c| c.pkg_dir.as_str())
    }

    /// @PLN24 arc G — must a `#c` symbol declared in `file` be resolved at RUN
    /// time rather than linked?
    ///
    /// True when its owning package declares any `[c] optional-libs` entry. The
    /// question is answered per PACKAGE and not per symbol because **a `#c`
    /// declaration does not name the library it comes from** — nothing in the
    /// source says which of a package's libraries exports a given symbol, and
    /// guessing from a name prefix would be a second source of truth of exactly
    /// the kind this plan's invariant refuses.
    ///
    /// The consequence is deliberate and worth stating: a package that declares
    /// only required libraries emits what arc C emitted, byte for byte, so the
    /// lazy path cannot regress a library that never asked for it.
    #[must_use]
    pub fn c_symbol_is_lazy(&self, file: &str) -> bool {
        let Some(pkg) = self.c_owner_pkg(file) else {
            return false;
        };
        self.c_libraries
            .iter()
            .any(|c| c.pkg_dir == pkg && c.optional)
    }

    /// @PLN102 arc C — is the source CURRENTLY being compiled owned (see
    /// [`Data::source_is_owned`])?  This is the caller-provenance fact the steer
    /// gate reads at a call site: `self.source` is the source of the code doing
    /// the call, so a steer fires only when the caller is the entry project,
    /// never when the call sits in a re-parsed dependency's or the stdlib's source.
    #[must_use]
    pub fn caller_source_is_owned(&self) -> bool {
        self.source_is_owned(self.source)
    }

    #[must_use]
    pub fn source_nr(&self, source: u16, name: &str) -> u32 {
        if source == u16::MAX {
            return self.def_nr(name);
        }
        let Some(nr) = self.def_names.get(&(name.to_string(), source)) else {
            return u32::MAX;
        };
        *nr
    }

    /** Get the definition by name
    # Panics
    When an unknown definition is requested
    */
    #[must_use]
    pub fn name_type(&self, name: &str, source: u16) -> u16 {
        let nr = if let Some(nr) = self.def_names.get(&(name.to_string(), source)) {
            *nr
        } else if let Some(nr) = self.def_names.get(&(name.to_string(), STD_SOURCE)) {
            *nr
        } else {
            return u16::MAX;
        };
        self.definitions[nr as usize].known_type
    }

    /** Get the definition by name from a given source file
    # Panics
    When an unknown definition is requested
    */
    #[must_use]
    pub fn source_name(&self, source: u16, name: &str) -> &Definition {
        let Some(nr) = self.def_names.get(&(name.to_string(), source)) else {
            panic!("Unknown definition {name}");
        };
        &self.definitions[*nr as usize]
    }

    /// #271: true if some source defines `name` as a NON-`pub` struct/enum type.
    /// A consumer that `use`s a library only imports the library's `pub` names
    /// (`import_all`), so brace-constructing a private library type resolves to
    /// no definition — letting the parser explain "it's private" instead of a
    /// baffling `Expect token ;` at the `{`.
    pub fn has_private_type(&self, name: &str) -> bool {
        self.def_names.iter().any(|((n, _), &d)| {
            n == name
                && !self.definitions[d as usize].pub_visible
                && matches!(self.def_type(d), DefType::Struct | DefType::Enum)
        })
    }

    /// Import all names from `lib_source` into `into_source`.
    /// Names already present in `into_source` (local definitions) are kept unchanged.
    /// The per-source name-visibility picture: one [`SourceView`] per source that has
    /// any definition or any reachable name, plus every import [`AliasView`].
    ///
    /// This is the state that decides whether an unqualified name resolves, and until
    /// now nothing could look at it: diagnosing why a `use`d library's function was
    /// unknown at a debugger frame took an `eprintln!` in the parser printing
    /// `source_nr(0..3, name)` and a rebuild (@PLN120 E.4).  An alias here is a
    /// `def_names` entry whose source differs from the definition's own — read off the
    /// table itself, so a rebuild that drops aliases shows as an empty list rather
    /// than as a call that mysteriously fails.
    #[must_use]
    pub fn resolution_view(&self) -> (Vec<SourceView>, Vec<AliasView>) {
        let mut defined: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
        let mut visible: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
        let mut file_of: std::collections::BTreeMap<u16, String> =
            std::collections::BTreeMap::new();
        for d in &self.definitions {
            *defined.entry(d.source).or_default() += 1;
            let f = &d.position.file;
            if !f.is_empty() {
                file_of.entry(d.source).or_insert_with(|| f.clone());
            }
        }
        let mut aliases = Vec::new();
        for ((name, src), &def_nr) in &self.def_names {
            *visible.entry(*src).or_default() += 1;
            let own = self
                .definitions
                .get(def_nr as usize)
                .map_or(*src, |d| d.source);
            if own != *src {
                aliases.push(AliasView {
                    name: name.clone(),
                    into_source: *src,
                    from_source: own,
                    def_nr,
                });
            }
        }
        aliases.sort_by(|a, b| {
            (a.into_source, &a.name, a.from_source).cmp(&(b.into_source, &b.name, b.from_source))
        });
        // Library name per source, so a `use`d source is identifiable by the name the
        // program wrote rather than only by path.
        let mut lib_of: std::collections::BTreeMap<u16, String> = std::collections::BTreeMap::new();
        for (lib, src) in &self.use_names {
            lib_of.entry(*src).or_insert_with(|| lib.clone());
        }
        let mut srcs: Vec<u16> = defined.keys().chain(visible.keys()).copied().collect();
        srcs.sort_unstable();
        srcs.dedup();
        let views = srcs
            .into_iter()
            .map(|nr| {
                let name = match (lib_of.get(&nr), file_of.get(&nr)) {
                    (Some(lib), Some(f)) => format!("{lib} ({f})"),
                    (Some(lib), None) => lib.clone(),
                    (None, Some(f)) => f.clone(),
                    (None, None) => "<unknown>".to_string(),
                };
                SourceView {
                    nr,
                    name,
                    defined: defined.get(&nr).copied().unwrap_or(0),
                    visible: visible.get(&nr).copied().unwrap_or(0),
                }
            })
            .collect();
        (views, aliases)
    }

    /// Where `name` is defined and from which sources it is reachable — the
    /// `--why <name>` query.  Returns `(def_nr, own_source, reachable_from)`, with
    /// `reachable_from` carrying `true` when the entry is the definition's own source
    /// and `false` when it is an import alias.  `None` when no source can see it,
    /// which is itself the answer to "why can't I call this".
    #[must_use]
    pub fn visibility_of(&self, name: &str) -> Option<(u32, u16, Vec<(u16, bool)>)> {
        // Functions are stored under the `n_` prefix; accept either spelling.
        let keys = [name.to_string(), format!("n_{name}")];
        let mut def_nr = None;
        let mut reachable: Vec<(u16, bool)> = Vec::new();
        for ((n, src), &nr) in &self.def_names {
            if !keys.iter().any(|k| k == n) {
                continue;
            }
            let own = self.definitions.get(nr as usize).map_or(*src, |d| d.source);
            if own == *src {
                def_nr = Some((nr, own));
            }
            reachable.push((*src, own == *src));
        }
        reachable.sort_unstable();
        let (nr, own) = def_nr.or_else(|| {
            // Visible only as an alias (its own source is not in the table) — still
            // report it, using the first alias's target.
            reachable.first().map(|&(src, _)| {
                let nr = self.def_names.iter().find_map(|((n, s), &nr)| {
                    (keys.iter().any(|k| k == n) && *s == src).then_some(nr)
                });
                (nr.unwrap_or(u32::MAX), src)
            })
        })?;
        Some((nr, own, reachable))
    }

    /// Retain one applied import for [`rebuild_indices`](Self::rebuild_indices) to
    /// replay.  Idempotent — re-applying the same `use` does not grow the list.
    fn remember_import(
        &mut self,
        lib_source: u16,
        into_source: u16,
        name: Option<(String, String)>,
    ) {
        let entry = AppliedImport {
            lib_source,
            into_source,
            name,
        };
        if !self.applied.contains(&entry) {
            self.applied.push(entry);
        }
    }

    /// Re-apply every retained import.  Called at the end of a `def_names` rebuild,
    /// which can only reconstruct each definition under its OWN source and so drops
    /// the cross-source aliases an import creates.
    fn replay_imports(&mut self) {
        // `take` drains the list, and `import_all` / `import_name` re-`remember` each
        // entry as they run — so the list is refilled by the replay itself and a LATER
        // rebuild still has it.  Load-bearing: without the re-record, only the first
        // rebuild would restore the aliases.
        for imp in std::mem::take(&mut self.applied) {
            match &imp.name {
                None => self.import_all(imp.lib_source, imp.into_source),
                Some((name, bind)) => {
                    self.import_name(imp.lib_source, imp.into_source, name, bind);
                }
            }
        }
    }

    pub fn import_all(&mut self, lib_source: u16, into_source: u16) {
        self.remember_import(lib_source, into_source, None);
        let names: Vec<(String, u32)> = self
            .def_names
            .iter()
            .filter(|((_, src), def_nr)| {
                *src == lib_source && self.definitions[**def_nr as usize].pub_visible
            })
            .map(|((name, _), &def_nr)| (name.clone(), def_nr))
            .collect();
        for (name, def_nr) in names {
            self.def_names.entry((name, into_source)).or_insert(def_nr);
        }
    }

    /// Import a single name from `lib_source` into `into_source`, BINDING it
    /// under `bind` (= `name` for a plain `use lib::name`, or the alias for
    /// `use lib::name as bind` — @PLN22 Phase 3).  Returns `false` if neither the
    /// plain name nor its `n_`-prefixed function form exists in `lib_source`, so
    /// the caller can emit an appropriate error.  Names already present in
    /// `into_source` are kept unchanged (local wins).
    pub fn import_name(
        &mut self,
        lib_source: u16,
        into_source: u16,
        name: &str,
        bind: &str,
    ) -> bool {
        self.remember_import(
            lib_source,
            into_source,
            Some((name.to_string(), bind.to_string())),
        );
        // Functions are stored under the `n_` prefix; try both forms.
        let fn_key = format!("n_{name}");
        let bind_fn_key = format!("n_{bind}");
        let found_plain = self
            .def_names
            .get(&(name.to_string(), lib_source))
            .copied()
            .filter(|&d| self.definitions[d as usize].pub_visible);
        let found_fn = self
            .def_names
            .get(&(fn_key, lib_source))
            .copied()
            .filter(|&d| self.definitions[d as usize].pub_visible);
        if found_plain.is_none() && found_fn.is_none() {
            return false;
        }
        if let Some(def_nr) = found_plain {
            self.def_names
                .entry((bind.to_string(), into_source))
                .or_insert(def_nr);
        }
        if let Some(def_nr) = found_fn {
            self.def_names
                .entry((bind_fn_key, into_source))
                .or_insert(def_nr);
        }
        true
    }

    /// Variant of [`import_all`] that **overwrites** forward-reference stubs
    /// in the target source.  Real local definitions still win (local
    /// precedence), but bindings that currently point to a `DefType::Unknown`
    /// stub are replaced by the imported real definition.
    ///
    /// Used by the package-mode driver's Phase C: when file B creates an
    /// Unknown stub for a type that will come from file A's re-exported
    /// namespace, this variant is what makes B's later references resolve
    /// to A's real definition.
    pub fn import_all_overwrite(&mut self, lib_source: u16, into_source: u16) {
        let names: Vec<(String, u32)> = self
            .def_names
            .iter()
            .filter(|((_, src), def_nr)| {
                *src == lib_source && self.definitions[**def_nr as usize].pub_visible
            })
            .map(|((name, _), &def_nr)| (name.clone(), def_nr))
            .collect();
        for (name, def_nr) in names {
            self.insert_or_replace_stub((name, into_source), def_nr);
        }
    }

    /// Variant of [`import_name`] that overwrites forward-reference stubs.
    /// See [`import_all_overwrite`] for the rationale.  Returns the same
    /// `false` on lookup miss as [`import_name`].
    pub fn import_name_overwrite(
        &mut self,
        lib_source: u16,
        into_source: u16,
        name: &str,
        bind: &str,
    ) -> bool {
        let fn_key = format!("n_{name}");
        let bind_fn_key = format!("n_{bind}");
        let found_plain = self
            .def_names
            .get(&(name.to_string(), lib_source))
            .copied()
            .filter(|&d| self.definitions[d as usize].pub_visible);
        let found_fn = self
            .def_names
            .get(&(fn_key, lib_source))
            .copied()
            .filter(|&d| self.definitions[d as usize].pub_visible);
        if found_plain.is_none() && found_fn.is_none() {
            return false;
        }
        if let Some(def_nr) = found_plain {
            self.insert_or_replace_stub((bind.to_string(), into_source), def_nr);
        }
        if let Some(def_nr) = found_fn {
            self.insert_or_replace_stub((bind_fn_key, into_source), def_nr);
        }
        true
    }

    /// Insert `def_nr` at `key`, or replace an existing binding when the
    /// existing binding points to a `DefType::Unknown` stub.  Real local
    /// bindings are preserved (local wins over imports).
    fn insert_or_replace_stub(&mut self, key: (String, u16), def_nr: u32) {
        match self.def_names.get(&key) {
            Some(&existing)
                if matches!(
                    self.definitions[existing as usize].def_type,
                    DefType::Unknown
                ) =>
            {
                self.def_names.insert(key, def_nr);
            }
            None => {
                self.def_names.insert(key, def_nr);
            }
            _ => {}
        }
    }

    /// Rewrite every `Type::Unknown(stub_nr)` occurrence in any definition's
    /// `returned` type or attribute typedefs to the resolved type from
    /// `target_def_nr`.  Walks compound types (`Vector`, `RefVar`, `Iterator`,
    /// `Tuple`, `Function`, `Rewritten`) recursively.
    ///
    /// Used by the package-mode driver's Phase C after imports have been
    /// propagated: stub def_nrs created during Phase B's deferred parsing
    /// become resolvable once the real definition is reachable via
    /// `def_names`, and this helper patches every `Type::Unknown(stub_nr)`
    /// pointer to the real type.  Direct mutation of the stored type bypasses
    /// `set_attr_type`'s panic guard, which only accepts replacement when the
    /// outer type is already `Unknown` — we need to patch `Vector<Unknown>`,
    /// `RefVar<Unknown>`, etc., where the outer wrapper is not `Unknown`.
    pub fn rewrite_unknown_refs(&mut self, stub_nr: u32, target_def_nr: u32) {
        let target_type = self.definitions[target_def_nr as usize].returned.clone();
        for d_nr in 0..self.definitions.len() {
            if let Some(new_ret) =
                Self::rewrite_type_opt(&self.definitions[d_nr].returned, stub_nr, &target_type)
            {
                self.definitions[d_nr].returned = new_ret;
            }
            let n_attrs = self.definitions[d_nr].attributes.len();
            for a_nr in 0..n_attrs {
                if let Some(new_ty) = Self::rewrite_type_opt(
                    &self.definitions[d_nr].attributes[a_nr].typedef,
                    stub_nr,
                    &target_type,
                ) {
                    self.definitions[d_nr].attributes[a_nr].typedef = new_ty;
                }
            }
        }
    }

    /// Recursive helper for [`rewrite_unknown_refs`].  Returns
    /// `Some(new_type)` when the subtree contained `Type::Unknown(stub)`
    /// and was rewritten, or `None` when the subtree is unchanged.
    #[allow(clippy::only_used_in_recursion)] // kept as associated fn for clarity
    fn rewrite_type_opt(t: &Type, stub: u32, target: &Type) -> Option<Type> {
        match t {
            Type::Unknown(n) if *n == stub => Some(target.clone()),
            Type::Vector(inner, deps) => Self::rewrite_type_opt(inner, stub, target)
                .map(|new_inner| Type::Vector(Box::new(new_inner), deps.clone())),
            Type::RefVar(inner) => Self::rewrite_type_opt(inner, stub, target)
                .map(|new_inner| Type::RefVar(Box::new(new_inner))),
            Type::Rewritten(inner) => Self::rewrite_type_opt(inner, stub, target)
                .map(|new_inner| Type::Rewritten(Box::new(new_inner))),
            Type::Iterator(step, internal) => {
                let new_step = Self::rewrite_type_opt(step, stub, target);
                let new_internal = Self::rewrite_type_opt(internal, stub, target);
                if new_step.is_none() && new_internal.is_none() {
                    None
                } else {
                    Some(Type::Iterator(
                        Box::new(new_step.unwrap_or_else(|| (**step).clone())),
                        Box::new(new_internal.unwrap_or_else(|| (**internal).clone())),
                    ))
                }
            }
            Type::Tuple(elems) => {
                let mut changed = false;
                let new_elems: Vec<Type> = elems
                    .iter()
                    .map(|e| match Self::rewrite_type_opt(e, stub, target) {
                        Some(new_e) => {
                            changed = true;
                            new_e
                        }
                        None => e.clone(),
                    })
                    .collect();
                if changed {
                    Some(Type::Tuple(new_elems))
                } else {
                    None
                }
            }
            Type::Function(args, ret, deps) => {
                let mut changed = false;
                let new_args: Vec<Type> = args
                    .iter()
                    .map(|a| match Self::rewrite_type_opt(a, stub, target) {
                        Some(new_a) => {
                            changed = true;
                            new_a
                        }
                        None => a.clone(),
                    })
                    .collect();
                let new_ret_opt = Self::rewrite_type_opt(ret, stub, target);
                if changed || new_ret_opt.is_some() {
                    Some(Type::Function(
                        new_args,
                        Box::new(new_ret_opt.unwrap_or_else(|| (**ret).clone())),
                        deps.clone(),
                    ))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /** Get a definition.
    # Panics
    When no definition on that number is found
    */
    #[must_use]
    pub fn def(&self, dnr: u32) -> &Definition {
        assert_ne!(dnr, u32::MAX, "Unknown definition");
        &self.definitions[dnr as usize]
    }

    /// Plan-07 phase 5 suggestions.  Find a similar type name —
    /// struct, enum, or enum-value — across all loaded definitions.
    /// Skips synthetic compiler-generated types (`__tuple<…>`,
    /// `__fn_ref`, `Self`, etc.) and single-character names (which
    /// are almost always interface generic-type parameters like
    /// `T` / `K` / `V` and would mis-suggest in user code).
    ///
    /// Free function on `Data` so both `Parser::suggest_type_name`
    /// and `typedef::actual_types`'s "Undefined type" emitter can
    /// reach it without threading the parser through.
    #[must_use]
    pub fn suggest_type_name(&self, name: &str) -> Option<String> {
        // A name a newcomer types out of habit from another language resolves by
        // TABLE, not by edit distance — `int`/`str`/`i64` are 3 characters (below
        // `suggest_similar_capped`'s floor) and `bool`→`boolean` (3),
        // `char`→`character` (5) and `string`→`text` (unrelated) all exceed its
        // distance cap.  Distance can reach none of them, so the table is the
        // whole mechanism for this class; edit distance below still catches real
        // typos (`intger`, `bolean`).
        if let Some(alias) = builtin_type_alias(name) {
            return Some(alias.to_string());
        }
        let candidates: Vec<&str> = self
            .definitions
            .iter()
            .filter_map(|d| {
                if !matches!(
                    d.def_type,
                    // `Type` is the base types (`integer`, `text`, …) — without it
                    // a mistyped BUILTIN had no candidates at all, which is why
                    // `intger` used to suggest nothing.
                    DefType::Struct | DefType::Enum | DefType::EnumValue | DefType::Type
                ) {
                    return None;
                }
                if d.name.starts_with("__") || d.name == "Self" {
                    return None;
                }
                // Filter out single-character names — they're
                // generic-type placeholders (`T`, `K`, `V`, …) on
                // interface declarations; suggesting `T` for an
                // unknown user type is more misleading than helpful.
                if d.name.chars().count() <= 1 {
                    return None;
                }
                Some(d.name.as_str())
            })
            .collect();
        crate::diagnostics::suggest_similar_capped(name, &candidates).map(String::from)
    }

    /// Plan-06 phase 5b' (DESIGN.md D12) — every user-defined
    /// function's def_nr (excludes stdlib `n_*` natives whose code
    /// body is `Value::Null`, excludes structs / enums / constants).
    ///
    /// "User function" = a `DefType::Function` definition with a
    /// non-Null code body.  This includes functions declared in
    /// `default/*.loft` that have explicit loft-body code, but
    /// excludes `#native`-only declarations.  The set is what the
    /// par-safety analyser (phase 5e) iterates over for the
    /// fixed-point classification.
    #[must_use]
    pub fn user_fn_d_nrs(&self) -> Vec<u32> {
        self.definitions
            .iter()
            .enumerate()
            .filter_map(|(idx, def)| {
                if def.def_type == DefType::Function && !matches!(def.code, Value::Null) {
                    Some(idx as u32)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Plan-06 phase 5b' (DESIGN.md D12) — every user fn that calls
    /// `callee_d_nr`, lazily-built and cached on first call.
    ///
    /// Walks every user fn body once collecting `Value::Call(callee, _)`
    /// edges, builds the inverted index `callee → [callers]`, caches
    /// it for the program's lifetime.  `Value::CallRef(local, _)` is
    /// not added to the graph because the actual callee is a runtime
    /// value (a function reference stored in a local variable);
    /// phase 5e's analyser pessimises any caller of a CallRef-routed
    /// fn (treats it as par-unsafe by default).
    ///
    /// Returns an empty slice if `callee_d_nr` has no callers (or
    /// only CallRef-style callers).
    ///
    /// Cost: linear in the call-graph edge count for the first call;
    /// O(1) thereafter.  For a typical loft codebase (~150 stdlib
    /// fns + a few hundred user fns), the build is sub-50 ms.
    pub fn callers_of(&self, callee_d_nr: u32) -> Vec<u32> {
        let map = self.caller_index.get_or_init(|| self.build_caller_index());
        map.get(&callee_d_nr).cloned().unwrap_or_default()
    }

    /// Internal helper for `callers_of`.  Walks every user fn body,
    /// collects `Value::Call(callee, _)` edges, returns the
    /// inverted `callee → [callers]` map.
    fn build_caller_index(&self) -> HashMap<u32, Vec<u32>> {
        let mut edges: Vec<(u32, u32)> = Vec::new(); // (callee, caller)
        for caller_d_nr in self.user_fn_d_nrs() {
            collect_callees(&self.def(caller_d_nr).code, caller_d_nr, &mut edges);
        }
        let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
        for (callee, caller) in edges {
            map.entry(callee).or_default().push(caller);
        }
        // Deduplicate: a fn that calls another fn many times still
        // counts as one caller.  Sort for stable test output.
        for callers in map.values_mut() {
            callers.sort_unstable();
            callers.dedup();
        }
        map
    }

    /// Post-2c: return the explicit `size(N)` annotation on the definition,
    /// if any.  Used by field allocation and sizeof() to honor `pub type i32 =
    /// integer size(4);` — the size overrides the limit()-based heuristic.
    /// Returns `None` when no annotation was provided (use the heuristic).
    #[must_use]
    pub fn forced_size(&self, dnr: u32) -> Option<u8> {
        if dnr == u32::MAX || (dnr as usize) >= self.definitions.len() {
            return None;
        }
        self.definitions[dnr as usize].forced_size
    }

    /// Return the `def_nr`s of all definitions whose `parent` field equals `parent_nr`.
    /// Used by the interface satisfaction checker (I6) to enumerate an interface's method stubs.
    pub fn children_of(&self, parent_nr: u32) -> impl Iterator<Item = u32> + '_ {
        self.definitions
            .iter()
            .enumerate()
            .filter(move |(_, d)| d.parent == parent_nr)
            .map(|(i, _)| i as u32)
    }

    /// @PLN22 Phase 1 — resolve a variant by name within ONE enum's members
    /// (the `(enum, variant)` scope key).  The single chokepoint every variant
    /// resolution routes through, replacing the bare global `def_nr(name)` +
    /// `parent == e_nr` dance — so two enums may share a variant name and a
    /// variant is never found without a contextual enum.  Returns `u32::MAX`
    /// when `name` is not a variant of `enum_nr`.
    #[must_use]
    pub fn variant_of(&self, enum_nr: u32, name: &str) -> u32 {
        if enum_nr == u32::MAX {
            return u32::MAX;
        }
        self.children_of(enum_nr)
            .find(|&c| self.def_type(c) == DefType::EnumValue && self.def(c).name() == name)
            .unwrap_or(u32::MAX)
    }

    /// @PLN22 Phase 1 — every enum that has a variant named `name`, in
    /// definition order.  Used by the resolver's error path to recognise a bare
    /// variant used with no type context and name the enum(s) to qualify with.
    #[must_use]
    pub fn enums_with_variant(&self, name: &str) -> Vec<u32> {
        let mut out = Vec::new();
        for i in 0..self.definitions.len() as u32 {
            if self.def_type(i) == DefType::Enum && self.variant_of(i, name) != u32::MAX {
                out.push(i);
            }
        }
        out
    }

    /// @PLN22 Phase 1 — resolve a variant by name across every enum defined in
    /// `source` (a library-qualified `lib::Variant`).  Returns the FIRST match;
    /// a library exposing two enums with the same variant name needs the
    /// `lib::Enum::Variant` form to disambiguate (Phase 3 `as` aliasing).
    /// `u32::MAX` when no enum in `source` has the variant.
    #[must_use]
    pub fn variant_in_source(&self, source: u16, name: &str) -> u32 {
        for i in 0..self.definitions.len() as u32 {
            if self.def(i).source == source && self.def_type(i) == DefType::Enum {
                let v = self.variant_of(i, name);
                if v != u32::MAX {
                    return v;
                }
            }
        }
        u32::MAX
    }

    /// # Panics
    /// When no definition on that number is found.
    pub fn def_mut(&mut self, dnr: u32) -> &mut Definition {
        assert_ne!(dnr, u32::MAX, "Unknown definition");
        &mut self.definitions[dnr as usize]
    }

    #[must_use]
    pub fn has_op(&self, op: u16) -> bool {
        self.operators.contains_key(&op)
    }

    #[must_use]
    pub fn operator(&self, op: u16) -> &Definition {
        self.def(self.operators[&op])
    }

    /// The display name of opcode `op`, or `None` when it is not a registered
    /// operator (e.g. the `255` extended-op prefix seen without its ext byte).
    /// Non-panicking companion to `operator` for diagnostics (`LOFT_UAF_GEN`).
    #[must_use]
    pub fn operator_name(&self, op: u16) -> Option<&str> {
        self.operators.get(&op).map(|&d| self.def(d).name.as_str())
    }

    pub fn attr_used(&mut self, d_nr: u32, a_nr: usize) {
        self.used_attributes.insert((d_nr, a_nr));
    }

    pub fn def_used(&mut self, d_nr: u32) {
        self.used_definitions.insert(d_nr);
    }

    #[must_use]
    pub fn type_def_nr(&self, tp: &Type) -> u32 {
        match tp {
            Type::Rewritten(t) => self.type_def_nr(t),
            // @PLN25 slice (b): `Optional(τ)` resolves to its base's type def.
            Type::Optional(t) => self.type_def_nr(t),
            Type::Integer(_) => self.source_nr(0, "integer"),
            Type::Boolean => self.source_nr(0, "boolean"),
            Type::Float => self.source_nr(0, "float"),
            Type::Text(_) => self.source_nr(0, "text"),
            Type::Single => self.source_nr(0, "single"),
            Type::Character => self.source_nr(0, "character"),
            Type::Routine(d_nr)
            | Type::Enum(d_nr, _, _)
            | Type::Reference(d_nr, _)
            | Type::Unknown(d_nr) => *d_nr,
            Type::Vector(_, _) => self.source_nr(0, "vector"),
            Type::RefVar(t) if matches!(**t, Type::Reference(_, _)) => self.type_def_nr(t),
            Type::RefVar(_) => self.source_nr(0, "reference"),
            Type::Sorted(_, _, _) => self.source_nr(0, "sorted"),
            Type::Index(_, _, _) => self.source_nr(0, "index"),
            Type::Hash(_, _, _) => self.source_nr(0, "hash"),
            Type::Radix(_, _, _) => self.source_nr(0, "spatial"),
            // P189: look up the synthetic tuple struct registered by
            // `tuple_def` at parse time.  Returns u32::MAX if the
            // tuple shape was never registered (caller must register
            // via `tuple_def` before reaching here, e.g. in `sub_type`
            // when parsing `vector<(...)>`).
            Type::Tuple(types) => {
                let inner_names: Vec<String> = types.iter().map(|t| t.name(self)).collect();
                let name = format!("__tuple<{}>", inner_names.join(","));
                self.def_nr(&name)
            }
            // Plan-06 phase 4d.A.2 — fn-ref vector elements use 4-byte
            // i32 d_nr storage.  Route to `i32`'s def_nr (registered as
            // a type alias for `integer size(4)` in `default/01_code.loft`)
            // so the vector storage path treats fn-ref vectors
            // identically to `vector<i32>`.
            Type::Function(_, _, _) => self.def_nr("i32"),
            _ => u32::MAX,
        }
    }

    #[must_use]
    /// Get the definition number for the given type.
    /// # Panics
    /// When no element of a type exists
    pub fn type_elm(&self, tp: &Type) -> u32 {
        match tp {
            Type::Rewritten(t) => self.type_elm(t),
            // @PLN25: `Optional(τ)` resolves to its base's element def (mirrors the sibling
            // `type_def_nr`). Missing this returned `u32::MAX` → `data.def(MAX)` panic / a
            // silently-skipped field for a nullable vector/field element.
            Type::Optional(t) => self.type_elm(t),
            Type::Integer(_) => self.source_nr(0, "integer"),
            Type::Boolean => self.source_nr(0, "boolean"),
            Type::Float => self.source_nr(0, "float"),
            Type::Text(_) => self.source_nr(0, "text"),
            Type::Single => self.source_nr(0, "single"),
            Type::Character => self.source_nr(0, "character"),
            Type::Routine(d_nr) | Type::Enum(d_nr, _, _) | Type::Reference(d_nr, _) => *d_nr,
            Type::Vector(tp, _) | Type::RefVar(tp) => {
                if let Type::Reference(td, _) = **tp {
                    td
                } else {
                    self.type_def_nr(tp)
                }
            }
            Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Radix(_, _, _) => self.source_nr(0, "reference"),
            // P189: tuple element types resolve to the synthetic
            // tuple struct registered by `tuple_def`.  Same lookup
            // as `type_def_nr`'s Tuple arm.
            Type::Tuple(types) => {
                let inner_names: Vec<String> = types.iter().map(|t| t.name(self)).collect();
                let name = format!("__tuple<{}>", inner_names.join(","));
                self.def_nr(&name)
            }
            // Plan-06 phase 4d.A.2 — fn-ref element types route to
            // `i32` (4-byte int alias) so vector storage is flat.
            // Same lookup as `type_def_nr`'s Function arm.
            Type::Function(_, _, _) => self.def_nr("i32"),
            _ => u32::MAX,
        }
    }

    /// Return a user-facing type name string for use by `type_name()`.
    #[must_use]
    pub fn type_name_str(&self, tp: &Type) -> String {
        match tp {
            Type::Optional(inner) => format!("{}?", self.type_name_str(inner)),
            Type::Unknown(_) => "unknown".to_string(),
            Type::Null => "null".to_string(),
            Type::Void => "void".to_string(),
            Type::Never => "never".to_string(),
            Type::Integer(s) if s.is_signed32_template() => "integer".to_string(),
            Type::Integer(_) => "integer".to_string(),
            Type::Boolean => "boolean".to_string(),
            Type::Float => "float".to_string(),
            Type::Single => "single".to_string(),
            Type::Character => "character".to_string(),
            Type::Text(_) => "text".to_string(),
            Type::Keys => "keys".to_string(),
            Type::Enum(d_nr, _, _) | Type::Reference(d_nr, _) => self.def(*d_nr).name.clone(),
            Type::RefVar(inner) => format!("&{}", self.type_name_str(inner)),
            Type::Vector(inner, _) => format!("vector<{}>", self.type_name_str(inner)),
            Type::Sorted(d_nr, _, _) => format!("sorted<{}>", self.def(*d_nr).name),
            Type::Index(d_nr, _, _) => format!("index<{}>", self.def(*d_nr).name),
            Type::Hash(d_nr, _, _) => format!("hash<{}>", self.def(*d_nr).name),
            Type::Routine(_) => "fn".to_string(),
            Type::Function(args, ret, _) => {
                let args_s: Vec<String> = args.iter().map(|a| self.type_name_str(a)).collect();
                format!("fn({}) -> {}", args_s.join(", "), self.type_name_str(ret))
            }
            Type::Iterator(inner, _) => format!("iterator<{}>", self.type_name_str(inner)),
            Type::Rewritten(inner) => self.type_name_str(inner),
            Type::Radix(d_nr, _, _) => format!("spatial<{}>", self.def(*d_nr).name),
            Type::Tuple(elems) => {
                let es: Vec<String> = elems.iter().map(|e| self.type_name_str(e)).collect();
                format!("({})", es.join(", "))
            }
        }
    }

    /**
    Return the rust type for definitions.
    # Panics
    When the rust type cannot be determined.
    */
    #[must_use]
    pub fn rust_type(&self, tp: &Type, context: &Context) -> String {
        if context == &Context::Reference {
            let mut result = String::new();
            result += "&";
            result += &self.rust_type(tp, &Context::Argument);
            return result;
        }
        match tp {
            // A declared `size(N)` picks the Rust type, the same way it picks the
            // storage width in `variables::size`.  The range ladder below has no
            // 4-byte rung, so without this a `size(4)` alias would be READ as an
            // `i64` while being WRITTEN as 4 bytes — the reader and the writer
            // disagreeing about one operand, which is how loft#654's jump
            // displacement silently landed in the wrong place.  Inert for every
            // alias that predates it: `u8` / `i8` / `u16` / `i16` force exactly
            // what the ladder already gives them, and plain `integer` forces
            // nothing.
            Type::Integer(s) if s.forced_size.map(NonZeroU8::get) == Some(4) => {
                if i64::from(s.min) >= 0 { "u32" } else { "i32" }
            }
            Type::Integer(s) if s.range() - 1 <= 255 && i64::from(s.min) >= 0 => "u8",
            Type::Integer(s) if s.range() - 1 <= 65536 && i64::from(s.min) >= 0 => "u16",
            Type::Integer(s) if s.range() - 1 <= 255 => "i8",
            Type::Integer(s) if s.range() - 1 <= 65536 => "i16",
            Type::Integer(_) => "i64",
            Type::Enum(_, false, _) => "u8",
            Type::Text(_) if context == &Context::Variable => "String",
            Type::Text(_) => "Str",
            // @PLN17: boolean is tri-state in storage (0/1/255).  The variable
            // form holds the raw byte (`u8`, null-capable); the expression form is
            // a 2-state `bool`.  Mirrors the text String/Str Context split above.
            Type::Boolean if context == &Context::Variable => "u8",
            Type::Boolean => "bool",
            Type::Float => "f64",
            Type::Single => "f32",
            Type::Character => "char",
            Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Hash(_, _, _)
            | Type::Sorted(_, _, _)
            | Type::RefVar(_)
            | Type::Enum(_, true, _)
            | Type::Index(_, _, _) => "DbRef",
            Type::Routine(_) => "u32",
            Type::Unknown(_) => "??",
            Type::Iterator(_, _) => "Iterator",
            Type::Keys => "&[Key]",
            // @PLN25: `Optional(τ)` shares its base's Rust type (sentinel storage) — mirrors
            // the `generation::rust_type` twin. Missing this panicked the native bridge
            // generator on an `integer?`/`text?` attribute.
            Type::Optional(inner) => return self.rust_type(inner, context),
            _ => panic!("Incorrect type {}", tp.name(self)),
        }
        .to_string()
    }

    pub fn find_unused(&self, diagnostics: &mut Diagnostics) {
        for (d_nr, def) in self.definitions.iter().enumerate() {
            if self.used_definitions.contains(&(d_nr as u32)) {
                for (a_nr, attr) in def.attributes.iter().enumerate() {
                    if !self.used_attributes.contains(&(d_nr as u32, a_nr)) {
                        diagnostics.add(
                            Level::Warning,
                            &format!(
                                "Unused field {}.{} at {}",
                                def.name, attr.name, def.position
                            ),
                        );
                    }
                }
            } else {
                diagnostics.add(
                    Level::Warning,
                    &format!("Unused definition {} at {}", def.name, def.position),
                );
            }
        }
    }

    /**
    Dump the internal parse tree to the standard output.
    # Panics
    Will not, this is to internal data structures instead of a file.
    */
    pub fn dump(&self, d_nr: u32) {
        let mut vars = Function::copy(&self.def(d_nr).variables);
        let mut s = Into { str: String::new() };
        self.show_code(&mut s, &mut vars, &self.def(d_nr).code, 0, true)
            .unwrap();
        println!("dump {}", s.str);
    }

    /**
    Dump the internal parse tree to the standard output.
    # Panics
    Will not, this is to internal data structures instead of a file.
    */
    pub fn dump_fn(&self, value: &Value, vars: &Function) {
        let mut vars = Function::copy(vars);
        let mut s = Into { str: String::new() };
        self.show_code(&mut s, &mut vars, value, 0, true).unwrap();
        println!("dump_fn {}", s.str);
    }

    /**
    Dump the internal parse tree to file.
    # Panics
    On incorrect rewritten code
    # Errors
    When the file cannot be written.
    */
    #[allow(clippy::too_many_lines)]
    pub fn show_code(
        &self,
        write: &mut dyn Write,
        vars: &mut Function,
        value: &Value,
        indent: u32,
        start: bool,
    ) -> Result<()> {
        if start {
            for _i in 0..indent {
                write!(write, "  ")?;
            }
        }
        match value {
            Value::Null => write!(write, "null"),
            Value::Int(i) => write!(write, "{i}i32"),
            Value::Enum(e, tp) => write!(write, "{e}u8({tp})"),
            Value::Boolean(true) => write!(write, "true"),
            Value::Boolean(_) => write!(write, "false"),
            Value::Float(f) => write!(write, "{f}f64"),
            Value::Long(l) => write!(write, "{l}i64"),
            Value::Single(f) => write!(write, "{f}f32"),
            Value::Text(t) => write!(write, "\"{t}\""),
            Value::Iter(_, _, _, _) => panic!("Rewrite!"),
            Value::Call(t, ex) => {
                write!(write, "{}(", self.def(*t).name)?;
                for (v_nr, v) in ex.iter().enumerate() {
                    if v_nr > 0 {
                        write!(write, ", ")?;
                    }
                    self.show_code(write, vars, v, indent, false)?;
                }
                write!(write, ")")
            }
            Value::CallRef(v, ex) => {
                write!(write, "fn_ref[{v}](")?;
                for (i, a) in ex.iter().enumerate() {
                    if i > 0 {
                        write!(write, ", ")?;
                    }
                    self.show_code(write, vars, a, indent, false)?;
                }
                write!(write, ")")
            }
            Value::Block(bl) => self.show_block(write, vars, bl, indent),
            Value::Var(v) => write!(write, "{}({})", vars.name(*v), vars.scope(*v)),
            Value::Set(v, to) => {
                if *v == u16::MAX {
                    write!(write, "unknown(??):?? = ")?;
                } else {
                    write!(
                        write,
                        "{}({}):{} = ",
                        vars.name(*v),
                        vars.scope(*v),
                        vars.tp(*v).show(self, vars)
                    )?;
                }
                self.show_code(write, vars, to, indent, false)
            }
            Value::Return(ex) => {
                write!(write, "return ")?;
                self.show_code(write, vars, ex, indent, false)
            }
            Value::Insert(i) => self.show_insert(write, vars, i, indent),
            Value::Break(v) => write!(write, "break({v})"),
            Value::BreakWith(v, expr) => {
                write!(write, "break({v}) ")?;
                self.show_code(write, vars, expr, indent, false)
            }
            Value::Continue(v) => write!(write, "continue({v})"),
            Value::If(test, t, f) => {
                write!(write, "if ")?;
                self.show_code(write, vars, test, indent, false)?;
                write!(write, " ")?;
                self.show_code(write, vars, t, indent, false)?;
                write!(write, " else ")?;
                self.show_code(write, vars, f, indent, false)
            }
            Value::Loop(lp) => self.show_loop(write, vars, lp, indent),
            Value::Drop(v) => {
                write!(write, "drop ")?;
                self.show_code(write, vars, v, indent, false)
            }
            Value::Keys(keys) => {
                write!(write, "&{keys:?}")
            }
            Value::Line(line) => write!(write, "[{line}] "),
            Value::Tuple(elems) => {
                write!(write, "(")?;
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(write, ", ")?;
                    }
                    self.show_code(write, vars, e, indent, false)?;
                }
                write!(write, ")")
            }
            Value::TupleGet(var, idx) => {
                write!(write, "{}.{idx}", vars.name(*var))
            }
            Value::TuplePut(var, idx, val) => {
                write!(write, "{}.{idx} = ", vars.name(*var))?;
                self.show_code(write, vars, val, indent, false)
            }
            Value::Yield(inner) => {
                write!(write, "yield ")?;
                self.show_code(write, vars, inner, indent, false)
            }
            Value::FnRef(d_nr, clos_var, _) => {
                write!(write, "FnRef({d_nr}, {})", vars.name(*clos_var))
            }
            Value::FnRefDnr(v_nr) => {
                write!(write, "FnRefDnr({})", vars.name(*v_nr))
            }
            Value::Parallel(arms) => {
                writeln!(write, "parallel {{")?;
                for arm in arms {
                    self.show_code(write, vars, arm, indent + 1, true)?;
                    writeln!(write, ";")?;
                }
                for _i in 0..indent {
                    write!(write, "  ")?;
                }
                write!(write, "}}")
            }
            // Plan-07 phase 1 — Span is transparent in pretty-print.
            Value::Span(b) => self.show_code(write, vars, &b.1, indent, start),
            Value::ParFor(b) => {
                let stitch_name = match b.stitch_id {
                    0 => "concat",
                    1 => "discard",
                    2 => "reduce",
                    3 => "queue",
                    _ => "?",
                };
                write!(write, "par_for[{stitch_name}](x={}, r=", b.x_var)?;
                if b.r_var == u16::MAX {
                    write!(write, "_")?;
                } else {
                    write!(write, "{}", b.r_var)?;
                }
                write!(write, ", input=")?;
                self.show_code(write, vars, &b.input, 0, false)?;
                write!(write, ", worker=")?;
                self.show_code(write, vars, &b.worker, 0, false)?;
                write!(write, ", threads=")?;
                self.show_code(write, vars, &b.threads, 0, false)?;
                writeln!(write, ", body=")?;
                self.show_code(write, vars, &b.body, indent + 1, true)?;
                write!(write, ")")
            }
            // Phase 09 phase 00 step 0.7 — RawExpr is a codegen-internal
            // pretty-print should never see it (it's only created during
            // native emission, downstream of this IR walker).
            Value::RawExpr(s) => write!(write, "raw({s})"),
        }
    }

    fn show_block(
        &self,
        write: &mut dyn Write,
        vars: &mut Function,
        bl: &crate::data::Block,
        indent: u32,
    ) -> Result<()> {
        if !bl.operators.is_empty() {
            writeln!(
                write,
                "{{#{}({}):{}",
                bl.name,
                bl.scope,
                bl.result.show(self, vars)
            )?;
            let mut starting = true;
            for val in &bl.operators {
                self.show_code(write, vars, val, indent + 1, starting)?;
                starting = if matches!(val, Value::Line(_)) {
                    false
                } else {
                    writeln!(write, ";")?;
                    true
                };
            }
            for _i in 0..indent {
                write!(write, "  ")?;
            }
            write!(
                write,
                "}}#{}({}):{}",
                bl.name,
                bl.scope,
                bl.result.show(self, vars)
            )?;
        }
        Ok(())
    }

    fn show_loop(
        &self,
        write: &mut dyn Write,
        vars: &mut Function,
        lp: &Block,
        indent: u32,
    ) -> Result<()> {
        writeln!(write, "loop {{#{}_{}", lp.name, lp.scope)?;
        for val in &lp.operators {
            self.show_code(write, vars, val, indent + 1, true)?;
            writeln!(write, ";")?;
        }
        for _i in 0..indent {
            write!(write, "  ")?;
        }
        write!(write, "}}#{}_{}", lp.name, lp.scope)?;
        Ok(())
    }

    fn show_insert(
        &self,
        write: &mut dyn Write,
        vars: &mut Function,
        items: &[Value],
        indent: u32,
    ) -> Result<()> {
        writeln!(write, "{{ !! INSERT")?;
        for v in items {
            self.show_code(write, vars, v, indent + 1, true)?;
            writeln!(write)?;
        }
        for _i in 0..indent {
            write!(write, "  ")?;
        }
        write!(write, "}}")
    }
}

#[test]
fn value_sizes() {
    // Debugging function to validate the sizes of the variants for the Value enum.
    assert_eq!(size_of::<Value>(), 32);
    assert_eq!(size_of::<Vec<Value>>(), 24);
    assert_eq!(size_of::<Box<Value>>(), 8);
    assert_eq!(size_of::<(u8, u32)>(), 8); // Int
    assert_eq!(size_of::<(u8, u8, u16)>(), 4); // Enum
    assert_eq!(size_of::<(u8, f64)>(), 16); // Float
    assert_eq!(size_of::<(u8, String)>(), 32); // Text
    assert_eq!(size_of::<(u8, u32, Vec<Value>)>(), 32); // Call
    assert_eq!(size_of::<(u8, Box<(Vec<Value>, Type, &'static str)>)>(), 16); // Block
    assert_eq!(size_of::<(u8, u16, Box<Value>)>(), 16); // Set
    assert_eq!(size_of::<(u8, Box<Value>, Box<Value>, Box<Value>)>(), 32); // If
    assert_eq!(size_of::<(u8, Box<Value>, Box<Value>)>(), 24); // Iter
    assert_eq!(size_of::<(u8, Box<(Position, Value)>)>(), 16); // Span (plan-07 phase 1)
}

#[test]
fn span_clone_and_eq_roundtrip() {
    // Plan-07 phase 1, step 1.1 acceptance: construct a Span, clone it,
    // debug-format it, assert round-trip equality.  Span is a transparent
    // wrapper, so the cloned tree must compare equal to the original.
    let pos = Position {
        file: "x.loft".to_string(),
        line: 17,
        pos: 4,
    };
    let v = Value::Span(Box::new((pos.clone(), Value::Int(7))));
    let v2 = v.clone();
    assert_eq!(v, v2, "clone must be Eq");
    let dbg = format!("{v:?}");
    assert!(
        dbg.contains("x.loft") && dbg.contains("17") && dbg.contains("Int(7)"),
        "debug shows file, line, and inner: {dbg}"
    );
}

#[test]
fn span_unspan_strips_wrapper() {
    // Plan-07 phase 1, step 1.B.0 acceptance: `unspan()` returns the
    // inner non-Span node, recursing through any number of wraps.
    let pos = Position {
        file: "y.loft".to_string(),
        line: 3,
        pos: 7,
    };
    let inner = Value::Int(42);
    // Single wrap.
    let wrapped = Value::Span(Box::new((pos.clone(), inner.clone())));
    assert_eq!(wrapped.unspan(), &inner);
    // Doubly wrapped.
    let double = Value::Span(Box::new((pos.clone(), wrapped.clone())));
    assert_eq!(double.unspan(), &inner);
    // Non-Span passes through unchanged.
    assert_eq!(inner.unspan(), &inner);
    // Mutable variant.
    let mut wrapped_mut = Value::Span(Box::new((pos, Value::Int(99))));
    if let Value::Int(n) = wrapped_mut.unspan_mut() {
        *n = 100;
    }
    assert_eq!(wrapped_mut.unspan(), &Value::Int(100));
}

#[cfg(test)]
mod caller_graph_tests {
    use super::{Block, Data, DefType, Deps, Type, Value};
    use crate::lexer::Position;

    /// Build a synthetic Data with three user fns:
    ///   fn0: calls fn1
    ///   fn1: calls fn2 + fn2 again (test dedup)
    ///   fn2: leaf (no calls)
    /// Plus a stdlib-style entry (Value::Null body) that should be
    /// excluded from user_fn_d_nrs.
    fn build_test_data() -> Data {
        let mut d = Data::new();
        let pos = Position {
            file: String::new(),
            line: 0,
            pos: 0,
        };
        // fn0: calls fn1 with no args
        let d0 = d.add_def("fn0", &pos, DefType::Function);
        // fn1: calls fn2 twice
        let d1 = d.add_def("fn1", &pos, DefType::Function);
        // fn2: leaf
        let d2 = d.add_def("fn2", &pos, DefType::Function);
        // n_native: stdlib-style (Value::Null body — excluded)
        let _dn = d.add_def("n_native", &pos, DefType::Function);
        // Set codes.
        d.definitions[d0 as usize].code = Value::Call(d1, vec![]);
        d.definitions[d1 as usize].code = Value::Block(Box::new(Block {
            name: "test",
            operators: vec![Value::Call(d2, vec![]), Value::Call(d2, vec![])],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));
        d.definitions[d2 as usize].code = Value::Int(42);
        // n_native stays Value::Null.
        d
    }

    #[test]
    fn user_fn_d_nrs_excludes_null_body_natives() {
        let d = build_test_data();
        let user_fns = d.user_fn_d_nrs();
        assert_eq!(user_fns.len(), 3, "fn0/fn1/fn2 only — n_native excluded");
        // fn0/1/2 are at indices 0/1/2.
        assert_eq!(user_fns, vec![0, 1, 2]);
    }

    #[test]
    fn callers_of_finds_direct_callers() {
        let d = build_test_data();
        // fn1's caller is fn0.
        assert_eq!(d.callers_of(1), vec![0]);
        // fn2's caller is fn1 (deduplicated despite two call sites).
        assert_eq!(d.callers_of(2), vec![1]);
    }

    #[test]
    fn callers_of_uncalled_returns_empty() {
        let d = build_test_data();
        // fn0 has no callers.
        assert!(d.callers_of(0).is_empty());
    }

    #[test]
    fn callers_of_caches_after_first_call() {
        let d = build_test_data();
        // First call builds the cache.
        let _ = d.callers_of(1);
        // Cache is now populated.
        assert!(d.caller_index.get().is_some());
        // Second call returns the same answer cheaply.
        assert_eq!(d.callers_of(1), vec![0]);
    }

    #[test]
    fn callers_of_walks_block_and_call_args_recursively() {
        let mut d = Data::new();
        let pos = Position {
            file: String::new(),
            line: 0,
            pos: 0,
        };
        let d_inner = d.add_def("inner", &pos, DefType::Function);
        let d_outer = d.add_def("outer", &pos, DefType::Function);
        // outer's body wraps inner's call inside an If condition.
        d.definitions[d_inner as usize].code = Value::Int(1);
        d.definitions[d_outer as usize].code = Value::If(
            Box::new(Value::Call(d_inner, vec![])),
            Box::new(Value::Int(0)),
            Box::new(Value::Int(0)),
        );
        assert_eq!(d.callers_of(d_inner), vec![d_outer]);
    }

    /// Regression: the synthetic `main_vector<T>` wrapper (and its
    /// `tuple_def` / `fn_ref_def` siblings) must be stamped `source = 0`.
    /// A cache reload's `rebuild_indices` keys `def_names` on each def's own
    /// `source`, so a wrapper first created under a non-zero source (a `use`d
    /// module) used to lose its global `(name, 0)` binding on a warm start —
    /// `name_type("main_vector<T>", other_source)` then returned `u16::MAX` and
    /// codegen emitted `OpDatabase(db_tp=u16::MAX)` → `claim(size=0)`
    /// "Incomplete record" (the crawler `build_walls` panic).  Stamping
    /// `source = 0` makes the global binding survive the round-trip.
    #[test]
    fn synthetic_vector_wrapper_is_global_source_zero() {
        let mut d = Data::new();
        let pos = Position {
            file: String::new(),
            line: 0,
            pos: 0,
        };
        let foo = d.add_def("Foo", &pos, DefType::Struct);
        let mut lexer = crate::lexer::Lexer::from_str("", "test");
        // Register the wrapper from a NON-zero source (as if from a `use`d module).
        d.source = 5;
        let vd = d.vector_def(&mut lexer, &Type::Reference(foo, Deps::none()));
        assert_eq!(
            d.definitions[vd as usize].source, 0,
            "main_vector<Foo> wrapper must be stamped source=0"
        );
        // After `rebuild_indices` (the cache-reload step) the wrapper must still
        // resolve from a DIFFERENT source via the `(name, 0)` fallback.
        d.rebuild_indices();
        d.source = 7;
        assert_ne!(
            d.def_nr("main_vector<Foo>"),
            u32::MAX,
            "wrapper must resolve cross-source after rebuild_indices"
        );
    }
}

#[cfg(test)]
mod type_name_user_facing_tests {
    //! Plan-07 phase 6.1 — `Type::name()` must produce loft-surface
    //! syntax for every variant.  Pre-fix many variants fell through
    //! to the Display impl which lower-cased the debug format
    //! (e.g. `tuple([integer(...), text([])])`); user-visible error
    //! messages now render proper loft syntax.

    use super::{Data, DefType, Deps, IntegerSpec, Type};
    use crate::lexer::Position;

    fn make_data() -> Data {
        let mut d = Data::new();
        let pos = Position {
            file: String::new(),
            line: 0,
            pos: 0,
        };
        d.add_def("Foo", &pos, DefType::Struct);
        d
    }

    #[test]
    fn unknown_renders_as_unknown() {
        let d = Data::new();
        assert_eq!(Type::Unknown(0).name(&d), "unknown");
    }

    #[test]
    fn null_renders_as_null() {
        let d = Data::new();
        assert_eq!(Type::Null.name(&d), "null");
    }

    #[test]
    fn void_renders_as_void() {
        let d = Data::new();
        assert_eq!(Type::Void.name(&d), "void");
    }

    #[test]
    fn never_renders_as_never() {
        let d = Data::new();
        assert_eq!(Type::Never.name(&d), "never");
    }

    #[test]
    fn boolean_renders_as_boolean() {
        let d = Data::new();
        assert_eq!(Type::Boolean.name(&d), "boolean");
    }

    #[test]
    fn float_renders_as_float() {
        let d = Data::new();
        assert_eq!(Type::Float.name(&d), "float");
    }

    #[test]
    fn single_renders_as_single() {
        let d = Data::new();
        assert_eq!(Type::Single.name(&d), "single");
    }

    #[test]
    fn character_renders_as_character() {
        let d = Data::new();
        assert_eq!(Type::Character.name(&d), "character");
    }

    #[test]
    fn integer_default_renders_as_integer() {
        let d = Data::new();
        assert_eq!(Type::Integer(IntegerSpec::signed32()).name(&d), "integer");
    }

    #[test]
    fn integer_byte_renders_as_byte() {
        let d = Data::new();
        let spec = IntegerSpec {
            min: 0,
            max: 256,
            not_null: false,
            forced_size: None,
        };
        assert_eq!(Type::Integer(spec).name(&d), "byte");
    }

    #[test]
    fn integer_bounded_renders_with_range() {
        let d = Data::new();
        let spec = IntegerSpec {
            min: 1,
            max: 99,
            not_null: false,
            forced_size: None,
        };
        assert_eq!(Type::Integer(spec).name(&d), "integer(1, 99)");
    }

    #[test]
    fn keys_renders_as_keys() {
        let d = Data::new();
        assert_eq!(Type::Keys.name(&d), "keys");
    }

    #[test]
    fn iterator_renders_with_inner_type() {
        let d = Data::new();
        let it = Type::Iterator(Box::new(Type::Boolean), Box::new(Type::Null));
        assert_eq!(it.name(&d), "iterator<boolean>");
    }

    #[test]
    fn tuple_renders_as_paren_csv() {
        let d = Data::new();
        let t = Type::Tuple(vec![Type::Boolean, Type::Text(Deps::none())]);
        assert_eq!(t.name(&d), "(boolean, text)");
    }

    #[test]
    fn function_void_return_omits_arrow() {
        let d = Data::new();
        let f = Type::Function(vec![Type::Boolean], Box::new(Type::Void), Deps::none());
        assert_eq!(f.name(&d), "fn(boolean)");
    }

    #[test]
    fn function_with_return_includes_arrow() {
        let d = Data::new();
        let f = Type::Function(
            vec![Type::Boolean, Type::Float],
            Box::new(Type::Text(Deps::none())),
            Deps::none(),
        );
        assert_eq!(f.name(&d), "fn(boolean, float) -> text");
    }

    #[test]
    fn reference_renders_struct_name() {
        let d = make_data();
        let foo_d_nr = d.def_nr("Foo");
        assert_eq!(Type::Reference(foo_d_nr, Deps::none()).name(&d), "Foo");
    }

    #[test]
    fn vector_of_text_renders_with_angle_brackets() {
        let d = Data::new();
        let v = Type::Vector(Box::new(Type::Text(Deps::none())), Deps::none());
        assert_eq!(v.name(&d), "vector<text>");
    }

    #[test]
    fn vector_of_unknown_renders_as_bare_vector() {
        let d = Data::new();
        let v = Type::Vector(Box::new(Type::Unknown(0)), Deps::none());
        assert_eq!(v.name(&d), "vector");
    }

    #[test]
    fn ref_var_renders_with_ampersand() {
        let d = Data::new();
        let r = Type::RefVar(Box::new(Type::Text(Deps::none())));
        assert_eq!(r.name(&d), "&text");
    }

    // @PLN25 — the `τ?` former's invariants: idempotent (N-Idem), normalising over
    // non-values, renders as `τ?`, and peels back to its base.
    #[test]
    fn optional_is_idempotent_normalising_and_renders_with_question_mark() {
        let d = Data::new();
        let txt = Type::Text(Deps::none());
        let o = Type::optional(txt.clone());
        assert!(matches!(o, Type::Optional(_)));
        assert_eq!(o.name(&d), "text?"); // renders `τ?`
        assert!(o.peel_optional().1); // reads as optional
        assert_eq!(*o.base(), txt); // base is the wrapped type
        assert_eq!(Type::optional(o.clone()), o); // N-Idem: no Optional(Optional)
        assert_eq!(Type::optional(Type::Never), Type::Never); // normalise non-values
        assert_eq!(Type::optional(Type::Null), Type::Null);
        assert!(!txt.peel_optional().1); // a plain type is not optional
    }
}
