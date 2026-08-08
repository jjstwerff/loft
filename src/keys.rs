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

/// Build a deterministic hasher whose bucket distribution depends only on
/// `seed`.  `DefaultHasher::new()` fixes the SipHash-1-3 keys (k0=k1=0), so
/// the SAME `seed` maps a key to the SAME bucket in EVERY process.  That is
/// what makes a persisted hash portable: a reader restores the seed stored
/// in the hash's own bucket record (`hash.rs`) and re-derives identical
/// buckets, so a cross-process / remote lookup lands in the right bucket.
///
/// The per-hash random `seed` (drawn by [`fresh_seed`] when a hash is first
/// populated) preserves the P253 hash-DoS defense (2026-05-11): without a
/// seed, every loft process shared the fixed-key hasher, so an attacker who
/// supplied keys could pre-compute N strings that all collide to a single
/// bucket → O(N²) insertion / lookup (the 2011/2012 Python / Ruby / PHP /
/// Java / Node hash-DoS, CVE-2011-4815 et al.).  An attacker cannot
/// pre-compute collisions without knowing the hash's seed.
#[must_use]
fn seeded_hasher(seed: u64) -> DefaultHasher {
    let mut hasher = DefaultHasher::new();
    hasher.write_u64(seed);
    hasher
}

/// `LOFT_HASH_SEED`, read once: the fixed seed every hash uses instead of an
/// unpredictable one.  `None` (unset, empty, or not a number) keeps the random
/// default.
///
/// loft#710 — a random seed is stored in the hash's bucket record and decides
/// the bucket ORDER, so a persisted store built twice from identical data came
/// out byte-different every run.  For a pipeline that publishes stores with a
/// per-block checksum that is fatal: "the data changed" and "it was rebuilt"
/// become indistinguishable, and so does a freshness gate built on either.
///
/// It stays OPT-IN because the randomness is the P253 hash-DoS defense and a
/// program taking attacker-supplied keys still wants it.  A build that needs
/// reproducible artifacts sets this; a server does not.
fn fixed_seed() -> Option<u64> {
    static FIXED: OnceLock<Option<u64>> = OnceLock::new();
    *FIXED.get_or_init(|| {
        std::env::var("LOFT_HASH_SEED")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
    })
}

/// Draw a fresh, unpredictable 64-bit seed for a newly-populated hash table.
/// Each hash gets its own random seed (the P253 DoS defense); the seed is
/// then stored IN the hash's bucket record so any reader re-derives the same
/// buckets (see [`seeded_hasher`]).  Never returns 0 — a stored seed of 0
/// marks an un-seeded (empty / legacy) bucket record.
///
/// `LOFT_HASH_SEED` replaces the draw with one fixed value for every hash, so a
/// run is byte-reproducible — see [`fixed_seed`].
#[must_use]
pub fn fresh_seed() -> u64 {
    let s = fixed_seed().unwrap_or_else(|| RandomState::new().build_hasher().finish());
    if s == 0 { 0x9E37_79B9_7F4A_7C15 } else { s }
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
    /// The field's storage START — the `min` of a `Parts::Byte` / `Parts::Short`, which
    /// is what those two widths subtract when they store a value and must add back when
    /// they read one (loft#812).
    ///
    /// It has to travel WITH the key because the comparison happens in `compare_key` /
    /// `hash_ref` / `get_key`, none of which can see the type table. Before this field
    /// they passed a literal `0`, so the record side decoded `val - min` while the lookup
    /// side had the user's `val`: the two differed by exactly `min` and never compared
    /// Equal. A key declared `i8`, `i16` or `integer limit(min, max)` with a non-zero
    /// `min` therefore inserted fine, counted fine, and could never be looked up. Ordering
    /// survived, because subtracting a constant is monotonic — only equality was wrong,
    /// which is why it read as "the record is missing" rather than as a decode bug.
    ///
    /// `0` for every width that stores raw (`integer`, `long`, `text`, `float`, `single`,
    /// `Parts::Int`, `Parts::ShortRaw`) — for those it is inert, and it is also the
    /// correct value for a `u8` / `u16` whose range starts at zero.
    pub start: i32,
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
    uaf_check_enabled() || uaf_src_enabled() || uaf_gen_enabled()
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

/// `LOFT_NO_BRIDGE_ORPHAN_FREE=1` — @PLN118 arc F opt-out / differential switch. The shared-store
/// bridge (`native_lib::shared_bridge_wrapper`) frees a FALLBACK destination record it allocated
/// itself when the inner fn ignored the retbuf and returned a fresh store (a struct-literal return
/// does) — otherwise that record is orphaned across the interp↔cdylib boundary, one leaked store
/// per call. Setting this reproduces the pre-fix leak (the arc-D "second implementation to flip
/// to" + the differential leak oracle's positive control). Default OFF (the fix is active). One
/// cached env read; the free itself only fires when the fallback was allocated.
#[must_use]
pub fn bridge_orphan_free_disabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_BRIDGE_ORPHAN_FREE").is_some())
}

/// `LOFT_REPORT_COPIES=1` (or the `--report-copies` CLI flag) — @PLN90 Step 5. The USER-FACING
/// copy report: the *unbound* structure copies (Avoidable + Forced) with a source location, the
/// copied type, and a fix hint, plus a rollup + the ranked Avoidable worklist. Enables the
/// bound-vs-unbound survival classification (like `LOFT_COPY_SURVIVAL`) so the buckets are real.
/// Off by default, one cached env read. See `use_analysis::report_copies`, COPY_DIAGNOSTICS.md.
#[must_use]
pub fn report_copies_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_REPORT_COPIES").is_some())
}

/// `LOFT_EXPLAIN=1` (or `--explain`) — @PLN131: print the FIX line(s) under each diagnostic
/// that carries them.
///
/// A diagnostic says what is WRONG; a fix says what to write INSTEAD, and that second half is
/// where most of the learning is. Opt-in, and showing only — nothing rewrites source. The
/// concept named on each line (`move`) is a handle onto the feature catalogue rather than an
/// explanation inline, because the explaining belongs in the docs.
///
/// Off by default: the fix lines are worth reading when you are acting on a diagnostic and
/// noise when you are not, and loft is meant to be quiet.
#[must_use]
pub fn explain_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_EXPLAIN").is_some())
}

/// `LOFT_COPY_MANIFEST=1` — @PLN130: the emission-manifest GUARD. Each generator records every
/// deep copy it WRITES; this reports the ones the copy diagnostic produced no verdict for.
///
/// Not a user diagnostic — it reports a hole in the COMPILER (a copy no analysis accounts for),
/// so its audience is CI and this repo, not a loft author. Compile-time only: nothing it measures
/// reaches a compiled program. Opt-in while the uncovered set is non-empty; the intent is a CI
/// gate once it reaches zero, not a message on a user's build.
#[must_use]
pub fn copy_manifest_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_COPY_MANIFEST").is_some())
}

/// `LOFT_WARN_COPIES=1` — @PLN90 W5: the ENFORCED copy lint. Routes the user-facing copy report's
/// **Avoidable** rows (a still-live structure duplicated where a borrow/move would do) through the
/// normal `Level::Warning` diagnostics channel, so they surface as warnings during a normal compile
/// (not only under `--report-copies`). Opt-IN + default OFF: the Avoidable set is not yet drained
/// (A2/field-return still copies), so default-on would over-warn on copies the compiler is about to
/// stop making — promote to default once that set is empty. One cached env read. See
/// `use_analysis::warn_copies`, COPY_DIAGNOSTICS.md.
#[must_use]
pub fn warn_copies_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_WARN_COPIES").is_some())
}

/// @PLN107: the dead-store lint. Warns when a non-escaping local OWNS a copy that is mutated via
/// an `OpSet*` but whose value is never read (the copy-mutate footgun, e.g. `d = self.data;
/// d[i] = x` where the bind COPIES so the write is lost). **Default ON** (S5) after the S4
/// suite-wide sweep proved the whole corpus clean (stdlib + all `tests/scripts` + fixture libs +
/// `tests/lib` + examples); `LOFT_NO_DEAD_STORES` opts out. One cached env read. See
/// `use_analysis::warn_dead_stores`, `doc/claude/plans/107-dead-code-lint/`.
#[must_use]
pub fn dead_stores_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_DEAD_STORES").is_none())
}

/// `LOFT_LINK_WIDEN=1` — @PLN102 transparent-link widening. **OPT-IN, DEFAULT OFF** — built +
/// validated (steps 1–4) but NOT defaulted on: step 5's copy-count measurement found the win is ~0
/// in practice (the read-only-both field-bind pattern it targets is essentially absent in real loft
/// code — even the ~2000-line viewer eliminates 0 copies), while defaulting on adds per-bind analysis
/// cost (an O(n) alias scan). C86's revisit trigger ("a profiler shows bind-copies dominating a real
/// consumer") is NOT met, so the mechanism stays opt-in, ready for when a consumer demonstrates the
/// win. When set, `analyze_fn` → `ElidePlan` links a copy-fill bind `a = s.v` where it is provably
/// SAFE (`link_is_safe`) AND UNOBSERVABLE (`link_is_unobservable`, alias-aware) — the observable
/// result is unchanged (link ≡ copy there). Cached once. See alias-where-correct{,-build}.md.
#[must_use]
pub fn link_widen_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_LINK_WIDEN").is_some())
}

/// `LOFT_DUMP_LINK_OBS=1` — @PLN102 transparent-link widening, build step 3 (the observability
/// oracle). REPORT-ONLY: emits one `link-obs-dbg: fn=… var=… base=… unobs=0|1` line per copy-fill
/// bind — the "would a shared-store link be UNOBSERVABLE (copy ≡ link)?" verdict: neither the local
/// nor the source's store is mutated after the bind, ALIAS-AWARE (a write through a sibling `&`-alias
/// of the source counts). Drives NO codegen — byte-identical. Pinned by `tests/link_obs_oracle.rs`.
#[must_use]
pub fn dump_link_obs() -> bool {
    std::env::var_os("LOFT_DUMP_LINK_OBS").is_some()
}

/// `LOFT_DUMP_LINK_SAFE=1` — @PLN102 transparent-link widening, build step 2 (the safety oracle).
/// REPORT-ONLY: when set, the parser emits one `link-safe-dbg: fn=… var=… base=… safe=0|1` line per
/// copy-fill bind, the conservative "would a shared-store link be UAF-safe here?" verdict (source
/// outlives the local + the local does not escape). Drives NO codegen — it only prints, so it is
/// byte-identical to today. Pinned against the safety matrix by `tests/link_safe_oracle.rs`.
#[must_use]
pub fn dump_link_safe() -> bool {
    std::env::var_os("LOFT_DUMP_LINK_SAFE").is_some()
}

/// @PLN90 W1: the temporary-subject borrow-return materialise (the coordinated promotion-verdict
/// fix) — **DEFAULT ON**. When a `-> vector` fn's tail is a buffer-ABI call returning a borrow of a
/// TEMPORARY subject the fn constructs (`fn h() -> vector<E> { g(Filled{..}) }`), the borrowed view
/// dangles once the temp is freed at scope exit — a use-after-free (`cell-escape-temp`, loud on both
/// backends under `LOFT_POISON`). This SKIPS promoting the subject work-ref (keeps it a distinct
/// local, freed after the copy) and MATERIALISES the buffer work-ref into `__retbuf` (an owned copy)
/// instead of renaming it — so the subject, the scratch buffer, and `__retbuf` stay three distinct
/// stores (no collapse). Opt OUT with `LOFT_NO_A1B` (restores the old collapse — the UAF). One
/// cached env read. See `classify_ret_promotion`, borrow-return/DESIGN.md § A1b.
#[must_use]
pub fn a1b_materialise_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_A1B").is_none())
}

/// The @PLN90 phase B last-use MOVE-elision REWRITE — **DEFAULT ON** (B1.5 flip). Build a
/// dead-after owned source directly into its destination field/element instead of copy-then-free,
/// for every proven-safe shape (Record `v[i]=e`/`o.f=src`; Construct field-append, fresh
/// construction, and `a.field=base` replacement; flat + nested). Opt OUT with `LOFT_NO_MOVE_ELIDE`
/// (restores the copy — the always-correct fallback). One cached env read.
/// See `scopes::move_elide` / `use_analysis::move_plans`, phase-b-design.md.
#[must_use]
pub fn move_elide_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_MOVE_ELIDE").is_none())
}

/// `LOFT_MOVE_ELIDE=1` — the MOVE-PLAN detection DUMP (a diagnostic). Opt-IN and independent of the
/// now-default-on rewrite, so the elision does not spew `MOVE-PLAN …` lines on every run.
#[must_use]
pub fn move_elide_dump_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_MOVE_ELIDE").is_some())
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

/// @PLN102 null-flow — the general null-flow laws (`doc/claude/formal/types.md` § Null-flow):
/// `(N-Store)` warn-unless-narrow (a nullable into a full-width non-null slot WARNS + still runs,
/// the slot holds null; a NARROW `u8`…`u32` target keeps the hard ERROR — its width has no bit
/// pattern for null), `(N-Prop)` null propagates through arithmetic, `(N-Domain)` float `/`/`sqrt`/
/// `ln`/… type `τ?`, `(N-Cast)` a `text as τ` parse is an assertion (bare → error; use `as τ?`).
/// **FLIPPED DEFAULT-ON (2026-07-11, @PLN102 the null-flow cutover):** this is now the DEFAULT.
/// `LOFT_NO_NULLFLOW` opts out (the escape hatch while any last consumer settles); the old opt-in
/// `LOFT_NULLFLOW` is now a redundant no-op. One cached env read. Mirrors `join_own_enabled`. See
/// `doc/claude/plans/102-stability-contract/nullflow-flip-plan.md`.
#[must_use]
pub fn nullflow_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_NULLFLOW").is_none())
}

/// `LOFT_NO_STEER=1` (@PLN102 arc C — the recommended-idiom steer channel) — DEFAULT ON, opt OUT.
/// When on, a call FROM OWNED source (the entry project) to a `#superseded "Y"` symbol emits a
/// `Level::Warning` steering the author toward `Y` (the old form keeps working — a never-break
/// signpost, never a removal). Inert regardless until a symbol is actually marked `#superseded`,
/// so default-on is safe from day one. One cached env read; mirrors `nullflow_enabled`. See
/// `doc/claude/plans/102-stability-contract/recommended-idiom-channel.md`.
#[must_use]
pub fn steer_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_STEER").is_none())
}

/// `LOFT_NO_CALLARG_NSTORE=1` (@PLN102 gate-2 residual — the call-arg N-Store hole) — DEFAULT ON,
/// opt OUT. The `(N-Store)` teeth previously sat only on the LOCAL-slot / field / return / index
/// store sites, so a nullable `τ?` (or bare `null`) passed into a non-null PARAMETER slipped
/// through `convert` (which leniently peels the `Optional`) — `takes(x)` with `x: integer?`
/// silently bound `null` into `n: integer`. This applies the same `n_store_violation` check at the
/// param-binding chokepoint (`process_call_args`), with the identical Phase-1 warn/error split (a
/// non-narrow scalar/heap param WARNS and the null binds; a narrow width hard-errors). The escape
/// hatch exists because it is a compile-time tightening of a previously-accepted (unsound) program;
/// it must land before the 1.0 freeze (rejecting later would break compat).
#[must_use]
pub fn callarg_nstore_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_CALLARG_NSTORE").is_none())
}

/// `LOFT_NO_QQ_NULL=1` (@PLN102 gate-2 residual — the `?? null` typing soundness fix) — DEFAULT
/// ON, opt OUT. `a ?? b` discharges null only as far as the FALLBACK `b` can: when `b` is itself
/// nullable (a bare `null` literal, or a `τ?`-typed expression) the coalesce can still yield null,
/// so its RESULT type stays `τ?` instead of peeling to the non-null base. Without this,
/// `y: integer = x ?? null` (and `?? <nullableVar>`) was accepted and a non-null slot held the null
/// sentinel — the exact "null in a non-null slot" incoherence the null-model gate exists to remove.
/// The escape hatch exists because the tightening is a compile-time reject of a previously-accepted
/// (unsound) program; it must land before the 1.0 freeze (rejecting later would break compat).
#[must_use]
pub fn qq_null_typing_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_QQ_NULL").is_none())
}

/// `LOFT_NO_MATH_DOMAIN=1` (@PLN102 case B — soften-nullflow-discharge.md) — DEFAULT ON, opt
/// OUT. Extends the Phase-3.5 constant-in-domain elision from constant args to provably
/// in-domain EXPRESSIONS via a sign lattice (`sqrt(a*a + b*b)`, `sqrt(max(x, 0.01))`,
/// `pow(abs(x), y)` type non-null instead of `τ?`), so no `??` is forced on a provably-safe
/// fault op. Flipped default-on (B5) after the whole-corpus measure confirmed zero runtime-null
/// leaks and the redundant-lint grandfather (`call_declares_nullable`) prevents a now-non-null
/// `sqrt(sum) ?? d` from newly warning under `LOFT_DENY_WARNINGS`.
pub fn math_domain_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_MATH_DOMAIN").is_none())
}

/// The cognitive-complexity score at which a function earns a split nudge.
///
/// Just under p98 of real loft (47), so it speaks for roughly the top 3%. Deliberately a
/// round number rather than a fitted one: the exact cut is a judgement, and a suspiciously
/// precise threshold invites tuning it until the corpus goes quiet, which is how a lint stops
/// meaning anything.
pub const COMPLEXITY_ADVICE_AT: u32 = 40;

/// Trailing boolean parameters with no default, at which a function earns a
/// use-defaults nudge.
///
/// Two, not one: a single trailing flag is idiomatic and common (1.0% of real loft), while
/// two or more is a steering CLUSTER — the shape defaults exist for.  The pattern is
/// naturally rare, which is what keeps this quiet: 96.9% of functions have none at all, and
/// `>=2` covers 2.1%.  A nudge that fires on a common shape gets suppressed, and then the
/// thing it was advertising never gets adopted.
pub const BOOL_FLAG_ADVICE_AT: u32 = 2;

/// `LOFT_NO_DEFAULT_HINT=1` opts OUT of the default-parameter ADVICE — default ON.
///
/// Advertises a genuinely under-used feature rather than reporting a fault: `fn f(a: T, loud:
/// boolean = false)` lets callers say what they mean and omit the rest.  Trailing booleans
/// are where it pays most, because a call site reading `f(x, true, false, true)` carries no
/// information at the point a reader needs it.
///
/// Worth advertising because adopting it is FREE under the compatibility promise: giving an
/// existing parameter a default is purely additive — every existing call passes it explicitly
/// and keeps working, while new calls may leave it out.  Nothing to migrate, so the only
/// thing standing between the feature and its users is knowing it is there.
pub fn default_params_lint_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_DEFAULT_HINT").is_none())
}

/// The count of REQUIRED parameters at which a function earns a bundle-them nudge.
///
/// Separate from [`COMPLEXITY_ADVICE_AT`] on purpose, because the two measure different
/// burdens with different fixes: parameters are what a CALLER carries and the fix is a
/// struct; nesting is what a READER carries and the fix is an extracted function.
///
/// Folding parameters into the complexity score was measured and rejected — it misses the
/// case that motivates the check.  `th_subdiv` takes 12 required parameters with a
/// complexity of 2: trivial to read, hard to call.  At +1 per parameter it scores 14 and
/// stays silent, so the one function most needing the nudge would never get it.  It would
/// also make the complexity message untrue, since most of such a score would not be control
/// flow.
///
/// 8 is read off real loft: 86% of functions take 4 or fewer, `>=6` is 8.5%, `>=8` is 2.1%
/// — about the share the complexity nudge speaks for.
pub const PARAM_ADVICE_AT: u32 = 8;

/// `LOFT_NO_PARAM_COUNT=1` opts OUT of the required-parameter ADVICE — default ON.
///
/// Counts only what a caller must supply: parameters with a DEFAULT are excluded (they cost
/// the caller nothing), as are compiler-injected hidden ones (`__retbuf`, work buffers) which
/// no author wrote.  The default exemption currently exempts nothing — no function in 5,915
/// of real loft uses a default value — but it is the rule that makes the count mean "what a
/// caller must know", so it is applied rather than assumed away.
pub fn param_count_lint_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_PARAM_COUNT").is_none())
}

/// `LOFT_NO_COMPLEXITY=1` opts OUT of the function-complexity ADVICE — default ON.
///
/// Advice, never a warning: a complex function is correct, so ignoring this cannot produce a
/// wrong result.  It exists because a whole algorithm wired into one function is the thing
/// nobody chooses and everybody inherits.
///
/// Cognitive, not cyclomatic — a construct costs `1 + nesting`, so DEPTH is what is
/// expensive.  Eight sequential `if`s cost 8; three nested cost 6; a flat `match` costs 1
/// however many arms it has.  "Many branches" and "hard to follow" are different properties,
/// and only the second is worth a nudge.
///
/// The boundary is calibrated, not picked: over 5,972 functions of real loft the distribution
/// runs p50 1, p90 15, p95 27, p98 47.  [`COMPLEXITY_ADVICE_AT`] sits just under p98 so it
/// speaks for ~3% — few enough that each one is worth reading.
pub fn complexity_lint_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_COMPLEXITY").is_none())
}

/// `LOFT_LINT_STRICT_INDEX=1` (@PLN102 case D audit) — opt-in, DEFAULT OFF. The index-trust
/// model types `v[i]` non-null for a for-loop iter var (like a constant index), trusting the
/// loop bounds the vector. That trust is unchecked, so `for i in 0..len(v) { w[i] }` (or a
/// mid-loop resize) types non-null yet reads C80-null on overrun. When set, this warns where a
/// loop-var index is bounded by `len(<one vector>)` but indexes a DIFFERENT vector — the
/// mismatched-vector index that is the silent-null hazard. Advisory only: the type is unchanged
/// (tightening the trust to a proof would break the ubiquitous `for i in 0..n { v[i] }` idiom).
pub fn strict_index_lint_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_LINT_STRICT_INDEX").is_some())
}

/// `LOFT_NO_STRICT_INDEX_TEXT` (@PLN110 3a) — the TEXT strict-index units lint, DEFAULT ON
/// (opt-out). After the @PLN110 flip `len(text)` is a CHARACTER count while `text[i]` is
/// byte-indexed, so `for i in 0..len(s) { s[i] }` under-runs / misreads multi-byte text (it walks
/// char-count byte positions). Warns on exactly that shape — a loop var bounded by `len(s)` used to
/// index that same text `s`. Advisory only (the type is unchanged): iterate with `for c in s`, or
/// use `0..size(s)` for a genuine byte walk. Unlike the vector lint this is default-on, because for
/// text the units are ALWAYS mismatched (char count vs byte offset), not just on a mismatched
/// collection.
pub fn text_index_units_lint_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_STRICT_INDEX_TEXT").is_none())
}

/// `LOFT_NO_CONST_EFFECT` — the re-evaluated-constant lint, DEFAULT ON (opt-out).
///
/// A file-scope `NAME = expr;` is an INLINED expression, not a once-computed value: the
/// expression is substituted at every reference, so an initialiser that costs something
/// pays that cost per use.  For a literal (`PI = 3.14;`) this is invisible and free;
/// for `FNT = load_bundled();` it is not.  A consumer wrote exactly that, referenced it
/// once per word per frame, and the browser ran out of memory — the font was re-parsed
/// hundreds of times per reflow.  Nothing said the word "constant" did not mean
/// "computed once".
///
/// Warns when such an initialiser CALLS something that can cost: a user-defined
/// function (any source but the stdlib), or a stdlib function annotated
/// `#impure(category)`.  A pure stdlib call or plain arithmetic stays silent, so
/// `MAX = 10 * 3;` and `PI = 3.14;` never warn.  Advisory — the semantics are
/// unchanged; the fix is a function plus an explicit cache.
#[must_use]
pub fn const_effect_lint_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_CONST_EFFECT").is_none())
}

/// `LOFT_PLN25_DN1=1` (@PLN25 Phase-2 CONTRACT, IN PROGRESS) — the DEFAULT FLIP: a plain scalar
/// (`integer`, `text`, `bool`, …) is NON-NULL by default; `τ?` is the only nullable form. Turns
/// `IntegerSpec.not_null` default `false → true` (and the analog for other scalars rides
/// `Type::Optional`), so a bare `null` returned/stored into a plain scalar is rejected (beyond the
/// Optional `(N-Store)` teeth, which only catch `Optional → non-null`). Opt-IN while the `.loft`
/// sweep migrates the misses to `?`; OFF keeps the nullable-by-default behaviour. Implies
/// `LOFT_PLN25_DN3` (the `(N-Store)` teeth) and `LOFT_PLN25_OPT` (the `Optional` marker). See
/// plans/25-nullable-sequences/dn1-flip-blast-radius.md and RESUME.md § Step 3.
/// @PLN85 D-own-1 — THE FLIP (2026-07-02): the `deps`-driven ownership fixes
/// (`local_source` displaced-owned, `elem_accumulate` source-free,
/// `match_return` owned-copy synthesis, the `OpBindOrCopy` join delivery) are
/// now the DEFAULT.  The old opt-in `LOFT_JOIN_OWN` is a redundant no-op;
/// `LOFT_NO_JOIN_OWN` opts out (the escape hatch while consumers settle).
/// Evidence at the flip: the 54-cell over-free map 6/54 opt-out -> 0/54
/// default; full suite green both ways (tests/use_analysis.rs pins both legs).
#[must_use]
pub fn join_own_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_JOIN_OWN").is_none())
}

/// #497 — the reassignment-path adopt-vs-copy fix: a Reference local
/// REASSIGNED from a `!return_adopts_fresh_store()` call deep-copies
/// (parity with the first-Set path) instead of adopting a possibly
/// borrowed store the owned pre-Set free then whole-store-frees.
/// `LOFT_NO_REASSIGN_COPY` preserves the raw path so the fuzz gate's
/// crash-channel positive control stays non-vacuous — the same
/// preservation pattern as `LOFT_NO_JOIN_OWN` above.
#[must_use]
pub fn reassign_copy_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NO_REASSIGN_COPY").is_none())
}

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

/// @PLN25 F2 — the RANGE RECONCILIATION, FLIPPED DEFAULT-ON (2026-07-02): a plain
/// (non-`Optional`) narrow integer is NON-null under DN1, so its usable range is the FULL width
/// (no reserved null sentinel); only an `Optional`-wrapped narrow (`u8?`) reserves the top value
/// as the sentinel. This completes DN1 for the *range* dimension (the `IntegerSpec.not_null`
/// default DN1 had left at `false` for narrow scalars), making `not null` redundant — the
/// prerequisite for retiring it. Rides `pln25_dn1_enabled` like the other @PLN25 gates
/// (`LOFT_PLN25_OFF` opts the whole model out; the interim opt-in `LOFT_PLN25_F2` is now a no-op).
/// Flip prerequisites all landed: the struct-format read-width fix (`ac432914`), the
/// cross-statement `expr_not_null` leak (`edee7ec8`), and the `not null` hint retirement (below).
#[must_use]
pub fn pln25_f2_enabled() -> bool {
    pln25_dn1_enabled()
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

/// `LOFT_NO_SLOT_REUSE=1` — @PLN118 arc D: never reclaim a freed store slot (always
/// grow). Diagnostic stopgap to test whether a corruption is slot-reuse-while-referenced.
///
/// Implied by [`strict_stores`], which needs the no-reuse guarantee to make its
/// dead-store check exact.
#[must_use]
pub fn no_slot_reuse() -> bool {
    static NR: OnceLock<bool> = OnceLock::new();
    *NR.get_or_init(|| std::env::var_os("LOFT_NO_SLOT_REUSE").is_some() || strict_stores())
}

/// `LOFT_TRACE_DB=1` — print every record allocation (`OpDatabase`) with the type it
/// allocates and the `DbRef` the target slot held on entry.  Reach for it when a store
/// slot looks like it has two owners: the entry `DbRef` is what says whether an
/// allocation ADOPTED a slot some other variable still names.
///
/// Read once because both backends call it on every struct-typed local's
/// initialisation.  Answered in ONE place so the two backends cannot disagree about
/// what the switch means — the interpreter had it and the native runtime did not,
/// which made the trace silent for exactly the calls that go through a package's
/// shared library (loft#810).
#[must_use]
pub fn trace_db() -> bool {
    static TD: OnceLock<bool> = OnceLock::new();
    *TD.get_or_init(|| std::env::var_os("LOFT_TRACE_DB").is_some())
}

/// `LOFT_STRICT_STORES=1` (@PLN130 F8) — strict store lifetime, for PROBES.
///
/// Turns the two store-lifetime faults from silent-or-advisory into hard errors:
///
/// * **Use after free** — a freed store stays dead for the rest of the run, and any
///   access through a `DbRef` naming it is reported at the access.
/// * **Never freed** — a store still live at exit is reported, by type.
///
/// The exactness comes from implying [`no_slot_reuse`]: a slot that is never recycled
/// cannot be legitimately re-occupied, so `free == true` at an access is unambiguous —
/// no generation stamp, no `DbRef` widening, and no false positives to explain away.
///
/// **Opt-in, and deliberately not for the normal suite.** Never reusing a slot means a
/// long run walks off the end of the `u16` store space, so this is written for small
/// probe programs that exercise one lifetime question each. Under it a probe that would
/// otherwise print a plausible wrong number fails loudly instead.
#[must_use]
pub fn strict_stores() -> bool {
    static SS: OnceLock<bool> = OnceLock::new();
    *SS.get_or_init(|| std::env::var_os("LOFT_STRICT_STORES").is_some())
}

/// Violations recorded by [`strict_stores`] mode, so one run surfaces every site rather
/// than stopping at the first — the same reasoning as `LOFT_DEV_SOFT_HALT`.
static STRICT_VIOLATIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

thread_local! {
    /// Where each store was FREED: slot -> (pc, the variable name `free_named` was given).
    /// A side table rather than two more `Store` fields, so the bookkeeping costs nothing
    /// when strict mode is off.
    static STRICT_FREE_SITE: std::cell::RefCell<std::collections::HashMap<u16, (u32, String)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Record where store `slot` was freed. Call only when [`strict_stores`] is on.
pub fn strict_note_free(slot: u16, pc: u32, name: &str) {
    STRICT_FREE_SITE.with(|m| {
        m.borrow_mut().insert(slot, (pc, name.to_string()));
    });
}

/// The recorded free site for `slot`, if any.
#[must_use]
pub fn strict_free_site(slot: u16) -> Option<(u32, String)> {
    STRICT_FREE_SITE.with(|m| m.borrow().get(&slot).cloned())
}

/// Report an access to a store that was freed. Call only when [`strict_stores`] is on.
///
/// Deliberately COMPILER-DEVELOPER detail, not a user diagnostic: it names the store slot
/// and the three pcs that bound the store's life, because the question this answers is
/// "which emitter produced a reference that outlived its store", and that cannot be
/// answered from the loft source alone. Resolve a pc to a line with `LOFT_LOG=static`.
#[expect(
    clippy::too_many_arguments,
    reason = "one report line, one argument each"
)]
pub fn strict_store_violation(
    store_nr: u16,
    rec: u32,
    pos: u32,
    what: &str,
    type_name: &str,
    created_at: u32,
    last_op_at: u32,
    now_pc: u32,
) {
    let n = STRICT_VIOLATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Cap the output: a stale ref inside a loop reports every iteration, and the first
    // few already name the site.
    if n < 20 {
        let site = strict_free_site(store_nr);
        let who = site.as_ref().map_or("<not recorded>", |(_, nm)| {
            if nm.is_empty() { "<anon>" } else { nm.as_str() }
        });
        // The pcs are BYTECODE positions, so they exist on `--interpret` and are all zero
        // under `--native` (generated Rust has no pc). Print them only when they say
        // something — a line of `pc=0, pc=0, pc=0` reads as data and is not.
        let freed_pc = site.as_ref().map_or(0, |(pc, _)| *pc);
        let spans = created_at | last_op_at | now_pc | freed_pc;
        let where_ = if spans == 0 {
            "  (no bytecode positions — native run; re-run with --interpret for pcs)".to_string()
        } else {
            format!(
                "  created at pc={created_at}, last legitimate op at pc={last_op_at}, \
                 freed at pc={freed_pc}, {what} now at pc={now_pc}"
            )
        };
        eprintln!(
            "[strict-store] USE AFTER FREE ({what}) store #{store_nr} type={type_name} \
             rec={rec} pos={pos}\n  killed by the free of `{who}`\n{where_}"
        );
    } else if n == 20 {
        eprintln!("[strict-store] ... further use-after-free reports suppressed");
    }
}

/// Count `n` never-freed stores as violations, so the exit status covers both halves.
pub fn strict_store_leaks(n: usize) {
    STRICT_VIOLATIONS.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
}

/// How many strict-mode violations were recorded this run.
#[must_use]
pub fn strict_store_violations() -> usize {
    STRICT_VIOLATIONS.load(std::sync::atomic::Ordering::Relaxed)
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
    /// Shadow of the operand stack: keyed by eval-stack byte offset, holding the STORE a
    /// DbRef named at PUSH together with that slot's gen. At POP, a mismatch against the
    /// slot's current gen = reused-since-push.
    ///
    /// The store number is carried, not just the gen, because an offset alone cannot say
    /// WHICH store a stamp is about. `put_stack` is the only writer that keeps the shadow
    /// in step, and it is not the only writer of the eval stack — a raw `copy_block` slide
    /// can replace the value under a stamp. A gen-only stamp was then read as a claim about
    /// whatever DbRef happened to land there, and reported a store freed since some
    /// UNRELATED value occupied the offset. Matching the store makes such a leftover stamp
    /// inert instead of a false positive, for every bypassing writer rather than the ones
    /// that had been found.
    static STACK_SHADOW: std::cell::RefCell<std::collections::HashMap<u32, (u16, u32)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

thread_local! {
    /// @PLN118 arc B refinement — depth of record deep-copies (`copy_record`/`finish_record`)
    /// on the stack. A `LOFT_UAF_GEN` stale read while > 0 is a record COPY reading a stale
    /// sub-reference (the copy is incomplete; the correctly-placed source free is NOT the bug)
    /// — a distinct root from a plain deref of a genuinely prematurely-freed store.
    static COPY_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Enter a record deep-copy (bump the depth). Gate at the call site on `uaf_gen_enabled`.
pub fn uaf_copy_enter() {
    COPY_DEPTH.with(|d| d.set(d.get() + 1));
}

/// Leave a record deep-copy (drop the depth). Saturating so a mismatched pair can't underflow.
pub fn uaf_copy_exit() {
    COPY_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
}

/// True when a record deep-copy is currently executing (a stale read here is a copy-incomplete,
/// not a premature-free).
#[must_use]
pub fn uaf_in_copy() -> bool {
    COPY_DEPTH.with(|d| d.get() > 0)
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

/// Stamp eval-stack offset `off` with the store a pushed DbRef names and that slot's
/// generation.
pub fn uaf_stamp_shadow(off: u32, store_nr: u16, generation: u32) {
    STACK_SHADOW.with(|s| {
        s.borrow_mut().insert(off, (store_nr, generation));
    });
}

/// The gen stamped at `off` — but only when the stamp is ABOUT `store_nr`. A stamp naming
/// a different store is a leftover from a value that has since been overwritten by a
/// writer that bypasses `put_stack`; it says nothing about the ref being popped now, so it
/// reads as absent rather than as evidence.
#[must_use]
pub fn uaf_shadow_gen(off: u32, store_nr: u16) -> Option<u32> {
    STACK_SHADOW.with(|s| {
        s.borrow()
            .get(&off)
            .and_then(|&(st, stamped)| (st == store_nr).then_some(stamped))
    })
}

/// Move a stamp from one eval-stack offset to another, for a writer that relocates a DbRef
/// without going through `put_stack` (the `copy_result` return slide). Clears `to` when
/// `from` holds no stamp, so the destination never keeps its previous occupant's.
pub fn uaf_move_shadow(from: u32, to: u32) {
    STACK_SHADOW.with(|s| {
        let mut m = s.borrow_mut();
        match m.remove(&from) {
            Some(v) => {
                m.insert(to, v);
            }
            None => {
                m.remove(&to);
            }
        }
    });
}

/// `LOFT_UAF_GEN_INJECT=1` — the positive control for detector (c). Bumps a store's
/// generation immediately after each DbRef push stamps it, so every ref on the eval stack
/// is stale while live exactly as a premature free would leave it, and any checked pop
/// must report.
///
/// Without this, a silent `LOFT_UAF_GEN` run is not evidence: a detector that can no
/// longer fire at all is indistinguishable from a clean corpus. Aging a single ref would
/// not do — whether that one is ever popped through the checked path depends on the
/// program, so a silent run would stay ambiguous. Never set in production.
#[must_use]
pub fn uaf_gen_inject_enabled() -> bool {
    static INJ: OnceLock<bool> = OnceLock::new();
    *INJ.get_or_init(|| std::env::var_os("LOFT_UAF_GEN_INJECT").is_some())
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
    /// @PLN118 arc B — the free site keyed by `(slot, post-free generation)`. For a slot
    /// freed-and-reused many times, `FREED_AT` (last free only) names the LAST occupant's
    /// free, not the free that made a specific stale ref stale. Keyed by generation, the
    /// gen-detector can name the CAUSAL free (the one at the ref's stamped gen + 1).
    static FREED_AT_GEN: std::cell::RefCell<std::collections::HashMap<(u16, u32), (u32, u32, u16)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Record (under `LOFT_UAF`/`LOFT_UAF_GEN`) that store `slot` was freed while executing
/// `pc` in function `d_nr`, by the op whose opcode is `op` (names the freeing op category:
/// a scope-exit `OpFreeRef`, an `OpFreeRefIfDistinct`, or a `copy_record` free-bit). Also
/// stamps it against the slot's now-current (post-free) generation for causal attribution.
pub fn uaf_record_free(slot: u16, pc: u32, d_nr: u32, op: u16) {
    FREED_AT.with(|m| {
        m.borrow_mut().insert(slot, (pc, d_nr, op));
    });
    if uaf_gen_enabled() {
        let slot_gen = uaf_slot_gen(slot);
        FREED_AT_GEN.with(|m| {
            m.borrow_mut().insert((slot, slot_gen), (pc, d_nr, op));
        });
    }
}

/// The `(code_pos, d_nr, op_code)` of `slot`'s most-recent recorded free, if any.
#[must_use]
pub fn uaf_freed_pc(slot: u16) -> Option<(u32, u32, u16)> {
    FREED_AT.with(|m| m.borrow().get(&slot).copied())
}

/// The free that took `slot` to generation `gen` (the CAUSAL free for a DbRef stamped at
/// `gen - 1`), if recorded. Falls back to `uaf_freed_pc` at the call site when absent.
#[must_use]
pub fn uaf_freed_pc_at_gen(slot: u16, want_gen: u32) -> Option<(u32, u32, u16)> {
    FREED_AT_GEN.with(|m| m.borrow().get(&(slot, want_gen)).copied())
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
        (Content::Long(v), 9) => v.cmp(&i64::from(s.get_short(
            record.rec,
            record.pos + pos,
            key.start,
        ))),
        (Content::Long(v), 10) => v.cmp(&i64::from(s.get_byte(
            record.rec,
            record.pos + pos,
            key.start,
        ))),
        (Content::Long(v), 11) => v.cmp(&i64::from(s.get_short_full(
            record.rec,
            record.pos + pos,
            key.start,
        ))),
        (Content::Long(v), _) => v.cmp(&i64::from(s.get_byte(
            record.rec,
            record.pos + pos,
            key.start,
        ))),
        _ => panic!("Undefined compare {k:?} vs {}", key.type_nr),
    };
    if key.type_nr < 0 { c.reverse() } else { c }
}

/// A one-field key whose EQUALITY test is a single direct read, resolved once so a
/// probe loop does not re-derive it. Carries the field's byte offset within the record
/// and the value to match.
///
/// [`compare_key`] answers a full `Ordering` and re-runs its `(Content, type_nr)` match
/// for every record probed. A hash probe only ever asks *equal or not*, and it asks it
/// about the same key each time, so the match belongs outside the loop: measured on 1M
/// `integer`-keyed lookups it is ~10 ns of a ~33 ns cache-resident lookup (@PLN135
/// arc B).
///
/// Only widths whose test is exactly the arm above are listed — the shifted ones
/// (`Short`, `Byte`, and the catch-all) are deliberately absent, so [`fast_key`]
/// answers `None` for them and the probe falls back to `compare_key` unchanged.
pub enum FastKey<'a> {
    /// `integer` (type_nr 1), the 8-byte read with the in-band null sentinel.
    Int(u32, i64),
    /// `long` (2), the raw 8-byte read.
    Long(u32, i64),
    /// A `size(4)` integer (8), sign-extended from the raw 4 bytes.
    I32(u32, i64),
    /// A `Parts::ShortRaw` 2-byte integer (11), decoded `read + start`.
    ShortRaw(u32, i32, i64),
    /// `text` (6): the offset holds a 4-byte string handle.
    Str(u32, &'a str),
}

/// Resolve a one-field key to its [`FastKey`], or `None` when the probe must keep
/// using [`key_compare`] — a compound key, a width not listed there, or a `Content`
/// that does not match the descriptor.
#[must_use]
pub fn fast_key<'a>(keys: &[Key], key: &'a [Content]) -> Option<FastKey<'a>> {
    let ([k], [c]) = (keys, key) else { return None };
    let pos = u32::from(k.position);
    match (c, k.type_nr.abs()) {
        (Content::Long(v), 1) => Some(FastKey::Int(pos, *v)),
        (Content::Long(v), 2) => Some(FastKey::Long(pos, *v)),
        (Content::Long(v), 8) => Some(FastKey::I32(pos, *v)),
        (Content::Long(v), 11) => Some(FastKey::ShortRaw(pos, k.start, *v)),
        (Content::Str(v), 6) => Some(FastKey::Str(pos, v.str())),
        _ => None,
    }
}

impl FastKey<'_> {
    /// Does the record at `rec` (whose fields start at byte 8) carry this key?
    ///
    /// Each arm is the equality half of the identically-numbered arm of
    /// [`compare_key`]; keep them together when either changes.
    #[must_use]
    pub fn matches(&self, s: &Store, rec: u32) -> bool {
        match self {
            FastKey::Int(pos, v) => s.get_int(rec, 8 + pos) == *v,
            FastKey::Long(pos, v) => s.get_long(rec, 8 + pos) == *v,
            FastKey::I32(pos, v) => i64::from(s.get_i32_raw(rec, 8 + pos)) == *v,
            FastKey::ShortRaw(pos, start, v) => {
                i64::from(s.get_short_full(rec, 8 + pos, *start)) == *v
            }
            FastKey::Str(pos, v) => s.get_str(s.get_u32_raw(rec, 8 + pos)) == *v,
        }
    }
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
        9 => s
            .get_short(r1.rec, p1, key.start)
            .cmp(&s.get_short(r2.rec, p2, key.start)),
        10 => s
            .get_byte(r1.rec, p1, key.start)
            .cmp(&s.get_byte(r2.rec, p2, key.start)),
        11 => s
            .get_short_full(r1.rec, p1, key.start)
            .cmp(&s.get_short_full(r2.rec, p2, key.start)),
        _ => s
            .get_byte(r1.rec, p1, key.start)
            .cmp(&s.get_byte(r2.rec, p2, key.start)),
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
            // A `single` / `float` key.  Without these two arms the width fell to the
            // catch-all below, which reads ONE BYTE and calls it a `Content::Long` — so
            // every key whose low byte is zero (0.5, 1.5, 2.0, … — almost every float a
            // program writes) became the SAME key.  `compare_key` then had no
            // `(Content::Long, 3|4)` arm either and compared that byte, answering Equal
            // for every pair: a `sorted<T[k]>` collapsed to its LAST insert, and a
            // `hash<T[k]>` lookup hashed a byte where `hash_ref` hashes nothing, so it
            // probed the wrong bucket and missed records that were present.  The
            // comparator arms for both widths already existed; only the reader was short.
            3 => {
                let v = store(record, stores).get_single(record.rec, p);
                result.push(Content::Single(v));
            }
            4 => {
                let v = store(record, stores).get_float(record.rec, p);
                result.push(Content::Float(v));
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
                let v = store(record, stores).get_short(record.rec, p, k.start);
                result.push(Content::Long(i64::from(v)));
            }
            10 => {
                let v = store(record, stores).get_byte(record.rec, p, k.start);
                result.push(Content::Long(i64::from(v)));
            }
            11 => {
                let v = store(record, stores).get_short_full(record.rec, p, k.start);
                result.push(Content::Long(i64::from(v)));
            }
            _ => {
                let v = store(record, stores).get_byte(record.rec, p, k.start);
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
pub fn hash(rec: &DbRef, stores: &[Store], keys: &[Key], seed: u64) -> u64 {
    let mut hasher = seeded_hasher(seed);
    for key in keys {
        let pos = rec.pos + u32::from(key.position);
        hash_ref(rec, stores, key, pos, &mut hasher);
    }
    hasher.finish()
}

#[must_use]
pub fn key_hash(key: &[Content], seed: u64) -> u64 {
    let mut hasher = seeded_hasher(seed);
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
        9 => i64::from(s.get_short(r.rec, p, key.start)).hash(hasher),
        10 => i64::from(s.get_byte(r.rec, p, key.start)).hash(hasher),
        // `Parts::ShortRaw` stores `(val - min) as u16` with no null sentinel, so it
        // decodes as `read + min` — the `get_short_full` its own writer is paired with
        // (loft#812).
        11 => i64::from(s.get_short_full(r.rec, p, key.start)).hash(hasher),
        _ => i64::from(s.get_byte(r.rec, p, key.start)).hash(hasher),
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
