// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! A store-backed binary PATRICIA (radix) tree, keyed one bit at a time.
//!
//! Use it when records must stay in **key order** and the key is best produced
//! lazily rather than materialised: the tree never asks for a whole key, only for
//! one bit of it.  That is what lets a spatial index interleave several coordinate
//! axes into a Morton code on demand (@PLN48) without ever building the code.
//!
//! The one rule every operation rests on:
//!
//! > **I1** — each internal node stores an *absolute* bit index `bit(n)`; along any
//! > root→leaf path `bit` strictly increases; every record below `n`'s FALSE child
//! > has `key(bit(n)) = 0`, and every record below its TRUE child has `1`.
//!
//! Descending on `key(bit(n))` therefore lands on the single leaf sharing the
//! longest prefix with the search key.  Bits that path compression skipped are
//! never re-checked, so that leaf is a **candidate**: [`rtree_find`] returns it and
//! the caller compares keys, or [`rtree_get`] does the comparing.  In-order
//! traversal (FALSE subtree, then TRUE) emits records in strictly increasing key
//! order.
//!
//! Because `bit(n)` is absolute, no operation accumulates "bits consumed so far"
//! while descending — the class of off-by-one errors that representation invites
//! cannot be written here.  The bits a node *skips* are never stored: the run
//! length is `bit(n) - bit(parent) - 1`, and the values can be read off any leaf
//! below `n`, because every record there agrees on them (**I2**).
//!
//! # The key the tree actually uses
//!
//! A caller supplies a [`KeySpec`]: bit `b` of a record's key, and how many bits
//! that key has.  The tree keys on neither directly.  It keys on the **infinite bit
//! string**
//!
//! ```text
//!     user bits ‖ 0x00 terminator ‖ 32-bit record id ‖ zeros forever
//! ```
//!
//! which is what makes the structure total rather than fragile:
//!
//! * **No key ever ends,** so a comparison never has to decide what "one key
//!   stopped" means.  Prefix-freeness is not an obligation; it is a consequence.
//! * **Distinct records always differ,** because their ids do.  So there is always
//!   a bit to split on, and insertion cannot fail.
//! * **Order is lexicographic.**  The terminator is what buys this, not the
//!   prefix-freeness: without it, `"ab" ‖ id` versus `"abc" ‖ id` would compare a
//!   record id against `'c'`, so which sorted first would depend on allocation.
//!   `0x00` is below every UTF-8 byte, so `"ab"` precedes `"abc"` always.
//! * **A probe key is the same string with id `0`,** so it sorts to the head of its
//!   own bucket.  [`rtree_seek`] on `"ab"` therefore lands on the first key with
//!   prefix `"ab"` — a prefix query needs no separate entry point.
//! * **Fixed-width keys pay nothing for the suffix.**  Every Morton code has the
//!   same length, so no two records ever diverge inside the terminator; path
//!   compression means no node is created there.  One rule, no modes.
//!
//! Two records may share a user key — several entities in one spatial cell.  They
//! differ only in the id suffix, so they land **adjacent** in key order and a
//! "bucket" is just a contiguous run.  No bucket structure exists or is needed.
//!
//! The single assumption: **no key byte is `0x00`**, true for UTF-8 text and vacuous
//! for fixed-width numeric keys.  Were it violated, the ids still differ, so the
//! tree stays a structurally valid PATRICIA; only lexicographic order would
//! disagree with key order for that pair.  It degrades, it does not corrupt — and
//! nothing here panics on any input.
//!
//! Design and the step-by-step verification plan:
//! `doc/claude/plans/48-spacial-index/RADIX_TREE.md`.

// The tree is exercised only by its own tests until @PLN48 deliverable S2 wires it
// to `spacial<T[…]>`; the unit tests are what hold it correct in the meantime.
#![allow(dead_code)]

use crate::store::Store;
use std::cmp::Ordering;

/// Bit `bit` of `rec`'s user key, most significant first.  Called only for
/// `bit < (KeySpec::bits)(rec)`.
pub type BitFn = fn(store: &Store, rec: u32, bit: u32) -> bool;

/// How many bits `rec`'s user key has.  A constant for fixed-width keys; `8 * len`
/// for text.
pub type LenFn = fn(store: &Store, rec: u32) -> u32;

/// How to read a record's key.  The tree extends it with a terminator and the
/// record id; see the module header.
#[derive(Clone, Copy)]
pub struct KeySpec {
    pub bit: BitFn,
    pub bits: LenFn,
}

/// The zero byte that separates a user key from the id suffix, so a shorter key
/// sorts before a longer one that extends it.
const TERM_BITS: u32 = 8;
/// The record id, which makes every key unique.
const ID_BITS: u32 = 32;
/// Everything the tree appends to a user key.
const SUFFIX_BITS: u32 = TERM_BITS + ID_BITS;

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
/// `bit: u32`, `parent: u32`, `false: i32`, `true: i32` — exactly two store words.
const NODE_SIZE: u32 = 16;

/// Bit `bit` of the string the tree really keys on, given the user key's length `n`.
/// Total: past the end of everything it reads as `0`, so the string never ends and
/// comparison is total.
fn full_n(store: &Store, spec: KeySpec, rec: u32, n: u32, bit: u32) -> bool {
    if bit < n {
        (spec.bit)(store, rec, bit)
    } else if bit < n + TERM_BITS {
        false
    } else if bit < n + SUFFIX_BITS {
        let j = bit - n - TERM_BITS;
        (rec >> (ID_BITS - 1 - j)) & 1 == 1
    } else {
        false
    }
}

/// One-off read of a key bit, re-deriving the length.  Prefer [`key_fn`] in a loop.
fn full(store: &Store, spec: KeySpec, rec: u32, bit: u32) -> bool {
    full_n(store, spec, rec, (spec.bits)(store, rec), bit)
}

/// A record's key as a closure, with its length read **once** rather than per bit.
fn key_fn(store: &Store, spec: KeySpec, rec: u32, n: u32) -> impl Fn(u32) -> bool + '_ {
    move |bit| full_n(store, spec, rec, n, bit)
}

/// First bit at which two key strings differ, and the first one's value there.
///
/// `None` means they agree on every bit below `limit`.  For two *distinct* records
/// that cannot happen — their id suffixes differ — so every caller treats `None` as
/// "the same key", never as a failure.
fn first_diff<A, B>(a: &A, b: &B, limit: u32) -> Option<(u32, bool)>
where
    A: Fn(u32) -> bool,
    B: Fn(u32) -> bool,
{
    (0..limit).find_map(|bit| {
        let x = a(bit);
        (x != b(bit)).then_some((bit, x))
    })
}

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

/// A cursor: the record under it, and the node above that record (`0` at the root).
///
/// It holds **no path** — climbing runs on the parent index in each node — so a
/// cursor is three words, `Copy`, and costs no allocation.  That is what lets one
/// seek drive two cursors, walking predecessor and successor outward together:
/// @PLN48's proximity query, with no malloc in the frame loop.
///
/// `budget` is what keeps a corrupted tree from hanging its caller.  A walk over
/// `LEN` records cannot legitimately yield more than `LEN` of them, so a cursor that
/// tries simply stops.  Bounding the parent *climb* is not enough: a stale parent
/// leaves the child pointers a valid tree, so `next` keeps returning records and it
/// is the caller's loop that never ends.  Every intended walk is monotone —
/// iteration, and the outward proximity scan — so the bound never fires on a healthy
/// tree.  It degrades; it does not spin, and it does not panic.
#[derive(Clone, Copy)]
pub struct RadixIter {
    rec: u32,
    node: u32,
    budget: u32,
}

impl RadixIter {
    fn empty() -> RadixIter {
        RadixIter {
            rec: 0,
            node: 0,
            budget: 0,
        }
    }

    /// A cursor at `rec` (whose parent is `node`), allowed to walk the whole tree.
    fn at(store: &Store, tree: u32, rec: u32, node: u32) -> RadixIter {
        RadixIter {
            rec,
            node,
            budget: rtree_len(store, tree) + 1,
        }
    }

    /// The record under the cursor; `0` once the walk is exhausted.
    #[must_use]
    pub fn rec(&self) -> u32 {
        self.rec
    }

    /// Step to the next record in increasing key order.
    ///
    /// Climb the parent chain to the deepest ancestor entered from its FALSE side,
    /// cross to its TRUE child, then take the FALSE-most path down — the in-order
    /// successor.  Amortised O(1) over a full traversal.
    pub fn next(&mut self, store: &Store, tree: u32) -> Option<u32> {
        self.step(store, tree, true)
    }

    /// Step to the previous record, mirroring [`RadixIter::next`].
    pub fn prev(&mut self, store: &Store, tree: u32) -> Option<u32> {
        self.step(store, tree, false)
    }

    fn step(&mut self, store: &Store, tree: u32, forward: bool) -> Option<u32> {
        if self.rec == 0 || self.budget == 0 {
            self.rec = 0;
            return None;
        }
        self.budget -= 1;
        let mut from = Child::Rec(self.rec);
        let mut n = self.node;
        while n != 0 {
            // Which side did we come up from?  The children are distinct, so one
            // comparison settles it.
            let came_true = child(store, tree, n, true) == from;
            if came_true != forward {
                let c = child(store, tree, n, forward);
                let (rec, node) = descend_extreme(store, tree, c, !forward, n);
                self.rec = rec;
                self.node = node;
                return Some(rec);
            }
            from = Child::Node(n);
            n = node_parent(store, tree, n);
        }
        self.rec = 0;
        self.node = 0;
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

/// The node this node hangs under; `0` for the root.
fn node_parent(store: &Store, tree: u32, node: u32) -> u32 {
    store.get_u32_raw(tree, node_off(node) + 4)
}

fn set_node_parent(store: &mut Store, tree: u32, node: u32, parent: u32) {
    store.set_u32_raw(tree, node_off(node) + 4, parent);
}

fn child_off(node: u32, dir: bool) -> u32 {
    node_off(node) + if dir { 12 } else { 8 }
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

/// Follow `key` down to the candidate leaf, and the node directly above it.
/// Leaf `0` = empty tree; parent `0` = the leaf is the root.
fn descend<K>(store: &Store, tree: u32, key: &K) -> (u32, u32)
where
    K: Fn(u32) -> bool,
{
    let mut cur = top(store, tree);
    let mut parent = 0;
    loop {
        match cur {
            Child::Empty => return (0, 0),
            Child::Rec(r) => return (r, parent),
            Child::Node(n) => {
                let dir = key(node_bit(store, tree, n));
                cur = child(store, tree, n, dir);
                parent = n;
            }
        }
    }
}

/// Always branch the same way; reaches the FALSE-most (or TRUE-most) leaf and the
/// node above it.
fn descend_extreme(
    store: &Store,
    tree: u32,
    mut cur: Child,
    dir: bool,
    mut parent: u32,
) -> (u32, u32) {
    loop {
        match cur {
            Child::Empty => return (0, 0),
            Child::Rec(r) => return (r, parent),
            Child::Node(n) => {
                cur = child(store, tree, n, dir);
                parent = n;
            }
        }
    }
}

/// Where a new node testing bit `d` must be spliced in: the node whose child slot
/// points at the subtree that diverges there, and which side.  `parent == 0` means
/// the slot is TOP.
///
/// I1 makes `bit(n) == d` impossible on a descent path: a node testing `d` would
/// have sent the search key and the candidate leaf the same way, so they would agree
/// at `d`, contradicting `d` being their first difference.  So `<` and `>` partition
/// the path and no third case exists.
fn split_point<K>(store: &Store, tree: u32, key: &K, d: u32) -> (u32, bool)
where
    K: Fn(u32) -> bool,
{
    let mut cur = top(store, tree);
    let (mut parent, mut dir) = (0, false);
    while let Child::Node(n) = cur {
        let bit = node_bit(store, tree, n);
        if bit > d {
            break;
        }
        dir = key(bit);
        cur = child(store, tree, n, dir);
        parent = n;
    }
    (parent, dir)
}

/// The child slot `split_point` named — the subtree a split displaces.
fn subtree_below(store: &Store, tree: u32, parent: u32, dir: bool) -> Child {
    if parent == 0 {
        top(store, tree)
    } else {
        child(store, tree, parent, dir)
    }
}

/// A probe key: the caller's bits, then zeros — the same string a record gets, with
/// an id of `0`.  It therefore sorts to the head of its own bucket.
fn probe_key<K>(probe: &K, probe_bits: u32) -> impl Fn(u32) -> bool + '_
where
    K: Fn(u32) -> bool,
{
    move |bit| bit < probe_bits && probe(bit)
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// Position at the **candidate** leaf for a probe — the record sharing its longest
/// prefix.
///
/// Despite the name this is not a lookup: descent skips the bits path compression
/// elided, so for an absent key the candidate is some other record entirely.
/// Compare keys before trusting it, or call [`rtree_get`], which does.
#[must_use]
pub fn rtree_find<K>(store: &Store, tree: u32, probe: &K, probe_bits: u32) -> RadixIter
where
    K: Fn(u32) -> bool,
{
    let (rec, node) = descend(store, tree, &probe_key(probe, probe_bits));
    RadixIter::at(store, tree, rec, node)
}

/// Position at the lowest record whose key is `>= probe` — a lower bound.
///
/// The candidate leaf a descent reaches is *not* generally that record, because the
/// descent skipped the bits path compression elided.  So re-ascend to the subtree
/// that diverges from the probe at bit `d`: by I2 every record in it agrees with the
/// probe below `d` and agrees with the candidate *at* `d`.  If the probe has `0`
/// there it precedes the whole subtree; if `1`, it follows all of it.
///
/// Because a probe carries id `0`, seeking a text prefix lands on the first record
/// bearing that prefix.
#[must_use]
pub fn rtree_seek<K>(
    store: &Store,
    tree: u32,
    probe: &K,
    probe_bits: u32,
    spec: KeySpec,
) -> RadixIter
where
    K: Fn(u32) -> bool,
{
    let kp = probe_key(probe, probe_bits);
    let (cand, cand_parent) = descend(store, tree, &kp);
    if cand == 0 {
        return RadixIter::empty();
    }
    let n = (spec.bits)(store, cand);
    let kc = key_fn(store, spec, cand, n);
    let limit = (probe_bits + SUFFIX_BITS).max(n + SUFFIX_BITS);
    let Some((d, probe_bit)) = first_diff(&kp, &kc, limit) else {
        return RadixIter::at(store, tree, cand, cand_parent);
    };
    let (parent, dir) = split_point(store, tree, &kp, d);
    let sub = subtree_below(store, tree, parent, dir);
    if probe_bit {
        // The probe sorts after every record in the subtree: take its last, step on.
        let (rec, node) = descend_extreme(store, tree, sub, true, parent);
        let mut it = RadixIter::at(store, tree, rec, node);
        it.next(store, tree);
        it
    } else {
        let (rec, node) = descend_extreme(store, tree, sub, false, parent);
        RadixIter::at(store, tree, rec, node)
    }
}

/// Does `rec` carry exactly this user key?  Lets a caller walk a bucket — the
/// contiguous run of records sharing one key — from [`rtree_seek`].
#[must_use]
pub fn rtree_key_eq<K>(store: &Store, rec: u32, probe: &K, probe_bits: u32, spec: KeySpec) -> bool
where
    K: Fn(u32) -> bool,
{
    (spec.bits)(store, rec) == probe_bits
        && (0..probe_bits).all(|bit| (spec.bit)(store, rec, bit) == probe(bit))
}

/// The first record whose user key equals the probe, or `0` when none does.
///
/// This is the exact lookup [`rtree_find`] deliberately is not.  When several
/// records share the key, it is the head of that run; walk on with
/// [`RadixIter::next`] and [`rtree_key_eq`].
#[must_use]
pub fn rtree_get<K>(store: &Store, tree: u32, probe: &K, probe_bits: u32, spec: KeySpec) -> u32
where
    K: Fn(u32) -> bool,
{
    let rec = rtree_seek(store, tree, probe, probe_bits, spec).rec();
    if rec != 0 && rtree_key_eq(store, rec, probe, probe_bits, spec) {
        rec
    } else {
        0
    }
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
    let c = top(store, tree);
    let (rec, node) = descend_extreme(store, tree, c, dir, 0);
    RadixIter::at(store, tree, rec, node)
}

// ---------------------------------------------------------------------------
// Mutation
// ---------------------------------------------------------------------------

/// Insert `rec`, returning the (possibly relocated) tree record id.
///
/// Growth can move the container, so the returned id must replace the caller's.
/// Inserting a record the tree already holds is a no-op.  Two *different* records
/// may share a user key; they differ in the id suffix, so they land adjacent.
///
/// This cannot fail: distinct records always differ somewhere in the key string,
/// so there is always a bit to split on.
pub fn rtree_insert(store: &mut Store, tree: u32, rec: u32, spec: KeySpec) -> u32 {
    let len = rtree_len(store, tree);
    if len == 0 {
        set_top(store, tree, Child::Rec(rec));
        store.set_u32_raw(tree, LEN, 1);
        return tree;
    }

    // Everything the split needs is read before the tree can move.
    let (d, rec_bit, parent, dir, displaced) = {
        let s: &Store = store;
        let n_rec = (spec.bits)(s, rec);
        let ka = key_fn(s, spec, rec, n_rec);
        let (cand, _) = descend(s, tree, &ka);
        if cand == rec {
            return tree;
        }
        let n_cand = (spec.bits)(s, cand);
        let kb = key_fn(s, spec, cand, n_cand);
        let limit = (n_rec + SUFFIX_BITS).max(n_cand + SUFFIX_BITS);
        // Unreachable: two distinct records differ in their id suffix.
        let Some((d, rec_bit)) = first_diff(&ka, &kb, limit) else {
            return tree;
        };
        let (parent, dir) = split_point(s, tree, &ka, d);
        let displaced = subtree_below(s, tree, parent, dir);
        (d, rec_bit, parent, dir, displaced)
    };

    let (tree, node) = alloc_node(store, tree);
    set_node_bit(store, tree, node, d);
    set_node_parent(store, tree, node, parent);
    set_child(store, tree, node, rec_bit, Child::Rec(rec));
    set_child(store, tree, node, !rec_bit, displaced);
    if let Child::Node(sub) = displaced {
        set_node_parent(store, tree, sub, node);
    }

    if parent == 0 {
        set_top(store, tree, Child::Node(node));
    } else {
        set_child(store, tree, parent, dir, Child::Node(node));
    }
    store.set_u32_raw(tree, LEN, len + 1);
    tree
}

/// Remove `rec`; `false` if it is not present.
///
/// Splicing the parent out keeps every internal node at exactly two children,
/// which is what makes `live_nodes == LEN - 1` a total check on the structure.
pub fn rtree_remove(store: &mut Store, tree: u32, rec: u32, spec: KeySpec) -> bool {
    let (leaf, parent) = {
        let s: &Store = store;
        let n = (spec.bits)(s, rec);
        descend(s, tree, &key_fn(s, spec, rec, n))
    };
    if leaf != rec {
        return false;
    }
    let len = rtree_len(store, tree);
    if parent == 0 {
        set_top(store, tree, Child::Empty);
    } else {
        let grand = node_parent(store, tree, parent);
        let rec_is_true = child(store, tree, parent, true) == Child::Rec(rec);
        let sibling = child(store, tree, parent, !rec_is_true);
        if grand == 0 {
            set_top(store, tree, sibling);
        } else {
            let up_is_true = child(store, tree, grand, true) == Child::Node(parent);
            set_child(store, tree, grand, up_is_true, sibling);
        }
        if let Child::Node(sub) = sibling {
            set_node_parent(store, tree, sub, grand);
        }
        free_node(store, tree, parent);
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
/// store, and what [`rtree_seek`] leans on when it argues that the whole divergence
/// subtree sits on one side of the probe.
///
/// **I3** — `parent(child(n, d)) == n`, and the root's parent is `0`.  The parent index
/// is what makes a cursor allocation-free, and the three sites that maintain it
/// (insert's split, remove's splice, the displaced subtree) would each fail silently;
/// this check is what makes them loud.
///
/// I1 and I2 are independent: put two keys that first diverge at bit 0 under a root
/// that tests bit 1, and every I1-shaped check still passes.
///
/// Checking the subtree's least and greatest leaf suffices, by induction: the
/// recursive call establishes that every leaf under a child agrees with its
/// representative on all bits below the child's bit, which strictly exceeds this
/// node's.
#[cfg(test)]
pub fn rtree_validate(store: &Store, tree: u32, spec: KeySpec) {
    struct Walk<'a> {
        store: &'a Store,
        tree: u32,
        spec: KeySpec,
        len: u32,
        leaves: u32,
        live: u32,
        /// Every `(bit, side)` decision on the path down to the current child.
        constraints: Vec<(u32, bool)>,
    }

    impl Walk<'_> {
        /// Returns a representative leaf from each end of this subtree.
        fn visit(&mut self, cur: Child, parent_bit: Option<u32>, up: u32) -> Option<(u32, u32)> {
            match cur {
                Child::Empty => {
                    assert_eq!(self.len, 0, "empty child in a non-empty tree");
                    None
                }
                Child::Rec(r) => {
                    self.leaves += 1;
                    for &(bit, dir) in &self.constraints {
                        assert_eq!(
                            full(self.store, self.spec, r, bit),
                            dir,
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
                    assert_eq!(
                        node_parent(self.store, self.tree, n),
                        up,
                        "I3: node {n} does not point back at the node above it"
                    );
                    let mut ends = [0u32; 2];
                    for dir in [false, true] {
                        let c = child(self.store, self.tree, n, dir);
                        assert_ne!(c, Child::Empty, "node {n} has no {dir} child");
                        self.constraints.push((bit, dir));
                        let (lo, hi) = self.visit(c, Some(bit), n).expect("a child has leaves");
                        self.constraints.pop();
                        ends[usize::from(dir)] = if dir { hi } else { lo };
                    }
                    let (lo, hi) = (ends[0], ends[1]);

                    // I2: the bits this node skipped are common to the whole subtree,
                    // so they never needed storing.
                    let from = parent_bit.map_or(0, |pb| pb + 1);
                    for b in from..bit {
                        assert_eq!(
                            full(self.store, self.spec, lo, b),
                            full(self.store, self.spec, hi, b),
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
        spec,
        len,
        leaves: 0,
        live: 0,
        constraints: Vec::new(),
    };
    let _ = walk.visit(top(store, tree), None, 0);
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

    // ---- fixed-width oracle: a `u32` code at `fld 4` -----------------------

    fn code_bit(store: &Store, rec: u32, bit: u32) -> bool {
        (store.get_u32_raw(rec, 4) >> (31 - bit)) & 1 == 1
    }

    fn code_bits(_store: &Store, _rec: u32) -> u32 {
        32
    }

    const CODE: KeySpec = KeySpec {
        bit: code_bit,
        bits: code_bits,
    };

    /// The tree orders on `(code, rec)`: equal codes tie-break on the id suffix.
    fn ordered_key(code: u32, rec: u32) -> u64 {
        (u64::from(code) << 32) | u64::from(rec)
    }

    fn code_probe(code: u32) -> impl Fn(u32) -> bool {
        move |bit| (code >> (31 - bit)) & 1 == 1
    }

    fn add(store: &mut Store, code: u32) -> u32 {
        let rec = store.claim(1);
        store.set_u32_raw(rec, 4, code);
        rec
    }

    /// Deterministic pseudo-random codes — reproducible failures, no `rand` dep.
    ///
    /// Shifts by 32, not 33: a 31-bit code would pin the key's most significant bit
    /// to `0` for every record, so no node would ever branch on it.
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

    /// R0 — a cursor is three words.  A `Vec` is 24 bytes, so this is a standing
    /// guard that `seek`/`first`/`next` stay allocation-free.
    #[test]
    fn r0_cursor_is_two_words_and_allocates_nothing() {
        assert!(
            std::mem::size_of::<RadixIter>() <= 16,
            "a Vec cannot fit here"
        );
        fn assert_copy<T: Copy>() {}
        assert_copy::<RadixIter>();
    }

    /// R1 — a tree that is initialised and freed leaves no claim behind.
    ///
    /// The host record is claimed first so the tree cannot land on `PRIMARY`, whose
    /// deletion would empty `claims` on its own and hide a leak.
    #[test]
    fn r1_init_and_free_leak_nothing() {
        let mut store = Store::new_in_use(8);
        let host = store.claim(1);
        let tree = rtree_init(&mut store, 4);
        assert_ne!(tree, host, "the tree must claim its own record");
        assert_eq!(rtree_len(&store, tree), 0);
        assert_eq!(store.get_u32_raw(tree, CAP), 4, "room for 4 nodes");

        rtree_free(&mut store, tree);
        store.delete(host);
        assert!(store.claims_empty(), "the tree must leave no claim behind");
    }

    /// R2 — the 0→1→2 transitions, and `find` reaching each record.
    #[test]
    fn r2_insert_two_and_find() {
        let mut store = Store::new_in_use(32);
        let mut tree = rtree_init(&mut store, 4);
        let a = add(&mut store, 0b1000);
        let b = add(&mut store, 0b0100);

        tree = rtree_insert(&mut store, tree, a, CODE);
        assert_eq!(rtree_len(&store, tree), 1);
        assert_eq!(
            store.get_u32_raw(tree, NODES),
            0,
            "one record needs no node"
        );

        tree = rtree_insert(&mut store, tree, b, CODE);
        assert_eq!(rtree_len(&store, tree), 2);
        assert_eq!(store.get_u32_raw(tree, NODES), 1, "NODES == LEN - 1");
        rtree_validate(&store, tree, CODE);

        for r in [a, b] {
            let code = store.get_u32_raw(r, 4);
            assert_eq!(rtree_get(&store, tree, &code_probe(code), 32, CODE), r);
        }
    }

    /// R2b — `rtree_get` answers exactly, where `rtree_find` only guesses.
    #[test]
    fn r2b_get_is_exact_where_find_is_a_candidate() {
        let mut store = Store::new_in_use(32);
        let mut tree = rtree_init(&mut store, 8);
        let recs: Vec<u32> = (0..8).map(|i| add(&mut store, i * 0x0100_0001)).collect();
        for &r in &recs {
            tree = rtree_insert(&mut store, tree, r, CODE);
        }
        for &r in &recs {
            let code = store.get_u32_raw(r, 4);
            assert_eq!(rtree_get(&store, tree, &code_probe(code), 32, CODE), r);
        }
        let absent = code_probe(0xdead_beef);
        assert_ne!(
            rtree_find(&store, tree, &absent, 32).rec(),
            0,
            "find guesses"
        );
        assert_eq!(
            rtree_get(&store, tree, &absent, 32, CODE),
            0,
            "get is exact"
        );
    }

    /// R2c — re-inserting a record the tree already holds is a no-op, not a panic
    /// and not a silent corruption.
    #[test]
    fn r2c_reinserting_the_same_record_is_a_noop() {
        let mut store = Store::new_in_use(32);
        let mut tree = rtree_init(&mut store, 4);
        let a = add(&mut store, 7);
        tree = rtree_insert(&mut store, tree, a, CODE);
        tree = rtree_insert(&mut store, tree, a, CODE);
        assert_eq!(rtree_len(&store, tree), 1, "no phantom second copy");
        rtree_validate(&store, tree, CODE);

        let b = add(&mut store, 9);
        tree = rtree_insert(&mut store, tree, b, CODE);
        tree = rtree_insert(&mut store, tree, b, CODE);
        assert_eq!(rtree_len(&store, tree), 2);
        rtree_validate(&store, tree, CODE);
    }

    /// R2d — two records may share a user key.  They differ only in the id suffix,
    /// so they land adjacent: @PLN48's per-cell bucket, with no bucket structure.
    #[test]
    fn r2d_duplicate_user_keys_form_a_contiguous_bucket() {
        let mut store = Store::new_in_use(32);
        let mut tree = rtree_init(&mut store, 8);
        // Three records in one "cell" (code 42), plus neighbours either side.
        let below = add(&mut store, 41);
        let cell: Vec<u32> = (0..3).map(|_| add(&mut store, 42)).collect();
        let above = add(&mut store, 43);
        for &r in std::iter::once(&below)
            .chain(&cell)
            .chain(std::iter::once(&above))
        {
            tree = rtree_insert(&mut store, tree, r, CODE);
        }
        assert_eq!(rtree_len(&store, tree), 5, "every record is stored");
        rtree_validate(&store, tree, CODE);

        let walk = collect(&store, tree);
        let mut expect = vec![below];
        let mut sorted = cell.clone();
        sorted.sort_unstable(); // within a cell, order is by record id
        expect.extend(sorted.iter().copied());
        expect.push(above);
        assert_eq!(
            walk, expect,
            "the cell's records are contiguous and in id order"
        );

        // Seeking the cell lands on its head; walking it out yields exactly the cell.
        let probe = code_probe(42);
        let mut it = rtree_seek(&store, tree, &probe, 32, CODE);
        assert_eq!(rtree_get(&store, tree, &probe, 32, CODE), sorted[0]);
        let mut bucket = Vec::new();
        let mut r = it.rec();
        while r != 0 && rtree_key_eq(&store, r, &probe, 32, CODE) {
            bucket.push(r);
            r = it.next(&store, tree).unwrap_or(0);
        }
        assert_eq!(bucket, sorted, "the bucket is a contiguous run");
    }

    /// R3 — I1 and I2 hold after every one of a thousand inserts.
    #[test]
    fn r3_insert_many_keeps_the_invariant() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0x1234_5678_9abc_def0;
        for i in 0..1000u32 {
            let rec = add(&mut store, lcg(&mut seed));
            tree = rtree_insert(&mut store, tree, rec, CODE);
            assert_eq!(rtree_len(&store, tree), i + 1);
            rtree_validate(&store, tree, CODE);
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
            expect.push(ordered_key(code, rec));
            tree = rtree_insert(&mut store, tree, rec, CODE);
        }
        expect.sort_unstable();

        let forward = collect(&store, tree);
        let keys: Vec<u64> = forward
            .iter()
            .map(|&r| ordered_key(store.get_u32_raw(r, 4), r))
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
            let code = lcg(&mut seed) & !1; // even codes: odd probes are absent
            let rec = add(&mut store, code);
            keys.push(ordered_key(code, rec));
            tree = rtree_insert(&mut store, tree, rec, CODE);
        }
        keys.sort_unstable();

        let mut probe_seed = 0x5eed_5eed_5eed_5eed_u64;
        for _ in 0..300 {
            let code = lcg(&mut probe_seed) | 1; // odd: absent from the tree
            let it = rtree_seek(&store, tree, &code_probe(code), 32, CODE);
            // A probe carries id 0, so it sorts at the head of its own cell.
            let want = keys.iter().find(|&&k| k >= ordered_key(code, 0)).copied();
            let got =
                (it.rec() != 0).then(|| ordered_key(store.get_u32_raw(it.rec(), 4), it.rec()));
            assert_eq!(got, want, "seek({code:#x}) must be the least key >= it");
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
            oracle.insert(ordered_key(code, rec), rec);
            tree = rtree_insert(&mut store, tree, rec, CODE);
        }

        let mut order: Vec<u32> = oracle.values().copied().collect();
        for i in 0..order.len() {
            let j = (lcg(&mut seed) as usize) % order.len();
            order.swap(i, j);
        }

        for rec in order {
            assert!(rtree_remove(&mut store, tree, rec, CODE), "remove {rec}");
            oracle.retain(|_, &mut v| v != rec);
            rtree_validate(&store, tree, CODE);
            assert_eq!(rtree_len(&store, tree) as usize, oracle.len());
            let want: Vec<u32> = oracle.values().copied().collect();
            assert_eq!(collect(&store, tree), want, "walk tracks the oracle");
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
            tree = rtree_insert(&mut store, tree, r, CODE);
        }
        let high_water = store.get_u32_raw(tree, NODES);
        assert!(rtree_remove(&mut store, tree, recs[3], CODE));
        assert_eq!(store.get_u32_raw(tree, NODES), high_water, "no new node");
        tree = rtree_insert(&mut store, tree, recs[3], CODE);
        assert_eq!(
            store.get_u32_raw(tree, NODES),
            high_water,
            "the freed node id must be reused, not appended"
        );
        rtree_validate(&store, tree, CODE);
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
            expect.push(ordered_key(code, rec));
            let grown = rtree_insert(&mut store, tree, rec, CODE);
            moved |= grown != tree;
            tree = grown;
            rtree_validate(&store, tree, CODE);
        }
        assert!(
            moved,
            "200 inserts from CAP=0 must have relocated the record"
        );

        expect.sort_unstable();
        let keys: Vec<u64> = collect(&store, tree)
            .iter()
            .map(|&r| ordered_key(store.get_u32_raw(r, 4), r))
            .collect();
        assert_eq!(
            keys, expect,
            "growth must preserve every record and its order"
        );
    }

    // ---- variable-length oracle: a byte string at `fld 8`, length at `fld 4` ----

    fn text_bit(store: &Store, rec: u32, bit: u32) -> bool {
        let byte = store.get_byte(rec, 8 + bit / 8, 0) as u32;
        (byte >> (7 - bit % 8)) & 1 == 1
    }

    fn text_bits(store: &Store, rec: u32) -> u32 {
        store.get_u32_raw(rec, 4) * 8
    }

    const TEXT: KeySpec = KeySpec {
        bit: text_bit,
        bits: text_bits,
    };

    fn add_text(store: &mut Store, s: &str) -> u32 {
        let rec = store.claim(2); // 16 bytes: len at 4, up to 8 bytes at 8..16
        store.set_u32_raw(rec, 4, s.len() as u32);
        for (i, b) in s.bytes().enumerate() {
            store.set_byte(rec, 8 + i as u32, 0, i32::from(b));
        }
        rec
    }

    fn text_probe(s: &'static str) -> impl Fn(u32) -> bool {
        move |bit| (s.as_bytes()[(bit / 8) as usize] >> (7 - bit % 8)) & 1 == 1
    }

    /// R8 — variable-length keys sort lexicographically, and a *prefix* probe seeks
    /// to the first record bearing that prefix.
    ///
    /// This is what the `0x00` terminator buys.  Without it `"ab"` versus `"abc"`
    /// would compare a record id against `'c'`, and which sorted first would depend
    /// on allocation order.
    #[test]
    fn r8_text_keys_sort_lexicographically_and_seek_by_prefix() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        // Inserted in an order unrelated to their sort order.
        let words = ["b", "abd", "ab", "abc", "a"];
        let mut rec_of = Vec::new();
        for w in words {
            let r = add_text(&mut store, w);
            rec_of.push((w, r));
            tree = rtree_insert(&mut store, tree, r, TEXT);
        }
        rtree_validate(&store, tree, TEXT);

        let order: Vec<&str> = collect(&store, tree)
            .iter()
            .map(|r| rec_of.iter().find(|(_, x)| x == r).unwrap().0)
            .collect();
        assert_eq!(
            order,
            ["a", "ab", "abc", "abd", "b"],
            "a shorter key must precede the longer key that extends it"
        );

        // A probe carries id 0, so `seek("ab")` lands on "ab" itself, and walking on
        // yields every key with that prefix.
        let it = rtree_seek(&store, tree, &text_probe("ab"), 16, TEXT);
        let head = rec_of.iter().find(|(w, _)| *w == "ab").unwrap().1;
        assert_eq!(it.rec(), head, "seek(\"ab\") lands on \"ab\"");
        assert_eq!(rtree_get(&store, tree, &text_probe("ab"), 16, TEXT), head);

        // `seek` of a prefix that is not itself a key still lands on its first extension.
        let it = rtree_seek(&store, tree, &text_probe("abc"), 24, TEXT);
        assert_eq!(
            it.rec(),
            rec_of.iter().find(|(w, _)| *w == "abc").unwrap().1
        );
        assert_eq!(
            rtree_get(&store, tree, &text_probe("b"), 8, TEXT),
            rec_of[0].1
        );
    }

    /// R8c — the terminator is load-bearing, and only a **large** record id shows it.
    ///
    /// Without the `0x00` terminator, `"ab"` versus `"abc"` compares the id's leading
    /// byte against `'c'` (0x63).  Small ids have a leading byte of `0x00`, which
    /// silently plays the terminator's part — so R8 alone cannot tell the two designs
    /// apart.  These ids lead with `0x70`, which is *above* `'c'`, so dropping the
    /// terminator flips the order.  The tree never dereferences a record, so the
    /// oracle can hand it ids no store would allocate.
    #[test]
    fn r8c_terminator_orders_a_prefix_below_its_extension() {
        const AB: u32 = 0x7000_0001;
        const ABC: u32 = 0x7000_0002;

        fn word(rec: u32) -> &'static [u8] {
            if rec == AB { b"ab" } else { b"abc" }
        }
        fn big_bit(_store: &Store, rec: u32, bit: u32) -> bool {
            (word(rec)[(bit / 8) as usize] >> (7 - bit % 8)) & 1 == 1
        }
        fn big_bits(_store: &Store, rec: u32) -> u32 {
            word(rec).len() as u32 * 8
        }
        const BIG: KeySpec = KeySpec {
            bit: big_bit,
            bits: big_bits,
        };

        assert!(
            AB >> 24 > u32::from(b'c'),
            "the probe needs a leading byte above 'c'"
        );

        let mut store = Store::new_in_use(16);
        let mut tree = rtree_init(&mut store, 4);
        for r in [ABC, AB] {
            tree = rtree_insert(&mut store, tree, r, BIG);
        }
        rtree_validate(&store, tree, BIG);
        assert_eq!(
            collect(&store, tree),
            vec![AB, ABC],
            "\"ab\" must precede \"abc\" whatever the record ids are"
        );
    }

    /// R8b — a text key repeated: distinct records, same key, adjacent, both found.
    #[test]
    fn r8b_repeated_text_keys_are_distinct_records() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        let a = add_text(&mut store, "dup");
        let b = add_text(&mut store, "dup");
        let c = add_text(&mut store, "dux");
        for r in [a, b, c] {
            tree = rtree_insert(&mut store, tree, r, TEXT);
        }
        assert_eq!(rtree_len(&store, tree), 3);
        rtree_validate(&store, tree, TEXT);

        let walk = collect(&store, tree);
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        assert_eq!(
            walk,
            vec![lo, hi, c],
            "equal keys are adjacent, ordered by id"
        );
        assert_eq!(rtree_get(&store, tree, &text_probe("dup"), 24, TEXT), lo);
    }
}
