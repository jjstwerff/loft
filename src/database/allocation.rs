// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Memory/store allocation helpers and claim management.

use crate::database::{Parts, Stores, WorkerStores};
use crate::keys::DbRef;
use crate::store::Store;
use crate::tree;
use crate::vector;

impl Stores {
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
        // S29 (P1-R4 M4-b): find the lowest free slot using the free_bits bitmap.
        // If a freed slot exists below max, reuse it; otherwise grow max.
        let slot = self.find_free_slot();
        if slot >= self.allocations.len() as u16 {
            self.allocations.push(Store::new(100));
        } else {
            // A free slot may still carry a stale lock if set_store_lock was
            // called with a dangling DbRef after the store was freed.  Clear
            // the lock before reinitialising to prevent a spurious panic in
            // Store::init().
            self.allocations[slot as usize].unlock();
            self.allocations[slot as usize].init();
        }
        if slot == self.max {
            self.max += 1;
        }
        // Clear the bitmap bit for this slot (it is now active).
        self.clear_free_bit(slot);
        let store = &mut self.allocations[slot as usize];
        assert!(store.free, "Allocating a used store");
        store.free = false;
        store.ref_count = 1;
        store.created_at = 0;
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
        if std::env::var("LOFT_STORE_LOG").is_ok() {
            if name.is_empty() {
                eprintln!(
                    "[store] alloc store={} rec={} size={size}",
                    result.store_nr, result.rec
                );
            } else {
                eprintln!(
                    "[store] alloc store={} \"{name}\" rec={} size={size}",
                    result.store_nr, result.rec
                );
            }
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
        if std::env::var("LOFT_STORE_LOG").is_ok() {
            if name.is_empty() {
                eprintln!("[store] free  store={al} (max={})", self.max);
            } else {
                eprintln!("[store] free  store={al} \"{name}\" (max={})", self.max);
            }
        }
        debug_assert!(al < self.allocations.len() as u16, "Incorrect store");
        // Issue #120: decrement reference count. Only free when it reaches 0.
        // Multiple variables may alias the same store through const borrowing.
        let store = &mut self.allocations[al as usize];
        if store.ref_count > 1 {
            store.ref_count -= 1;
            if std::env::var("LOFT_STORE_LOG").is_ok() {
                eprintln!(
                    "[store] decref store={al} ref_count={} (created_at={}, last_op={})",
                    store.ref_count, store.created_at, store.last_op_at
                );
            }
            return;
        }
        store.ref_count = 0;
        debug_assert!(!store.free, "Double free store");
        // S36: clear the lock before marking free.
        store.unlock();
        store.free = true;
        // S29 (P1-R4 M4-b): mark slot as free in the bitmap so database_named()
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
        // Issue #120: when multiple variables alias the same store through
        // const parameter borrowing, the first FreeRef frees the store.
        // Subsequent accesses from aliased variables should not panic.
        // The proper fix is store reference counting; for now, tolerate this.
        if self.allocations[db.store_nr as usize].free {
            return;
        }
    }

    pub fn clear(&mut self, db: &DbRef) {
        let store = &mut self.allocations[db.store_nr as usize];
        // Clear any stale lock before reinitialising — OpDatabase may
        // reinitialise a store that was previously locked by a const
        // parameter in a prior function call within the same loop iteration.
        store.unlock();
        store.init();
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
        &self.allocations[r.store_nr as usize]
    }

    pub fn store_mut(&mut self, r: &DbRef) -> &mut Store {
        &mut self.allocations[r.store_nr as usize]
    }

    /// Lock the store that contains the record pointed to by `r`.
    /// The lock persists until explicitly cleared via `unlock_store`.
    pub fn lock_store(&mut self, r: &DbRef) {
        if r.rec != 0 && (r.store_nr as usize) < self.allocations.len() {
            debug_assert!(
                !self.allocations[r.store_nr as usize].free,
                "Locking a freed store (store_nr={}, rec={})",
                r.store_nr, r.rec
            );
            self.allocations[r.store_nr as usize].lock();
        }
    }

    /// Unlock the store that contains the record pointed to by `r`.
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
            files: Vec::new(),
            max: self.max,
            free_bits,
            scratch: Vec::new(),
            last_parse_errors: Vec::new(),
            parallel_ctx: None,
            logger: self.logger.clone(),
            had_fatal: false,
            source_dir: String::new(),
            frame_yield: false,
            report_asserts: false,
            assert_results: Vec::new(),
            #[cfg(not(feature = "wasm"))]
            start_time: self.start_time,
            #[cfg(feature = "wasm")]
            start_time_ms: self.start_time_ms,
            call_stack_snapshot: Vec::new(),
            closure_map: std::collections::HashMap::new(),
        })
    }

    /// A14.3: produce a light-worker view — main stores borrowed read-only,
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
            files: Vec::new(),
            max: self.allocations.len() as u16 + pool_slice.len() as u16,
            free_bits,
            scratch: Vec::new(),
            last_parse_errors: Vec::new(),
            parallel_ctx: None,
            logger: self.logger.clone(),
            had_fatal: false,
            source_dir: String::new(),
            frame_yield: false,
            report_asserts: false,
            assert_results: Vec::new(),
            #[cfg(not(feature = "wasm"))]
            start_time: self.start_time,
            #[cfg(feature = "wasm")]
            start_time_ms: self.start_time_ms,
            call_stack_snapshot: Vec::new(),
            closure_map: std::collections::HashMap::new(),
        })
    }

    #[must_use]
    pub fn store_nr(&self, nr: u16) -> &Store {
        &self.allocations[nr as usize]
    }

    pub(super) fn copy_claims_seq_vector(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let length = vector::length_vector(rec, &self.allocations);
        let size = u32::from(self.size(tp));
        let cur = self.store(rec).get_int(rec.rec, rec.pos) as u32;
        if cur == 0 {
            self.store_mut(to).set_int(to.rec, to.pos, 0);
            return;
        }
        let into = self.store_mut(to).claim(1 + (size * length).div_ceil(8));
        debug_assert!(
            i32::try_from(into).is_ok(),
            "vector allocation offset overflow: {into}"
        );
        self.store_mut(to).set_int(to.rec, to.pos, into as i32);
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
        for i in 0..length {
            self.copy_claims(
                &DbRef {
                    store_nr: rec.store_nr,
                    rec: cur,
                    pos: 8 + size * i,
                },
                &DbRef {
                    store_nr: to.store_nr,
                    rec: into,
                    pos: 8 + size * i,
                },
                tp,
            );
        }
    }

    pub(super) fn copy_claims_array_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let length = vector::length_vector(rec, &self.allocations);
        let size = u32::from(self.size(tp));
        let cur = self.store(rec).get_int(rec.rec, rec.pos) as u32;
        if cur == 0 {
            self.store_mut(to).set_int(to.rec, to.pos, 0);
            return;
        }
        let into = self.store_mut(to).claim(1 + cur.div_ceil(2));
        self.store_mut(to).set_int(to.rec, to.pos, into as i32);
        for i in 0..length {
            let elm = self.store(rec).get_int(cur, 8 + 4 * i) as u32;
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
            self.store_mut(to).set_int(into, 8 + 4 * i, new as i32);
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
                tp,
            );
        }
    }

    pub(super) fn copy_claims_hash_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let size = u32::from(self.size(tp));
        let cur = self.store(rec).get_int(rec.rec, rec.pos) as u32;
        if cur == 0 {
            self.store_mut(to).set_int(to.rec, to.pos, 0);
            return;
        }
        let length = self.store(rec).get_int(cur, 0) as u32;
        let into = self.store_mut(to).claim(length);
        self.store_mut(to).set_int(to.rec, to.pos, into as i32);
        for i in 1..length * 2 {
            let elm = self.store(rec).get_int(cur, 8 + 4 * i) as u32;
            if elm == 0 {
                self.store_mut(to).set_int(into, 8 + 4 * i, 0);
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
            self.store_mut(to).set_int(into, 8 + 4 * i, new as i32);
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
                tp,
            );
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
        let cur = self.store(rec).get_int(rec.rec, rec.pos) as u32;
        if cur == 0 {
            self.store_mut(to).set_int(to.rec, to.pos, 0);
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
        let nodes = self.collect_index_nodes(rec, left);
        // Initialize the destination tree root to empty before inserting.
        self.store_mut(to).set_int(to.rec, to.pos, 0);
        for src_node in nodes {
            // Allocate element record in the destination store.
            let dst_node = self.store_mut(to).claim(1 + size.div_ceil(8));
            // Back-reference to the destination parent record (offset 4).
            self.store_mut(to).set_int(dst_node, 4, to.rec as i32);
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
    When a field points to a spacial structure.
    */
    pub fn copy_claims(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        // TODO prevent copying secondary structures
        match &self.types[tp as usize].parts {
            Parts::Base if tp == 5 => {
                // text
                let store = self.store(rec);
                let s = store.get_str(store.get_int(rec.rec, rec.pos) as u32);
                if s.is_empty() {
                    self.store_mut(to).set_int(to.rec, to.pos, 0);
                } else {
                    let into = self.store_mut(to);
                    let s_pos = into.set_str(s) as i32;
                    into.set_int(to.rec, to.pos, s_pos);
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
            Parts::Vector(v) | Parts::Sorted(v, _) => {
                self.copy_claims_seq_vector(rec, to, *v);
            }
            Parts::Array(v) | Parts::Ordered(v, _) => {
                self.copy_claims_array_body(rec, to, *v);
            }
            Parts::Hash(v, _) => {
                self.copy_claims_hash_body(rec, to, *v);
            }
            Parts::Spacial(_, _) => panic!("Not implemented"),
            Parts::Index(_, _, _) => self.copy_claims_index_body(rec, to, tp),
            Parts::Enum(values) => {
                let e_nr = self.store(rec).get_byte(rec.rec, rec.pos, -1);
                let tp = values[e_nr as usize].0;
                // Do not copy claims on simple enumerate types.
                if tp != u16::MAX {
                    self.copy_claims(rec, to, tp);
                }
            }
            _ => {}
        }
    }

    /**
    Remove claimed data for a record. Both strings and substructures are freed.
    It will not free the record itself because that might be a part of a vector.
    # Panics
    When a field points to an index or spacial structure.
    */
    #[allow(clippy::too_many_lines)]
    pub fn remove_claims(&mut self, rec: &DbRef, tp: u16) {
        // TODO prevent removing records twice via secondary structures
        match &self.types[tp as usize].parts {
            Parts::Base if tp == 5 => {
                // text
                let store = self.store_mut(rec);
                let cur = store.get_int(rec.rec, rec.pos);
                if cur == 0 {
                    return;
                }
                store.delete(cur as u32);
                store.set_int(rec.rec, rec.pos, 0);
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields.clone() {
                    self.remove_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos: rec.pos + u32::from(f.position),
                        },
                        f.content,
                    );
                }
            }
            Parts::Vector(v) | Parts::Sorted(v, _) => {
                let tp = *v;
                let length = vector::length_vector(rec, &self.allocations);
                let size = u32::from(self.size(tp));
                let cur = self.store(rec).get_int(rec.rec, rec.pos);
                if cur == 0 {
                    // Do nothing if the structure was empty
                    return;
                }
                for i in 0..length {
                    self.remove_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: cur as u32,
                            pos: 8 + size * i,
                        },
                        tp,
                    );
                }
                let store = self.store_mut(rec);
                store.delete(cur as u32);
                store.set_int(rec.rec, rec.pos, 0);
            }
            Parts::Array(v) | Parts::Ordered(v, _) => {
                let tp = *v;
                let length = vector::length_vector(rec, &self.allocations);
                let cur = self.store(rec).get_int(rec.rec, rec.pos) as u32;
                if cur == 0 {
                    // Do nothing if the structure was empty
                    return;
                }
                for i in 0..length {
                    let elm = self.store(rec).get_int(cur, 8 + i * 4) as u32;
                    self.remove_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: elm,
                            pos: 8,
                        },
                        tp,
                    );
                    self.store_mut(rec).delete(elm);
                }
                let store = self.store_mut(rec);
                store.delete(cur);
                store.set_int(rec.rec, rec.pos, 0);
            }
            Parts::Hash(v, _) => {
                let tp = *v;
                let cur = self.store(rec).get_int(rec.rec, rec.pos) as u32;
                if cur == 0 {
                    // Do nothing if the structure was empty
                    return;
                }
                let length = self.store(rec).get_int(cur, 0) as u32 * 2;
                for i in 0..length {
                    let elm = self.store(rec).get_int(cur, 8 + i * 4) as u32;
                    if elm == 0 {
                        continue;
                    }
                    self.remove_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: elm,
                            pos: 8,
                        },
                        tp,
                    );
                    self.store_mut(rec).delete(elm);
                }
                let store = self.store_mut(rec);
                store.delete(cur);
                store.set_int(rec.rec, rec.pos, 0);
            }
            Parts::Spacial(_, _) => panic!("Not implemented"),
            Parts::Index(c, _, _) => {
                let content_tp = *c;
                let left = self.fields(tp);
                let cur = self.store(rec).get_int(rec.rec, rec.pos) as u32;
                if cur == 0 {
                    return;
                }
                let nodes = self.collect_index_nodes(rec, left);
                for node in nodes {
                    self.remove_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: node,
                            pos: 8,
                        },
                        content_tp,
                    );
                    self.store_mut(rec).delete(node);
                }
                self.store_mut(rec).set_int(rec.rec, rec.pos, 0);
            }
            _ => {}
        }
    }
}
