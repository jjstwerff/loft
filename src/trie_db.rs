// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)

//! The bit-key oracle for a TEXT-keyed radix collection.
//!
//! Its own module, not a branch inside `radix_db`. A trie and a spatial index share
//! the radix TREE and nothing above it: `spatial`'s properties are geometric — Morton
//! interleaving, bounding boxes, near/within/nearest — and none of them mean anything
//! for a word. Sharing the storage structure is not sharing the kind, which is why
//! `spatial` is not called `radix` at the surface. See
//! `doc/claude/plans/text-keyed-trie.md`.
//!
//! The tree itself needs nothing new: `radix_tree.rs` is a PATRICIA tree over an
//! abstract bit-key oracle with per-record `bits()`, and its `TERM_BITS` suffix exists
//! so a shorter key sorts before a longer one that extends it — a string concern,
//! proven by `r8c`. This module supplies the bytes; the tree does the rest.
//!
//! Nothing consumes it yet — the trie KIND is step 2 of the plan. It lands first, and
//! proven, so the steps that build on it start from a known-good key reading.

use crate::keys::{Content, DbRef, Key};
use crate::radix_tree::{self as rt, KeyOracle};
use crate::store::Store;

/// Payload offset of an element record's fields; mirrors `radix_db`.
const PAYLOAD: u32 = 8;

/// The text key's bytes, read through the field's string pointer.
///
/// `type_nr == 6` is a `text` key; the field holds a `u32` pointer to the string
/// record, which is the same indirection `compare_key` follows.
fn text_bytes<'a>(store: &'a Store, rec: u32, key: &Key) -> &'a str {
    let p = PAYLOAD + u32::from(key.position);
    store.get_str(store.get_u32_raw(rec, p))
}

/// `text`'s key type number, as `compare_key` spells it.
///
/// Read only by this module's tests until the trie KIND lands (step 2 of
/// `doc/claude/plans/text-keyed-trie.md`), which is what will select an oracle by key
/// type. Kept here rather than inlined into the tests because it is a fact about the
/// schema, not about the fixture.
#[allow(dead_code)]
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

/// The bytes a query key carries, when it is a text one.
///
/// A non-text probe cannot match a text key: answering an empty slice makes that a
/// miss rather than a panic, which is the same choice `probe_code` makes on the
/// coordinate side.
fn probe_bytes(key: &[Content]) -> &[u8] {
    match key.first() {
        Some(Content::Str(v)) => v.str().as_bytes(),
        _ => &[],
    }
}

/// Insert element record `rec` into the `Trie` collection at field `coll`.
///
/// The tree lives in `coll`'s store, its record id in `coll`'s 4-byte field (the
/// `hash`-bucket convention). A growing insert can relocate the tree, so the
/// (possibly new) id is written back. Two records may share a key — they differ in
/// the id suffix and land adjacent, which `r8b` pins.
pub fn add(coll: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    let Some(key) = keys.first() else {
        return;
    };
    let store = crate::keys::mut_store(coll, stores);
    let mut tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        tree = rt::rtree_init(store, 0);
        store.set_u32_raw(coll.rec, coll.pos, tree);
    }
    let tree = rt::rtree_insert(store, tree, rec.rec, &TextOracle { key });
    store.set_u32_raw(coll.rec, coll.pos, tree);
}

/// The record whose key equals `key`, or a null `DbRef` (`rec == 0`) when absent.
/// When several records share the key, the first in order.
#[must_use]
pub fn find(coll: &DbRef, stores: &[Store], keys: &[Key], key: &[Content]) -> DbRef {
    let store = crate::keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    let rec = match keys.first() {
        Some(k) if tree != 0 => {
            // The probe reads bytes exactly as `TextOracle` does — via the same
            // `bytes_word`. The two reading a key differently is the classic radix
            // fault and presents as "inserted, then not found".
            let q = probe_bytes(key);
            let probe = |word: u32| bytes_word(q, word);
            rt::rtree_get(
                store,
                tree,
                &probe,
                q.len() as u32 * 8,
                &TextOracle { key: k },
            )
        }
        _ => 0,
    };
    DbRef {
        store_nr: coll.store_nr,
        rec,
        pos: PAYLOAD,
    }
}

/// Unlink element record `rec`; the caller frees `rec` itself. `false` when it was
/// not present — the tree removes only the record whose key AND id match, so a
/// same-key sibling is never removed by mistake.
pub fn remove(coll: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) -> bool {
    let Some(key) = keys.first() else {
        return false;
    };
    let store = crate::keys::mut_store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        return false;
    }
    rt::rtree_remove(store, tree, rec.rec, &TextOracle { key })
}

/// Number of element records. Reads the tree's cached length word (O(1)).
#[must_use]
pub fn count(coll: &DbRef, stores: &[Store]) -> u32 {
    let store = crate::keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        0
    } else {
        rt::rtree_len(store, tree)
    }
}

/// Every element record, in key order. Key-free (a plain tree walk), so teardown
/// and iteration reach it without building an oracle.
#[must_use]
pub fn records(coll: &DbRef, stores: &[Store]) -> Vec<u32> {
    let store = crate::keys::store(coll, stores);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radix_tree as rt;

    /// One `text` key at payload offset 0.
    fn w_key() -> Key {
        Key {
            type_nr: TEXT_TYPE_NR,
            position: 0,
        }
    }

    /// An element record whose text field holds `w` — the shape `compare_key` reads:
    /// a `u32` pointer at `PAYLOAD + position` to the string record.
    fn add_word(store: &mut Store, w: &str) -> u32 {
        let ptr = store.set_str(w);
        let rec = store.claim(4);
        store.set_u32_raw(rec, PAYLOAD, ptr);
        rec
    }

    const WORDS: [&str; 6] = [
        "kerkstraat",
        "kerk",
        "lonneker",
        "kerf",
        "kerkweg",
        "kerklaan",
    ];

    /// Fill a tree with `WORDS` in an order that is neither sorted nor reverse-sorted,
    /// so a passing order cannot have come from the insertion sequence.
    fn build(store: &mut Store) -> (u32, Vec<(&'static str, u32)>) {
        let key = w_key();
        let mut tree = rt::rtree_init(store, 8);
        let mut recs = Vec::new();
        for w in WORDS {
            let rec = add_word(store, w);
            recs.push((w, rec));
            tree = rt::rtree_insert(store, tree, rec, &TextOracle { key: &key });
        }
        (tree, recs)
    }

    /// In-order traversal is lexicographic, and a prefix sorts before what extends it.
    ///
    /// The `TERM_BITS` property, exercised through the DB record layout rather than a
    /// tree-local fixture: `kerk` must precede the three keys that extend it.
    #[test]
    fn text_keys_order_lexicographically_through_the_db_layout() {
        let mut store = Store::new_in_use(1024);
        let key = w_key();
        let (tree, recs) = build(&mut store);
        rt::rtree_validate(&store, tree, &TextOracle { key: &key });

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

    /// A key just inserted must be found, and one that was not must not be — the two
    /// halves loft#799 got wrong for a text-keyed `spatial`.
    #[test]
    fn text_keys_answer_exact_lookups_through_the_db_layout() {
        let mut store = Store::new_in_use(1024);
        let key = w_key();
        let (tree, recs) = build(&mut store);

        for (w, want) in &recs {
            let q = w.as_bytes();
            let probe = |word: u32| bytes_word(q, word);
            let got = rt::rtree_get(
                &store,
                tree,
                &probe,
                q.len() as u32 * 8,
                &TextOracle { key: &key },
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
                &TextOracle { key: &key },
            );
            assert_eq!(got, 0, "{absent:?} is absent and must not be found");
        }
    }

    // ---- the OPERATIONS, through the collection surface ---------------------
    //
    // The tests above drive the tree directly. These drive `add` / `find` / `remove` /
    // `records` / `count` — the functions `search.rs` and `structures.rs` now call —
    // so a mistake in the collection plumbing (the tree id written back into the
    // field, an empty key list, a probe built the wrong way) cannot hide behind a
    // correct tree.

    /// A collection field holding the tree id, plus an element record per word.
    fn collection(store: &mut Store) -> (DbRef, Vec<(&'static str, u32)>) {
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };
        let keys = vec![w_key()];
        let mut recs = Vec::new();
        for w in WORDS {
            let ptr = store.set_str(w);
            let rec = store.claim(4);
            store.set_u32_raw(rec, PAYLOAD, ptr);
            add(
                &coll,
                &DbRef {
                    store_nr: 0,
                    rec,
                    pos: PAYLOAD,
                },
                std::slice::from_mut(store),
                &keys,
            );
            recs.push((w, rec));
        }
        (coll, recs)
    }

    /// The round trip: every word inserted is found by its own key, and an absent key
    /// answers `rec == 0` rather than a neighbour.
    #[test]
    fn the_collection_round_trips_every_key() {
        let mut store = Store::new_in_use(1 << 14);
        let keys = vec![w_key()];
        let (coll, recs) = collection(&mut store);
        let stores = std::slice::from_ref(&store);

        assert_eq!(count(&coll, stores), WORDS.len() as u32, "count");

        for (w, want) in &recs {
            let got = find(
                &coll,
                stores,
                &keys,
                &[Content::Str(crate::keys::Str::new(w))],
            );
            assert_eq!(got.rec, *want, "find {w:?}");
        }
        // `kerks` extends `kerk` and is a prefix of `kerkstraat` — the shape a probe
        // built differently from the oracle answers for.
        for absent in ["kerks", "ker", "kerkstraatx", "aaa", "zzz"] {
            let got = find(
                &coll,
                stores,
                &keys,
                &[Content::Str(crate::keys::Str::new(absent))],
            );
            assert_eq!(got.rec, 0, "{absent:?} must not be found");
        }
    }

    /// `records` walks in key order, so iteration and teardown see a prefix before
    /// what extends it.
    #[test]
    fn the_collection_walks_in_key_order() {
        let mut store = Store::new_in_use(1 << 14);
        let (coll, recs) = collection(&mut store);
        let walk: Vec<&str> = records(&coll, std::slice::from_ref(&store))
            .into_iter()
            .map(|r| recs.iter().find(|(_, x)| *x == r).expect("known rec").0)
            .collect();
        assert_eq!(
            walk,
            [
                "kerf",
                "kerk",
                "kerklaan",
                "kerkstraat",
                "kerkweg",
                "lonneker"
            ]
        );
    }

    /// Removal unlinks exactly one record and leaves its neighbours findable — the
    /// `kerk` family is the interesting case, since they share a prefix.
    #[test]
    fn removal_takes_one_key_and_leaves_the_rest() {
        let mut store = Store::new_in_use(1 << 14);
        let keys = vec![w_key()];
        let (coll, recs) = collection(&mut store);
        let (_, kerklaan) = *recs.iter().find(|(w, _)| *w == "kerklaan").unwrap();

        let gone = remove(
            &coll,
            &DbRef {
                store_nr: 0,
                rec: kerklaan,
                pos: PAYLOAD,
            },
            std::slice::from_mut(&mut store),
            &keys,
        );
        assert!(gone, "kerklaan was present");
        assert_eq!(count(&coll, std::slice::from_ref(&store)), 5, "one fewer");

        let stores = std::slice::from_ref(&store);
        let missing = find(
            &coll,
            stores,
            &keys,
            &[Content::Str(crate::keys::Str::new("kerklaan"))],
        );
        assert_eq!(missing.rec, 0, "kerklaan is gone");
        for w in ["kerk", "kerkstraat", "kerkweg", "kerf", "lonneker"] {
            let got = find(
                &coll,
                stores,
                &keys,
                &[Content::Str(crate::keys::Str::new(w))],
            );
            assert_ne!(
                got.rec, 0,
                "{w:?} survives the removal of its prefix-sibling"
            );
        }
    }
}
