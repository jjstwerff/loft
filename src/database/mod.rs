// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Database operations on stores
#![allow(dead_code)]

mod allocation;
mod format;
mod io;
mod search;
mod structures;
mod types;

pub use types::Type;

/// Store index reserved for compile-time constant data (vectors, long strings).
/// Always allocated during `State::new()`, locked before execution begins.
/// See `doc/claude/CONST_STORE.md` for the full design.
pub const CONST_STORE: u16 = 1;

use crate::keys::{Content, DbRef};
use crate::store::Store;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter, Write as _};
use std::sync::{Arc, Mutex};
// the `--html` build compiles for wasm32-unknown-unknown
// WITHOUT the `wasm` feature (the feature carries wasm-bindgen
// host bridges that `--html`'s hand-rolled JS runtime does not
// provide).  That leaves `std::time::Instant` on a target with no
// time source — calling `Instant::now()` panics, and the panic
// compiles to `(unreachable)` which was the root of every
// `--html loft_start` trap.  Use Instant only on non-wasm32
// targets; wasm32 (with or without the feature) tracks time in
// milliseconds through the host bridge.
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// Type alias for a native function callable from loft bytecode.
pub type Call = fn(&mut Stores, &mut DbRef);

/// Context injected into `Stores` by `State::execute()` so that native
/// functions such as `n_parallel_for_int` can access the interpreter's
/// bytecode, text segment, library, and compiled data for spawning workers.
///
/// All raw pointers are valid for the duration of the `execute()` call
/// that set them.
pub struct ParallelCtx {
    pub bytecode: *const Arc<Vec<u8>>,
    pub library: *const Arc<Vec<Call>>,
    pub data: *const crate::data::Data,
    /// Cached library index of `n_stack_trace`; `u16::MAX` = not found.
    /// Copied into worker `State::stack_trace_lib_nr` so workers can snapshot
    /// the call stack when `stack_trace()` is called (fix #92).
    pub stack_trace_lib_nr: u16,
}

// Safety: the pointed-to data lives for the duration of `State::execute()`,
// which is on the main thread and outlives all worker threads it spawns
// (workers are joined before execute() returns).
unsafe impl Send for ParallelCtx {}
unsafe impl Sync for ParallelCtx {}

/// TR1.4: snapshot of one local variable's runtime value, captured by
/// `State::static_call` for inclusion in a `StackFrame.variables` vector.
/// All fields are owned values — no raw pointers — so the snapshot is safe
/// to retain across native function boundaries.
#[derive(Debug, Clone)]
pub struct VarSnapshot {
    pub name: String,
    pub type_name: String,
    pub value: VarValueSnapshot,
}

/// Owned snapshot of a variable's typed runtime value.  Mirrors the loft
/// `ArgValue` enum so the native can populate `VarInfo.value` directly.
#[derive(Debug, Clone)]
pub enum VarValueSnapshot {
    Null,
    Bool(bool),
    Int(i32),
    Long(i64),
    Float(f64),
    Single(f32),
    Char(char),
    Text(String),
    Ref { store: i32, rec: i32, pos: i32 },
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    /// Known-type number of the field's value type — needed by
    /// runtime struct-schema walkers (e.g. `n_struct_from_jsonvalue`)
    /// that iterate `Parts::Struct(_)`.
    pub content: u16,
    pub position: u16,
    pub default: Content,
    pub(self) other_indexes: Vec<u16>, // For now only fields on the same record
}

#[derive(Debug, Clone, PartialEq)]
pub enum Parts {
    Base,                              // One of the simple base types or text.
    Struct(Vec<Field>),                // The fields of this record.
    Enum(Vec<(u16, String)>),          // Enumerate type with possible values.
    EnumValue(u8, Vec<Field>),         // Enumerate value with actual value for typed structures.
    Byte(i32, bool),                   // start number and nullable flag
    Short(i32, bool),                  // start number and nullable flag
    Int(i32, bool), // 4-byte integer field (size(4) annotation). Null sentinel: i32::MIN.
    ShortRaw(i32, bool), // P184 Phase 4b: 2-byte narrow vector element. Direct encoding (no +1 shift). Null sentinel: i16::MIN.
    Vector(u16),         // The records are part of the vector
    Array(u16),          // The array holds references for each record
    Sorted(u16, Vec<(u16, bool)>), // Sorted vector on fields with an ascending flag
    Ordered(u16, Vec<(u16, bool)>), // Sorted array on fields with an ascending flag
    Hash(u16, Vec<u16>), // A hash table, listing the field numbers that define its key
    Index(u16, Vec<(u16, bool)>, u16), // An index to a table, listing the key fields and the left field-nr
    Spacial(u16, Vec<u16>),            // A spacial index with the listed coordinate fields as a key
}

impl PartialEq for Content {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Content::Long(l), Content::Long(r)) => l == r,
            (Content::Float(l), Content::Float(r)) => l == r,
            (Content::Single(l), Content::Single(r)) => l == r,
            (Content::Str(s), Content::Str(o)) => s.str() == o.str(),
            _ => false,
        }
    }
}

impl Debug for Content {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Content::Long(l) => f.write_fmt(format_args!("Long({l})"))?,
            Content::Float(v) => f.write_fmt(format_args!("Float({v})"))?,
            Content::Single(s) => f.write_fmt(format_args!("Single({s})"))?,
            Content::Str(t) => {
                f.write_char('"')?;
                f.write_str(t.str())?;
                f.write_char('"')?;
            }
        }
        Ok(())
    }
}

#[allow(clippy::struct_excessive_bools)]
pub struct Stores {
    pub types: Vec<Type>,
    pub names: HashMap<String, u16>,
    pub allocations: Vec<Store>,
    #[cfg(not(feature = "wasm"))]
    pub files: Vec<Option<std::fs::File>>,
    #[cfg(feature = "wasm")]
    pub files: Vec<()>,
    pub max: u16,
    /// S29: bitmap of free store slots — bit `i` is set when `allocations[i]`
    /// is free and eligible for reuse.  `database_named` finds the lowest set bit below `max`
    /// and reuses that slot instead of always growing `max`.  This eliminates the LIFO-order
    /// requirement on `free()` that the old cascade-based scan imposed.
    pub free_bits: Vec<u64>,
    /// Temporary strings produced by text-returning native functions.
    /// Cleared by `OpClearScratch` at statement boundaries.
    pub scratch: Vec<String>,
    /// per-definition DbRef into the CONST_STORE for vector
    /// constants (e.g. `pub HEIGHT_STEP_LABELS: vector<text> = […]`).
    /// Indexed by `d_nr`; a null DbRef (store_nr = u16::MAX) means
    /// that definition isn't a constant.  Populated by
    /// `compile::build_const_vectors` (interpreter path) or by the
    /// `init()` function emitted by `src/generation/` (native path).
    /// Mirrors `State.const_refs` so the native codegen's substitution
    /// `s.const_refs` → `stores.const_refs` works from any function
    /// context that only has `&mut Stores` (which is every native
    /// function — native code doesn't carry a State reference).
    pub const_refs: Vec<DbRef>,
    /// Errors from the last `Type.parse()` call, read via `s#errors`.
    pub last_parse_errors: Vec<String>,
    /// errors from the last `json_parse()` call, read via
    /// `json_errors()`.  Cleared on every successful `json_parse`;
    /// populated with `format!("{msg} (byte {pos})")` on parse failure.
    pub last_json_errors: Vec<String>,
    /// Set by `State::execute()` to allow native functions to access the
    /// interpreter's bytecode, library, and compiled data during execution.
    pub parallel_ctx: Option<Box<ParallelCtx>>,
    /// Shared runtime logger.  Set by `main.rs` after the State is created.
    /// Cloned (Arc clone) into worker Stores so all threads share a single logger.
    pub logger: Option<Arc<Mutex<crate::logger::Logger>>>,
    /// Set to `true` when a loft `panic()` or failed `assert` fires in production mode
    /// (where the error is logged instead of aborting).  `main.rs` checks this after
    /// execution and exits with code 1 so shell scripts can detect failure.
    pub had_fatal: bool,
    /// Directory of the main source file being executed.
    /// Set by `main.rs` after parsing; used by `source_dir()` built-in.
    pub source_dir: String,
    /// FY.1: When true, the interpreter loop yields back to the caller.
    /// Set by `gl_swap_buffers` in WASM mode; cleared by `resume_frame`.
    pub frame_yield: bool,
    /// When true, `free_named` overwrites the freed store's buffer with a
    /// poison pattern (`0xDEADBEEF` i32 words) so subsequent reads through a
    /// stale DbRef hit recognisable garbage instead of whatever bytes the
    /// allocator leaves.  Enabled by `LOFT_LOG=poison_free` via
    /// `execute_log_impl` (or anywhere else that wires it).
    pub poison_free: bool,
    /// When true, assert() reports results (pass/fail) to `assert_results`
    /// instead of panicking on failure.  Used by the WASM playground.
    pub report_asserts: bool,
    /// Structured assert results: (passed, message, file, line).
    pub assert_results: Vec<(bool, String, String, u32)>,
    /// Script-level arguments (set by the CLI after parsing its own flags).
    /// When non-empty, `os_arguments()` returns these instead of raw `std::env::args`.
    pub user_args: Vec<String>,
    /// Monotonic timestamp captured at `Stores::new()`.  Used by `ticks()` to return
    /// microseconds elapsed since program start; cloned into worker Stores unchanged so
    /// all threads share the same reference point.
    #[cfg(not(target_arch = "wasm32"))]
    pub start_time: Instant,
    /// Under any wasm32 target (both the `wasm` feature's host-bridge
    /// build and the `--html` no-feature build): milliseconds since
    /// Unix epoch at program start.  `n_ticks` uses this plus the
    /// host-imported `time_ticks` to compute elapsed time without
    /// `std::time::Instant`.  Instant is unavailable on wasm32 (for
    /// either feature variant), so we snapshot elapsed ms here.
    #[cfg(target_arch = "wasm32")]
    pub start_time_ms: i64,
    /// TR1.3: snapshot of (`fn_name`, file, line) for each call frame.
    /// Populated by `State::static_call` when `n_stack_trace` is invoked.
    pub call_stack_snapshot: Vec<(String, String, u32)>,
    /// TR1.4: per-frame variable snapshot.  Outer Vec is parallel to
    /// `call_stack_snapshot` (one entry per frame); inner Vec is the live
    /// variables in that frame as `(name, type_name, ArgValueSnapshot)`.
    /// Populated alongside `call_stack_snapshot` in `State::static_call`.
    pub variables_snapshot: Vec<Vec<VarSnapshot>>,
    /// Native-code closure store. Maps lambda d_nr → closure DbRef.
    /// Set by `OpStoreClosure` (native) immediately before calling the lambda;
    /// read by `OpGetClosure` in the match-dispatch arm.
    pub closure_map: HashMap<u32, DbRef>,
    /// Shared `JsonValue::JNull` sentinel record for `n_field` / `n_item`
    /// fallback paths.  Lazily allocated on first use (after JsonValue's
    /// `known_type` has been registered), kept for the process lifetime —
    /// its containing store is flagged `free = false` so `check_store_leaks`
    /// ignores it.
    pub jnull_sentinel: Option<DbRef>,
}

impl Default for Stores {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Stores {
    /// Clone the type-schema portion of a `Stores`.
    /// Runtime-only fields (`allocations`, `files`, `parallel_ctx`)
    /// are reset to empty/None because they are only valid during execution.
    fn clone(&self) -> Self {
        Self {
            types: self.types.clone(),
            names: self.names.clone(),
            allocations: Vec::new(),
            files: Vec::new(),
            max: self.max,
            free_bits: Vec::new(),
            scratch: Vec::new(),
            const_refs: Vec::new(),
            last_parse_errors: Vec::new(),
            last_json_errors: Vec::new(),
            parallel_ctx: None,
            logger: self.logger.clone(),
            had_fatal: false,
            source_dir: String::new(),
            frame_yield: false,
            poison_free: self.poison_free,
            report_asserts: false,
            assert_results: Vec::new(),
            user_args: self.user_args.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            start_time: self.start_time,
            #[cfg(target_arch = "wasm32")]
            start_time_ms: self.start_time_ms,
            call_stack_snapshot: Vec::new(),
            variables_snapshot: Vec::new(),
            closure_map: HashMap::new(),
            jnull_sentinel: None,
        }
    }
}

// Safety: `Content::Str` raw pointers in type metadata point into parse-time
// source strings that live for the program duration and are never mutated.
// Workers only read this metadata.  `Store` is already `unsafe impl Send`.
// `Sync` is additionally required so that `OnceLock<(Data, Stores)>` can be
// used as a process-wide static; the same invariant (read-only after parse)
// makes concurrent shared access safe.
unsafe impl Send for Stores {}
unsafe impl Sync for Stores {}

/// Type-level proof that a [`Stores`] was produced by [`Stores::clone_for_worker`]
/// and belongs to exactly one worker thread.
///
/// `WorkerStores` is `Send` (movable to a worker thread) but intentionally not
/// `Sync` (cannot be shared across threads).  The `PhantomData<*mut ()>` field
/// suppresses the auto-derived `Sync` implementation; the explicit `Send`
/// implementation restores send-ability.  This ensures that passing a worker
/// snapshot to `State::new_worker` at the call site is a compile-time guarantee
/// rather than a runtime convention.
pub struct WorkerStores {
    pub(crate) stores: Stores,
    _not_sync: std::marker::PhantomData<*mut ()>,
}

// SAFETY: each worker thread receives exclusive ownership of its WorkerStores.
// The inner Stores is a locked snapshot of main-thread data; workers never
// access the main thread's mutable state through this value.
unsafe impl Send for WorkerStores {}

impl WorkerStores {
    pub(crate) fn new(stores: Stores) -> Self {
        WorkerStores {
            stores,
            _not_sync: std::marker::PhantomData,
        }
    }
}

impl std::ops::Deref for WorkerStores {
    type Target = Stores;
    fn deref(&self) -> &Stores {
        &self.stores
    }
}

impl std::ops::DerefMut for WorkerStores {
    fn deref_mut(&mut self) -> &mut Stores {
        &mut self.stores
    }
}

/// Plan-06 phase 1 — marker telling the parent which slot in this
/// worker's `WorkerStores.allocations` to extract after join.
///
/// The output slot is a regular `Store` inside the worker's allocations
/// table, written via ordinary `OpSet*` opcodes addressed by a normal
/// `DbRef`.  After the worker thread joins, the parent calls
/// `WorkerStores::take_slot(slot.store_nr)` to extract the inner Store
/// and `Stores::adopt_store(store)` to install it into the parent's
/// allocations.  See plan-06 DESIGN.md D2.1 for the rationale.
///
/// Just a `u16`; no Drop logic.  The worker's `WorkerStores` owns the
/// underlying Store until `take_slot` extracts it.  If the worker
/// panics, the `WorkerStores` is dropped and the slot's Store is
/// freed via `Store::Drop`.
#[derive(Debug, Clone, Copy)]
pub struct WorkerOutputSlot {
    pub store_nr: u16,
}

impl WorkerStores {
    /// Append a fresh empty Store to `allocations` and return the
    /// new slot's index as a `WorkerOutputSlot` marker.
    ///
    /// Called by the parallel dispatcher right after `clone_for_worker`,
    /// before handing the `WorkerStores` to the worker thread.  The
    /// worker writes its result into the slot via ordinary `OpSet*`
    /// opcodes addressed by a `DbRef { store_nr: slot.store_nr, .. }`.
    ///
    /// `slot_words` is the requested capacity in 8-byte words; the
    /// minimum is one word so the underlying allocator never sees zero.
    pub fn add_output_slot(&mut self, slot_words: u32) -> WorkerOutputSlot {
        let store_nr = self.stores.allocations.len() as u16;
        let mut store = Store::new(slot_words.max(1));
        // Worker output slots are writable (free=true→false handled by
        // the worker's first claim).  Mark non-free so debug invariants
        // don't think this is a freed slot.
        store.free = false;
        store.ref_count = 1;
        self.stores.allocations.push(store);
        if store_nr >= self.stores.max {
            self.stores.max = store_nr + 1;
        }
        WorkerOutputSlot { store_nr }
    }

    /// Move the inner `Store` out of the slot, replacing it with a
    /// freed sentinel so the worker's `Drop` is a no-op for the
    /// extracted slot.
    ///
    /// Called by the parent thread after the worker joins.  The
    /// returned `Store` retains its bytes — installation into the
    /// parent's allocations table happens via `Stores::adopt_store`.
    ///
    /// # Panics
    /// Panics if `slot_nr` is out of range or has already been taken
    /// (sentinel-replaced) — both indicate dispatcher bugs.
    pub fn take_slot(&mut self, slot_nr: u16) -> Store {
        let pos = slot_nr as usize;
        assert!(
            pos < self.stores.allocations.len(),
            "take_slot: slot {slot_nr} out of range",
        );
        let sentinel = crate::store::Store::new_freed_sentinel();
        std::mem::replace(&mut self.stores.allocations[pos], sentinel)
    }

    /// Plan-06 phase 2 — extract every worker-allocated Store
    /// (those at index ≥ `parent_store_count`) for adoption by the
    /// parent.  Returns `(worker_local_store_nr, Store)` pairs in
    /// ascending store_nr order.
    ///
    /// Each returned slot is sentinel-replaced in the worker's
    /// allocations table, so the worker's `Drop` won't double-free
    /// adopted stores.  Slots that were freed by the worker during
    /// execution (free=true) are skipped.
    ///
    /// `parent_store_count` is the number of stores the parent had
    /// before the worker ran — these are clones of parent stores
    /// (the worker only read them) and must NOT be adopted.
    ///
    /// Used by the parent's stitch logic in conjunction with
    /// `Stores::adopt_store` and the `StoreRebase` rebase map (see
    /// `src/parallel.rs::StoreRebase`).
    pub fn take_all_owned(&mut self, parent_store_count: u16) -> Vec<(u16, Store)> {
        let mut out = Vec::new();
        let total = self.stores.allocations.len();
        for nr in (parent_store_count as usize)..total {
            if self.stores.allocations[nr].free {
                continue;
            }
            let sentinel = crate::store::Store::new_freed_sentinel();
            let s = std::mem::replace(&mut self.stores.allocations[nr], sentinel);
            out.push((nr as u16, s));
        }
        out
    }
}

impl Stores {
    /// Install an externally-allocated `Store` into this `Stores`'
    /// allocations table.  Returns the parent-side `store_nr`.
    ///
    /// Used by the parent thread after `WorkerStores::take_slot`
    /// extracts a worker's output slot.  The `Store` keeps its bytes
    /// — no memcpy, no claim translation.  Phase 2's rebase walk
    /// rewrites cross-store DbRefs after every worker's slot is
    /// adopted.
    ///
    /// Reuses a free slot if one is available below `max`; otherwise
    /// pushes a new slot at the end.
    pub fn adopt_store(&mut self, store: Store) -> u16 {
        // Inline the free-slot scan rather than calling allocation.rs's
        // private `find_free_slot` — keeping mod.rs from depending on
        // that private helper avoids cross-file plumbing for one
        // 5-line scan.
        let mut chosen: Option<u16> = None;
        for (wi, &word) in self.free_bits.iter().enumerate() {
            if word != 0 {
                let bit = word.trailing_zeros() as u16;
                let slot = wi as u16 * 64 + bit;
                if slot < self.max {
                    chosen = Some(slot);
                    break;
                }
            }
        }
        let store_nr = if let Some(slot) = chosen {
            self.allocations[slot as usize] = store;
            slot
        } else {
            self.allocations.push(store);
            (self.allocations.len() - 1) as u16
        };
        if store_nr >= self.max {
            self.max = store_nr + 1;
        }
        // Clear the free bit (slot is now active).
        let wi = store_nr as usize / 64;
        let bi = store_nr as usize % 64;
        if wi < self.free_bits.len() {
            self.free_bits[wi] &= !(1u64 << bi);
        }
        store_nr
    }
}

#[cfg(test)]
mod worker_output_slot_tests {
    use super::{Stores, WorkerStores};

    #[test]
    fn add_output_slot_returns_next_index() {
        let mut s = Stores::new();
        let initial = s.allocations.len();
        let mut ws = WorkerStores::new(s);
        let slot = ws.add_output_slot(64);
        assert_eq!(slot.store_nr as usize, initial);
        assert!(ws.stores.allocations[slot.store_nr as usize].capacity_words() >= 64);
    }

    #[test]
    fn add_output_slot_minimum_one_word() {
        let mut ws = WorkerStores::new(Stores::new());
        let slot = ws.add_output_slot(0);
        assert!(ws.stores.allocations[slot.store_nr as usize].capacity_words() >= 1);
    }

    #[test]
    fn take_slot_returns_owned_store_and_leaves_sentinel() {
        let mut ws = WorkerStores::new(Stores::new());
        let slot = ws.add_output_slot(32);
        let pos = slot.store_nr as usize;
        let cap = ws.stores.allocations[pos].capacity_words();
        let taken = ws.take_slot(slot.store_nr);
        assert_eq!(taken.capacity_words(), cap);
        // Sentinel left behind has tiny capacity (Store::new_freed_sentinel = 4 words)
        // and is marked free.
        assert!(ws.stores.allocations[pos].free);
    }

    #[test]
    fn adopt_store_pushes_to_parent_allocations() {
        let mut parent = Stores::new();
        let initial_len = parent.allocations.len();
        let mut donor = WorkerStores::new(Stores::new());
        let slot = donor.add_output_slot(16);
        let store = donor.take_slot(slot.store_nr);
        let nr = parent.adopt_store(store);
        assert!((nr as usize) < parent.allocations.len() || (nr as usize) == initial_len);
        assert!(!parent.allocations[nr as usize].free);
    }

    #[test]
    #[should_panic(expected = "take_slot: slot")]
    fn take_slot_out_of_range_panics() {
        let mut ws = WorkerStores::new(Stores::new());
        let _ = ws.take_slot(9999);
    }

    #[test]
    fn take_all_owned_skips_parent_clone_slots() {
        // Parent has 2 stores; worker will get 2 clones + add 1 output.
        let mut parent = Stores::new();
        parent.allocations.push(crate::store::Store::new(8));
        parent.allocations.push(crate::store::Store::new(8));
        parent.max = 2;
        let parent_count = parent.allocations.len() as u16;

        // Build a synthetic worker view with 2 cloned slots + 1 output.
        let mut ws_inner = Stores::new();
        ws_inner.allocations.push(crate::store::Store::new(8));
        ws_inner.allocations.push(crate::store::Store::new(8));
        ws_inner.max = 2;
        let mut ws = WorkerStores::new(ws_inner);
        let _slot = ws.add_output_slot(16);

        let owned = ws.take_all_owned(parent_count);
        assert_eq!(owned.len(), 1, "only worker-allocated slot is adopted");
        assert_eq!(owned[0].0, 2, "adopted slot is at parent_count");
    }

    #[test]
    fn take_all_owned_returns_multiple_in_order() {
        let mut ws = WorkerStores::new(Stores::new());
        let _s0 = ws.add_output_slot(8);
        let _s1 = ws.add_output_slot(16);
        let _s2 = ws.add_output_slot(32);
        let owned = ws.take_all_owned(0);
        assert_eq!(owned.len(), 3);
        assert_eq!(owned[0].0, 0);
        assert_eq!(owned[1].0, 1);
        assert_eq!(owned[2].0, 2);
    }

    #[test]
    fn take_all_owned_skips_freed_slots() {
        let mut ws = WorkerStores::new(Stores::new());
        let s0 = ws.add_output_slot(8);
        let _s1 = ws.add_output_slot(16);
        // Mark s0 as freed.
        ws.stores.allocations[s0.store_nr as usize].free = true;
        let owned = ws.take_all_owned(0);
        assert_eq!(owned.len(), 1, "freed slot skipped");
        assert_eq!(owned[0].0, 1);
    }
}

#[cfg(test)]
mod store_rebase_tests {
    use super::DbRef;
    use crate::parallel::StoreRebase;

    #[test]
    fn translate_passes_through_unmapped() {
        let r = StoreRebase::new();
        let db = DbRef {
            store_nr: 5,
            rec: 10,
            pos: 8,
        };
        let out = r.translate(&db);
        assert_eq!(out.store_nr, 5);
        assert_eq!(out.rec, 10);
        assert_eq!(out.pos, 8);
    }

    #[test]
    fn translate_rewrites_mapped() {
        let mut r = StoreRebase::new();
        r.add(2, 7);
        let db = DbRef {
            store_nr: 2,
            rec: 1,
            pos: 4,
        };
        let out = r.translate(&db);
        assert_eq!(out.store_nr, 7, "store_nr translated");
        assert_eq!(out.rec, 1, "rec preserved");
        assert_eq!(out.pos, 4, "pos preserved");
    }

    #[test]
    fn multiple_translations_disjoint() {
        let mut r = StoreRebase::new();
        r.add(2, 7);
        r.add(3, 8);
        r.add(4, 9);
        for (worker_nr, expected_parent) in [(2, 7), (3, 8), (4, 9)] {
            let db = DbRef {
                store_nr: worker_nr,
                rec: 0,
                pos: 0,
            };
            assert_eq!(r.translate(&db).store_nr, expected_parent);
        }
    }
}

#[allow(dead_code)]
impl Stores {
    #[must_use]
    pub fn new() -> Stores {
        let mut result = Stores {
            types: Vec::new(),
            names: HashMap::new(),
            allocations: Vec::new(),
            files: Vec::new(),
            max: 0,
            free_bits: Vec::new(),
            scratch: Vec::new(),
            const_refs: Vec::new(),
            last_parse_errors: Vec::new(),
            last_json_errors: Vec::new(),
            parallel_ctx: None,
            logger: None,
            had_fatal: false,
            source_dir: String::new(),
            frame_yield: false,
            poison_free: false,
            report_asserts: false,
            assert_results: Vec::new(),
            user_args: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            start_time: Instant::now(),
            // `Stores::new()` must not call `Instant::now()` or
            // `SystemTime::now()` on wasm32-unknown-unknown — both
            // trap as `(unreachable)` with no time source.  The
            // `--html` build (wasm32, no `wasm` feature) uses 0 as
            // the epoch stub; the full `wasm` feature build routes
            // through the host bridge.
            #[cfg(all(target_arch = "wasm32", feature = "wasm"))]
            start_time_ms: crate::wasm::host_time_now(),
            #[cfg(all(target_arch = "wasm32", not(feature = "wasm")))]
            start_time_ms: 0,
            call_stack_snapshot: Vec::new(),
            variables_snapshot: Vec::new(),
            closure_map: HashMap::new(),
            jnull_sentinel: None,
        };
        result.base_type("integer", 8); // 0  (Phase 2c: widened from 4)
        result.base_type("long", 8); // 1
        result.base_type("single", 4); // 2
        result.base_type("float", 8); // 3
        result.base_type("boolean", 1); // 4
        result.base_type("text", 4); // 5
        result.base_type("character", 4); // 6
        result
    }

    /// Initiative 03 Phase 3b: return a `Str` pointing into the
    /// constant store.  Native-mode counterpart to
    /// `State::string_from_const_store`, which pushes the Str onto
    /// the bytecode interpreter's stack.  Native code uses the
    /// value directly via the `#rust"…"` template substitution
    /// `s.string_from_const_store` → `stores.string_from_const_store`.
    #[must_use]
    pub fn string_from_const_store(&self, rec: u32, _pos: u32) -> crate::keys::Str {
        let store = &self.allocations[CONST_STORE as usize];
        let len = store.get_u32_raw(rec, 4);
        let ptr = unsafe { store.ptr.offset(rec as isize * 8 + 8) };
        crate::keys::Str { ptr, len }
    }

    #[must_use]
    pub fn get<T: 'static>(&mut self, stack: &mut DbRef) -> &T {
        debug_assert!(
            stack.pos >= size_of::<T>() as u32,
            "Stack underflow in get<{}>: stack.pos={} but need {} bytes",
            std::any::type_name::<T>(),
            stack.pos,
            size_of::<T>(),
        );
        stack.pos -= size_of::<T>() as u32;
        let r = self.store(stack).addr::<T>(stack.rec, stack.pos);
        #[cfg(debug_assertions)]
        {
            if std::any::TypeId::of::<T>() == std::any::TypeId::of::<DbRef>() {
                let db: &DbRef = unsafe { &*(r as *const T as *const DbRef) };
                debug_assert!(
                    db.store_nr == u16::MAX || (db.store_nr as usize) < self.allocations.len(),
                    "get<DbRef>: OOB store_nr={} (allocations.len()={}) \
                     rec={} pos={} — corrupt DbRef on stack",
                    db.store_nr,
                    self.allocations.len(),
                    db.rec,
                    db.pos,
                );
            }
        }
        r
    }

    pub fn put<T: 'static>(&mut self, stack: &mut DbRef, val: T) {
        #[cfg(debug_assertions)]
        {
            if std::any::TypeId::of::<T>() == std::any::TypeId::of::<DbRef>() {
                let db: &DbRef = unsafe { &*(&val as *const T as *const DbRef) };
                debug_assert!(
                    db.store_nr == u16::MAX || (db.store_nr as usize) < self.allocations.len(),
                    "put<DbRef>: OOB store_nr={} (allocations.len()={}) \
                     rec={} pos={} — corrupt DbRef being pushed",
                    db.store_nr,
                    self.allocations.len(),
                    db.rec,
                    db.pos,
                );
            }
        }
        let m = self.store_mut(stack).addr_mut::<T>(stack.rec, stack.pos);
        *m = val;
        stack.pos += size_of::<T>() as u32;
    }

    /// Look up a type by index, panicking with a diagnostic if the index is out of range.
    ///
    /// # Panics
    /// Panics if `nr` is out of range for the types table.
    #[must_use]
    pub fn get_type(&self, nr: u16) -> &Type {
        self.types.get(nr as usize).unwrap_or_else(|| {
            panic!(
                "type index {} out of range (total: {})",
                nr,
                self.types.len()
            )
        })
    }
}

pub struct ShowDb<'a> {
    pub stores: &'a Stores,
    pub store: u16,
    pub rec: u32,
    pub pos: u32,
    pub known_type: u16,
    pub pretty: bool,
    pub json: bool,
}

/// Structured debug dump with store/record references, depth and element limits.
/// Used for `tests/dumps/*.txt` diagnostics and `LOFT_LOG` execution trace.
pub struct DumpDb<'a> {
    pub stores: &'a Stores,
    pub store: u16,
    pub rec: u32,
    pub pos: u32,
    pub known_type: u16,
    /// Maximum nesting depth (0 = just the value, 1 = one level of fields, etc.)
    pub max_depth: u16,
    /// Maximum number of array/vector elements to show before `...`
    pub max_elements: u16,
    /// When true, output stays on a single line (spaces instead of newlines).
    pub compact: bool,
}

/// `get_type()` with an out-of-range index must panic with a helpful message.
#[test]
#[should_panic(expected = "type index 999 out of range")]
fn get_type_out_of_range_panics() {
    let stores = Stores::new();
    let _ = stores.get_type(999);
}

// These values are for amd64 or arm64 systems.
// It's not possible to test these continuously as these will fail on 32-bit systems.
#[test]
fn sizes() {
    /*
    assert_eq!(size_of::<DbRef>(), 12);
    assert_eq!(size_of::<String>(), 24);
    assert_eq!(size_of::<&str>(), 16);
    */
}
