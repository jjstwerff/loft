// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Integration tests for the time library functions:
//   now()    — milliseconds since Unix epoch (wall clock)
//   ticks()  — microseconds since program start (monotonic)

extern crate loft;

mod testing;

/// now() returns a positive long well above the year-2000 epoch.
#[test]
fn now_is_positive() {
    code!(
        "fn test() {
    t = now();
    assert(t > 946684800000l, \"now() too small: {t}\");
}"
    );
}

/// now() is non-null (not i64::MIN).
#[test]
fn now_is_not_null() {
    code!(
        "fn test() {
    t = now();
    assert(t != null, \"now() returned null\");
}"
    );
}

/// ticks() is non-negative.
#[test]
fn ticks_is_non_negative() {
    code!(
        "fn test() {
    t = ticks();
    assert(t >= 0, \"ticks() negative: {t}\");
}"
    );
}

/// Two successive calls to ticks() return non-decreasing values.
#[test]
fn ticks_is_monotonic() {
    code!(
        "fn test() {
    t1 = ticks();
    t2 = ticks();
    assert(t2 >= t1, \"ticks() went backwards: {t1} then {t2}\");
}"
    );
}
