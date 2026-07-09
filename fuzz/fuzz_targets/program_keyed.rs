// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN53 F2.5 — the KEYED-CONTAINER program fuzzer.
//!
//! An `arbitrary`-derived [`Input`] maps to a `loft::fuzz_keyed::KeyedSpec`,
//! which `generate_keyed` turns into a valid-by-construction, self-checking
//! `hash`/`sorted`/`index` program; `check_generated_with(.., poison = true)`
//! runs it under arena poison-on-free. All the generation + checking logic
//! lives in `loft::fuzz_keyed` (behind the crate's `fuzzing` feature), so the
//! code FUZZED here is exactly the code the F2.1–F2.4 `cargo test` sweep
//! exercises on stable — this file is only the shim.
//!
//! A finding is a returned `Err` (a failed self-check / a generator bug) turned
//! into a panic, a poison-induced SIGSEGV, or a compiler ICE — libfuzzer records
//! each with the crashing artifact. Contract + design:
//! `doc/claude/plans/53-program-level-fuzzing/F2-DESIGN.md`.
//!
//! Run (needs nightly + cargo-fuzz):
//!   cargo +nightly fuzz run program_keyed
//!   cargo +nightly fuzz run program_keyed -- -runs=100000   (bounded)

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use loft::fuzz_keyed::{Kind, KeyedSpec, check_generated_with, generate_keyed};

#[derive(Arbitrary, Debug)]
struct Input {
    kind: u8,
    n_keys: u8,
    closures: bool,
    remove: Vec<u8>,
}

fuzz_target!(|input: Input| {
    let kind = match input.kind % 3 {
        0 => Kind::Hash,
        1 => Kind::Sorted,
        _ => Kind::Index,
    };
    let n_keys = u32::from(input.n_keys % 20) + 1;
    // Remove indices are taken mod n_keys, so every byte names a real key.
    let remove = input
        .remove
        .iter()
        .map(|&b| u32::from(b) % n_keys)
        .collect();
    let spec = KeyedSpec {
        kind,
        n_keys,
        remove,
        closures: input.closures,
    };
    let src = generate_keyed(&spec);
    if let Err(msg) = check_generated_with(&src, true) {
        panic!("KEYED FINDING:\n{msg}");
    }
});
