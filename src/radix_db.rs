// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! The bridge between the database's keyed collections and [`crate::radix_tree`]
//! — the counterpart of [`crate::hash`] for the `Radix` kind (`spatial<T[…]>`, and
//! later `radix<T[k]>`).  @PLN48 S2.
//!
//! The tree keys on an abstract bit-string; this module supplies the one the schema
//! implies.  Each key field becomes an **axis**, read from the element record and
//! encoded so that the tree's key order matches the order the existing keyed
//! collections use (`compare_ref`).  For one axis that is a plain ordered radix; for
//! two or three integer axes the axes interleave into a Morton (Z-order) code, so
//! spatially near records share a long key prefix.
//!
//! **Order preservation is the invariant.**  A signed integer's 2's-complement bits
//! are *not* ordered (a negative has its top bit set, so it would sort after every
//! positive).  Each axis is therefore mapped to **offset-binary** — flip the sign
//! bit — which makes an unsigned bit-compare agree with the signed value compare, so
//! the radix walk visits records in the same order `sorted`/`index` would.
//!
//! Scope of this step: **integer** key fields (the spatial games case).  Every axis
//! is read as an `i64` and given 64 bits — correct for any integer width, if wider
//! than a narrow field needs.  Text keys (the `radix<T[text]>` prefix path) and
//! per-width packing are follow-ups; until the parser gate lifts, only these tests
//! reach this code.

#![allow(dead_code)]

use crate::keys::{self, Content, DbRef, Key};
use crate::radix_tree::{self as rt, KeyOracle};
use crate::store::Store;

/// Element payload base: an element record holds its back-pointer at offset 4 and its
/// struct payload from offset 8 (the shared keyed-collection convention).
const PAYLOAD: u32 = 8;
/// Bits given to each axis.  Uniform so the interleave is a plain Morton code.
const AXIS_BITS: u32 = 64;
/// The most axes a spatial key interleaves (`spatial<T[x,y,z]>`).  The parser
/// rejects a `spatial<T[…]>` with more key fields than this (else the Morton
/// interleave indexes past the `[u64; MAX_AXES]` code array — a runtime panic).
pub const MAX_AXES: usize = 3;

/// A key field's value, mapped to an order-preserving 64-bit code (offset-binary).
/// A key field's signed coordinate value (for distance arithmetic).
fn axis_i64(store: &Store, rec: u32, key: &Key) -> i64 {
    let p = PAYLOAD + u32::from(key.position);
    match key.type_nr.unsigned_abs() {
        2 => store.get_long(rec, p),
        8 => i64::from(store.get_i32_raw(rec, p)),
        9 => i64::from(store.get_short(rec, p, 0)),
        10 => i64::from(store.get_byte(rec, p, 0)),
        11 => {
            let raw: u16 = *store.addr(rec, p);
            i64::from(raw as i16)
        }
        // type_nr 1 (`integer`) and any other integer default.
        _ => store.get_int(rec, p),
    }
}

/// The order-preserving (offset-binary) code of a signed coordinate — the axis's
/// contribution to the Morton key.  Flipping the sign bit makes an unsigned compare
/// agree with the signed value compare.
fn coord_code(v: i64) -> u64 {
    (v as u64) ^ (1u64 << 63)
}

fn axis_code(store: &Store, rec: u32, key: &Key) -> u64 {
    coord_code(axis_i64(store, rec, key))
}

/// The same code from an already-extracted query value.  Must match [`axis_code`]:
/// `get_key` yields `Content::Long(i64)` for every integer width, so both sides
/// offset-binary the same `i64`.
fn probe_code(c: &Content) -> u64 {
    match c {
        Content::Long(v) => (*v as u64) ^ (1u64 << 63),
        // A non-integer probe cannot match an integer axis; encode the minimum.
        _ => 0,
    }
}

/// The `word`-th 64-bit chunk of the Z-order interleave of `n` axes.  Composite bit
/// `b` (most-significant first) is axis `b % n`'s bit `b / n` — so the axes' most
/// significant bits lead, which is what makes the interleave order-preserving.
fn interleave(word: u32, n: usize, code: impl Fn(usize) -> u64) -> u64 {
    let mut codes = [0u64; MAX_AXES];
    for (a, slot) in codes.iter_mut().enumerate().take(n) {
        *slot = code(a);
    }
    let total = AXIS_BITS * n as u32;
    let mut out = 0u64;
    for i in 0..64 {
        let b = word * 64 + i;
        if b >= total {
            break;
        }
        let axis = (b as usize) % n;
        let level = b / n as u32;
        let bit = (codes[axis] >> (AXIS_BITS - 1 - level)) & 1;
        out |= bit << (63 - i);
    }
    out
}

/// The text key's bytes, read through the field's string pointer.
///
/// `type_nr == 6` is a `text` key; the field holds a `u32` pointer to the string
/// record, which is the same indirection `compare_key` follows.
fn text_bytes<'a>(store: &'a Store, rec: u32, key: &Key) -> &'a str {
    let p = PAYLOAD + u32::from(key.position);
    store.get_str(store.get_u32_raw(rec, p))
}

/// Is this key spec a single `text` field — the bytewise case rather than the
/// coordinate one?
fn is_text_key(keys: &[Key]) -> bool {
    keys.len() == 1 && keys[0].type_nr.abs() == TEXT_TYPE_NR
}

/// `text`'s key type number, as `compare_key` spells it.
const TEXT_TYPE_NR: i8 = 6;

/// How the tree reads a TEXT record's key: its bytes, big-endian, 8 at a time.
///
/// Big-endian is what makes bytewise order and bit order the same, so the tree's
/// in-order traversal is lexicographic and a prefix descends to its extensions.
/// `bits()` is per-record — the variable-length half of [`KeyOracle`] that the
/// Morton oracle never exercises, and that `radix_tree`'s `TERM_BITS` suffix exists
/// to order (`"ab"` sorts before `"abc"`; proven by `r8c`).
struct TextOracle<'a> {
    key: &'a Key,
}

impl KeyOracle for TextOracle<'_> {
    fn bits(&self, store: &Store, rec: u32) -> u32 {
        text_bytes(store, rec, self.key).len() as u32 * 8
    }
    fn word(&self, store: &Store, rec: u32, word: u32) -> u64 {
        bytes_word(text_bytes(store, rec, self.key).as_bytes(), word)
    }
}

/// The `word`-th big-endian 8-byte chunk of `b`, zero-padded past the end.
/// Shared by the record oracle and the probe so both read a key the same way — the
/// two disagreeing is the classic radix fault, and it presents as "inserted, not
/// found".
fn bytes_word(b: &[u8], word: u32) -> u64 {
    let mut out = 0u64;
    for i in 0..8 {
        let idx = (word * 8 + i) as usize;
        out = (out << 8) | u64::from(b.get(idx).copied().unwrap_or(0));
    }
    out
}

/// The oracle a collection's key spec calls for.
///
/// One enum rather than a generic parameter so every operation below keeps a SINGLE
/// construction point: a kind selected in six places is how a per-kind dispatch
/// omission gets in (DATABASE.md warns about exactly this, and loft#799 is what one
/// looks like from the outside — a declaration accepted, then every lookup NULL).
enum DbOracle<'a> {
    Morton { keys: &'a [Key] },
    Text { key: &'a Key },
}

impl<'a> DbOracle<'a> {
    fn of(keys: &'a [Key]) -> DbOracle<'a> {
        if is_text_key(keys) {
            DbOracle::Text { key: &keys[0] }
        } else {
            DbOracle::Morton { keys }
        }
    }
}

impl KeyOracle for DbOracle<'_> {
    fn bits(&self, store: &Store, rec: u32) -> u32 {
        match self {
            DbOracle::Morton { keys } => AXIS_BITS * keys.len() as u32,
            DbOracle::Text { key } => TextOracle { key }.bits(store, rec),
        }
    }
    fn word(&self, store: &Store, rec: u32, word: u32) -> u64 {
        match self {
            DbOracle::Morton { keys } => {
                interleave(word, keys.len(), |a| axis_code(store, rec, &keys[a]))
            }
            DbOracle::Text { key } => TextOracle { key }.word(store, rec, word),
        }
    }
}

/// How the tree reads a record's key: the Morton interleave of its coordinate axes.
struct RadixOracle<'a> {
    keys: &'a [Key],
}

impl KeyOracle for RadixOracle<'_> {
    fn bits(&self, _store: &Store, _rec: u32) -> u32 {
        AXIS_BITS * self.keys.len() as u32
    }
    fn word(&self, store: &Store, rec: u32, word: u32) -> u64 {
        interleave(word, self.keys.len(), |a| {
            axis_code(store, rec, &self.keys[a])
        })
    }
}

fn key_bits(keys: &[Key]) -> u32 {
    AXIS_BITS * keys.len() as u32
}

/// Insert element record `rec` into the `Radix` collection at field `coll`.
///
/// The tree lives in `coll`'s store, its record id held in `coll`'s 4-byte field
/// (the `hash`-bucket convention).  A growing insert can relocate the tree, so the
/// (possibly new) id is written back.  Two records with the same coordinates both
/// stay — they differ in the id suffix and land adjacent — which is what lets many
/// entities share a bucket; no dedup here.
pub fn add(coll: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    let store = keys::mut_store(coll, stores);
    let mut tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        tree = rt::rtree_init(store, 0);
        store.set_u32_raw(coll.rec, coll.pos, tree);
    }
    let tree = rt::rtree_insert(store, tree, rec.rec, &DbOracle::of(keys));
    store.set_u32_raw(coll.rec, coll.pos, tree);
}

/// The record whose key equals `key`, or a null `DbRef` (`rec == 0`) when absent.
/// When several records share the key, the first in order.
#[must_use]
pub fn find(coll: &DbRef, stores: &[Store], keys: &[Key], key: &[Content]) -> DbRef {
    let store = keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    let rec = if tree == 0 {
        0
    } else {
        // The probe must read a key the same way the oracle does; the two
        // disagreeing presents as "inserted, then not found".
        if is_text_key(keys) {
            let q: &[u8] = match key.first() {
                Some(Content::Str(v)) => v.str().as_bytes(),
                // A non-text probe cannot match a text key.
                _ => &[],
            };
            let probe = |word: u32| bytes_word(q, word);
            rt::rtree_get(store, tree, &probe, q.len() as u32 * 8, &DbOracle::of(keys))
        } else {
            let probe = |word: u32| interleave(word, key.len(), |a| probe_code(&key[a]));
            rt::rtree_get(store, tree, &probe, key_bits(keys), &DbOracle::of(keys))
        }
    };
    DbRef {
        store_nr: coll.store_nr,
        rec,
        pos: PAYLOAD,
    }
}

/// Unlink element record `rec` from the collection; the caller frees `rec` itself.
/// `false` if it was not present (the tree removes only the record whose key AND id
/// match, so a same-bucket sibling is never removed by mistake).
pub fn remove(coll: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) -> bool {
    let store = keys::mut_store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        return false;
    }
    rt::rtree_remove(store, tree, rec.rec, &DbOracle::of(keys))
}

/// Number of element records in the collection.  Reads the tree's cached length
/// word (O(1)); key-free, mirroring `hash::count`.
#[must_use]
pub fn count(coll: &DbRef, stores: &[Store]) -> u32 {
    let store = keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        0
    } else {
        rt::rtree_len(store, tree)
    }
}

/// Every element record in the collection, in key order.  Key-free (a plain tree
/// walk), so teardown and iteration reach it without building an oracle.
#[must_use]
pub fn records(coll: &DbRef, stores: &[Store]) -> Vec<u32> {
    let store = keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    let mut out = Vec::new();
    if tree != 0 {
        let mut it = rt::rtree_first(store, tree);
        let mut r = it.rec();
        while r != 0 {
            out.push(r);
            r = it.next(store, tree).unwrap_or(0);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Proximity queries — @PLN48 S3, the Rust ports of `src/spatial.rs`, reading
// coordinates through the schema's key fields instead of fixed offsets.
// ---------------------------------------------------------------------------

/// The full multi-word Morton code of a point given each axis's order-preserving
/// code.  `n` axes × 64 bits interleave into `n` words (word 0 most significant).
fn morton_words(n: usize, code: impl Fn(usize) -> u64) -> [u64; MAX_AXES] {
    let mut words = [0u64; MAX_AXES];
    for (w, slot) in words.iter_mut().enumerate().take(n) {
        *slot = interleave(w as u32, n, &code);
    }
    words
}

/// Lexicographic compare of two `n`-word Morton codes (word 0 most significant).
fn code_gt(a: &[u64; MAX_AXES], b: &[u64; MAX_AXES], n: usize) -> bool {
    for i in 0..n {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    false
}

/// The records whose Morton code lies in `[from, till]` (or `[from, ∞)` when `till`
/// is `None`), in natural Morton order, capped at `limit` records (`None` = all).
///
/// This is the primitive behind `spatial` range slicing: `xs[(x,y)..]`,
/// `xs[(x,y)..:n]`, and the bounding box `xs[(x1,y1)..(x2,y2)]`.  It is the raw code
/// interval — for a bounding box that is a *superset* of the geometric box (Z-order
/// threads through codes outside it), exactly as a keyed range slice is the raw key
/// range; the caller filters or `break`s as needed.
#[must_use]
pub fn range(
    coll: &DbRef,
    stores: &[Store],
    keys: &[Key],
    from: &[i64],
    till: Option<&[i64]>,
    limit: Option<usize>,
) -> Vec<u32> {
    let store = keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        return Vec::new();
    }
    let n = keys.len();
    let from_lo = |a: usize| coord_code(from[a]);
    let till_code = till.map(|t| morton_words(n, |a| coord_code(t[a])));
    let oracle = RadixOracle { keys };
    let probe = |word: u32| interleave(word, n, from_lo);
    let mut it = rt::rtree_seek(store, tree, &probe, key_bits(keys), &oracle);
    let cap = limit.unwrap_or(usize::MAX);
    let mut out = Vec::new();
    let mut rec = it.rec();
    while rec != 0 && out.len() < cap {
        if let Some(hi) = &till_code {
            let rec_code = morton_words(n, |a| axis_code(store, rec, &keys[a]));
            if code_gt(&rec_code, hi, n) {
                break;
            }
        }
        out.push(rec);
        rec = it.next(store, tree).unwrap_or(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: &mut u64) -> i64 {
        *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (*seed >> 32) as i64
    }

    // ---- text keys: the bytewise oracle through the DB record layout ------

    /// One `text` axis at payload offset 0.
    fn w_key() -> Vec<Key> {
        vec![Key {
            type_nr: TEXT_TYPE_NR,
            position: 0,
        }]
    }

    /// An element record whose text field holds `w` — the same shape `compare_key`
    /// reads: a `u32` pointer at `PAYLOAD + position` to the string record.
    fn add_word(store: &mut Store, w: &str) -> u32 {
        let ptr = store.set_str(w);
        let rec = store.claim(4);
        store.set_u32_raw(rec, PAYLOAD, ptr);
        rec
    }

    /// The oracle selects on the KEY TYPE, and a text key must reach the bytewise
    /// side.  Asserted first, because everything below is meaningless if the
    /// selector silently fell through to Morton — which is exactly what a
    /// coordinate-shaped answer to a text key looks like (loft#799).
    #[test]
    fn a_text_key_selects_the_bytewise_oracle() {
        assert!(
            is_text_key(&w_key()),
            "a lone text field is the bytewise case"
        );
        assert!(!is_text_key(&xy_keys()), "two integer axes stay Morton");
        assert!(
            matches!(DbOracle::of(&w_key()), DbOracle::Text { .. }),
            "a text key must select the text oracle"
        );
        assert!(
            matches!(DbOracle::of(&xy_keys()), DbOracle::Morton { .. }),
            "coordinate keys must keep the Morton oracle"
        );
    }

    /// In-order traversal is lexicographic, and a prefix sorts before what extends
    /// it — through the DB record layout, not the test fixture in `radix_tree`.
    #[test]
    fn text_keys_order_lexicographically_through_the_db_layout() {
        let mut store = Store::new_in_use(1024);
        let keys = w_key();
        let mut tree = rt::rtree_init(&mut store, 8);
        // Neither sorted nor reverse-sorted, so a pass cannot come from insert order.
        let words = [
            "kerkstraat",
            "kerk",
            "lonneker",
            "kerf",
            "kerkweg",
            "kerklaan",
        ];
        let mut recs = Vec::new();
        for w in words {
            let rec = add_word(&mut store, w);
            recs.push((w, rec));
            tree = rt::rtree_insert(&mut store, tree, rec, &DbOracle::of(&keys));
        }
        rt::rtree_validate(&store, tree, &DbOracle::of(&keys));

        let mut it = rt::rtree_first(&store, tree);
        let mut seen = Vec::new();
        let mut r = it.rec();
        while r != 0 {
            seen.push(
                recs.iter()
                    .find(|(_, rec)| *rec == r)
                    .expect("record came from this tree")
                    .0,
            );
            r = it.next(&store, tree).unwrap_or(0);
        }
        assert_eq!(
            seen,
            [
                "kerf",
                "kerk",
                "kerklaan",
                "kerkstraat",
                "kerkweg",
                "lonneker"
            ],
            "bytewise order, with `kerk` before the three keys that extend it"
        );
    }

    /// A key just inserted must be found, and one that was not must not be — the
    /// two halves loft#799 got wrong for a text-keyed `spatial`.
    #[test]
    fn text_keys_answer_exact_lookups_through_the_db_layout() {
        let mut store = Store::new_in_use(1024);
        let keys = w_key();
        let mut tree = rt::rtree_init(&mut store, 8);
        let words = [
            "kerkstraat",
            "kerk",
            "lonneker",
            "kerf",
            "kerkweg",
            "kerklaan",
        ];
        let mut recs = Vec::new();
        for w in words {
            let rec = add_word(&mut store, w);
            recs.push((w, rec));
            tree = rt::rtree_insert(&mut store, tree, rec, &DbOracle::of(&keys));
        }
        for (w, want) in &recs {
            let q = w.as_bytes();
            let probe = |word: u32| bytes_word(q, word);
            let got = rt::rtree_get(
                &store,
                tree,
                &probe,
                q.len() as u32 * 8,
                &DbOracle::of(&keys),
            );
            assert_eq!(got, *want, "exact lookup of {w:?}");
        }
        // `kerks` extends `kerk` AND is a prefix of `kerkstraat` — the shape a wrong
        // terminator or a probe/oracle mismatch answers for.
        for absent in ["kerks", "ker", "kerkstraatx", "aaa", "zzz"] {
            let q = absent.as_bytes();
            let probe = |word: u32| bytes_word(q, word);
            let got = rt::rtree_get(
                &store,
                tree,
                &probe,
                q.len() as u32 * 8,
                &DbOracle::of(&keys),
            );
            assert_eq!(got, 0, "{absent:?} is absent and must not be found");
        }
    }

    /// Two `integer` axes at payload offsets 0 and 8.
    fn xy_keys() -> Vec<Key> {
        vec![
            Key {
                type_nr: 1,
                position: 0,
            },
            Key {
                type_nr: 1,
                position: 8,
            },
        ]
    }

    /// Claim an element record and write its coordinates (payload at offset 8).
    fn add_point(store: &mut Store, coll: &DbRef, keys: &[Key], x: i64, y: i64) -> u32 {
        let rec = store.claim(3); // 24 bytes: header + 16-byte payload from offset 8
        store.set_int(rec, PAYLOAD, x);
        store.set_int(rec, PAYLOAD + 8, y);
        add(
            coll,
            &DbRef {
                store_nr: 0,
                rec,
                pos: PAYLOAD,
            },
            std::slice::from_mut(store),
            keys,
        );
        rec
    }

    /// D1 — the bridge round-trips: every inserted point is found by its key, the
    /// walk enumerates exactly the inserted records, and the order-preserving code
    /// is what makes `find` land on the right one.
    #[test]
    fn d1_insert_and_find_round_trip() {
        let mut store = Store::new_in_use(1 << 15);
        let keys = xy_keys();
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };

        let mut seed = 0x2024_1111_2222_3333;
        let mut pts = Vec::new();
        for _ in 0..400 {
            // Signed coordinates, including negatives, to exercise offset-binary.
            let (x, y) = (lcg(&mut seed) % 2000 - 1000, lcg(&mut seed) % 2000 - 1000);
            // Keep coordinates distinct so `find` has one answer.
            if pts.iter().any(|&(px, py, _)| px == x && py == y) {
                continue;
            }
            let rec = add_point(&mut store, &coll, &keys, x, y);
            pts.push((x, y, rec));
        }

        let stores = std::slice::from_ref(&store);
        for &(x, y, rec) in &pts {
            let found = find(&coll, stores, &keys, &[Content::Long(x), Content::Long(y)]);
            assert_eq!(found.rec, rec, "find({x},{y}) must reach its record");
            assert_eq!(found.pos, PAYLOAD);
        }

        // A coordinate never inserted is absent.
        let absent = find(
            &coll,
            stores,
            &keys,
            &[Content::Long(999_999), Content::Long(1)],
        );
        assert_eq!(absent.rec, 0, "an absent key returns null");

        let walked = records(&coll, stores);
        assert_eq!(
            walked.len(),
            pts.len(),
            "the walk enumerates every record once"
        );
        let unique: std::collections::HashSet<u32> = walked.iter().copied().collect();
        assert_eq!(unique.len(), pts.len(), "no record appears twice");
    }

    /// D2 — several records at the *same* bucket all stay (no dedup), adjacent in the
    /// walk: the per-bucket bucket @PLN48 needs.
    #[test]
    fn d2_same_cell_keeps_every_record() {
        let mut store = Store::new_in_use(1 << 13);
        let keys = xy_keys();
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };

        // Three entities at (5, 7), plus neighbours either side.
        add_point(&mut store, &coll, &keys, 1, 1);
        let bucket: Vec<u32> = (0..3)
            .map(|_| add_point(&mut store, &coll, &keys, 5, 7))
            .collect();
        add_point(&mut store, &coll, &keys, 9, 9);

        let stores = std::slice::from_ref(&store);
        assert_eq!(records(&coll, stores).len(), 5, "no record was dropped");

        // The bucket's records are contiguous in the walk.
        let walk = records(&coll, stores);
        let first = walk.iter().position(|r| bucket.contains(r)).unwrap();
        let run: Vec<u32> = walk[first..first + 3].to_vec();
        let mut got = run.clone();
        got.sort_unstable();
        let mut want = bucket.clone();
        want.sort_unstable();
        assert_eq!(
            got, want,
            "the three same-bucket records are one contiguous run"
        );
    }

    /// D4 — `range` returns exactly the records whose Morton code is in the interval,
    /// in Morton order, respecting the limit.
    #[test]
    fn d4_range_matches_the_code_interval() {
        let mut store = Store::new_in_use(1 << 15);
        let keys = xy_keys();
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };
        // The full 128-bit Morton code of a point, for the brute-force oracle.
        let code_of = |store: &Store, rec: u32| -> [u64; MAX_AXES] {
            morton_words(2, |a| axis_code(store, rec, &keys[a]))
        };
        let mut seed = 0x77aa_33cc_11ee_9988;
        let mut pts = Vec::new();
        for _ in 0..400 {
            let (x, y) = (lcg(&mut seed) % 400 - 200, lcg(&mut seed) % 400 - 200);
            let rec = add_point(&mut store, &coll, &keys, x, y);
            pts.push(rec);
        }
        let stores = std::slice::from_ref(&store);
        for _ in 0..100 {
            let (fx, fy) = (lcg(&mut seed) % 400 - 200, lcg(&mut seed) % 400 - 200);
            let from = morton_words(2, |a| coord_code([fx, fy][a]));
            let got = range(&coll, stores, &keys, &[fx, fy], None, None);
            // brute: records with code >= from, sorted by (code, rec).
            let mut want: Vec<([u64; MAX_AXES], u32)> = pts
                .iter()
                .map(|&rec| (code_of(&store, rec), rec))
                .filter(|(c, _)| !code_gt(&from, c, 2))
                .collect();
            want.sort_unstable();
            let want_recs: Vec<u32> = want.into_iter().map(|(_, r)| r).collect();
            assert_eq!(
                got, want_recs,
                "range from ({fx},{fy}) must be the code tail in order"
            );
        }
        let all = range(&coll, stores, &keys, &[-1000, -1000], None, None);
        let capped = range(&coll, stores, &keys, &[-1000, -1000], None, Some(5));
        assert_eq!(capped.len(), 5.min(all.len()));
        assert_eq!(
            capped,
            all[..capped.len()].to_vec(),
            "limit is a prefix of the full walk"
        );
    }

    /// D3 — the code is order-preserving    /// D3 — the code is order-preserving    /// D3 — the code is order-preserving even across zero: a negative axis must sort
    /// before a positive one, which raw 2's-complement bits would get backwards.
    #[test]
    fn d3_offset_binary_orders_negatives_below_positives() {
        assert!(axis_code_of(-1) < axis_code_of(0), "-1 must encode below 0");
        assert!(axis_code_of(0) < axis_code_of(1), "0 below 1");
        assert!(
            axis_code_of(i64::MIN) < axis_code_of(i64::MAX),
            "min below max"
        );
        // And a raw cast would NOT: (-1 as u64) is all-ones, the largest.
        assert!(
            (-1i64 as u64) > (1i64 as u64),
            "sanity: raw bits are misordered"
        );
    }

    fn axis_code_of(v: i64) -> u64 {
        (v as u64) ^ (1u64 << 63)
    }
}
