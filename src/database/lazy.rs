// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN129 arc B — the SOURCE seam: what a lazy collection consults on a miss.
//
// Arc A had one source (a `.store` image) and reached it inline from
// `fetch_missing`. Arc B adds a second (a relational database), and the two
// differ in every detail except the question they answer — so the question is
// what lives here, and the details live in one implementation each.

use crate::database::Stores;
use crate::keys::{Content, DbRef};

/// What a lazy collection is bound to, derived from the binding's source string.
///
/// Derived rather than declared: `store_bind_lazy` takes one string, and which
/// kind of source it names is a fact about the string. That keeps the loft
/// surface at one function and lets a new source arrive without a new builtin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LazySource {
    /// A `.store` image — a local path or an `http(s)` URL read by page
    /// (`REMOTE_STORES.md`). loft's own bytes, so a fetch is a range read.
    File(String),
}

impl LazySource {
    /// Classify a binding's source string.
    ///
    /// Everything is a file today, which is exactly arc A's behaviour: the
    /// seam's whole claim at this step is that it changes nothing.
    #[must_use]
    pub fn of(source: &str) -> LazySource {
        LazySource::File(source.to_string())
    }
}

/// The answer a source gives to *"fetch the entry for this key"* — three
/// outcomes, because two of them are the reason arc C exists.
///
/// `Absent` and `Unreachable` are the same `null` to a loft program and must
/// never be the same fact here: "no such person" is stable and true, "the source
/// is down" is neither (README failure path 1).
#[derive(Debug)]
pub enum Fetched {
    /// The entry is now IN the collection. The caller re-runs the lookup rather
    /// than trusting a ref from here — the collection stays the only authority
    /// on what is resident.
    Inserted,
    /// The source was reached and does not hold this key.
    Absent,
    /// The source could not be consulted; the string says why, and it is what a
    /// program reads back through `store_lazy_error`.
    Unreachable(String),
}

impl Stores {
    /// Ask one source for one key. The dispatch that makes the seam a seam.
    #[cfg(paged_store)]
    pub(crate) fn fetch_from_source(
        &mut self,
        data: &DbRef,
        src: &LazySource,
        key: &[Content],
    ) -> Fetched {
        match src {
            LazySource::File(path) => self.fetch_from_file(path, data, key),
        }
    }

    /// The `.store` image source — arc A's fetch, unchanged.
    ///
    /// The order is load-bearing and predates the seam: TRY the fetch, and only
    /// on failure ask whether the source was reachable at all. Asking first
    /// would put a stat on the path of every successful fault.
    #[cfg(paged_store)]
    fn fetch_from_file(&mut self, path: &str, data: &DbRef, key: &[Content]) -> Fetched {
        // One key, whichever spelling it arrived in. A composite key is not
        // fetchable yet — @PLN125's associated types are what would carry it —
        // so it is left to answer absent rather than fetching the wrong row.
        let hit = match key {
            [Content::Long(k)] => self.load_key(data, path, *k),
            [Content::Str(s)] => self.load_key_text(data, path, s.str()),
            _ => false,
        };
        if hit {
            return Fetched::Inserted;
        }
        match crate::paged_reader::PageSource::open(path) {
            Err(reason) => Fetched::Unreachable(reason),
            // Reached it and the key was not there — a genuine absence.
            Ok(_) => Fetched::Absent,
        }
    }
}
