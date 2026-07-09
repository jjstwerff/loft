// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN53 F1 — the mutational RAW-SOURCE program fuzzer.
//!
//! Takes libfuzzer's bytes as loft source and drives them through
//! `parse → byte_code → execute` in-process. All the logic — the three-state
//! clean/finding classifier, the F4 poison-on-free amplifier — lives in
//! `loft::fuzz_oracle` (behind the crate's `fuzzing` feature), so the code
//! FUZZED here is exactly the code the seed-corpus replay TESTS on stable
//! (`cargo test --lib fuzz_oracle`). This file is only the shim.
//!
//! A finding is a panic (or a poison-induced SIGSEGV), which libfuzzer records
//! with the crashing artifact. Contract + design:
//! `doc/claude/plans/53-program-level-fuzzing/F1-DESIGN.md`.
//!
//! Seed the corpus, then run (needs nightly + cargo-fuzz):
//!   ./fuzz/seed_program_source.sh
//!   cargo +nightly fuzz run program_source
//!   cargo +nightly fuzz run program_source -- -runs=100000   (bounded)

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    loft::fuzz_oracle::fuzz_one_source(data);
});
