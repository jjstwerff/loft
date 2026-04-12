// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A radix tree implementation.
//! This is especially useful for spacial indexes.
#![allow(dead_code)]

use crate::store::Store;

static RAD_TOP: u32 = 4;
static RAD_SIZE: u32 = 8;
static RAD_BITS: u32 = 12;
// position of the bits vector
static RAD_FALSE: u32 = 12;
// first node = 1
static RAD_TRUE: u32 = 16; // first node = 1

const MAX_DEPTH: usize = 64;

pub struct RadixIter {
    // The last position will be the found record reference
    // All those before will be node references, negative means the FALSE side, positive TRUE
    positions: [i32; MAX_DEPTH],
    depth: i32,
    rec: u32,
}

impl RadixIter {
    #[allow(clippy::unused_self)] // stub for not-yet-implemented radix tree iterator; &mut self is kept to match the planned interface
    pub fn next(&mut self, _store: &Store, _tree: u32) -> Option<u32> {
        // in node: walk FALSE, walk TRUE; when encountered ref => up again in positions
        None
    }

    pub fn remove() {
        // collapse current node
        // move last node here; find that record & move last found pos
    }

    fn node(&self) -> i32 {
        self.positions[self.depth as usize - 1]
    }

    fn add(&mut self, node: i32, bit: bool) {
        self.positions[self.depth as usize] = if bit { -node } else { node };
        self.depth += 1;
    }
}

pub fn rtree_init(store: &mut Store, initial: u32) -> u32 {
    let tree = store.claim(2 + initial);
    let bits = store.claim(1 + initial / 8);
    store.set_int(tree, RAD_TOP, 0);
    store.set_int(tree, RAD_SIZE, 0);
    store.set_int(tree, RAD_BITS, bits as i32);
    store.set_byte(bits, 4, 0, 0);
    tree
}

fn get_node(store: &Store, tree: u32, node: u32, bit: bool) -> i32 {
    store.get_int(tree, if bit { RAD_TRUE } else { RAD_FALSE } + node * 8)
}

fn set_node(store: &mut Store, tree: u32, node: u32, bit: bool, val: i32) {
    store.set_int(tree, if bit { RAD_TRUE } else { RAD_FALSE } + node * 8, val);
}

fn get_bits(store: &Store, bits: u32, node: u32) -> u32 {
    if node > 0 {
        store.get_byte(bits, 3 + node, 0) as u32
    } else {
        0
    }
}

fn set_bits(store: &mut Store, bits: u32, node: u32, nr: u32) {
    if node > 0 {
        store.set_byte(bits, 3 + node, nr as i32, 0);
    }
}

/// Return an iterator pointing at the lowest record
pub fn rtree_first(store: &Store, tree: u32) -> RadixIter {
    straight(store, tree, false)
}

/// Return an iterator pointing at the highest record, walking backwards
pub fn rtree_last(store: &Store, tree: u32) -> RadixIter {
    straight(store, tree, true)
}

/// Always step the same direction till a record reference is found
fn straight(store: &Store, tree: u32, dir: bool) -> RadixIter {
    let mut res = RadixIter {
        positions: [0; MAX_DEPTH],
        depth: 0,
        rec: 0,
    };
    let mut node = store.get_int(tree, RAD_TOP);
    while node < 0 {
        res.add(node, dir);
        node = get_node(store, tree, node as u32, dir);
    }
    res.rec = node as u32;
    res
}

/// Return an iterator to the lowest matching record
pub fn rtree_find<F>(store: &Store, tree: u32, key: F) -> RadixIter
where
    F: Fn(u32) -> bool,
{
    let bits = store.get_int(tree, RAD_BITS) as u32;
    let mut res = RadixIter {
        positions: [0; MAX_DEPTH],
        depth: 0,
        rec: 0,
    };
    // There are no nodes to check
    if store.get_int(tree, RAD_SIZE) == 0 {
        return res;
    }
    let mut node = store.get_int(tree, RAD_TOP);
    let mut bit = get_bits(store, bits, (-node) as u32);
    while node < 0 {
        let check = key(bit);
        res.add(node, check);
        node = get_node(store, tree, (-node) as u32, check);
        bit += get_bits(store, bits, (-node) as u32);
    }
    res.rec = node as u32;
    res
}

/// Compare bits of two record keys
fn compare_bits(
    store: &Store,
    rec: u32,
    cur: u32,
    higher: &mut bool,
    key: fn(store: &Store, rec: u32, bit: u32) -> Option<bool>,
) -> u32 {
    let mut bit = 0;
    loop {
        let new = key(store, rec, bit);
        let sto = key(store, cur, bit);
        match (new, sto) {
            (None, None) => break,
            (Some(_b), None) => break,
            (None, Some(_c)) => {
                *higher = false;
                break;
            }
            (Some(b), Some(c)) => {
                if b != c {
                    if c {
                        *higher = false;
                    }
                    break;
                }
            }
        }
        bit += 1;
    }
    bit
}

/// Insert a new record into the tree
pub fn rtree_insert(
    store: &mut Store,
    tree: u32,
    rec: u32,
    key: fn(store: &Store, rec: u32, bit: u32) -> Option<bool>,
) {
    let size = store.get_int(tree, RAD_SIZE);
    store.set_int(tree, RAD_SIZE, size + 1);
    if size == 0 {
        // no node on first element
        store.set_int(tree, RAD_TOP, rec as i32);
        return;
    }
    let it = rtree_find(store, tree, |bit| key(store, rec, bit).unwrap_or(false));
    let cur = it.rec;
    let bits = store.get_int(tree, RAD_BITS) as u32;
    // assume the new key is higher, so we place equal keys in order of entering
    let mut higher = true;
    let diff_bit = compare_bits(store, rec, cur, &mut higher, key);
    // Top node
    if size == 1 {
        store.set_int(tree, RAD_TOP, -size);
        set_node(store, tree, size as u32, higher, rec as i32);
        set_node(store, tree, size as u32, !higher, cur as i32);
        set_bits(store, bits, size as u32, 0);
        return;
    }
    // Find node to split, this is not necessarily the last
    let mut bit = 0;
    let mut node = 0;
    for depth in 0..it.depth {
        node = it.positions[depth as usize];
        let skip = get_bits(store, bits, (-node) as u32);
        if diff_bit < bit + skip {
            // we found the node to split
            break;
        }
        bit += skip;
    }
    // Split the node
    set_bits(store, bits, (-node) as u32, diff_bit - bit);
}

/// Validate the consistency of the tree
#[cfg(test)]
pub fn rtree_validate(
    store: &Store,
    tree: u32,
    _key: fn(store: &Store, rec: u32, bit: u32) -> Option<bool>,
) {
    let size = store.get_int(tree, RAD_SIZE);
    let bits = store.get_int(tree, RAD_BITS) as u32;
    let mut it = rtree_first(store, tree);
    let mut _rec = it.rec;
    let mut count = 0;
    while let Some(_v) = it.next(store, tree) {
        let _node = it.node();
        let cur = it.rec;
        let _skip = get_bits(store, bits, i32::abs(it.node()) as u32);
        // validate 0 on rec's
        // get the key bits of rec and cur
        // validate the ordering
        // validate the number of skipped bits
        _rec = cur;
        count += 1;
    }
    assert_eq!(count, size, "Incorrect number of elements");
}

/// Fully optimize = start a new vector
pub fn rtree_optimize(store: &Store, tree: u32) {
    let mut it = rtree_first(store, tree);
    while let Some(_v) = it.next(store, tree) {
        // fill next store
    }
}
