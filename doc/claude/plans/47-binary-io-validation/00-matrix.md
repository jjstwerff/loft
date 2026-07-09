<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN47 phase 00 — matrix freeze + harness wiring

**Goal of this phase:** freeze the cell grid, stand up
`tests/binary_io_matrix.rs` (reusing the @PLAN14 `cross_mode!`
harness), land one smoke cell, and run the pre-flight survey.

**RESULT (2026-07-09): the whole matrix is GREEN on both backends.** The
harness landed 32 cross-mode cells; building it surfaced the W2–W4/W9/W10
bugs that the pre-flight survey had wrongly predicted as shipped (see the
[README](README.md) fix table).

## The grid (measured)

Rows = value type (W0–W11), columns = format (F1/F2/F3) × access
pattern (A1–A4).  Each cell is one of:

- ✅ — round-trips, interp == native, has a `cross_mode!` test
- ❌ — fails today (write wrong bytes / read wrong width / unimplemented)
- N/A — combination not meaningful (e.g. binary scalar in TextFile)

### Scalars (W0–W6) — all ✅ (interp == native)

| Type | F1 LE | F2 BE | A1 append | A2 offset | A3 trunc | A4 sync |
|---|---|---|---|---|---|---|
| W0 `integer` i64 (8B) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| W1 `i32` (4B) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| W1 `u32` (4B) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| W2 `i16`/`u16` (2B) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| W3 `i8`/`u8`/`bool` (1B) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| W4 `character` (4B) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| W5 `float` f64 (8B) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| W6 `single` f32 (4B) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

Signed narrow ints (`i8`/`i16`) sign-extend on read (fixed this pass — interp
had zero-extended, diverging from native).  `u32`/`i32` ≥ 2³¹ round-trip via
raw bytes but read back as negative i64 in expressions (@P293 caveat).

### Variable-width (W7–W11)

| Type | F1 LE | F2 BE | F3 Text | Notes |
|---|---|---|---|---|
| W7 `text` | ✅ | ✅ | ✅ | explicit-count `f#read(N) as text` |
| W8 `vector<scalar>` | ✅ | ✅ | N/A | `f#read(N)` counts N **bytes**; warns when N ∤ element width |
| W9 `struct` scalars | ✅ | ✅ | N/A | per-field walk both directions; leaks 1 record/read (pre-existing block-temp ownership bug, tracked separately) |
| W10 `struct` w/ text/vec field | ⛔ | ⛔ | N/A | **rejected at compile time** (variable-width field) — by design, not a gap |
| W11 nested plain `struct` | ✅ | ✅ | N/A | rides the W9 per-field walk |

## Harness

`tests/binary_io_matrix.rs`: each cell is a loft snippet that writes a
known value to a temp file, reopens, reads it back, and prints the
read value.  The `cross_mode!` macro runs the snippet under both
interp and native and asserts byte-identical stdout.  Temp paths are
per-test (`test_loft_biom_<cell>.bin`) and cleaned up.

Cell skeleton (LittleEndian, W1 u32, A4 sync):

```loft
fn main() {
  delete("test_loft_biom_w1_u32.bin");
 {f = file("test_loft_biom_w1_u32.bin");
  f#format = LittleEndian;
  f += (0x1ECEC0DE as u32);   // high bit clear — avoids the u32→i64
  f.sync();                   // signed-display caveat (see @P293)
 }
 {f = file("test_loft_biom_w1_u32.bin");
  f#format = LittleEndian;
  v: u32 = f#read as u32;
  assert(v == (0x1ECEC0DE as u32), "u32 LE round-trip");
  assert(f#size == 4, "u32 wrote exactly 4 bytes");
 }
  delete("test_loft_biom_w1_u32.bin");
  println("w1_u32_le_sync ok");
}
```

## Pre-flight survey

Run one cell per W-row at F1/A1 before building phases 02–05.  Record
the pass/fail here so the later phases know which cells are lock-in
(already pass) vs feature-build (currently fail).  Prediction:

- W0–W6: ✅ (scalar width/endian is correct after @P293 / @P284).
- W7: ❌ — `f#read as text` has no length to read.
- W8: ❌ — no count convention.
- W9–W11: ❌ — `f += struct` writes the handle, `f#read as Struct`
  unimplemented.

If the prediction holds, the plan is well-scoped: phase 01 is pure
lock-in, phases 02–05 are the P289 build-out.

## Known caveats to encode as matrix notes

- **u32 / i32 signed range** (@P293): values ≥ 2³¹ round-trip via raw
  bytes but read back as negative i64 in loft expressions.  Test with
  high-bit-clear values; document the caveat for high-bit-set.
- **`f += <integer>` width warning**: a bare `integer` (no cast) writes
  8 bytes and warns for binary formats.  Matrix cells use explicit
  casts; the warning itself is correct behaviour, not a failure.
- **Append vs offset** (CHANGELOG 0.8.6): A1 appends, A2 (`f#next = N`)
  overwrites at N.  A3 (`set_file_size(0)`) is the snapshot-replace
  idiom — cells that re-use a path must truncate first.

## Exit criteria for phase 00

- `tests/binary_io_matrix.rs` compiles and the smoke cell passes on
  both backends.
- Grid above filled with the pre-flight survey results.
- No production code touched.
