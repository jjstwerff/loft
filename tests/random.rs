// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Integration tests for the random-number library functions:
//   rand(lo, hi)       — uniform integer in [lo, hi]
//   rand_seed(seed)    — reproducible sequences
//   rand_indices(n)    — shuffled [0..n-1]

extern crate loft;

mod testing;

// ---------------------------------------------------------------------------
// rand()
// ---------------------------------------------------------------------------

/// rand() should stay within [lo, hi].
#[test]
fn rand_in_range() {
    code!(
        "fn test() {
    rand_seed(42);
    ok = true;
    for i in 0..200 {
        r = rand(10, 20);
        if r < 10 || r > 20 { ok = false }
    }
    assert(ok, \"rand out of range\");
}"
    );
}

/// rand() with lo == hi always returns lo.
#[test]
fn rand_single_value() {
    code!("fn test() { assert(rand(7, 7) == 7, \"single value\"); }");
}

/// rand() with lo > hi returns null.
#[test]
fn rand_invalid_range() {
    code!("fn test() { assert(!rand(10, 5), \"invalid range should be null\"); }");
}

/// rand_seed() produces a reproducible sequence.
#[test]
fn rand_seed_reproducible() {
    code!(
        "fn test() {
    rand_seed(99);
    a = rand(0, 1000);
    b = rand(0, 1000);
    rand_seed(99);
    assert(rand(0, 1000) == a, \"first value must match\");
    assert(rand(0, 1000) == b, \"second value must match\");
}"
    );
}

/// Different seeds produce different sequences (extremely unlikely to collide).
#[test]
fn rand_different_seeds() {
    code!(
        "fn test() {
    rand_seed(1);
    a = rand(0, 1000000);
    rand_seed(2);
    b = rand(0, 1000000);
    assert(a != b, \"different seeds should give different values\");
}"
    );
}

// ---------------------------------------------------------------------------
// rand_indices()
// ---------------------------------------------------------------------------

/// rand_indices(n) returns a vector of length n.
#[test]
fn rand_indices_length() {
    code!(
        "fn test() {
    rand_seed(7);
    v = rand_indices(10);
    assert(len(v) == 10, \"size {len(v)}\");
}"
    );
}

/// rand_indices(n) contains each value in [0, n-1] exactly once.
#[test]
fn rand_indices_permutation() {
    code!(
        "fn test() {
    rand_seed(42);
    n = 20;
    v = rand_indices(n);
    assert(len(v) == n, \"wrong size\");
    // check every value 0..n appears exactly once
    for expected in 0..n {
        found = false;
        for x in v { if x == expected { found = true } }
        assert(found, \"missing {expected}\");
    }
}"
    );
}

/// rand_indices(0) returns an empty vector.
#[test]
fn rand_indices_zero() {
    code!(
        "fn test() {
    v = rand_indices(0);
    assert(len(v) == 0, \"expected empty vector\");
}"
    );
}

/// rand_indices produces different orderings with different seeds.
#[test]
fn rand_indices_different_orderings() {
    code!(
        "fn test() {
    rand_seed(1);
    a = rand_indices(10);
    rand_seed(9999);
    b = rand_indices(10);
    // At least one position should differ (not a mathematical guarantee
    // for n=10, but for these seeds it reliably holds).
    diff = false;
    for i in 0..10 { if a[i] != b[i] { diff = true } }
    assert(diff, \"expected different orderings\");
}"
    );
}

/// rand_indices with seeded RNG produces a reproducible order.
#[test]
fn rand_indices_reproducible() {
    code!(
        "fn test() {
    rand_seed(123);
    a = rand_indices(8);
    rand_seed(123);
    b = rand_indices(8);
    same = true;
    for i in 0..8 { if a[i] != b[i] { same = false } }
    assert(same, \"seeded rand_indices should be reproducible\");
}"
    );
}
