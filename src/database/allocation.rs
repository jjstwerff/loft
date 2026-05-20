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
        // Maintain the invariant `max == highest_allocated_index + 1`.
        // The dispenser path can yield indices > current max (each
        // dispense skips ahead in parent-namespace), so a bare
        // `slot == self.max` check would leave max stale and
        // subsequent free() trims to the wrong position.
        if slot >= self.max {
            self.max = slot + 1;
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
        debug_assert!(al < self.allocations.len() as u16, "Incorrect store");
        let store = &mut self.allocations[al as usize];
        if store.free {
            return; // Already freed — no-op (replaces Issue #120 tolerance hack).
        }
        // Reference counting: decrement and only free when rc drops to 0.
        if store.ref_count > 1 {
            store.ref_count -= 1;
            if std::env::var("LOFT_STORES").as_deref() == Ok("log") {
                let label = if name.is_empty() { "" } else { name };
                eprintln!(
                    "[store]   dec_rc #{al} {label:>12} | rc={}",
                    store.ref_count
                );
            }
            return;
        }
        // P259 commit 4: cascade-free closure-record DbRef attributes.
        // When the store being freed holds a `__closure_*` record,
        // each Parts::DbRef field references a captured cell whose rc
        // was inc'd at capture time (commit 2 OpIncRc emission).
        // Walk those fields, read each 12-byte stored DbRef, and
        // recursively free_named so the cell's rc drops back to its
        // pre-capture level.  Cells shared between multiple closures
        // only actually free when the LAST referencing closure dies
        // (recursive free_named respects rc).
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
        if std::env::var("LOFT_STORES").as_deref() == Ok("log") {
            let active = self.allocations.iter().filter(|s| !s.free).count();
            let label = if name.is_empty() { "" } else { name };
            eprintln!(
                "[store] - free   #{al} {label:>12} | active={:<4} max={}",
                active, self.max
            );
        }
        // S36: clear the lock before marking free.
        let store = &mut self.allocations[al as usize];
        store.ref_count = 0;
        store.unlock();
        // LOFT_LOG=poison_free: overwrite the freed buffer with a
        // recognisable pattern so subsequent stale-DbRef reads hit
        // loud garbage (0xDEADBEEF repeated) instead of whatever bytes
        // the allocator happens to have left.  Skip the size-header
        // word (offset 0..8) so the bitmap/housekeeping can still read
        // the "freed" marker; start poisoning from offset 8.
        if self.poison_free {
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

    /// Increment the reference count of the store at `store_nr`.
    /// No-op for the null sentinel (u16::MAX) and store 0 (stack store).
    pub fn inc_rc(&mut self, store_nr: u16) {
        if store_nr == u16::MAX || store_nr as usize >= self.allocations.len() {
            return;
        }
        self.allocations[store_nr as usize].ref_count += 1;
    }

    /// Decrement the reference count of the store at `store_nr`.
    /// Returns true if the store was actually freed (rc dropped to 0).
    /// No-op for the null sentinel (u16::MAX).
    pub fn dec_rc(&mut self, store_nr: u16) -> bool {
        if store_nr == u16::MAX || store_nr as usize >= self.allocations.len() {
            return false;
        }
        let store = &mut self.allocations[store_nr as usize];
        if store.free {
            return false;
        }
        if store.ref_count <= 1 {
            // Last reference — actually free the store.
            return true;
        }
        store.ref_count -= 1;
        false
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
        // Note: accessing a freed store can still happen when a closure captures
        // a variable whose store was freed by copy_record's source-free.
        // The rc system prevents double-free; this access is benign (reads stale data
        // that will be overwritten).  A full fix requires inc_rc on closure capture.
    }

    pub fn clear(&mut self, db: &DbRef) {
        let store = &mut self.allocations[db.store_nr as usize];
        // Clear any stale lock before reinitialising — OpDatabase may
        // reinitialise a store that was previously locked by a const
        // parameter in a prior function call within the same loop iteration.
        // never unlock a constant store (ref_count >= u32::MAX / 2).
        if store.ref_count < u32::MAX / 2 {
            store.unlock();
        }
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
        let n = recs.len();
        // C60 piece 3 edit A: allocate IN THE HASH'S STORE, not a
        // fresh one.  This makes the yielded scratch DbRef share
        // `store_nr` with the hash records — so when Ordered
        // iteration yields `DbRef{store=scratch.store_nr, rec=<u32
        // rec-nr from vector>, pos=8}`, the rec-nr resolves to a
        // valid hash record in the same store.  No new on=4 mode,
        // no bytecode protocol change — hash iteration reuses the
        // existing Ordered (on=3) path.
        //
        // 8-byte header + n * 4 bytes of u32 rec-nrs, rounded up to
        // 8-byte words (store claim granularity).
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
            files: Vec::new(),
            max: self.max,
            free_bits,
            scratch: Vec::new(),
            const_refs: self.const_refs.clone(),
            last_parse_errors: Vec::new(),
            last_json_errors: Vec::new(),
            parallel_ctx: None,
            par_buffer_stack: Vec::new(),
            par_text_buffer_stack: Vec::new(),
            par_ref_buffer_stack: Vec::new(),
            par_narrow_buffer_stack: Vec::new(),
            par_fn_buffer_stack: Vec::new(),
            logger: self.logger.clone(),
            had_fatal: false,
            runtime_error: None,
            format_fault_tag: None,
            source_dir: String::new(),
            frame_yield: false,
            poison_free: self.poison_free,
            disable_slot_reuse: self.disable_slot_reuse,
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
            files: Vec::new(),
            max: self.allocations.len() as u16 + pool_slice.len() as u16,
            free_bits,
            scratch: Vec::new(),
            const_refs: self.const_refs.clone(),
            last_parse_errors: Vec::new(),
            last_json_errors: Vec::new(),
            parallel_ctx: None,
            par_buffer_stack: Vec::new(),
            par_text_buffer_stack: Vec::new(),
            par_ref_buffer_stack: Vec::new(),
            par_narrow_buffer_stack: Vec::new(),
            par_fn_buffer_stack: Vec::new(),
            logger: self.logger.clone(),
            had_fatal: false,
            runtime_error: None,
            format_fault_tag: None,
            source_dir: String::new(),
            frame_yield: false,
            poison_free: self.poison_free,
            disable_slot_reuse: self.disable_slot_reuse,
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

    pub(super) fn copy_claims_seq_vector(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let length = vector::length_vector(rec, &self.allocations);
        let size = u32::from(self.size(tp));
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
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        if cur == 0 {
            self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
            return;
        }
        let into = self.store_mut(to).claim(1 + cur.div_ceil(2));
        self.store_mut(to).set_u32_raw(to.rec, to.pos, into);
        for i in 0..length {
            let elm = self.store(rec).get_u32_raw(cur, 8 + 4 * i);
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
                tp,
            );
        }
    }

    pub(super) fn copy_claims_hash_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let size = u32::from(self.size(tp));
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        if cur == 0 {
            self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
            return;
        }
        // @P290 / sibling — the hash bucket record's layout, per
        // `src/hash.rs::add`, is:
        //   offset 0: room   (i32, in words — total record size)
        //   offset 4: length (u32, current entry count)
        //   offset 8..: elms = (room - 1) * 2 bucket slots (u32 rec-nrs)
        // The previous code read `room` from offset 0 (calling it
        // `length`), claimed `room` words correctly, but then iterated
        // `1..room*2` — which both skipped the first bucket (offset 8,
        // i=0) AND walked 2 indices PAST the claim (offsets 72 + 76 for
        // a 72-byte allocation), corrupting heap metadata → SIGSEGV.
        // The correct bound is `elms = (room - 1) * 2` starting at i=0.
        let room = self.store(rec).get_u32_raw(cur, 0);
        let elms = (room - 1) * 2;
        let into = self.store_mut(to).claim(room);
        self.store_mut(to).set_u32_raw(to.rec, to.pos, into);
        for i in 0..elms {
            let elm = self.store(rec).get_u32_raw(cur, 8 + 4 * i);
            if elm == 0 {
                self.store_mut(to).set_u32_raw(into, 8 + 4 * i, 0);
                continue;
            }
            // @P295 — element record layout (per `record_new`'s
            // Hash arm): offset 0 = record header, offset 4 = back-pointer,
            // offset 8 = struct payload (`size` bytes).  The OLD copy claimed
            // `size.div_ceil(8)` words (missing the +1 header word) and
            // `copy_block`'d `size - 4` bytes from offset 4 — reaching only
            // offset `size`, UNDER-copying the struct (which ends at
            // `8 + size`) and leaving the last field's bytes as whatever
            // stale data the reclaimed-after-`remove_claims` memory held
            // (nondeterministic garbage in the high bits of an i64 field).
            // Match the proven `copy_claims_index_body`: claim with the
            // header word, set the back-pointer, copy the full `size`-byte
            // payload from offset 8.
            let new = self.store_mut(to).claim(1 + size.div_ceil(8));
            self.store_mut(to).set_u32_raw(new, 4, to.rec);
            self.copy_block(
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
                size,
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
        let nodes = self.collect_index_nodes(rec, left);
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
    When a field points to a spacial structure.
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
            Parts::Vector(v) | Parts::Sorted(v, _) => {
                self.copy_claims_seq_vector(rec, to, *v);
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
                        let src_store: &Store;
                        let dst_store: &mut Store;
                        unsafe {
                            src_store = crate::keys::store(
                                &src_db,
                                &*std::ptr::from_ref::<[Store]>(&self.allocations),
                            );
                            dst_store = crate::keys::mut_store(
                                to,
                                &mut *std::ptr::from_mut::<[Store]>(&mut self.allocations),
                            );
                        }
                        src_store.copy_block_between(
                            src_rec,
                            8,
                            dst_store,
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
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    return;
                }
                store.delete(cur);
                store.set_u32_raw(rec.rec, rec.pos, 0);
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
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    // Do nothing if the structure was empty
                    return;
                }
                for i in 0..length {
                    self.remove_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: cur,
                            pos: 8 + size * i,
                        },
                        tp,
                    );
                }
                let store = self.store_mut(rec);
                store.delete(cur);
                store.set_u32_raw(rec.rec, rec.pos, 0);
            }
            Parts::ChildRec(content_kt) => {
                // P213: read rec-id; if 0, nothing to free.  Otherwise
                // recurse to free child's nested heap data first, then
                // delete the child record itself.  Resets the field's
                // u32 to 0 so reassignment / re-host-construction starts
                // from a clean slate.
                let content_kt = *content_kt;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    return;
                }
                let child_db = DbRef {
                    store_nr: rec.store_nr,
                    rec: cur,
                    pos: 8,
                };
                self.remove_claims(&child_db, content_kt);
                let store = self.store_mut(rec);
                store.delete(cur);
                store.set_u32_raw(rec.rec, rec.pos, 0);
            }
            Parts::Array(v) | Parts::Ordered(v, _) => {
                let tp = *v;
                let length = vector::length_vector(rec, &self.allocations);
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    // Do nothing if the structure was empty
                    return;
                }
                for i in 0..length {
                    let elm = self.store(rec).get_u32_raw(cur, 8 + i * 4);
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
                store.set_u32_raw(rec.rec, rec.pos, 0);
            }
            Parts::Hash(v, _) => {
                let tp = *v;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    // Do nothing if the structure was empty
                    return;
                }
                // @P290 sibling — the hash bucket record (per `src/hash.rs::add`)
                // is: offset 0 = room (words), offset 4 = entry count, offset
                // 8.. = (room - 1) * 2 bucket slots.  The bound MUST be
                // `(room - 1) * 2`; the old `room * 2` walked 2 slots past the
                // allocation, reading the next record's header bytes as
                // bucket rec-nrs and calling `delete()` on garbage → SIGSEGV.
                // Matches `copy_claims_hash_body`'s @P290 fix; surfaced by
                // @P295 (keyed-local reassignment calls `remove_claims` on a
                // hash via `OpReplaceKeyed`).
                let length = (self.store(rec).get_u32_raw(cur, 0) - 1) * 2;
                for i in 0..length {
                    let elm = self.store(rec).get_u32_raw(cur, 8 + i * 4);
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
                store.set_u32_raw(rec.rec, rec.pos, 0);
            }
            Parts::Spacial(_, _) => panic!("Not implemented"),
            Parts::Index(c, _, _) => {
                let content_tp = *c;
                let left = self.fields(tp);
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
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
                self.store_mut(rec).set_u32_raw(rec.rec, rec.pos, 0);
            }
            _ => {}
        }
    }
}
