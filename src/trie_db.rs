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

/// Every record whose key begins with `pre`, in key order, capped at `limit`.
///
/// The capability that earns the kind its place, and the one `sorted` cannot offer.
/// A `sorted` range needs a SUCCESSOR string — `c["kerk".."kerl"]` — which the caller
/// has to construct, gets wrong at a byte boundary, and which answers a key INTERVAL
/// rather than a prefix. Here the prefix is the query: seek to it, then walk while the
/// key still begins with it.
///
/// Both halves lean on facts the tree already guarantees. `rtree_seek` positions at
/// the lowest key `>= pre`, and because a probe carries id `0` that is the first
/// record BEARING the prefix (`radix_tree`, and `r8` pins it). In-order traversal is
/// increasing key order, so every extension of the prefix is contiguous from there —
/// which makes the first key that does not begin with `pre` a correct stop, not a
/// heuristic one.
///
/// An empty `pre` is every record, which is what "begins with nothing" means and what
/// `starts_with` already answers.
#[must_use]
pub fn prefix(
    coll: &DbRef,
    stores: &[Store],
    keys: &[Key],
    pre: &[u8],
    limit: Option<usize>,
) -> Vec<u32> {
    let store = crate::keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    let mut out = Vec::new();
    let Some(key) = keys.first() else {
        return out;
    };
    if tree == 0 {
        return out;
    }
    let probe = |word: u32| bytes_word(pre, word);
    let mut it = rt::rtree_seek(
        store,
        tree,
        &probe,
        pre.len() as u32 * 8,
        &TextOracle { key },
    );
    let cap = limit.unwrap_or(usize::MAX);
    let mut rec = it.rec();
    while rec != 0 && out.len() < cap {
        if !text_bytes(store, rec, key).as_bytes().starts_with(pre) {
            break;
        }
        out.push(rec);
        rec = it.next(store, tree).unwrap_or(0);
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

    /// The step-5 gate. Every expectation is hand-computed from the bytes.
    ///
    /// `WORDS` sorts `kerf kerk kerklaan kerkstraat kerkweg lonneker`, so:
    ///   "kerk"  -> kerk and its three extensions, NOT kerf ('f' < 'k')
    ///   "ker"   -> those four AND kerf, since `kerf` begins with `ker` too
    ///   "kerkl" -> kerklaan alone
    ///   "kerx"  -> nothing; it sorts between kerkweg and lonneker, so a seek lands on
    ///              lonneker and a missing stop-check would answer that instead
    ///   ""      -> everything
    #[test]
    fn a_prefix_yields_its_extensions_and_stops() {
        let mut store = Store::new_in_use(1 << 14);
        let keys = vec![w_key()];
        let (coll, recs) = collection(&mut store);
        let stores = std::slice::from_ref(&store);
        let names = |v: Vec<u32>| -> Vec<&str> {
            v.into_iter()
                .map(|r| recs.iter().find(|(_, x)| *x == r).expect("known rec").0)
                .collect()
        };

        assert_eq!(
            names(prefix(&coll, stores, &keys, b"kerk", None)),
            ["kerk", "kerklaan", "kerkstraat", "kerkweg"],
            "the prefix itself is included, and kerf is not"
        );
        assert_eq!(
            names(prefix(&coll, stores, &keys, b"ker", None)),
            ["kerf", "kerk", "kerklaan", "kerkstraat", "kerkweg"],
            "kerf begins with `ker`, so a shorter prefix widens the answer"
        );
        assert_eq!(
            names(prefix(&coll, stores, &keys, b"kerkl", None)),
            ["kerklaan"],
            "a prefix that is not itself a key still finds its extension"
        );
        assert_eq!(
            names(prefix(&coll, stores, &keys, b"lonneker", None)),
            ["lonneker"],
            "an exact key is a prefix of itself"
        );
        assert!(
            prefix(&coll, stores, &keys, b"kerx", None).is_empty(),
            "an absent prefix answers NOTHING, not the neighbour a seek lands on"
        );
        assert!(
            prefix(&coll, stores, &keys, b"zzz", None).is_empty(),
            "a prefix past the last key answers nothing"
        );
        assert_eq!(
            names(prefix(&coll, stores, &keys, b"", None)).len(),
            WORDS.len(),
            "the empty prefix is every record"
        );
        assert_eq!(
            names(prefix(&coll, stores, &keys, b"kerk", Some(2))),
            ["kerk", "kerklaan"],
            "the cap takes the first N in key order"
        );
    }

    /// A prefix query must not be disturbed by a removal inside its run.
    #[test]
    fn a_prefix_reflects_a_removal_in_its_run() {
        let mut store = Store::new_in_use(1 << 14);
        let keys = vec![w_key()];
        let (coll, recs) = collection(&mut store);
        let (_, kerklaan) = *recs.iter().find(|(w, _)| *w == "kerklaan").unwrap();
        remove(
            &coll,
            &DbRef {
                store_nr: 0,
                rec: kerklaan,
                pos: PAYLOAD,
            },
            std::slice::from_mut(&mut store),
            &keys,
        );
        let got: Vec<&str> = prefix(&coll, std::slice::from_ref(&store), &keys, b"kerk", None)
            .into_iter()
            .map(|r| recs.iter().find(|(_, x)| *x == r).expect("known rec").0)
            .collect();
        assert_eq!(got, ["kerk", "kerkstraat", "kerkweg"], "the run closes up");
    }
}

/// @PLN134 — how many 64 KB PAGES one prefix query touches, and what a LAYOUT
/// would do to that number.
///
/// The measurement the plan opens on, and the reason it opens on one at all: a
/// PATRICIA descent is cheap in NODES — one root→leaf path, branching on bits of a
/// probe the caller already holds — and that says nothing about what it costs over a
/// link. A reader fetches `paged_reader::PAGE_SIZE` (64 KB) at a time, and node ids
/// are handed out in INSERTION order, so a path through a large tree can land on a
/// fresh page at every hop.
///
/// Nodes are one contiguous array (`radix_tree::node_off` is `HDR + 16*(id-1)`), so
/// the span a descent can reach is bounded by `16 × node_count` — which is the fact
/// that makes the answer worth measuring rather than assuming in either direction.
/// Step 1 measured it: ~27 pages as built, ~15 renumbered breadth-first, to read
/// ~330 bytes of nodes. So the question became which layout reaches the floor, and
/// every candidate here is a PERMUTATION fed to the same counter — nothing is
/// rebuilt, so a layout costs one walk to evaluate rather than a persist pass.
///
/// Three questions, one fixture:
///
/// * [`prefix_query_page_span`] — the node array, under five numberings.
/// * [`record_placement`] — the records a query RETURNS, which live elsewhere.
/// * [`warm_cache_session`] — what the second keystroke costs, cache still warm.
///
/// `#[ignore]` because they read the host's dictionaries and print rather than
/// assert — they answer a design question, they do not defend an invariant.
///
/// ```text
/// cargo test --release --lib trie_db::pages -- --ignored --nocapture
/// ```
#[cfg(test)]
mod pages {
    #![allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]

    use super::*;
    use crate::radix_tree as rt;
    use std::collections::{BTreeSet, HashMap};

    const PAGE: u64 = 64 * 1024;
    /// The finer granularity the same touch set is bucketed at. A descent reads a
    /// handful of 16-byte nodes; what that costs is decided by the fetch size, not
    /// by the walk, and one page size cannot show that.
    const FINE: u64 = 4096;
    /// `PagedReader`'s default cache, in pages — what a warm session keeps.
    const CACHE_PAGES: usize = 64;
    /// How many records a query returns; `t["kerk"..:20]` caps it, and the cap is
    /// what makes a search box cheap.
    const CAP: usize = 20;

    fn w_key() -> Key {
        Key {
            type_nr: TEXT_TYPE_NR,
            position: 0,
        }
    }

    fn add_word(store: &mut Store, w: &str) -> u32 {
        let ptr = store.set_str(w);
        let rec = store.claim(4);
        store.set_u32_raw(rec, PAYLOAD, ptr);
        rec
    }

    /// Real words, not generated ones: prefix SHARING is the whole subject, and a
    /// synthetic vocabulary has whatever sharing its generator happened to produce.
    /// Several languages, because routing's index is place names — proper nouns
    /// across languages, which is closer to this than one language's lexicon.
    fn vocabulary() -> Vec<String> {
        let mut words = BTreeSet::new();
        for f in [
            "/usr/share/dict/american-english",
            "/usr/share/dict/british-english",
            "/usr/share/dict/french",
            "/usr/share/dict/spanish",
            "/usr/share/dict/italian",
            "/usr/share/dict/ngerman",
            "/usr/share/dict/portuguese",
        ] {
            let Ok(text) = std::fs::read_to_string(f) else {
                continue;
            };
            for w in text.lines() {
                let w = w.trim().to_lowercase();
                // ASCII only: the oracle is bytewise and a multi-byte word is not
                // wrong here, just harder to reason about when reading the output.
                if (2..=24).contains(&w.len()) && w.chars().all(|c| c.is_ascii_lowercase()) {
                    words.insert(w);
                }
            }
        }
        words.into_iter().collect()
    }

    /// The tree all three measurements read.
    struct Fixture {
        words: Vec<String>,
        store: Store,
        tree: u32,
        key: Key,
        nodes: u32,
    }

    /// Build the vocabulary trie, or `None` where the host has no dictionaries.
    fn build() -> Option<Fixture> {
        let words = vocabulary();
        if words.len() < 20_000 {
            println!("SKIP — only {} words available on this host", words.len());
            return None;
        }
        let key = w_key();
        let mut store = Store::new_in_use(1 << 20);
        let mut tree = rt::rtree_init(&mut store, 8);
        // INSERTION ORDER IS THE VARIABLE UNDER TEST. Sorted input would hand the
        // node array the one layout that cannot scatter, and answer a question
        // nobody asked — a real vocabulary arrives in whatever order its source
        // had. Shuffled deterministically so the number is reproducible.
        let mut order: Vec<usize> = (0..words.len()).collect();
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        for i in (1..order.len()).rev() {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            order.swap(i, (seed >> 33) as usize % (i + 1));
        }
        for &i in &order {
            let rec = add_word(&mut store, &words[i]);
            tree = rt::rtree_insert(&mut store, tree, rec, &TextOracle { key: &key });
        }
        let nodes = store.get_u32_raw(tree, 12);
        Some(Fixture {
            words,
            store,
            tree,
            key,
            nodes,
        })
    }

    impl Fixture {
        /// Bytes the node array occupies — `HDR + 16 × nodes`.
        fn node_bytes(&self) -> u64 {
            24 + 16 * u64::from(self.nodes)
        }

        /// The prefixes a search box actually issues: what a person has typed after
        /// `plen` keystrokes, sampled across the vocabulary so every probe HITS. A
        /// miss walks less, so hits are the honest side.
        fn probes(&self, plen: usize, want: usize) -> Vec<&str> {
            let step = (self.words.len() / want).max(1);
            self.words
                .iter()
                .step_by(step)
                .filter(|w| w.len() >= plen)
                .map(|w| &w[..plen])
                .collect()
        }

        /// The seek + bounded walk `prefix` performs, without the `DbRef`/`Stores`
        /// plumbing — this measures the TREE, and a collection wrapper would only
        /// add its own constant. Returns the records it would hand back.
        fn query(&self, pre: &str) -> Vec<u32> {
            let pre = pre.as_bytes();
            let probe = |word: u32| bytes_word(pre, word);
            let spec = TextOracle { key: &self.key };
            let mut it =
                rt::rtree_seek(&self.store, self.tree, &probe, pre.len() as u32 * 8, &spec);
            let mut hits = Vec::new();
            let mut rec = it.rec();
            while rec != 0 && hits.len() < CAP {
                if !text_bytes(&self.store, rec, &self.key)
                    .as_bytes()
                    .starts_with(pre)
                {
                    break;
                }
                hits.push(rec);
                rec = it.next(&self.store, self.tree).unwrap_or(0);
            }
            hits
        }

        /// One query's node reads and its results, in one recording window.
        fn query_touch(&self, pre: &str) -> (BTreeSet<u32>, Vec<u32>) {
            rt::touch_begin();
            let hits = self.query(pre);
            (rt::touch_end(), hits)
        }
    }

    /// A candidate node numbering: where each node would sit if a persist-time pass
    /// wrote the array in this order.
    ///
    /// It maps a RECORDED offset to the offset it would have had, so one walk of the
    /// tree evaluates a layout — no image is rewritten to ask what one costs.
    struct Layout {
        name: &'static str,
        rank: Vec<u32>,
    }

    impl Layout {
        fn new(name: &'static str, order: &[u32], nodes: u32) -> Layout {
            let mut rank = vec![u32::MAX; (nodes + 2) as usize];
            for (i, &n) in order.iter().enumerate() {
                assert_eq!(
                    rank[n as usize],
                    u32::MAX,
                    "{name} numbers node {n} twice — not a permutation"
                );
                rank[n as usize] = u32::try_from(i).unwrap_or(u32::MAX);
            }
            assert_eq!(
                order.len(),
                nodes as usize,
                "{name} covers {} of {nodes} nodes",
                order.len()
            );
            Layout { name, rank }
        }

        /// Ids as handed out, so every number below has its own baseline rather
        /// than a separately computed one.
        fn as_built(nodes: u32) -> Layout {
            Layout::new("as built", &(1..=nodes).collect::<Vec<_>>(), nodes)
        }

        /// Offset -> node id -> rank -> the offset it WOULD have had.
        fn moved(&self, off: u32) -> u64 {
            let id = (off - 24) / 16 + 1;
            let within = u64::from((off - 24) % 16);
            match self.rank.get(id as usize) {
                Some(&r) if r != u32::MAX => 24 + 16 * u64::from(r) + within,
                _ => u64::from(off),
            }
        }

        fn pages(&self, touched: &BTreeSet<u32>, page: u64) -> usize {
            touched
                .iter()
                .map(|o| self.moved(*o) / page)
                .collect::<BTreeSet<u64>>()
                .len()
        }
    }

    /// Every candidate, in the order they are worth reading.
    fn layouts(f: &Fixture) -> Vec<Layout> {
        vec![
            Layout::as_built(f.nodes),
            Layout::new("BFS", &rt::bfs_order(&f.store, f.tree), f.nodes),
            Layout::new("DFS", &rt::dfs_order(&f.store, f.tree), f.nodes),
            Layout::new("key order", &rt::key_order(&f.store, f.tree), f.nodes),
            Layout::new("vEB", &rt::veb_order(&f.store, f.tree), f.nodes),
        ]
    }

    /// One printed row: each item formatted and appended in turn.
    ///
    /// `map(format!).collect::<String>()` is the obvious spelling and allocates per
    /// cell; one helper keeps the four tables here reading the same way.
    fn row<T>(items: impl IntoIterator<Item = T>, cell: impl Fn(T) -> String) -> String {
        items.into_iter().fold(String::new(), |mut s, i| {
            s.push_str(&cell(i));
            s
        })
    }

    fn mean(v: &[usize]) -> f64 {
        v.iter().sum::<usize>() as f64 / v.len().max(1) as f64
    }

    fn pct(v: &[usize], p: f64) -> usize {
        if v.is_empty() {
            return 0;
        }
        let mut s = v.to_vec();
        s.sort_unstable();
        s[((s.len() - 1) as f64 * p) as usize]
    }

    /// Add the pages a byte span covers. A record straddles a boundary as often as
    /// not, and counting only its first page understates every layout equally —
    /// which is worse than it sounds, because the comparison is the point.
    fn add_span(pages: &mut BTreeSet<u64>, off: u64, len: u64, page: u64) {
        for p in (off / page)..=((off + len.saturating_sub(1)) / page) {
            pages.insert(p);
        }
    }

    /// The node array under five numberings.
    #[test]
    #[ignore = "measurement — run with --release --lib --ignored --nocapture"]
    fn prefix_query_page_span() {
        let Some(f) = build() else { return };
        println!(
            "\nvocabulary {} words | {} nodes | node array {:.2} MB = {} pages of 64 KB",
            f.words.len(),
            f.nodes,
            f.node_bytes() as f64 / (1024.0 * 1024.0),
            f.node_bytes().div_ceil(PAGE)
        );
        let ls = layouts(&f);
        println!(
            "\n  distinct pages per query, mean (p95)      floor = 1 page\n  {:>6} {:>5} {}",
            "prefix",
            "page",
            row(&ls, |l| format!("{:>13}", l.name))
        );
        for plen in [2usize, 3, 4, 6, 8] {
            let probes = f.probes(plen, 400);
            let touched: Vec<BTreeSet<u32>> = probes
                .iter()
                .map(|p| f.query_touch(p).0)
                .collect::<Vec<_>>();
            let bytes = mean(&touched.iter().map(|t| t.len() * 4).collect::<Vec<_>>());
            for (page, label) in [(PAGE, "64K"), (FINE, "4K")] {
                let cells = row(&ls, |l| {
                    let v: Vec<usize> =
                        touched.iter().map(|t| l.pages(t, page)).collect::<Vec<_>>();
                    format!("{:>8.1} ({:>2})", mean(&v), pct(&v, 0.95))
                });
                println!("  {plen:>4}ch {label:>5}{cells}");
            }
            println!(
                "         {:>5}  {bytes:.0} bytes of node data actually read",
                ""
            );
        }
    }

    /// The records a query RETURNS — a separate allocation with its own layout, and
    /// the half the node measurement says nothing about.
    ///
    /// The counterfactual is the placement `routing` already uses for its postings:
    /// write the records in trie KEY order, so one prefix is one contiguous run.
    #[test]
    #[ignore = "measurement — run with --release --lib --ignored --nocapture"]
    fn record_placement() {
        let Some(f) = build() else { return };
        // Where a key-order pass would put each record: its element record and its
        // string, together, in the order the tree walks them.
        let mut place: HashMap<u32, (u64, u64)> = HashMap::with_capacity(f.words.len());
        let mut at = 0u64;
        let mut it = rt::rtree_first(&f.store, f.tree);
        let mut rec = it.rec();
        while rec != 0 {
            let len =
                u64::from(f.store.record_words(rec) + f.store.record_words(str_of(&f, rec))) * 8;
            place.insert(rec, (at, len));
            at += len;
            rec = it.next(&f.store, f.tree).unwrap_or(0);
        }
        println!(
            "\n  records {} | {:.2} MB packed in key order = {} pages of 64 KB",
            place.len(),
            at as f64 / (1024.0 * 1024.0),
            at.div_ceil(PAGE)
        );
        println!(
            "\n  distinct pages for the {CAP} records a query returns, mean (p95)\n  \
             {:>6} {:>5} {:>13} {:>13}",
            "prefix", "page", "as inserted", "key order"
        );
        for plen in [2usize, 3, 4, 6, 8] {
            let hits: Vec<Vec<u32>> = f
                .probes(plen, 400)
                .iter()
                .map(|p| f.query(p))
                .collect::<Vec<_>>();
            for (page, label) in [(PAGE, "64K"), (FINE, "4K")] {
                let built: Vec<usize> = hits
                    .iter()
                    .map(|h| {
                        let mut pages = BTreeSet::new();
                        for &r in h {
                            let s = str_of(&f, r);
                            add_span(
                                &mut pages,
                                u64::from(r) * 8,
                                u64::from(f.store.record_words(r)) * 8,
                                page,
                            );
                            add_span(
                                &mut pages,
                                u64::from(s) * 8,
                                u64::from(f.store.record_words(s)) * 8,
                                page,
                            );
                        }
                        pages.len()
                    })
                    .collect();
                let keyed: Vec<usize> = hits
                    .iter()
                    .map(|h| {
                        let mut pages = BTreeSet::new();
                        for &r in h {
                            let (off, len) = place[&r];
                            add_span(&mut pages, off, len, page);
                        }
                        pages.len()
                    })
                    .collect();
                println!(
                    "  {plen:>4}ch {label:>5} {:>8.1} ({:>2}) {:>8.1} ({:>2})",
                    mean(&built),
                    pct(&built, 0.95),
                    mean(&keyed),
                    pct(&keyed, 0.95)
                );
            }
        }
    }

    /// One search box session: the keystrokes of a single word, against a reader
    /// that keeps `CACHE_PAGES` pages.
    ///
    /// Every number in the other two measurements is the COLD cost of one query.
    /// A real search box issues a query per keystroke, each a prefix of the last,
    /// and the top of the tree is shared by all of them — so what a user waits for
    /// is the MARGINAL fetch, not the first one.
    #[test]
    #[ignore = "measurement — run with --release --lib --ignored --nocapture"]
    fn warm_cache_session() {
        let Some(f) = build() else { return };
        let ls = layouts(&f);
        let keys = [1usize, 2, 3, 4, 5, 6, 7, 8];
        println!(
            "\n  pages FETCHED per keystroke, {CACHE_PAGES}-page cache, cold at keystroke 1\n  \
             {:>10}{}",
            "layout",
            row(&keys, |k| format!("{k:>6}"))
        );
        for l in &ls {
            let words: Vec<&String> = f
                .words
                .iter()
                .step_by((f.words.len() / 200).max(1))
                .filter(|w| w.len() >= 8)
                .collect();
            let mut per_key = vec![Vec::new(); keys.len()];
            for w in &words {
                // A cache per session: a user types one word, and the next user
                // starts cold. The steady state across many words is kinder still,
                // and this is the honest side.
                let mut cache: Vec<u64> = Vec::with_capacity(CACHE_PAGES);
                for (i, &k) in keys.iter().enumerate() {
                    let touched = f.query_touch(&w[..k]).0;
                    let want: BTreeSet<u64> = touched.iter().map(|o| l.moved(*o) / PAGE).collect();
                    let mut fetched = 0;
                    for p in want {
                        if let Some(at) = cache.iter().position(|&c| c == p) {
                            let p = cache.remove(at);
                            cache.push(p);
                        } else {
                            fetched += 1;
                            if cache.len() == CACHE_PAGES {
                                cache.remove(0);
                            }
                            cache.push(p);
                        }
                    }
                    per_key[i].push(fetched);
                }
            }
            println!(
                "  {:>10}{}",
                l.name,
                row(&per_key, |v| format!("{:>6.1}", mean(v)))
            );
        }
    }

    /// The string record behind an element record's only field.
    fn str_of(f: &Fixture, rec: u32) -> u32 {
        f.store
            .get_u32_raw(rec, PAYLOAD + u32::from(f.key.position))
    }
}
