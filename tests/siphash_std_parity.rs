// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#827 — `loft::siphash::SipHasher13` computes exactly what `DefaultHasher` does.
//!
//! A persisted `hash<T[k]>` stores its seed so every reader re-derives the same buckets,
//! which makes the hash function part of the STORE FORMAT. loft used to borrow that
//! function from `std::hash::DefaultHasher`, whose algorithm std explicitly does not
//! guarantee across releases — so a toolchain upgrade could move bucket placement with no
//! loft change, and therefore nothing for a layout identity or a `placement` token to
//! refuse. loft now owns the definition.
//!
//! Owning it is only safe if the copy is EXACT: anything else silently invalidates every
//! store already written. This file is that proof, and it is a differential test rather
//! than a golden one on purpose — a golden pins whatever the author happened to produce,
//! while this compares against the thing being replaced, over a corpus wide enough to
//! cross every branch of SipHash's buffering.
//!
//! # When this test fails
//!
//! Two very different causes, and they need opposite responses:
//!
//! * **loft's copy drifted** — someone edited `siphash.rs`. Fix the copy. Existing stores
//!   depend on it.
//! * **std changed its hasher** — the rustc in use computes something else. loft is
//!   UNAFFECTED, which is the entire point of the change: this now surfaces as a red test
//!   on a machine rather than as wrong lookups in someone's data. Retire this file (and
//!   the `DefaultHasher` mention in `siphash.rs`); do NOT follow std, because following it
//!   would break every store loft has written.
//!
//! Deciding which happened is a `git log src/siphash.rs` away.

use loft::siphash::SipHasher13;
use std::hash::{DefaultHasher, Hash, Hasher};

/// What loft feeds a hasher, expressed once so both sides run the identical sequence.
///
/// These are exactly the two kinds `keys::hash_ref` and `keys::key_hash` produce: an
/// integer key (every narrow width is widened to `i64` first) and a text key. The leading
/// `u64` is the per-hash seed `keys::seeded_hasher` always writes.
#[derive(Debug, Clone)]
enum Feed {
    Seed(u64),
    Int(i64),
    Text(&'static str),
}

fn via_std(feeds: &[Feed]) -> u64 {
    let mut h = DefaultHasher::new();
    for f in feeds {
        match f {
            Feed::Seed(s) => h.write_u64(*s),
            Feed::Int(i) => i.hash(&mut h),
            Feed::Text(t) => t.hash(&mut h),
        }
    }
    h.finish()
}

fn via_loft(feeds: &[Feed]) -> u64 {
    let mut h = SipHasher13::new();
    for f in feeds {
        match f {
            Feed::Seed(s) => h.write_u64(*s),
            Feed::Int(i) => h.write_i64(*i),
            Feed::Text(t) => h.write_str(t),
        }
    }
    h.finish()
}

fn agree(feeds: &[Feed], what: &str) {
    assert_eq!(
        via_loft(feeds),
        via_std(feeds),
        "loft's SipHash-1-3 and std's DefaultHasher disagree on {what} — see this file's \
         header before changing anything: if loft's copy drifted, fix the copy; if std \
         changed, loft is fine and this test is what retires"
    );
}

/// Integer keys, across the seeds and values a real store produces.
#[test]
fn integer_keys_hash_identically() {
    let seeds: [u64; 6] = [0, 1, u64::MAX, 0x0123_4567_89ab_cdef, 0xdead_beef, 42];
    let values: [i64; 12] = [
        0,
        1,
        -1,
        2,
        -2,
        127,
        128,
        i64::from(i32::MAX),
        i64::from(i32::MIN),
        i64::MAX,
        i64::MIN,
        0x00ff_00ff_00ff_00ff,
    ];
    for s in seeds {
        for v in values {
            agree(
                &[Feed::Seed(s), Feed::Int(v)],
                &format!("seed {s} / int {v}"),
            );
        }
    }
}

/// Text keys. The lengths matter more than the contents: SipHash buffers into an 8-byte
/// tail, so every length mod 8 exercises a different path through `write`, and the `0xFF`
/// terminator shifts which one a given string lands on.
#[test]
fn text_keys_hash_identically() {
    const WORDS: [&str; 18] = [
        "",
        "a",
        "ab",
        "abc",
        "abcd",
        "abcde",
        "abcdef",
        "abcdefg",
        "abcdefgh",
        "abcdefghi",
        "abcdefghijklmno",
        "abcdefghijklmnop",
        "abcdefghijklmnopq",
        "kerk",
        "kerkweg",
        "lonneker",
        // Multi-byte UTF-8: the byte length is what the hasher sees, not the char count.
        "Ω≈ç√∫˜µ≤≥÷",
        "日本語のキー",
    ];
    for s in [0u64, 7, u64::MAX] {
        for w in WORDS {
            agree(
                &[Feed::Seed(s), Feed::Text(w)],
                &format!("seed {s} / {w:?}"),
            );
        }
    }
}

/// Compound keys — several fields into one hasher. This is where a buffering bug hides:
/// each feed leaves a partial tail that the next one has to continue from, so the second
/// value is written at every possible offset within the 8-byte word.
#[test]
fn compound_keys_hash_identically() {
    const WORDS: [&str; 5] = ["", "a", "abcdefg", "abcdefgh", "abcdefghi"];
    for a in WORDS {
        for b in WORDS {
            agree(
                &[Feed::Seed(9), Feed::Text(a), Feed::Text(b)],
                &format!("text {a:?} then {b:?}"),
            );
        }
        for v in [0_i64, -1, i64::MAX] {
            agree(
                &[Feed::Seed(9), Feed::Text(a), Feed::Int(v)],
                &format!("text {a:?} then int {v}"),
            );
            agree(
                &[Feed::Seed(9), Feed::Int(v), Feed::Text(a)],
                &format!("int {v} then text {a:?}"),
            );
        }
    }
    // Three fields, so a key that spans more than two writes is covered too.
    agree(
        &[
            Feed::Seed(3),
            Feed::Int(7),
            Feed::Text("abcde"),
            Feed::Int(-9),
        ],
        "a three-field compound key",
    );
}

/// A long payload, so the 8-byte fast loop runs many times rather than only the tail path.
/// Lengths straddle the word boundary in both directions around several multiples of 8.
#[test]
fn long_payloads_hash_identically() {
    let base: String = ('a'..='z').cycle().take(300).collect();
    for len in [
        0, 1, 7, 8, 9, 15, 16, 17, 63, 64, 65, 127, 128, 129, 255, 256,
    ] {
        let s: &'static str = Box::leak(base[..len].to_string().into_boxed_str());
        agree(
            &[Feed::Seed(11), Feed::Text(s)],
            &format!("{len}-byte text"),
        );
    }
}

/// The raw byte interface, independent of how loft feeds it — this is the layer the two
/// implementations actually share, so a disagreement here localises the bug to `write`.
#[test]
fn raw_byte_writes_hash_identically() {
    let bytes: Vec<u8> = (0..=255u8).collect();
    for len in 0..=bytes.len() {
        let mut lo = SipHasher13::new();
        lo.write(&bytes[..len]);
        let mut st = DefaultHasher::new();
        st.write(&bytes[..len]);
        assert_eq!(lo.finish(), st.finish(), "raw write of {len} bytes");
    }
    // Split across two writes at every boundary: the tail carried between calls is the
    // part a from-scratch implementation is most likely to get wrong.
    for split in 0..=32 {
        let mut lo = SipHasher13::new();
        lo.write(&bytes[..split]);
        lo.write(&bytes[split..64]);
        let mut st = DefaultHasher::new();
        st.write(&bytes[..split]);
        st.write(&bytes[split..64]);
        assert_eq!(lo.finish(), st.finish(), "64 bytes split at {split}");
    }
}
