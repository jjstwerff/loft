// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN97 arc G Phase 2 — a read-only, transient PAGED view over a store image.
//!
//! A [`Store`](crate::store::Store) reads through one contiguous buffer
//! (`*(ptr + rec·8 + fld)`). A remote or very large store has no such buffer —
//! bytes arrive a page at a time. This module is the P0 `b-builtin` result: a
//! self-contained Rust reader that resolves any `(offset, len)` by fetching
//! only the pages it touches, so a `find`/range traversal over a 0.5 GB block
//! pulls a handful of pages, not the whole file. The paged reader is **read
//! only and transient** — it resolves + copies out; the *result* of a
//! `store_load_*` is a normal heap [`Store`](crate::store::Store).
//!
//! Two invariants shape it (see `plans/97-layout-contract/REMOTE_STORE_LOADER.md`):
//! - **byte-identical working set** — because a store file IS its serialization
//!   (`byte = rec·8`, native-endian, per `formal/layout.md`), reading a record
//!   is copying its bytes; [`PagedReader::u32_at`] etc. mirror the
//!   [`Store`](crate::store::Store) accessors exactly.
//! - **zero cost to non-users** — the whole module is behind the `remote-store`
//!   feature, so a lean build compiles none of it and drags in no HTTP deps.
//!
//! The page provider is swappable: [`LocalFileProvider`] is the deterministic
//! test harness (it logs every range fetched, making "bytes fetched ≪ file" a
//! countable assertion); #517 HTTP swaps in at Phase 5 behind the same trait.

/// Page granularity (bytes) — the unit of a fetch. 64 KiB matches the design's
/// recommendation for the small-word index traversal.
pub const PAGE_SIZE: usize = 64 * 1024;

/// The source of a store image's bytes. A [`PagedReader`] only ever asks for
/// page-aligned ranges, and tolerates a fetch running past EOF (the tail is
/// zero-padded), so a provider may clamp to its own size.
pub trait PageProvider {
    /// Total byte length of the backing image.
    fn size(&self) -> u64;
    /// Return exactly `len` bytes starting at `off`. Bytes at or past
    /// [`size`](PageProvider::size) read as zero.
    fn fetch(&mut self, off: u64, len: usize) -> Vec<u8>;
}

/// A local-file page provider — reads ranges from a store file on disk and
/// **logs every `(off, len)` fetched**, so a test can assert the working-set
/// load touched ≪ the whole file. This is the deterministic stand-in for the
/// #517 HTTP `Range` provider (Phase 5); the traversal code is identical.
pub struct LocalFileProvider {
    file: std::fs::File,
    size: u64,
    /// Every `(off, len)` this provider was asked to fetch, in order.
    pub fetches: Vec<(u64, usize)>,
}

impl LocalFileProvider {
    /// Open `path` read-only.
    ///
    /// # Errors
    /// Returns the underlying `io::Error` if `path` can't be opened or its
    /// metadata (length) can't be read.
    pub fn open(path: &str) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            file,
            size,
            fetches: Vec::new(),
        })
    }

    /// Total bytes fetched so far — the numerator of the "≪ file" assertion.
    #[must_use]
    pub fn bytes_fetched(&self) -> usize {
        self.fetches.iter().map(|(_, l)| l).sum()
    }
}

impl PageProvider for LocalFileProvider {
    fn size(&self) -> u64 {
        self.size
    }

    fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
        use std::io::{Read, Seek, SeekFrom};
        self.fetches.push((off, len));
        let mut buf = vec![0u8; len];
        if off < self.size {
            // Read what actually exists; anything past EOF stays zero.
            let want = len.min((self.size - off) as usize);
            if self.file.seek(SeekFrom::Start(off)).is_ok() {
                let _ = self.file.read_exact(&mut buf[..want]);
            }
        }
        buf
    }
}

/// A read-only, transient paged view: a sparse LRU page table + fetch-on-miss
/// [`resolve`](PagedReader::resolve). Only resident pages cost memory, so a
/// small cache over a huge store is fine — it just fetches more. Correctness is
/// **independent of residency** (the `capacity == 1` test proves it).
pub struct PagedReader<P: PageProvider> {
    provider: P,
    page_size: usize,
    /// `page_idx -> page bytes` (always `page_size` long; the last page is
    /// zero-padded past EOF).
    pages: std::collections::HashMap<u64, Vec<u8>>,
    /// LRU order, front = least-recently used; bounded by `capacity`.
    lru: std::collections::VecDeque<u64>,
    /// Max resident pages.
    capacity: usize,
}

impl<P: PageProvider> PagedReader<P> {
    /// A reader with the default 64 KiB page and a 64-page (4 MiB) cache.
    pub fn new(provider: P) -> Self {
        Self::with_config(provider, PAGE_SIZE, 64)
    }

    /// A reader with an explicit page size and cache cap (both clamped to ≥ 1).
    /// The tiny-page form is for tests that want boundary-spanning reads without
    /// a 64 KiB fixture.
    pub fn with_config(provider: P, page_size: usize, capacity: usize) -> Self {
        Self {
            provider,
            page_size: page_size.max(1),
            pages: std::collections::HashMap::new(),
            lru: std::collections::VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// The underlying provider (e.g. to read its fetch log).
    pub fn provider(&self) -> &P {
        &self.provider
    }

    /// Total image size in bytes.
    pub fn size(&self) -> u64 {
        self.provider.size()
    }

    /// Make page `pidx` resident (fetching on miss) and mark it most-recently
    /// used, evicting the LRU page(s) once over capacity.
    fn ensure_page(&mut self, pidx: u64) {
        if self.pages.contains_key(&pidx) {
            if let Some(p) = self.lru.iter().position(|&x| x == pidx) {
                self.lru.remove(p);
            }
            self.lru.push_back(pidx);
            return;
        }
        let bytes = self
            .provider
            .fetch(pidx * self.page_size as u64, self.page_size);
        self.pages.insert(pidx, bytes);
        self.lru.push_back(pidx);
        while self.pages.len() > self.capacity {
            if let Some(old) = self.lru.pop_front() {
                self.pages.remove(&old);
            }
        }
    }

    /// Copy `len` bytes at absolute byte offset `off` into a fresh buffer,
    /// fetching any missing page(s). A read within one page copies from that
    /// page; a read spanning a page boundary is coalesced here. The common
    /// index-traversal read (4–8 bytes) stays within a page.
    pub fn resolve(&mut self, off: u64, len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        let mut done = 0usize;
        while done < len {
            let abs = off + done as u64;
            let pidx = abs / self.page_size as u64;
            let in_page = (abs % self.page_size as u64) as usize;
            let take = (self.page_size - in_page).min(len - done);
            self.ensure_page(pidx);
            let page = &self.pages[&pidx];
            out[done..done + take].copy_from_slice(&page[in_page..in_page + take]);
            done += take;
        }
        out
    }

    // --- typed reads at (rec, fld), byte = rec·8 + fld, native-endian ---
    // These mirror the `Store` accessors so a traversal over the paged reader
    // sees the same values a whole-file `Store` would.

    /// 4-byte unsigned read (bucket slots, record size headers, ptrs).
    pub fn u32_at(&mut self, rec: u32, fld: u32) -> u32 {
        let b = self.resolve(u64::from(rec) * 8 + u64::from(fld), 4);
        u32::from_ne_bytes([b[0], b[1], b[2], b[3]])
    }

    /// 4-byte signed read.
    pub fn i32_at(&mut self, rec: u32, fld: u32) -> i32 {
        self.u32_at(rec, fld) as i32
    }

    /// 8-byte signed read (`integer` / `long` key fields, the seed halves).
    pub fn i64_at(&mut self, rec: u32, fld: u32) -> i64 {
        let b = self.resolve(u64::from(rec) * 8 + u64::from(fld), 8);
        i64::from_ne_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }

    /// The record's size header (word 0) = its word count (`room`).
    pub fn record_words(&mut self, rec: u32) -> u32 {
        self.u32_at(rec, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory provider over a byte image — hermetic, and logs fetches like
    /// the file provider so residency/bounded-fetch tests need no disk.
    struct VecProvider {
        bytes: Vec<u8>,
        fetches: Vec<(u64, usize)>,
    }
    impl VecProvider {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                fetches: Vec::new(),
            }
        }
        fn pages_fetched(&self) -> usize {
            self.fetches.len()
        }
    }
    impl PageProvider for VecProvider {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
            self.fetches.push((off, len));
            let mut buf = vec![0u8; len];
            let start = off as usize;
            if start < self.bytes.len() {
                let end = (start + len).min(self.bytes.len());
                buf[..end - start].copy_from_slice(&self.bytes[start..end]);
            }
            buf
        }
    }

    fn ramp(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn resolve_returns_exact_bytes_including_page_spans() {
        let img = ramp(1000);
        // Tiny 16-byte pages so most reads straddle a boundary.
        let mut r = PagedReader::with_config(VecProvider::new(img.clone()), 16, 64);
        // every offset/len combo must equal the source slice
        for &off in &[0u64, 1, 15, 16, 17, 31, 100, 500, 984] {
            for &len in &[1usize, 4, 8, 16, 33, 64] {
                if off as usize + len > img.len() {
                    continue;
                }
                let got = r.resolve(off, len);
                assert_eq!(
                    got,
                    &img[off as usize..off as usize + len],
                    "resolve({off},{len}) mismatch"
                );
            }
        }
    }

    #[test]
    fn resolve_past_eof_zero_pads() {
        let img = ramp(20);
        let mut r = PagedReader::with_config(VecProvider::new(img.clone()), 16, 64);
        let got = r.resolve(12, 16); // 12..28, but only 12..20 exists
        assert_eq!(&got[..8], &img[12..20]);
        assert!(
            got[8..].iter().all(|&b| b == 0),
            "tail past EOF must be zero"
        );
    }

    #[test]
    fn only_touched_pages_are_fetched() {
        // 100 pages of 16 bytes = 1600 bytes; read one word near the end.
        let mut r = PagedReader::with_config(VecProvider::new(ramp(1600)), 16, 64);
        let _ = r.u32_at(50, 0); // byte 400 → page 25 only
        // Exactly one page fetched, not the whole file.
        assert_eq!(
            r.provider().pages_fetched(),
            1,
            "a single small read must touch one page, not the file"
        );
    }

    #[test]
    fn correctness_is_independent_of_residency() {
        // capacity == 1: every read past the current page re-fetches, but the
        // bytes must still be correct — proves correctness ⟂ cache size.
        let img = ramp(2000);
        let mut small = PagedReader::with_config(VecProvider::new(img.clone()), 16, 1);
        let mut big = PagedReader::with_config(VecProvider::new(img.clone()), 16, 999);
        for &off in &[0u64, 100, 500, 1000, 1500, 1984] {
            assert_eq!(
                small.resolve(off, 12),
                big.resolve(off, 12),
                "capacity-1 reader must agree with a fully-resident one at {off}"
            );
        }
        // The tiny cache never holds more than its cap.
        assert!(small.pages.len() <= 1);
    }

    #[test]
    fn typed_reads_are_native_endian_at_rec_times_8() {
        // Lay out a u32 at (rec=3, fld=4) → byte 28, and an i64 at (rec=5,fld=0)
        // → byte 40, then read them back through the paged reader.
        let mut img = vec![0u8; 64];
        img[28..32].copy_from_slice(&0xDEAD_BEEFu32.to_ne_bytes());
        img[40..48].copy_from_slice(&(-1234567890123i64).to_ne_bytes());
        let mut r = PagedReader::with_config(VecProvider::new(img), 16, 64);
        assert_eq!(r.u32_at(3, 4), 0xDEAD_BEEF);
        assert_eq!(r.i64_at(5, 0), -1234567890123i64);
    }
}
