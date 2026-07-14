// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)
//! Memory/store allocation helpers and claim management.

use crate::database::{Parts, Stores, WorkerStores};
use crate::hash;
use crate::keys::DbRef;
use crate::radix_db;
use crate::store::Store;
use crate::tree;
use crate::vector;

// @PLN103 P3 — the `LOFT_STORES=timeline` runtime store timeline. A pure diagnostic
// (gated on the env var), so its state lives thread-local rather than on `Stores` — no
// struct/constructor ripple. A per-logical-store id is `<store_nr>.<seq>`: `store_nr` is a
// REUSED slot index (P0.4), so `seq` (a monotonic alloc counter) disambiguates slot reuse.
// `live` maps each live slot to its current seq so a free prints the SAME id as its alloc.
#[derive(Default)]
struct TimelineState {
    seq: u64,
    live: std::collections::HashMap<u16, u64>,
    peak_live: usize,
    total_alloc: u64,
    total_free: u64,
}

thread_local! {
    static TIMELINE: std::cell::RefCell<TimelineState> = std::cell::RefCell::new(TimelineState::default());
}

/// True when `LOFT_STORES=timeline` (cached per thread — the env is stable for a run).
fn timeline_on() -> bool {
    thread_local! { static ON: bool = std::env::var("LOFT_STORES").as_deref() == Ok("timeline"); }
    ON.with(|b| *b)
}

/// @PLN103 P3.3 — the working-set-vs-leak summary (the disambiguation `LOFT_STORES=warn`
/// cannot make): **peak concurrency** (the working set) vs a real leak. `real_leaked` is the
/// AUTHORITATIVE count from `collect_store_leaks()` (which excludes the eval-stack / const /
/// locked runtime infrastructure the raw timeline `live` map would false-positive on — the
/// interp-vs-native divergence this reconciles). Called at exit; no-op unless timeline mode.
pub fn timeline_summary(real_leaked: usize) {
    if !timeline_on() {
        return;
    }
    TIMELINE.with(|t| {
        let t = t.borrow();
        let verdict = if real_leaked == 0 {
            "NO leak (every user store freed)".to_string()
        } else {
            format!("{real_leaked} user store(s) LEAKED — see the leak warning for which")
        };
        eprintln!(
            "[timeline] SUMMARY: {} allocs, {} frees, peak {} concurrently-live (working set) — {verdict}",
            t.total_alloc, t.total_free, t.peak_live
        );
    });
}

/// One owned nested-heap child of a container record, as enumerated by
/// [`Stores::for_each_owned_child`] — the single per-`Parts` heap-cascade walk
/// that `remove_claims` and `copy_claims_hash_body` read instead of each
/// re-encoding the container layout (loop bounds, element strides, slot drift).
/// The historical `@P290`/`@P306`/`@P318`/`@P309` bugs were all in this walk,
/// hand-copied divergently across the dispatchers; carrying it once is the fix by
/// construction.
///
/// Two other per-`Parts` walkers deliberately do NOT fold onto this keystone — the
/// boundary is load-bearing (H10 / Cluster C), so it is stated here rather than
/// re-derived:
/// - `validate_claims` is a separate DEFENSIVE family, NOT a thin visitor over this
///   walk.  Enumerating a collection's children HERE means dereferencing the
///   container pointer — `length_vector`, `record_words(cur)`, tree navigation — which
///   this keystone TRUSTS (the accessors `debug_assert!` on a freed/out-of-range
///   record).  `validate_claims` runs on *suspected-corrupt* heaps (the `#306`
///   `LOFT_TRACE_CR` pre-walk before `OpCopyRecord`, naming the first broken edge
///   instead of faulting on it), so it bounds-checks each pointer BEFORE following it
///   and does not recurse into the per-element-record kinds (`Array`/`Ordered`/`Hash`/
///   `Index`) at all.  Its `Struct`/`Vector`/`ChildRec`/`Enum` child arithmetic
///   matches this keystone's, but those arms need no bound check and save nothing by
///   folding; the arms that WOULD save code are exactly the ones whose
///   guard-before-deref is the whole point — folding them would turn "name the broken
///   edge" back into "fault on it".  Keep it separate.
/// - `copy_claims`' destination construction is genuinely per-kind (allocate-into-`to`,
///   header writes, re-insert-vs-slot-copy) and is not a walk over this enumeration.
///   Its SOURCE enumeration now reads this keystone in ALL FOUR kinds — `hash_body`,
///   `index_body`, `array_body`, and `seq_vector` (folded H10, 2026-07) — so the
///   per-`Parts` source layout lives in ONE place.  Each helper then pairs a keystone
///   source child with its own per-kind destination build (re-insert for hash/index, a
///   freshly-claimed slot for array, a same-offset position after the bulk copy for
///   vector).
///
/// `child` is the `DbRef` (in the SOURCE record's store) at which the child value
/// lives; `child_tp` is its type.  `owning_elem` is the separate element record
/// that HOLDS this child for the per-element container kinds (`Array`/`Ordered`,
/// `Hash`, `Index`) — `remove_claims` `delete`s it after recursing — and is
/// `None` for the kinds whose children sit inline in the parent / container block
/// (`Struct`, `Vector`/`Sorted` contiguous elements, `Enum` variant re-dispatch,
/// `ChildRec`).
#[derive(Clone, Copy)]
pub(super) struct OwnedChild {
    pub child: DbRef,
    pub child_tp: u16,
    pub owning_elem: Option<u32>,
}

/// The container-level teardown a `remove_claims` walk performs AFTER recursing
/// into every owned child: which container/spine record (if any) to `delete`, and
/// whether the field pointer must be zeroed.  Read from the same keystone so the
/// per-kind spine layout lives in ONE place.
///
/// - `container_rec` is the block that backs the collection (the `Vector`/`Array`/
///   `Hash` payload record, or the `ChildRec` child record); `Index` has no
///   separate container block (its nodes ARE the records, freed via
///   `OwnedChild::owning_elem`), so it is `None` there.
/// - `zero_field` is true for the kinds whose value is reached through a heap
///   POINTER stored at `(rec.rec, rec.pos)` (`Vector`/`Sorted`, `Array`/`Ordered`,
///   `Hash`, `Index`, `ChildRec`) — `remove_claims` resets that pointer to 0 so a
///   later reassignment starts clean.  `Struct`/`EnumValue`/`Enum` hold their
///   children INLINE (no pointer to zero), so it is false.
pub(super) struct OwnedWalk {
    pub children: Vec<OwnedChild>,
    pub container_rec: Option<u32>,
    pub zero_field: bool,
}

impl Stores {
    /// The Cluster-C keystone: enumerate the owned nested-heap children of the
    /// record `rec` of type `tp`, plus the container record backing them.  This
    /// is the SINGLE per-`Parts` heap-cascade walk (element type, stride,
    /// container traversal) that `remove_claims` consumes as a thin visitor, and
    /// that `copy_claims_hash_body` reads for its source-bucket enumeration —
    /// replacing the divergent per-dispatcher re-encodings that produced the
    /// `@P290`/`@P306`/`@P318`/`@P309` family.
    ///
    /// Returns the children to recurse on (each carrying its own `DbRef`, type,
    /// and — for per-element kinds — the element record that owns it) and the
    /// container record to free.  Leaf / empty / null shapes yield no children
    /// and no container.  `Radix` is unsupported (callers panic on it); the
    /// keystone returns an empty walk so a non-cascading caller stays safe.
    ///
    /// Collects into a `Vec` (rather than borrowing an iterator) so callers can
    /// take `&mut self` to recurse — the same shape `collect_index_nodes`
    /// already uses.
    pub(super) fn for_each_owned_child(&self, rec: &DbRef, tp: u16) -> OwnedWalk {
        let mut children = Vec::new();
        let mut container_rec = None;
        match &self.types[tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields {
                    children.push(OwnedChild {
                        child: DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos: rec.pos + u32::from(f.position),
                        },
                        child_tp: f.content,
                        owning_elem: None,
                    });
                }
            }
            Parts::Vector(v) | Parts::Sorted(v, _) => {
                let v = *v;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 {
                    let length = vector::length_vector(rec, &self.allocations);
                    let size = u32::from(self.size(v));
                    for i in 0..length {
                        children.push(OwnedChild {
                            child: DbRef {
                                store_nr: rec.store_nr,
                                rec: cur,
                                pos: 8 + size * i,
                            },
                            child_tp: v,
                            owning_elem: None,
                        });
                    }
                    container_rec = Some(cur);
                }
            }
            Parts::Array(v) | Parts::Ordered(v, _) => {
                let v = *v;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 {
                    let length = vector::length_vector(rec, &self.allocations);
                    for i in 0..length {
                        let elm = self.store(rec).get_u32_raw(cur, 8 + i * 4);
                        children.push(OwnedChild {
                            child: DbRef {
                                store_nr: rec.store_nr,
                                rec: elm,
                                pos: 8,
                            },
                            child_tp: v,
                            owning_elem: Some(elm),
                        });
                    }
                    container_rec = Some(cur);
                }
            }
            Parts::Hash(v, _) => {
                let v = *v;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 {
                    // Enumerate the live element records through `hash::records`,
                    // the single owner of the bucket-record layout (the seed word
                    // and bucket offset live only in `src/hash.rs`).  Mirrors the
                    // `Index` arm's `collect_index_nodes`.  `cur` is the bucket
                    // record itself — a separate container to free (below).
                    for elm in hash::records(rec, &self.allocations) {
                        children.push(OwnedChild {
                            child: DbRef {
                                store_nr: rec.store_nr,
                                rec: elm,
                                pos: 8,
                            },
                            child_tp: v,
                            owning_elem: Some(elm),
                        });
                    }
                    container_rec = Some(cur);
                }
            }
            Parts::Index(c, _, _) => {
                let c = *c;
                let left = self.fields(tp);
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 {
                    for node in self.collect_index_nodes(rec, left) {
                        children.push(OwnedChild {
                            child: DbRef {
                                store_nr: rec.store_nr,
                                rec: node,
                                pos: 8,
                            },
                            child_tp: c,
                            owning_elem: Some(node),
                        });
                    }
                    // No separate container block: index nodes ARE the records.
                }
            }
            Parts::ChildRec(ct) => {
                let ct = *ct;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 {
                    children.push(OwnedChild {
                        child: DbRef {
                            store_nr: rec.store_nr,
                            rec: cur,
                            pos: 8,
                        },
                        child_tp: ct,
                        owning_elem: None,
                    });
                    container_rec = Some(cur);
                }
            }
            Parts::Enum(values) => {
                // Inline struct-enum: the live variant's payload sits at the SAME
                // pos (in-place re-dispatch).  `get_byte(.., -1)` shifts so stored
                // byte 1 = variant 0; a null/absent enum reads NEGATIVE and owns no
                // payload (skip rather than index `values` OOB).  A simple
                // (payload-less) variant is marked `u16::MAX`.
                let e_nr = self.store(rec).get_byte(rec.rec, rec.pos, -1);
                if e_nr >= 0 && (e_nr as usize) < values.len() {
                    let vtp = values[e_nr as usize].0;
                    if vtp != u16::MAX {
                        children.push(OwnedChild {
                            child: *rec,
                            child_tp: vtp,
                            owning_elem: None,
                        });
                    }
                }
            }
            Parts::Radix(v, _) => {
                let v = *v;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 {
                    // The tree's leaves ARE the element records; walk them key-free
                    // (`radix_db::records` → `rtree_first`/`next`).  `cur` is the tree
                    // container — a separate block to free (below).  Mirrors Hash.
                    for elm in radix_db::records(rec, &self.allocations) {
                        children.push(OwnedChild {
                            child: DbRef {
                                store_nr: rec.store_nr,
                                rec: elm,
                                pos: 8,
                            },
                            child_tp: v,
                            owning_elem: Some(elm),
                        });
                    }
                    container_rec = Some(cur);
                }
            }
            // Base text leaf, scalars, DbRef: no cascade.
            _ => {}
        }
        // The value is reached through a heap pointer (zeroed on teardown) for the
        // collection / child-record kinds; `Struct`/`EnumValue`/`Enum` hold their
        // children inline and have no pointer field to reset.
        let zero_field = matches!(
            self.types[tp as usize].parts,
            Parts::Vector(_)
                | Parts::Sorted(_, _)
                | Parts::Array(_)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Radix(_, _)
                | Parts::Index(_, _, _)
                | Parts::ChildRec(_)
        );
        OwnedWalk {
            children,
            container_rec,
            zero_field,
        }
    }

    /// True when `store_nr` is the interpreter's protected eval-stack store
    /// (slot 0 with `stack_store_at_zero` set by `State::new`).  The native
    /// runtime has no stack store — its slot 0 is an ordinary heap store that
    /// must stay freeable and leak-checkable, so every "skip the stack store"
    /// guard must use this predicate instead of a bare `store_nr == 0`
    /// (#490 hid the first leaked native store; #491 made it unfreeable).
    #[must_use]
    pub fn is_stack_store(&self, store_nr: u16) -> bool {
        store_nr == 0 && self.stack_store_at_zero
    }

    /**
    Try to allocate a new store.
    # Panics
    When a store already in use is allocated again.
    */
    pub fn database(&mut self, size: u32) -> DbRef {
        self.database_named(size, "")
    }

    /// Try to allocate a new named store.
    /// # Panics
    /// When a store already in use is allocated again.
    pub fn database_named(&mut self, size: u32, name: &str) -> DbRef {
        // S29: find the lowest free slot using the free_bits bitmap.
        // If a freed slot exists below max, reuse it; otherwise grow max.
        //
        // ARC.md A2: workers spawned by `run_parallel_queue_ref` set
        // `disable_slot_reuse = true` and carry a shared
        // `worker_slot_dispenser` (an `Arc<AtomicU16>`).  Every named
        // alloc consumes the next index from the dispenser, extends
        // the worker's clone's `allocations` to fit, and records the
        // index in `worker_allocated_indices`.  Cross-thread
        // collisions are impossible because the atomic dispenses
        // globally-unique indices; growth is unbounded (replaces
        // 8d.3's fixed 16-slot per-thread cap).
        // ARC.md A2.3 — invariant: if a dispenser is attached, every
        // named allocation MUST route through it.  The dispenser
        // becomes the single source of truth for the worker's
        // parent-namespace indices; bypassing it (e.g. by clearing
        // `disable_slot_reuse` mid-call) would let the worker push
        // into its own clone at an index that collides with another
        // worker's dispensed slot, silently corrupting the swap-back
        // at thread join.
        //
        // Always-on (not gated by `debug_assertions`) because the
        // loft library compiles with `debug-assertions = false` in
        // the test profile (per `[profile.dev.package.loft]`), so a
        // `debug_assert!` here would be silently a no-op in `cargo
        // test`.  This is a slow-path check (one-shot per fresh
        // store allocation), so the perf cost is negligible.
        assert!(
            self.worker_slot_dispenser.is_none() || self.disable_slot_reuse,
            "database_named: worker has a dispenser but disable_slot_reuse \
             was cleared — every dispenser-attached allocation must go \
             through the offset-aware path"
        );
        let slot = if let Some(dispenser) = self.worker_slot_dispenser.clone()
            && self.disable_slot_reuse
        {
            let idx = dispenser.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.worker_allocated_indices.push(idx);
            // Worker's clone may not have a slot at `idx` yet —
            // extend `allocations` (with empty Store::new(100)
            // placeholders for any skipped indices owned by other
            // threads).  Skipped indices stay `free=true` in this
            // worker's clone; only the just-allocated `idx` will be
            // reinitialised below.
            while self.allocations.len() <= idx as usize {
                self.allocations.push(Store::new(100));
            }
            idx
        } else if self.disable_slot_reuse {
            // 8d.2: push at the end of allocations so worker writes
            // never reuse a cloned slot.  Used when no dispenser is
            // attached (single-thread queue dispatch or legacy paths
            // that opt into disable_slot_reuse without queue_ref's
            // dispatcher).
            self.allocations.len() as u16
        } else {
            self.find_free_slot()
        };
        // #306 — slot space is u16 and store_nr 65535 is the null-DbRef
        // sentinel.  Without this cap, `max = slot + 1` wraps to 0 at slot
        // 65535 and the next allocation hands out slot 0 — the eval-stack
        // store — corrupting the whole runtime (SIGSEGV at the next big
        // copy).  Fail loudly instead: hitting this cap means ~65k stores
        // are live at once, which in practice is a store leak.
        assert!(
            slot != u16::MAX,
            "store table exhausted: 65535 stores live at once (store_nr is \
             u16; 65535 is the null sentinel).  This usually indicates a \
             store leak — run with LOFT_STORES=summary to list live stores \
             by type."
        );
        if slot >= self.allocations.len() as u16 {
            self.allocations.push(Store::new(100));
        } else {
            // A free slot may still carry a stale lock if set_store_lock was
            // called with a dangling DbRef after the store was freed.  Clear
            // the lock before reinitialising to prevent a spurious panic in
            // Store::init().
            self.allocations[slot as usize].unlock();
            self.reinit_reused_slot(slot as usize);
        }
        // Maintain the invariant `max == highest_allocated_index + 1`.
        // The dispenser path can yield indices > current max (each
        // dispense skips ahead in parent-namespace), so a bare
        // `slot == self.max` check would leave max stale and
        // subsequent free() trims to the wrong position.
        if slot >= self.max {
            self.max = slot + 1;
            // Monotonic watermark: the only site where `max` grows.  `peak`
            // never decrements, so it survives to end-of-run as the store
            // high-water (plan-57).
            if self.max > self.peak {
                self.peak = self.max;
            }
        }
        // Clear the bitmap bit for this slot (it is now active).
        self.clear_free_bit(slot);
        // @PLN101 Slice 0 — the true heap metric: a store slot just went live.
        self.stores_allocated += 1;
        // @PLN105 leak provenance — stamp the currently-executing op position so a
        // leaked store's allocation site is attributable for EVERY path (reuse/copy
        // included), not only `OpDatabase`. Read before the mutable slot borrow.
        let alloc_pc = self.alloc_pc;
        let store = &mut self.allocations[slot as usize];
        // @P317 — enrich the tripwire: this fires when the free-bitmap and the
        // per-store `free` flag disagree (a double-free, an rc under-count, or a
        // store-pool overflow that wrapped `max` to 0 and re-selected slot 0).
        // The slot + rc + type + name pinpoint the victim far faster than the
        // bare message did during the @P311/@P313/@P317 store-ownership hunts.
        assert!(
            store.free,
            "Allocating a used store #{slot} (known_type={}, requested by={})",
            store.known_type,
            if name.is_empty() { "<anon>" } else { name },
        );
        store.free = false;
        store.pinned = false;
        store.created_at = alloc_pc;
        store.last_op_at = 0;
        let rec = if size == u32::MAX {
            0
        } else {
            store.claim(size)
        };
        let result = DbRef {
            store_nr: slot,
            rec,
            pos: 8,
        };
        // LOFT_STORES=log  → full alloc/free trace
        // LOFT_STORES=warn → only warn when active stores > 30
        let active = self.allocations.iter().filter(|s| !s.free).count();
        match std::env::var("LOFT_STORES").as_deref() {
            Ok("log") => {
                let label = if name.is_empty() { "" } else { name };
                eprintln!(
                    "[store] + alloc #{} {label:>12} | active={active:<4} max={:<4} size={size}",
                    result.store_nr, self.max
                );
            }
            Ok("warn") if active > 30 => {
                eprintln!(
                    "[store] WARNING: {active} active stores (max={}) — possible leak at alloc #{}",
                    self.max, result.store_nr
                );
            }
            Ok("timeline") => {
                let seq = TIMELINE.with(|t| {
                    let mut t = t.borrow_mut();
                    let s = t.seq;
                    t.seq += 1;
                    t.total_alloc += 1;
                    t.live.insert(result.store_nr, s);
                    t.peak_live = t.peak_live.max(t.live.len());
                    s
                });
                let label = if name.is_empty() { "·" } else { name };
                eprintln!(
                    "[timeline] alloc #{}.{seq}  {label:<14} live={active} size={size}",
                    result.store_nr
                );
            }
            _ => {}
        }
        result
    }

    /**
    Free a reference to a store. Make it available again for later code.
    # Panics
    When the code doesn't free the last claimed store first.
    */
    pub fn free(&mut self, db: &DbRef) {
        self.free_named(db, "");
    }

    /**
    Like [`free`], but includes the loft variable name in `LOFT_STORE_LOG` output.
    Generated native code calls this variant via `OpFreeRef(stores, var, "var_name")`.
    */
    pub fn free_named(&mut self, db: &DbRef, name: &str) {
        // u16::MAX is the null-sentinel used by OpNullRefSentinel for inline-ref temporaries
        // that were never assigned a real store.  Nothing to free in this case.
        if db.store_nr == u16::MAX {
            return;
        }
        let al = db.store_nr;
        // #405 — a wrong/stale free can carry an out-of-range store_nr (e.g. a
        // stack-NRVO loop-local's dep slot read on a not-taken branch before it
        // is initialised). An out-of-range nr is never a live store, so refuse it
        // loudly rather than panicking the whole runtime (generalises the #306
        // stack-store guard). Debug builds still trip the debug_assert below so
        // the wrong-free site surfaces for a codegen root fix.
        #[cfg(not(debug_assertions))]
        if al as usize >= self.allocations.len() {
            // Warn once per process — the wrong-free can recur every loop
            // iteration, so a per-occurrence print would flood stderr.
            use std::sync::atomic::{AtomicBool, Ordering};
            static WARNED: AtomicBool = AtomicBool::new(false);
            if !WARNED.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "loft: BUG (#405): refused free of out-of-range store #{al} \
                     (allocations.len={}, rec={}, pos={}, var='{name}') — wrong/stale \
                     ref; further such frees are silently refused this run",
                    self.allocations.len(),
                    db.rec,
                    db.pos,
                );
            }
            return;
        }
        debug_assert!(al < self.allocations.len() as u16, "Incorrect store");
        // #306 — store 0 is the eval-stack store; freeing it destroys every
        // live frame and lets the allocator recycle slot 0 as a heap store.
        // A ref to a stack-allocated record (OpCreateStack) must never be
        // whole-store freed: refuse loudly so the wrong-free site surfaces
        // instead of corrupting the entire runtime.
        if al == 0 && self.stack_store_at_zero {
            eprintln!(
                "loft: BUG (#306): refused to free the stack store (#0) \
                 (rec={}, pos={}, var='{name}') — a stack-record ref was \
                 treated as an owned heap store",
                db.rec, db.pos,
            );
            return;
        }
        let store = &mut self.allocations[al as usize];
        if store.free {
            return; // Already freed — no-op (replaces Issue #120 tolerance hack).
        }
        // Plan-57 Phase C: const/global stores are PINNED — never freed (they
        // live for the whole program).  This replaces the `ref_count = u32::MAX/2`
        // sentinel + the `ref_count > 1` guard below as the ref-count is removed.
        if store.pinned {
            return;
        }
        // Plan-57 Phase C: the Stores ref-count is removed.  Every non-pinned
        // store is single-owner (closure-captured cells are owned by the closure
        // record's cascade, not rc — see Phase B), so `free_named` always frees.
        // (Pinned const/global stores returned above.)
        // P259 commit 4: cascade-free closure-record DbRef attributes.
        // When the store being freed holds a `__closure_*` record,
        // each Parts::DbRef field references either the closure's
        // captured `__cell_<T>` (which the record OWNS — C74 limits a
        // mutated cell to one capturing closure, so this cascade is
        // the cell's single owner free) or a captured live original
        // (a `Reference` capture).  Walk those fields, read each
        // 12-byte stored DbRef, and recursively free_named.  There is
        // no ref-count (plan-57 phase C removed it): when the target
        // was already freed — e.g. the defining frame freed a
        // captured original before this record died — the recursive
        // call hits the `store.free` no-op above.  Sound because all
        // sharers of a DbRef die with the same frame; a sharer
        // escaping its defining frame is exactly what C74 forbids for
        // cells.
        //
        // Gated on the type name's `__closure_` prefix because:
        // - Only closure records hold cells via Parts::DbRef.
        // - User code can't define identifiers with `__` prefix
        //   (loft parser rejects), so the prefix check is leak-free.
        // - Cascading every Parts::DbRef field would break P213
        //   ChildRec storage and any future DbRef-holding struct.
        let cascade_targets: Vec<DbRef> = {
            let store_ref = &self.allocations[al as usize];
            let known_type = store_ref.known_type;
            if known_type != u16::MAX
                && self.types[known_type as usize]
                    .name
                    .starts_with("__closure_")
            {
                let dbref_positions: Vec<u16> = if let Parts::Struct(fields) =
                    &self.types[known_type as usize].parts
                {
                    fields
                        .iter()
                        .filter(|f| matches!(self.types[f.content as usize].parts, Parts::DbRef))
                        .map(|f| f.position)
                        .collect()
                } else {
                    Vec::new()
                };
                dbref_positions
                    .iter()
                    .map(|&fpos| {
                        let off = db.pos + u32::from(fpos);
                        let store_nr = store_ref.get_u32_raw(db.rec, off) as u16;
                        let rec = store_ref.get_u32_raw(db.rec, off + 4);
                        let pos = store_ref.get_u32_raw(db.rec, off + 8);
                        DbRef { store_nr, rec, pos }
                    })
                    .collect()
            } else {
                Vec::new()
            }
        };
        match std::env::var("LOFT_STORES").as_deref() {
            Ok("log") => {
                let active = self.allocations.iter().filter(|s| !s.free).count();
                let label = if name.is_empty() { "" } else { name };
                eprintln!(
                    "[store] - free   #{al} {label:>12} | active={:<4} max={}",
                    active, self.max
                );
            }
            Ok("timeline") => {
                // @PLN103 P3 — print the SAME `<store_nr>.<seq>` id the alloc printed, so
                // a reader matches alloc↔free across slot reuse. `?` = a free with no live
                // alloc record (a double-free or a pre-timeline store).
                let (seq, live_after) = TIMELINE.with(|t| {
                    let mut t = t.borrow_mut();
                    t.total_free += 1;
                    let s = t.live.remove(&al);
                    (s, t.live.len())
                });
                let seq = seq.map_or_else(|| "?".to_string(), |s| s.to_string());
                let label = if name.is_empty() { "·" } else { name };
                eprintln!("[timeline] free  #{al}.{seq}  {label:<14} live={live_after}");
            }
            _ => {}
        }
        // S36: clear the lock before marking free.
        let store = &mut self.allocations[al as usize];
        store.unlock();
        // LOFT_POISON=1 (@PLN54 S3) or LOFT_LOG=poison_free: overwrite the
        // freed buffer with a recognisable pattern so subsequent stale-DbRef
        // reads hit loud garbage (0xDEADBEEF repeated) instead of whatever
        // bytes the allocator happens to have left.  Skip the size-header
        // word (offset 0..8) so the bitmap/housekeeping can still read the
        // "freed" marker; start poisoning from offset 8.  The env flag is the
        // dedicated front door (works on BOTH backends — native calls this
        // same free); the struct flag is the LOFT_LOG=poison_free path.
        // A FILE-BACKED store's memory IS the mmap'd file: poisoning it
        // persists 0xDEADBEEF into durable state (the store_persist reload
        // read back an empty hash under LOFT_POISON).  The detector targets
        // in-memory stale reads; durable bytes are out of its scope.
        if (self.poison_free || crate::keys::poison_enabled()) && !store.is_file_backed() {
            let cap_bytes = store.capacity_words() as usize * 8;
            if cap_bytes > 8 {
                unsafe {
                    let base = store.ptr.add(8);
                    // Write 0xDEADBEEF to every i32-aligned word past the
                    // size header.  Use a byte-level loop to avoid worrying
                    // about alignment requirements on the raw pointer.
                    const POISON: [u8; 4] = [0xEF, 0xBE, 0xAD, 0xDE];
                    for off in 0..(cap_bytes - 8) {
                        *base.add(off) = POISON[off & 3];
                    }
                }
            }
        }
        store.free = true;
        // LOFT_UAF: record the freed slot so the dispatch loop can scan, after
        // this op, for live variables that still read it (a premature free).
        if crate::keys::uaf_any_enabled() {
            self.uaf_freed_this_op.push(al);
        }
        // LOFT_UAF_GEN (c): bump the slot's generation so a DbRef minted at the old
        // gen is detectably stale if read after this free (+ any later reuse).
        if crate::keys::uaf_gen_enabled() {
            crate::keys::uaf_bump_gen(al);
        }
        // S29: mark slot as free in the bitmap so database_named()
        // can reuse it without LIFO ordering.
        self.set_free_bit(al);
        // Trim max when freeing the top slot(s) so that database_named() doesn't
        // needlessly grow the allocations Vec when all top slots are free.
        if al == self.max - 1 {
            self.max -= 1;
            while self.max > 0 && self.allocations[(self.max - 1) as usize].free {
                self.max -= 1;
            }
        }
        // P259 commit 4: cascade-free the captured-cell DbRefs collected
        // above.  Done AFTER the closure record's own free so that
        // a recursive cascade on a closure-record cell sees this slot
        // as already-freed and does not re-enter.  Skip the null
        // sentinel pattern (store_nr=0, rec=0) which is the default
        // value written by `set_default_value` for unset DbRef fields.
        for target in cascade_targets {
            if target.store_nr != 0 || target.rec != 0 {
                self.free_named(&target, "<cascade>");
            }
        }
    }

    /// S29: Find the lowest free slot index below `max` using the `free_bits` bitmap.
    /// Returns `self.max` when no freed slot is available (caller must grow the Vec).
    fn find_free_slot(&self) -> u16 {
        for (wi, &word) in self.free_bits.iter().enumerate() {
            if word != 0 {
                let bit = word.trailing_zeros() as u16;
                let slot = wi as u16 * 64 + bit;
                if slot < self.max {
                    return slot;
                }
            }
        }
        self.max
    }

    /// S29: Set bit `slot` in `free_bits`, growing the Vec as needed.
    fn set_free_bit(&mut self, slot: u16) {
        let wi = slot as usize / 64;
        let bi = slot as usize % 64;
        while self.free_bits.len() <= wi {
            self.free_bits.push(0);
        }
        self.free_bits[wi] |= 1u64 << bi;
    }

    /// S29: Clear bit `slot` in `free_bits` (slot is now active).
    fn clear_free_bit(&mut self, slot: u16) {
        let wi = slot as usize / 64;
        let bi = slot as usize % 64;
        if wi < self.free_bits.len() {
            self.free_bits[wi] &= !(1u64 << bi);
        }
    }

    /// @PLN16 M1a — the lowest store-slot index guaranteed unused: above every
    /// allocated slot *and* the `max` watermark.  A value built at or above this floor
    /// occupies slots that are free here, which is what lets the debugger's whole-value
    /// heap edit graft it in with **no `DbRef` remap**.
    #[must_use]
    pub fn high_water(&self) -> u16 {
        (self.allocations.len() as u16).max(self.max)
    }

    /// @PLN16 M1a — force this (throwaway *build*) `Stores`' next store allocations to
    /// land at slot `floor` and above.  Pads `allocations` to `floor` with in-use
    /// placeholders and marks every slot below `floor` in-use, so [`find_free_slot`]
    /// returns `floor` and `database_named` pushes there.  Used by the debugger heap
    /// edit so the value's stores occupy slots that are free in the *live* paused
    /// store, enabling the no-remap graft ([`adopt_value_stores`](Self::adopt_value_stores)).
    /// The placeholders are never read — the edit's literal is self-contained, so the
    /// constructor only touches its own (real, recompiled) const stores and the new
    /// value-stores — they exist only so the slot indices line up.
    pub fn raise_floor(&mut self, floor: u16) {
        while (self.allocations.len() as u16) < floor {
            let mut placeholder = Store::new(2);
            placeholder.free = false; // in-use marker; bytes are never read
            self.allocations.push(placeholder);
        }
        // Clearing every free bit makes `find_free_slot` return `max`; setting `max =
        // floor` (== allocations.len()) makes the next claim a push at exactly `floor`.
        self.free_bits.clear();
        self.max = floor;
        if self.max > self.peak {
            self.peak = self.max;
        }
    }

    /// @PLN16 M1a — graft the value-stores a debugger heap edit built on `build` into
    /// this (live paused) `Stores`.  `build` raised its floor above this store's
    /// high-water then constructed the new value there, so every value-store sits on a
    /// slot that is **free here** — the move needs no `DbRef` remap (the root and its
    /// whole internal graph keep their slot numbers, valid here unchanged).
    ///
    /// Each in-use `build` slot in `[floor, build.max)` is moved here at the same index
    /// (its `Drop` defused by swapping in a freed sentinel); a slot `build` claimed then
    /// freed mid-construction is skipped (it is free here too).  Slots below `floor`
    /// that this store lacks are padded with free sentinels so indices line up.
    pub fn adopt_value_stores(&mut self, build: &mut Stores, floor: u16) {
        let top = build.allocations.len() as u16;
        while (self.allocations.len() as u16) < top {
            let idx = self.allocations.len() as u16;
            self.allocations.push(Store::new_freed_sentinel());
            self.set_free_bit(idx); // a slot this store never had is free here
        }
        for slot in floor..top {
            let si = slot as usize;
            if build.allocations[si].free {
                continue; // a transient the constructor claimed then freed
            }
            let moved = std::mem::replace(&mut build.allocations[si], Store::new_freed_sentinel());
            self.allocations[si] = moved;
            self.clear_free_bit(slot);
            if slot + 1 > self.max {
                self.max = slot + 1;
                if self.max > self.peak {
                    self.peak = self.max;
                }
            }
        }
    }

    /// Collect a description for every leaked store at program exit.
    ///
    /// Mirrors `State::collect_store_leaks` (which operates on the
    /// interpreter's `State`) but lives on `Stores` so the **native**
    /// runtime can run the same check — the generated `main` bootstrap
    /// calls this when `LOFT_NATIVE_LEAK_CHECK` is set so leak
    /// regressions surface on `--native` as well as `--interpret`.
    /// Same filtering: skip the stack store when one occupies slot 0
    /// (`stack_store_at_zero` — interp only; the native runtime's
    /// slot 0 is an ordinary heap store and MUST be checked, #490),
    /// locked constants / worker borrows, and `const_refs`.
    #[must_use]
    /// @P317 — leaked stores grouped BY TYPE, most-leaked first.  The previous
    /// per-store `N(bc:created_at)` listing (truncated to 5 by the leak-check
    /// preview) buried the signal in store numbers; aggregating by type names
    /// the culprit directly (e.g. `kt=68 ChunkKey×6026`), which is what
    /// pinpointed the @P317 native ref-local leak.  Used by
    /// `LOFT_NATIVE_LEAK_CHECK` (native) and `LOFT_STORES=summary` (interp).
    pub fn collect_store_leaks(&self) -> Vec<String> {
        let mut by_type: std::collections::BTreeMap<(u16, &str), usize> =
            std::collections::BTreeMap::new();
        for (s_nr, s) in self.allocations.iter().enumerate() {
            if s_nr == 0 && self.stack_store_at_zero {
                continue; // stack store — always alive
            }
            if s.is_locked() || self.const_refs.iter().any(|cr| cr.store_nr == s_nr as u16) {
                continue;
            }
            if !s.free {
                let tn = self
                    .types
                    .get(s.known_type as usize)
                    .map_or("?", |t| t.name.as_str());
                *by_type.entry((s.known_type, tn)).or_default() += 1;
            }
        }
        let mut leaked: Vec<((u16, &str), usize)> = by_type.into_iter().collect();
        leaked.sort_by_key(|&(_, n)| std::cmp::Reverse(n)); // most-leaked first
        leaked
            .into_iter()
            .map(|((kt, tn), n)| format!("kt={kt} {tn}×{n}"))
            .collect()
    }

    /**
    Validate if a reference is already freed before.
    # Panics
    When the store was already freed before.
    */
    pub fn valid(&self, db: &DbRef) {
        if db.store_nr == u16::MAX {
            return; // null-sentinel: never allocated, always valid-as-null
        }
        debug_assert!(
            db.store_nr < self.allocations.len() as u16,
            "Incorrect store"
        );
        // Note: accessing a freed store can still happen when a closure captures
        // a variable whose store was freed by copy_record's source-free.
        // The rc system prevents double-free; this access is benign (reads stale data
        // that will be overwritten).  A full fix requires inc_rc on closure capture.
    }

    /// Re-initialise a REUSED slot to a clean empty store.  If the slot still
    /// holds a file-backed (mmap) store from a `store_persist_bind`, replace it
    /// with a fresh anonymous store instead of `init()`-ing THROUGH the mmap:
    /// `init()` writes the empty-store header into the mapped file, blanking the
    /// persisted store's in-memory view so a later bind into the reused slot
    /// reads an empty hash (the on-disk bytes, unsynced, survive — so a fresh
    /// process still reads the data).  Dropping the old store flushes + closes
    /// the mmap.  #513.
    fn reinit_reused_slot(&mut self, slot: usize) {
        if self.allocations[slot].is_file_backed() {
            self.allocations[slot] = Store::new(100);
        } else {
            self.allocations[slot].init();
        }
    }

    pub fn clear(&mut self, db: &DbRef) {
        let slot = db.store_nr;
        // Clear any stale lock before reinitialising — OpDatabase may
        // reinitialise a store that was previously locked by a const
        // parameter in a prior function call within the same loop iteration.
        // never unlock a PINNED (const/global) store.
        if !self.allocations[slot as usize].pinned {
            self.allocations[slot as usize].unlock();
        }
        // OpDatabase may adopt a store its variable freed at the end of the
        // previous loop iteration (the slot still holds the stale DbRef).
        // Adoption IS ownership: mark the store in use and unlink it from
        // the free bitmap, or `find_free_slot` hands the SAME slot to the
        // next fresh allocation — two owners, and the second's writes wipe
        // the first's record (#348: a File record clobbered by a sibling
        // call's result vector).  #513: a file-backed slot is replaced, not
        // init()'d through the mmap (see reinit_reused_slot).
        self.reinit_reused_slot(slot as usize);
        self.allocations[slot as usize].free = false;
        self.clear_free_bit(slot);
        // free_named's top-slot trim may have dropped `max` BELOW this slot
        // (the stale frame ref outlives the watermark).  Restore it, or
        // find_free_slot returns `max` == this very slot and the fresh-alloc
        // path `init()`s the just-adopted live store.
        if slot >= self.max {
            self.max = slot + 1;
            if self.max > self.peak {
                self.peak = self.max;
            }
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn type_claim(&self, tp: u16) -> u32 {
        u32::from(self.types[tp as usize].size).div_ceil(8)
    }

    pub fn claim(&mut self, db: &DbRef, size: u32) -> DbRef {
        let store = &mut self.allocations[db.store_nr as usize];
        let rec = store.claim(size);
        DbRef {
            store_nr: db.store_nr,
            rec,
            pos: 8,
        }
    }

    #[must_use]
    pub fn null(&mut self) -> DbRef {
        self.database(u32::MAX)
    }

    /// Like [`null`], but includes the loft variable name in `LOFT_STORE_LOG` output.
    /// Generated native code calls this for each `DbRef` variable declaration.
    pub fn null_named(&mut self, name: &str) -> DbRef {
        self.database_named(u32::MAX, name)
    }

    #[must_use]
    pub fn store(&self, r: &DbRef) -> &Store {
        let s = &self.allocations[r.store_nr as usize];
        #[cfg(debug_assertions)]
        if s.free {
            eprintln!(
                "[store] ACCESS FREED store #{} rec={} pos={} — data will be garbage",
                r.store_nr, r.rec, r.pos
            );
        }
        s
    }

    /// C60 Step 3 (path 2c, piece 1): build a fresh vector of u32
    /// rec-nrs from the hash's records, sorted ascending by key.
    ///
    /// Called by the `on=4` hash-iteration arm in `OpIterate` at
    /// runtime.  The returned DbRef points at a header record whose
    /// offset-4 word is the data-record number; the data record's
    /// offset-4 word is the element count (n), and offset 8 onwards
    /// holds n `u32` rec-nrs at 4-byte stride.
    ///
    /// **Layout matches `Ordered`-style vectors** (see
    /// `src/state/io.rs:777` and `src/vector.rs:448`) — that's why the
    /// `step` handler for on=4 can walk this vector with the same
    /// u32-stride logic Ordered uses, yielding
    /// `DbRef{store=hash_store, rec=<u32>, pos=8}` per iteration.
    ///
    /// Note: `elem_store` is NOT encoded in the scratch; the runtime
    /// retains the original hash's `store_nr` via the companion
    /// iterator-local allocated by `parse_for_iter_setup`.
    #[allow(dead_code)]
    pub fn build_hash_sorted_vec(&mut self, hash_ref: &DbRef, tp: u16) -> DbRef {
        let keys = self.types[tp as usize].keys.clone();
        let recs = crate::hash::records_sorted(hash_ref, &self.allocations, &keys);
        self.build_rec_scratch(hash_ref, &recs)
    }

    /// Like `build_hash_sorted_vec` but in raw bucket-walk order, skipping the
    /// O(n log n) key sort.  Used to feed `for e in h par(...)`: the parallel
    /// queue preserves input order, but a hash has no user-meaningful order, so
    /// sorting the records only to hand them straight to worker threads is
    /// wasted work.  Iteration order therefore differs from sequential
    /// `for e in h` (which is key-ordered) — acceptable for a hash.
    pub fn build_hash_unsorted_vec(&mut self, hash_ref: &DbRef, _tp: u16) -> DbRef {
        let recs = crate::hash::records(hash_ref, &self.allocations);
        self.build_rec_scratch(hash_ref, &recs)
    }

    /// @PLN48 — the Radix counterpart, feeding the same Ordered (on=3) iteration
    /// path via `build_rec_scratch`.  Unlike a hash, a radix tree has a **natural
    /// order** (its in-order walk is key order — Morton/Z-order for a spatial
    /// index), so `radix_db::records` already yields the records sorted: no O(n log n)
    /// key sort, just the O(n) tree walk.  The `tp` is unused for the same reason.
    pub fn build_radix_sorted_vec(&mut self, coll: &DbRef, _tp: u16) -> DbRef {
        let recs = crate::radix_db::records(coll, &self.allocations);
        self.build_rec_scratch(coll, &recs)
    }

    /// @PLN48 S3 — a `spatial` range slice as an iterable scratch vector, feeding the
    /// same Ordered (on=3) path as `build_radix_sorted_vec`.  Records whose Morton code
    /// lies in `[from, till]` (or `[from, ∞)` when `has_till == 0`), in natural order,
    /// capped at `limit` (`< 0` = no cap).  Backs `xs[(x,y)..]`, `xs[(x,y)..:n]`, and the
    /// bounding box `xs[(x1,y1)..(x2,y2)]`.  Coordinates arrive as a fixed `MAX_AXES`-wide
    /// triple; only the collection's own `keys.len()` axes are read (a 2D collection
    /// ignores `fz`/`tz`), so the same ABI serves 1D…3D slices.
    #[allow(clippy::too_many_arguments)]
    pub fn build_radix_range_vec(
        &mut self,
        coll: &DbRef,
        tp: u16,
        fx: i64,
        fy: i64,
        fz: i64,
        has_till: i64,
        tx: i64,
        ty: i64,
        tz: i64,
        limit: i64,
    ) -> DbRef {
        let keys = self.types[tp as usize].keys.clone();
        let n = keys.len().min(crate::radix_db::MAX_AXES);
        let from = [fx, fy, fz];
        let till = [tx, ty, tz];
        let till_ref = (has_till != 0).then_some(&till[..n]);
        let cap = (limit >= 0).then_some(limit as usize);
        let recs =
            crate::radix_db::range(coll, &self.allocations, &keys, &from[..n], till_ref, cap);
        self.build_rec_scratch(coll, &recs)
    }

    /// Materialise `recs` (live hash rec-nrs) into a rec-nr scratch vector that
    /// the Ordered (on=3) iteration path walks.
    ///
    /// C60 piece 3 edit A: allocate IN THE HASH'S STORE, not a fresh one.  This
    /// makes the yielded scratch DbRef share `store_nr` with the hash records —
    /// so when Ordered iteration yields `DbRef{store=scratch.store_nr,
    /// rec=<u32 rec-nr from vector>, pos=8}`, the rec-nr resolves to a valid
    /// hash record in the same store.  No new on=4 mode, no bytecode protocol
    /// change — hash iteration reuses the existing Ordered (on=3) path.
    fn build_rec_scratch(&mut self, hash_ref: &DbRef, recs: &[u32]) -> DbRef {
        let n = recs.len();
        // 8-byte header + n * 4 bytes of u32 rec-nrs, rounded up to 8-byte
        // words (store claim granularity).
        let vec_words = ((n as u32) * 4 + 8).div_ceil(8);
        let vec_words = vec_words.max(1);
        let vec_cr = self.claim(hash_ref, vec_words);
        let vec_rec = vec_cr.rec;
        let header_cr = self.claim(hash_ref, 1);
        let header_rec = header_cr.rec;
        {
            let store = self.store_mut(hash_ref);
            store.set_u32_raw(vec_rec, 4, n as u32);
            for (i, &rec_nr) in recs.iter().enumerate() {
                let base = 8 + (i as u32) * 4;
                store.set_u32_raw(vec_rec, base, rec_nr);
            }
            store.set_u32_raw(header_rec, 4, vec_rec);
        }
        DbRef {
            store_nr: hash_ref.store_nr,
            rec: header_rec,
            pos: 4,
        }
    }

    pub fn store_mut(&mut self, r: &DbRef) -> &mut Store {
        #[cfg(debug_assertions)]
        if self.allocations[r.store_nr as usize].free {
            eprintln!(
                "[store] WRITE TO FREED store #{} rec={} pos={} — corruption",
                r.store_nr, r.rec, r.pos
            );
        }
        &mut self.allocations[r.store_nr as usize]
    }

    /// Lock the store that contains the record pointed to by `r` —
    /// user-facing `d#lock = true` semantics.  Sets the HARD `read_only`
    /// flag so subsequent writes / claims / deletes panic immediately;
    /// reads remain legal.  Use `set_free_protected` directly when only
    /// frees need blocking (e.g. the fn-call deep-copy bracket from
    /// @P290 — currently unused but kept available).
    pub fn lock_store(&mut self, r: &DbRef) {
        if r.rec != 0 && (r.store_nr as usize) < self.allocations.len() {
            debug_assert!(
                !self.allocations[r.store_nr as usize].free,
                "Locking a freed store (store_nr={}, rec={})",
                r.store_nr, r.rec
            );
            let origin = format!("lock_store(store_nr={}, rec={})", r.store_nr, r.rec);
            self.allocations[r.store_nr as usize].lock_with_origin(origin);
        }
    }

    /// Unlock the store that contains the record pointed to by `r` —
    /// counterpart of `lock_store`.  Clears the user-facing read_only
    /// lock; leaves `free_protected` untouched.
    pub fn unlock_store(&mut self, r: &DbRef) {
        if r.rec != 0 && (r.store_nr as usize) < self.allocations.len() {
            self.allocations[r.store_nr as usize].unlock();
        }
    }

    /// Return whether the store containing the record pointed to by `r` is locked.
    #[must_use]
    pub fn is_store_locked(&self, r: &DbRef) -> bool {
        r.rec != 0
            && (r.store_nr as usize) < self.allocations.len()
            && self.allocations[r.store_nr as usize].is_locked()
    }

    /// Deep-copy a struct record from a worker's `Stores` into a pre-allocated
    /// destination in this (main) `Stores`.
    ///
    /// Uses a temporary "graft": the worker's source store is swapped into
    /// `self.allocations` at its `store_nr` index so that `copy_block` and
    /// `copy_claims` can reach both source and destination through the same
    /// `Stores` instance.  After copying the graft is swapped back out.
    pub fn copy_from_worker(
        &mut self,
        src_ref: &DbRef,
        dest: &DbRef,
        worker_stores: &mut Stores,
        tp: u16,
    ) {
        let ws = src_ref.store_nr as usize;

        // Extend allocations so the worker's store index is reachable.
        while self.allocations.len() <= ws {
            self.allocations.push(Store::new(100));
        }

        // Graft the worker's store in.
        std::mem::swap(
            &mut self.allocations[ws],
            &mut worker_stores.allocations[ws],
        );

        // Raw byte copy + deep-copy of owned sub-fields (text, nested refs).
        let size = u32::from(self.size(tp));
        self.copy_block(src_ref, dest, size);
        self.copy_claims(src_ref, dest, tp);

        // Un-graft: put the worker's store back.
        std::mem::swap(
            &mut self.allocations[ws],
            &mut worker_stores.allocations[ws],
        );
    }

    /// Plan-06 phase 2 step 2b — narrow rebase path for structs whose
    /// fields are entirely self-contained (no text, no DbRef sub-fields).
    /// `copy_block`s the struct bytes from the worker store into the
    /// dest record without invoking `copy_claims` — there's nothing to
    /// deep-copy.  The graft + ungraft swap is replaced by a single
    /// graft (the worker's store is borrowed via the existing slot
    /// for the duration of the copy_block, then swapped back).
    ///
    /// Caller MUST verify `Stores::has_owned_sub_fields(tp) == false`
    /// before calling — otherwise the dest's text/DbRef fields will
    /// reference invalid positions/store_nrs after the worker's stores
    /// are dropped at thread join.
    ///
    /// This is the "no copy_claims" version of `copy_from_worker`,
    /// applicable to ~40 % of typical struct returns (those without
    /// text or nested references — the common case for compute-heavy
    /// workloads returning numeric records).
    pub fn copy_from_worker_unowned(
        &mut self,
        src_ref: &DbRef,
        dest: &DbRef,
        worker_stores: &mut Stores,
        tp: u16,
    ) {
        debug_assert!(
            !self.has_owned_sub_fields(tp),
            "copy_from_worker_unowned called with owned-fields struct (tp={tp})",
        );

        let ws = src_ref.store_nr as usize;
        while self.allocations.len() <= ws {
            self.allocations.push(Store::new(100));
        }

        // Graft the worker's store in (so copy_block can read from it
        // through the parent's allocations table).
        std::mem::swap(
            &mut self.allocations[ws],
            &mut worker_stores.allocations[ws],
        );

        let size = u32::from(self.size(tp));
        self.copy_block(src_ref, dest, size);
        // No copy_claims — caller verified the struct is self-contained.

        // Un-graft.
        std::mem::swap(
            &mut self.allocations[ws],
            &mut worker_stores.allocations[ws],
        );
    }

    /// Clone all current stores as locked read-only copies for use in a worker thread.
    /// The returned `Stores` has the same type schema but no files and no `parallel_ctx`.
    /// When a worker `State` is created from this, `State::new()` will allocate its own
    /// stack store at index `self.max` without conflicting with the cloned data stores.
    /// Freed slots (store.free == true) are replaced with fresh empty stores so that
    /// `State::new_worker → Stores::database` can safely re-initialise them without
    /// hitting the "Write to locked store" debug assert.
    #[must_use]
    pub fn clone_for_worker(&self) -> WorkerStores {
        let allocations = self
            .allocations
            .iter()
            .map(|s| {
                if s.free {
                    super::super::store::Store::new(100)
                } else {
                    // S29/P1-R3: use claims-free clone — workers never call validate()
                    s.clone_locked_for_worker()
                }
            })
            .collect();
        // S29: build a free_bits bitmap for the worker that reflects which slots are
        // free (main-thread freed slots become fresh empty stores in the worker clone,
        // so they are available for re-allocation by the worker).
        let mut free_bits: Vec<u64> = Vec::new();
        for (i, s) in self.allocations.iter().enumerate() {
            if s.free {
                let word = i / 64;
                let bit = i % 64;
                while free_bits.len() <= word {
                    free_bits.push(0);
                }
                free_bits[word] |= 1u64 << bit;
            }
        }
        WorkerStores::new(Stores {
            types: self.types.clone(),
            names: self.names.clone(),
            allocations,
            records_created: self.records_created,
            stores_allocated: self.stores_allocated,
            alloc_pc: self.alloc_pc,
            stack_store_at_zero: self.stack_store_at_zero,
            files: Vec::new(),
            max: self.max,
            peak: self.max,
            free_bits,
            bridge_text_dest: None,
            const_refs: self.const_refs.clone(),
            last_parse_errors: Vec::new(),
            last_json_errors: Vec::new(),
            parallel_ctx: None,
            par_buffer_stack: Vec::new(),
            par_text_buffer_stack: Vec::new(),
            par_ref_buffer_stack: Vec::new(),
            par_narrow_buffer_stack: Vec::new(),
            par_fn_buffer_stack: Vec::new(),
            par_fn_native_buffer_stack: Vec::new(),
            logger: self.logger.clone(),
            had_fatal: false,
            runtime_error: None,
            format_fault_tag: None,
            // #255 / @PLN9: a parallel worker's file ops must resolve paths the
            // same way as the main thread — carry the anchor + mode.
            source_dir: self.source_dir.clone(),
            program_relative: self.program_relative,
            frame_yield: false,
            poison_free: self.poison_free,
            disable_slot_reuse: self.disable_slot_reuse,
            uaf_freed_this_op: Vec::new(),
            worker_slot_dispenser: None,
            worker_allocated_indices: Vec::new(),
            report_asserts: false,
            assert_results: Vec::new(),
            user_args: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            start_time: self.start_time,
            #[cfg(target_arch = "wasm32")]
            start_time_ms: self.start_time_ms,
            call_stack_snapshot: Vec::new(),
            variables_snapshot: Vec::new(),
            closure_map: std::collections::HashMap::new(),
            jnull_sentinel: None,
        })
    }

    /// Produce a light-worker view — main stores borrowed read-only,
    /// pool stores provide allocation capacity.
    ///
    /// # Safety
    /// `pool_slice` must remain valid and exclusively owned by this worker.
    /// The original `Stores` must outlive the worker (guaranteed by `thread::scope`).
    pub unsafe fn clone_for_light_worker(&self, pool_slice: &mut [Store]) -> WorkerStores {
        // Borrow ALL stores — the input vector may reference any store.
        let mut allocations: Vec<Store> = self
            .allocations
            .iter()
            .map(|s| {
                if s.free {
                    Store::new_freed_sentinel()
                } else {
                    unsafe { s.borrow_locked_for_light_worker() }
                }
            })
            .collect();
        // Append pool stores as free slots for the worker's own allocations.
        for store in pool_slice.iter_mut() {
            store.init();
            store.free = true;
            // Take the store's buffer into the worker via a borrow with owned semantics.
            // The pool store keeps its buffer; after the scope the worker's stores are dropped
            // (borrowed flag prevents double-free for main stores; pool stores are NOT borrowed).
            allocations.push(Store::new(store.byte_capacity() as u32 / 8));
        }
        // Build free_bits: main-thread freed slots + all pool slots.
        let mut free_bits: Vec<u64> = Vec::new();
        for (i, s) in allocations.iter().enumerate() {
            if s.free {
                let word = i / 64;
                let bit = i % 64;
                while free_bits.len() <= word {
                    free_bits.push(0);
                }
                free_bits[word] |= 1u64 << bit;
            }
        }
        WorkerStores::new(Stores {
            types: self.types.clone(),
            names: self.names.clone(),
            allocations,
            records_created: self.records_created,
            stores_allocated: self.stores_allocated,
            alloc_pc: self.alloc_pc,
            stack_store_at_zero: self.stack_store_at_zero,
            files: Vec::new(),
            max: self.allocations.len() as u16 + pool_slice.len() as u16,
            peak: self.allocations.len() as u16 + pool_slice.len() as u16,
            free_bits,
            bridge_text_dest: None,
            const_refs: self.const_refs.clone(),
            last_parse_errors: Vec::new(),
            last_json_errors: Vec::new(),
            parallel_ctx: None,
            par_buffer_stack: Vec::new(),
            par_text_buffer_stack: Vec::new(),
            par_ref_buffer_stack: Vec::new(),
            par_narrow_buffer_stack: Vec::new(),
            par_fn_buffer_stack: Vec::new(),
            par_fn_native_buffer_stack: Vec::new(),
            logger: self.logger.clone(),
            had_fatal: false,
            runtime_error: None,
            format_fault_tag: None,
            // #255 / @PLN9: a parallel worker's file ops must resolve paths the
            // same way as the main thread — carry the anchor + mode.
            source_dir: self.source_dir.clone(),
            program_relative: self.program_relative,
            frame_yield: false,
            poison_free: self.poison_free,
            disable_slot_reuse: self.disable_slot_reuse,
            uaf_freed_this_op: Vec::new(),
            worker_slot_dispenser: None,
            worker_allocated_indices: Vec::new(),
            report_asserts: false,
            assert_results: Vec::new(),
            user_args: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            start_time: self.start_time,
            #[cfg(target_arch = "wasm32")]
            start_time_ms: self.start_time_ms,
            call_stack_snapshot: Vec::new(),
            variables_snapshot: Vec::new(),
            closure_map: std::collections::HashMap::new(),
            jnull_sentinel: None,
        })
    }

    #[must_use]
    pub fn store_nr(&self, nr: u16) -> &Store {
        &self.allocations[nr as usize]
    }

    /// #306 diagnostic (`LOFT_TRACE_CR`) — bounds-checked mirror of the
    /// `copy_claims` walk.  Reports (instead of faulting on) the first broken
    /// interior edges of `rec`'s claim graph: a text offset or vector record
    /// id outside its store's buffer, a freed/out-of-range store, or an
    /// insane vector length.  Diagnostic only; never dereferences unchecked.
    pub fn validate_claims(&self, rec: &DbRef, tp: u16, path: &str, problems: &mut u32) {
        if *problems > 8 {
            return;
        }
        if rec.store_nr as usize >= self.allocations.len() {
            eprintln!(
                "[cr-check] {path}: ref #{}.{},{} — store out of range",
                rec.store_nr, rec.rec, rec.pos
            );
            *problems += 1;
            return;
        }
        let store = &self.allocations[rec.store_nr as usize];
        if store.free {
            eprintln!(
                "[cr-check] {path}: ref #{}.{},{} — store FREED",
                rec.store_nr, rec.rec, rec.pos
            );
            *problems += 1;
            return;
        }
        let cap = store.capacity_words();
        if rec.rec >= cap || u64::from(rec.rec) * 8 + u64::from(rec.pos) >= u64::from(cap) * 8 {
            eprintln!(
                "[cr-check] {path}: ref #{}.{},{} — record beyond store capacity ({cap} words)",
                rec.store_nr, rec.rec, rec.pos
            );
            *problems += 1;
            return;
        }
        if (tp as usize) >= self.types.len() {
            eprintln!("[cr-check] {path}: type {tp} out of range");
            *problems += 1;
            return;
        }
        match &self.types[tp as usize].parts {
            Parts::Base if tp == 5 => {
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur != 0 && cur >= cap {
                    eprintln!(
                        "[cr-check] {path}: text offset {cur} beyond store #{} capacity {cap}",
                        rec.store_nr
                    );
                    *problems += 1;
                }
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields {
                    self.validate_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos: rec.pos + u32::from(f.position),
                        },
                        f.content,
                        &format!("{path}.{}", f.name),
                        problems,
                    );
                }
            }
            Parts::Vector(v) | Parts::Sorted(v, _) => {
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    return;
                }
                if cur >= cap {
                    eprintln!(
                        "[cr-check] {path}: vector rec {cur} beyond store #{} capacity {cap}",
                        rec.store_nr
                    );
                    *problems += 1;
                    return;
                }
                let len = store.get_u32_raw(cur, 4);
                let size = u32::from(self.size(*v));
                if u64::from(len) * u64::from(size) > u64::from(cap) * 8 {
                    eprintln!(
                        "[cr-check] {path}: vector rec {cur} len {len} (elem size {size}) \
                         exceeds store #{} capacity {cap} words",
                        rec.store_nr
                    );
                    *problems += 1;
                    return;
                }
                for i in 0..len.min(16) {
                    self.validate_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: cur,
                            pos: 8 + size * i,
                        },
                        *v,
                        &format!("{path}[{i}]"),
                        problems,
                    );
                }
            }
            Parts::ChildRec(ct) => {
                let r = store.get_u32_raw(rec.rec, rec.pos);
                if r == 0 {
                    return;
                }
                if r >= cap {
                    eprintln!(
                        "[cr-check] {path}: child rec {r} beyond store #{} capacity {cap}",
                        rec.store_nr
                    );
                    *problems += 1;
                    return;
                }
                self.validate_claims(
                    &DbRef {
                        store_nr: rec.store_nr,
                        rec: r,
                        pos: 8,
                    },
                    *ct,
                    &format!("{path}->child"),
                    problems,
                );
            }
            Parts::Enum(values) => {
                // Mirrors `copy_claims`' Enum arm (direct index, no -1 shift).
                let e_nr = store.get_byte(rec.rec, rec.pos, -1);
                if e_nr >= 0 && (e_nr as usize) < values.len() {
                    let etp = values[e_nr as usize].0;
                    if etp != u16::MAX {
                        self.validate_claims(rec, etp, path, problems);
                    }
                }
            }
            Parts::Hash(v, _) => {
                // Bounds-checked hash walk (the per-element kind for_each_owned_child
                // trusts; here it is guard-before-deref).  Mirrors hash.rs: the root
                // word holds the bucket-record claim; bucket layout is word 0 = room,
                // buckets from fld 16 (`BUCKET0`), `elms = (room - 2) * 2`.
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    return;
                }
                if cur >= cap {
                    eprintln!(
                        "[cr-check] {path}: hash bucket rec {cur} beyond store #{} capacity {cap}",
                        rec.store_nr
                    );
                    *problems += 1;
                    return;
                }
                let room = store.get_u32_raw(cur, 0);
                if room < 2 || u64::from(room) > u64::from(cap) {
                    eprintln!(
                        "[cr-check] {path}: hash bucket rec {cur} insane room {room} (cap {cap})",
                    );
                    *problems += 1;
                    return;
                }
                let elms = (room - 2) * 2;
                for i in 0..elms {
                    let entry = store.get_u32_raw(cur, 16 + i * 4);
                    if entry == 0 {
                        continue;
                    }
                    if entry >= cap {
                        eprintln!(
                            "[cr-check] {path}: hash entry rec {entry} beyond store #{} capacity {cap}",
                            rec.store_nr
                        );
                        *problems += 1;
                        if *problems > 8 {
                            return;
                        }
                        continue;
                    }
                    self.validate_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: entry,
                            pos: 8,
                        },
                        *v,
                        &format!("{path}{{#{entry}}}"),
                        problems,
                    );
                }
            }
            _ => {}
        }
    }

    /// `tp` is the CONTAINER type (`Vector` / `Sorted`), not the content type — the
    /// same convention as `array_body` / `hash_body` / `index_body`, so the keystone
    /// walk keys on the container's `Parts` and the content type is read back out.
    pub(super) fn copy_claims_seq_vector(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let content_tp = match &self.types[tp as usize].parts {
            Parts::Vector(v) | Parts::Sorted(v, _) => *v,
            other => {
                panic!("copy_claims_seq_vector called with non-vector type {tp} (parts: {other:?})")
            }
        };
        let length = vector::length_vector(rec, &self.allocations);
        let size = u32::from(self.size(content_tp));
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        if cur == 0 {
            self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
            return;
        }
        let into = self.store_mut(to).claim(1 + (size * length).div_ceil(8));
        debug_assert!(
            i32::try_from(into).is_ok(),
            "vector allocation offset overflow: {into}"
        );
        self.store_mut(to).set_u32_raw(to.rec, to.pos, into);
        // DESTINATION build: one bulk copy of the whole element block (elements are
        // INLINE in the container, unlike array/hash/index where each is a separate
        // record).  Keep it — the keystone only enumerates the SOURCE; it does not
        // build the destination.
        self.copy_block(
            &DbRef {
                store_nr: rec.store_nr,
                rec: cur,
                pos: 4,
            },
            &DbRef {
                store_nr: to.store_nr,
                rec: into,
                pos: 4,
            },
            length * size + 4,
        );
        // SOURCE enumeration reads the keystone walk — the single home of the
        // per-`Parts` source layout.  Its `Vector`/`Sorted` arm yields one child per
        // element at `pos = 8 + size*i` with `owning_elem: None` (inline, no separate
        // record).  We iterate it ALONGSIDE the bulk copy above (two passes over the
        // same elements, deliberately not merged — the bulk copy moves the bytes, this
        // pass deep-copies the nested claims).  The bulk copy laid the destination out
        // byte-identically, so each element sits at the SAME offset in `into`; reuse
        // `child.pos` for the destination instead of recomputing `8 + size*i`.
        let children = self.for_each_owned_child(rec, tp).children;
        debug_assert_eq!(
            u32::try_from(children.len()).unwrap_or(u32::MAX),
            length,
            "keystone element count disagrees with the vector length header (tp={tp})"
        );
        for child in children {
            self.copy_claims(
                &child.child,
                &DbRef {
                    store_nr: to.store_nr,
                    rec: into,
                    pos: child.child.pos,
                },
                child.child_tp,
            );
        }
    }

    /// `tp` is the CONTAINER type (`Array` / `Ordered`), not the content type: the
    /// keystone walk is keyed on the container's `Parts`.  The content type is read
    /// back out of it, so the caller no longer has to know which of the two it is.
    pub(super) fn copy_claims_array_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let content_tp = match &self.types[tp as usize].parts {
            Parts::Array(v) | Parts::Ordered(v, _) => *v,
            other => {
                panic!("copy_claims_array_body called with non-array type {tp} (parts: {other:?})")
            }
        };
        let length = vector::length_vector(rec, &self.allocations);
        let size = u32::from(self.size(content_tp));
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        if cur == 0 {
            self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
            return;
        }
        // @P309 — claim by element COUNT (header word + one 4-byte rec-id
        // slot per element, 2 slots/word), NOT by `cur` (the source
        // structure's rec-id, which is meaningless as a size), and WRITE THE
        // LENGTH HEADER (offset 4) — without it the copied `array`/`ordered`
        // read back as length 0 (silent data loss; e.g. a `sorted<T>` field
        // becomes an `ordered<T>` secondary index when an `index<T>` exists,
        // and deep-copying the owning struct lost its elements).
        let into = self.store_mut(to).claim(1 + length.div_ceil(2));
        self.store_mut(to).set_u32_raw(to.rec, to.pos, into);
        self.store_mut(to).set_u32_raw(into, 4, length);
        // SOURCE enumeration reads the keystone walk — the single home of the
        // per-`Parts` source layout.  Its `Array`/`Ordered` arm computes the same
        // `get_u32_raw(cur, 8 + i * 4)` this loop used to, and hands each element
        // record back as `owning_elem`, so the fold is position-for-position.
        // The DESTINATION build below (claim → `copy_block` → rec-id slot → recurse)
        // stays per-kind, including the @P309 length header written above.
        let elems: Vec<u32> = self
            .for_each_owned_child(rec, tp)
            .children
            .into_iter()
            .filter_map(|c| c.owning_elem)
            .collect();
        // The keystone reads the same `length_vector` header, so the counts are one
        // fact seen twice.  Assert it rather than let a future divergence write the
        // header for N and fill N-1 slots (the @P309 shape) — the nightly
        // debug-assertions gate runs this over the whole interpreter corpus.
        debug_assert_eq!(
            u32::try_from(elems.len()).unwrap_or(u32::MAX),
            length,
            "keystone element count disagrees with the vector length header (tp={tp})"
        );
        for (i, elm) in elems.into_iter().enumerate() {
            let i = u32::try_from(i).unwrap_or(u32::MAX);
            // @PLN102 heap copy/alias audit — an `elm == 0` slot is an ABSENT element
            // (no record). Every runtime path keeps `length` in sync with the filled
            // slots (append/copy fill, `#remove` compacts, nullable elements stay inline
            // so a null is never a 0 rec-id), so this is unreachable today — but the walk
            // must NOT dereference record 0 (it is reserved). Preserve the hole (slot ← 0)
            // and skip the deref, keeping positions and the length header intact rather
            // than fabricating an element from record 0. Mirrors the free-side skip below.
            if elm == 0 {
                self.store_mut(to).set_u32_raw(into, 8 + 4 * i, 0);
                continue;
            }
            let new = self.store_mut(to).claim(size.div_ceil(8));
            self.copy_block(
                &DbRef {
                    store_nr: rec.store_nr,
                    rec: elm,
                    pos: 4,
                },
                &DbRef {
                    store_nr: to.store_nr,
                    rec: new,
                    pos: 4,
                },
                size - 4,
            );
            self.store_mut(to).set_u32_raw(into, 8 + 4 * i, new);
            self.copy_claims(
                &DbRef {
                    store_nr: rec.store_nr,
                    rec: elm,
                    pos: 8,
                },
                &DbRef {
                    store_nr: to.store_nr,
                    rec: new,
                    pos: 8,
                },
                content_tp,
            );
        }
    }

    /// Deep-copy a `hash<T[key]>` field from `rec` into `to`.  `tp` is the
    /// HASH type (not the content type).
    ///
    /// @P318 — re-INSERT each entry into an emptied destination via `hash::add`
    /// rather than copying the bucket array slot-for-slot.  A hash's bucket
    /// layout (slot count + offsets, now including a per-hash seed word) lives
    /// only in `src/hash.rs`; `room` comes from the record's SIZE HEADER
    /// (offset 0) — and `Store::claim` may hand back a block LARGER than
    /// requested (it only splits when the surplus exceeds 1/3; see
    /// `Store::claim_block`).  The old slot-for-slot copy laid entries out for
    /// the SOURCE's `room`, but the destination record's header (its own
    /// `room`) could differ, so `hash::find` later probed `key % dest_elms` —
    /// a DIFFERENT start slot — and missed entries.  That surfaced only as a
    /// wrong / NON-deterministic result (whether `claim` over-sizes depends on
    /// free-list state) and was immune to `zero_claim` (the probe START shifts,
    /// not just the slack).  Re-inserting rebuilds the destination consistently
    /// with its own `room`, exactly as the source was built — mirroring
    /// `copy_claims_index_body`.  `hash::add` also maintains the length word and
    /// rehash invariants, subsuming the earlier @P317 length-word and @P290
    /// loop-bound fixes.
    pub(super) fn copy_claims_hash_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        // Start the destination as an empty hash; `hash::add` claims the bucket
        // record on the first insert and rehashes (growing `room`) as it fills.
        self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
        if cur == 0 {
            return;
        }
        let content_tp = match &self.types[tp as usize].parts {
            Parts::Hash(c, _) => *c,
            other => {
                panic!("copy_claims_hash_body called with non-hash type {tp} (parts: {other:?})")
            }
        };
        let size = u32::from(self.size(content_tp));
        let keys = self.types[tp as usize].keys.clone();
        // Source-bucket enumeration reads the SAME keystone walk `remove_claims`
        // uses (`for_each_owned_child` → `hash::records`), so the bucket layout
        // lives in ONE place.  Each child carries its element record in
        // `owning_elem`; re-insert that entry into the emptied destination.
        for child in self.for_each_owned_child(rec, tp).children {
            let Some(elm) = child.owning_elem else {
                continue;
            };
            // @P295 — element record layout (per `record_new`'s Hash arm):
            // offset 0 = header, offset 4 = back-pointer to the parent record,
            // offset 8 = struct payload (`size` bytes).  Claim WITH the header
            // word, set the back-pointer, copy the full `size`-byte payload from
            // offset 8, deep-copy its nested claims, then re-insert by key.
            let new = self.store_mut(to).claim(1 + size.div_ceil(8));
            self.store_mut(to).set_u32_raw(new, 4, to.rec);
            let src_db = DbRef {
                store_nr: rec.store_nr,
                rec: elm,
                pos: 8,
            };
            let new_db = DbRef {
                store_nr: to.store_nr,
                rec: new,
                pos: 8,
            };
            self.copy_block(&src_db, &new_db, size);
            self.copy_claims(&src_db, &new_db, content_tp);
            hash::add(to, &new_db, &mut self.allocations, &keys);
        }
    }

    /// Deep-copy a `Radix` collection: rebuild the destination tree by re-inserting
    /// each source element.  A radix tree cannot be byte-copied — node ids and the
    /// container relocate — so, like `copy_claims_hash_body`, it re-inserts entry by
    /// entry through the same keystone walk `remove_claims` uses.
    pub(super) fn copy_claims_radix_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        // Start the destination as an empty tree; `radix_db::add` claims + grows it.
        self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        if cur == 0 {
            return;
        }
        let content_tp = match &self.types[tp as usize].parts {
            Parts::Radix(c, _) => *c,
            other => panic!("copy_claims_radix_body called with non-radix type {tp} ({other:?})"),
        };
        let size = u32::from(self.size(content_tp));
        let keys = self.types[tp as usize].keys.clone();
        for child in self.for_each_owned_child(rec, tp).children {
            let Some(elm) = child.owning_elem else {
                continue;
            };
            // Element layout (record_new): header, back-pointer at 4, payload at 8.
            let new = self.store_mut(to).claim(1 + size.div_ceil(8));
            self.store_mut(to).set_u32_raw(new, 4, to.rec);
            let src_db = DbRef {
                store_nr: rec.store_nr,
                rec: elm,
                pos: 8,
            };
            let new_db = DbRef {
                store_nr: to.store_nr,
                rec: new,
                pos: 8,
            };
            self.copy_block(&src_db, &new_db, size);
            self.copy_claims(&src_db, &new_db, content_tp);
            radix_db::add(to, &new_db, &mut self.allocations, &keys);
        }
    }

    /// Collect all record numbers in an RB-tree index by in-order traversal.
    /// `rec` points to the i32 tree-root field; `left` is `self.fields(index_tp)`.
    pub(super) fn collect_index_nodes(&self, rec: &DbRef, left: u16) -> Vec<u32> {
        let mut nodes = Vec::new();
        let mut curr = tree::first(rec, left, &self.allocations).rec;
        while curr != 0 {
            nodes.push(curr);
            curr = tree::next(
                &self.allocations[rec.store_nr as usize],
                &DbRef {
                    store_nr: rec.store_nr,
                    rec: curr,
                    pos: u32::from(left),
                },
            );
        }
        nodes
    }

    /// Deep-copy an `index<T>` field from `rec` into `to`.
    /// `tp` is the index type (not the content type).
    pub(super) fn copy_claims_index_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        if cur == 0 {
            self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
            return;
        }
        let left = self.fields(tp);
        let content_tp = match &self.types[tp as usize].parts {
            Parts::Index(c, _, _) => *c,
            other => {
                panic!("copy_claims_index_body called with non-index type {tp} (parts: {other:?})")
            }
        };
        let size = u32::from(self.size(content_tp));
        let keys = self.types[tp as usize].keys.clone();
        // SOURCE enumeration reads the keystone walk — the single home of the
        // per-`Parts` source layout — exactly as `remove_claims` and
        // `copy_claims_hash_body` do.  The keystone's `Index` arm makes the same
        // `collect_index_nodes(rec, left)` call and hands each node back as
        // `owning_elem`, so this is position-for-position the walk it replaces.
        // The DESTINATION build (claim → back-pointer → `copy_block` → recurse →
        // `tree::add`) stays per-kind: unifying it is how @P318/@P309 come back.
        let nodes: Vec<u32> = self
            .for_each_owned_child(rec, tp)
            .children
            .into_iter()
            .filter_map(|c| c.owning_elem)
            .collect();
        // Initialize the destination tree root to empty before inserting.
        self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
        for src_node in nodes {
            // Allocate element record in the destination store.
            let dst_node = self.store_mut(to).claim(1 + size.div_ceil(8));
            // Back-reference to the destination parent record (offset 4).
            self.store_mut(to).set_u32_raw(dst_node, 4, to.rec);
            // Bulk-copy element data bytes (pos=8, size bytes).
            self.copy_block(
                &DbRef {
                    store_nr: rec.store_nr,
                    rec: src_node,
                    pos: 8,
                },
                &DbRef {
                    store_nr: to.store_nr,
                    rec: dst_node,
                    pos: 8,
                },
                size,
            );
            // Deep-copy nested claims (strings, sub-structures).
            self.copy_claims(
                &DbRef {
                    store_nr: rec.store_nr,
                    rec: src_node,
                    pos: 8,
                },
                &DbRef {
                    store_nr: to.store_nr,
                    rec: dst_node,
                    pos: 8,
                },
                content_tp,
            );
            // Insert into the destination tree; tree::add initialises nav fields.
            tree::add(
                to,
                &DbRef {
                    store_nr: to.store_nr,
                    rec: dst_node,
                    pos: 8,
                },
                left,
                &mut self.allocations,
                &keys,
            );
        }
    }

    /**
    Copy string fields and substructures from `rec` to `to`.
    # Panics
    When a field points to a spatial structure.
    */
    pub fn copy_claims(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        // TODO prevent copying secondary structures
        match &self.types[tp as usize].parts {
            Parts::Base if tp == 5 => {
                // text — P220: discriminate null source from empty-string
                // source.  `get_str(0)` returns `STRING_NULL` ("\0"), so the
                // old `s.is_empty()` check never fired for a null source
                // (s would be "\0", not "") — but it DID fire for a
                // genuinely empty `""` source, writing 0 (the null
                // sentinel) into the destination and silently
                // re-classifying the value as null.  Result: a `""`
                // element copied through this path read back as `null`.
                // Discriminate on the source `cur` instead.
                let store = self.store(rec);
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
                } else {
                    let s = store.get_str(cur);
                    let into = self.store_mut(to);
                    let s_pos = into.set_str(s);
                    into.set_u32_raw(to.rec, to.pos, s_pos);
                }
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields.clone() {
                    self.copy_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos: rec.pos + u32::from(f.position),
                        },
                        &DbRef {
                            store_nr: to.store_nr,
                            rec: to.rec,
                            pos: to.pos + u32::from(f.position),
                        },
                        f.content,
                    );
                }
            }
            Parts::Vector(_) | Parts::Sorted(_, _) => {
                // Pass the CONTAINER type: the keystone walk keys on it, and the helper
                // reads the content type back out (same shape as array/hash/index).
                self.copy_claims_seq_vector(rec, to, tp);
            }
            Parts::ChildRec(content_kt) => {
                // P213: read source rec-id; if 0 (empty / non-capturing),
                // clear destination.  Otherwise claim a fresh record in
                // dest's Store, byte-copy the child's payload, recurse on
                // the child's nested heap fields, and write the new rec-id
                // into dest's field.
                let src_rec = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if src_rec == 0 {
                    self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
                } else {
                    let content_kt = *content_kt;
                    let size = u32::from(self.size(content_kt));
                    let new_rec = self.allocations[to.store_nr as usize].claim(size);
                    let src_db = DbRef {
                        store_nr: rec.store_nr,
                        rec: src_rec,
                        pos: 8,
                    };
                    let new_db = DbRef {
                        store_nr: to.store_nr,
                        rec: new_rec,
                        pos: 8,
                    };
                    // Cross-store byte copy of payload.
                    if rec.store_nr == to.store_nr {
                        self.store_mut(to)
                            .copy_block(src_rec, 8, new_rec, 8, size as isize);
                    } else {
                        // @PLAN53 cluster 3: sound disjoint-borrow cross-store copy
                        // (src_db.store_nr == rec.store_nr != to.store_nr in this else).
                        self.copy_block_cross_store(
                            src_db.store_nr,
                            src_rec,
                            8,
                            to.store_nr,
                            new_rec,
                            8,
                            size as isize,
                        );
                    }
                    // Deep-copy nested heap fields (text, Reference, etc.).
                    self.copy_claims(&src_db, &new_db, content_kt);
                    self.store_mut(to).set_u32_raw(to.rec, to.pos, new_rec);
                }
            }
            Parts::Array(_) | Parts::Ordered(_, _) => {
                // Pass the CONTAINER type: the keystone walk is keyed on it, and the
                // helper reads the content type back out (same shape as `Hash`/`Index`).
                self.copy_claims_array_body(rec, to, tp);
            }
            Parts::Hash(_, _) => {
                self.copy_claims_hash_body(rec, to, tp);
            }
            Parts::Radix(_, _) => self.copy_claims_radix_body(rec, to, tp),
            Parts::Index(_, _, _) => self.copy_claims_index_body(rec, to, tp),
            Parts::Enum(values) => {
                let e_nr = self.store(rec).get_byte(rec.rec, rec.pos, -1);
                // @PLN25 — `get_byte(.., -1)` applies a -1 shift, so a valid variant is
                // `e_nr` in `0..values.len()` (stored byte 1 = variant 0).  A null/absent
                // inline enum reads NEGATIVE here (stored 0 → -1, an absent source rec →
                // i32::MIN) and carries no payload claims; skip rather than index `values`
                // out of bounds (matches `validate_claims`'s `>= 0` arm).  Covers
                // `vr[i] = null` and whole-vector copy of a null element.
                if e_nr >= 0 && (e_nr as usize) < values.len() {
                    let tp = values[e_nr as usize].0;
                    // Do not copy claims on simple enumerate types.
                    if tp != u16::MAX {
                        self.copy_claims(rec, to, tp);
                    }
                }
            }
            _ => {}
        }
    }

    /// @P317 debug — `LOFT_LOG=copy_check` (or `LOFT_COPY_CHECK=1`): after a
    /// deep-copy, walk SOURCE and DESTINATION in parallel and warn about every
    /// nested vector/array/hash length that DIFFERS — the signature of a
    /// `copy_claims` bug that silently inflates or truncates a nested
    /// collection (the kind valgrind can't see and that surfaces only as a
    /// wrong / non-deterministic result).  Read once, cached; zero cost off.
    #[inline]
    #[allow(clippy::unused_self)] // `&self` is for ergonomic call sites; the flag is a cached static
    pub(crate) fn copy_check_enabled(&self) -> bool {
        static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *FLAG.get_or_init(|| {
            std::env::var("LOFT_COPY_CHECK").is_ok()
                || std::env::var("LOFT_LOG")
                    .is_ok_and(|v| v.split([',', ':', ' ']).any(|p| p.trim() == "copy_check"))
        })
    }

    /// Does `tp` own nested heap worth recursing into?  Primitive vector
    /// elements (u8/integer/single/…) have none, so a length compare at the
    /// vector level suffices — skipping them avoids walking every element of a
    /// multi-thousand-element vector.
    fn has_nested_heap(&self, tp: u16) -> bool {
        matches!(
            self.types[tp as usize].parts,
            Parts::Struct(_)
                | Parts::EnumValue(_, _)
                | Parts::Vector(_)
                | Parts::Sorted(_, _)
                | Parts::Array(_)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Index(_, _, _)
                | Parts::ChildRec(_)
                | Parts::Enum(_)
        ) || tp == 5 // text
    }

    /// @P317 — entry point for the `copy_check` validator.  Compares the
    /// structure of a just-copied `src`/`dst` pair and prints `[copy_check]`
    /// warnings (deduped by field path, so a vector doesn't spam per element).
    /// Warn-and-continue: never panics, so one run gives a full census.
    pub fn report_copy_mismatches(&self, src: &DbRef, dst: &DbRef, tp: u16, label: &str) {
        if !self.copy_check_enabled() {
            return;
        }
        let mut seen = std::collections::HashSet::new();
        self.walk_copy_cmp(src, dst, tp, label.to_string(), &mut seen);
    }

    fn walk_copy_cmp(
        &self,
        src: &DbRef,
        dst: &DbRef,
        tp: u16,
        path: String,
        seen: &mut std::collections::HashSet<String>,
    ) {
        match &self.types[tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                let fields = fields.clone();
                for f in fields {
                    let s = DbRef {
                        store_nr: src.store_nr,
                        rec: src.rec,
                        pos: src.pos + u32::from(f.position),
                    };
                    let d = DbRef {
                        store_nr: dst.store_nr,
                        rec: dst.rec,
                        pos: dst.pos + u32::from(f.position),
                    };
                    self.walk_copy_cmp(&s, &d, f.content, format!("{path}.{}", f.name), seen);
                }
            }
            Parts::Vector(v) | Parts::Sorted(v, _) => {
                let v = *v;
                let sl = vector::length_vector(src, &self.allocations);
                let dl = vector::length_vector(dst, &self.allocations);
                if sl != dl && seen.insert(path.clone()) {
                    eprintln!(
                        "[copy_check] MISMATCH {path}: src_len={sl} dst_len={dl} (vector elem kt={v})"
                    );
                }
                if self.has_nested_heap(v) {
                    let s_cur = self.store(src).get_u32_raw(src.rec, src.pos);
                    let d_cur = self.store(dst).get_u32_raw(dst.rec, dst.pos);
                    if s_cur != 0 && d_cur != 0 {
                        let size = u32::from(self.size(v));
                        for i in 0..sl.min(dl) {
                            let s = DbRef {
                                store_nr: src.store_nr,
                                rec: s_cur,
                                pos: 8 + size * i,
                            };
                            let d = DbRef {
                                store_nr: dst.store_nr,
                                rec: d_cur,
                                pos: 8 + size * i,
                            };
                            self.walk_copy_cmp(&s, &d, v, format!("{path}[]"), seen);
                        }
                    }
                }
            }
            Parts::Array(v) | Parts::Ordered(v, _) => {
                let v = *v;
                let sl = vector::length_vector(src, &self.allocations);
                let dl = vector::length_vector(dst, &self.allocations);
                if sl != dl && seen.insert(path.clone()) {
                    eprintln!(
                        "[copy_check] MISMATCH {path}: src_len={sl} dst_len={dl} (array/ordered kt={v})"
                    );
                }
            }
            Parts::Hash(v, _) => {
                let v = *v;
                let s_cur = self.store(src).get_u32_raw(src.rec, src.pos);
                let d_cur = self.store(dst).get_u32_raw(dst.rec, dst.pos);
                if s_cur == 0 || d_cur == 0 {
                    return;
                }
                let s_room = self.store(src).get_u32_raw(s_cur, 0);
                let d_room = self.store(dst).get_u32_raw(d_cur, 0);
                let s_len = self.store(src).get_u32_raw(s_cur, 4);
                let d_len = self.store(dst).get_u32_raw(d_cur, 4);
                if s_len != d_len && seen.insert(format!("{path}#count")) {
                    eprintln!(
                        "[copy_check] MISMATCH {path}: src_count={s_len} dst_count={d_len} (hash)"
                    );
                }
                // copy_claims_hash_body copies bucket-by-bucket, so when the two
                // tables have equal `room` the bucket index pairs src↔dst.
                if s_room == d_room && self.has_nested_heap(v) {
                    let elms = s_room.saturating_sub(1) * 2;
                    for i in 0..elms {
                        let se = self.store(src).get_u32_raw(s_cur, 8 + 4 * i);
                        let de = self.store(dst).get_u32_raw(d_cur, 8 + 4 * i);
                        if se != 0 && de != 0 {
                            let s = DbRef {
                                store_nr: src.store_nr,
                                rec: se,
                                pos: 8,
                            };
                            let d = DbRef {
                                store_nr: dst.store_nr,
                                rec: de,
                                pos: 8,
                            };
                            self.walk_copy_cmp(&s, &d, v, format!("{path}[]"), seen);
                        }
                    }
                }
            }
            Parts::ChildRec(content_kt) => {
                let content_kt = *content_kt;
                let sc = self.store(src).get_u32_raw(src.rec, src.pos);
                let dc = self.store(dst).get_u32_raw(dst.rec, dst.pos);
                if sc != 0 && dc != 0 {
                    let s = DbRef {
                        store_nr: src.store_nr,
                        rec: sc,
                        pos: 8,
                    };
                    let d = DbRef {
                        store_nr: dst.store_nr,
                        rec: dc,
                        pos: 8,
                    };
                    self.walk_copy_cmp(&s, &d, content_kt, path, seen);
                }
            }
            Parts::Enum(values) => {
                let e_nr = self.store(src).get_byte(src.rec, src.pos, -1);
                if let Some(&(vtp, _)) = values.get(e_nr as usize)
                    && vtp != u16::MAX
                {
                    // Re-borrow-safe: `vtp` is copied out before recursing.
                    self.walk_copy_cmp(src, dst, vtp, path, seen);
                }
            }
            _ => {}
        }
    }

    /**
    Remove claimed data for a record. Both strings and substructures are freed.
    It will not free the record itself because that might be a part of a vector.

    Reads the [`Stores::for_each_owned_child`] keystone for every cascade kind
    (struct / enum / vector / sorted / array / ordered / hash / index / childrec):
    one walk recurses into each owned child, frees the per-element record it lived
    in, then frees the container block and clears the field pointer.  Only the text
    leaf and the (unimplemented) `Radix` teardown stay special-cased.
    # Panics
    When a field points to a spatial structure (teardown unimplemented).
    */
    pub fn remove_claims(&mut self, rec: &DbRef, tp: u16) {
        // A null/absent container (the `store_nr == u16::MAX` sentinel) owns nothing to tear
        // down — no-op, mirroring `free_named`'s own guard. The `for_each_owned_child` keystone
        // guards absent CHILDREN (`cur != 0`), but not a null CONTAINER; without this a nullable
        // DbRef passed as the container would index `allocations[u16::MAX]` and panic. The
        // invariant now holds HERE, not by every caller pre-checking the container.
        if rec.store_nr == u16::MAX {
            return;
        }
        // TODO prevent removing records twice via secondary structures
        match &self.types[tp as usize].parts {
            Parts::Base if tp == 5 => {
                // Text leaf: free the string record and clear the pointer.  Not a
                // cascade kind (no owned children), so it stays out of the keystone.
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 {
                    let cap = self.store(rec).capacity_words();
                    // (d) cluster-462 stale-interior-claim guard — the slice the
                    // copy-source detectors miss. The DESTINATION record's text-field
                    // pointer is read here and `delete()`-ed; if the dst slot was reused
                    // and this field holds a STALE pointer past the store's end, the
                    // delete reads a record header OOB → SIGSEGV (release skips the
                    // bounds debug_assert). Under LOFT_UAF* name the bad field and SKIP
                    // the delete (leak, not crash) so the run continues and the site is
                    // located — this is what catches the #462 sim.loft:3546 fault.
                    if crate::keys::uaf_any_enabled() && cur >= cap {
                        let rec_ok = self.store(rec).valid(rec.rec, rec.pos);
                        let (sfree, skt) = {
                            let a = &self.allocations[rec.store_nr as usize];
                            (a.free, a.known_type)
                        };
                        thread_local! {
                            static RPT: std::cell::RefCell<std::collections::HashSet<(u16, u32, u32)>> =
                                std::cell::RefCell::new(std::collections::HashSet::new());
                        }
                        if RPT.with(|s| s.borrow_mut().insert((rec.store_nr, rec.rec, rec.pos))) {
                            eprintln!(
                                "[uaf-claim] remove_claims: TEXT field at store #{} rec={} pos={} \
                                 holds STALE pointer cur={cur} past store end (cap_words={cap}; \
                                 slot free={sfree} known_type={skt} rec_pos_valid={rec_ok}) — dst \
                                 record has a dangling interior claim (reused-slot or borrowed-ref \
                                 over-free); skipping delete to avoid SIGSEGV",
                                rec.store_nr, rec.rec, rec.pos,
                            );
                        }
                        self.store_mut(rec).set_u32_raw(rec.rec, rec.pos, 0);
                    } else {
                        let store = self.store_mut(rec);
                        store.delete(cur);
                        store.set_u32_raw(rec.rec, rec.pos, 0);
                    }
                }
            }
            // `Radix` teardown is unimplemented; the keystone yields nothing for
            // it, so guard explicitly to preserve the loud failure (a silent no-op
            // would leak).
            // Every owned-child cascade kind (Struct/Enum/Vector/Sorted/Array/
            // Ordered/Hash/Index/ChildRec) reads the SINGLE keystone walk: recurse
            // into each child, free the per-element record it lived in (Array/Hash/
            // Index), then free the container block and clear the field pointer.
            // The historical loop-bound / slot-drift / length-header bugs
            // (@P290/@P306/@P318/@P309) lived in the per-dispatcher copies of this
            // walk; reading it once removes them by construction.
            _ => {
                let walk = self.for_each_owned_child(rec, tp);
                for c in walk.children {
                    // @PLN102 heap-free audit — an `owning_elem == Some(0)` slot is an
                    // ABSENT element record (Array/Ordered/Hash/Index). Recursing into it
                    // would walk reserved record 0 as if it were a live element, and the
                    // `delete(0)` below would free it. Unreachable today (the length/slots
                    // invariant holds it off), guarded by construction so a future desync
                    // can never fault here. Mirrors the copy-side skip in
                    // `copy_claims_array_body`.
                    if c.owning_elem == Some(0) {
                        continue;
                    }
                    self.remove_claims(&c.child, c.child_tp);
                    if let Some(elm) = c.owning_elem {
                        self.store_mut(rec).delete(elm);
                    }
                }
                if let Some(cur) = walk.container_rec {
                    self.store_mut(rec).delete(cur);
                }
                if walk.zero_field {
                    self.store_mut(rec).set_u32_raw(rec.rec, rec.pos, 0);
                }
            }
        }
    }

    /// `LOFT_WATCH_STORE` (cluster-462 write-watch) — read-only sibling of `remove_claims`:
    /// walk the record's text fields and return the first whose stored pointer is
    /// out-of-bounds (`>= capacity_words`, i.e. would fault a later `delete`/`get_str`),
    /// as `(field_pos, bad_pointer)`. Never deletes, never recurses through a bad pointer
    /// (it only follows OWNED-CHILD edges, which `for_each_owned_child` derives from the
    /// type, not from heap pointers — so the walk itself cannot fault). Used to NAME the
    /// copy that first wrote a garbage text-pointer into the watched store.
    #[must_use]
    pub fn first_oob_text(&self, rec: &DbRef, tp: u16) -> Option<(u32, u32)> {
        match &self.types[tp as usize].parts {
            Parts::Base if tp == 5 => {
                let store = self.store(rec);
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur != 0 && cur >= store.capacity_words() {
                    Some((rec.pos, cur))
                } else {
                    None
                }
            }
            _ => {
                let walk = self.for_each_owned_child(rec, tp);
                for c in walk.children {
                    if let Some(hit) = self.first_oob_text(&c.child, c.child_tp) {
                        return Some(hit);
                    }
                }
                None
            }
        }
    }

    /// `LOFT_WATCH_STORE` write-watch — call right AFTER a copy/append that may have
    /// written `to` (type `tp`). If the watched store now holds an out-of-bounds text
    /// pointer, report the writing op (pc/line via `crash_report`) + whether the SOURCE
    /// already had the bad pointer (`src_oob` distinguishes propagated-from-source vs
    /// introduced-here). `ctx` names the path. No-op unless `to.store_nr` is watched.
    pub fn watch_oob_text(&self, to: &DbRef, tp: u16, src: Option<&DbRef>, ctx: &str) {
        if crate::keys::watch_store() != Some(to.store_nr) {
            return;
        }
        let Some((bad_pos, bad_cur)) = self.first_oob_text(to, tp) else {
            return;
        };
        let src_oob = src.and_then(|s| self.first_oob_text(s, tp));
        let (pc, _op, _d) = crate::crash_report::last_context();
        let line = crate::crash_report::source_loc_for_pc(pc).map_or(0, |p| p.line);
        let cap = self.store(to).capacity_words();
        thread_local! {
            static RPT_W: std::cell::RefCell<std::collections::HashSet<(u16, u32, u32)>> =
                std::cell::RefCell::new(std::collections::HashSet::new());
        }
        if RPT_W.with(|s| s.borrow_mut().insert((to.store_nr, to.rec, bad_pos))) {
            let (s_nr, s_rec, s_pos) = src.map_or((u16::MAX, 0, 0), |s| (s.store_nr, s.rec, s.pos));
            eprintln!(
                "[watch-store/{ctx}] OOB text-ptr now in watched store #{}: dst rec={} field \
                 pos={bad_pos} holds ptr={bad_cur} (cap_words={cap}, tp={tp}) — src=#{s_nr}(rec={s_rec},\
                 pos={s_pos}) src_oob_text={src_oob:?} — at pc={pc} (line {line})",
                to.store_nr, to.rec,
            );
        }
    }

    /// @PLAN38 — bind the Store at `slot` to a file at `path`, returning
    /// `true` on success.  Dryopea-driven "the hash IS the file" entry
    /// point: a freshly-allocated container (e.g. `hash<X[k]>::new()`)
    /// can be re-rooted onto disk in one call, after which all mutations
    /// are durable via mmap/msync without any explicit save loop.
    ///
    /// Two modes, selected by whether the file already exists:
    ///
    /// 1. **Fresh file (path does not exist or is empty):** the current
    ///    in-memory Store's bytes are padded out to ≥ 1024 words with a
    ///    valid trailing free block, written to disk via
    ///    `MmapStorage::open` + `resize`, then `Store::open` mmaps it
    ///    back.  Caller-side `DbRef`s into this slot stay valid because
    ///    the on-disk record layout is byte-identical to the in-memory
    ///    one we just copied.
    ///
    /// 2. **Existing file:** `Store::open(path)` validates the SIGNATURE
    ///    and rebuilds the free-list; the in-memory state at this slot is
    ///    dropped in favour of the on-disk image.  Caller-side `DbRef`s
    ///    remain valid IFF the on-disk record at `rec=1` describes the
    ///    same type as the in-memory container the caller just made —
    ///    this is the load-on-startup path and assumes the consumer
    ///    pinned the type at allocation time (typical pattern: allocate
    ///    an empty `hash<X[k]>`, immediately call `bind_path`, then read
    ///    keys back).
    ///
    /// Failure modes (return `false`):
    /// - `slot >= self.allocations.len()`.
    /// - Path is empty / not valid UTF-8 / I/O error writing the snapshot.
    /// - Existing-file branch encounters a bad SIGNATURE (caught via
    ///   `std::panic::catch_unwind`, since `Store::open` panics on
    ///   format mismatch).
    ///
    /// Metadata preserved across the swap: `ref_count`, `known_type`,
    /// `free`, `created_at`, `last_op_at`, `lock_origin` (the bookkeeping
    /// the `Stores` collection needs to keep the slot accounted for in
    /// the bitmap and the rc walk).  Cleared on the new Store:
    /// `claims` (init() wipes it; matches `database_named` post-swap
    /// state), `borrowed`, `read_only`.
    ///
    /// Off when the `mmap` feature is disabled: returns `false`.
    #[cfg(feature = "mmap")]
    pub fn bind_path(&mut self, slot: u16, path: &std::path::Path) -> bool {
        let slot_idx = slot as usize;
        if slot_idx >= self.allocations.len() {
            return false;
        }
        let path_str = match path.to_str() {
            Some(s) if !s.is_empty() => s,
            _ => return false,
        };

        // Distinguish fresh vs existing by file size — a valid loft Store
        // file is always ≥ 8 bytes (SIGNATURE + free-space index).  A
        // smaller or missing file is "fresh" and triggers the
        // snapshot-then-mmap path.
        let exists = std::fs::metadata(path).is_ok_and(|m| m.len() >= 8);

        // Preserve the slot's bookkeeping across the swap.
        let preserved = {
            let s = &self.allocations[slot_idx];
            (s.known_type, s.free, s.created_at, s.last_op_at, s.pinned)
        };

        if !exists {
            // FRESH PATH — snapshot current bytes, pad to a valid ≥ 1024-word
            // image, write to disk, then re-open via mmap.
            let src_words = self.allocations[slot_idx].capacity_words();
            let snapshot = {
                let s = &self.allocations[slot_idx];
                let raw =
                    unsafe { std::slice::from_raw_parts(s.base_ptr(), (src_words as usize) * 8) };
                raw.to_vec()
            };
            let target_words = src_words.max(1024);
            let padded = match build_padded_store_image(&snapshot, src_words, target_words) {
                Some(b) => b,
                None => return false,
            };
            if std::fs::write(path, &padded).is_err() {
                return false;
            }
        }

        // Both branches: open the file via Store::open.  Wrapped in
        // catch_unwind because Store::open panics on signature mismatch
        // for the existing-file branch — we surface that as a clean
        // `false` return instead of crashing the interpreter.
        let new_store = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::store::Store::open(path_str)
        }));
        let mut new_store = match new_store {
            Ok(s) => s,
            Err(_) => return false,
        };

        // Re-apply preserved metadata onto the new Store.  Slot bitmap sees
        // continuity; the on-disk bytes carry the user data.
        new_store.known_type = preserved.0;
        new_store.free = preserved.1;
        new_store.created_at = preserved.2;
        new_store.last_op_at = preserved.3;
        new_store.pinned = preserved.4;

        self.allocations[slot_idx] = new_store;
        // @PLN97 3b.5 — record the layout identity beside the store
        // (`<path>.dschema`) so a later (possibly remote) working-set load can
        // reject a mismatched layout BEFORE range-reading foreign bytes at
        // schema-derived offsets. Best-effort: a store without a sidecar just
        // falls back to the post-copy `store_verify` backstop on load.
        if preserved.0 != u16::MAX {
            let id = crate::schema_sidecar::LayoutIdentity::of(self, &[preserved.0]);
            if id.write_beside(path).is_err() && std::env::var_os("LOFT_LOADER_STATS").is_some() {
                eprintln!(
                    "store_persist_bind: could not write layout sidecar beside {}",
                    path.display()
                );
            }
        }
        true
    }

    /// No-op shim when the `mmap` feature is disabled.  Always returns
    /// `false` so consumers branch into their non-mmap fallback (today,
    /// JSON via `text as Struct`).  Avoids `cfg`-gating every caller.
    #[cfg(not(feature = "mmap"))]
    #[allow(clippy::unused_self)]
    pub fn bind_path(&mut self, _slot: u16, _path: &std::path::Path) -> bool {
        false
    }

    /// Load a persisted store image at `path` into `slot`, HEAP-backed — the
    /// portable, non-durable counterpart of [`bind_path`](Stores::bind_path).
    /// Unlike `bind_path` there is no fresh/create branch (you load an
    /// EXISTING store) and no mmap, so it works on **every** backend — this is
    /// the piece wasm lacked.  The slot's empty store is replaced by a heap
    /// store copied from the file, keeping the slot bookkeeping.  Returns
    /// `false` on a missing / truncated / wrong-format file (via the
    /// `Store::load` signature check under `catch_unwind`), never a misread.
    /// @PLN97 arc G Phase 1 (#522).
    pub fn load_path(&mut self, slot: u16, path: &std::path::Path) -> bool {
        let slot_idx = slot as usize;
        if slot_idx >= self.allocations.len() {
            return false;
        }
        let path_str = match path.to_str() {
            Some(s) if !s.is_empty() => s,
            _ => return false,
        };
        // load needs an existing, plausibly-valid store file (≥ the header).
        if !std::fs::metadata(path).is_ok_and(|m| m.len() >= 16) {
            return false;
        }

        // Preserve the slot's bookkeeping across the swap (mirrors bind_path).
        let preserved = {
            let s = &self.allocations[slot_idx];
            (s.known_type, s.free, s.created_at, s.last_op_at, s.pinned)
        };

        // Store::load panics on a bad signature / unreadable file — surface
        // that as a clean `false` instead of crashing the interpreter.
        let new_store = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::store::Store::load(path_str)
        }));
        let mut new_store = match new_store {
            Ok(s) => s,
            Err(_) => return false,
        };

        new_store.known_type = preserved.0;
        new_store.free = preserved.1;
        new_store.created_at = preserved.2;
        new_store.last_op_at = preserved.3;
        new_store.pinned = preserved.4;

        self.allocations[slot_idx] = new_store;
        true
    }

    /// Adopt an in-memory, already-authenticity-verified byte buffer as the store
    /// in `slot` — the slice counterpart of [`Stores::load_path`] (a heap copy
    /// keeping the slot's bookkeeping).  Returns `false` (a clean reject, never a
    /// misread/crash) when the slot is out of range or the buffer is not a
    /// well-formed store image (bad signature / too small).  The caller MUST have
    /// established the bytes' authenticity first — see [`Stores::load_url_verified`].
    /// @PLN97 arc G Phase 0.
    pub fn load_bytes(&mut self, slot: u16, bytes: &[u8]) -> bool {
        let slot_idx = slot as usize;
        if slot_idx >= self.allocations.len() {
            return false;
        }
        // Preserve the slot's bookkeeping across the swap (mirrors load_path).
        let preserved = {
            let s = &self.allocations[slot_idx];
            (s.known_type, s.free, s.created_at, s.last_op_at, s.pinned)
        };
        let mut new_store = match crate::store::Store::from_bytes(bytes) {
            Some(s) => s,
            None => return false,
        };
        new_store.known_type = preserved.0;
        new_store.free = preserved.1;
        new_store.created_at = preserved.2;
        new_store.last_op_at = preserved.3;
        new_store.pinned = preserved.4;
        self.allocations[slot_idx] = new_store;
        true
    }

    /// @PLN97 arc G Phase 0 — load a persisted store IMAGE over HTTP(S) from a
    /// TRUSTED source into `slot`, establishing authenticity BEFORE the bytes are
    /// adopted: fetch the whole image, verify its SHA-256 against the caller-
    /// pinned `sha256_hex`, and only on a match adopt it in memory (the bytes
    /// never touch disk).  A fetch error OR a hash mismatch REFUSES the load
    /// (returns `false`, adopts nothing) — the same fetch→verify→trust discipline
    /// the registry install path uses (`registry_index::verify_sha256`), bridged
    /// onto the store loader.  `url` may be `http(s)://` or `file://`
    /// (offline / testing).  This is the whole-file counterpart of the paged
    /// `load_key(s)` / `load_range` loaders; per-page authenticity for the paged
    /// path is a separate (Merkle-tree) problem, out of Phase 0's scope.
    #[cfg(feature = "registry")]
    pub fn load_url_verified(&mut self, slot: u16, url: &str, sha256_hex: &str) -> bool {
        let bytes = match crate::registry_index::http_get_bytes(url) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("store loader: refusing {url} — fetch failed: {e}");
                return false;
            }
        };
        if let Err(e) = crate::registry_index::verify_sha256(&bytes, sha256_hex) {
            eprintln!("store loader: refusing {url} — {e}");
            return false;
        }
        self.load_bytes(slot, &bytes)
    }

    /// @PLN97 arc G Phase 0 — load a whole store IMAGE over HTTP(S)/`file://` from
    /// a **TRUSTED** source (no authenticity check) into `slot` — the *instant*
    /// counterpart of [`Stores::load_url_verified`].  Skips the SHA-256 pin (you
    /// trust the origin) but is still **structurally safe**: `load_bytes` →
    /// `Store::from_bytes` runs `validate_structure`, so a corrupt/malformed
    /// image is rejected (`false`), never adopted — the heap invariant holds
    /// regardless.  Use `load_url_verified` when the source is untrusted.
    ///
    /// Available wherever [`crate::net::fetch_bytes`] can fetch: any native build
    /// with the `registry` client, AND the browser (`--html`) target, where the
    /// fetch is bridged to JS `fetch()` via the asyncify host import — so the
    /// loft-visible `store_load_url_trusted(r, url) -> boolean` is identical on
    /// both, per the same-internal-API requirement.
    #[cfg(any(
        feature = "registry",
        all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))
    ))]
    pub fn load_url(&mut self, slot: u16, url: &str) -> bool {
        let bytes = match crate::net::fetch_bytes(url) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("store loader: {url} — fetch failed: {e}");
                return false;
            }
        };
        self.load_bytes(slot, &bytes)
    }

    /// @PLN97 arc G Phase 2 — load a store IMAGE from a local file that may be
    /// **UNTRUSTED** into `slot`, the structurally-validated counterpart of
    /// [`Stores::load_path`].  Reads the whole file and adopts it via
    /// `load_bytes` → `Store::from_bytes`, which runs the always-on
    /// `validate_structure` gate — so a crafted / corrupt file cannot hang
    /// (0-size block) or drive a heap over-read; it is rejected (`false`).
    /// `load_path` (the trusted path) validates only in debug and is faster;
    /// use this for a file whose provenance you don't control.
    pub fn load_path_untrusted(&mut self, slot: u16, path: &std::path::Path) -> bool {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => return false,
        };
        self.load_bytes(slot, &bytes)
    }

    /// @PLN97 3b.5 — the layout-identity gate for a working-set load. Returns
    /// `false` (reject) when the store's `.dschema` sidecar records a layout that
    /// differs from THIS program's collection type — so the loader never
    /// range-reads foreign-layout bytes at schema-derived offsets (a silent
    /// corruption / wild-pointer hazard the post-hoc `store_verify` should not be
    /// the only guard against). An ABSENT sidecar (a legacy / pre-3b.5 store) or
    /// an untyped local store passes; `store_verify` stays the backstop. `path`
    /// is a local file or an `http(s)://` URL — the sidecar is read from beside
    /// it (`<path>.dschema`), over the same transport.
    #[cfg(feature = "remote-store")]
    fn layout_gate_ok(&self, path: &str, local: &DbRef) -> bool {
        let known_type = self.allocations[local.store_nr as usize].known_type;
        if known_type == u16::MAX {
            return true; // untyped store — no identity to compare against
        }
        let current = crate::schema_sidecar::LayoutIdentity::of(self, &[known_type]);
        let verdict = crate::paged_reader::check_sidecar(path, &current);
        if verdict.is_raw_safe() {
            return true;
        }
        // A layout mismatch is a rare, important safety event — always surface it
        // so a rejected load is not silently mistaken for "key absent".
        eprintln!(
            "store loader: refusing {path} — its recorded layout differs from this \
             program's (verdict: {verdict:?}); not range-reading foreign bytes"
        );
        false
    }

    /// @PLN97 arc G Phase 4 / 3b.7 — load the entries with integer key in
    /// `[lo, hi]` from a persisted SORTED collection into the (empty) local
    /// `sorted<T[k]>`, fetching only the pages the range walk touches. The
    /// range-friendly counterpart of `load_key(s)` — routing's tile-window
    /// fetch. Returns the count loaded; refuses a non-sorted / non-copyable
    /// collection. A Sorted collection is a sorted INLINE vector, so the source
    /// range is already ordered: build `local`'s vector directly in key order
    /// (no per-element sort), then relocate each element's heap graph. Root + key
    /// schema come from `local`'s live type (same collection type ⇒ same
    /// structural root position in the image), NOT the raw bytes.
    #[cfg(feature = "remote-store")]
    pub fn load_range(&mut self, local: &DbRef, path: &str, lo: i64, hi: i64) -> i64 {
        if !self.layout_gate_ok(path, local) {
            return 0;
        }
        use crate::paged_reader::{PageSource, PagedReader};
        let Ok(source) = PageSource::open(path) else {
            return 0;
        };
        let mut reader = PagedReader::new(source);
        let tp = self.allocations[local.store_nr as usize].known_type;
        if tp == u16::MAX {
            return 0;
        }
        let content_tp = match self.types[tp as usize].parts {
            Parts::Sorted(c, _) => c,
            _ => return 0, // range needs an ordered collection
        };
        if !self.is_copyable_entry(content_tp) {
            return 0;
        }
        let keys = self.keys(tp).to_vec();
        let esize = u32::from(self.size(content_tp));
        if esize == 0 {
            return 0;
        }
        let lo_c = [crate::keys::Content::Long(lo)];
        let hi_c = [crate::keys::Content::Long(hi)];
        let (src_rec, positions) = crate::paged_reader::sorted_range_positions(
            &mut reader,
            local.rec,
            local.pos,
            esize,
            &lo_c,
            &hi_c,
            &keys,
        );
        let count = positions.len() as u32;
        if count == 0 {
            return 0;
        }
        let store_nr = local.store_nr;
        // Build the local sorted vector: header (8 bytes) + count elements.
        let words = (8 + count * esize).div_ceil(8).max(2);
        let vec_rec = self.allocations[store_nr as usize].claim(words);
        self.allocations[store_nr as usize].zero_fill(vec_rec);
        self.allocations[store_nr as usize].set_u32_raw(vec_rec, 4, count); // length
        self.allocations[store_nr as usize].set_u32_raw(local.rec, local.pos, vec_rec);

        for (i, &epos) in positions.iter().enumerate() {
            let dst_pos = 8 + (i as u32) * esize;
            // Copy the element's field bytes (esize is 4-aligned — all fields are).
            let mut b = 0u32;
            while b + 4 <= esize {
                let w = reader.u32_at(src_rec, epos + b);
                self.allocations[store_nr as usize].set_u32_raw(vec_rec, dst_pos + b, w);
                b += 4;
            }
            // Relocate this element's heap graph into local.
            self.relocate_ptr_fields(
                &mut reader,
                vec_rec,
                dst_pos,
                src_rec,
                epos,
                content_tp,
                store_nr,
            );
        }

        if std::env::var_os("LOFT_LOADER_STATS").is_some() {
            eprintln!(
                "store_load_range: [{lo},{hi}] loaded={count} bytes_fetched={} file={}",
                reader.provider().bytes_fetched(),
                reader.size()
            );
        }
        i64::from(count)
    }

    /// Verify a store-rooted collection `r`'s heap graph is structurally sound —
    /// every interior pointer (text offset, vector/child rec, hash bucket +
    /// entries) stays within its store's bounds. Reuses the DEFENSIVE
    /// [`validate_claims`](Stores::validate_claims) walk (guard-before-deref —
    /// it never faults on a wild pointer, it NAMES the broken edge). Its type
    /// comes from the live schema (`known_type`). Returns `true` when sound;
    /// `false` (with `[cr-check]` reasons on stderr) otherwise.
    ///
    /// This is the instrument that makes the relocating working-set copy
    /// (Phase 3b) *checkable*: after a `store_load*`, `store_verify(local)`
    /// proves the copy left no pointer aimed outside the store.
    pub fn verify_graph_ok(&self, r: &DbRef) -> bool {
        let tp = self.allocations[r.store_nr as usize].known_type;
        if tp == u16::MAX {
            return false;
        }
        let mut problems = 0u32;
        self.validate_claims(r, tp, "store_verify", &mut problems);
        problems == 0
    }

    /// True when a field's type stores its value INLINE (a fixed-width scalar),
    /// so a working-set copy can move it as raw bytes with no pointer to
    /// relocate. Text (type 5) and Reference (type 6) are POINTERS; vectors /
    /// nested structs / keyed collections are heap-owned — all need the
    /// relocating graph-copy (3b.2+), so they are NOT inline.
    #[cfg(feature = "remote-store")]
    fn is_inline_scalar(&self, tp: u16) -> bool {
        match self.types[tp as usize].parts {
            Parts::Int(..) | Parts::Byte(..) | Parts::Short(..) | Parts::ShortRaw(..) => true,
            // Base covers the numeric primitives AND text(5) / Reference(6);
            // only the numerics are inline.
            Parts::Base => !matches!(tp, 5 | 6),
            _ => false,
        }
    }

    /// True when a field can be moved by the working-set copy today: an inline
    /// scalar (raw word-copy), a `text` (relocated string, 3b.2), or a
    /// `vector<scalar>` (relocated flat inner record, 3b.3). Text and a
    /// scalar-vector are BOTH a single flat sub-record behind a `u32` pointer, so
    /// they relocate with identical code. A `vector<struct>` / `vector<text>` /
    /// nested struct / reference still needs the recursive copy (3b.4+).
    #[cfg(feature = "remote-store")]
    fn is_copyable_field(&self, tp: u16) -> bool {
        if self.is_inline_scalar(tp) || tp == 5 {
            return true;
        }
        match &self.types[tp as usize].parts {
            // vector<scalar> (flat inner record) OR vector<copyable struct>
            // (copy inner + relocate each element, 3b.4b). vector<text> /
            // vector<vector> are NOT handled — the element pointers would dangle.
            Parts::Vector(e) => {
                self.is_inline_scalar(*e)
                    || (matches!(
                        self.types[*e as usize].parts,
                        Parts::Struct(_) | Parts::EnumValue(_, _)
                    ) && self.is_copyable_field(*e))
            }
            // An INLINE nested struct: copyable when every one of its own fields
            // is (its `text`/`vector` fields relocate at a nested offset, 3b.4).
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.iter().all(|f| self.is_copyable_field(f.content))
            }
            _ => false,
        }
    }

    /// True when an entry of type `content_tp` can be partially loaded today —
    /// every field is [copyable](Stores::is_copyable_field). Otherwise the
    /// collection is refused (safe-refusal, never a broken heap).
    #[cfg(feature = "remote-store")]
    fn is_copyable_entry(&self, content_tp: u16) -> bool {
        match &self.types[content_tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.iter().all(|f| self.is_copyable_field(f.content))
            }
            _ => self.is_copyable_field(content_tp),
        }
    }

    #[cfg(feature = "remote-store")]
    pub fn load_key(&mut self, local: &DbRef, path: &str, key: i64) -> bool {
        self.load_keys(local, path, std::slice::from_ref(&key)) > 0
    }

    /// @PLN97 arc G Phase 3b.6 — load ONE TEXT-keyed entry from a persisted
    /// `hash<T[textkey]>` (e.g. a place-name or z/x/y tile-id index), fetching
    /// only the pages the lookup touches. Same working-set fetch as
    /// [`load_key`](Stores::load_key), keyed by a string: `find_hash_entry`
    /// hashes the `Content::Str`, and the entry's text key is compared over the
    /// reader. Returns false when absent / unreadable / not an integer-or-text
    /// -keyed copyable hash.
    #[cfg(feature = "remote-store")]
    pub fn load_key_text(&mut self, local: &DbRef, path: &str, key: &str) -> bool {
        if !self.layout_gate_ok(path, local) {
            return false;
        }
        use crate::paged_reader::{PageSource, PagedReader};
        let Ok(source) = PageSource::open(path) else {
            return false;
        };
        let mut reader = PagedReader::new(source);
        let tp = self.allocations[local.store_nr as usize].known_type;
        if tp == u16::MAX {
            return false;
        }
        let content_tp = match self.types[tp as usize].parts {
            Parts::Hash(c, _) => c,
            _ => return false,
        };
        if !self.is_copyable_entry(content_tp) {
            return false;
        }
        let keys = self.keys(tp).to_vec();
        let key_content = [crate::keys::Content::Str(crate::keys::Str::new(key))];
        self.load_one(&mut reader, local, content_tp, &keys, &key_content)
    }

    /// @PLN97 arc G Phase 3a — load the requested integer keys' entries from a
    /// persisted HASH image at `path` into the empty local hash `local`,
    /// fetching only the pages the lookups touch. Returns the count actually
    /// found (keys absent in the remote are silently skipped). The paged reader
    /// is opened ONCE and shared across all keys, so its LRU cache is reused.
    /// See [`load_key`](Stores::load_key) for the single-key form. Same
    /// FLAT-struct restriction (scalar fields only — no relocation yet).
    #[cfg(feature = "remote-store")]
    pub fn load_keys(&mut self, local: &DbRef, path: &str, keys_vals: &[i64]) -> i64 {
        if !self.layout_gate_ok(path, local) {
            return 0;
        }
        use crate::paged_reader::{PageSource, PagedReader};
        // `path` is a local file OR an `http(s)://` URL — the paged reader pulls
        // only the pages a lookup touches, from disk or over the network (#517).
        let Ok(source) = PageSource::open(path) else {
            return 0;
        };
        let mut reader = PagedReader::new(source);

        // Schema from the LIVE type of `local` (never reverse-engineered bytes).
        let tp = self.allocations[local.store_nr as usize].known_type;
        if tp == u16::MAX {
            return 0;
        }
        // SAFE REFUSAL (3b.1) — `load_one` can copy an entry whose fields are
        // inline-scalar (raw word-copy) or `text` (relocated string, 3b.2). A
        // vector / nested / reference field still needs the recursive relocating
        // copy (3b.3+); until then, refuse the whole collection (load nothing)
        // rather than build a heap with a dangling pointer. `store_verify` would
        // catch a broken copy — this makes sure one is never built.
        let content_tp = match self.types[tp as usize].parts {
            Parts::Hash(c, _) => c,
            _ => return 0, // only Hash supported so far (Sorted lands at 3b.7)
        };
        if !self.is_copyable_entry(content_tp) {
            return 0;
        }
        let keys = self.keys(tp).to_vec();

        let mut loaded = 0i64;
        for &kv in keys_vals {
            if self.load_one(
                &mut reader,
                local,
                content_tp,
                &keys,
                &[crate::keys::Content::Long(kv)],
            ) {
                loaded += 1;
            }
        }

        // Observability for the "bytes fetched ≪ file" invariant: at scale N
        // keys touch O(N) pages, not O(file). Off unless asked.
        if std::env::var_os("LOFT_LOADER_STATS").is_some() {
            eprintln!(
                "store_load_keys: asked={} loaded={loaded} bytes_fetched={} file={}",
                keys_vals.len(),
                reader.provider().bytes_fetched(),
                reader.size()
            );
        }
        loaded
    }

    /// Find one integer key in the paged image and, if present, FLAT-copy its
    /// entry record into `local` and link it via the verified `hash::add`. The
    /// entry's field words (fld 8 .. size·8) hold only scalars, so a straight
    /// word copy into a fresh claim is correct — no internal `rec` pointers to
    /// relocate. Returns whether the key was found.
    #[cfg(feature = "remote-store")]
    fn load_one(
        &mut self,
        reader: &mut crate::paged_reader::PagedReader<crate::paged_reader::PageSource>,
        local: &DbRef,
        content_tp: u16,
        keys: &[crate::keys::Key],
        key_content: &[crate::keys::Content],
    ) -> bool {
        let matched =
            crate::paged_reader::find_hash_entry(reader, local.rec, local.pos, key_content, keys);
        if matched == 0 {
            return false;
        }
        let size = reader.record_words(matched);
        if size < 2 {
            return false;
        }
        // 1) Move the entry record's field words verbatim. Inline scalars are
        //    now correct; every `text` field still holds the SOURCE store's
        //    string-record id (a dangling pointer) — fixed in step 2.
        let store = &mut self.allocations[local.store_nr as usize];
        let new_rec = store.claim(size);
        store.zero_fill(new_rec);
        let mut fld = 8u32;
        while fld + 4 <= size * 8 {
            let w = reader.u32_at(matched, fld);
            self.allocations[local.store_nr as usize].set_u32_raw(new_rec, fld, w);
            fld += 4;
        }

        // 2) 3b.2–3b.4b — relocate the entry's pointer fields (text / vector /
        //    nested / vector<struct>) so the local copy owns its whole graph.
        //    Hash entry: dst + src field data both start at fld 8.
        self.relocate_ptr_fields(reader, new_rec, 8, matched, 8, content_tp, local.store_nr);

        let entry = DbRef {
            store_nr: local.store_nr,
            rec: new_rec,
            pos: 8,
        };
        crate::hash::add(local, &entry, &mut self.allocations, keys);
        true
    }

    /// Recursively relocate the POINTER fields of struct `tp` whose (already
    /// flat-copied) data starts at byte `dst_fb` in local record `dst_rec`, with
    /// the source starting at `src_fb` in `src_rec` read over `reader`. The two
    /// field-bases are separate because a hash entry copies same-offset (both 8)
    /// while a Sorted element copies from `8 + i·esize` in the source vector into
    /// a slot at a different local position. Handles: text / vector<scalar> (flat
    /// sub-record copy), inline nested struct (recurse, deeper base), vector<struct>
    /// (copy inner + recurse each element). Inline scalars are already correct.
    #[cfg(feature = "remote-store")]
    #[allow(clippy::too_many_arguments)] // dst/src (rec,base) + reader + tp + store — all essential
    fn relocate_ptr_fields<P: crate::paged_reader::PageProvider>(
        &mut self,
        reader: &mut crate::paged_reader::PagedReader<P>,
        dst_rec: u32,
        dst_fb: u32,
        src_rec: u32,
        src_fb: u32,
        tp: u16,
        store_nr: u16,
    ) {
        let fields = match &self.types[tp as usize].parts {
            Parts::Struct(f) | Parts::EnumValue(_, f) => f.clone(),
            _ => return,
        };
        for f in fields {
            let pos = u32::from(f.position);
            let (dst_off, src_off) = (dst_fb + pos, src_fb + pos);
            let ftp = f.content;
            if ftp == 5
                || matches!(self.types[ftp as usize].parts, Parts::Vector(e) if self.is_inline_scalar(e))
            {
                // Flat sub-record pointer (text / vector<scalar>).
                let src_sub = reader.u32_at(src_rec, src_off);
                if src_sub != 0 {
                    let new_sub = self.copy_flat_subrecord(reader, src_sub, store_nr);
                    self.allocations[store_nr as usize].set_u32_raw(dst_rec, dst_off, new_sub);
                }
            } else if matches!(
                self.types[ftp as usize].parts,
                Parts::Struct(_) | Parts::EnumValue(_, _)
            ) {
                // Inline nested struct: same records, deeper bases.
                self.relocate_ptr_fields(reader, dst_rec, dst_off, src_rec, src_off, ftp, store_nr);
            } else if let Parts::Vector(elem_tp) = self.types[ftp as usize].parts {
                // vector<struct>: copy the inner record + recurse each element.
                let src_sub = reader.u32_at(src_rec, src_off);
                if src_sub != 0 {
                    let new_sub = self.copy_vector_of_struct(reader, src_sub, elem_tp, store_nr);
                    self.allocations[store_nr as usize].set_u32_raw(dst_rec, dst_off, new_sub);
                }
            }
        }
    }

    /// Copy a FLAT record (a string or a `vector<scalar>` inner record — no
    /// interior pointers) at `src_rec` into a fresh local claim; the size header
    /// (fld 0) is set by `claim`, so copy the length + payload from fld 4 on.
    #[cfg(feature = "remote-store")]
    fn copy_flat_subrecord<P: crate::paged_reader::PageProvider>(
        &mut self,
        reader: &mut crate::paged_reader::PagedReader<P>,
        src_rec: u32,
        store_nr: u16,
    ) -> u32 {
        let ssz = reader.record_words(src_rec);
        let new = self.allocations[store_nr as usize].claim(ssz.max(2));
        self.allocations[store_nr as usize].zero_fill(new);
        let mut sf = 4u32;
        while sf + 4 <= ssz * 8 {
            let w = reader.u32_at(src_rec, sf);
            self.allocations[store_nr as usize].set_u32_raw(new, sf, w);
            sf += 4;
        }
        new
    }

    /// Copy a `vector<struct>` inner record: first the flat bytes (length + the
    /// contiguous element structs), then relocate EACH element's pointer fields
    /// (its text / vector / nested graph). Elements are at `8 + i·elem_size`.
    #[cfg(feature = "remote-store")]
    fn copy_vector_of_struct<P: crate::paged_reader::PageProvider>(
        &mut self,
        reader: &mut crate::paged_reader::PagedReader<P>,
        src_inner: u32,
        elem_tp: u16,
        store_nr: u16,
    ) -> u32 {
        let new_inner = self.copy_flat_subrecord(reader, src_inner, store_nr);
        let length = reader.u32_at(src_inner, 4);
        let esize = u32::from(self.size(elem_tp));
        if esize == 0 {
            return new_inner;
        }
        for i in 0..length {
            // element i's data starts at byte 8 + i·esize in BOTH the copied
            // inner record and the source (this is a copy, so same offsets).
            let fb = 8 + i * esize;
            self.relocate_ptr_fields(reader, new_inner, fb, src_inner, fb, elem_tp, store_nr);
        }
        new_inner
    }

    /// Read a `vector<integer>` (`keys_vec`) into an `i64` slice and load those
    /// keys via [`load_keys`](Stores::load_keys). The single vector-reading home
    /// used by BOTH backends (the interpreter handler and the `#rust` codegen
    /// body), so the element layout lives in one place. `integer` elements are
    /// i64 at `8 + i·8` within the vector's inner record.
    #[cfg(feature = "remote-store")]
    pub fn load_keys_vec(&mut self, local: &DbRef, path: &str, keys_vec: &DbRef) -> i64 {
        let length = crate::vector::length_vector(keys_vec, &self.allocations);
        let inner = self.store(keys_vec).get_u32_raw(keys_vec.rec, keys_vec.pos);
        let mut vals = Vec::with_capacity(length as usize);
        for i in 0..length {
            vals.push(self.store(keys_vec).get_int(inner, 8 + i * 8));
        }
        self.load_keys(local, path, &vals)
    }
}

/// @PLAN38 — pad a Store byte image out to `target_words` while keeping
/// the record chain valid.  Walks the source record headers (i32 at each
/// word position, abs = block size in words), then either extends the
/// trailing free block or appends a new one to cover the padding.  Returns
/// `None` if the source bytes don't describe a walkable record chain
/// (corrupt size word, zero-size block).
///
/// Output buffer is always exactly `target_words * 8` bytes.  When
/// `target_words <= src_words` the buffer is the source verbatim (no
/// truncation; we treat that as the "no padding needed" path).
#[cfg(feature = "mmap")]
fn build_padded_store_image(src: &[u8], src_words: u32, target_words: u32) -> Option<Vec<u8>> {
    if (src.len() as u32) < src_words.saturating_mul(8) {
        return None;
    }
    let out_bytes = (target_words as usize) * 8;
    let mut out = vec![0u8; out_bytes.max(src.len())];
    out[..src.len()].copy_from_slice(src);

    if target_words <= src_words {
        out.truncate((src_words as usize) * 8);
        return Some(out);
    }

    // Walk the source record chain to find the last block.  PRIMARY = 1
    // is the first record's word position; size word is the i32 at that
    // word's byte offset 0.
    let mut rec: u32 = 1;
    let mut last_rec: u32 = 1;
    let mut last_size: i32 = 0;
    while rec < src_words {
        let off = (rec as usize) * 8;
        if off + 4 > src.len() {
            return None;
        }
        let sz = i32::from_le_bytes([src[off], src[off + 1], src[off + 2], src[off + 3]]);
        if sz == 0 {
            return None;
        }
        last_rec = rec;
        last_size = sz;
        let step = sz.unsigned_abs();
        rec = rec.checked_add(step)?;
    }
    if rec != src_words {
        // Chain didn't terminate exactly at the end of the source — the
        // image is malformed or doesn't follow the loft Store layout.
        return None;
    }

    let pad_words = target_words - src_words;
    if last_size < 0 {
        // Extend the trailing free block in-place.
        let new_size = last_size.checked_sub(pad_words as i32)?;
        let off = (last_rec as usize) * 8;
        out[off..off + 4].copy_from_slice(&new_size.to_le_bytes());
    } else {
        // Active record at the tail — append a new free block right after it.
        let new_rec = (last_rec).checked_add(last_size as u32)?;
        if new_rec != src_words {
            return None;
        }
        let new_size = -(pad_words as i32);
        let off = (new_rec as usize) * 8;
        out[off..off + 4].copy_from_slice(&new_size.to_le_bytes());
    }
    Some(out)
}

#[cfg(test)]
mod p318_hash_deepcopy {
    use crate::database::Parts;
    use crate::database::Stores;
    use crate::keys::DbRef;

    /// @P318 — deep-copying a struct whose `hash<T[k]>` field is populated must
    /// keep the hash FIND-CONSISTENT even when the destination bucket record is
    /// OVER-sized.  `Store::claim` returns a block up to 1/3 larger than
    /// requested without splitting (`claim_block`), and a hash reads its `room`
    /// (bucket count, `elms = (room-2)*2`) from that size header — so the old
    /// slot-for-slot copy laid the dest buckets out for the SOURCE room while
    /// `find` later probed `key % dest_elms` (a DIFFERENT start slot) and missed
    /// entries.  The gap `(room, room*4/3]` is tiny for small rooms (e.g. (9,12],
    /// (17,22]), so a freed record easily lands in it.  This test forces the
    /// over-size deterministically — claim `big, gap=room+1, big` contiguously
    /// then delete the middle, so `fl_take_ge(room)` returns the gap block — and
    /// asserts every key survives the deep copy.
    /// Positive control for the hash arm of `validate_claims` (the walk behind
    /// `store_verify`) — a green check is only evidence once the detector is
    /// known to FIRE. Build a sound hash (must validate clean), then corrupt the
    /// root pointer to an out-of-range bucket record and confirm the walk NAMES
    /// the broken edge instead of faulting on it — exactly what a bad relocation
    /// would leave behind (a source rec-number larger than the small local store).
    #[test]
    // @PLN54 S5(b) — the DA gate: `validate_claims`' GRACEFUL out-of-range
    // detection (name the broken edge, don't fault) is a RELEASE-mode contract.
    // Under debug-assertions the lower-level `Store::addr` bounds `debug_assert!`
    // (store.rs) fail-fasts on the dangling read FIRST — an equally valid, louder
    // signal, but not the graceful path this positive control asserts.  So skip
    // it under DA (where the fail-fast is the correct behavior), run it otherwise.
    #[cfg_attr(
        debug_assertions,
        ignore = "graceful dangling-walk is release-mode; DA addr() fail-fasts first"
    )]
    fn verify_graph_catches_a_dangling_pointer() {
        let mut stores = Stores::new();
        let cell = stores.structure("VCell", -1);
        stores.field(cell, "k", 0); // integer
        stores.field(cell, "v", 0); // integer
        stores.finish();
        let hash_tp = stores.hash(cell, &["k".to_string()]);
        let holder = stores.structure("VHolder", -1);
        stores.field(holder, "h", hash_tp);
        stores.finish();

        let words = |sz: u16| 1 + ((u32::from(sz) + 7) >> 3);
        let cell_words = words(stores.size(cell));
        let holder_words = words(stores.size(holder));

        let root = stores.database(holder_words);
        let h = DbRef {
            store_nr: root.store_nr,
            rec: root.rec,
            pos: root.pos, // field `h` at struct-position 0
        };
        for k in 0..5i64 {
            let e = stores.database(cell_words);
            stores.store_mut(&e).set_int(e.rec, e.pos, k);
            stores.store_mut(&e).set_int(e.rec, e.pos + 8, k + 100);
            stores.set_keyed(&h, &e, hash_tp, false);
        }

        // Sound to start — no false positive.
        let mut problems = 0u32;
        stores.validate_claims(&h, hash_tp, "ctl", &mut problems);
        assert_eq!(problems, 0, "a well-formed hash must validate clean");

        // Corrupt: aim the hash root at a bucket record beyond the store.
        stores.store_mut(&h).set_u32_raw(h.rec, h.pos, 99_999);
        let mut broken = 0u32;
        stores.validate_claims(&h, hash_tp, "ctl", &mut broken);
        assert!(
            broken > 0,
            "validate_claims MUST catch an out-of-range bucket pointer (positive control), \
             not fault on it"
        );
    }

    #[test]
    fn hash_deepcopy_survives_oversized_dest_bucket() {
        let mut stores = Stores::new();
        let cell_tp = stores.structure("CellP318", -1);
        stores.field(cell_tp, "ck", 0); // integer
        stores.field(cell_tp, "payload", 0); // integer
        stores.finish();
        let hash_tp = stores.hash(cell_tp, &["ck".to_string()]);
        let holder_tp = stores.structure("HolderP318", -1);
        stores.field(holder_tp, "h", hash_tp);
        stores.finish();

        let words = |sz: u16| 1 + ((u32::from(sz) + 7) >> 3);
        let cell_words = words(stores.size(cell_tp));
        let holder_words = words(stores.size(holder_tp));

        // --- source Holder with a populated hash (n=14 -> room 17) ---
        let src = stores.database(holder_words);
        let src_h = DbRef {
            store_nr: src.store_nr,
            rec: src.rec,
            pos: src.pos, // field `h` is at struct-position 0
        };
        let n: i64 = 14;
        for k in 0..n {
            let v = stores.database(cell_words);
            stores.store_mut(&v).set_int(v.rec, v.pos, k); // ck = k
            stores.store_mut(&v).set_int(v.rec, v.pos + 8, k + 1000); // payload
            stores.set_keyed(&src_h, &v, hash_tp, false);
        }
        let cur = stores.store(&src_h).get_u32_raw(src_h.rec, src_h.pos);
        let room = stores.store(&src_h).record_words(cur);
        assert!(room >= 3, "source room {room} too small to over-size");

        // --- destination Holder + an engineered over-size free block ---
        let dst = stores.database(holder_words);
        let dst_h = DbRef {
            store_nr: dst.store_nr,
            rec: dst.rec,
            pos: dst.pos,
        };
        {
            let s = &mut stores.allocations[dst.store_nr as usize];
            let _p1 = s.claim(room * 8); // big
            let gap = s.claim(room + 1); // gap-sized: room < room+1 <= room*4/3 (room>=3)
            let _p3 = s.claim(room * 8); // big — pins the gap so delete can't merge it
            s.delete(gap); // free block of size room+1; the smallest free >= room
        }

        // --- deep-copy the populated hash field into the dest ---
        stores.copy_claims(&src_h, &dst_h, hash_tp);

        // --- every key must still be found, with the right payload ---
        let keys = stores.types[hash_tp as usize].keys.clone();
        let mut missing = 0;
        for k in 0..n {
            let probe = stores.database(cell_words);
            stores.store_mut(&probe).set_int(probe.rec, probe.pos, k);
            let key = crate::keys::get_key(&probe, &stores.allocations, &keys);
            let found = crate::hash::find(&dst_h, &stores.allocations, &keys, &key);
            if found.rec == 0 {
                missing += 1;
            } else {
                let pay = stores.store(&found).get_int(found.rec, found.pos + 8);
                assert_eq!(pay, k + 1000, "wrong payload for key {k}");
            }
        }
        assert_eq!(
            missing, 0,
            "deep-copied hash lost {missing}/{n} entries (source room {room})"
        );
    }

    /// @PLN102 heap-free audit — the null/absent container is a no-op at the free/teardown
    /// chokepoints, safe by CONSTRUCTION not by caller convention. Without the `remove_claims`
    /// guard this indexes `allocations[u16::MAX]` (`for_each_owned_child` reads `store(rec)`) and
    /// OOB-panics; the assertion is that it does not. `free_named`/`free` already guarded — pinned
    /// here too so a future null-container caller can never reintroduce the panic.
    #[test]
    fn free_and_teardown_no_op_on_the_null_sentinel() {
        let mut stores = Stores::new();
        let cell = stores.structure("NCell", -1);
        stores.field(cell, "x", 0); // integer
        stores.finish();
        // None of these may panic; each is a no-op on the `store_nr == u16::MAX` sentinel.
        stores.remove_claims(&DbRef::NULL, cell);
        stores.free_named(&DbRef::NULL, "");
        stores.free(&DbRef::NULL);
    }

    /// @PLN102 heap copy/alias audit — positive control for the `elm == 0` (absent element)
    /// guards in `copy_claims_array_body` (copy) and `remove_claims` (free). A `vector<S>`
    /// field alongside an `index<S[k]>` over the SAME `S` becomes a record-per-element `Array`;
    /// every runtime path keeps its length in sync with the filled slots, so a 0 rec-id slot
    /// is unreachable in practice. Here we hand-build that state (slot0 = real element,
    /// slot1 = 0) and assert the walks treat the 0 slot as an absent hole — the copy preserves
    /// it as 0 (never fabricating an element from reserved record 0) and the free skips it
    /// (never `delete(0)` nor recursing into record 0). Proven to FAIL without the copy guard:
    /// the absent slot then copies as a bogus non-zero record id.
    #[test]
    fn array_copy_and_free_skip_an_absent_element_slot() {
        let mut stores = Stores::new();
        let cell = stores.structure("ACell", -1);
        stores.field(cell, "k", 0); // integer key
        stores.field(cell, "v", 0); // integer
        stores.finish();
        let vec_tp = stores.vector(cell);
        // An `index<ACell[k]>` field marks `ACell` LINKED, so the sibling `vector<ACell>`
        // is laid out record-per-element (`Array`) rather than inline (`Vector`).
        let idx_tp = stores.index(cell, &[("k".to_string(), false)]);
        let holder = stores.structure("AHolder", -1);
        stores.field(holder, "list", vec_tp);
        stores.field(holder, "ix", idx_tp);
        stores.finish();

        assert!(
            matches!(stores.types[vec_tp as usize].parts, Parts::Array(_)),
            "setup: a linked vector<ACell> must lay out as an Array (got {:?})",
            stores.types[vec_tp as usize].parts
        );

        let words = |sz: u16| 1 + ((u32::from(sz) + 7) >> 3);
        let cell_words = words(stores.size(cell));
        let holder_words = words(stores.size(holder));
        let Parts::Struct(fields) = &stores.types[holder as usize].parts else {
            unreachable!("holder is a struct")
        };
        let list_pos = u32::from(fields[0].position);

        // Source holder whose Array container has slot0 = a real element, slot1 = 0 (absent).
        let src = stores.database(holder_words);
        let list_src = DbRef {
            store_nr: src.store_nr,
            rec: src.rec,
            pos: src.pos + list_pos,
        };
        let elem = stores.store_mut(&src).claim(cell_words);
        stores.store_mut(&src).set_int(elem, 4, 7); // element payload
        let cur = stores.store_mut(&src).claim(2); // container: header word + one slot word (len 2)
        stores.store_mut(&src).set_u32_raw(cur, 4, 2); // length header = 2
        stores.store_mut(&src).set_u32_raw(cur, 8, elem); // slot0 → real element
        stores.store_mut(&src).set_u32_raw(cur, 12, 0); // slot1 → ABSENT (the guarded state)
        stores
            .store_mut(&list_src)
            .set_u32_raw(list_src.rec, list_src.pos, cur);

        // COPY — the absent slot must copy as a hole (0), never a record fabricated from rec 0.
        let dest = stores.database(holder_words);
        let list_dest = DbRef {
            store_nr: dest.store_nr,
            rec: dest.rec,
            pos: dest.pos + list_pos,
        };
        stores.copy_claims(&list_src, &list_dest, vec_tp);
        let dcur = stores
            .store(&list_dest)
            .get_u32_raw(list_dest.rec, list_dest.pos);
        assert_ne!(dcur, 0, "copied Array container must exist");
        assert_eq!(
            stores.store(&list_dest).get_u32_raw(dcur, 4),
            2,
            "copied length header preserved"
        );
        assert_ne!(
            stores.store(&list_dest).get_u32_raw(dcur, 8),
            0,
            "slot0 (real element) is copied"
        );
        assert_eq!(
            stores.store(&list_dest).get_u32_raw(dcur, 12),
            0,
            "slot1 (absent element) stays a hole — copy MUST NOT dereference record 0"
        );

        // FREE — teardown must not recurse into / delete record 0 for the absent slots
        // (both the hand-built source and the copied destination carry one).
        stores.remove_claims(&src, holder);
        stores.free(&dest);
    }
}
