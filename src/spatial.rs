// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! Proximity queries over moving 2D points — @PLN48 deliverable S3, at the Rust
//! layer that `spacial<T[x,y]>` will call.
//!
//! A point is a store record with two `u32` coordinates, `x` at byte offset 4 and
//! `y` at offset 8.  Its key in the [`crate::radix_tree`] is the **Morton (Z-order)
//! code** [`morton`] of `(x, y)`: interleaving the two axes so that spatially near
//! points share a long key prefix and land close together in the tree.  The tree
//! itself stays geometry-free — it only ever sees the 64-bit code — so all distance
//! reasoning lives here.
//!
//! Three queries, two exact and one deliberately not:
//!
//! * [`within`] — every point inside a radius.  **Exact.**
//! * [`nearest`] — the `k` closest points.  **Exact**, by an expanding box.
//! * [`Near`] — points in roughly-increasing distance, cheaply.  **Approximate**:
//!   it walks the Z-order curve, which is close to distance order but jumps at
//!   quadrant boundaries, so it is for "good enough" uses (aggro, interest
//!   management), never for a correct radius or k-NN.  Use [`within`] / [`nearest`]
//!   when the answer must be right.
//!
//! The exactness of the first two rests on one property of the Morton code:
//!
//! > **Monotonicity.** `morton(x, y) = spread(x) + 2·spread(y)`, and `spread` is
//! > strictly increasing, so the code is increasing in each axis separately.  A box
//! > `[x0,x1] × [y0,y1]` therefore has its least code at `(x0, y0)` and its greatest
//! > at `(x1, y1)`, and every point of the box has a code in `[morton(x0,y0),
//! > morton(x1,y1)]`.
//!
//! So scanning the tree across that code interval visits **every** point of the box
//! (plus some outside it, where Z-order leaves and re-enters — those the distance
//! test drops).  No point of the box is ever skipped, which is what makes a scan a
//! sound basis for an exact query.

// Exercised by its own tests until @PLN48 deliverable S2 wires it to the stdlib.
#![allow(dead_code)]

use crate::radix_tree::{self as rt, KeySpec, RadixIter};
use crate::store::Store;

/// A coordinate axis: `x` at offset 4, `y` at offset 8 in a point record.
const X_OFF: u32 = 4;
const Y_OFF: u32 = 8;

/// Spread the 32 bits of `v` into the even bit positions of a `u64`, so the odd
/// positions are free for the other axis to interleave into.
fn spread(v: u32) -> u64 {
    let mut x = u64::from(v);
    x = (x | (x << 16)) & 0x0000_ffff_0000_ffff;
    x = (x | (x << 8)) & 0x00ff_00ff_00ff_00ff;
    x = (x | (x << 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    x = (x | (x << 2)) & 0x3333_3333_3333_3333;
    x = (x | (x << 1)) & 0x5555_5555_5555_5555;
    x
}

/// The Morton (Z-order) code of `(x, y)`: `x` in the even bits, `y` in the odd ones.
#[must_use]
pub fn morton(x: u32, y: u32) -> u64 {
    spread(x) | (spread(y) << 1)
}

fn coord(store: &Store, rec: u32, off: u32) -> u32 {
    store.get_u32_raw(rec, off)
}

fn point(store: &Store, rec: u32) -> (u32, u32) {
    (coord(store, rec, X_OFF), coord(store, rec, Y_OFF))
}

fn code_of(store: &Store, rec: u32) -> u64 {
    let (x, y) = point(store, rec);
    morton(x, y)
}

/// Squared Euclidean distance — integer, so no floats and no rounding.  `i64` holds
/// the square of any `u32`-coordinate difference.
fn dist2(a: (u32, u32), b: (u32, u32)) -> i64 {
    let dx = i64::from(a.0) - i64::from(b.0);
    let dy = i64::from(a.1) - i64::from(b.1);
    dx * dx + dy * dy
}

fn morton_word(store: &Store, rec: u32, word: u32) -> u64 {
    if word == 0 { code_of(store, rec) } else { 0 }
}

fn morton_bits(_store: &Store, _rec: u32) -> u32 {
    64
}

/// How the radix tree reads a point's key: the whole 64-bit Morton code as one word.
pub const MORTON2D: KeySpec = KeySpec {
    word: morton_word,
    bits: morton_bits,
};

fn code_probe(code: u64) -> impl Fn(u32) -> u64 {
    move |word| if word == 0 { code } else { 0 }
}

/// Scatter the low `bits` of `v` across a `u64` at stride `stride`: bit `i` of `v`
/// lands at position `i * stride`, leaving the `stride − 1` positions between free
/// for the other axes.  The bit-at-a-time form here is the runtime counterpart of the
/// closed-form [`spread`]; for `stride == 2` the two agree exactly.
fn scatter(v: u32, stride: u32, bits: u32) -> u64 {
    (0..bits).fold(0u64, |out, i| {
        out | (u64::from((v >> i) & 1) << (i * stride))
    })
}

/// A Morton key assembled at run time from a record's coordinate fields, rather than
/// from a compile-time [`KeySpec`].
///
/// This is the shape `spacial<T[…]>` needs: the interleaved axes are not known when
/// the code is written, they are the coordinate key fields a schema discovers, so the
/// oracle must *carry* their byte offsets.  Each axis is a `u32` at the listed offset;
/// they interleave LSB-aligned, the first axis into bit 0, the next into bit 1, and so
/// on — so `MortonKey` over `[x, y]` produces exactly [`morton`]`(x, y)`, which `s5`
/// checks against [`MORTON2D`].
///
/// Two axes give 32 bits each (a 64-bit code); three give 21 each (63 bits, one word).
pub struct MortonKey {
    offsets: Vec<u32>,
}

impl MortonKey {
    /// Key on the `u32` coordinates at these byte offsets, in interleave order.
    #[must_use]
    pub fn new(offsets: Vec<u32>) -> MortonKey {
        assert!(
            (1..=3).contains(&offsets.len()),
            "a Morton key interleaves 1–3 axes"
        );
        MortonKey { offsets }
    }

    fn axis_bits(&self) -> u32 {
        64 / self.offsets.len() as u32
    }
}

impl crate::radix_tree::KeyOracle for MortonKey {
    fn word(&self, store: &Store, rec: u32, word: u32) -> u64 {
        if word != 0 {
            return 0;
        }
        let stride = self.offsets.len() as u32;
        let bits = self.axis_bits();
        self.offsets
            .iter()
            .enumerate()
            .fold(0u64, |code, (axis, &off)| {
                code | (scatter(coord(store, rec, off), stride, bits) << axis as u32)
            })
    }

    fn bits(&self, _store: &Store, _rec: u32) -> u32 {
        self.axis_bits() * self.offsets.len() as u32
    }
}

// ---------------------------------------------------------------------------
// Building and moving points
// ---------------------------------------------------------------------------

/// An empty index; returns its tree record id (see [`rt::rtree_init`]).
pub fn grid_init(store: &mut Store) -> u32 {
    rt::rtree_init(store, 0)
}

/// Add a point at `(x, y)`; returns the (possibly relocated) tree id and the new
/// record.
pub fn insert(store: &mut Store, tree: u32, x: u32, y: u32) -> (u32, u32) {
    let rec = store.claim(2);
    store.set_u32_raw(rec, X_OFF, x);
    store.set_u32_raw(rec, Y_OFF, y);
    let tree = rt::rtree_insert(store, tree, rec, MORTON2D);
    (tree, rec)
}

/// Move a point to `(x, y)`.  This is the per-frame operation: the coordinates
/// change, so the Morton key changes, so the index is updated — cheaply, because a
/// small move keeps a long key prefix (see [`rt::rtree_move`]).
pub fn move_to(store: &mut Store, tree: u32, rec: u32, x: u32, y: u32) -> u32 {
    rt::rtree_move(store, tree, rec, MORTON2D, move |s| {
        s.set_u32_raw(rec, X_OFF, x);
        s.set_u32_raw(rec, Y_OFF, y);
    })
}

/// Remove a point; `false` if it was not indexed.
pub fn remove(store: &mut Store, tree: u32, rec: u32) -> bool {
    rt::rtree_remove(store, tree, rec, MORTON2D)
}

// ---------------------------------------------------------------------------
// Exact queries
// ---------------------------------------------------------------------------

/// Call `visit` for every record whose Morton code lies in `[lo_code, hi_code]`.
///
/// By monotonicity that is every record in the box `lo..=hi`, plus records outside
/// it that Z-order threads through the same code range — so a caller must still test
/// each one geometrically.  The scan seeds at `lo_code` (a probe carries id `0`, so
/// it lands at the first record `>= lo_code`) and stops the moment a code passes
/// `hi_code`.
fn scan_codes(store: &Store, tree: u32, lo_code: u64, hi_code: u64, mut visit: impl FnMut(u32)) {
    let probe = code_probe(lo_code);
    let mut it = rt::rtree_seek(store, tree, &probe, 64, MORTON2D);
    let mut r = it.rec();
    while r != 0 {
        if code_of(store, r) > hi_code {
            break;
        }
        visit(r);
        r = it.next(store, tree).unwrap_or(0);
    }
}

/// The corners of the axis-aligned box that just contains the disc of radius `r`
/// about `(cx, cy)`, clamped to the coordinate range.  Clamping only enlarges the
/// box, so it still contains the disc.
fn disc_box(cx: u32, cy: u32, r: u32) -> ((u32, u32), (u32, u32)) {
    (
        (cx.saturating_sub(r), cy.saturating_sub(r)),
        (cx.saturating_add(r), cy.saturating_add(r)),
    )
}

/// Every point within Euclidean distance `radius` of `(cx, cy)`, sorted by record id.
///
/// **Exact.**  The disc lies inside its bounding box (monotonicity), the box lies
/// inside the scanned code interval, so the scan sees every point of the disc; the
/// `dist2 <= radius²` test then keeps exactly those inside it.  No false negatives.
#[must_use]
pub fn within(store: &Store, tree: u32, cx: u32, cy: u32, radius: u32) -> Vec<u32> {
    let (lo, hi) = disc_box(cx, cy, radius);
    let r2 = i64::from(radius) * i64::from(radius);
    let mut out = Vec::new();
    scan_codes(store, tree, morton(lo.0, lo.1), morton(hi.0, hi.1), |rec| {
        if dist2(point(store, rec), (cx, cy)) <= r2 {
            out.push(rec);
        }
    });
    out.sort_unstable();
    out
}

/// The `k` points closest to `(cx, cy)`, nearest first; fewer than `k` only if the
/// index holds fewer.  Ties in distance break by record id, so the result is total.
///
/// **Exact**, by an expanding box.  Scan the box of half-width `r`; if it holds at
/// least `k` points and the `k`-th nearest is within `r`, stop — because any point
/// *outside* a box of half-width `r` has `max(|dx|, |dy|) > r`, hence distance `> r`,
/// hence is farther than that `k`-th point and cannot belong in the answer.  Until
/// the guard holds, double `r`.  If the box grows to cover the whole coordinate range
/// without reaching `k`, the index simply holds fewer than `k`.
#[must_use]
pub fn nearest(store: &Store, tree: u32, cx: u32, cy: u32, k: usize) -> Vec<u32> {
    if k == 0 {
        return Vec::new();
    }
    let mut r: u32 = 8;
    let mut prev_box = None;
    loop {
        let (lo, hi) = disc_box(cx, cy, r);
        let mut found: Vec<(i64, u32)> = Vec::new();
        scan_codes(store, tree, morton(lo.0, lo.1), morton(hi.0, hi.1), |rec| {
            found.push((dist2(point(store, rec), (cx, cy)), rec));
        });
        found.sort_unstable();

        let saturated = prev_box == Some((lo, hi));
        let kth_within = found.len() >= k && found[k - 1].0 <= i64::from(r) * i64::from(r);
        if kth_within || saturated {
            found.truncate(k);
            return found.into_iter().map(|(_, rec)| rec).collect();
        }
        prev_box = Some((lo, hi));
        r = r.saturating_mul(2);
    }
}

// ---------------------------------------------------------------------------
// Approximate walk
// ---------------------------------------------------------------------------

/// An outward walk from a query point in roughly-increasing distance.
///
/// **Approximate.**  It yields records in increasing *Morton* distance from the query
/// code — cheap (two cursors, no allocation, no distance sort) and usually close to
/// spatial order, but Z-order jumps at quadrant boundaries, so a truly-near point can
/// arrive late.  Fine for aggro / interest management; use [`within`] or [`nearest`]
/// when the answer must be exact.
///
/// It yields every record eventually, each once, in non-decreasing `|code − query|`.
pub struct Near {
    query: u64,
    back: RadixIter,
    fore: RadixIter,
}

/// Begin an approximate outward walk from `(cx, cy)`.
#[must_use]
pub fn near(store: &Store, tree: u32, cx: u32, cy: u32) -> Near {
    let query = morton(cx, cy);
    let probe = code_probe(query);
    let fore = rt::rtree_seek(store, tree, &probe, 64, MORTON2D);
    // `fore` is the first record with code >= query; `back` is the one just below it.
    // When the query sits past every record, `fore` is empty and the last record is
    // the nearest, so seed `back` from the tail.
    let back = if fore.rec() == 0 {
        rt::rtree_last(store, tree)
    } else {
        let mut b = fore;
        b.prev(store, tree);
        b
    };
    Near { query, back, fore }
}

impl Near {
    /// The next record outward, or `None` once the whole index is walked.
    pub fn next(&mut self, store: &Store, tree: u32) -> Option<u32> {
        let gap = |rec: u32| code_of(store, rec).abs_diff(self.query);
        match (self.back.rec(), self.fore.rec()) {
            (0, 0) => None,
            (b, 0) => {
                self.back.prev(store, tree);
                Some(b)
            }
            (0, f) => {
                self.fore.next(store, tree);
                Some(f)
            }
            (b, f) => {
                if gap(b) <= gap(f) {
                    self.back.prev(store, tree);
                    Some(b)
                } else {
                    self.fore.next(store, tree);
                    Some(f)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn lcg(seed: &mut u64) -> u32 {
        *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (*seed >> 32) as u32
    }

    /// A field of random points, plus the list of records so tests can brute-force.
    fn field(store: &mut Store, count: usize, grid: u32, seed: &mut u64) -> (u32, Vec<u32>) {
        let mut tree = grid_init(store);
        let mut recs = Vec::new();
        for _ in 0..count {
            let (x, y) = (lcg(seed) % grid, lcg(seed) % grid);
            let (grown, rec) = insert(store, tree, x, y);
            tree = grown;
            recs.push(rec);
        }
        (tree, recs)
    }

    fn brute_within(store: &Store, recs: &[u32], cx: u32, cy: u32, radius: u32) -> Vec<u32> {
        let r2 = i64::from(radius) * i64::from(radius);
        let mut v: Vec<u32> = recs
            .iter()
            .copied()
            .filter(|&r| dist2(point(store, r), (cx, cy)) <= r2)
            .collect();
        v.sort_unstable();
        v
    }

    fn brute_nearest(store: &Store, recs: &[u32], cx: u32, cy: u32, k: usize) -> Vec<u32> {
        let mut v: Vec<(i64, u32)> = recs
            .iter()
            .map(|&r| (dist2(point(store, r), (cx, cy)), r))
            .collect();
        v.sort_unstable();
        v.truncate(k);
        v.into_iter().map(|(_, r)| r).collect()
    }

    /// The Morton code must increase with each axis on its own — the property both
    /// exact queries rest on.
    #[test]
    fn s0_morton_is_monotone_per_axis() {
        let mut seed = 0x1111_2222_3333_4444;
        for _ in 0..2000 {
            let (x, y, d) = (
                lcg(&mut seed) % 1000,
                lcg(&mut seed) % 1000,
                lcg(&mut seed) % 50,
            );
            assert!(
                morton(x, y) <= morton(x + d, y),
                "x must not lower the code"
            );
            assert!(
                morton(x, y) <= morton(x, y + d),
                "y must not lower the code"
            );
        }
    }

    /// S1 — `within` equals brute force over many random discs.
    #[test]
    fn s1_within_matches_brute_force() {
        let mut store = Store::new_in_use(1 << 15);
        let mut seed = 0xabcd_0000_1234_5678;
        let (tree, recs) = field(&mut store, 600, 500, &mut seed);
        for _ in 0..300 {
            let (cx, cy, radius) = (
                lcg(&mut seed) % 500,
                lcg(&mut seed) % 500,
                1 + lcg(&mut seed) % 60,
            );
            assert_eq!(
                within(&store, tree, cx, cy, radius),
                brute_within(&store, &recs, cx, cy, radius),
                "within({cx},{cy},{radius})"
            );
        }
    }

    /// S2 — `nearest` equals brute force, across `k` from 1 to more than the field.
    #[test]
    fn s2_nearest_matches_brute_force() {
        let mut store = Store::new_in_use(1 << 14);
        let mut seed = 0x9999_8888_7777_6666;
        let (tree, recs) = field(&mut store, 250, 400, &mut seed);
        for _ in 0..300 {
            let (cx, cy) = (lcg(&mut seed) % 400, lcg(&mut seed) % 400);
            for &k in &[1usize, 3, 8, 25, 250, 400] {
                assert_eq!(
                    nearest(&store, tree, cx, cy, k),
                    brute_nearest(&store, &recs, cx, cy, k),
                    "nearest({cx},{cy},{k})"
                );
            }
        }
    }

    /// S2b — ties in distance are broken by record id, so `nearest` is deterministic
    /// even when several points sit the same distance away.
    #[test]
    fn s2b_nearest_breaks_ties_by_record_id() {
        let mut store = Store::new_in_use(1 << 12);
        let mut tree = grid_init(&mut store);
        // Four points equidistant from (10, 10).
        let mut recs = Vec::new();
        for (x, y) in [(10, 5), (10, 15), (5, 10), (15, 10)] {
            let (t, r) = insert(&mut store, tree, x, y);
            tree = t;
            recs.push(r);
        }
        let got = nearest(&store, tree, 10, 10, 2);
        let mut want = recs.clone();
        want.sort_unstable();
        want.truncate(2);
        assert_eq!(got, want, "ties resolve to the lowest record ids");
    }

    /// S3 — `near` yields every record once, in non-decreasing Morton distance.  This
    /// is the structural guarantee; it is *not* Euclidean order (see S3b).
    #[test]
    fn s3_near_is_a_total_walk_in_morton_order() {
        let mut store = Store::new_in_use(1 << 14);
        let mut seed = 0x2468_1357_9ace_bdf0;
        let (tree, recs) = field(&mut store, 200, 300, &mut seed);
        let (cx, cy) = (150, 150);
        let query = morton(cx, cy);

        let mut walk = near(&store, tree, cx, cy);
        let mut yielded = Vec::new();
        let mut last_gap = 0u64;
        while let Some(rec) = walk.next(&store, tree) {
            let gap = code_of(&store, rec).abs_diff(query);
            assert!(gap >= last_gap, "Morton distance must not decrease");
            last_gap = gap;
            yielded.push(rec);
        }
        let unique: HashSet<u32> = yielded.iter().copied().collect();
        assert_eq!(yielded.len(), recs.len(), "every record is yielded");
        assert_eq!(unique.len(), recs.len(), "each exactly once");
    }

    /// S3b — `near` is only approximate: over many queries its first few results miss
    /// a true nearest often enough to prove it cannot stand in for `nearest`.  If this
    /// ever stops failing, `near` was quietly turned exact and the docs are now wrong.
    #[test]
    fn s3b_near_is_not_euclidean_order() {
        let mut store = Store::new_in_use(1 << 15);
        let mut seed = 0x0f0f_f0f0_0f0f_f0f0;
        let (tree, recs) = field(&mut store, 500, 256, &mut seed);
        let mut disagreements = 0;
        for _ in 0..200 {
            let (cx, cy) = (lcg(&mut seed) % 256, lcg(&mut seed) % 256);
            let truth = brute_nearest(&store, &recs, cx, cy, 1)[0];
            let mut walk = near(&store, tree, cx, cy);
            let first = walk.next(&store, tree).unwrap();
            if first != truth {
                disagreements += 1;
            }
        }
        assert!(
            disagreements > 0,
            "near's first result should sometimes miss the true nearest — it is approximate"
        );
    }

    #[allow(clippy::cast_precision_loss)] // counts stay well under 2^52
    fn per_op(d: std::time::Duration, n: usize) -> f64 {
        d.as_secs_f64() * 1e9 / n as f64
    }

    /// S5 — the runtime key is the compile-time key.
    ///
    /// The bridge that unblocks the database: a `spacial<T[…]>` will build a
    /// [`MortonKey`] from schema-discovered coordinate offsets, not from a fixed
    /// [`KeySpec`].  This inserts the same points into two trees — one keyed by the
    /// `fn`-pointer `MORTON2D`, one by a `MortonKey` over the same two offsets — and
    /// requires byte-identical behaviour: same walk order, same `get`, same length.
    #[test]
    fn s5_runtime_morton_key_matches_the_compile_time_one() {
        use crate::radix_tree::{self as rt, KeyOracle};
        let key = MortonKey::new(vec![X_OFF, Y_OFF]);

        let mut store = Store::new_in_use(1 << 15);
        let mut fixed = rt::rtree_init(&mut store, 0);
        let mut runtime = rt::rtree_init(&mut store, 0);
        let mut seed = 0x5151_7272_9393_b4b4;
        let mut recs = Vec::new();

        for _ in 0..500 {
            let (x, y) = (lcg(&mut seed) % 1000, lcg(&mut seed) % 1000);
            let rec = store.claim(2);
            store.set_u32_raw(rec, X_OFF, x);
            store.set_u32_raw(rec, Y_OFF, y);
            recs.push(rec);
            fixed = rt::rtree_insert(&mut store, fixed, rec, MORTON2D);
            runtime = rt::rtree_insert(&mut store, runtime, rec, &key);
            // The two must agree at every step, not merely at the end.
            assert_eq!(rt::rtree_len(&store, fixed), rt::rtree_len(&store, runtime));
        }

        // Same in-order walk.
        let (mut a, mut b) = (
            rt::rtree_first(&store, fixed),
            rt::rtree_first(&store, runtime),
        );
        loop {
            assert_eq!(a.rec(), b.rec(), "the two keys must walk identically");
            if a.rec() == 0 {
                break;
            }
            a.next(&store, fixed);
            b.next(&store, runtime);
        }

        // Same code for every record, and the same exact lookup.
        for &rec in &recs {
            let (x, y) = point(&store, rec);
            assert_eq!(morton(x, y), key.word(&store, rec, 0), "codes must match");
            let probe = code_probe(morton(x, y));
            assert_eq!(
                rt::rtree_get(&store, runtime, &probe, 64, &key),
                rt::rtree_get(&store, fixed, &probe, 64, MORTON2D),
                "get must agree"
            );
        }
    }

    /// BENCH — the crossover.  A spatial index is not free; at a small field the
    /// brute-force scan is all cache-friendly distance checks and wins.  This sweeps
    /// the field size so the point where the index overtakes brute force is visible,
    /// not asserted.
    ///
    /// Run: `cargo test --release --lib spatial::tests::bench -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark — run with --release --ignored --nocapture"]
    fn bench_queries() {
        use std::time::Instant;
        const GRID: u32 = 2048;
        const QUERIES: usize = 4000;
        const ROUNDS: usize = 3;
        const RADIUS: u32 = 24;
        const KK: usize = 8;

        println!(
            "\nspatial queries — {GRID}×{GRID} field, best of {ROUNDS}, ns/query (index vs brute)"
        );
        println!(
            "  {:>7} | {:>18} | {:>18} | {:>10}",
            "points", "within(r=24)", "nearest(k=8)", "near(k=8)"
        );

        for &count in &[200usize, 1_000, 5_000, 20_000] {
            let mut store = Store::new_in_use(1 << 18);
            let mut seed = 0x1234_abcd_5678_ef01;
            let (tree, recs) = field(&mut store, count, GRID, &mut seed);
            let queries: Vec<(u32, u32)> = (0..QUERIES)
                .map(|_| (lcg(&mut seed) % GRID, lcg(&mut seed) % GRID))
                .collect();

            let mut best = [f64::MAX; 5];
            for _ in 0..ROUNDS {
                let clock = Instant::now();
                let mut acc = 0usize;
                for &(cx, cy) in &queries {
                    acc += within(&store, tree, cx, cy, RADIUS).len();
                }
                best[0] = best[0].min(per_op(clock.elapsed(), QUERIES));
                assert!(acc < usize::MAX);

                let clock = Instant::now();
                for &(cx, cy) in &queries {
                    let _ = brute_within(&store, &recs, cx, cy, RADIUS);
                }
                best[1] = best[1].min(per_op(clock.elapsed(), QUERIES));

                let clock = Instant::now();
                for &(cx, cy) in &queries {
                    let _ = nearest(&store, tree, cx, cy, KK);
                }
                best[2] = best[2].min(per_op(clock.elapsed(), QUERIES));

                let clock = Instant::now();
                for &(cx, cy) in &queries {
                    let _ = brute_nearest(&store, &recs, cx, cy, KK);
                }
                best[3] = best[3].min(per_op(clock.elapsed(), QUERIES));

                let clock = Instant::now();
                let mut acc = 0u64;
                for &(cx, cy) in &queries {
                    let mut walk = near(&store, tree, cx, cy);
                    for _ in 0..KK {
                        acc += u64::from(walk.next(&store, tree).unwrap_or(0));
                    }
                }
                best[4] = best[4].min(per_op(clock.elapsed(), QUERIES));
                assert!(acc < u64::MAX);
            }
            println!(
                "  {count:>7} | {:>7.0} vs{:>7.0} | {:>7.0} vs{:>7.0} | {:>10.0}",
                best[0], best[1], best[2], best[3], best[4]
            );
        }
        println!("  (within/nearest show index-vs-brute; near is approximate, no exact baseline)");
    }

    /// S4 — a moved point is found at its new home and gone from its old one: the
    /// query reflects the live position after a move.
    #[test]
    fn s4_moving_a_point_updates_what_queries_see() {
        let mut store = Store::new_in_use(1 << 13);
        let mut tree = grid_init(&mut store);
        let (t, mover) = insert(&mut store, tree, 10, 10);
        tree = t;
        // Neighbours near the destination, so the query is non-trivial there.
        for (x, y) in [(200, 200), (203, 198), (198, 202)] {
            let (t, _) = insert(&mut store, tree, x, y);
            tree = t;
        }
        assert!(within(&store, tree, 10, 10, 5).contains(&mover));
        assert!(!within(&store, tree, 200, 200, 10).contains(&mover));

        tree = move_to(&mut store, tree, mover, 201, 201);

        assert!(
            !within(&store, tree, 10, 10, 5).contains(&mover),
            "gone from the old cell"
        );
        assert!(
            within(&store, tree, 200, 200, 10).contains(&mover),
            "present at the new one"
        );
        assert_eq!(
            nearest(&store, tree, 201, 201, 1),
            vec![mover],
            "now the nearest there"
        );
    }
}
