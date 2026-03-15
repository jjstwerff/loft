// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Memory/store allocation helpers and claim management.

use crate::database::{Parts, Stores};
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
        if self.max >= self.allocations.len() as u16 {
            self.allocations.push(Store::new(100));
        } else {
            self.allocations[self.max as usize].init();
        }
        let store = &mut self.allocations[self.max as usize];
        assert!(store.free, "Allocating a used store");
        store.free = false;
        let rec = if size == u32::MAX {
            0
        } else {
            store.claim(size)
        };
        self.max += 1;
        DbRef {
            store_nr: self.max - 1,
            rec,
            pos: 8,
        }
    }

    /**
    Free a reference to a store. Make it available again for later code.
    # Panics
    When the code doesn't free the last claimed store first.
    */
    pub fn free(&mut self, db: &DbRef) {
        // u16::MAX is the null-sentinel used by OpNullRefSentinel for inline-ref temporaries
        // that were never assigned a real store.  Nothing to free in this case.
        if db.store_nr == u16::MAX {
            return;
        }
        let al = db.store_nr;
        debug_assert!(al < self.allocations.len() as u16, "Incorrect store");
        debug_assert!(!self.allocations[al as usize].free, "Double free store");
        debug_assert!(
            al == self.max - 1,
            "Stores must be freed in LIFO order: freeing store {al} but max is {}",
            self.max
        );
        self.allocations[al as usize].free = true;
        self.max -= 1;
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
        debug_assert!(
            !self.allocations[db.store_nr as usize].free,
            "Use after free"
        );
    }

    pub fn clear(&mut self, db: &DbRef) {
        let store = &mut self.allocations[db.store_nr as usize];
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

    /// Clone all current stores as locked read-only copies for use in a worker thread.
    /// The returned `Stores` has the same type schema but no files and no `parallel_ctx`.
    /// When a worker `State` is created from this, `State::new()` will allocate its own
    /// stack store at index `self.max` without conflicting with the cloned data stores.
    /// Freed slots (store.free == true) are replaced with fresh empty stores so that
    /// `State::new_worker → Stores::database` can safely re-initialise them without
    /// hitting the "Write to locked store" debug assert.
    #[must_use]
    pub fn clone_for_worker(&self) -> Stores {
        let allocations = self
            .allocations
            .iter()
            .map(|s| {
                if s.free {
                    super::super::store::Store::new(100)
                } else {
                    s.clone_locked()
                }
            })
            .collect();
        Stores {
            types: self.types.clone(),
            names: self.names.clone(),
            allocations,
            files: Vec::new(),
            max: self.max,
            parallel_ctx: None,
            logger: self.logger.clone(),
            scratch: Vec::new(),
            had_fatal: false,
            start_time: self.start_time,
        }
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
        let into = self.store_mut(to).claim(1 + (size * cur).div_ceil(8));
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
            _ => unreachable!(),
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
