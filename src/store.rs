// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! An in memory store that can be allocated in small steps.
//! A store has a structure of unclaimed data.
//! There can be a mapped file behind each storage instead of only memory.
//!
//! There is always a specific record as the main record of a store describing vectors and indexes with sub-records.
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]

use mmap_storage::file::Storage as MmapStorage;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt::{Debug, Formatter};

#[allow(dead_code)]
static A: System = System;
const SIGNATURE: u32 = 0x53_74_6f_31;
pub const PRIMARY: u32 = 1;

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

pub struct Store {
    // format 0 = SIGNATURE, 4 = free_space_index, 8 = record_size, 12 = content
    pub ptr: *mut u8,
    claims: HashSet<u32>,
    size: u32,
    file: Option<MmapStorage>,
    pub(crate) free: bool,
    /// When `true`, all writes to this store are illegal.
    /// In debug builds this panics; in release builds writes are silently discarded.
    pub locked: bool,
    /// Root of the LLRB free-space tree (0 = empty).
    /// Populated lazily: `open()` calls `fl_rebuild()`; `new()` starts empty
    /// and the tree fills as blocks are freed.
    free_root: u32,
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
        if self.file.is_none() {
            let l = Layout::from_size_align(self.size as usize * 8, 8).expect("Problem");
            unsafe { A.dealloc(self.ptr, l) };
        }
    }
}

#[allow(dead_code)]
impl Store {
    /// Total capacity of this store in bytes.
    #[must_use]
    pub fn byte_capacity(&self) -> u64 {
        u64::from(self.size) * 8
    }

    pub fn new(size: u32) -> Store {
        let l = Layout::from_size_align(size as usize * 8, 8).expect("Problem");
        let ptr = unsafe { A.alloc(l) };
        let mut store = Store {
            ptr,
            size,
            claims: HashSet::new(),
            file: None,
            free: true,
            locked: false,
            free_root: 0,
        };
        store.init(); // sets claims = {PRIMARY} and free_root = 0
        store
    }

    pub fn open(path: &str) -> Store {
        let mut file = MmapStorage::open(path).expect("Opening file");
        let size = (file.capacity() / 8) as u32;
        let init = if size < 1024 {
            file.resize(8192).unwrap();
            true
        } else {
            false
        };
        let ptr = std::ptr::addr_of!(file.as_slice()[0]).cast_mut();
        let mut store = Store {
            file: Some(file),
            ptr,
            claims: HashSet::new(),
            size,
            free: true,
            locked: false,
            free_root: 0,
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

    /// Claim the space of a record
    /// # Arguments
    /// * `size` - The requested record size in 8 byte words
    pub fn claim(&mut self, size: u32) -> u32 {
        debug_assert!(!self.locked, "Claim on locked store (size={size})");
        #[cfg(not(debug_assertions))]
        if self.locked {
            return 0;
        }
        assert!(size >= 1, "Incomplete record");
        #[cfg(debug_assertions)]
        self.fl_validate();
        // Fast path: find the smallest tracked free block that fits.
        if let Some(pos) = self.fl_take_ge(size as i32) {
            let result = self.claim_block(pos, size);
            #[cfg(debug_assertions)]
            self.fl_validate();
            return result;
        }
        // Slow path: linear scan (handles size-1 blocks and first-time allocation).
        let result = self.claim_scan(size);
        #[cfg(debug_assertions)]
        self.fl_validate();
        result
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
        self.resize_store(if last_claim < 0 {
            (self.size as i32 + size as i32 + last_claim) as u32
        } else {
            self.size + size
        });
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
                if claim - next_size > act {
                    let new_next = rec + act as u32;
                    let new_free_size = (-next_size) as u32 + next - new_next;
                    *self.addr_mut(rec, 0) = act;
                    *self.addr_mut(new_next, 0) = -(new_free_size as i32);
                    self.fl_insert(new_next);
                } else {
                    *self.addr_mut(rec, 0) = claim - next_size;
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
        debug_assert!(!self.locked, "Delete on locked store (rec={rec})");
        #[cfg(not(debug_assertions))]
        if self.locked {
            return;
        }
        self.valid(rec, 4);
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
        #[cfg(debug_assertions)]
        self.fl_validate();
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

    /// Change the store size, do not mutate content
    fn resize_store(&mut self, to_size: u32) {
        if to_size < self.size {
            return;
        }
        let inc = self.size * 7 / 3;
        let size = if to_size > inc { to_size } else { inc };
        if let Some(f) = &mut self.file {
            f.resize(size as usize * 8).expect("Resize");
            self.ptr = std::ptr::addr_of!(f.as_slice()[0]).cast_mut();
            self.size = size;
            return;
        }
        let bytes = size as usize * 8;
        let l = Layout::from_size_align(self.size as usize * 8, 8).expect("Problem");
        self.ptr = unsafe { A.realloc(self.ptr, l, bytes) };
        self.size = size;
    }

    /// Lock this store against writes. In debug builds any subsequent write panics.
    /// In release builds writes are silently discarded.
    pub fn lock(&mut self) {
        self.locked = true;
    }

    /// Unlock this store (only callable from Rust; loft code cannot unlock via d#lock = false
    /// on a const variable).
    pub fn unlock(&mut self) {
        self.locked = false;
    }

    /// Return the current lock state.
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.locked
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
            file: None,
            free: self.free,
            locked: true,
            free_root: 0, // workers never claim/delete; no free tree needed
        }
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

    #[inline]
    pub fn addr<T>(&self, rec: u32, fld: u32) -> &T {
        debug_assert!(
            rec as isize * 8 + fld as isize + std::mem::size_of::<T>() as isize
                <= self.size as isize * 8,
            "Store read out of bounds: rec={rec} fld={fld} size={} store_size={}",
            std::mem::size_of::<T>(),
            self.size * 8,
        );
        unsafe {
            let off = self.ptr.offset(rec as isize * 8 + fld as isize).cast::<T>();
            off.as_mut().expect("Reference")
        }
    }

    #[inline]
    pub fn addr_mut<T>(&mut self, rec: u32, fld: u32) -> &mut T {
        debug_assert!(!self.locked, "Write to locked store at rec={rec} fld={fld}");
        debug_assert!(
            rec as isize * 8 + fld as isize + std::mem::size_of::<T>() as isize
                <= self.size as isize * 8,
            "Store write out of bounds: rec={rec} fld={fld} size={} store_size={}",
            std::mem::size_of::<T>(),
            self.size * 8,
        );
        #[cfg(not(debug_assertions))]
        if self.locked {
            // In release builds silently discard the write by returning a thread-local dummy.
            thread_local! {
                static DUMMY: std::cell::UnsafeCell<[u8; 256]> =
                    const { std::cell::UnsafeCell::new([0u8; 256]) };
            }
            return DUMMY
                .with(|d| unsafe { ((*d.get()).as_mut_ptr() as *mut T).as_mut().expect("dummy") });
        }
        unsafe {
            let off = self.ptr.offset(rec as isize * 8 + fld as isize).cast::<T>();
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

    /// Try to validate a record reference as much as possible.
    /// Complete validations are only done in 'test' mode.
    pub fn valid(&self, rec: u32, fld: u32) -> bool {
        debug_assert!(self.claims.contains(&rec), "Unknown record {rec}");
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
    pub fn get_int(&self, rec: u32, fld: u32) -> i32 {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            i32::MIN
        }
    }

    #[inline]
    pub fn set_int(&mut self, rec: u32, fld: u32, val: i32) -> bool {
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
            } else if val >= min || val <= min + 65536 {
                *self.addr_mut(rec, fld) = (val - min + 1) as u16;
                true
            } else {
                false
            }
        } else {
            false
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
            } else if val >= min || val <= min + 256 {
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
        let len = self.get_int(rec, 4);
        #[cfg(debug_assertions)]
        assert!(
            len >= 0 && len <= self.addr::<i32>(rec, 0) * 8,
            "Inconsistent text store"
        );
        assert!(
            (len / 8) as u32 + rec <= self.size,
            "Inconsistent text store"
        );
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
        self.set_int(res, 4, val.len() as i32);
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
        self.set_int(res, 4, len as i32);
        unsafe {
            std::ptr::copy_nonoverlapping(ptr, self.ptr.offset(res as isize * 8 + 8), len);
        }
        res
    }

    #[inline]
    pub fn append_str(&mut self, record: u32, val: &str) -> u32 {
        let prev = self.get_int(record, 4);
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
        if self.valid(rec, fld) {
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
        if self.valid(rec, fld) {
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
// `addr_mut()` on a locked store panics in debug and discards in release.
unsafe impl Send for Store {}
