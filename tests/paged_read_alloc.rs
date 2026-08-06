// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — loft#787: a small paged read must not allocate.
//
//! **The browser was never needed to establish this, and reaching for it first was the
//! mistake.** loft#787 is a browser-only *symptom*, so three probes were taken natively, saw
//! nothing, and a browser A/B was built to see more. But what the loader actually does —
//! how many ranges it fetches, how many bytes, how many pages it keeps resident, and how
//! many times it calls the allocator — is a property of the ALGORITHM. It is identical on
//! every target. Only the PRICE of an allocation is target-specific.
//!
//! So the countable half belongs here, in a test that runs in 20 ms on every CI machine,
//! and the browser is left to answer the one question it alone can: what that count costs.
//!
//! What this pins: `PagedReader::u32_at`/`i64_at` used to route through `resolve`, which
//! returns an owning `Vec` — so reading a four-byte index word cost one malloc and one
//! free. A single viewport in the consumer that reported #787 performs ~800 000 such reads
//! (their loft#783 measurement), so it made ~800 000 allocations to carry four bytes at a
//! time. Natively that hides inside glibc's per-thread cache; in a browser it is dlmalloc
//! compiled to wasm, on the linear heap, with no thread-local fast path — which is why the
//! cost showed up per READ, tracking a count that went DOWN while wall time went UP.
//!
//! The fix is `read_into`, which fills a caller buffer. This test asserts the property that
//! makes it a fix — **zero allocations for a resident small read** — rather than a duration,
//! because a duration here would measure this machine and assert nothing about a phone.

#![cfg(feature = "remote-store")]

use loft::paged_reader::{PageProvider, PagedReader};
use std::alloc::{GlobalAlloc, Layout, System};

/// Counts allocations made **by the arming thread**, while armed.
///
/// ⚠ Thread-local, and that is not fastidiousness. A pair of shared statics reads as
/// obviously fine and is not: `cargo test` runs the tests in this binary on parallel
/// THREADS, so a sibling test building its fixture allocates inside another test's armed
/// window and the count comes back wrong — non-deterministically, and only when the whole
/// file runs, so it passes when you re-run the failing test alone. Which is exactly what it
/// did here before this comment existed.
///
/// `Cell<usize>` with a `const` initialiser: no destructor and no allocation, so the
/// allocator cannot re-enter itself through its own counter.
struct Counting;

thread_local! {
    static ALLOCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        // `try_with`: during thread teardown the TLS is gone, and a panic in the
        // allocator is not recoverable.
        let _ = ARMED.try_with(|a| {
            if a.get() {
                let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
            }
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Pages already in hand — the reader under test, with the fetch taken out of the
/// picture so what remains is the per-read cost and nothing else.
struct Resident(Vec<u8>);

impl PageProvider for Resident {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        let lo = usize::try_from(off).unwrap_or(0).min(self.0.len());
        let hi = (lo + len).min(self.0.len());
        out[..hi - lo].copy_from_slice(&self.0[lo..hi]);
        out
    }
}

fn armed<T>(f: impl FnOnce() -> T) -> (T, usize) {
    ALLOCS.with(|c| c.set(0));
    ARMED.with(|a| a.set(true));
    let out = f();
    ARMED.with(|a| a.set(false));
    (out, ALLOCS.with(std::cell::Cell::get))
}

/// **A resident small read does not allocate, and the count does not grow with reads.**
///
/// Stated as a scaling property rather than a bare zero, because a bare zero is fragile in
/// the wrong direction: `ensure_page` allocates the page it stores, which is legitimate and
/// per-PAGE, so a test pinned at zero fails the moment the read range widens by one page and
/// tells you nothing about the defect. The defect was that the count tracked READS. So the
/// same page range is read 300 times and 300 000 times, and the two counts must be EQUAL.
///
/// Before `read_into` that would have been 300 against 300 000.
#[test]
fn the_allocation_count_does_not_grow_with_the_number_of_reads() {
    let img: Vec<u8> = (0..64u32 * 1024).map(|i| (i % 251) as u8).collect();
    let mut r = PagedReader::new(Resident(img));
    let _ = r.u32_at(0, 0); // fault the one page in; everything below stays inside it

    // Both loops read the SAME 100 records — one page, no faults — so any allocation left
    // is a per-read one.
    let mut count = |laps: u32| {
        armed(|| {
            let mut sum = 0u64;
            for _ in 0..laps {
                for rec in 0..100u32 {
                    sum += u64::from(r.u32_at(rec, 0));
                    sum = sum.wrapping_add(r.i64_at(rec, 0) as u64);
                }
            }
            assert_ne!(sum, 0, "the reads must actually happen");
        })
        .1
    };
    let few = count(1); // 200 reads
    let many = count(1_500); // 300 000 reads

    assert_eq!(
        few, many,
        "the allocation count moved with the read count ({few} for 200 reads, {many} for \
         300 000) — that is loft#787: a four-byte index word carried in an owned Vec, one \
         malloc and one free each, ~800 000 times per viewport in the reporting consumer"
    );
    assert_eq!(
        many, 0,
        "a resident small read should allocate nothing at all"
    );
}

/// A read that STRADDLES a page boundary still must not allocate — that is the
/// branch the fast path declines, and the one most likely to regress silently
/// because it is rare and correct-looking either way.
#[test]
fn a_seam_crossing_small_read_does_not_allocate() {
    let img: Vec<u8> = (0..128u32 * 1024).map(|i| (i % 251) as u8).collect();
    let mut r = PagedReader::new(Resident(img));
    // 64 KiB page: byte 65 532 starts 4 bytes before the seam, so an 8-byte read at
    // rec = 8191, fld = 4 spans pages 0 and 1. Fault both in first.
    let _ = r.u32_at(0, 0);
    let _ = r.u32_at(8_192, 0);

    let (v, allocs) = armed(|| r.i64_at(8_191, 4));
    assert_ne!(v, 0, "the read must actually happen");
    assert_eq!(
        allocs, 0,
        "a seam-crossing 8-byte read allocated {allocs} times"
    );
}

/// The span read — a record body being relocated — is ALLOWED its buffer, and this
/// says so out loud. Driving that to zero too would mean handing callers a borrow of
/// the page table, and the record they are copying may straddle pages that eviction
/// can move underneath them. One allocation per RECORD is the right unit; one per
/// four-byte field was not.
#[test]
fn a_span_read_may_still_allocate_once() {
    let img: Vec<u8> = (0..64u32 * 1024).map(|i| (i % 251) as u8).collect();
    let mut r = PagedReader::new(Resident(img));
    let _ = r.u32_at(0, 0);

    let (got, allocs) = armed(|| r.resolve(0, 4_096));
    assert_eq!(got.len(), 4_096);
    assert_eq!(
        allocs, 1,
        "a span read owns its buffer — exactly one allocation, not one per field"
    );
}

/// **And the other per-read cost: how many times the page map is hashed.**
///
/// The allocation was one of two per-read costs on this path; the second was that the read
/// hashed the SAME key twice. `ensure_page` asked `contains_key(&pidx)` and returned
/// nothing, so the caller indexed `self.pages[&pidx]` for the page it had just confirmed —
/// two hashes and two table probes to carry four bytes. The `entry` API answers both halves
/// of "is it there, and if not put it there" with one hash, which is the question the read
/// path was asking all along.
///
/// Counted, not reasoned about: `page_ops` is bumped at every map operation in the module,
/// so this is the whole cost and not the part that was remembered.
#[test]
fn a_resident_read_hashes_the_page_key_once() {
    let img: Vec<u8> = (0..64u32 * 1024).map(|i| (i % 251) as u8).collect();
    let mut r = PagedReader::new(Resident(img));
    let _ = r.u32_at(0, 0); // fault the page; below is hits only

    let before = r.page_ops();
    let mut sum = 0u64;
    for rec in 0..1_000u32 {
        sum += u64::from(r.u32_at(rec, 0));
    }
    let per_read = (r.page_ops() - before) as f64 / 1_000.0;

    assert_ne!(sum, 0, "the reads must actually happen");
    assert!(
        (per_read - 1.0).abs() < f64::EPSILON,
        "a resident small read costs {per_read} page-map operations, not 1 — the read path \
         is hashing the same key more than once (loft#787)"
    );
}
