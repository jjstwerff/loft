// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 Phase 1 — the `deliver` boundary parity gate. `deliver(tag, value)` hands
// a live value's descriptor handle to the loopback host, which reconstructs the
// value via its layout descriptor (@PLN105 Phase 0) and prints it. Each cell runs
// under BOTH the interpreter and `--native` and asserts byte-identical stdout AND
// the exact expected bytes (so the cell is non-vacuous). Proves the handle reaches
// the host correctly and identically across the loft-call boundary on both backends
// — the "read deep into record + vector without knowing the layout" capability.

mod common;
extern crate loft;

use common::cross_mode::run_cross_mode_expect;

/// A flat struct: two integers + text. Bytes (declaration order, little-endian):
/// x=7 (i64) | y=20 (i64) | "hi".
#[test]
fn deliver_flat_struct_parity() {
    run_cross_mode_expect(
        "deliver_flat_struct",
        r#"
        struct P { x: integer, y: integer, label: text }
        fn test() {
          p = P { x: 7, y: 20, label: "hi" };
          deliver(1, p);
        }
        "#,
        "deliver tag=1 type=P bytes=070000000000000014000000000000006869",
    );
}

/// A struct with a nested (inline) struct and an inline scalar vector — the deep
/// walk. Bytes: "hi" | inner.a=7 | inner.b=9 | nums 10,20,30 | flag=1.
#[test]
fn deliver_nested_record_and_vector_parity() {
    run_cross_mode_expect(
        "deliver_nested",
        r#"
        struct Inner { a: integer, b: integer }
        struct Outer { label: text, inner: Inner, nums: vector<integer>, flag: boolean }
        fn test() {
          o = Outer { label: "hi", inner: Inner { a: 7, b: 9 }, nums: [10, 20, 30], flag: true };
          deliver(2, o);
        }
        "#,
        "deliver tag=2 type=Outer \
         bytes=6869070000000000000009000000000000000a0000000000000014000000000000001e0000000000000001",
    );
}
