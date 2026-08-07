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

use crate::keys::Key;
use crate::radix_tree::KeyOracle;
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
}
