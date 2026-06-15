// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Runtime value types for store pointers, string views, and collection keys.
//!
//! - [`DbRef`] — universal pointer into a [`Store`](crate::store::Store):
//!   `(store_nr, rec, pos)`.  12 bytes on the stack.
//! - [`Str`] — 16-byte borrowed string view `(ptr, len)`.  Used for text
//!   arguments on the stack; the backing data lives in a `String` or in
//!   the static `text_code` buffer.
//! - [`Key`] / [`Content`] — typed keys and values for hash/sorted/index
//!   collections, used by the collection lookup operators.

#![allow(dead_code)]

use crate::store::Store;
use std::cmp::Ordering;
use std::collections::hash_map::RandomState;
use std::fmt::Formatter;
use std::hash::{BuildHasher, DefaultHasher, Hash, Hasher};
use std::sync::OnceLock;

/// P253 fix (2026-05-11) — process-wide seeded `RandomState` so the
/// `keys::hash` / `key_hash` functions produce a different hash
/// distribution across processes.  Without seeding, `DefaultHasher::new()`
/// constructs a SipHash-1-3 hasher with fixed seed (k0=0, k1=0); every
/// loft process used the identical hash function, so an attacker who
/// supplied hash-table keys could pre-compute N strings that all
/// collided to a single bucket → O(N²) insertion / lookup.  Same root
/// cause as the 2011/2012 hash-DoS in Python / Ruby / PHP / Java /
/// Node.js (CVE-2011-4815 et al.).
///
/// `RandomState::new()` seeds from `getrandom` on first call; we
/// memoise on a `OnceLock` so subsequent hashers share the same seed
/// (otherwise resize / lookup would see a different distribution than
/// insertion).  Lookups via `hasher()` clone the seed-state and build
/// a fresh `DefaultHasher` per call — same shape as `HashMap`.
fn hasher_state() -> &'static RandomState {
    static STATE: OnceLock<RandomState> = OnceLock::new();
    STATE.get_or_init(RandomState::new)
}

#[must_use]
fn build_hasher() -> DefaultHasher {
    hasher_state().build_hasher()
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Str {
    pub ptr: *const u8,
    pub len: u32,
}

impl Str {
    #[must_use]
    pub fn new(v: &str) -> Str {
        Str {
            ptr: v.as_ptr(),
            len: v.len() as u32,
        }
    }

    #[must_use]
    pub fn str<'a>(&self) -> &'a str {
        // @P355: the cheap pointer guards stay (a null or low/dangling ptr —
        // e.g. an empty Rust `String`'s `NonNull::dangling()` ≈ 0x1 — reads
        // back as ""), but there is deliberately NO `len` cap here.  A prior
        // `self.len > 10_000_000` clause (added in #140 for the stack_trace
        // introspection path) silently truncated any text over 10 MB to "" on
        // the UNIVERSAL correctness accessor — so `file(big).content()` and any
        // >10 MB string read back empty (silent data loss; bit the training
        // port on a 38 MB JSON export).  The garbage-tolerance that motivated
        // the cap belongs to `try_str()` (the fallible variant the introspection
        // / trace dump in `src/state/debug.rs` actually uses), not here: `str()`
        // is on the value-read hot path and must return the real bytes.  If a
        // genuinely corrupt `Str` ever reaches here, a loud fault surfaces the
        // producing bug — strictly better than silent corruption of valid data.
        if self.ptr.is_null() || (self.ptr as usize) < (1 << 16) {
            return "";
        }
        unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.ptr, self.len as usize))
        }
    }

    /// Safe conversion for trace/debug display.  Returns `None` when the pointer
    /// or length look like uninitialised stack garbage, avoiding SIGSEGV.
    #[must_use]
    pub fn try_str<'a>(&self) -> Option<&'a str> {
        // An empty Rust `String` has its `ptr` set to `NonNull::dangling()` —
        // typically a small alignment-sized value like 0x1.  Treat `len == 0`
        // as a valid empty string regardless of the ptr, so trace output
        // shows `""` instead of `<raw:0x1>` (parser_debug dump).
        if self.len == 0 {
            return Some("");
        }
        if self.ptr.is_null()
            || (self.ptr as usize) < (1 << 16)
            || self.len > 10_000_000
            || (self.ptr as usize).checked_add(self.len as usize).is_none()
        {
            return None;
        }
        let slice = unsafe { std::slice::from_raw_parts(self.ptr, self.len as usize) };
        std::str::from_utf8(slice).ok()
    }
}

impl std::ops::Deref for Str {
    type Target = str;
    fn deref(&self) -> &str {
        self.str()
    }
}

impl std::fmt::Display for Str {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.str())
    }
}

impl PartialEq<str> for Str {
    fn eq(&self, other: &str) -> bool {
        self.str() == other
    }
}

impl PartialEq<&str> for Str {
    fn eq(&self, other: &&str) -> bool {
        self.str() == *other
    }
}

impl PartialEq<Str> for &str {
    fn eq(&self, other: &Str) -> bool {
        *self == other.str()
    }
}

impl PartialEq<String> for Str {
    fn eq(&self, other: &String) -> bool {
        self.str() == other.as_str()
    }
}

impl PartialEq<Str> for String {
    fn eq(&self, other: &Str) -> bool {
        self.as_str() == other.str()
    }
}

impl PartialOrd<str> for Str {
    fn partial_cmp(&self, other: &str) -> Option<std::cmp::Ordering> {
        self.str().partial_cmp(other)
    }
}

impl PartialOrd<&str> for Str {
    fn partial_cmp(&self, other: &&str) -> Option<std::cmp::Ordering> {
        self.str().partial_cmp(*other)
    }
}

impl PartialOrd<Str> for &str {
    fn partial_cmp(&self, other: &Str) -> Option<std::cmp::Ordering> {
        (*self).partial_cmp(other.str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    pub type_nr: i8,
    pub position: u16,
}

#[derive(Clone)]
pub enum Content {
    Long(i64),
    Float(f64),
    Single(f32),
    Str(Str),
}

impl std::fmt::Display for Content {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Content::Long(l) => write!(f, "{l}"),
            Content::Float(l) => write!(f, "{l}"),
            Content::Single(l) => write!(f, "{l}"),
            Content::Str(l) => write!(f, "{}", l.str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub enum Simple {
    Number(i64),
    Text(String),
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub struct DbRef {
    pub store_nr: u16,
    pub rec: u32,
    pub pos: u32,
}

impl DbRef {
    /// The canonical null-reference sentinel (`store_nr == u16::MAX`).  Store
    /// allocation asserts `slot != u16::MAX`, so this is distinct from every
    /// real store — and from a valid-but-empty vector (a real store with
    /// `rec == 0`).  It is the one representation of an *absent* heap value:
    /// struct reference, struct-enum, and — plan-25 (@PLN25) — vector.
    pub const NULL: DbRef = DbRef {
        store_nr: u16::MAX,
        rec: 0,
        pos: 0,
    };

    /// True when this reference is the null sentinel (absent value).  The
    /// single home for the null test: every store accessor consults it before
    /// dereferencing, so an absent value never indexes `stores[u16::MAX]`.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        self.store_nr == u16::MAX
    }

    #[must_use]
    pub fn plus(&self, pos: u32) -> DbRef {
        DbRef {
            store_nr: self.store_nr,
            rec: self.rec,
            pos: self.pos + pos,
        }
    }

    #[must_use]
    pub fn min(&self, size: u32) -> DbRef {
        DbRef {
            store_nr: self.store_nr,
            rec: self.rec,
            pos: self.pos - size,
        }
    }

    pub fn push<T>(&mut self, stores: &mut [Store], value: T) {
        *stores[self.store_nr as usize].addr_mut::<T>(self.rec, self.pos) = value;
        self.pos += size_of::<T>() as u32;
    }
}

#[inline]
fn single_cmp(v1: f32, v2: f32) -> Ordering {
    v1.total_cmp(&v2)
}

#[inline]
fn float_cmp(v1: f64, v2: f64) -> Ordering {
    v1.total_cmp(&v2)
}

/// Use-after-free detector (`LOFT_UAF=1`).  When on, `free_named` records each
/// freed store slot; after the op completes, the dispatch loop scans every
/// active frame for a variable that (a) still holds a `DbRef` into a
/// just-freed slot and (b) has a FUTURE READ in its function's bytecode (a
/// `OpVarRef`/`OpVarVector` load of its slot not consumed by an `OpFreeRef`-
/// family op).  Under the single-owner store model (Plan-57: no refcount) that
/// combination means the store was freed while still in use — the premature
/// free behind the store-lifetime Heisenbug family (#248/#290/#303) — and the
/// panic lands AT the offending free.  One cached env read; off by default.
pub fn uaf_check_enabled() -> bool {
    static UAF: OnceLock<bool> = OnceLock::new();
    *UAF.get_or_init(|| std::env::var_os("LOFT_UAF").is_some())
}

#[must_use]
pub fn store<'a>(r: &DbRef, stores: &'a [Store]) -> &'a Store {
    debug_assert!(
        (r.store_nr as usize) < stores.len(),
        "DbRef store_nr {} out of bounds (allocations.len() = {})",
        r.store_nr,
        stores.len()
    );
    &stores[r.store_nr as usize]
}

#[must_use]
pub fn mut_store<'a>(r: &DbRef, stores: &'a mut [Store]) -> &'a mut Store {
    debug_assert!(
        (r.store_nr as usize) < stores.len(),
        "DbRef store_nr {} out of bounds (allocations.len() = {})",
        r.store_nr,
        stores.len()
    );
    &mut stores[r.store_nr as usize]
}

#[must_use]
pub fn compare(rec1: &DbRef, rec2: &DbRef, stores: &[Store], keys: &[Key]) -> Ordering {
    for key in keys {
        let pos1 = rec1.pos + u32::from(key.position);
        let pos2 = rec2.pos + u32::from(key.position);
        let c = compare_ref(rec1, rec2, stores, key, pos1, pos2);
        if c != Ordering::Equal {
            return c;
        }
    }
    Ordering::Equal
}

#[must_use]
pub fn key_compare(key: &[Content], rec: &DbRef, stores: &[Store], keys: &[Key]) -> Ordering {
    for (k_nr, val) in key.iter().enumerate() {
        let k = &keys[k_nr];
        let pos_r = u32::from(k.position);
        let c = compare_key(val, rec, stores, k, pos_r);
        if c != Ordering::Equal {
            return c;
        }
    }
    Ordering::Equal
}

fn compare_key(k: &Content, record: &DbRef, stores: &[Store], key: &Key, pos: u32) -> Ordering {
    let s = store(record, stores);
    let c = match (k, key.type_nr.abs()) {
        (Content::Long(v), 1) => v.cmp(&s.get_int(record.rec, record.pos + pos)),
        (Content::Long(v), 2) => v.cmp(&s.get_long(record.rec, record.pos + pos)),
        (Content::Single(v), 3) => single_cmp(*v, s.get_single(record.rec, record.pos + pos)),
        (Content::Float(v), 4) => float_cmp(*v, s.get_float(record.rec, record.pos + pos)),
        (Content::Str(v), 6) => v
            .str()
            .cmp(s.get_str(s.get_u32_raw(record.rec, record.pos + pos))),
        // Narrow integer keys — match hash_ref / get_key.
        (Content::Long(v), 8) => v.cmp(&i64::from(s.get_i32_raw(record.rec, record.pos + pos))),
        (Content::Long(v), 9) => v.cmp(&i64::from(s.get_short(record.rec, record.pos + pos, 0))),
        (Content::Long(v), 10) => v.cmp(&i64::from(s.get_byte(record.rec, record.pos + pos, 0))),
        (Content::Long(v), 11) => {
            let raw: u16 = *s.addr(record.rec, record.pos + pos);
            v.cmp(&i64::from(raw as i16))
        }
        (Content::Long(v), _) => v.cmp(&i64::from(s.get_byte(record.rec, record.pos + pos, 0))),
        _ => panic!("Undefined compare {k:?} vs {}", key.type_nr),
    };
    if key.type_nr < 0 { c.reverse() } else { c }
}

fn compare_ref(r1: &DbRef, r2: &DbRef, stores: &[Store], key: &Key, p1: u32, p2: u32) -> Ordering {
    let s = store(r1, stores);
    let c = match key.type_nr.abs() {
        1 => s.get_int(r1.rec, p1).cmp(&s.get_int(r2.rec, p2)),
        2 => s.get_long(r1.rec, p1).cmp(&s.get_long(r2.rec, p2)),
        3 => single_cmp(s.get_single(r1.rec, p1), s.get_single(r2.rec, p2)),
        4 => float_cmp(s.get_float(r1.rec, p1), s.get_float(r2.rec, p2)),
        6 => s
            .get_str(s.get_u32_raw(r1.rec, p1))
            .cmp(s.get_str(s.get_u32_raw(r2.rec, p2))),
        8 => s.get_i32_raw(r1.rec, p1).cmp(&s.get_i32_raw(r2.rec, p2)),
        9 => s.get_short(r1.rec, p1, 0).cmp(&s.get_short(r2.rec, p2, 0)),
        10 => s.get_byte(r1.rec, p1, 0).cmp(&s.get_byte(r2.rec, p2, 0)),
        11 => {
            let v1: u16 = *s.addr(r1.rec, p1);
            let v2: u16 = *s.addr(r2.rec, p2);
            (v1 as i16).cmp(&(v2 as i16))
        }
        _ => s.get_byte(r1.rec, p1, 0).cmp(&s.get_byte(r2.rec, p2, 0)),
    };
    if key.type_nr < 0 { c.reverse() } else { c }
}

#[must_use]
pub fn get_key(record: &DbRef, stores: &[Store], keys: &[Key]) -> Vec<Content> {
    let mut result = Vec::new();
    for k in keys {
        let p = record.pos + u32::from(k.position);
        match k.type_nr.abs() {
            1 => {
                let v = store(record, stores).get_int(record.rec, p);
                result.push(Content::Long(v));
            }
            2 => {
                let v = store(record, stores).get_long(record.rec, p);
                result.push(Content::Long(v));
            }
            6 => {
                let v =
                    store(record, stores).get_str(store(record, stores).get_u32_raw(record.rec, p));
                result.push(Content::Str(Str::new(v)));
            }
            8 => {
                let v = store(record, stores).get_i32_raw(record.rec, p);
                result.push(Content::Long(i64::from(v)));
            }
            9 => {
                let v = store(record, stores).get_short(record.rec, p, 0);
                result.push(Content::Long(i64::from(v)));
            }
            10 => {
                let v = store(record, stores).get_byte(record.rec, p, 0);
                result.push(Content::Long(i64::from(v)));
            }
            11 => {
                let raw: u16 = *store(record, stores).addr(record.rec, p);
                result.push(Content::Long(i64::from(raw as i16)));
            }
            _ => {
                let v = store(record, stores).get_byte(record.rec, p, 0);
                result.push(Content::Long(i64::from(v)));
            }
        }
    }
    result
}

#[must_use]
pub fn get_simple(record: &DbRef, stores: &[Store], keys: &[Key]) -> Vec<Simple> {
    let mut result = Vec::new();
    let k = get_key(record, stores, keys);
    for f in k {
        match f {
            Content::Long(l) => result.push(Simple::Number(l)),
            Content::Str(s) => result.push(Simple::Text(s.str().to_string())),
            _ => {}
        }
    }
    result
}

#[must_use]
pub fn hash(rec: &DbRef, stores: &[Store], keys: &[Key]) -> u64 {
    let mut hasher = build_hasher();
    for key in keys {
        let pos = rec.pos + u32::from(key.position);
        hash_ref(rec, stores, key, pos, &mut hasher);
    }
    hasher.finish()
}

#[must_use]
pub fn key_hash(key: &[Content]) -> u64 {
    let mut hasher = build_hasher();
    for k in key {
        match k {
            Content::Long(l) => l.hash(&mut hasher),
            Content::Str(s) => s.str().hash(&mut hasher),
            _ => (),
        }
    }
    hasher.finish()
}

fn hash_ref(r: &DbRef, stores: &[Store], key: &Key, p: u32, hasher: &mut DefaultHasher) {
    let s = store(r, stores);
    match key.type_nr.abs() {
        1 => s.get_int(r.rec, p).hash(hasher),
        2 => s.get_long(r.rec, p).hash(hasher),
        3 | 4 => (),
        6 => s.get_str(s.get_u32_raw(r.rec, p)).hash(hasher),
        // Narrow-integer key storage (Parts::Int / Short / ShortRaw / Byte).
        // Each yields an i64 view so the hash matches `get_key`'s
        // Content::Long(i64) reconstruction at the lookup site.
        8 => i64::from(s.get_i32_raw(r.rec, p)).hash(hasher),
        9 => i64::from(s.get_short(r.rec, p, 0)).hash(hasher),
        10 => i64::from(s.get_byte(r.rec, p, 0)).hash(hasher),
        11 => {
            // Parts::ShortRaw stores a raw u16; no min shift, no null
            // sentinel.  Sign-extend through i16 → i64 to match
            // compare_key / get_key's reconstruction.
            let raw: u16 = *s.addr(r.rec, p);
            i64::from(raw as i16).hash(hasher);
        }
        _ => i64::from(s.get_byte(r.rec, p, 0)).hash(hasher),
    }
}

#[cfg(test)]
mod p355_large_str {
    use super::Str;

    /// @P355: `Str::str()` must NOT silently truncate a legitimately-large
    /// text.  A prior `self.len > 10_000_000` guard returned "" for any text
    /// over 10 MB on the universal correctness accessor — so `file(big)
    /// .content()` (and any >10 MB string) read back empty.  Verify a 12 MB
    /// backing buffer round-trips its full length.
    #[test]
    fn str_does_not_cap_large_text() {
        let big = "x".repeat(12_000_000);
        let s = Str::new(&big);
        assert_eq!(
            s.str().len(),
            12_000_000,
            "str() must return the full length, not cap at 10M"
        );
        assert_eq!(s.len, 12_000_000);
        // Just below and just above the old 10 MB cap both read fully.
        let at_cap = "y".repeat(10_485_760);
        assert_eq!(Str::new(&at_cap).str().len(), 10_485_760);
    }

    /// The cheap pointer guards are preserved: a null/dangling ptr (e.g. an
    /// empty Rust String's NonNull::dangling()) still reads back as "".
    #[test]
    fn str_empty_and_null_guard_preserved() {
        let empty = String::new(); // ptr = dangling (small, < 1<<16)
        let s = Str::new(&empty);
        assert_eq!(s.str(), "");
        let nullish = Str {
            ptr: std::ptr::null(),
            len: 5,
        };
        assert_eq!(nullish.str(), "");
    }
}
