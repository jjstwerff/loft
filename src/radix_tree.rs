// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! A store-backed binary PATRICIA (radix) tree, keyed one bit at a time.
//!
//! Use it when records must stay in **key order** and the key is best produced
//! lazily rather than materialised: the tree never asks for a whole key, only for
//! `key(rec, bit)`.  That is what lets a spatial index interleave several
//! coordinate axes into a Morton code on demand (@PLN48) without ever building
//! the code itself.
//!
//! The one rule every operation rests on:
//!
//! > **I1** — each internal node stores an *absolute* bit index `bit(n)`; along any
//! > root→leaf path `bit` strictly increases; every record below `n`'s FALSE child
//! > has `key(bit(n)) = 0`, and every record below its TRUE child has `1`.
//!
//! Descending on `key(bit(n))` therefore lands on the single leaf sharing the
//! longest prefix with the search key.  Bits that path compression skipped are
//! never re-checked, so that leaf is a **candidate**: `find` returns it, and the
//! caller compares full keys.  In-order traversal (FALSE subtree, then TRUE)
//! emits records in strictly increasing key order.
//!
//! Because `bit(n)` is absolute, no operation accumulates "bits consumed so far"
//! while descending — the class of off-by-one errors that representation invites
//! cannot be written here.
//!
//! # Obligations on the key oracle
//!
//! Bit `0` is the most significant.  `None` means the key has ended.
//!
//! * **P1 — prefix-closed:** `Some` for every `bit < len(rec)`, `None` beyond.
//! * **P2 — prefix-free:** no key is a proper prefix of another.  A node branches
//!   on a *bit*; it cannot branch on "the key ended here".
//! * **P3 — distinct:** no two live records share a key, or no differing bit
//!   exists and [`rtree_insert`] has no bit to split on.
//!
//! Fixed-width keys (an integer, a Morton code) satisfy P2 for free.  Text keys
//! need a virtual `NUL` terminator, which is sound because loft text is UTF-8 and
//! excludes `0x00`.  Both discharge P3 by appending the record id.  Violations of
//! P2 and P3 trip a `debug_assert` rather than silently building a tree that
//! breaks I1.
//!
//! Design and the step-by-step verification plan:
//! `doc/claude/plans/48-spacial-index/RADIX_TREE.md`.

// The tree is exercised only by its own tests until @PLN48 deliverable S2 wires it
// to `spacial<T[…]>`; the unit tests are what hold it correct in the meantime.
#![allow(dead_code)]

use crate::store::Store;
use std::cmp::Ordering;

/// Supplies bit `bit` of `rec`'s key, most significant first; `None` past its end.
pub type KeyFn = fn(store: &Store, rec: u32, bit: u32) -> Option<bool>;

// Container-record header, in bytes.  `fld 0..4` is the Store's own claim header.
/// Root child: `0` empty, `>0` a record id, `<0` a node id.
const TOP: u32 = 4;
/// Number of records held.
const LEN: u32 = 8;
/// Node high-water mark; live ids are drawn from `1..=NODES`.
const NODES: u32 = 12;
/// How many nodes the current claim can hold.
const CAP: u32 = 16;
/// Head of the free-node list, `0` when empty.
const FREE: u32 = 20;
/// First node sits here; a multiple of 8, so nodes stay word-aligned.
const HDR: u32 = 24;
/// `bit: u32`, `false: i32`, `true: i32`.
const NODE_SIZE: u32 = 12;

/// Guards a key oracle that never returns `None` against an unbounded descent.
const MAX_KEY_BITS: u32 = 1 << 20;

/// A child slot: the sign encoding lives here and nowhere else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Child {
    Empty,
    Rec(u32),
    Node(u32),
}

impl Child {
    fn decode(v: i32) -> Child {
        match v.cmp(&0) {
            Ordering::Equal => Child::Empty,
            Ordering::Greater => Child::Rec(v as u32),
            Ordering::Less => Child::Node(v.unsigned_abs()),
        }
    }

    fn encode(self) -> i32 {
        match self {
            Child::Empty => 0,
            Child::Rec(r) => r as i32,
            Child::Node(n) => -(n as i32),
        }
    }
}

/// One branch taken during a descent: which node, and down which side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Step {
    node: u32,
    took_true: bool,
}

/// A position in the tree, plus the path that reached it.
///
/// The path holds node *ids*, not addresses, so it survives the record relocation
/// a growing [`rtree_insert`] can cause.
pub struct RadixIter {
    path: Vec<Step>,
    rec: u32,
}

impl RadixIter {
    fn empty() -> RadixIter {
        RadixIter {
            path: Vec::new(),
            rec: 0,
        }
    }

    /// The record under the cursor; `0` once the walk is exhausted.
    #[must_use]
    pub fn rec(&self) -> u32 {
        self.rec
    }

    /// Step to the next record in increasing key order.
    ///
    /// Climb until we find a node we entered on the FALSE side, cross to its TRUE
    /// child, then take the FALSE-most path down — the in-order successor.
    pub fn next(&mut self, store: &Store, tree: u32) -> Option<u32> {
        self.step(store, tree, true)
    }

    /// Step to the previous record, mirroring [`RadixIter::next`].
    pub fn prev(&mut self, store: &Store, tree: u32) -> Option<u32> {
        self.step(store, tree, false)
    }

    fn step(&mut self, store: &Store, tree: u32, forward: bool) -> Option<u32> {
        if self.rec == 0 {
            return None;
        }
        while let Some(s) = self.path.pop() {
            if s.took_true == forward {
                continue;
            }
            self.path.push(Step {
                node: s.node,
                took_true: forward,
            });
            let c = child(store, tree, s.node, forward);
            self.rec = descend_extreme(store, tree, c, !forward, &mut self.path);
            return Some(self.rec);
        }
        self.rec = 0;
        None
    }
}

// ---------------------------------------------------------------------------
// Container + node accessors
// ---------------------------------------------------------------------------

fn node_off(node: u32) -> u32 {
    debug_assert!(node >= 1, "node ids are 1-based");
    HDR + NODE_SIZE * (node - 1)
}

fn node_bit(store: &Store, tree: u32, node: u32) -> u32 {
    store.get_u32_raw(tree, node_off(node))
}

fn set_node_bit(store: &mut Store, tree: u32, node: u32, bit: u32) {
    store.set_u32_raw(tree, node_off(node), bit);
}

fn child_off(node: u32, dir: bool) -> u32 {
    node_off(node) + if dir { 8 } else { 4 }
}

fn child(store: &Store, tree: u32, node: u32, dir: bool) -> Child {
    Child::decode(store.get_i32_raw(tree, child_off(node, dir)))
}

fn set_child(store: &mut Store, tree: u32, node: u32, dir: bool, c: Child) {
    store.set_i32_raw(tree, child_off(node, dir), c.encode());
}

fn top(store: &Store, tree: u32) -> Child {
    Child::decode(store.get_i32_raw(tree, TOP))
}

fn set_top(store: &mut Store, tree: u32, c: Child) {
    store.set_i32_raw(tree, TOP, c.encode());
}

fn cap_for_words(words: u32) -> u32 {
    (words * 8).saturating_sub(HDR) / NODE_SIZE
}

/// Number of records in the tree.
#[must_use]
pub fn rtree_len(store: &Store, tree: u32) -> u32 {
    store.get_u32_raw(tree, LEN)
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Claim an empty tree with room for `initial` nodes; returns its record id.
///
/// The record id is the tree's identity and is stored in the owning field, the
/// way a hash collection stores its bucket record.  [`rtree_insert`] may move the
/// record, so always keep the id it returns.
pub fn rtree_init(store: &mut Store, initial: u32) -> u32 {
    let words = (HDR + NODE_SIZE * initial).div_ceil(8).max(HDR / 8);
    let tree = store.claim(words);
    let cap = cap_for_words(store.record_words(tree));
    store.set_i32_raw(tree, TOP, 0);
    store.set_u32_raw(tree, LEN, 0);
    store.set_u32_raw(tree, NODES, 0);
    store.set_u32_raw(tree, CAP, cap);
    store.set_u32_raw(tree, FREE, 0);
    tree
}

/// Release the tree's own record.  The records it indexed are not touched.
pub fn rtree_free(store: &mut Store, tree: u32) {
    store.delete(tree);
}

/// Grow the node array, returning the (possibly relocated) tree record id.
fn grow(store: &mut Store, tree: u32) -> u32 {
    let words = store.record_words(tree);
    let tree = store.resize(tree, (words * 2).max(words + 2));
    let cap = cap_for_words(store.record_words(tree));
    store.set_u32_raw(tree, CAP, cap);
    tree
}

/// Take a node id from the free list, or mint a fresh one, growing if needed.
fn alloc_node(store: &mut Store, tree: u32) -> (u32, u32) {
    let free = store.get_u32_raw(tree, FREE);
    if free != 0 {
        let next = store.get_i32_raw(tree, child_off(free, false)) as u32;
        store.set_u32_raw(tree, FREE, next);
        return (tree, free);
    }
    let used = store.get_u32_raw(tree, NODES);
    let tree = if used >= store.get_u32_raw(tree, CAP) {
        grow(store, tree)
    } else {
        tree
    };
    store.set_u32_raw(tree, NODES, used + 1);
    (tree, used + 1)
}

/// Thread a dead node onto the free list through its FALSE slot.
fn free_node(store: &mut Store, tree: u32, node: u32) {
    let head = store.get_u32_raw(tree, FREE);
    store.set_i32_raw(tree, child_off(node, false), head as i32);
    store.set_u32_raw(tree, FREE, node);
}

// ---------------------------------------------------------------------------
// Descent
// ---------------------------------------------------------------------------

/// Follow `key` down to the candidate leaf, recording the path.  Leaf `0` = empty.
fn descend<K>(store: &Store, tree: u32, key: &K) -> (Vec<Step>, u32)
where
    K: Fn(u32) -> Option<bool>,
{
    let mut path = Vec::new();
    let mut cur = top(store, tree);
    loop {
        match cur {
            Child::Empty => return (path, 0),
            Child::Rec(r) => return (path, r),
            Child::Node(n) => {
                let dir = key(node_bit(store, tree, n)).unwrap_or(false);
                path.push(Step {
                    node: n,
                    took_true: dir,
                });
                cur = child(store, tree, n, dir);
            }
        }
    }
}

/// Always branch the same way; reaches the FALSE-most (or TRUE-most) leaf.
fn descend_extreme(
    store: &Store,
    tree: u32,
    mut cur: Child,
    dir: bool,
    path: &mut Vec<Step>,
) -> u32 {
    loop {
        match cur {
            Child::Empty => return 0,
            Child::Rec(r) => return r,
            Child::Node(n) => {
                path.push(Step {
                    node: n,
                    took_true: dir,
                });
                cur = child(store, tree, n, dir);
            }
        }
    }
}

/// First bit at which `key` and `rec`'s key differ, and `key`'s value there.
///
/// `None` means the two keys are identical — an exact hit for a lookup, and a P3
/// violation for an insert, which is why only the caller can judge it.
fn first_diff_bit<K>(store: &Store, key: &K, rec: u32, key_of: KeyFn) -> Option<(u32, bool)>
where
    K: Fn(u32) -> Option<bool>,
{
    let mut bit = 0;
    while bit < MAX_KEY_BITS {
        match (key(bit), key_of(store, rec, bit)) {
            (Some(a), Some(b)) if a != b => return Some((bit, a)),
            (Some(_), Some(_)) => {}
            (None, None) => return None,
            _ => {
                debug_assert!(
                    false,
                    "P2: one key is a proper prefix of the other at {bit}"
                );
                return None;
            }
        }
        bit += 1;
    }
    debug_assert!(false, "key oracle never ended within {MAX_KEY_BITS} bits");
    None
}

/// Index of the first step whose node tests a bit at or beyond `d`.
///
/// I1 makes `bit(n) == d` impossible on a descent path: a node testing `d` would
/// have sent the search key and the candidate leaf the same way, so they would
/// agree at `d`, contradicting `d` being their first difference.
fn split_index(store: &Store, tree: u32, path: &[Step], d: u32) -> usize {
    debug_assert!(
        path.iter().all(|s| node_bit(store, tree, s.node) != d),
        "I1 violated: a node on the path already tests bit {d}"
    );
    path.iter()
        .position(|s| node_bit(store, tree, s.node) > d)
        .unwrap_or(path.len())
}

/// The child slot hanging below `path[..i]` — the subtree the split displaces.
fn subtree_at(store: &Store, tree: u32, path: &[Step], i: usize) -> Child {
    if i == 0 {
        top(store, tree)
    } else {
        let p = path[i - 1];
        child(store, tree, p.node, p.took_true)
    }
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// Position at the candidate leaf for `key` — the record sharing its longest prefix.
///
/// The candidate is not necessarily a match: compare full keys before trusting it.
#[must_use]
pub fn rtree_find<K>(store: &Store, tree: u32, key: &K) -> RadixIter
where
    K: Fn(u32) -> Option<bool>,
{
    let (path, rec) = descend(store, tree, key);
    RadixIter { path, rec }
}

/// Position at the lowest record, walking forward from there.
#[must_use]
pub fn rtree_first(store: &Store, tree: u32) -> RadixIter {
    straight(store, tree, false)
}

/// Position at the highest record, walking backward from there.
#[must_use]
pub fn rtree_last(store: &Store, tree: u32) -> RadixIter {
    straight(store, tree, true)
}

fn straight(store: &Store, tree: u32, dir: bool) -> RadixIter {
    let mut path = Vec::new();
    let c = top(store, tree);
    let rec = descend_extreme(store, tree, c, dir, &mut path);
    RadixIter { path, rec }
}

/// Position at the lowest record whose key is `>= key` (a lower bound).
///
/// The candidate leaf a descent reaches is *not* generally that record, because
/// the descent skipped the bits path compression elided.  So re-ascend to the
/// subtree that diverges from `key` at bit `d`: every record in it agrees with
/// `key` below `d` and agrees with the candidate *at* `d`.  If `key` has `0`
/// there it precedes the whole subtree; if `1`, it follows all of it.
#[must_use]
pub fn rtree_seek<K>(store: &Store, tree: u32, key: &K, key_of: KeyFn) -> RadixIter
where
    K: Fn(u32) -> Option<bool>,
{
    let (path, cand) = descend(store, tree, key);
    if cand == 0 {
        return RadixIter::empty();
    }
    let Some((d, key_bit)) = first_diff_bit(store, key, cand, key_of) else {
        return RadixIter { path, rec: cand };
    };
    let i = split_index(store, tree, &path, d);
    let sub = subtree_at(store, tree, &path, i);
    let mut trimmed: Vec<Step> = path[..i].to_vec();
    if key_bit {
        // `key` sorts after every record in the subtree: take its last, step on.
        let rec = descend_extreme(store, tree, sub, true, &mut trimmed);
        let mut it = RadixIter { path: trimmed, rec };
        it.next(store, tree);
        it
    } else {
        let rec = descend_extreme(store, tree, sub, false, &mut trimmed);
        RadixIter { path: trimmed, rec }
    }
}

// ---------------------------------------------------------------------------
// Mutation
// ---------------------------------------------------------------------------

/// Insert `rec`, returning the (possibly relocated) tree record id.
///
/// Growth can move the container, so the returned id must replace the caller's.
pub fn rtree_insert(store: &mut Store, tree: u32, rec: u32, key_of: KeyFn) -> u32 {
    let len = rtree_len(store, tree);
    if len == 0 {
        set_top(store, tree, Child::Rec(rec));
        store.set_u32_raw(tree, LEN, 1);
        return tree;
    }

    // Everything the split needs is read before the tree can move.
    let (path, d, rec_bit, displaced) = {
        let s: &Store = store;
        let key = |bit: u32| key_of(s, rec, bit);
        let (path, cand) = descend(s, tree, &key);
        let Some((d, rec_bit)) = first_diff_bit(s, &key, cand, key_of) else {
            debug_assert!(false, "P3: a record with this key is already in the tree");
            return tree;
        };
        let i = split_index(s, tree, &path, d);
        let displaced = subtree_at(s, tree, &path, i);
        (path[..i].to_vec(), d, rec_bit, displaced)
    };

    let (tree, node) = alloc_node(store, tree);
    set_node_bit(store, tree, node, d);
    set_child(store, tree, node, rec_bit, Child::Rec(rec));
    set_child(store, tree, node, !rec_bit, displaced);

    match path.last() {
        None => set_top(store, tree, Child::Node(node)),
        Some(p) => set_child(store, tree, p.node, p.took_true, Child::Node(node)),
    }
    store.set_u32_raw(tree, LEN, len + 1);
    tree
}

/// Remove `rec`; `false` if it is not present.
///
/// Splicing the parent out keeps every internal node at exactly two children,
/// which is what makes `live_nodes == LEN - 1` a total check on the structure.
pub fn rtree_remove(store: &mut Store, tree: u32, rec: u32, key_of: KeyFn) -> bool {
    let (path, leaf) = {
        let s: &Store = store;
        descend(s, tree, &|bit: u32| key_of(s, rec, bit))
    };
    if leaf != rec {
        return false;
    }
    let len = rtree_len(store, tree);
    match path.last() {
        None => set_top(store, tree, Child::Empty),
        Some(p) => {
            let sibling = child(store, tree, p.node, !p.took_true);
            match path.len() {
                1 => set_top(store, tree, sibling),
                n => {
                    let gp = path[n - 2];
                    set_child(store, tree, gp.node, gp.took_true, sibling);
                }
            }
            free_node(store, tree, p.node);
        }
    }
    store.set_u32_raw(tree, LEN, len - 1);
    true
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Assert I1 and I2 over the whole tree.
///
/// **I1** — strictly increasing bits, two children per node, every leaf on the
/// side its key bit chooses, and the counts a two-children-per-node tree forces
/// (`leaves == LEN`, `live_nodes == LEN - 1`, `live + freed == NODES`).  A leaked
/// node, a double-freed node, and a failed splice each break one of these.
///
/// **I2 — the licence to skip bits.**  Every record under node `n` agrees on
/// *every* bit below `bit(n)`.  This is what makes a skipped bit unnecessary to
/// store: its run length is `bit(n) - bit(parent) - 1`, and its values can be read
/// off any leaf in the subtree.  It is also what [`rtree_seek`] leans on when it
/// argues that the whole divergence subtree sits on one side of the probe key.
///
/// Checking the subtree's least and greatest leaf suffices: leaves under `n` are
/// contiguous in key order, so if the two extremes share a bit-prefix, everything
/// between them does too.
#[cfg(test)]
pub fn rtree_validate(store: &Store, tree: u32, key_of: KeyFn) {
    struct Walk<'a> {
        store: &'a Store,
        tree: u32,
        key_of: KeyFn,
        len: u32,
        leaves: u32,
        live: u32,
        /// Every `(bit, side)` decision on the path down to the current child.
        constraints: Vec<(u32, bool)>,
    }

    impl Walk<'_> {
        /// Returns the least and greatest leaf of this subtree, in key order.
        fn visit(&mut self, cur: Child, parent_bit: Option<u32>) -> Option<(u32, u32)> {
            match cur {
                Child::Empty => {
                    assert_eq!(self.len, 0, "empty child in a non-empty tree");
                    None
                }
                Child::Rec(r) => {
                    self.leaves += 1;
                    for &(bit, dir) in &self.constraints {
                        assert_eq!(
                            (self.key_of)(self.store, r, bit),
                            Some(dir),
                            "record {r} sits on the {dir} side of bit {bit}, its key disagrees"
                        );
                    }
                    Some((r, r))
                }
                Child::Node(n) => {
                    self.live += 1;
                    let bit = node_bit(self.store, self.tree, n);
                    if let Some(pb) = parent_bit {
                        assert!(bit > pb, "I1: bit {bit} does not exceed parent bit {pb}");
                    }
                    let mut ends = [0u32; 2];
                    for dir in [false, true] {
                        let c = child(self.store, self.tree, n, dir);
                        assert_ne!(c, Child::Empty, "node {n} has no {dir} child");
                        self.constraints.push((bit, dir));
                        let (lo, hi) = self.visit(c, Some(bit)).expect("a child has leaves");
                        self.constraints.pop();
                        ends[usize::from(dir)] = if dir { hi } else { lo };
                    }
                    let (lo, hi) = (ends[0], ends[1]);

                    // I2: the bits this node skipped over are common to the whole
                    // subtree, so they never needed storing.
                    let from = parent_bit.map_or(0, |pb| pb + 1);
                    for b in from..bit {
                        assert_eq!(
                            (self.key_of)(self.store, lo, b),
                            (self.key_of)(self.store, hi, b),
                            "I2: node {n} skips bit {b} on its way to bit {bit}, \
                             but records {lo} and {hi} below it disagree there"
                        );
                    }
                    Some((lo, hi))
                }
            }
        }
    }

    let len = rtree_len(store, tree);
    let mut walk = Walk {
        store,
        tree,
        key_of,
        len,
        leaves: 0,
        live: 0,
        constraints: Vec::new(),
    };
    let _ = walk.visit(top(store, tree), None);
    let (leaves, live) = (walk.leaves, walk.live);
    assert_eq!(leaves, len, "leaf count disagrees with LEN");
    assert_eq!(live, len.saturating_sub(1), "live nodes must be LEN - 1");

    let mut freed = 0u32;
    let mut f = store.get_u32_raw(tree, FREE);
    while f != 0 {
        freed += 1;
        assert!(freed <= store.get_u32_raw(tree, NODES), "cyclic free list");
        f = store.get_i32_raw(tree, child_off(f, false)) as u32;
    }
    assert_eq!(
        live + freed,
        store.get_u32_raw(tree, NODES),
        "live + freed nodes must account for every minted node"
    );
}

// ---------------------------------------------------------------------------
// Tests — the steps of doc/claude/plans/48-spacial-index/RADIX_TREE.md §7
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A record is one word holding a `u32` code at `fld 4`.  Its key is that code
    /// followed by the record id: 64 fixed bits, so P2 holds by equal length and
    /// P3 by the unique id.
    fn key_of(store: &Store, rec: u32, bit: u32) -> Option<bool> {
        match bit {
            0..=31 => {
                let code = store.get_u32_raw(rec, 4);
                Some((code >> (31 - bit)) & 1 == 1)
            }
            32..=63 => Some((rec >> (63 - bit)) & 1 == 1),
            _ => None,
        }
    }

    /// The same key as a `u64`, for ordering oracles.
    fn full_key(code: u32, rec: u32) -> u64 {
        (u64::from(code) << 32) | u64::from(rec)
    }

    fn search_key(k: u64) -> impl Fn(u32) -> Option<bool> {
        move |bit| (bit < 64).then(|| (k >> (63 - bit)) & 1 == 1)
    }

    fn add(store: &mut Store, code: u32) -> u32 {
        let rec = store.claim(1);
        store.set_u32_raw(rec, 4, code);
        rec
    }

    /// Deterministic pseudo-random codes — reproducible failures, no `rand` dep.
    ///
    /// Shifts by 32, not 33: a 31-bit code would pin the key's most significant
    /// bit to `0` for every record, so no node would ever branch on it and the
    /// top of the key would go untested.
    fn lcg(seed: &mut u64) -> u32 {
        *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (*seed >> 32) as u32
    }

    fn collect(store: &Store, tree: u32) -> Vec<u32> {
        let mut it = rtree_first(store, tree);
        let mut out = Vec::new();
        let mut r = it.rec();
        while r != 0 {
            out.push(r);
            r = it.next(store, tree).unwrap_or(0);
        }
        out
    }

    /// R1 — a tree that is initialised and freed leaves no claim behind.
    ///
    /// The host record is claimed first so the tree cannot land on `PRIMARY`,
    /// whose deletion would empty `claims` on its own and hide a leak.  A tree
    /// that claimed a second block (as the old side `bits` vector did) and freed
    /// only one would fail the final assertion.
    #[test]
    fn r1_init_and_free_leak_nothing() {
        let mut store = Store::new_in_use(8);
        let host = store.claim(1);
        let tree = rtree_init(&mut store, 4);
        assert_ne!(tree, host, "the tree must claim its own record");
        assert_eq!(rtree_len(&store, tree), 0);
        assert_eq!(
            store.get_u32_raw(tree, CAP),
            4,
            "room for the 4 asked-for nodes"
        );

        rtree_free(&mut store, tree);
        store.delete(host);
        assert!(
            store.claims_empty(),
            "freeing the tree and its host must leave no claim behind"
        );
    }

    /// R2 — the 0→1→2 transitions, and `find` reaching each record.
    #[test]
    fn r2_insert_two_and_find() {
        let mut store = Store::new_in_use(32);
        let mut tree = rtree_init(&mut store, 4);
        let a = add(&mut store, 0b1000);
        let b = add(&mut store, 0b0100);

        tree = rtree_insert(&mut store, tree, a, key_of);
        assert_eq!(rtree_len(&store, tree), 1);
        assert_eq!(
            store.get_u32_raw(tree, NODES),
            0,
            "one record needs no node"
        );

        tree = rtree_insert(&mut store, tree, b, key_of);
        assert_eq!(rtree_len(&store, tree), 2);
        assert_eq!(store.get_u32_raw(tree, NODES), 1, "NODES == LEN - 1");
        rtree_validate(&store, tree, key_of);

        for r in [a, b] {
            let code = store.get_u32_raw(r, 4);
            let it = rtree_find(&store, tree, &search_key(full_key(code, r)));
            assert_eq!(it.rec(), r, "find must reach record {r}");
        }
    }

    /// R3 — I1 holds after every one of a thousand inserts.
    #[test]
    fn r3_insert_many_keeps_the_invariant() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0x1234_5678_9abc_def0;
        for i in 0..1000u32 {
            let rec = add(&mut store, lcg(&mut seed));
            tree = rtree_insert(&mut store, tree, rec, key_of);
            assert_eq!(rtree_len(&store, tree), i + 1);
            rtree_validate(&store, tree, key_of);
        }
    }

    /// R4 — in-order traversal is strictly increasing; backward is its exact reverse.
    #[test]
    fn r4_walk_is_ordered_both_ways() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0xfeed_face_dead_beef;
        let mut expect = Vec::new();
        for _ in 0..500 {
            let code = lcg(&mut seed);
            let rec = add(&mut store, code);
            expect.push(full_key(code, rec));
            tree = rtree_insert(&mut store, tree, rec, key_of);
        }
        expect.sort_unstable();

        let forward = collect(&store, tree);
        let keys: Vec<u64> = forward
            .iter()
            .map(|&r| full_key(store.get_u32_raw(r, 4), r))
            .collect();
        assert_eq!(keys, expect, "in-order walk must be sorted by key");

        let mut it = rtree_last(&store, tree);
        let mut backward = vec![it.rec()];
        while let Some(r) = it.prev(&store, tree) {
            backward.push(r);
        }
        backward.reverse();
        assert_eq!(
            backward, forward,
            "the reverse walk must mirror the forward"
        );
    }

    /// R5 — `seek` is a true lower bound, including for keys the tree never held.
    #[test]
    fn r5_seek_is_a_lower_bound() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0x0bad_c0de_0bad_c0de;
        let mut keys = Vec::new();
        for _ in 0..300 {
            // Even codes only, so odd probe codes are guaranteed absent.
            let code = lcg(&mut seed) & !1;
            let rec = add(&mut store, code);
            keys.push(full_key(code, rec));
            tree = rtree_insert(&mut store, tree, rec, key_of);
        }
        keys.sort_unstable();

        let mut probe = 0x5eed_5eed_5eed_5eed_u64;
        for _ in 0..300 {
            probe = probe
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let k = probe | 1 << 32; // force an odd code: absent from the tree
            let it = rtree_seek(&store, tree, &search_key(k), key_of);
            let want = keys.iter().find(|&&x| x >= k).copied();
            let got = (it.rec() != 0).then(|| full_key(store.get_u32_raw(it.rec(), 4), it.rec()));
            assert_eq!(got, want, "seek({k:#x}) must be the least key >= k");
        }

        // And an exact hit lands on the record itself.
        for &k in &keys {
            let it = rtree_seek(&store, tree, &search_key(k), key_of);
            assert_eq!(full_key(store.get_u32_raw(it.rec(), 4), it.rec()), k);
        }
    }

    /// R6 — removal against a `BTreeMap` oracle, ending in an empty, leak-free tree.
    #[test]
    fn r6_remove_matches_a_btreemap() {
        let mut store = Store::new_in_use(64);
        let host = store.claim(1);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0xa5a5_5a5a_a5a5_5a5a;
        let mut oracle: BTreeMap<u64, u32> = BTreeMap::new();
        for _ in 0..300 {
            let code = lcg(&mut seed);
            let rec = add(&mut store, code);
            oracle.insert(full_key(code, rec), rec);
            tree = rtree_insert(&mut store, tree, rec, key_of);
        }

        let mut order: Vec<u32> = oracle.values().copied().collect();
        for i in 0..order.len() {
            let j = (lcg(&mut seed) as usize) % order.len();
            order.swap(i, j);
        }

        for rec in order {
            assert!(rtree_remove(&mut store, tree, rec, key_of), "remove {rec}");
            oracle.retain(|_, &mut v| v != rec);
            rtree_validate(&store, tree, key_of);
            assert_eq!(rtree_len(&store, tree) as usize, oracle.len());
            let walk = collect(&store, tree);
            let want: Vec<u32> = oracle.values().copied().collect();
            assert_eq!(
                walk, want,
                "walk must track the oracle after removing {rec}"
            );
            store.delete(rec);
        }

        assert_eq!(rtree_len(&store, tree), 0);
        rtree_free(&mut store, tree);
        store.delete(host);
        assert!(store.claims_empty(), "removal must not leak");
    }

    /// R6b — a removed node's id is reused rather than minted afresh.
    #[test]
    fn r6b_free_list_reuses_node_ids() {
        let mut store = Store::new_in_use(32);
        let mut tree = rtree_init(&mut store, 8);
        let recs: Vec<u32> = (0..8).map(|i| add(&mut store, i * 17)).collect();
        for &r in &recs {
            tree = rtree_insert(&mut store, tree, r, key_of);
        }
        let high_water = store.get_u32_raw(tree, NODES);
        assert!(rtree_remove(&mut store, tree, recs[3], key_of));
        assert_eq!(store.get_u32_raw(tree, NODES), high_water, "no new node");
        tree = rtree_insert(&mut store, tree, recs[3], key_of);
        assert_eq!(
            store.get_u32_raw(tree, NODES),
            high_water,
            "the freed node id must be reused, not appended"
        );
        rtree_validate(&store, tree, key_of);
    }

    /// R7 — growing past `CAP` relocates the record, and the tree survives it.
    #[test]
    fn r7_growth_relocates_and_survives() {
        let mut store = Store::new_in_use(16);
        let mut tree = rtree_init(&mut store, 0);
        assert_eq!(store.get_u32_raw(tree, CAP), 0, "no room for nodes yet");

        let mut seed = 0x2222_3333_4444_5555;
        let mut expect = Vec::new();
        let mut moved = false;
        for _ in 0..200 {
            let code = lcg(&mut seed);
            let rec = add(&mut store, code);
            expect.push(full_key(code, rec));
            let grown = rtree_insert(&mut store, tree, rec, key_of);
            moved |= grown != tree;
            tree = grown;
            rtree_validate(&store, tree, key_of);
        }
        assert!(
            moved,
            "200 inserts from CAP=0 must have relocated the record"
        );

        expect.sort_unstable();
        let keys: Vec<u64> = collect(&store, tree)
            .iter()
            .map(|&r| full_key(store.get_u32_raw(r, 4), r))
            .collect();
        assert_eq!(
            keys, expect,
            "growth must preserve every record and its order"
        );
    }
}
