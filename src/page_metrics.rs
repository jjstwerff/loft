// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)

//! The counter the layout measurements are fed to — @PLN134, @PLN136.
//!
//! A query over a persisted image is cheap or expensive for one reason: how many
//! distinct PAGES the bytes it reads fall on. Everything here turns a set of touched
//! byte offsets into that number, and does it the same way for every question asked
//! of it.
//!
//! **One counter, because the answers are compared to each other.** @PLN134 chose a
//! node layout by ranking five candidates; @PLN136 asks whether a bounding-box walk
//! over the same tree reaches the same place. A second counter that bucketed
//! straddling records differently, or sampled a different percentile, would make
//! those two numbers look comparable while measuring different things — and the
//! whole point of the second plan is the comparison.
//!
//! Test-only: it exists to answer design questions, and nothing in the shipped
//! runtime counts pages.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::collections::BTreeSet;

/// The page a remote reader fetches — `PagedReader`'s default.
pub(crate) const PAGE: u64 = 64 * 1024;
/// The finer granularity the same touch set is bucketed at. A descent reads a
/// handful of 16-byte nodes; what that costs is decided by the fetch size, not by
/// the walk, and one page size cannot show that.
pub(crate) const FINE: u64 = 4096;
/// `PagedReader`'s default cache, in pages — what a warm session keeps.
pub(crate) const CACHE_PAGES: usize = 64;

/// A candidate node numbering: where each node would sit if a persist-time pass
/// wrote the array in this order.
///
/// It maps a RECORDED offset to the offset it would have had, so one walk of the
/// tree evaluates a layout — no image is rewritten to ask what one costs.
pub(crate) struct Layout {
    pub(crate) name: &'static str,
    rank: Vec<u32>,
}

impl Layout {
    pub(crate) fn new(name: &'static str, order: &[u32], nodes: u32) -> Layout {
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

    /// Ids as handed out, so every number below has its own baseline rather than a
    /// separately computed one.
    pub(crate) fn as_built(nodes: u32) -> Layout {
        Layout::new("as built", &(1..=nodes).collect::<Vec<_>>(), nodes)
    }

    /// Offset -> node id -> rank -> the offset it WOULD have had.
    pub(crate) fn moved(&self, off: u32) -> u64 {
        let id = (off - 24) / 16 + 1;
        let within = u64::from((off - 24) % 16);
        match self.rank.get(id as usize) {
            Some(&r) if r != u32::MAX => 24 + 16 * u64::from(r) + within,
            _ => u64::from(off),
        }
    }

    pub(crate) fn pages(&self, touched: &BTreeSet<u32>, page: u64) -> usize {
        touched
            .iter()
            .map(|o| self.moved(*o) / page)
            .collect::<BTreeSet<u64>>()
            .len()
    }
}

/// Every candidate numbering of `tree`, in the order they are worth reading.
pub(crate) fn layouts(store: &crate::store::Store, tree: u32, nodes: u32) -> Vec<Layout> {
    use crate::radix_tree as rt;
    vec![
        Layout::as_built(nodes),
        Layout::new("BFS", &rt::bfs_order(store, tree), nodes),
        Layout::new("DFS", &rt::dfs_order(store, tree), nodes),
        Layout::new("key order", &rt::key_order(store, tree), nodes),
        Layout::new("vEB", &rt::veb_order(store, tree), nodes),
    ]
}

/// One printed row: each item formatted and appended in turn.
///
/// `map(format!).collect::<String>()` is the obvious spelling and allocates per
/// cell; one helper keeps the tables reading the same way.
pub(crate) fn row<T>(items: impl IntoIterator<Item = T>, cell: impl Fn(T) -> String) -> String {
    items.into_iter().fold(String::new(), |mut s, i| {
        s.push_str(&cell(i));
        s
    })
}

pub(crate) fn mean(v: &[usize]) -> f64 {
    v.iter().sum::<usize>() as f64 / v.len().max(1) as f64
}

pub(crate) fn pct(v: &[usize], p: f64) -> usize {
    if v.is_empty() {
        return 0;
    }
    let mut s = v.to_vec();
    s.sort_unstable();
    s[((s.len() - 1) as f64 * p) as usize]
}

/// Add the pages a byte span covers. A record straddles a boundary as often as not,
/// and counting only its first page understates every layout equally — which is
/// worse than it sounds, because the comparison is the point.
pub(crate) fn add_span(pages: &mut BTreeSet<u64>, off: u64, len: u64, page: u64) {
    for p in (off / page)..=((off + len.saturating_sub(1)) / page) {
        pages.insert(p);
    }
}

/// What one session FETCHES, against a reader that keeps `CACHE_PAGES` pages.
///
/// Every cold-query number says what the first request costs. A real session issues
/// a run of related queries — the keystrokes of a word, the steps of a pan — and
/// they share the top of the tree, so what a user waits for is the MARGINAL fetch.
///
/// The cache is per session: one user's run starts cold. The steady state across
/// many sessions is kinder still, and this is the honest side.
pub(crate) struct SessionCache {
    lru: Vec<u64>,
}

impl SessionCache {
    pub(crate) fn new() -> SessionCache {
        SessionCache {
            lru: Vec::with_capacity(CACHE_PAGES),
        }
    }

    /// How many of `want` were not already resident, taking them all.
    pub(crate) fn fetch(&mut self, want: &BTreeSet<u64>) -> usize {
        let mut fetched = 0;
        for &p in want {
            if let Some(at) = self.lru.iter().position(|&c| c == p) {
                let p = self.lru.remove(at);
                self.lru.push(p);
            } else {
                fetched += 1;
                if self.lru.len() == CACHE_PAGES {
                    self.lru.remove(0);
                }
                self.lru.push(p);
            }
        }
        fetched
    }
}
