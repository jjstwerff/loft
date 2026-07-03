// @PLAN53 Wave 2 — structure-aware fuzzing of the store allocator.
//
// Drives random *valid* claim/delete/resize sequences against a `Store`
// (model-based: Delete/Resize index into the live set, so every call is a
// legal API use and any crash is a real bug).  Three oracles stack:
//   1. ASan (cargo-fuzz default) — heap UAF / OOB.
//   2. The store's own debug validators (`fl_validate`, the record-count
//      consistency assert), kept live via debug-assertions in the fuzz profile.
//   3. In-target invariants below: live records never overlap, and a sentinel
//      written into each record always reads back (catches silent corruption
//      from a mis-sized claim / bad coalesce / overlapping resize).
#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use loft::store::Store;

#[derive(Arbitrary, Debug)]
enum Op {
    Claim(u8),      // claim a record of 1..=16 words
    Delete(u8),     // delete the live record at index % len
    Resize(u8, u8), // resize the live record at index % len to 1..=16 words
}

/// A record we currently hold: its offset, its actual claimed word count
/// (claim may round up), and the sentinel we stamped (None if too small).
struct Live {
    rec: u32,
    words: u32,
    sentinel: Option<i64>,
}

/// 8-aligned byte offset past the first (header) word — a safe place to stamp
/// a sentinel in any record of >= 2 words.
const SENTINEL_FLD: u32 = 8;

fn header_words(store: &Store, rec: u32) -> u32 {
    // The first i32 of a claimed record holds its size in words (positive).
    (*store.addr::<i32>(rec, 0)) as u32
}

fn check_no_overlap(live: &[Live]) {
    let mut spans: Vec<(u32, u32)> = live.iter().map(|l| (l.rec, l.rec + l.words)).collect();
    spans.sort_unstable();
    for w in spans.windows(2) {
        assert!(
            w[0].1 <= w[1].0,
            "live records overlap: [{}..{}) and [{}..{})",
            w[0].0,
            w[0].1,
            w[1].0,
            w[1].1,
        );
    }
}

fuzz_target!(|ops: Vec<Op>| {
    let mut store = Store::new_in_use(16);
    let mut live: Vec<Live> = Vec::new();
    let mut next_sentinel: i64 = 0x5EED_0000_0000_0001u64 as i64;

    for op in ops {
        match op {
            Op::Claim(w) => {
                let words = (u32::from(w) % 16) + 1; // 1..=16
                let rec = store.claim(words);
                let actual = header_words(&store, rec);
                let sentinel = if actual >= 2 {
                    let s = next_sentinel;
                    next_sentinel = next_sentinel.wrapping_add(1);
                    assert!(
                        store.set_int(rec, SENTINEL_FLD, s),
                        "set_int rejected an in-bounds field (rec={rec} words={actual})",
                    );
                    Some(s)
                } else {
                    None
                };
                live.push(Live {
                    rec,
                    words: actual,
                    sentinel,
                });
            }
            Op::Delete(i) => {
                if live.is_empty() {
                    continue;
                }
                let idx = (i as usize) % live.len();
                let l = live.swap_remove(idx);
                store.delete(l.rec);
            }
            Op::Resize(i, w) => {
                if live.is_empty() {
                    continue;
                }
                let idx = (i as usize) % live.len();
                let words = (u32::from(w) % 16) + 1;
                let new_rec = store.resize(live[idx].rec, words);
                live[idx].rec = new_rec;
                live[idx].words = header_words(&store, new_rec);
                // resize copies data on move, so an existing sentinel must
                // survive (verified below).  If the record only now grew big
                // enough to hold one, stamp it.
                if live[idx].words >= 2 && live[idx].sentinel.is_none() {
                    let s = next_sentinel;
                    next_sentinel = next_sentinel.wrapping_add(1);
                    store.set_int(new_rec, SENTINEL_FLD, s);
                    live[idx].sentinel = Some(s);
                }
            }
        }

        // Invariants after every operation.
        check_no_overlap(&live);
        for l in &live {
            if let Some(s) = l.sentinel {
                assert_eq!(
                    store.get_int(l.rec, SENTINEL_FLD),
                    s,
                    "record {} sentinel corrupted (words={})",
                    l.rec,
                    l.words,
                );
            }
        }
    }
});
