// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 arc A0 — a typed field cursor over a store record, the precondition
//! for the store-backed `Data` accessor seam.
//!
//! `Store` (`src/store.rs`) stays a dumb word-addressed buffer with raw
//! `(rec, fld)` primitives (`get_int` / `set_int` / `get_byte` / …).  This
//! module sits **beside** `data.rs` at the IR layer and binds a `&Store` + a
//! record number once, so an IR field read becomes a named, width-typed call
//! (`r.int(fld)`, `r.float(fld)`, `r.enum_tag()`) instead of open-coded
//! `store.get_int(rec, fld)` offset arithmetic.
//!
//! Why this and not a method on `Store`: the cursor is an *IR-side* concept —
//! it is where the eventual store-backed accessors (`def.name()`,
//! `value.as_call()`, plan-54 arc C) will live.  `Store` should not grow IR
//! knowledge; the IR layer wraps the store's primitives.
//!
//! PURE PASS-THROUGH: every method delegates to the existing `get_*` / `set_*`
//! primitive — same bounds checks, same invalid-access sentinels (`i64::MIN`,
//! `f64::NAN`, …).  No new `unsafe`.  Additive only; no existing caller is
//! required to migrate.
//!
//! SCOPE (A0): the simple scalar primitives — the widths `--native` emits for
//! struct/enum fields (see plan-54 § "What the generated Rust gives us": the
//! generated reads are exactly these `get_int(db.pos+N)` / `get_byte(...,32,0)`
//! calls).  The 12-byte stored `DbRef` (`Parts::DbRef`) and following a child
//! record / vector are NOT simple `get_*` wraps and are added when arc A/B/C
//! needs them, not guessed here.

use crate::store::Store;

/// A typed, read-only view of one record (`rec`) in a [`Store`].
///
/// Obtain via [`RecordCursor::new`].  Field offsets (`fld`) and the `min`
/// shift for narrow integers are still the caller's to supply — A0 removes the
/// `store` + `rec` repetition and names the width; a schema-aware layer that
/// knows a `Type`'s field offsets can wrap this later (plan-54 arc A).
#[derive(Clone, Copy)]
pub struct RecordCursor<'a> {
    store: &'a Store,
    rec: u32,
}

impl<'a> RecordCursor<'a> {
    /// Bind a read cursor to record `rec` in `store`.
    #[must_use]
    #[inline]
    pub fn new(store: &'a Store, rec: u32) -> Self {
        RecordCursor { store, rec }
    }

    /// The record number this cursor is bound to.
    #[must_use]
    #[inline]
    pub fn rec(&self) -> u32 {
        self.rec
    }

    /// 8-byte `integer` field (`i64::MIN` on invalid).  → [`Store::get_int`].
    #[must_use]
    #[inline]
    pub fn int(&self, fld: u32) -> i64 {
        self.store.get_int(self.rec, fld)
    }

    /// 8-byte `long` field (`i64::MIN` on invalid).  → [`Store::get_long`].
    #[must_use]
    #[inline]
    pub fn long(&self, fld: u32) -> i64 {
        self.store.get_long(self.rec, fld)
    }

    /// 4-byte unsigned raw (collection headers, string-record pointers/lengths).
    /// → [`Store::get_u32_raw`].
    #[must_use]
    #[inline]
    pub fn u32(&self, fld: u32) -> u32 {
        self.store.get_u32_raw(self.rec, fld)
    }

    /// 4-byte signed raw (type tags, `i32::MIN`-relative sentinels).
    /// → [`Store::get_i32_raw`].
    #[must_use]
    #[inline]
    pub fn i32(&self, fld: u32) -> i32 {
        self.store.get_i32_raw(self.rec, fld)
    }

    /// 2-byte `+1`-encoded short (`min`-relative; `i32::MIN` null).
    /// → [`Store::get_short`].
    #[must_use]
    #[inline]
    pub fn short(&self, fld: u32, min: i32) -> i32 {
        self.store.get_short(self.rec, fld, min)
    }

    /// 2-byte direct-encoded short (`Parts::ShortRaw` elements).
    /// → [`Store::get_i16_raw`].
    #[must_use]
    #[inline]
    pub fn i16_raw(&self, fld: u32, min: i32) -> i32 {
        self.store.get_i16_raw(self.rec, fld, min)
    }

    /// 1-byte `min`-relative byte (`i32::MIN` null).  → [`Store::get_byte`].
    /// The enum discriminant of a struct-enum record is a `byte(0)` field —
    /// read it with `min = 0`.
    #[must_use]
    #[inline]
    pub fn byte(&self, fld: u32, min: i32) -> i32 {
        self.store.get_byte(self.rec, fld, min)
    }

    /// `float` field (`f64::NAN` on invalid).  → [`Store::get_float`].
    #[must_use]
    #[inline]
    pub fn float(&self, fld: u32) -> f64 {
        self.store.get_float(self.rec, fld)
    }

    /// `single` field (`f32::NAN` on invalid).  → [`Store::get_single`].
    #[must_use]
    #[inline]
    pub fn single(&self, fld: u32) -> f32 {
        self.store.get_single(self.rec, fld)
    }

    /// Boolean bit selected by `mask`.  → [`Store::get_boolean`].
    #[must_use]
    #[inline]
    pub fn boolean(&self, fld: u32, mask: u8) -> bool {
        self.store.get_boolean(self.rec, fld, mask)
    }

    /// A `text` field: read the 4-byte string-record pointer at `fld`, then
    /// follow it to the string bytes.  Mirrors the generated-native pattern
    /// `store.get_str(store.get_u32_raw(rec, fld))`.  Returns `STRING_NULL`
    /// (the empty-string sentinel) for a null/zero pointer.
    #[must_use]
    #[inline]
    pub fn text(&self, fld: u32) -> &'a str {
        self.store.get_str(self.store.get_u32_raw(self.rec, fld))
    }
}

/// A typed, mutable view of one record (`rec`) in a [`Store`].
///
/// Obtain via [`RecordCursorMut::new`].  Each setter returns the primitive's
/// `bool` (`true` on a successful write; `false` on an invalid `(rec, fld)`),
/// preserving the underlying contract exactly.
pub struct RecordCursorMut<'a> {
    store: &'a mut Store,
    rec: u32,
}

impl<'a> RecordCursorMut<'a> {
    /// Bind a mutable cursor to record `rec` in `store`.
    #[inline]
    pub fn new(store: &'a mut Store, rec: u32) -> Self {
        RecordCursorMut { store, rec }
    }

    /// The record number this cursor is bound to.
    #[must_use]
    #[inline]
    pub fn rec(&self) -> u32 {
        self.rec
    }

    /// A read cursor over the same record (for read-then-write paths).
    #[must_use]
    #[inline]
    pub fn as_read(&self) -> RecordCursor<'_> {
        RecordCursor {
            store: self.store,
            rec: self.rec,
        }
    }

    /// 8-byte `integer` field.  → [`Store::set_int`].
    #[inline]
    pub fn set_int(&mut self, fld: u32, val: i64) -> bool {
        self.store.set_int(self.rec, fld, val)
    }

    /// 8-byte `long` field.  → [`Store::set_long`].
    #[inline]
    pub fn set_long(&mut self, fld: u32, val: i64) -> bool {
        self.store.set_long(self.rec, fld, val)
    }

    /// 4-byte unsigned raw.  → [`Store::set_u32_raw`].
    #[inline]
    pub fn set_u32(&mut self, fld: u32, val: u32) -> bool {
        self.store.set_u32_raw(self.rec, fld, val)
    }

    /// 4-byte signed raw.  → [`Store::set_i32_raw`].
    #[inline]
    pub fn set_i32(&mut self, fld: u32, val: i32) -> bool {
        self.store.set_i32_raw(self.rec, fld, val)
    }

    /// 2-byte `+1`-encoded short.  → [`Store::set_short`].
    #[inline]
    pub fn set_short(&mut self, fld: u32, min: i32, val: i32) -> bool {
        self.store.set_short(self.rec, fld, min, val)
    }

    /// 2-byte direct-encoded short.  → [`Store::set_i16_raw`].
    #[inline]
    pub fn set_i16_raw(&mut self, fld: u32, min: i32, val: i32) -> bool {
        self.store.set_i16_raw(self.rec, fld, min, val)
    }

    /// 1-byte `min`-relative byte.  → [`Store::set_byte`].
    #[inline]
    pub fn set_byte(&mut self, fld: u32, min: i32, val: i32) -> bool {
        self.store.set_byte(self.rec, fld, min, val)
    }

    /// `float` field.  → [`Store::set_float`].
    #[inline]
    pub fn set_float(&mut self, fld: u32, val: f64) -> bool {
        self.store.set_float(self.rec, fld, val)
    }

    /// `single` field.  → [`Store::set_single`].
    #[inline]
    pub fn set_single(&mut self, fld: u32, val: f32) -> bool {
        self.store.set_single(self.rec, fld, val)
    }

    /// Boolean bit selected by `mask`.  → [`Store::set_boolean`].
    #[inline]
    pub fn set_boolean(&mut self, fld: u32, mask: u8, val: bool) -> bool {
        self.store.set_boolean(self.rec, fld, mask, val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch in-use store with one claimed record big enough for a few
    /// 8-byte fields plus a string record.
    fn scratch() -> (Store, u32) {
        let mut s = Store::new(64);
        s.free = false; // mark in-use so validate() does not reject it
        let rec = s.claim(8); // 8 words = 64 bytes of field space
        (s, rec)
    }

    /// Every scalar width written through the mutable cursor reads back through
    /// the read cursor identically to the raw `get_*` / `set_*` primitives —
    /// the cursor is a faithful pass-through.
    #[test]
    fn cursor_round_trips_each_width() {
        let (mut s, rec) = scratch();
        {
            let mut w = RecordCursorMut::new(&mut s, rec);
            assert!(w.set_int(8, 0x0102_0304_0506_0708));
            assert!(w.set_long(16, -42));
            assert!(w.set_u32(24, 0xDEAD_BEEF));
            assert!(w.set_i32(28, -7));
            assert!(w.set_float(32, 3.5));
            assert!(w.set_single(40, 2.5));
            assert!(w.set_boolean(44, 0b0000_0100, true));
            assert!(w.set_byte(45, 0, 200));
            assert!(w.set_short(46, 100, 1000));
        }
        let r = RecordCursor::new(&s, rec);
        assert_eq!(r.rec(), rec);
        // Cursor reads match the raw primitive reads exactly.
        assert_eq!(r.int(8), s.get_int(rec, 8));
        assert_eq!(r.int(8), 0x0102_0304_0506_0708);
        assert_eq!(r.long(16), s.get_long(rec, 16));
        assert_eq!(r.long(16), -42);
        assert_eq!(r.u32(24), s.get_u32_raw(rec, 24));
        assert_eq!(r.u32(24), 0xDEAD_BEEF);
        assert_eq!(r.i32(28), s.get_i32_raw(rec, 28));
        assert_eq!(r.i32(28), -7);
        // bit-pattern equality: the cursor read must be byte-identical to the
        // raw read (faithful pass-through), and equal to the value written.
        assert_eq!(r.float(32).to_bits(), s.get_float(rec, 32).to_bits());
        assert_eq!(r.float(32).to_bits(), 3.5_f64.to_bits());
        assert_eq!(r.single(40).to_bits(), s.get_single(rec, 40).to_bits());
        assert_eq!(r.single(40).to_bits(), 2.5_f32.to_bits());
        assert!(r.boolean(44, 0b0000_0100));
        assert!(!r.boolean(44, 0b0000_0010));
        assert_eq!(r.byte(45, 0), s.get_byte(rec, 45, 0));
        assert_eq!(r.byte(45, 0), 200);
        assert_eq!(r.short(46, 100), s.get_short(rec, 46, 100));
        assert_eq!(r.short(46, 100), 1000);
    }

    /// The enum-discriminant pattern: a struct-enum record stores its tag as a
    /// `byte(0)` field at offset 0 (as `--native` emits); the cursor reads it
    /// with `byte(0, 0)`.
    #[test]
    fn cursor_reads_enum_discriminant() {
        let (mut s, rec) = scratch();
        RecordCursorMut::new(&mut s, rec).set_byte(0, 0, 5);
        assert_eq!(RecordCursor::new(&s, rec).byte(0, 0), 5);
    }

    /// A `text` field round-trips: store a string record, write its pointer at
    /// the field, read it back through the cursor's follow-the-pointer path.
    #[test]
    fn cursor_text_field_round_trips() {
        let (mut s, rec) = scratch();
        let str_rec = s.set_str("hello cursor");
        {
            let mut w = RecordCursorMut::new(&mut s, rec);
            assert!(w.set_u32(8, str_rec));
        }
        assert_eq!(RecordCursor::new(&s, rec).text(8), "hello cursor");
    }

    /// `as_read` on a mutable cursor yields a read view of the same record.
    #[test]
    fn mut_cursor_as_read_sees_same_record() {
        let (mut s, rec) = scratch();
        let mut w = RecordCursorMut::new(&mut s, rec);
        w.set_int(8, 99);
        assert_eq!(w.as_read().int(8), 99);
        assert_eq!(w.as_read().rec(), rec);
    }
}
