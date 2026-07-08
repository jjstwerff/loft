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

/// An HTTP `Range` page provider — fetches byte ranges from a remote store over
/// `Range: bytes=a-b` GETs (the #517 stack), so a working-set load pulls only the
/// pages a lookup touches across the network. The remote counterpart of
/// [`LocalFileProvider`]; the paged reader + traversal + copy above it are
/// identical, so only the byte source differs.
pub struct HttpRangeProvider {
    url: String,
    size: u64,
    agent: ureq::Agent,
    /// Every `(off, len)` fetched — for the "bytes fetched ≪ file" assertion.
    pub fetches: Vec<(u64, usize)>,
}

/// Parse the total from a `Content-Range` header value (`"bytes 0-0/12345"` →
/// `12345`; an unknown total `"*"` → `None`).
fn parse_content_range_total(h: &str) -> Option<u64> {
    h.rsplit('/').next().and_then(|t| t.trim().parse().ok())
}

impl HttpRangeProvider {
    /// Open `url`, discovering the store's total size via a `Range: bytes=0-0`
    /// probe (`Content-Range: bytes 0-0/TOTAL`), falling back to `Content-Length`.
    ///
    /// # Errors
    /// Returns a message when the request fails or the size can't be determined.
    pub fn open(url: &str) -> Result<Self, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30))
            .build();
        let resp = agent
            .get(url)
            .set("Range", "bytes=0-0")
            .call()
            .map_err(|e| e.to_string())?;
        let size = resp
            .header("Content-Range")
            .and_then(parse_content_range_total)
            .or_else(|| resp.header("Content-Length").and_then(|c| c.parse().ok()))
            .ok_or_else(|| "remote store: could not determine size".to_string())?;
        Ok(Self {
            url: url.to_string(),
            size,
            agent,
            fetches: Vec::new(),
        })
    }

    /// Total bytes fetched so far.
    #[must_use]
    pub fn bytes_fetched(&self) -> usize {
        self.fetches.iter().map(|(_, l)| l).sum()
    }
}

impl PageProvider for HttpRangeProvider {
    fn size(&self) -> u64 {
        self.size
    }

    fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
        use std::io::Read;
        self.fetches.push((off, len));
        let mut buf = vec![0u8; len];
        if off >= self.size {
            return buf; // wholly past EOF — zero
        }
        let last = (off + len as u64 - 1).min(self.size - 1);
        let want = (last - off + 1) as usize;
        if let Ok(resp) = self
            .agent
            .get(&self.url)
            .set("Range", &format!("bytes={off}-{last}"))
            .call()
        {
            if resp.status() == 206 {
                // Partial content: the body IS the requested range.
                let _ = resp.into_reader().read_exact(&mut buf[..want]);
            } else {
                // Server ignored Range (200, whole file): skip to `off`, then read.
                // Correct (the prove-can-fail path), though it reads the prefix.
                let mut r = resp.into_reader();
                let mut skip = vec![0u8; off as usize];
                if r.read_exact(&mut skip).is_ok() {
                    let _ = r.read_exact(&mut buf[..want]);
                }
            }
        }
        buf
    }
}

/// The byte source a `store_load*` reads through, chosen by the path scheme: a
/// local file or an HTTP `Range` URL. Lets the generic [`PagedReader`] and the
/// find / relocating-copy above it stay ONE code path regardless of where the
/// bytes live — the whole point of the `PageProvider` seam.
pub enum PageSource {
    Local(LocalFileProvider),
    Http(HttpRangeProvider),
}

impl PageSource {
    /// Open `spec` as an HTTP URL (`http://` / `https://`) or a local file path.
    ///
    /// # Errors
    /// Propagates the underlying provider's open error.
    pub fn open(spec: &str) -> Result<Self, String> {
        if spec.starts_with("http://") || spec.starts_with("https://") {
            Ok(PageSource::Http(HttpRangeProvider::open(spec)?))
        } else {
            LocalFileProvider::open(spec)
                .map(PageSource::Local)
                .map_err(|e| e.to_string())
        }
    }

    /// Total bytes fetched so far, whichever provider backs this source.
    #[must_use]
    pub fn bytes_fetched(&self) -> usize {
        match self {
            PageSource::Local(p) => p.bytes_fetched(),
            PageSource::Http(p) => p.bytes_fetched(),
        }
    }
}

impl PageProvider for PageSource {
    fn size(&self) -> u64 {
        match self {
            PageSource::Local(p) => p.size(),
            PageSource::Http(p) => p.size(),
        }
    }
    fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
        match self {
            PageSource::Local(p) => p.fetch(off, len),
            PageSource::Http(p) => p.fetch(off, len),
        }
    }
}

/// The layout-identity gate for a working-set load (@PLN97 3b.5). Given the
/// store `spec` (a local path or an `http(s)://` URL), read the `.dschema`
/// sidecar beside it and compare its recorded layout with the running program's
/// `current` identity. An ABSENT sidecar (a legacy / pre-3b.5 store) or an
/// unreachable remote yields [`SchemaVerdict::Fresh`] — the caller proceeds and
/// the post-copy `store_verify` stays the backstop; a PRESENT sidecar that is
/// mismatched or unparseable yields a non-raw-safe verdict the caller rejects
/// on, so foreign-layout bytes are never range-read at schema-derived offsets.
#[must_use]
pub fn check_sidecar(
    spec: &str,
    current: &crate::schema_sidecar::LayoutIdentity,
) -> crate::schema_sidecar::SchemaVerdict {
    use crate::schema_sidecar::SchemaVerdict;
    if spec.starts_with("http://") || spec.starts_with("https://") {
        match http_get_text(&format!("{spec}.dschema")) {
            Some(text) => crate::schema_sidecar::verdict_for_sidecar_text(&text, current),
            None => SchemaVerdict::Fresh, // 404 / unreachable → proceed (store_verify backstop)
        }
    } else {
        crate::schema_sidecar::check_beside(std::path::Path::new(spec), current)
            .unwrap_or(SchemaVerdict::Fresh) // IO error (not NotFound) → proceed (backstop)
    }
}

/// GET a small text resource (the `.dschema` sidecar) in full. `None` on any
/// non-2xx status (e.g. 404 = the sidecar is absent) or a transport error.
fn http_get_text(url: &str) -> Option<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();
    agent.get(url).call().ok()?.into_string().ok()
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

/// Locate the record stored under `key` in a HASH rooted at `(root_rec,
/// root_pos)` of the paged image — a read-only port of `hash::find`
/// (`src/hash.rs`) over [`PagedReader`] instead of a `Store`. Returns the entry
/// record number, or 0 when absent. `keys` is the collection's key schema
/// (`Stores::keys(type)`); `key_hash` is reused verbatim (it hashes the
/// `Content` key, not a store). The bucket-record layout constants mirror
/// `hash.rs`: seed at fld 8, buckets from fld 16, `elms = (room-2)·2`.
pub fn find_hash_entry<P: PageProvider>(
    reader: &mut PagedReader<P>,
    root_rec: u32,
    root_pos: u32,
    key: &[crate::keys::Content],
    keys: &[crate::keys::Key],
) -> u32 {
    let claim = reader.u32_at(root_rec, root_pos);
    if claim == 0 {
        return 0;
    }
    let room = reader.record_words(claim);
    if room < 2 {
        return 0;
    }
    let elms = (room - 2) * 2;
    let seed = reader.i64_at(claim, 8) as u64; // SEED_FLD
    let hash_val = crate::keys::key_hash(key, seed);
    let mut index = (hash_val % u64::from(elms)) as u32;
    let mut rec = reader.u32_at(claim, 16 + index * 4); // BUCKET0
    for _ in 0..elms {
        if rec == 0 {
            return 0;
        }
        if key_compare_reader(reader, key, rec, 8, keys) == std::cmp::Ordering::Equal {
            return rec;
        }
        index = (index + 1) % elms;
        rec = reader.u32_at(claim, 16 + index * 4);
    }
    0
}

/// Read the string at string-record `rec` over the paged reader (fld 4 = length,
/// fld 8.. = UTF-8 bytes). A `rec == 0` / out-of-range yields the empty string —
/// `resolve` zero-pads, so this never faults on a wild pointer.
fn read_reader_str<P: PageProvider>(reader: &mut PagedReader<P>, rec: u32) -> String {
    if rec == 0 {
        return String::new();
    }
    let len = reader.u32_at(rec, 4) as usize;
    let bytes = reader.resolve(u64::from(rec) * 8 + 8, len);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Compare `key` against the key fields of the entry at `(rec, pos)` in the
/// paged image — the reader port of `keys::compare_key` for the fixed-width
/// numeric key types (1/2 = i64 `integer`/`long`, 8/9/10/11 = narrow ints) and
/// text keys (6).
/// Text/float keys (types 6/3/4) are a follow-up (they need a string-record
/// read / float compare); an unsupported type returns `Greater` so it never
/// falsely matches.
fn key_compare_reader<P: PageProvider>(
    reader: &mut PagedReader<P>,
    key: &[crate::keys::Content],
    rec: u32,
    pos: u32,
    keys: &[crate::keys::Key],
) -> std::cmp::Ordering {
    use crate::keys::Content;
    use std::cmp::Ordering;
    for (k_nr, val) in key.iter().enumerate() {
        let k = &keys[k_nr];
        let p = pos + u32::from(k.position);
        let c = match (val, k.type_nr.abs()) {
            (Content::Long(v), 1 | 2) => v.cmp(&reader.i64_at(rec, p)),
            (Content::Long(v), 8) => v.cmp(&i64::from(reader.i32_at(rec, p))),
            (Content::Long(v), 9) => {
                let b = reader.resolve(u64::from(rec) * 8 + u64::from(p), 2);
                v.cmp(&i64::from(i16::from_ne_bytes([b[0], b[1]])))
            }
            (Content::Long(v), 10) => {
                let b = reader.resolve(u64::from(rec) * 8 + u64::from(p), 1);
                v.cmp(&i64::from(b[0] as i8))
            }
            (Content::Long(v), 11) => {
                let b = reader.resolve(u64::from(rec) * 8 + u64::from(p), 2);
                v.cmp(&i64::from(u16::from_ne_bytes([b[0], b[1]]) as i16))
            }
            // text key (3b.6): the field holds a `u32` pointer to a string record
            // (fld 4 = length, fld 8.. = UTF-8 bytes). Read it over the reader and
            // compare as strings. `key_hash` already hashes the `Content::Str`.
            (Content::Str(v), 6) => {
                let str_rec = reader.u32_at(rec, p);
                let s = read_reader_str(reader, str_rec);
                v.str().cmp(s.as_str())
            }
            _ => return Ordering::Greater, // unsupported key type — never match
        };
        if c != Ordering::Equal {
            return if k.type_nr < 0 { c.reverse() } else { c };
        }
    }
    Ordering::Equal
}

/// Locate the byte offsets (within the sorted vector's inner record) of the
/// entries with key in `[lo, hi]`, in a SORTED collection rooted at `(root_rec,
/// root_pos)` of the paged image — a read-only port of `vector::sorted_find`'s
/// binary search + a forward walk over [`PagedReader`]. Returns `(inner_rec,
/// positions)`; `positions` are `8 + i·esize` for each in-range element, in key
/// order. `esize` is the element size in bytes.
#[must_use]
pub fn sorted_range_positions<P: PageProvider>(
    reader: &mut PagedReader<P>,
    root_rec: u32,
    root_pos: u32,
    esize: u32,
    lo: &[crate::keys::Content],
    hi: &[crate::keys::Content],
    keys: &[crate::keys::Key],
) -> (u32, Vec<u32>) {
    use std::cmp::Ordering;
    let inner = reader.u32_at(root_rec, root_pos);
    if inner == 0 || esize == 0 {
        return (inner, Vec::new());
    }
    let length = reader.u32_at(inner, 4);
    // Binary search for the FIRST index whose key is >= lo.
    let (mut left, mut right) = (0u32, length);
    while left < right {
        let mid = left + (right - left) / 2;
        let epos = 8 + mid * esize;
        // key_compare_reader(lo, elem) == Greater  ⇔  lo > elem  ⇔  elem < lo.
        if key_compare_reader(reader, lo, inner, epos, keys) == Ordering::Greater {
            left = mid + 1;
        } else {
            right = mid;
        }
    }
    // Walk forward while elem key <= hi.
    let mut out = Vec::new();
    let mut i = left;
    while i < length {
        let epos = 8 + i * esize;
        // key_compare_reader(hi, elem) == Less  ⇔  hi < elem  ⇔  past the range.
        if key_compare_reader(reader, hi, inner, epos, keys) == Ordering::Less {
            break;
        }
        out.push(epos);
        i += 1;
    }
    (inner, out)
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
        img[40..48].copy_from_slice(&(-1_234_567_890_123i64).to_ne_bytes());
        let mut r = PagedReader::with_config(VecProvider::new(img), 16, 64);
        assert_eq!(r.u32_at(3, 4), 0xDEAD_BEEF);
        assert_eq!(r.i64_at(5, 0), -1_234_567_890_123i64);
    }
}
