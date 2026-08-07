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

/// @PLN134 — how many 64 KB PAGES one prefix query touches.
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
///
/// Reports pages per query over a real vocabulary and a real prefix distribution.
/// `#[ignore]` because it reads the host's dictionaries and prints rather than
/// asserts — it answers a design question, it does not defend an invariant.
///
/// ```text
/// cargo test --release --lib trie_db::pages -- --ignored --nocapture
/// ```
#[cfg(test)]
mod pages {
    use super::*;
    use crate::radix_tree as rt;

    const PAGE: u32 = 64 * 1024;

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
        let mut words = std::collections::BTreeSet::new();
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

    fn pct(sorted: &[usize], p: f64) -> usize {
        if sorted.is_empty() {
            return 0;
        }
        #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
        let i = ((sorted.len() - 1) as f64 * p) as usize;
        sorted[i]
    }

    #[test]
    #[ignore = "measurement — run with --release --lib --ignored --nocapture"]
    fn prefix_query_page_span() {
        let words = vocabulary();
        if words.len() < 20_000 {
            println!("SKIP — only {} words available on this host", words.len());
            return;
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
        let node_bytes = 24 + 16 * nodes;
        println!(
            "\nvocabulary {} words | {} nodes | node array {:.2} MB = {} pages of 64 KB",
            words.len(),
            nodes,
            f64::from(node_bytes) / (1024.0 * 1024.0),
            node_bytes.div_ceil(PAGE)
        );

        // THE COUNTERFACTUAL. Every number below is reported twice: once for the
        // layout as built (ids in insertion order) and once for the same tree with
        // ids renumbered breadth-first. Nothing is rebuilt — the touched offsets are
        // mapped through the new ranking — so this is what a layout pass would buy,
        // measured before anyone writes one.
        let bfs = rt::bfs_order(&store, tree);
        let mut rank = vec![u32::MAX; (nodes + 2) as usize];
        for (i, &n) in bfs.iter().enumerate() {
            rank[n as usize] = u32::try_from(i).unwrap_or(u32::MAX);
        }
        let renumber = |off: u32| -> u32 {
            // offset -> node id -> BFS rank -> the offset it WOULD have had
            let id = (off - 24) / 16 + 1;
            let within = (off - 24) % 16;
            match rank.get(id as usize) {
                Some(&r) if r != u32::MAX => 24 + 16 * r + within,
                _ => off,
            }
        };

        // The prefix distribution a search box actually issues: what a person has
        // typed after 2-8 keystrokes, sampled from the vocabulary itself so every
        // probe HITS. A miss walks less, so hits are the honest side.
        for plen in [2usize, 3, 4, 6, 8] {
            let mut spans = Vec::new();
            let mut results = Vec::new();
            // The SAME touch set, bucketed three ways. A descent reads a handful of
            // 16-byte nodes; how much that costs is decided by the fetch granularity,
            // not by the walk, and one page size cannot show that.
            let mut p4 = Vec::new();
            let mut p512 = Vec::new();
            let mut bytes = Vec::new();
            let mut b4 = Vec::new();
            let mut b64 = Vec::new();
            let mut step = words.len() / 400;
            if step == 0 {
                step = 1;
            }
            for w in words.iter().step_by(step) {
                if w.len() < plen {
                    continue;
                }
                let pre = &w.as_bytes()[..plen];
                rt::touch_begin();
                let hits = prefix_offsets(&store, tree, &key, pre, 20);
                let touched = rt::touch_end();
                let pages: std::collections::BTreeSet<u32> =
                    touched.iter().map(|o| o / PAGE).collect();
                spans.push(pages.len());
                p4.push(
                    touched
                        .iter()
                        .map(|o| o / 4096)
                        .collect::<std::collections::BTreeSet<u32>>()
                        .len(),
                );
                p512.push(
                    touched
                        .iter()
                        .map(|o| o / 512)
                        .collect::<std::collections::BTreeSet<u32>>()
                        .len(),
                );
                // Each recorded offset is one 4-byte field read; a node is two words.
                bytes.push(touched.len() * 4);
                let moved: std::collections::BTreeSet<u32> =
                    touched.iter().map(|o| renumber(*o)).collect();
                b4.push(
                    moved
                        .iter()
                        .map(|o| o / 4096)
                        .collect::<std::collections::BTreeSet<u32>>()
                        .len(),
                );
                b64.push(
                    moved
                        .iter()
                        .map(|o| o / PAGE)
                        .collect::<std::collections::BTreeSet<u32>>()
                        .len(),
                );
                results.push(hits);
            }
            spans.sort_unstable();
            let total: usize = spans.iter().sum();
            #[allow(clippy::cast_precision_loss)]
            let mean = total as f64 / spans.len() as f64;
            #[allow(clippy::cast_precision_loss)]
            let mean_bytes = bytes.iter().sum::<usize>() as f64 / bytes.len() as f64;
            #[allow(clippy::cast_precision_loss)]
            let mean_4k = p4.iter().sum::<usize>() as f64 / p4.len() as f64;
            #[allow(clippy::cast_precision_loss)]
            let mean_512 = p512.iter().sum::<usize>() as f64 / p512.len() as f64;
            #[allow(clippy::cast_precision_loss)]
            let bfs_fine = b4.iter().sum::<usize>() as f64 / b4.len() as f64;
            #[allow(clippy::cast_precision_loss)]
            let bfs_wide = b64.iter().sum::<usize>() as f64 / b64.len() as f64;
            println!(
                "  {plen}ch n={:<4} | AS BUILT 64K {mean:5.1} (p95 {:3}) 4K {mean_4k:5.1} \
                 512B {mean_512:5.1} | BFS 64K {bfs_wide:5.1} 4K {bfs_fine:5.1} | bytes read {mean_bytes:4.0}",
                spans.len(),
                pct(&spans, 0.95),
            );
        }
        println!(
            "\n  whole image for comparison: the node array alone is {} pages; \
             a query is worth paging only if it stays far below that.",
            node_bytes.div_ceil(PAGE)
        );
    }

    /// The seek + bounded walk `prefix` performs, without the `DbRef`/`Stores`
    /// plumbing — this measures the TREE, and a collection wrapper would only add
    /// its own constant.
    fn prefix_offsets(store: &Store, tree: u32, key: &Key, pre: &[u8], cap: usize) -> usize {
        let probe = |word: u32| bytes_word(pre, word);
        let mut it = rt::rtree_seek(
            store,
            tree,
            &probe,
            pre.len() as u32 * 8,
            &TextOracle { key },
        );
        let mut n = 0;
        let mut rec = it.rec();
        while rec != 0 && n < cap {
            if !text_bytes(store, rec, key).as_bytes().starts_with(pre) {
                break;
            }
            n += 1;
            rec = it.next(store, tree).unwrap_or(0);
        }
        n
    }
}
