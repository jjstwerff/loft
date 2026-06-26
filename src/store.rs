// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

// @PLAN38 phase 01: this module is now `pub mod store` (was `mod store`).
// Promoting the module surfaced clippy::pedantic lints on existing
// public methods (panic/error doc sections, must_use, is_empty, etc.)
// that the codebase has always tolerated as internal — fixing each one
// is a project-wide doc sweep, out of scope here.  Allow them at the
// module level so this PR stays focused on the durable-store API.
#![allow(
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::collapsible_if,
    clippy::not_unsafe_ptr_arg_deref,
    clippy::len_without_is_empty,
    clippy::return_self_not_must_use,
    clippy::unnecessary_debug_formatting
)]

//! Word-addressed heap store with bump allocation and free-block reuse.
//!
//! Each [`Store`] is a contiguous buffer of 8-byte words.  Records are
//! allocated via [`claim`](Store::claim) and freed via
//! [`delete`](Store::delete).
//!
//! ## Memory layout
//!
//! Every record starts with a **signed size header** (one `i32` word):
//! - **Positive** → claimed record; magnitude = size in words (incl. header).
//! - **Negative** → free block; magnitude = size in words.
//!
//! Free blocks are tracked in a `BTreeMap` for gap-based allocation.
//! Adjacent free blocks are merged on `delete` to reduce fragmentation.
//!
//! Record 0 is the store header; record 1 (`PRIMARY`) is the main record
//! describing vectors and indexes with sub-records.  A store may optionally
//! be backed by a memory-mapped file (`mmap` feature).

#[cfg(feature = "mmap")]
use mmap_storage::file::Storage as MmapStorage;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt::{Debug, Formatter};

#[allow(dead_code)]
static A: System = System;
const SIGNATURE: u32 = 0x53_74_6f_31;
pub const PRIMARY: u32 = 1;
/// Maximum store size in words; offsets are stored as `i32` so this is the limit.
pub const MAX_STORE_WORDS: u32 = i32::MAX as u32;

/// Minimum free-block size (words) to register in the LLRB free-space tree.
/// A node needs 4 (left) + 4 (right) + 1 (color) bytes after the 4-byte header = 13 bytes.
/// Two 8-byte words (16 bytes) comfortably hold these fields.
const MIN_FREE_TREE: i32 = 2;
/// Byte offset of LLRB left-child field within a free block.
const FL_LEFT: u32 = 4;
/// Byte offset of LLRB right-child field within a free block.
const FL_RIGHT: u32 = 8;
/// Byte offset of LLRB color flag within a free block (1 = red, 0 = black).
const FL_COLOR: u32 = 12;

/// Internal space-utilisation snapshot of a single store — actual
/// claimed data vs free space, and how fragmented the free space is.
/// Produced by [`Store::usage`] by walking the block chain.
#[derive(Default, Clone, Copy)]
pub struct StoreUsage {
    /// Total store size in 8-byte words (the allocated buffer).
    pub capacity_words: u32,
    /// Sum of CLAIMED record sizes (the actual live data).
    pub claimed_words: u32,
    /// Number of claimed records.
    pub claimed_count: u32,
    /// Sum of FREE block sizes (reclaimable / fragmentation).
    pub free_words: u32,
    /// Number of distinct free blocks (the "free structure" elements).
    pub free_count: u32,
    /// Largest single free block (words) — a low value with high
    /// `free_words` means the free space is fragmented.
    pub largest_free_words: u32,
    /// Number of ADJACENT free-block pairs found during the walk — two
    /// consecutive free blocks that are physical neighbours and could be
    /// coalesced into one bigger gap.  `delete` is supposed to merge
    /// adjacent free blocks, so a non-zero count flags a missed merge
    /// (coalescing gap / fragmentation that better detection could
    /// reclaim).
    ///
    /// NOTE: this is a FIRST-CUT detector — it only catches free blocks
    /// that are immediately consecutive in the linear block walk.  Finer
    /// coalescing analysis (merge opportunities the LLRB free tree could
    /// realise, near-but-not-adjacent gaps, etc.) is expected to develop
    /// here; the metric and its plumbing are deliberately kept simple so
    /// they can be extended.
    pub mergeable_free_pairs: u32,
}

impl StoreUsage {
    /// Fraction of capacity holding live data, 0..100.
    #[must_use]
    pub fn used_pct(&self) -> f64 {
        if self.capacity_words == 0 {
            return 0.0;
        }
        100.0 * f64::from(self.claimed_words) / f64::from(self.capacity_words)
    }
}

/// A structural change recorded on a [`Store`] while @PLN16.J edit-recording is on
/// (see `crate::database::journal`).  A `Store` method does not know its own
/// `store_nr`, so each store buffers its own changes and `Stores` drains them into the
/// unified `Journal`, tagging the store index.  `Insert` keeps only the position and
/// size — the new record's bytes are read at drain (flush); `Free` snapshots `before`
/// at delete time, since a freed block's body is repurposed instantly (probe 5a).
#[derive(Debug)]
pub enum StoreChange {
    /// A record claimed at `pos` (`size` words); replayed via `claim_at` at flush.
    Insert { pos: u32, size: u32 },
    /// A record freed; `before` is its bytes captured *before* the `delete`.
    Free { pos: u32, before: Box<[u8]> },
}

// A low-level heap store: the several flags (free / read_only /
// free_protected / borrowed) are independent state bits on the same
// allocation, not a bundle that should become an enum.
#[allow(clippy::struct_excessive_bools)]
pub struct Store {
    // format 0 = SIGNATURE, 4 = free_space_index, 8 = record_size, 12 = content
    pub ptr: *mut u8,
    claims: HashSet<u32>,
    size: u32,
    #[cfg(feature = "mmap")]
    file: Option<MmapStorage>,
    pub(crate) free: bool,
    /// HARD lock: when `true`, the store is immutable.  All `addr_mut`,
    /// `claim`, and `delete` calls panic.  Set by CONST_STORE init
    /// (`compile.rs`), the JSON null sentinel (`native.rs`), and worker
    /// store borrows (`clone_locked` / `clone_locked_for_worker` /
    /// `borrow_locked_for_light_worker`).  Cleared via `unlock()`.
    pub read_only: bool,
    /// SOFT lock: when `true`, only `delete` is illegal — `addr_mut`
    /// and `claim` are still allowed.  Set by the fn-call deep-copy
    /// bracket (`Stores::lock_store(&r)` from
    /// `n_set_store_lock(r, true)`) to mark a caller's arg as
    /// PROTECTED-FROM-FREE for the duration of the call, so
    /// `OpCopyRecord`'s `0x8000` free-source branch skips the free
    /// when the callee returned a borrowed view of an arg.  Doesn't
    /// block scratch allocations made by the callee (e.g.
    /// `build_hash_sorted_vec` claiming sort scratch in the hash's
    /// own store per C60).  Cleared via `Stores::unlock_store(&r)`
    /// from `n_set_store_lock(r, false)`.  See @P290 for the
    /// rationale; replaced the prior origin-string discriminator.
    pub free_protected: bool,
    /// Root of the LLRB free-space tree (0 = empty).
    /// Populated lazily: `open()` calls `fl_rebuild()`; `new()` starts empty
    /// and the tree fills as blocks are freed.
    free_root: u32,
    /// P6: set by `delete` whenever a free block is produced; cleared by
    /// `coalesce_free`.  `claim` runs the lazy coalescing sweep only when
    /// this is set (something was freed since the last sweep), so an
    /// alloc-only workload never pays for a fruitless O(n) pass.  A single
    /// flag — NOT an index; it does not grow with the free-block count.
    needs_coalesce: bool,
    /// CO1.9/S28: monotonic counter incremented on every `claim`, `resize`, and `delete`.
    /// Saved into `CoroutineFrame` at yield; compared at resume to detect store mutations
    /// that may have invalidated `DbRef` locals held by the generator.  Always compiled in
    /// (was debug-only before CO1.9) so the guard fires in release builds too.
    pub generation: u32,
    /// @PLN16.J — when `Some`, the structural ops (`claim` via `claim_block`, `delete`)
    /// append a [`StoreChange`] here for the debugger's edit journal.  `None` on every
    /// normal run, so the cold alloc paths pay one branch.  Turned on by
    /// `Stores::start_recording`, drained by `Stores::take_journal`.
    recording: Option<Vec<StoreChange>>,
    /// Plan-57 store-identity gate (verification builds only): the allocation-site
    /// id written by `OpStoreTag` and verified by `OpFreeRefTag`.  `0` = untagged
    /// (no verification emitted for this store).  Catches wrong-store / cross-owner
    /// frees that `free_named` otherwise silently no-ops.  Set/checked only when the
    /// gated IR post-pass emits the tagged ops; inert in normal builds.
    pub tag: u32,
    /// When true, this Store borrows another's buffer — `Drop` must NOT dealloc.
    borrowed: bool,
    /// Bytecode position that allocated this store (via OpDatabase).
    /// Used for diagnostics when a store is leaked at program exit.
    pub created_at: u32,
    /// Bytecode position of the last significant operation on this store
    /// (OpCopyRecord, OpFreeRef skip, ref_count change, etc.).
    pub last_op_at: u32,
    /// Plan-57 Phase C: const/global stores are PINNED — `free_named` never
    /// frees them (they live for the whole program).  Replaced the Stores
    /// ref-count (deleted) as the only per-store free gate.
    pub pinned: bool,
    /// Plan-22 02d-vii follow-up — identifier of the call site that
    /// most recently locked this store.  Empty when the store is
    /// unlocked or was locked without an origin (legacy callers).
    /// Surfaced in panic messages from `addr_mut` / `claim` / `delete`
    /// so a "Write to read-only store" failure points directly at the
    /// locker rather than requiring `LOFT_LOG=locks` to re-trace.
    pub lock_origin: String,
    /// P259 — type-id of the loft type whose root record lives at
    /// `(rec=1, pos=8)` of this store.  `u16::MAX` when unknown
    /// (raw stores not allocated through `database_named`).
    /// Set by `Stores::set_known_type` after `database_named`
    /// returns the freshly-claimed store_nr.  Read by
    /// `Stores::free_named` to gate the cascade-free walk on
    /// closure records (type name starts with `__closure_`) —
    /// see commit 4 of the P259 fix.
    pub known_type: u16,
    /// @PLAN38 phase 01 — when `Some(path)`, this store was opened
    /// via `Store::open_durable` and on clean drop must (1) flush
    /// the mmap, (2) compute the payload CRC, and (3) rewrite the
    /// `<path>.dmeta` sidecar atomically.  Cleared (left `None`) on
    /// every non-durable constructor (`new`, `open`, `clone_locked*`,
    /// `borrow_locked_*`, `new_freed_sentinel`).  See
    /// `doc/claude/plans/43-loft-store-durable/`.
    #[cfg_attr(not(feature = "mmap"), allow(dead_code))]
    durable_meta_path: Option<std::path::PathBuf>,
    /// @PLAN38 phase 01 — durability-mode tier on this store.  Tracks
    /// which durability variant the consumer opted into so the Drop
    /// path can apply tier-specific shutdown logic.  `0` for non-durable
    /// stores; `1` for `IntegrityOnly`.  Tiers `2` (`SnapshotEvery`)
    /// and `3` (`WAL`) are reserved for future phases.
    #[cfg_attr(not(feature = "mmap"), allow(dead_code))]
    durable_tier: u16,
}

impl Debug for Store {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("Store[{}]", self.size))
    }
}

impl PartialEq for Store {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        // borrowed stores share another Store's buffer — do not free.
        if self.borrowed {
            return;
        }
        #[cfg(feature = "mmap")]
        if self.file.is_some() {
            // @PLAN38 phase 01 — for durable stores, flush mmap + rewrite
            // sidecar atomically BEFORE the mmap_storage handle's own Drop
            // runs and closes the file.  Failure to flush is logged to
            // stderr (panicking from Drop would abort the process); a
            // failed sidecar write leaves the on-disk state without a
            // valid clean-close marker → next open detects corruption →
            // callback fires.  By design.
            if self.durable_meta_path.is_some() && !self.read_only {
                if let Err(e) = self.flush_durable_sidecar() {
                    eprintln!(
                        "store_durable: clean-close sidecar write failed: {e}; \
                         next open will treat the store as corrupt"
                    );
                }
            }
            return;
        }
        let l = Layout::from_size_align(self.size as usize * 8, 8).expect("Problem");
        unsafe { A.dealloc(self.ptr, l) };
    }
}

#[allow(dead_code)]
impl Store {
    /// Total capacity of this store in bytes.
    #[must_use]
    pub fn byte_capacity(&self) -> u64 {
        u64::from(self.size) * 8
    }

    /// Total capacity of this store in 8-byte words.
    #[must_use]
    pub fn capacity_words(&self) -> u32 {
        self.size
    }

    /// @P294: grow this store's backing buffer to at least `words` words,
    /// in place — no record relocation, so any `(store_nr, rec, pos)`
    /// `DbRef` into this store stays valid (only the raw `ptr` moves, and
    /// every access re-derives it).  No-op when already large enough.
    /// Used by the interpreter to extend the value-stack store (#0) on
    /// deep call nesting, where the stack writes bypass `claim` and so
    /// never trigger the normal growth path.
    pub fn grow_words(&mut self, words: u32) {
        self.resize_store(words);
    }

    /// Raw base pointer to the store's memory buffer.
    #[must_use]
    pub fn base_ptr(&self) -> *mut u8 {
        self.ptr
    }

    pub fn new(size: u32) -> Store {
        // `init()` writes record 1's header at byte offset 8, so the
        // backing buffer must hold at least 2 words.  Smaller sizes
        // produce an OOB write that Linux's allocator slack tolerates
        // but Windows catches at deallocation as STATUS_HEAP_CORRUPTION
        // (0xc0000374).
        let size = size.max(2);
        let l = Layout::from_size_align(size as usize * 8, 8).expect("Problem");
        let ptr = unsafe { A.alloc_zeroed(l) };
        let mut store = Store {
            ptr,
            size,
            claims: HashSet::new(),
            #[cfg(feature = "mmap")]
            file: None,
            free: true,
            read_only: false,
            free_protected: false,
            borrowed: false,
            created_at: 0,
            last_op_at: 0,
            free_root: 0,
            needs_coalesce: false,
            generation: 0,
            recording: None,
            tag: 0,
            pinned: false,
            lock_origin: String::new(),
            known_type: u16::MAX,
            durable_meta_path: None,
            durable_tier: 0,
        };
        store.init(); // sets claims = {PRIMARY} and free_root = 0
        store
    }

    /// A standalone, immediately-usable store for tests and fuzzing.
    ///
    /// `new` returns a store flagged `free` (the database layer clears that
    /// when it registers the store into a `Stores`); without a `Stores` the
    /// store's own `validate()` rejects it as "freed".  This mirrors what the
    /// in-crate unit tests do by hand (`store.free = false`) and gives external
    /// callers (the fuzz harness) a clean entry point.
    #[doc(hidden)]
    #[must_use]
    pub fn new_in_use(size: u32) -> Store {
        let mut store = Store::new(size);
        store.free = false;
        store
    }

    /// @PLN11 arc E — cheap validity pre-check for a store image file: it
    /// exists, is large enough, and starts with the store [`SIGNATURE`].  Lets
    /// the startup cache reject a corrupt / non-store / truncated bundle and
    /// fall back to a cold parse instead of letting [`Store::open`] panic on a
    /// bad signature.  (`SIGNATURE` is written native-endian; a cache is never
    /// shared across architectures — it is keyed by target triple.)
    #[cfg(feature = "mmap")]
    #[must_use]
    pub fn is_store_file(path: &str) -> bool {
        use std::io::Read as _;
        let Ok(mut f) = std::fs::File::open(path) else {
            return false;
        };
        if f.metadata().map_or(0, |m| m.len()) < 16 {
            return false;
        }
        let mut buf = [0u8; 4];
        f.read_exact(&mut buf).is_ok() && u32::from_ne_bytes(buf) == SIGNATURE
    }

    #[cfg(not(feature = "mmap"))]
    pub fn open(_path: &str) -> Store {
        panic!(
            "mmap feature is not compiled in; enable the `mmap` Cargo feature to use file-backed stores"
        )
    }

    #[cfg(feature = "mmap")]
    pub fn open(path: &str) -> Store {
        let mut file = MmapStorage::open(path).expect("Opening file");
        let init = if (file.capacity() / 8) < 1024 {
            file.resize(8192).unwrap();
            true
        } else {
            false
        };
        // @PLAN38 phase 01 — `size` MUST be read AFTER the resize.  The
        // previous code captured it from pre-resize capacity, so for a
        // freshly-created file (`MmapStorage::open` initialises at 1 byte
        // → resized to 8192) the Store struct received `size = 0` while
        // the buffer was 8192 bytes.  `init()` then wrote the record-1
        // header using the bad size → on re-open, `fl_rebuild()` walked
        // garbage block headers and hit `block_size = 0` → infinite loop
        // in release builds.
        let size = (file.capacity() / 8) as u32;
        let ptr = std::ptr::addr_of!(file.as_slice()[0]).cast_mut();
        let mut store = Store {
            file: Some(file),
            ptr,
            claims: HashSet::new(),
            size,
            // An opened FILE-BACKED store is in use by definition (it
            // carries real data and `open` itself validates it below,
            // before any `adopt_store` registration could clear the
            // flag) — unlike `Store::new`, whose blank store stays
            // `free` until the database layer registers it.
            free: false,
            read_only: false,
            free_protected: false,
            free_root: 0,
            needs_coalesce: false,
            generation: 0,
            recording: None,
            tag: 0,
            borrowed: false,
            created_at: 0,
            last_op_at: 0,
            pinned: false,
            lock_origin: String::new(),
            known_type: u16::MAX,
            durable_meta_path: None,
            durable_tier: 0,
        };
        if init {
            store.init();
        } else {
            assert_eq!(
                unsafe { store.ptr.cast::<u32>().read_unaligned() },
                SIGNATURE,
                "Unknown file format"
            );
            #[cfg(debug_assertions)]
            store.validate(0);
            store.fl_rebuild();
        }
        store
    }

    pub fn init(&mut self) {
        // The normal routines will not write to rec=0, so we write a signature: StoreV01
        unsafe {
            self.ptr.cast::<u32>().write_unaligned(SIGNATURE);
            // The first empty space
            self.ptr.add(4).cast::<u32>().write_unaligned(1);
        }
        // Indicate the complete store as empty
        *self.addr_mut(1, 0) = -(self.size as i32) + 1;
        // Reset the LLRB free-space tree and claims to match the fresh store layout.
        // Without this, a re-used store's stale tree would cause fl_take_ge to allocate
        // from old split blocks at positions other than 1, breaking the rec=1 invariant
        // relied upon by database-level code.
        self.free_root = 0;
        self.claims.clear();
        self.claims.insert(PRIMARY);
    }

    /// @P317 debug — `LOFT_LOG=zero_claim` (or `LOFT_ZERO_CLAIM=1`) zeroes
    /// every freshly-claimed record's payload, so a read-before-write or a
    /// stale-`DbRef` read picks up a deterministic `0` instead of arena slack
    /// (old record data, or the freed block's LLRB free-list pointers, which
    /// live at offset 4 — exactly where a vector's length word sits).  This
    /// turns NON-deterministic uninitialised-arena bugs — which valgrind
    /// cannot see, because the arena buffer is itself validly allocated — into
    /// deterministic, reproducible failures.  Read once (cached); zero cost
    /// when off.  See `doc/claude/DEBUG.md` § store-ownership debugging.
    fn zero_claim_enabled() -> bool {
        // Default ON: a claimed block's PAYLOAD must read as zero.  `claim` reuses freed blocks
        // WITHOUT clearing them, so a caller that relies on zero-init — e.g. an empty `[]`
        // collection placeholder (`V{a:[]}` / `parts: vector<T> = []`), which assumes the field/
        // var handle is already 0 — instead inherits the freed block's STALE bytes.  That stale
        // collection handle then resolves to a non-claimed record in `remove_claims`/`length_vector`
        // → a use-after-free SIGSEGV (135-vector-u8-concat gate-on; @PLN25).  Zeroing the payload at
        // the single claim chokepoint makes the invariant hold for every caller (interpreter only;
        // native uses Rust ownership and never hits this).  `LOFT_NO_ZERO_CLAIM` disables it for
        // perf benchmarking only.
        static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *FLAG.get_or_init(|| std::env::var("LOFT_NO_ZERO_CLAIM").is_err())
    }

    /// Common tail of every `claim` path: zero the claimed payload when
    /// `zero_claim` is enabled (@P317 debugging lever).  No-op otherwise.
    #[inline]
    fn finish_claim(&mut self, pos: u32) -> u32 {
        if Self::zero_claim_enabled() {
            self.zero_fill(pos);
        }
        pos
    }

    /// Claim the space of a record
    /// # Arguments
    /// * `size` - The requested record size in 8 byte words
    pub fn claim(&mut self, size: u32) -> u32 {
        debug_assert!(
            !self.read_only,
            "Claim on read-only store (size={size}) (locked by: {})",
            self.lock_origin
        );
        assert!(
            !self.read_only,
            "Claim on read-only store (size={size}) (locked by: {})",
            self.lock_origin
        );
        assert!(size >= 1, "Incomplete record");
        // CO1.9/S28: increment generation so coroutine_next can detect store mutations
        // that may invalidate DbRef locals held by suspended generators.
        self.generation = self.generation.wrapping_add(1);
        #[cfg(debug_assertions)]
        self.fl_validate();
        // Fast path: find the smallest tracked free block that fits.
        if let Some(pos) = self.fl_take_ge(size as i32) {
            let result = self.claim_block(pos, size);
            #[cfg(debug_assertions)]
            self.fl_validate();
            return self.finish_claim(result);
        }
        // P6: a fast-path miss means no single tracked free block fits.
        // `delete` only merges forward, so adjacent frees that each don't
        // fit (but together would) are left uncoalesced and `claim_scan`
        // below would grow the store.  Coalesce the chain (reusing the one
        // free tree — no new index) and retry once before growing.  Guarded
        // by `needs_coalesce` so an alloc-only workload never sweeps.
        if self.needs_coalesce {
            self.coalesce_free();
            if let Some(pos) = self.fl_take_ge(size as i32) {
                let result = self.claim_block(pos, size);
                #[cfg(debug_assertions)]
                self.fl_validate();
                // This coalesce path reuses a freed block too — zero its payload like the other
                // claim paths (it previously returned WITHOUT `finish_claim`, leaking stale bytes).
                return self.finish_claim(result);
            }
        }
        // Slow path: linear scan (handles size-1 blocks and first-time allocation).
        let result = self.claim_scan(size);
        #[cfg(debug_assertions)]
        self.fl_validate();
        self.finish_claim(result)
    }

    /// Mark `pos` as claimed (splitting if the block is much larger than `size`).
    fn claim_block(&mut self, pos: u32, size: u32) -> u32 {
        let req_size = size as i32;
        let block_size = -(*self.addr::<i32>(pos, 0));
        assert!(block_size >= req_size, "Claimed block too small at {pos}");
        if block_size > req_size * 4 / 3 {
            *self.addr_mut(pos, 0) = req_size;
            let new_free = pos + size;
            *self.addr_mut(new_free, 0) = req_size - block_size; // negative = free
            self.fl_insert(new_free);
        } else {
            *self.addr_mut(pos, 0) = block_size; // positive = claimed
        }
        self.claims.insert(pos);
        // @PLN16.J: record the claim while edit-recording is on.  Read the *actual*
        // claimed size from the header (claim_block may take the whole block without
        // splitting), so replay's `claim_at` reproduces the exact extent.
        if self.recording.is_some() {
            let size = (*self.addr::<i32>(pos, 0)) as u32;
            if let Some(log) = self.recording.as_mut() {
                log.push(StoreChange::Insert { pos, size });
            }
        }
        pos
    }

    /// Linear-scan fallback for `claim()`: walks from PRIMARY until a free block
    /// of the required size is found, growing the store if necessary.
    fn claim_scan(&mut self, size: u32) -> u32 {
        let req_size = size as i32;
        let mut pos = PRIMARY;
        let mut last = pos;
        let mut claim = *self.addr::<i32>(pos, 0);
        while pos < self.size && (claim >= 0 || -claim < req_size) {
            last = pos;
            pos += i32::abs(claim) as u32;
            if pos >= self.size {
                break;
            }
            debug_assert_ne!(pos, last, "Inconsistent database zero sized block {pos}");
            claim = *self.addr::<i32>(pos, 0);
        }
        if pos >= self.size {
            // If the last block is free and tracked in the LLRB tree, remove it
            // before claim_grow changes its header in place.  Without this step
            // the tree retains a stale node that claim_block would later claim,
            // leaving a positive-header block reachable from free_root.
            if claim < 0 {
                self.fl_remove(last);
            }
            pos = self.claim_grow(size, last, claim);
            #[cfg(debug_assertions)]
            self.validate(0);
        }
        self.claim_block(pos, size)
    }

    /// Grow the store to accommodate `size` words and return the position of the
    /// new free block (either the extended last block or a fresh one).
    fn claim_grow(&mut self, size: u32, last: u32, last_claim: i32) -> u32 {
        let cur = self.size;
        let new_size = if last_claim < 0 {
            (self.size as i32 + size as i32 + last_claim) as u32
        } else {
            self.size.checked_add(size).unwrap_or_else(|| {
                panic!(
                    "store size limit exceeded: {} words ({} bytes)",
                    u64::from(self.size) + u64::from(size),
                    (u64::from(self.size) + u64::from(size)) * 8
                )
            })
        };
        self.resize_store(new_size);
        let increase = (self.size - cur) as i32;
        if last_claim < 0 {
            *self.addr_mut(last, 0) = last_claim - increase;
            last
        } else {
            *self.addr_mut(cur, 0) = -increase;
            cur
        }
    }

    /// Mutate the claimed size of a record
    pub fn resize(&mut self, rec: u32, size: u32) -> u32 {
        // CO1.9/S28: increment generation so coroutine_next can detect resize operations
        // that may invalidate DbRef locals (record relocation) held by suspended generators.
        self.generation = self.generation.wrapping_add(1);
        let req_size = size as i32;
        let claim = *self.addr::<i32>(rec, 0);
        if claim >= req_size {
            return rec;
        }
        let next = rec + claim as u32;
        if next < self.size {
            let next_size = *self.addr::<i32>(next, 0);
            if next_size < 0 && claim - next_size > req_size {
                // The adjacent free block can cover the growth.
                self.fl_remove(next);
                let act = req_size * 7 / 4;
                let new_size = if claim - next_size > act {
                    let new_next = rec + act as u32;
                    let new_free_size = (-next_size) as u32 + next - new_next;
                    *self.addr_mut(rec, 0) = act;
                    *self.addr_mut(new_next, 0) = -(new_free_size as i32);
                    self.fl_insert(new_next);
                    act
                } else {
                    *self.addr_mut(rec, 0) = claim - next_size;
                    claim - next_size
                };
                // The absorbed region (old end `claim` .. new end) held the freed block's
                // STALE bytes.  `claim`/`finish_claim` zero a payload on allocation so a
                // freshly-exposed slot reads as 0 (the invariant `set_default_value` and the
                // vector/text readers rely on); an in-place grow must uphold the SAME
                // invariant or a newly-exposed vector element carries garbage text/vec
                // handles that `remove_claims`/`length_vector` then follow into a UAF
                // (cluster-462, #462 @ sim.loft:3546).  Zero only the grown tail; the old
                // payload (words 1..claim) is preserved.  Same flag as `claim` so
                // `LOFT_NO_ZERO_CLAIM` toggles both together.
                if Self::zero_claim_enabled() {
                    self.zero_range(rec, claim as u32 * 8, (new_size - claim) as u32 * 8);
                }
                return rec;
            }
        }
        let new = self.claim(size);
        self.copy(rec, new);
        self.delete(rec);
        new
    }

    /// Delete a record, this assumes that all links towards this record are already removed
    pub fn delete(&mut self, rec: u32) {
        // BOTH `read_only` and `free_protected` block deletes: the
        // first is hard immutability (CONST_STORE / workers); the
        // second is the call-bracket's "don't free me" marker.
        let frozen = self.read_only || self.free_protected;
        debug_assert!(
            !frozen,
            "Delete on locked store (rec={rec}) (locked by: {})",
            self.lock_origin
        );
        assert!(
            !frozen,
            "Delete on locked store (rec={rec}) (locked by: {})",
            self.lock_origin
        );
        // CO1.9/S28: increment generation so coroutine_next can detect deletions that
        // may free a record still referenced by a suspended generator.
        self.generation = self.generation.wrapping_add(1);
        self.valid(rec, 4);
        // @PLN16.J: snapshot the record *before* delete repurposes its body as a
        // free-tree node (probe 5a), while edit-recording is on.
        if self.recording.is_some() {
            let words = (*self.addr::<i32>(rec, 0)) as u32;
            let before = self.read_span(rec, 0, words * 8);
            if let Some(log) = self.recording.as_mut() {
                log.push(StoreChange::Free { pos: rec, before });
            }
        }
        let mut claim = *self.addr::<i32>(rec, 0);
        // Coalesce with any adjacent free blocks that follow.
        while (rec + claim as u32) < self.size {
            let next_pos = rec + claim as u32;
            let next_header = *self.addr::<i32>(next_pos, 0);
            if next_header >= 0 {
                break;
            }
            // Remove the about-to-be-absorbed block from the tree before merging.
            self.fl_remove(next_pos);
            claim -= next_header;
        }
        *self.addr_mut(rec, 0) = -claim;
        self.claims.remove(&rec);
        // Register the (possibly coalesced) free block in the tree.
        self.fl_insert(rec);
        // P6: a free block now exists.  `delete` only merged FORWARD, so an
        // adjacent free PREDECESSOR (if any) is left uncoalesced; flag it so
        // the next allocation that would otherwise grow the store runs the
        // lazy coalescing sweep first.
        self.needs_coalesce = true;
        #[cfg(debug_assertions)]
        self.fl_validate();
    }

    /// Claim the record at an **exact** position with an **exact** size (words),
    /// carving it out of whatever free block currently covers `pos`.  Unlike
    /// [`claim`](Self::claim) — best-fit, position chosen by the allocator — this
    /// reproduces a *recorded* position.  It is the keystone of the @PLN16.J store
    /// journal's position-addressed replay (`Insert`-apply and `Free`-revert): forward
    /// and reverse replay are exact because positions are forced, not re-derived.
    ///
    /// `[pos, pos + size)` must lie entirely within free space.  The covering free
    /// block `[base, bend)` (with `base <= pos`, found by walking the block chain) is
    /// split three ways — `[base, pos)` free, `[pos, pos + size)` claimed,
    /// `[pos + size, bend)` free.  A remainder below `MIN_FREE_TREE` keeps a valid free
    /// header but is left untracked (coalesced later), the same rule `claim_block`
    /// follows.
    pub(crate) fn claim_at(&mut self, pos: u32, size: u32) -> u32 {
        debug_assert!(!self.read_only, "claim_at on read-only store");
        assert!(size >= 1, "claim_at: zero-size record");
        // S28: a structural op — bump generation like claim/delete/resize.
        self.generation = self.generation.wrapping_add(1);
        // Walk the block chain to find the block physically covering `pos`.
        let mut base = PRIMARY;
        loop {
            assert!(base < self.size, "claim_at: pos {pos} past end of store");
            let bsz = (*self.addr::<i32>(base, 0)).unsigned_abs();
            assert!(bsz > 0, "claim_at: zero-size block at {base}");
            if pos < base + bsz {
                break;
            }
            base += bsz;
        }
        let header = *self.addr::<i32>(base, 0);
        assert!(
            header < 0,
            "claim_at: region at {pos} is not free (covering block {base} is claimed)"
        );
        // The free region may be *fragmented* into several adjacent free blocks:
        // `delete` coalesces only forward, so a freed predecessor stays a separate block
        // until lazy `coalesce_free` runs.  Absorb consecutive free blocks (detaching
        // each from the tree) until they span `[pos, pos + size)`.  A claimed block
        // before then is a genuine "region not free" error.
        self.fl_remove(base);
        let mut bend = base + (-header) as u32;
        while bend < pos + size {
            assert!(
                bend < self.size,
                "claim_at: record [{pos}, {}) runs past the store end",
                pos + size
            );
            let next = *self.addr::<i32>(bend, 0);
            assert!(
                next < 0,
                "claim_at: record [{pos}, {}) hits a claimed block at {bend}",
                pos + size
            );
            self.fl_remove(bend);
            bend += (-next) as u32;
        }
        // [base, pos) prefix stays free.
        if base < pos {
            *self.addr_mut::<i32>(base, 0) = -((pos - base) as i32);
            self.fl_insert(base);
        }
        // [pos, pos + size) becomes the claimed record.
        *self.addr_mut::<i32>(pos, 0) = size as i32;
        self.claims.insert(pos);
        // [pos + size, bend) suffix stays free.
        let tail = pos + size;
        if tail < bend {
            *self.addr_mut::<i32>(tail, 0) = -((bend - tail) as i32);
            self.fl_insert(tail);
        }
        #[cfg(debug_assertions)]
        self.fl_validate();
        pos
    }

    /// @PLN16.J — begin buffering structural changes for the debugger's edit journal.
    /// No-op on a locked / freed store (those are never edit targets).
    pub(crate) fn start_recording(&mut self) {
        if !self.free && !self.read_only {
            self.recording = Some(Vec::new());
        }
    }

    /// @PLN16.J — stop recording and hand back the buffered changes (`None` if this
    /// store was never recording).
    pub(crate) fn take_recording(&mut self) -> Option<Vec<StoreChange>> {
        self.recording.take()
    }

    /// Validate the store
    pub fn validate(&self, recs: u32) {
        if !cfg!(debug_assertions) {
            return;
        }
        assert!(!self.free, "Using a freed store");
        let mut pos = PRIMARY;
        let mut alloc = 0;
        while pos < self.size {
            let claim = *self.addr::<i32>(pos, 0);
            assert!(
                pos + i32::abs(claim) as u32 <= self.size,
                "Incorrect record {pos} size {}",
                i32::abs(claim)
            );
            if claim < 0 {
                // ignore the open spaces for now, later we want to check if they are part of the open tree.
                pos += (-claim) as u32;
            } else {
                // check the claimed records
                alloc += 1;
                pos += claim as u32;
            }
        }
        assert_eq!(pos, self.size, "Incorrect {pos} size {}", self.size);
        assert!(
            recs == 0 || alloc == recs as usize,
            "Inconsistent number of records: claimed {alloc} walk {recs}"
        );
    }

    pub fn len(&self) -> u32 {
        self.size
    }

    /// Walk the block chain and report internal space utilisation.
    /// Same traversal as [`Self::validate`]: a negative size header
    /// (`addr::<i32>(pos, 0)`) is a free block of `-size` words, a
    /// positive header is a claimed record of `size` words.  Word 0 is
    /// the store header and is excluded.  A freed store reports only its
    /// capacity.
    #[must_use]
    pub fn usage(&self) -> StoreUsage {
        let mut u = StoreUsage {
            capacity_words: self.size,
            ..StoreUsage::default()
        };
        if self.free || self.size <= PRIMARY {
            return u;
        }
        let mut pos = PRIMARY;
        let mut prev_was_free = false;
        while pos < self.size {
            let claim = *self.addr::<i32>(pos, 0);
            if claim == 0 {
                break; // malformed / uninitialised tail — stop rather than spin
            }
            let sz = claim.unsigned_abs();
            if claim < 0 {
                u.free_words += sz;
                u.free_count += 1;
                if sz > u.largest_free_words {
                    u.largest_free_words = sz;
                }
                // Two consecutive free blocks are physical neighbours that
                // `delete` should have coalesced — flag the missed merge.
                if prev_was_free {
                    u.mergeable_free_pairs += 1;
                }
                prev_was_free = true;
            } else {
                u.claimed_words += sz;
                u.claimed_count += 1;
                prev_was_free = false;
            }
            pos += sz;
        }
        u
    }

    /// Change the store size, do not mutate content
    fn resize_store(&mut self, to_size: u32) {
        if to_size <= self.size {
            return;
        }
        assert!(
            to_size <= MAX_STORE_WORDS,
            "store offset overflow: requested {} words exceeds limit of {} ({} bytes)",
            to_size,
            MAX_STORE_WORDS,
            u64::from(MAX_STORE_WORDS) * 8
        );
        // saturating_mul prevents u32 overflow when the store is very large
        let inc = self.size.saturating_mul(7) / 3;
        let size = if to_size > inc { to_size } else { inc };
        #[cfg(feature = "mmap")]
        if let Some(f) = &mut self.file {
            f.resize(size as usize * 8).expect("Resize");
            self.ptr = std::ptr::addr_of!(f.as_slice()[0]).cast_mut();
            self.size = size;
            return;
        }
        let old_bytes = self.size as usize * 8;
        let bytes = size as usize * 8;
        let l = Layout::from_size_align(old_bytes, 8).expect("Problem");
        self.ptr = unsafe { A.realloc(self.ptr, l, bytes) };
        if bytes > old_bytes {
            unsafe { self.ptr.add(old_bytes).write_bytes(0, bytes - old_bytes) };
        }
        self.size = size;
    }

    /// Lock this store against writes. Any subsequent call to `addr_mut` panics.
    /// Sets `lock_origin` to a generic identifier — callers wanting richer
    /// context should use `lock_with_origin`.
    pub fn lock(&mut self) {
        self.lock_with_origin("Store::lock");
    }

    /// Lock this store + record an identifier of the lock-origin call site.
    /// The origin string surfaces in panic messages from `addr_mut` / `claim`
    /// / `delete` so a "Write to read-only store" failure points directly at the
    /// locker rather than requiring `LOFT_LOG=locks` to re-trace.
    pub fn lock_with_origin(&mut self, origin: impl Into<String>) {
        let origin = origin.into();
        // Plan-22 02d-vii follow-up — `LOFT_LOG=locks` trace.
        // Caught here (the lowest-level lock site) so direct
        // callers bypassing `Stores::lock_store(&r)` (e.g.
        // `compile.rs` const-store init, `native.rs` worker
        // store init) are visible too.
        if !self.read_only && crate::log_config::lock_trace_enabled() {
            eprintln!("[locks] LOCK   origin={origin:?}");
        }
        self.read_only = true;
        self.lock_origin = origin;
    }

    /// Unlock this store (only callable from Rust; loft code cannot unlock via d#lock = false
    /// on a const variable).
    pub fn unlock(&mut self) {
        if self.read_only && crate::log_config::lock_trace_enabled() {
            eprintln!("[locks] UNLOCK origin-was={:?}", self.lock_origin);
        }
        self.read_only = false;
        self.lock_origin.clear();
    }

    /// @P290 — mark the store as PROTECTED-FROM-FREE for the duration
    /// of a fn-call deep-copy bracket.  Writes / claims stay legal;
    /// only `delete` (and `OpCopyRecord`'s `0x8000` source-free) is
    /// blocked.  Cleared by `clear_free_protected()`.
    pub fn set_free_protected(&mut self, origin: impl Into<String>) {
        let origin = origin.into();
        if !self.free_protected && crate::log_config::lock_trace_enabled() {
            eprintln!("[locks] FREE_PROTECT origin={origin:?}");
        }
        self.free_protected = true;
        self.lock_origin = origin;
    }

    /// @P290 — clear the call-bracket free-protection.
    pub fn clear_free_protected(&mut self) {
        if self.free_protected && crate::log_config::lock_trace_enabled() {
            eprintln!("[locks] FREE_UNPROTECT origin-was={:?}", self.lock_origin);
        }
        self.free_protected = false;
        // Clear lock_origin only if the hard read_only lock isn't also
        // holding it (it shouldn't be — but be defensive).
        if !self.read_only {
            self.lock_origin.clear();
        }
    }

    /// Return whether this store is HARD-locked (read-only).
    /// CONST_STORE / worker borrow / JSON null sentinel.  Does NOT
    /// include the call-bracket free-protection.
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.read_only
    }

    /// Return whether this store is currently protected from frees
    /// by a fn-call deep-copy bracket.  @P290.
    #[must_use]
    pub fn is_free_protected(&self) -> bool {
        self.free_protected
    }

    /// Return whether this store is a borrowed view of another store's buffer.
    #[must_use]
    pub fn is_borrowed(&self) -> bool {
        self.borrowed
    }

    /// Return whether this store has an empty claims set (worker clones).
    #[must_use]
    pub fn claims_empty(&self) -> bool {
        self.claims.is_empty()
    }

    /// Create a locked deep-copy of this store for use in a worker thread.
    /// The clone always has `locked = true`; the mmap file is not shared (data is copied).
    pub fn clone_locked(&self) -> Store {
        let l = Layout::from_size_align(self.size as usize * 8, 8).expect("Problem");
        let ptr = unsafe { A.alloc(l) };
        unsafe { std::ptr::copy_nonoverlapping(self.ptr, ptr, self.size as usize * 8) };
        Store {
            ptr,
            size: self.size,
            claims: self.claims.clone(),
            #[cfg(feature = "mmap")]
            file: None,
            free: self.free,
            read_only: true,
            free_protected: false,
            free_root: 0, // workers never claim/delete; no free tree needed
            needs_coalesce: false,
            generation: self.generation,
            recording: None,
            tag: self.tag,
            borrowed: false,
            created_at: 0,
            last_op_at: 0,
            pinned: self.pinned,
            lock_origin: "clone_locked".to_string(),
            known_type: self.known_type,
            durable_meta_path: None,
            durable_tier: 0,
        }
    }

    /// Like [`clone_locked`] but omits the `claims` clone (S29/P1-R3).
    /// Worker stores are always locked and never call `validate()`, so the claims
    /// `HashSet` is wasted memory — O(records) allocation per worker per `par()` call.
    pub fn clone_locked_for_worker(&self) -> Store {
        let l = Layout::from_size_align(self.size as usize * 8, 8).expect("Problem");
        let ptr = unsafe { A.alloc(l) };
        unsafe { std::ptr::copy_nonoverlapping(self.ptr, ptr, self.size as usize * 8) };
        Store {
            ptr,
            size: self.size,
            claims: HashSet::new(), // S29/P1-R3: workers never validate(); skip clone
            #[cfg(feature = "mmap")]
            file: None,
            free: self.free,
            read_only: true,
            free_protected: false,
            free_root: 0,
            needs_coalesce: false,
            generation: self.generation,
            recording: None,
            tag: self.tag,
            borrowed: false,
            created_at: 0,
            last_op_at: 0,
            pinned: self.pinned,
            lock_origin: "clone_locked_for_worker".to_string(),
            known_type: self.known_type,
            durable_meta_path: None,
            durable_tier: 0,
        }
    }

    /// Create a read-only view that shares the original's buffer pointer.
    /// The borrow is `locked = true` (writes panic/discard) and `borrowed = true`
    /// (Drop does NOT free the buffer — the main thread owns it).
    ///
    /// # Safety
    /// The original `Store` must outlive all threads that hold the borrow.
    /// Guaranteed by `thread::scope` in `run_parallel_light`.
    pub unsafe fn borrow_locked_for_light_worker(&self) -> Store {
        Store {
            ptr: self.ptr,
            claims: HashSet::new(),
            size: self.size,
            #[cfg(feature = "mmap")]
            file: None,
            free: false,
            read_only: true,
            free_protected: false,
            free_root: self.free_root,
            needs_coalesce: false,
            generation: self.generation,
            recording: None,
            tag: self.tag,
            borrowed: true,
            created_at: 0,
            last_op_at: 0,
            pinned: self.pinned,
            lock_origin: "borrow_locked_for_light_worker".to_string(),
            known_type: self.known_type,
            durable_meta_path: None,
            durable_tier: 0,
        }
    }

    /// Create a sentinel for a freed main-thread slot.
    /// Tiny allocation, no data — just a placeholder so the worker's slot indices align.
    pub fn new_freed_sentinel() -> Store {
        let mut s = Store::new(4);
        s.free = true;
        s
    }

    // ---- LLRB free-space tree ------------------------------------------------
    // Nodes are stored inside free blocks using fields at FL_LEFT / FL_RIGHT /
    // FL_COLOR.  Key = (positive_block_size, block_position); ties break on pos.
    // Only blocks with size >= MIN_FREE_TREE are tracked.

    fn fl_size(&self, p: u32) -> i32 {
        -*self.addr::<i32>(p, 0)
    }

    fn fl_left(&self, p: u32) -> u32 {
        *self.addr::<u32>(p, FL_LEFT)
    }

    fn fl_right(&self, p: u32) -> u32 {
        *self.addr::<u32>(p, FL_RIGHT)
    }

    fn fl_red(&self, p: u32) -> bool {
        *self.addr::<u8>(p, FL_COLOR) != 0
    }

    fn fl_set_left(&mut self, p: u32, v: u32) {
        *self.addr_mut::<u32>(p, FL_LEFT) = v;
    }

    fn fl_set_right(&mut self, p: u32, v: u32) {
        *self.addr_mut::<u32>(p, FL_RIGHT) = v;
    }

    fn fl_set_red(&mut self, p: u32, v: bool) {
        *self.addr_mut::<u8>(p, FL_COLOR) = u8::from(v);
    }

    fn fl_cmp(&self, a: u32, b: u32) -> Ordering {
        match self.fl_size(a).cmp(&self.fl_size(b)) {
            Ordering::Equal => a.cmp(&b),
            other => other,
        }
    }

    fn fl_rotate_left(&mut self, h: u32) -> u32 {
        let x = self.fl_right(h);
        let x_left = self.fl_left(x);
        self.fl_set_right(h, x_left);
        self.fl_set_left(x, h);
        let h_red = self.fl_red(h);
        self.fl_set_red(x, h_red);
        self.fl_set_red(h, true);
        x
    }

    fn fl_rotate_right(&mut self, h: u32) -> u32 {
        let x = self.fl_left(h);
        let x_right = self.fl_right(x);
        self.fl_set_left(h, x_right);
        self.fl_set_right(x, h);
        let h_red = self.fl_red(h);
        self.fl_set_red(x, h_red);
        self.fl_set_red(h, true);
        x
    }

    fn fl_flip_colors(&mut self, h: u32) {
        let h_red = self.fl_red(h);
        self.fl_set_red(h, !h_red);
        let l = self.fl_left(h);
        if l != 0 {
            self.fl_set_red(l, !self.fl_red(l));
        }
        let r = self.fl_right(h);
        if r != 0 {
            self.fl_set_red(r, !self.fl_red(r));
        }
    }

    fn fl_balance(&mut self, mut h: u32) -> u32 {
        let r = self.fl_right(h);
        if r != 0 && self.fl_red(r) {
            h = self.fl_rotate_left(h);
        }
        let l = self.fl_left(h);
        let ll = if l != 0 { self.fl_left(l) } else { 0 };
        if l != 0 && self.fl_red(l) && ll != 0 && self.fl_red(ll) {
            h = self.fl_rotate_right(h);
        }
        let l2 = self.fl_left(h);
        let r2 = self.fl_right(h);
        if l2 != 0 && self.fl_red(l2) && r2 != 0 && self.fl_red(r2) {
            self.fl_flip_colors(h);
        }
        h
    }

    fn fl_insert_node(&mut self, h: u32, rec: u32) -> u32 {
        if h == 0 {
            self.fl_set_left(rec, 0);
            self.fl_set_right(rec, 0);
            self.fl_set_red(rec, true);
            return rec;
        }
        match self.fl_cmp(rec, h) {
            Ordering::Less => {
                let l = self.fl_left(h);
                let new_l = self.fl_insert_node(l, rec);
                self.fl_set_left(h, new_l);
            }
            Ordering::Greater | Ordering::Equal => {
                let r = self.fl_right(h);
                let new_r = self.fl_insert_node(r, rec);
                self.fl_set_right(h, new_r);
            }
        }
        self.fl_balance(h)
    }

    /// Register a free block in the LLRB free-space tree.
    /// Blocks with fewer than `MIN_FREE_TREE` words are silently ignored.
    fn fl_insert(&mut self, rec: u32) {
        if self.fl_size(rec) < MIN_FREE_TREE {
            return;
        }
        let root = self.free_root;
        self.free_root = self.fl_insert_node(root, rec);
        self.fl_set_red(self.free_root, false);
    }

    fn fl_min_node(&self, h: u32) -> u32 {
        if h == 0 {
            return 0;
        }
        let l = self.fl_left(h);
        if l == 0 { h } else { self.fl_min_node(l) }
    }

    fn fl_move_red_left(&mut self, mut h: u32) -> u32 {
        self.fl_flip_colors(h);
        let r = self.fl_right(h);
        let rl = if r != 0 { self.fl_left(r) } else { 0 };
        if rl != 0 && self.fl_red(rl) {
            let new_r = self.fl_rotate_right(r);
            self.fl_set_right(h, new_r);
            h = self.fl_rotate_left(h);
            self.fl_flip_colors(h);
        }
        h
    }

    fn fl_move_red_right(&mut self, mut h: u32) -> u32 {
        self.fl_flip_colors(h);
        let l = self.fl_left(h);
        let ll = if l != 0 { self.fl_left(l) } else { 0 };
        if ll != 0 && self.fl_red(ll) {
            h = self.fl_rotate_right(h);
            self.fl_flip_colors(h);
        }
        h
    }

    /// Remove the leftmost (minimum) node from the subtree rooted at `h`.
    fn fl_delete_min_node(&mut self, h: u32) -> u32 {
        if self.fl_left(h) == 0 {
            return 0;
        }
        let l = self.fl_left(h);
        let ll = self.fl_left(l);
        let mut cur = h;
        if !self.fl_red(l) && (ll == 0 || !self.fl_red(ll)) {
            cur = self.fl_move_red_left(cur);
        }
        let left = self.fl_left(cur);
        let new_left = self.fl_delete_min_node(left);
        self.fl_set_left(cur, new_left);
        self.fl_balance(cur)
    }

    /// Remove the block at `target_pos` from the subtree rooted at `h`.
    fn fl_delete_node(&mut self, mut h: u32, target_pos: u32) -> u32 {
        if h == 0 {
            return 0; // target not found in this subtree (shouldn't happen normally)
        }
        let target_sz = self.fl_size(target_pos);
        let h_sz = self.fl_size(h);
        if (target_sz, target_pos) < (h_sz, h) {
            let l = self.fl_left(h);
            let ll = if l != 0 { self.fl_left(l) } else { 0 };
            if l != 0 && !self.fl_red(l) && (ll == 0 || !self.fl_red(ll)) {
                h = self.fl_move_red_left(h);
            }
            let left = self.fl_left(h);
            let new_left = self.fl_delete_node(left, target_pos);
            self.fl_set_left(h, new_left);
        } else {
            if self.fl_left(h) != 0 && self.fl_red(self.fl_left(h)) {
                h = self.fl_rotate_right(h);
            }
            if h == target_pos && self.fl_right(h) == 0 {
                return 0;
            }
            let r = self.fl_right(h);
            let rl = if r != 0 { self.fl_left(r) } else { 0 };
            if r != 0 && !self.fl_red(r) && (rl == 0 || !self.fl_red(rl)) {
                h = self.fl_move_red_right(h);
            }
            if h == target_pos {
                let right = self.fl_right(h);
                if right == 0 {
                    // No right subtree after rotations; just return left.
                    return self.fl_left(h);
                }
                let succ = self.fl_min_node(right);
                let h_left = self.fl_left(h);
                let h_red = self.fl_red(h);
                let new_right = self.fl_delete_min_node(right);
                self.fl_set_left(succ, h_left);
                self.fl_set_right(succ, new_right);
                self.fl_set_red(succ, h_red);
                h = succ;
            } else {
                let right = self.fl_right(h);
                let new_right = self.fl_delete_node(right, target_pos);
                self.fl_set_right(h, new_right);
            }
        }
        self.fl_balance(h)
    }

    /// Find the position of the smallest free block with size >= `min_size`.
    /// Returns 0 when no suitable block exists.
    fn fl_find_ge(&self, h: u32, min_size: i32) -> u32 {
        if h == 0 {
            return 0;
        }
        if self.fl_size(h) < min_size {
            return self.fl_find_ge(self.fl_right(h), min_size);
        }
        let left_result = self.fl_find_ge(self.fl_left(h), min_size);
        if left_result != 0 { left_result } else { h }
    }

    /// Remove and return the smallest free block with size >= `min_size`.
    fn fl_take_ge(&mut self, min_size: i32) -> Option<u32> {
        if self.free_root == 0 {
            return None;
        }
        let found = self.fl_find_ge(self.free_root, min_size);
        if found == 0 {
            return None;
        }
        let root = self.free_root;
        self.free_root = self.fl_delete_node(root, found);
        if self.free_root != 0 {
            self.fl_set_red(self.free_root, false);
        }
        Some(found)
    }

    /// Remove `rec` from the free tree if it is currently tracked.
    fn fl_remove(&mut self, rec: u32) {
        if self.free_root == 0 || self.fl_size(rec) < MIN_FREE_TREE {
            return;
        }
        #[cfg(debug_assertions)]
        debug_assert!(
            self.fl_contains(rec),
            "fl_remove: block at {rec} (size={}) not in free tree",
            self.fl_size(rec)
        );
        let root = self.free_root;
        self.free_root = self.fl_delete_node(root, rec);
        if self.free_root != 0 {
            self.fl_set_red(self.free_root, false);
        }
    }

    /// Return `true` if `target` is reachable from the free-tree root.
    #[cfg(debug_assertions)]
    fn fl_contains(&self, target: u32) -> bool {
        self.fl_contains_node(self.free_root, target)
    }

    #[cfg(debug_assertions)]
    fn fl_contains_node(&self, h: u32, target: u32) -> bool {
        if h == 0 {
            return false;
        }
        if h == target {
            return true;
        }
        self.fl_contains_node(self.fl_left(h), target)
            || self.fl_contains_node(self.fl_right(h), target)
    }

    /// Scan the whole store and (re)build the free-space tree from scratch.
    /// Called once after `open()` to populate the tree from persisted data.
    pub fn fl_rebuild(&mut self) {
        self.free_root = 0;
        let mut pos = PRIMARY;
        while pos < self.size {
            let header = *self.addr::<i32>(pos, 0);
            let block_size = i32::abs(header);
            debug_assert!(block_size > 0, "zero-size block at {pos}");
            if header < 0 && -header >= MIN_FREE_TREE {
                self.fl_insert(pos);
            }
            pos += block_size as u32;
        }
    }

    /// P6: merge every run of adjacent free blocks in place, then rebuild
    /// the free tree from the coalesced chain.  `delete` only coalesces
    /// FORWARD (it cannot find a freed block's predecessor in the
    /// header-only layout), so adjacent frees accumulate; this single
    /// O(n) pass over the contiguous block chain (the same walk
    /// `claim_scan` / `usage` / `fl_rebuild` already do) catches them all,
    /// reusing the one existing free tree — no extra index, no footer.
    /// Called lazily by `claim` only when an allocation would otherwise
    /// grow the store, so freed space is reused instead.
    fn coalesce_free(&mut self) {
        let mut pos = PRIMARY;
        while pos < self.size {
            let header = *self.addr::<i32>(pos, 0);
            let mut block_size = i32::abs(header);
            debug_assert!(block_size > 0, "zero-size block at {pos}");
            if header < 0 {
                // Absorb following adjacent free blocks into this one.
                let mut next = pos + block_size as u32;
                while next < self.size {
                    let nh = *self.addr::<i32>(next, 0);
                    if nh >= 0 {
                        break;
                    }
                    block_size += i32::abs(nh);
                    next = pos + block_size as u32;
                }
                *self.addr_mut(pos, 0) = -block_size;
            }
            pos += block_size as u32;
        }
        self.fl_rebuild();
        self.needs_coalesce = false;
    }

    /// Debug-only: walk the LLRB tree and verify its invariants.
    ///
    /// Asserts that:
    /// - Every tree node has a negative fld-0 header (it is truly free).
    /// - No tree node is present in `claims` (freed ≠ claimed).
    #[cfg(debug_assertions)]
    pub fn fl_validate(&self) {
        self.fl_validate_node(self.free_root);
    }

    #[cfg(debug_assertions)]
    fn fl_validate_node(&self, h: u32) {
        if h == 0 {
            return;
        }
        let header: i32 = *self.addr(h, 0);
        debug_assert!(
            header < 0,
            "fl_validate: node at {h} has positive header {header} (should be free)"
        );
        debug_assert!(
            !self.claims.contains(&h),
            "fl_validate: node at {h} is both in the free tree and in claims"
        );
        self.fl_validate_node(self.fl_left(h));
        self.fl_validate_node(self.fl_right(h));
    }

    // ---- End of LLRB free-space tree -----------------------------------------

    /// Checked offset calculation — `rec * 8 + fld` using u64 to detect overflow.
    #[inline]
    fn checked_offset(rec: u32, fld: u32) -> isize {
        let off = u64::from(rec) * 8 + u64::from(fld);
        isize::try_from(off)
            .unwrap_or_else(|_| panic!("Store offset overflow: rec={rec} fld={fld}"))
    }

    #[inline]
    pub fn addr<T>(&self, rec: u32, fld: u32) -> &T {
        debug_assert!(
            Self::checked_offset(rec, fld) + std::mem::size_of::<T>() as isize
                <= self.size as isize * 8,
            "Store read out of bounds: rec={rec} fld={fld} size={} store_size={}",
            std::mem::size_of::<T>(),
            self.size * 8,
        );
        // Validate field offset against record's claimed size (first word).
        // rec=0 and rec=1 are special (store header / primary record).
        #[cfg(debug_assertions)]
        if rec > 1 && fld > 0 {
            let rec_header = unsafe {
                std::ptr::read_unaligned(
                    self.ptr
                        .add(Self::checked_offset(rec, 0) as usize)
                        .cast::<i32>(),
                )
            };
            let rec_size = rec_header.unsigned_abs() as isize * 8;
            debug_assert!(
                (fld as isize + std::mem::size_of::<T>() as isize) <= rec_size,
                "Fld {fld} is outside of record {rec} size {rec_size}",
            );
        }
        unsafe {
            let off = self.ptr.offset(Self::checked_offset(rec, fld)).cast::<T>();
            off.as_mut().expect("Reference")
        }
    }

    #[inline]
    pub fn addr_mut<T>(&mut self, rec: u32, fld: u32) -> &mut T {
        // Only hard `read_only` blocks writes.  Call-bracket
        // `free_protected` lets writes through (only frees are blocked).
        debug_assert!(
            !self.read_only,
            "Write to read-only store at rec={rec} fld={fld} (locked by: {})",
            self.lock_origin
        );
        debug_assert!(
            Self::checked_offset(rec, fld) + std::mem::size_of::<T>() as isize
                <= self.size as isize * 8,
            "Store write out of bounds: rec={rec} fld={fld} size={} store_size={}",
            std::mem::size_of::<T>(),
            self.size * 8,
        );
        #[cfg(debug_assertions)]
        if rec > 1 && fld > 0 {
            let rec_header = unsafe {
                std::ptr::read_unaligned(
                    self.ptr
                        .add(Self::checked_offset(rec, 0) as usize)
                        .cast::<i32>(),
                )
            };
            let rec_size = rec_header.unsigned_abs() as isize * 8;
            debug_assert!(
                (fld as isize + std::mem::size_of::<T>() as isize) <= rec_size,
                "Fld {fld} is outside of record {rec} size {rec_size}",
            );
        }
        assert!(
            !self.read_only,
            "Write to read-only store at rec={rec} fld={fld} (locked by: {})",
            self.lock_origin
        );
        unsafe {
            let off = self.ptr.offset(Self::checked_offset(rec, fld)).cast::<T>();
            off.as_mut().expect("Reference")
        }
    }

    pub fn buffer(&mut self, rec: u32) -> &mut [u8] {
        let size = *self.addr::<u32>(rec, 0) as usize * 8;
        unsafe {
            let p = self.ptr.offset(rec as isize * 8 + 8);
            std::slice::from_raw_parts_mut(p, size)
        }
    }

    /// @PLN16.J — read `len` raw bytes of a record starting at byte offset `off`
    /// (the same `(rec, off)` addressing as [`addr`](Self::addr)).  The store
    /// change journal uses this to snapshot a record region for undo / replay.
    #[must_use]
    pub fn read_span(&self, rec: u32, off: u32, len: u32) -> Box<[u8]> {
        debug_assert!(
            Self::checked_offset(rec, off) + len as isize <= self.size as isize * 8,
            "read_span out of bounds: rec={rec} off={off} len={len} store_size={}",
            self.size * 8,
        );
        let mut out = vec![0u8; len as usize];
        if len > 0 {
            // SAFETY: the span is bounds-checked above; `out` holds `len` bytes.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.ptr.offset(Self::checked_offset(rec, off)),
                    out.as_mut_ptr(),
                    len as usize,
                );
            }
        }
        out.into_boxed_slice()
    }

    /// @PLN16.J — write `bytes` into a record at byte offset `off` — the journal's
    /// only mutator (undo restores `before`, replay writes `after`).  A pure byte
    /// restore: no allocator interaction, so it never moves or resizes a record.
    /// Honors the hard `read_only` lock.
    pub fn write_span(&mut self, rec: u32, off: u32, bytes: &[u8]) {
        assert!(
            !self.read_only,
            "write_span on read-only store at rec={rec} (locked by: {})",
            self.lock_origin
        );
        debug_assert!(
            Self::checked_offset(rec, off) + bytes.len() as isize <= self.size as isize * 8,
            "write_span out of bounds: rec={rec} off={off} len={} store_size={}",
            bytes.len(),
            self.size * 8,
        );
        if !bytes.is_empty() {
            // SAFETY: the span is bounds-checked above; `bytes` is `bytes.len()` long.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    self.ptr.offset(Self::checked_offset(rec, off)),
                    bytes.len(),
                );
            }
        }
    }

    /// Fast check whether a value looks like a valid live record.
    /// Used by `get_ref()` to detect inline data that was misinterpreted
    /// as a record pointer.  Cheaper than `HashSet` lookup — just a range
    /// check and one memory read (the record header).
    #[must_use]
    pub fn is_valid_record(&self, rec: u32) -> bool {
        // Record must be within the store's allocated space and have a
        // positive header (live records have size > 0; freed have size < 0).
        rec > 0 && rec < self.size && *self.addr::<i32>(rec, 0) > 0
    }

    /// Try to validate a record reference as much as possible.
    /// Complete validations are only done in 'test' mode.
    pub fn valid(&self, rec: u32, fld: u32) -> bool {
        // S29/P1-R3: locked (worker) stores have empty claims by design — skip the
        // claims check.  Records in worker stores are valid copies of the originals.
        debug_assert!(
            self.read_only || self.claims.contains(&rec),
            "Unknown record {rec}"
        );
        // Read size before any multiplication to avoid overflow when fld 0 is negative
        // (a negative header means the block was freed — a bug if still in claims).
        let size: i32 = *self.addr(rec, 0);
        debug_assert!(
            size > 0,
            "Freed record {rec} (size={size}) accessed at fld {fld}"
        );
        debug_assert!(
            fld >= 4 && fld < 8 * size as u32,
            "Fld {fld} is outside of record {rec} size {}",
            8 * size as u32
        );
        debug_assert!(
            rec != 0 && u64::from(rec) * 8 + u64::from(fld) <= u64::from(self.size) * 8,
            "Reading outside store ({rec}.{fld}) > {}",
            self.size
        );
        if fld != 0 {
            // The first 4 positions are reserved for the record size
            debug_assert!(
                rec + size as u32 <= self.size,
                "Inconsistent record {rec} size {size} > {}",
                self.size
            );
            debug_assert!(
                fld >= 4,
                "Field {fld} too low, overlapping with size on ({rec}.{fld})"
            );
            debug_assert!(
                size >= 1 && fld <= size as u32 * 8,
                "Reading fields outside record ({rec}.{fld}) > {size}"
            );
        }
        true
    }

    #[inline]
    /// Copy only the content of a record, not the claimed size
    fn copy(&self, rec: u32, into: u32) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.ptr.offset(rec as isize * 8 + 4),
                self.ptr.offset(into as isize * 8 + 4),
                *self.addr::<i32>(rec, 0) as usize * 8 - 4,
            );
        }
    }

    #[inline]
    pub fn zero_fill(&self, rec: u32) {
        unsafe {
            std::ptr::write_bytes(
                self.ptr.offset(rec as isize * 8 + 4),
                0,
                *self.addr::<i32>(rec, 0) as usize * 8 - 4,
            );
        }
    }

    /// Zero `len` bytes at byte offset `pos` within record `rec`.  `claim`
    /// reuses freed blocks without clearing them, so a freshly-allocated vector
    /// element (or struct record) carries garbage; callers materialising into
    /// it must clear it first so unwritten vector-header sub-fields read as
    /// empty (`vec_rec = 0`) instead of dereferencing a junk record id.
    #[inline]
    pub fn zero_range(&mut self, rec: u32, pos: u32, len: u32) {
        if len == 0 {
            return;
        }
        let ptr: *mut u8 = self.addr_mut::<u8>(rec, pos);
        // SAFETY: callers pass a record/element's own `pos..pos+len`, which lies
        // within its claimed allocation.
        unsafe { std::ptr::write_bytes(ptr, 0, len as usize) }
    }

    #[inline]
    pub fn copy_block(
        &mut self,
        from_rec: u32,
        from_pos: isize,
        to_rec: u32,
        to_pos: isize,
        size: isize,
    ) {
        #[cfg(debug_assertions)]
        {
            let from_limit = *self.addr::<i32>(from_rec, 0) as isize * 8;
            let to_limit = *self.addr::<i32>(to_rec, 0) as isize * 8;
            debug_assert!(
                from_pos + size <= from_limit,
                "copy_block src OOB: rec={from_rec} [{from_pos}..+{size}] > {from_limit} bytes"
            );
            debug_assert!(
                to_pos + size <= to_limit,
                "copy_block dst OOB: rec={to_rec} [{to_pos}..+{size}] > {to_limit} bytes"
            );
        }
        unsafe {
            std::ptr::copy(
                self.ptr.offset(from_rec as isize * 8 + from_pos),
                self.ptr.offset(to_rec as isize * 8 + to_pos),
                size as usize,
            );
        }
    }

    #[inline]
    pub fn copy_block_between(
        &self,
        from_rec: u32,
        from_pos: isize,
        to_store: &mut Store,
        to_rec: u32,
        to_pos: isize,
        len: isize,
    ) {
        #[cfg(debug_assertions)]
        {
            let from_limit = *self.addr::<i32>(from_rec, 0) as isize * 8;
            let to_limit = *to_store.addr::<i32>(to_rec, 0) as isize * 8;
            debug_assert!(
                from_pos + len <= from_limit,
                "copy_block_between src OOB: rec={from_rec} [{from_pos}..+{len}] > {from_limit} bytes"
            );
            debug_assert!(
                to_pos + len <= to_limit,
                "copy_block_between dst OOB: rec={to_rec} [{to_pos}..+{len}] > {to_limit} bytes"
            );
        }
        unsafe {
            std::ptr::copy(
                self.ptr.offset(from_rec as isize * 8 + from_pos),
                to_store.ptr.offset(to_rec as isize * 8 + to_pos),
                len as usize,
            );
        }
    }

    #[inline]
    pub fn get_int(&self, rec: u32, fld: u32) -> i64 {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            i64::MIN
        }
    }

    #[inline]
    pub fn set_int(&mut self, rec: u32, fld: u32, val: i64) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }

    /// Word count of a live record, read from its size header (fld 0).
    /// The header word doubles as DATA for hash bucket records: a
    /// bucket's `room` IS its record size (`hash.rs::add` claims `room`
    /// words), so this is the one legitimate fld-0 read — `valid()`'s
    /// data-field gate (`fld >= 4`) correctly rejects it via the
    /// ordinary accessors.
    #[must_use]
    pub fn record_words(&self, rec: u32) -> u32 {
        debug_assert!(
            self.read_only || self.claims.contains(&rec),
            "Unknown record {rec}"
        );
        let size: i32 = *self.addr(rec, 0);
        debug_assert!(
            size > 0,
            "Freed record {rec} (size={size}) read as a record header"
        );
        size as u32
    }

    /// 4-byte unsigned raw read — for internal collection headers
    /// (vector `[rec:u32][len:u32]`, hash buckets, tree node ptrs,
    /// string-record lengths).  NOT for user `integer` fields.
    #[inline]
    pub fn get_u32_raw(&self, rec: u32, fld: u32) -> u32 {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            0
        }
    }

    /// 4-byte unsigned raw write — counterpart of [`get_u32_raw`].
    #[inline]
    pub fn set_u32_raw(&mut self, rec: u32, fld: u32, val: u32) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }

    /// 4-byte signed raw read — for internal type tags and sentinel
    /// comparisons that must stay `i32::MIN`-relative (e.g. `File.ref`).
    #[inline]
    pub fn get_i32_raw(&self, rec: u32, fld: u32) -> i32 {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            i32::MIN
        }
    }

    /// 4-byte signed raw write — counterpart of [`get_i32_raw`].
    #[inline]
    pub fn set_i32_raw(&mut self, rec: u32, fld: u32, val: i32) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn get_long(&self, rec: u32, fld: u32) -> i64 {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            i64::MIN
        }
    }

    #[inline]
    pub fn set_long(&mut self, rec: u32, fld: u32, val: i64) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn get_short(&self, rec: u32, fld: u32, min: i32) -> i32 {
        if rec != 0 && self.valid(rec, fld) {
            let read: u16 = *self.addr(rec, fld);
            if read != 0 {
                i32::from(read) + min - 1
            } else {
                i32::MIN
            }
        } else {
            i32::MIN
        }
    }

    #[inline]
    pub fn set_short(&mut self, rec: u32, fld: u32, min: i32, val: i32) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            if val == i32::MIN {
                *self.addr_mut(rec, fld) = 0;
                true
            } else if val >= min && val <= min + 65536 {
                *self.addr_mut(rec, fld) = (val - min + 1) as u16;
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Read a `Parts::ShortRaw` narrow-vector element.  ShortRaw is ALWAYS
    /// non-nullable (`narrow_vector_content` / `vectors.rs` build it with
    /// `nullable = false`), so it reserves NO sentinel — the full 2-byte range,
    /// identical to [`get_short_full`](Self::get_short_full) /
    /// [`get_byte`](Self::get_byte) (`read + min`; stored as `(val - min) as u16`
    /// by `set_i16_raw`, the raw-byte-copy path `vector_add` relies on).
    ///
    /// H6: it used to decode `u16::MAX → null`, which silently nulled a
    /// `vector<u16>`'s `65535` / `vector<i16>`'s `32767` — inconsistent with
    /// `vector<u8>` holding the full `0..=255` via `Byte`.  The write twin
    /// `set_i16_raw` keeps its `i32::MIN → u16::MAX` clamp purely as an underflow
    /// guard; narrow vectors store concrete values, never null (like `vector<u8>`).
    #[inline]
    pub fn get_i16_raw(&self, rec: u32, fld: u32, min: i32) -> i32 {
        self.get_short_full(rec, fld, min)
    }

    /// direct 2-byte write for `Parts::ShortRaw` vector
    /// elements.  Mirrors `set_byte`: stores `(val - min) as u16`.
    /// `val == i32::MIN` stores as `u16::MAX` (null sentinel).
    #[inline]
    pub fn set_i16_raw(&mut self, rec: u32, fld: u32, min: i32, val: i32) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            let v: u16 = if val == i32::MIN {
                u16::MAX
            } else {
                (val - min) as u16
            };
            *self.addr_mut(rec, fld) = v;
            true
        } else {
            false
        }
    }

    /// Direct 2-byte read for a NOT-NULL `u16`/`i16` field (`Parts::ShortFull`):
    /// the full 65536-value range, NO null sentinel — `read + min`
    /// unconditionally, the 2-byte twin of [`get_byte`](Self::get_byte).  Unlike
    /// `get_short` (`+1` encoding, reserves `0`, swallows `65535` as null), this
    /// reserves nothing, so a not-null `u16` can hold `65535`.  The `ShortRaw`
    /// narrow-vector read `get_i16_raw` now delegates here (same full-range
    /// decode).  The write reuses `set_i16_raw` (`OpSetShortRaw`): `(val - min)
    /// as u16`, matching this decode.
    #[inline]
    pub fn get_short_full(&self, rec: u32, fld: u32, min: i32) -> i32 {
        if rec != 0 && self.valid(rec, fld) {
            let read: u16 = *self.addr(rec, fld);
            i32::from(read) + min
        } else {
            i32::MIN
        }
    }

    #[inline]
    pub fn get_byte(&self, rec: u32, fld: u32, min: i32) -> i32 {
        if rec != 0 && self.valid(rec, fld) {
            let read: u8 = *self.addr(rec, fld);
            i32::from(read) + min
        } else {
            i32::MIN
        }
    }

    #[inline]
    pub fn set_byte(&mut self, rec: u32, fld: u32, min: i32, val: i32) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            if val == i32::MIN {
                *self.addr_mut(rec, fld) = 255;
                true
            } else if val >= min && val <= min + 256 {
                *self.addr_mut(rec, fld) = (val - min) as u8;
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    #[inline]
    pub fn get_str<'a>(&self, rec: u32) -> &'a str {
        if rec == 0 || rec > i32::MAX as u32 {
            return crate::state::STRING_NULL;
        }
        let len = self.get_u32_raw(rec, 4);
        if (len / 8) + rec > self.size {
            return crate::state::STRING_NULL;
        }
        unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                self.ptr.offset(rec as isize * 8 + 8),
                len as usize,
            ))
        }
    }

    #[inline]
    pub fn set_str(&mut self, val: &str) -> u32 {
        let res = self.claim(((val.len() + 15) / 8) as u32);
        self.set_u32_raw(res, 4, val.len() as u32);
        unsafe {
            std::ptr::copy_nonoverlapping(
                val.as_ptr(),
                self.ptr.offset(res as isize * 8 + 8),
                val.len(),
            );
        }
        res
    }

    #[inline]
    pub fn set_str_ptr(&mut self, ptr: *const u8, len: usize) -> u32 {
        let res = self.claim(((len + 15) / 8) as u32);
        self.set_u32_raw(res, 4, len as u32);
        unsafe {
            std::ptr::copy_nonoverlapping(ptr, self.ptr.offset(res as isize * 8 + 8), len);
        }
        res
    }

    #[inline]
    pub fn append_str(&mut self, record: u32, val: &str) -> u32 {
        let prev = self.get_u32_raw(record, 4);
        let result = self.resize(record, (prev as usize + val.len()).div_ceil(8) as u32);
        unsafe {
            std::ptr::copy_nonoverlapping(
                val.as_ptr(),
                self.ptr.offset(result as isize * 8 + 8 + prev as isize),
                val.len(),
            );
        }
        result
    }

    #[inline]
    pub fn get_boolean(&self, rec: u32, fld: u32, mask: u8) -> bool {
        if self.valid(rec, fld) {
            let read: u8 = *self.addr(rec, fld);
            (read & mask) > 0
        } else {
            false
        }
    }

    #[inline]
    pub fn set_boolean(&mut self, rec: u32, fld: u32, mask: u8, val: bool) -> bool {
        if self.valid(rec, fld) {
            let current: u8 = *self.addr(rec, fld);
            let mut write = current & !mask;
            if val {
                write |= mask;
            }
            *self.addr_mut(rec, fld) = write;
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn get_float(&self, rec: u32, fld: u32) -> f64 {
        // @P284 — guard `rec != 0` like the integer getters do.  In release
        // mode `valid()` is a no-op (all the inner checks are `debug_assert`)
        // so without this guard a null DbRef (rec=0) reads `*self.addr(0, 0)`
        // — the store's free-list header bytes interpreted as f64.  That
        // garbage value (~2.8e-282 in practice) is finite, so for-loop
        // iteration over `vector<float>` saw a non-null value past the end
        // and looped forever.
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            f64::NAN
        }
    }

    #[inline]
    pub fn set_float(&mut self, rec: u32, fld: u32, val: f64) -> bool {
        if self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn get_single(&self, rec: u32, fld: u32) -> f32 {
        // @P284 — sibling of `get_float`'s rec=0 guard; needed for
        // `for s in vector<single>` to terminate.
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            f32::NAN
        }
    }

    #[inline]
    pub fn set_single(&mut self, rec: u32, fld: u32, val: f32) -> bool {
        if self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }
}

// Safety: worker threads only call `addr()` (read-only) on locked stores.
// `addr_mut()` on a locked store always panics.
unsafe impl Send for Store {}

// =============================================================================
// @PLAN38 — durable-store support (phases 00 + 01 — IntegrityOnly tier).
//
// Design: a 40-byte `.dmeta` sidecar file alongside the main store file holds
// signature + tier + CRCs.  The main store file is bit-for-bit identical to
// a legacy (non-durable) store — durability is a metadata layer, not a
// payload-layout change.  See
// `doc/claude/plans/43-loft-store-durable/00-foundation.md`.
//
// Phase-01 reach: nothing inside the `loft` binary calls these yet (the
// training-port consumer that drives @PLAN38 lives in another repo and
// will reach them via a future loft-language builtin).  Tests under
// `tests/store_durable_*.rs` exercise them.  `#[allow(dead_code)]` markers
// on each top-level item keep the bin-side dead-code lint quiet until a
// runtime caller lands; remove them when the first in-tree caller wires up.
// =============================================================================

/// Sidecar signature bytes — `"DStoreV1"` (no NUL terminator).
#[allow(dead_code)]
const DURABLE_SIGNATURE: [u8; 8] = *b"DStoreV1";

/// Total size of a sidecar file in bytes.  Fixed; never grows.
#[allow(dead_code)]
const DURABLE_SIDECAR_BYTES: usize = 40;

/// Tier IDs.  Tier 0 means "not a durable store" and should never appear in a
/// sidecar; it exists so the field has a meaningful default in non-durable
/// `Store` instances.
#[allow(dead_code)]
const TIER_NONE: u16 = 0;
#[allow(dead_code)]
const TIER_INTEGRITY_ONLY: u16 = 1;
// const TIER_SNAPSHOT_EVERY: u16 = 2; // reserved — phase 02
// const TIER_WAL: u16 = 3;            // reserved — phase 03

/// On-disk format detected for a store path.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreFormat {
    /// Legacy non-durable store — main file has no `.dmeta` sidecar.
    /// `Store::open` is the right entry point.
    Legacy,
    /// Durable store — `.dmeta` sidecar is present.  The `u16` is the
    /// `tier_id` recorded in the sidecar (1 = IntegrityOnly, etc.).
    Durable(u16),
}

/// Integrity verdict for a durable store, returned by [`Store::validate_integrity`].
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreIntegrity {
    /// All checks passed; the main file matches the sidecar exactly.
    Clean,
    /// At least one check failed; the consumer must rebuild.  The
    /// `CorruptReason` says which check tripped first (in priority order).
    Corrupt(CorruptReason),
}

/// Reason a durable store failed integrity validation.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorruptReason {
    /// Sidecar signature bytes were not `"DStoreV1"`.
    SignatureMismatch,
    /// Sidecar's own header_crc did not match a recomputed CRC of bytes 0..12.
    HeaderCrcMismatch,
    /// Sidecar file does not exist.  Semantic: clean-close protocol never
    /// ran, so the on-disk payload state is unknown.  Treat as corruption.
    TailMarkerMissing,
    /// Sidecar's `payload_crc` did not match the recomputed CRC of the
    /// main file's bytes (within the recorded `payload_len`).
    TailCrcMismatch,
    /// Main file's actual on-disk byte length differs from sidecar's
    /// `payload_len`.  Either the file was truncated after the sidecar
    /// was written, or the sidecar predates a resize that never completed.
    TruncatedFile,
}

/// Durability mode for [`Store::open_durable`].  Tier 1 is the only variant
/// shipped in phase 01; tiers 2 and 3 land in later phases.
#[allow(dead_code)]
pub enum DurabilityMode {
    /// **Tier 1 — IntegrityOnly.**  No msync discipline on the hot write
    /// path; the OS page cache is trusted for in-flight writes.  On open,
    /// the sidecar is validated; on corruption (or when the file/sidecar
    /// is absent), `on_corruption` is invoked and is expected to rebuild
    /// the store from authoritative sources.  After the callback returns
    /// successfully, `open_durable` retries once.  Cap recursion depth at 1
    /// — if validation still fails after the rebuild, the error is returned
    /// to the caller (no infinite loop).
    ///
    /// Fresh-file note: when the main file does not exist yet (brand-new
    /// database), the same callback fires with `Corrupt(TailMarkerMissing)`.
    /// Consumers MUST implement `on_corruption` as a "rebuild OR initialise"
    /// routine, not a "repair existing file" one.
    IntegrityOnly {
        on_corruption: Box<dyn Fn(&std::path::Path) -> std::io::Result<()>>,
    },
}

/// Decoded sidecar contents.  Module-private; consumers see the public
/// [`StoreIntegrity`] verdict only.
#[allow(dead_code)]
struct SidecarHeader {
    tier_id: u16,
    #[allow(dead_code)] // reserved for future tier-specific behaviour
    flags: u16,
    #[allow(dead_code)] // surfaced for diagnostics; not load-bearing yet
    last_clean_ns: u64,
    payload_len: u64,
    payload_crc: u32,
}

/// Compute the CRC32 of the first 12 bytes of a sidecar.  Used both to
/// write a fresh sidecar and to validate one read off disk.
#[allow(dead_code)]
fn compute_header_crc(bytes: &[u8]) -> u32 {
    debug_assert!(bytes.len() >= 12);
    let mut h = crc32fast::Hasher::new();
    h.update(&bytes[..12]);
    h.finalize()
}

/// Read + parse a sidecar.  Returns `Ok(None)` if the file does not exist,
/// `Ok(Some(...))` if it does and the structural read succeeded.  CRC and
/// signature checks are the caller's job (`detect_format` and
/// `validate_integrity` differ on which checks they apply).
#[allow(dead_code)]
fn read_sidecar(meta_path: &std::path::Path) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::read(meta_path) {
        Ok(bytes) => {
            if bytes.len() != DURABLE_SIDECAR_BYTES {
                // A short/long sidecar is structurally invalid.  Treat it as
                // "present but unreadable" — let the caller decide the verdict.
                return Ok(Some(bytes));
            }
            Ok(Some(bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Parse a sidecar byte buffer.  Returns `Err(CorruptReason::...)` if any
/// of: signature, header_crc, or structural length checks fail.  Callers
/// add further checks (payload_crc, payload_len) by comparing against the
/// main file.
#[allow(dead_code)]
fn parse_sidecar(bytes: &[u8]) -> Result<SidecarHeader, CorruptReason> {
    if bytes.len() != DURABLE_SIDECAR_BYTES {
        return Err(CorruptReason::SignatureMismatch);
    }
    if bytes[..8] != DURABLE_SIGNATURE {
        return Err(CorruptReason::SignatureMismatch);
    }
    let tier_id = u16::from_le_bytes([bytes[8], bytes[9]]);
    let flags = u16::from_le_bytes([bytes[10], bytes[11]]);
    let stored_header_crc = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    if stored_header_crc != compute_header_crc(bytes) {
        return Err(CorruptReason::HeaderCrcMismatch);
    }
    let last_clean_ns = u64::from_le_bytes([
        bytes[16], bytes[17], bytes[18], bytes[19], bytes[20], bytes[21], bytes[22], bytes[23],
    ]);
    let payload_len = u64::from_le_bytes([
        bytes[24], bytes[25], bytes[26], bytes[27], bytes[28], bytes[29], bytes[30], bytes[31],
    ]);
    let payload_crc = u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]);
    // bytes[36..40] is reserved; not validated.
    Ok(SidecarHeader {
        tier_id,
        flags,
        last_clean_ns,
        payload_len,
        payload_crc,
    })
}

/// Build a fresh sidecar buffer for a clean-close write.  `payload_len` is
/// the byte length of the main file at the moment of capture, and
/// `payload_crc` is the CRC over those bytes.
#[allow(dead_code)]
fn encode_sidecar(
    tier_id: u16,
    last_clean_ns: u64,
    payload_len: u64,
    payload_crc: u32,
) -> [u8; DURABLE_SIDECAR_BYTES] {
    let mut buf = [0u8; DURABLE_SIDECAR_BYTES];
    buf[..8].copy_from_slice(&DURABLE_SIGNATURE);
    buf[8..10].copy_from_slice(&tier_id.to_le_bytes());
    buf[10..12].copy_from_slice(&0u16.to_le_bytes()); // flags
    let header_crc = compute_header_crc(&buf[..12]);
    buf[12..16].copy_from_slice(&header_crc.to_le_bytes());
    buf[16..24].copy_from_slice(&last_clean_ns.to_le_bytes());
    buf[24..32].copy_from_slice(&payload_len.to_le_bytes());
    buf[32..36].copy_from_slice(&payload_crc.to_le_bytes());
    buf[36..40].copy_from_slice(&0u32.to_le_bytes()); // reserved
    buf
}

/// Compute the CRC32 of the main store file's first `len` bytes.  Used both
/// to write a fresh sidecar (`compute_payload_crc(path, len)` against the
/// freshly-flushed main file) and to validate one (recompute, compare to
/// the sidecar's stored value).
#[allow(dead_code)]
fn compute_payload_crc(main_path: &std::path::Path, len: u64) -> std::io::Result<u32> {
    use std::io::Read;
    let mut f = std::fs::File::open(main_path)?;
    let mut h = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut remaining = len;
    while remaining > 0 {
        let want = std::cmp::min(remaining as usize, buf.len());
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            // File shorter than expected — caller's TruncatedFile check
            // will catch this; here we just stop the hash.
            break;
        }
        h.update(&buf[..n]);
        remaining -= n as u64;
    }
    Ok(h.finalize())
}

/// Write a sidecar atomically: write to `<meta_path>.tmp`, fsync, rename.
/// Cross-platform note: POSIX rename is atomic; Windows ReplaceFile (which
/// `std::fs::rename` uses on Windows) is atomic in the same sense for the
/// destination path.
#[allow(dead_code)]
fn write_sidecar_atomic(
    meta_path: &std::path::Path,
    bytes: &[u8; DURABLE_SIDECAR_BYTES],
) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp = meta_path.to_path_buf();
    let mut name = tmp
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    name.push(".tmp");
    tmp.set_file_name(name);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, meta_path)?;
    Ok(())
}

/// Trace helper for @PLAN38 debugging.  Writes a line to
/// `/tmp/loft-store-durable.trace` with explicit flush — bypasses Rust
/// stderr's pipe-full-buffering behaviour that swallowed eprintln output
/// during the phase-01 verification session.  Gated on env var
/// `LOFT_STORE_DURABLE_TRACE=1`; zero cost when unset (no file open).
#[allow(dead_code)]
fn dur_trace(line: &str) {
    if std::env::var("LOFT_STORE_DURABLE_TRACE").is_err() {
        return;
    }
    // Honour LOFT_STORE_DURABLE_TRACE_FILE if set (helps when /tmp is
    // sandbox-restricted); otherwise use TMPDIR or /tmp.
    let path = std::env::var("LOFT_STORE_DURABLE_TRACE_FILE").unwrap_or_else(|_| {
        let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
        format!("{base}/loft-store-durable.trace")
    });
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{line}");
        let _ = f.flush();
    }
}

/// Path of the metadata sidecar for a given main store path.
/// `tags.store` → `tags.store.dmeta`.
#[allow(dead_code)]
fn dmeta_path(main_path: &std::path::Path) -> std::path::PathBuf {
    let mut p = main_path.as_os_str().to_owned();
    p.push(".dmeta");
    std::path::PathBuf::from(p)
}

#[allow(dead_code)]
impl Store {
    /// **Phase 00.**  Detect the on-disk format for a store at `path`.
    /// Reads only the sidecar; the main file is not opened.
    ///
    /// - Sidecar absent → [`StoreFormat::Legacy`].
    /// - Sidecar present and structurally readable →
    ///   [`StoreFormat::Durable`]`(tier_id)`.  No CRC or signature
    ///   validation is performed here; use [`Store::validate_integrity`]
    ///   for that.
    ///
    /// Cheap (one file read of ≤ 40 bytes).  Suitable to call before
    /// deciding which open path to use.
    pub fn detect_format(path: &std::path::Path) -> std::io::Result<StoreFormat> {
        let meta = dmeta_path(path);
        let raw = read_sidecar(&meta)?;
        match raw {
            None => Ok(StoreFormat::Legacy),
            Some(bytes) => {
                if bytes.len() == DURABLE_SIDECAR_BYTES && bytes[..8] == DURABLE_SIGNATURE {
                    let tier_id = u16::from_le_bytes([bytes[8], bytes[9]]);
                    Ok(StoreFormat::Durable(tier_id))
                } else {
                    // A short/long sidecar or wrong-signature one is a
                    // durable store with damaged metadata.  Reporting
                    // `Durable(0)` would lie about the tier; the cleanest
                    // signal is "this is durable in intent but unreadable"
                    // — surface it via the validate path.  `detect_format`
                    // returns Legacy so callers that ONLY ran detect can
                    // still distinguish "no sidecar" from "broken sidecar"
                    // by calling validate_integrity for the verdict.
                    Ok(StoreFormat::Durable(0))
                }
            }
        }
    }

    /// **Phase 00.**  Validate the integrity of a durable store at `path`.
    ///
    /// Performs:
    /// 1. Sidecar exists → else [`CorruptReason::TailMarkerMissing`].
    /// 2. Sidecar signature is `"DStoreV1"` →
    ///    else [`CorruptReason::SignatureMismatch`].
    /// 3. Sidecar's own `header_crc` matches a recomputed CRC of bytes 0..12
    ///    → else [`CorruptReason::HeaderCrcMismatch`].
    /// 4. Main file's actual byte length matches sidecar's `payload_len`
    ///    → else [`CorruptReason::TruncatedFile`].
    /// 5. CRC32 of the main file's `payload_len` bytes matches the sidecar's
    ///    `payload_crc` → else [`CorruptReason::TailCrcMismatch`].
    ///
    /// Checks short-circuit on the first failure.
    pub fn validate_integrity(path: &std::path::Path) -> std::io::Result<StoreIntegrity> {
        let meta = dmeta_path(path);
        dur_trace(&format!("[validate] entry path={path:?} sidecar={meta:?}"));
        let raw = match read_sidecar(&meta)? {
            None => {
                dur_trace("[validate] sidecar missing → TailMarkerMissing");
                return Ok(StoreIntegrity::Corrupt(CorruptReason::TailMarkerMissing));
            }
            Some(bytes) => bytes,
        };
        dur_trace(&format!("[validate] sidecar read {} bytes", raw.len()));
        let hdr = match parse_sidecar(&raw) {
            Ok(h) => h,
            Err(reason) => {
                dur_trace(&format!("[validate] parse_sidecar → {reason:?}"));
                return Ok(StoreIntegrity::Corrupt(reason));
            }
        };
        dur_trace(&format!(
            "[validate] parsed: tier={} flags={} last_clean_ns={} payload_len={} payload_crc={:#x}",
            hdr.tier_id, hdr.flags, hdr.last_clean_ns, hdr.payload_len, hdr.payload_crc
        ));
        let main_meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                dur_trace("[validate] main file missing → TruncatedFile");
                return Ok(StoreIntegrity::Corrupt(CorruptReason::TruncatedFile));
            }
            Err(e) => return Err(e),
        };
        dur_trace(&format!(
            "[validate] main file len={} (sidecar says {})",
            main_meta.len(),
            hdr.payload_len
        ));
        if main_meta.len() != hdr.payload_len {
            dur_trace("[validate] length mismatch → TruncatedFile");
            return Ok(StoreIntegrity::Corrupt(CorruptReason::TruncatedFile));
        }
        dur_trace(&format!(
            "[validate] computing payload CRC over {} bytes …",
            hdr.payload_len
        ));
        let crc = compute_payload_crc(path, hdr.payload_len)?;
        dur_trace(&format!("[validate] computed CRC = {crc:#x}"));
        if crc != hdr.payload_crc {
            dur_trace("[validate] CRC mismatch → TailCrcMismatch");
            return Ok(StoreIntegrity::Corrupt(CorruptReason::TailCrcMismatch));
        }
        if hdr.tier_id == TIER_NONE {
            dur_trace("[validate] tier_id=0 → SignatureMismatch");
            return Ok(StoreIntegrity::Corrupt(CorruptReason::SignatureMismatch));
        }
        dur_trace("[validate] Clean");
        Ok(StoreIntegrity::Clean)
    }

    /// **Phase 01.**  Open a store with durability guarantees.
    ///
    /// On entry, the sidecar at `<path>.dmeta` is validated.  On any
    /// integrity failure (or when either file is missing), `on_corruption`
    /// is invoked once, then validation is retried.  A second failure
    /// returns `io::Error` (kind `InvalidData`) rather than looping.
    ///
    /// On success, the returned `Store` is wired so its `Drop` impl will
    /// flush the mmap and rewrite the sidecar on clean shutdown.  A
    /// `kill -9` (or any other path that skips `Drop`) leaves the sidecar
    /// stale → the next open detects corruption → callback re-fires.  This
    /// is by design — Tier 1 trusts the OS page cache for in-flight writes
    /// and recovers via rebuild.
    ///
    /// **Do not use Tier 1** for data that cannot be re-derived from
    /// authoritative sources; phases 02 (snapshots) and 03 (WAL) cover
    /// stronger guarantees.
    #[cfg(feature = "mmap")]
    pub fn open_durable(path: &std::path::Path, mode: DurabilityMode) -> std::io::Result<Store> {
        Self::open_durable_inner(path, mode, 0)
    }

    #[cfg(feature = "mmap")]
    fn open_durable_inner(
        path: &std::path::Path,
        mode: DurabilityMode,
        depth: u32,
    ) -> std::io::Result<Store> {
        dur_trace(&format!(
            "[open_durable_inner] entry depth={depth} path={path:?}"
        ));
        let verdict = Self::validate_integrity(path)?;
        dur_trace(&format!(
            "[open_durable_inner] validate_integrity → {verdict:?}"
        ));
        match verdict {
            StoreIntegrity::Clean => {
                let path_str = path.to_str().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "store path is not valid UTF-8",
                    )
                })?;
                dur_trace(&format!(
                    "[open_durable_inner] calling Store::open({path_str:?})"
                ));
                let mut store = Store::open(path_str);
                dur_trace("[open_durable_inner] Store::open returned");
                store.durable_meta_path = Some(path.to_path_buf());
                store.durable_tier = TIER_INTEGRITY_ONLY;
                Ok(store)
            }
            StoreIntegrity::Corrupt(_reason) => {
                if depth >= 1 {
                    dur_trace("[open_durable_inner] recursion cap hit; returning InvalidData");
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "store_durable: rebuild callback ran but store still fails integrity",
                    ));
                }
                let DurabilityMode::IntegrityOnly { ref on_corruption } = mode;
                dur_trace("[open_durable_inner] firing on_corruption callback");
                on_corruption(path)?;
                dur_trace("[open_durable_inner] on_corruption returned ok");
                let has_main = path.exists();
                dur_trace(&format!(
                    "[open_durable_inner] post-callback: main_exists={has_main}"
                ));
                if has_main {
                    // After the callback returns Ok, treat the main file as
                    // the new authoritative state and rewrite the sidecar to
                    // match.  Without this, a corruption that affected ONLY
                    // the sidecar (signature flip, payload_crc tear) would
                    // never reach a Clean state: the callback can't repair a
                    // sidecar from the main file's POV, and the stale
                    // sidecar would survive into the depth-1 recursion and
                    // trigger the recursion cap.  Rewriting unconditionally
                    // covers both "main file rebuilt" and "main file fine,
                    // sidecar damaged" cases with one code path.
                    let meta = dmeta_path(path);
                    if meta.exists() {
                        dur_trace(&format!(
                            "[open_durable_inner] removing stale sidecar {meta:?}"
                        ));
                        std::fs::remove_file(&meta)?;
                    }
                    dur_trace(&format!(
                        "[open_durable_inner] write_initial_sidecar({path:?})"
                    ));
                    Self::write_initial_sidecar(path)?;
                    dur_trace("[open_durable_inner] write_initial_sidecar returned");
                }
                dur_trace(&format!(
                    "[open_durable_inner] recursing to depth={}",
                    depth + 1
                ));
                Self::open_durable_inner(path, mode, depth + 1)
            }
        }
    }

    /// Write a fresh sidecar for a main file that exists but has no
    /// sidecar yet (e.g. the rebuild callback just initialised the file).
    /// **Phase 01b.**  Verdict-bool form of [`Store::validate_integrity`]
    /// for the loft-callable binding.  Returns `true` iff the sidecar
    /// validates cleanly against the main file at `path` (signature,
    /// header CRC, payload length, payload CRC, tier_id all OK).
    ///
    /// Any I/O error or `Corrupt(_)` verdict collapses to `false` —
    /// the loft caller can't act on the distinct `CorruptReason`
    /// variants distinctly (every non-Clean case routes to the same
    /// "rebuild from source" response on their side), so the binding
    /// surfaces a flat bool.
    #[cfg(feature = "mmap")]
    #[must_use]
    pub fn durable_check(path: &std::path::Path) -> bool {
        matches!(Self::validate_integrity(path), Ok(StoreIntegrity::Clean))
    }

    /// **Phase 01b.**  Write a fresh `.dmeta` sidecar capturing the
    /// current main-file's byte length + CRC32 + a clean-close
    /// timestamp.  Returns `true` on success, `false` on any I/O
    /// error (out-of-space, permission denied, parent dir missing,
    /// main file absent).
    ///
    /// This is the loft equivalent of the Rust API's Drop-driven
    /// sidecar write.  The loft caller invokes it explicitly after
    /// finishing a write session; if the program crashes between
    /// the last write and the seal, the sidecar stays stale and the
    /// next `durable_check` returns `false` → rebuild fires.
    ///
    /// Implementation note: this is `write_initial_sidecar` lifted
    /// into a pub function with the result mapped to bool.
    #[cfg(feature = "mmap")]
    #[must_use]
    pub fn durable_seal(path: &std::path::Path) -> bool {
        Self::write_initial_sidecar(path).is_ok()
    }

    /// Captures the current main-file length and CRC at the moment of call.
    #[cfg(feature = "mmap")]
    fn write_initial_sidecar(path: &std::path::Path) -> std::io::Result<()> {
        dur_trace(&format!("[init_sidecar] entry path={path:?}"));
        let main_meta = std::fs::metadata(path)?;
        let payload_len = main_meta.len();
        dur_trace(&format!("[init_sidecar] main file len={payload_len}"));
        let payload_crc = compute_payload_crc(path, payload_len)?;
        dur_trace(&format!("[init_sidecar] payload CRC = {payload_crc:#x}"));
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let bytes = encode_sidecar(TIER_INTEGRITY_ONLY, now_ns, payload_len, payload_crc);
        dur_trace(&format!(
            "[init_sidecar] writing {} bytes to {:?}",
            bytes.len(),
            dmeta_path(path)
        ));
        write_sidecar_atomic(&dmeta_path(path), &bytes)?;
        dur_trace("[init_sidecar] done");
        Ok(())
    }

    /// On clean drop of a durable store, capture the current main-file
    /// CRC and rewrite the sidecar atomically.  Returns `Err` if any
    /// step fails; the Drop impl logs (rather than panics) on failure
    /// since panicking during drop would abort the process.
    #[cfg(feature = "mmap")]
    fn flush_durable_sidecar(&mut self) -> std::io::Result<()> {
        // Step 1: flush the mmap to disk.
        if let Some(file) = &self.file {
            file.flush_sync()?;
        }
        // Step 2: compute payload CRC over the now-flushed main file.
        let path = match &self.durable_meta_path {
            Some(p) => p.clone(),
            None => return Ok(()),
        };
        let main_meta = std::fs::metadata(&path)?;
        let payload_len = main_meta.len();
        let payload_crc = compute_payload_crc(&path, payload_len)?;
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let bytes = encode_sidecar(self.durable_tier, now_ns, payload_len, payload_crc);
        // Step 3: write sidecar atomically (tmp → rename).
        write_sidecar_atomic(&dmeta_path(&path), &bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_STORE_WORDS, Store};

    /// Growing the store through many claims must not wrap or silently fail.
    #[test]
    fn store_grows_without_overflow() {
        let mut store = Store::new(4);
        store.free = false; // mark as in-use so validate() does not reject it
        for _ in 0..200 {
            store.claim(1);
        }
        // Store must have grown to hold 200 single-word claims (≥200 * 8 bytes).
        assert!(store.byte_capacity() >= 200 * 8);
    }

    /// P6: `delete` only coalesces forward, so freeing two blocks in
    /// FORWARD order leaves them as an uncoalesced adjacent pair.  The lazy
    /// `coalesce_free` sweep must merge them (mergeable-pairs → 0) and the
    /// merged block must be reused by the next allocation instead of
    /// growing the store.
    #[test]
    fn coalesce_free_merges_adjacent_and_reuses_space() {
        let mut store = Store::new(64);
        store.free = false;
        let _a = store.claim(5);
        let b = store.claim(5);
        let c = store.claim(5);
        let d = store.claim(5);
        assert!(b < c && c < d, "A,B,C,D are contiguous and ascending");
        // Free B then C (forward order): delete only merges with the NEXT
        // block, so freeing B (C still claimed) then C (D claimed) leaves
        // B|C adjacent-but-unmerged.
        store.delete(b);
        store.delete(c);
        assert!(
            store.usage().mergeable_free_pairs >= 1,
            "forward-order frees leave an uncoalesced adjacent pair"
        );
        // The lazy sweep merges them.
        store.coalesce_free();
        assert_eq!(
            store.usage().mergeable_free_pairs,
            0,
            "coalesce_free merges every adjacent free pair"
        );
        // The merged B+C block (10 words) is reused for a request that
        // neither B(5) nor C(5) could satisfy alone — no store growth.
        let cap_before = store.byte_capacity();
        let reclaimed = store.claim(9);
        assert_eq!(
            store.byte_capacity(),
            cap_before,
            "reused the merged free block instead of growing the store"
        );
        assert!(
            reclaimed >= b && reclaimed < d,
            "reclaimed from the old B/C region (got {reclaimed}, B={b}, D={d})"
        );
    }

    /// `resize_store` must panic when the requested size exceeds `MAX_STORE_WORDS`.
    #[test]
    #[should_panic(expected = "store offset overflow")]
    fn resize_store_exceeds_max_panics() {
        let mut store = Store::new(4);
        store.free = false;
        store.resize_store(MAX_STORE_WORDS + 1);
    }

    /// `addr_mut` on a locked store must panic (not silently discard the write).
    #[test]
    #[should_panic(expected = "Write to read-only store")]
    fn write_to_locked_store_panics() {
        let mut store = Store::new(64);
        let rec = store.claim(8);
        store.lock();
        let _: &mut u8 = store.addr_mut::<u8>(rec, 4);
    }

    /// Borrowed store reads return the same data as the original.
    #[test]
    fn borrow_locked_reads_original_data() {
        let mut store = Store::new(64);
        store.free = false;
        let rec = store.claim(4);
        *store.addr_mut::<i32>(rec, 0) = 42;
        let borrow = unsafe { store.borrow_locked_for_light_worker() };
        assert_eq!(*borrow.addr::<i32>(rec, 0), 42);
        assert!(borrow.read_only);
        assert!(borrow.borrowed);
        // Drop of borrow must NOT free the original's buffer.
        drop(borrow);
        assert_eq!(
            *store.addr::<i32>(rec, 0),
            42,
            "original intact after borrow dropped"
        );
    }

    /// Writing to a borrowed store panics.
    #[test]
    #[should_panic(expected = "Write to read-only store")]
    fn borrow_locked_write_panics() {
        let mut store = Store::new(64);
        store.free = false;
        let rec = store.claim(4);
        let mut borrow = unsafe { store.borrow_locked_for_light_worker() };
        // This must panic because the borrow is locked.
        let _: &mut u8 = borrow.addr_mut::<u8>(rec, 0);
    }

    /// Freed sentinel is a minimal store.
    #[test]
    fn freed_sentinel_is_free() {
        let sentinel = Store::new_freed_sentinel();
        assert!(sentinel.free);
    }

    /// cluster-462 / #462 regression: an in-place `resize` grow that absorbs an adjacent
    /// freed block MUST zero the newly-absorbed region, upholding the same "claimed payload
    /// reads zero" invariant `claim` provides. A freshly-exposed vector element slot that
    /// keeps the freed block's stale bytes (garbage text/vec handles) is followed by
    /// `remove_claims`/`length_vector` into a UAF (the sim.loft:3546 SIGSEGV). Pre-fix this
    /// region kept `0xDEAD_BEEF`; post-fix it reads 0.
    #[test]
    fn resize_in_place_zeroes_absorbed_region() {
        let mut store = Store::new(256);
        store.free = false;
        let a = store.claim(4); // 4-word record
        let b = store.claim(16); // adjacent 16-word record
        // Garbage at HIGH offsets in b, past the free-tree node header `delete` writes into
        // b's first words — so it survives the free and is what `resize` must clear.
        *store.addr_mut::<u32>(b, 80) = 0xDEAD_BEEF;
        *store.addr_mut::<u32>(b, 100) = 0x00CA_FE00;
        store.delete(b); // b becomes a free block adjacent to a
        let a2 = store.resize(a, 12); // grow a in place into b's region
        assert_eq!(
            a2, a,
            "resize should grow a in place (absorb the adjacent free block)"
        );
        // b started at word 4 relative to a; b byte 80/100 -> a byte 4*8+80 / 4*8+100.
        assert_eq!(
            *store.addr::<u32>(a, 32 + 80),
            0,
            "absorbed region must be zeroed (kept 0xDEADBEEF pre-fix)"
        );
        assert_eq!(
            *store.addr::<u32>(a, 32 + 100),
            0,
            "absorbed region must be zeroed (kept 0xCAFE pre-fix)"
        );
    }
}
