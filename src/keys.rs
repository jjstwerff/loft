// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I71 — DbRef pointers & collection keys

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

/// `LOFT_UAF_SRC` — the cheap companion to `LOFT_UAF`: record each free's pc and
/// report when `do_copy_record` reads a still-freed SOURCE, but SKIP the expensive
/// per-op frame scan (which floods + slows a real-scale run past its timeout before
/// the faulting copy is reached).  Enables the same `uaf_freed_this_op` recording.
pub fn uaf_src_enabled() -> bool {
    static UAF_SRC: OnceLock<bool> = OnceLock::new();
    *UAF_SRC.get_or_init(|| std::env::var_os("LOFT_UAF_SRC").is_some())
}

/// Either UAF instrument is on (gates the `uaf_freed_this_op` recording in `free_named`).
#[must_use]
pub fn uaf_any_enabled() -> bool {
    uaf_check_enabled() || uaf_src_enabled()
}

/// `LOFT_POISON=1` (@PLN54 S3) — the arena poison-on-free keystone. When on, `free_named`
/// overwrites a freed store's payload (past the 8-byte size header) with `0xDEADBEEF`, so a
/// dangling-`DbRef` read after free hits loud, deterministic garbage — an out-of-range
/// `store_nr` that trips the read guards — instead of silent stale data. This is the
/// store-internal use-after-free blind spot Miri/ASan/Valgrind all share, because loft's
/// arena "free" is not a libc `free()`. Works on BOTH backends (native-generated code calls
/// the same `free_named`), on any rustc, no nightly. One cached env read; off by default.
/// (The buried `LOFT_LOG=poison_free` path still flips the same poison; this is its dedicated,
/// documented front door, and the one the fuzz-proof harness drives.)
#[must_use]
pub fn poison_enabled() -> bool {
    static POISON: OnceLock<bool> = OnceLock::new();
    *POISON.get_or_init(|| std::env::var_os("LOFT_POISON").is_some())
}

/// `LOFT_COPY_DUMP=1` (@PLN90 phase 1) — print one line per executed deep STRUCTURE copy
/// (a record copy `OpCopyRecord`, or a vector append that deep-copies its source elements
/// `vector_add`). The instrument that makes copies VISIBLE: it is the runtime ground truth
/// for every copy + its size, so the compile-time copy-vs-borrow decision can be checked to
/// cover them all (COPY_DIAGNOSTICS.md). One cached env read; off by default, no hot-path cost.
#[must_use]
pub fn copy_dump_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_COPY_DUMP").is_some())
}

/// `LOFT_PLN25_OPT=1` (@PLN25 slice a, IN PROGRESS) — make the scalar/field postfix `?`
/// construct the real `Type::Optional` former instead of the Phase-0 no-op. Opt-IN while the
/// slice-(b) peel audit (the ~280 `match Type` consuming sites that must peel `Optional`) is
/// incomplete: OFF (default) keeps the suite byte-identical; ON exercises the new marker so
/// the remaining mis-routes surface. Flip to default-on (or retire the gate) once green on
/// both backends. See plans/25-nullable-sequences/RESUME.md § Step 3.
#[must_use]
pub fn pln25_optional_enabled() -> bool {
    // @PLN25 step f — DEFAULT-ON: the dense/non-null scalar model is the default,
    // so the `Optional` marker is always constructed. Implied by `pln25_dn1_enabled`
    // (`LOFT_PLN25_OFF` opts the whole model out during the transition).
    pln25_dn1_enabled()
}

/// `LOFT_PLN25_DN3=1` (@PLN25 slice c, IN PROGRESS) — the `(N-Store)` teeth: reject an
/// un-discharged `τ?` (Optional) flowing into a NON-null target (a plain `τ`). The nullable
/// value must first be discharged with `??` or `match` (which yield the non-null base). Opt-IN
/// while the enforcement + the DN1 default flip are being built; implies `LOFT_PLN25_OPT` (the
/// check is meaningless without the real `Optional` marker). OFF keeps the slice-(b)
/// behaviour-preserving implicit unwrap. See plans/25-nullable-sequences/RESUME.md § Step 3.
#[must_use]
pub fn pln25_dn3_enabled() -> bool {
    // @PLN25 step f — DEFAULT-ON: the `(N-Store)` teeth are the default. Implied by
    // `pln25_dn1_enabled` (`LOFT_PLN25_OFF` opts the whole model out).
    pln25_dn1_enabled()
}

/// `LOFT_PLN25_DN1=1` (@PLN25 Phase-2 CONTRACT, IN PROGRESS) — the DEFAULT FLIP: a plain scalar
/// (`integer`, `text`, `bool`, …) is NON-NULL by default; `τ?` is the only nullable form. Turns
/// `IntegerSpec.not_null` default `false → true` (and the analog for other scalars rides
/// `Type::Optional`), so a bare `null` returned/stored into a plain scalar is rejected (beyond the
/// Optional `(N-Store)` teeth, which only catch `Optional → non-null`). Opt-IN while the `.loft`
/// sweep migrates the misses to `?`; OFF keeps the nullable-by-default behaviour. Implies
/// `LOFT_PLN25_DN3` (the `(N-Store)` teeth) and `LOFT_PLN25_OPT` (the `Optional` marker). See
/// plans/25-nullable-sequences/dn1-flip-blast-radius.md and RESUME.md § Step 3.
#[must_use]
pub fn pln25_dn1_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    // @PLN25 step f — THE FLIP: the dense/non-null scalar model is now the DEFAULT.
    // A plain scalar is non-null; `τ?` is the only nullable form. The old opt-in envs
    // (`LOFT_PLN25_OPT`/`_DN3`/`_DN1`) are now redundant no-ops. `LOFT_PLN25_OFF` still
    // toggles the model off, but F1b(b)'s stdlib `τ?` overloads collide when `Optional`
    // is a no-op, so the stdlib no longer LOADS gate-OFF — the escape hatch is retired.
    *ON.get_or_init(|| std::env::var_os("LOFT_PLN25_OFF").is_none())
}

/// `LOFT_PLN25_F2=1` (@PLN25 F2 — the deferred RANGE RECONCILIATION, IN PROGRESS) — a plain
/// (non-`Optional`) narrow integer is NON-null under DN1, so its usable range is the FULL width
/// (no reserved null sentinel); only an `Optional`-wrapped narrow (`u8?`) reserves the top value
/// as the sentinel. This completes DN1 for the *range* dimension (the `IntegerSpec.not_null`
/// default that DN1 left at `false` for narrow scalars, deferring the storage change), which makes
/// `not null` redundant — the prerequisite for retiring it. Opt-IN while the blast radius (~47
/// `not_null` readers + store-codec baked layouts + narrow-int field storage, near loft's #1
/// weakness) is measured + migrated; OFF keeps the current (pre-F2) sentinel reservation for plain
/// narrow fields/locals. See plans/25-nullable-sequences/RESUME.md § FINISH LINE F2.
#[must_use]
pub fn pln25_f2_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| pln25_dn1_enabled() && std::env::var_os("LOFT_PLN25_F2").is_some())
}

/// `LOFT_UAF_REUSE` (detector b) — at `copy_record`, when the source slot is LIVE
/// (`free=false`) but structurally invalid for the copy's `tp` (a `validate_claims`
/// failure), the slot was freed-then-reused as a different record since the source ref
/// was minted: the stale-REUSED read behind the post-reuse SIGSEGV (#462 @ 3546),
/// caught before the fault. (A same-type reuse slips through, but that wouldn't fault.)
pub fn uaf_reuse_enabled() -> bool {
    static UAF_REUSE: OnceLock<bool> = OnceLock::new();
    *UAF_REUSE.get_or_init(|| std::env::var_os("LOFT_UAF_REUSE").is_some())
}

/// `LOFT_UAF_GEN` (detector c) — the SOUND reused detector: a per-slot generation
/// (bumped on free) plus a shadow stack stamping each operand-stack DbRef's gen at push;
/// at consume a shadow-vs-current mismatch is a slot freed-then-reused since the ref was
/// pushed. Catches the reused read that `store_nr` alone cannot — no `DbRef` widening.
pub fn uaf_gen_enabled() -> bool {
    static UAF_GEN: OnceLock<bool> = OnceLock::new();
    *UAF_GEN.get_or_init(|| std::env::var_os("LOFT_UAF_GEN").is_some())
}

/// `LOFT_WATCH_STORE=<n>` — the write-watch for cluster-462's root: after each
/// `copy_record` whose DESTINATION is store `<n>`, scan the just-written record's text
/// fields for an out-of-bounds pointer and report the op that produced it (pc/line +
/// source). This catches the BUILD copy that first writes a garbage text-pointer into the
/// watched store, distinguishing "uninitialised fresh slot" from "propagated over-free".
/// Returns the watched store number, or None when unset.
#[must_use]
pub fn watch_store() -> Option<u16> {
    static WATCH: OnceLock<Option<u16>> = OnceLock::new();
    *WATCH.get_or_init(|| {
        std::env::var("LOFT_WATCH_STORE")
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
    })
}

thread_local! {
    /// `LOFT_UAF_GEN` (c): generation per store slot — bumped on each free. A DbRef
    /// minted while a slot is at gen G is stale if the slot reaches gen >G (freed +
    /// reused) before the ref is read. The gen distinguishes the OLD occupant from a
    /// re-claimed NEW one — so it does NOT false-positive on free-then-reclaim (the
    /// flaw that sank the store_nr-only scan).
    static SLOT_GEN: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Shadow of the operand stack: the slot-gen stamped on each DbRef at PUSH, keyed by
    /// its eval-stack byte offset. At POP, a shadow-vs-current mismatch = reused-since-push.
    static STACK_SHADOW: std::cell::RefCell<std::collections::HashMap<u32, u32>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Bump store `slot`'s generation (called from `free_named` under `LOFT_UAF_GEN`).
pub fn uaf_bump_gen(slot: u16) {
    SLOT_GEN.with(|g| {
        let mut v = g.borrow_mut();
        let i = slot as usize;
        if i >= v.len() {
            v.resize(i + 1, 0);
        }
        v[i] = v[i].wrapping_add(1);
    });
}

/// Store `slot`'s current generation (0 if never freed).
#[must_use]
pub fn uaf_slot_gen(slot: u16) -> u32 {
    SLOT_GEN.with(|g| g.borrow().get(slot as usize).copied().unwrap_or(0))
}

/// Stamp the generation of the DbRef pushed at eval-stack offset `off`.
pub fn uaf_stamp_shadow(off: u32, generation: u32) {
    STACK_SHADOW.with(|s| {
        s.borrow_mut().insert(off, generation);
    });
}

/// The gen stamped at eval-stack offset `off`, if any.
#[must_use]
pub fn uaf_shadow_gen(off: u32) -> Option<u32> {
    STACK_SHADOW.with(|s| s.borrow().get(&off).copied())
}

/// Consume the shadow stamp at `off` (called on a DbRef POP). The stack is LIFO, so a
/// stamp is valid only for its matching pop; clearing it stops a stale stamp from
/// surviving to an unrelated later read once a non-DbRef push reuses the offset.
pub fn uaf_clear_shadow(off: u32) {
    STACK_SHADOW.with(|s| {
        s.borrow_mut().remove(&off);
    });
}

thread_local! {
    /// `LOFT_UAF` companion (cluster-462 tool-gap #1): store slot -> the
    /// execution `code_pos` of its most-recent free.  The frame-var scan in
    /// `uaf_scan_freed` misses a stale DbRef that lives on the OPERAND STACK
    /// (the source `OpCopyRecord` pops) — so this records every free's pc and
    /// `do_copy_record` reports it when it reads a still-freed source, pinning
    /// the premature-free op without a full operand-stack scan.
    static FREED_AT: std::cell::RefCell<std::collections::HashMap<u16, (u32, u32, u16)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Record (under `LOFT_UAF`) that store `slot` was freed while executing `pc` in
/// function `d_nr`, by the op whose opcode is `op` (names the freeing op category:
/// a scope-exit `OpFreeRef`, an `OpFreeRefIfDistinct`, or a `copy_record` free-bit).
pub fn uaf_record_free(slot: u16, pc: u32, d_nr: u32, op: u16) {
    FREED_AT.with(|m| {
        m.borrow_mut().insert(slot, (pc, d_nr, op));
    });
}

/// The `(code_pos, d_nr, op_code)` of `slot`'s most-recent recorded free, if any.
#[must_use]
pub fn uaf_freed_pc(slot: u16) -> Option<(u32, u32, u16)> {
    FREED_AT.with(|m| m.borrow().get(&slot).copied())
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
