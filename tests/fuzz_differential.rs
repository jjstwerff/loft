// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN53 F3.1 — differential `--interpret` ≡ `--native` on GENERATED keyed
//! programs.
//!
//! Each spec becomes a printing program (`generate_keyed_summary`: builds the
//! collection, prints the canonical population + ordered `key=value;` summary —
//! the F3 deterministic-output subset), run on both backends via the shared
//! `run_cross_mode` helper, which asserts both succeed AND their normalised
//! stdout is byte-identical. A divergence is a cross-backend codegen finding.
//!
//! Feature-gated (needs `loft::fuzz_keyed`) and `#[ignore]` (native shells out
//! to rustc per program — heavy). Run with:
//!   cargo test --features fuzzing --test fuzz_differential -- --ignored --nocapture
//!
//! Design: `doc/claude/plans/53-program-level-fuzzing/F3-DESIGN.md`.

#![cfg(feature = "fuzzing")]

mod common;

use loft::fuzz_keyed::{KeyedSpec, Kind, generate_keyed_summary};

#[test]
#[ignore = "F3.1 differential (rustc per program) — run with --features fuzzing --ignored"]
fn keyed_generated_agree_across_backends() {
    // A small curated corpus — bounded because native shells out to rustc per
    // program (the coverage cap F3-DESIGN.md § failure path 3 names). Covers
    // all three types × a few sizes × remove patterns, including an emptied
    // collection.
    let corpus: &[KeyedSpec] = &[
        spec(Kind::Hash, 5, &[]),
        spec(Kind::Hash, 6, &[2]),
        spec(Kind::Hash, 8, &[7]),
        spec(Kind::Sorted, 5, &[]),
        spec(Kind::Sorted, 7, &[0, 3]),
        spec(Kind::Sorted, 3, &[0, 1, 2]), // emptied
        spec(Kind::Index, 5, &[]),
        spec(Kind::Index, 6, &[1, 4]),
    ];

    for (i, s) in corpus.iter().enumerate() {
        let body = generate_keyed_summary(s);
        // run_cross_mode appends `fn main() { test(); }`, runs both backends,
        // and panics on failure or a normalised-stdout divergence.
        common::cross_mode::run_cross_mode(&format!("f31_keyed_{i}"), &body);
    }
    eprintln!(
        "F3.1: {} generated keyed programs agree interp==native",
        corpus.len()
    );
}

fn spec(kind: Kind, n_keys: u32, remove: &[u32]) -> KeyedSpec {
    KeyedSpec {
        kind,
        n_keys,
        remove: remove.to_vec(),
        closures: false,
    }
}
