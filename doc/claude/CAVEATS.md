// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Caveats

Verifiable edge cases and limitations that affect users or block tests.
Each entry has a reproducer and the test(s) that exercise it, so a release
build can be retested quickly.

**Maintenance rules:**
- Remove an entry when the underlying issue is fully fixed and the test passes
  without workarounds.
- Keep entries short — this is a quick-lookup document for release retesting.

---

## C3 — WASM backend: threading not implemented

`par(...)` loops are sequential in the WASM build.  Threading requires a
Web Worker pool (W1.18, targeted 1.1+).

**Workaround:** use the interpreter instead of `--native-wasm`.

---

## C7 — `spacial<T>` not implemented

The spatial index collection type is declared but all operations panic at
runtime.  A compile-time error is emitted for basic usage.

**Test:** `tests/scripts/36-parse-errors.loft::test_spacial_type_error`.
**Planned fix:** A4 (1.1+).

---

## C12 — No exception handling

Runtime errors from `assert` and `panic` abort the program.  No `try`/`catch`.

**Workaround:** validate inputs; use `FileResult` for file I/O errors.

---

## C38 — Closure capture is copy-at-definition-time

Captured values are copied into the closure at definition time.  Mutations
after capture are not visible inside the lambda (and vice versa).  By design
(value semantics, like Rust `move`).

**Test:** `tests/scripts/56-closures.loft::test_capture_timing`.

---

## C45 — Zone-2 slot reuse limited to Text-only

Reference and Vector variables cannot reuse dead zone-2 slots due to
IR-walk ordering and block-return frame sharing.  Only text-to-text reuse
works.  No correctness impact — just wastes some stack space.

---

## C51 — Libraries: no native extension loading

Libraries are pure `.loft` files.  The `loft.toml` `native = "..."` field
is parsed but native shared libraries are not loaded at runtime.

---

## C53 — Match arms cannot use library enum variants

Match arms do not support `testlib::Ok` or bare `Ok` for library enums.
Only same-file enum variants work in match patterns.

**Workaround:** use if-else with `==` comparisons.
**Test:** `tests/imports.rs` (library enum tests).
**Docs:** [PLANNING.md](PLANNING.md) § C53.

---

## C54 — `file.lines()` drops content without trailing newline

A file with content but no trailing `\n` returns 0 lines from `lines()`.

**Workaround:** ensure files end with `\n`, or use `f#read(n) as text`.
**Test:** `tests/scripts/71-caveats-problems.loft::test_c54_file_lines_no_trailing_newline` (`@EXPECT_FAIL`).

---

## C55–C58 — Fixed (2026-04-02)

- **C55** `rev(vector)` now works — parser accepts plain vectors for reverse iteration.
- **C56** Format `:<`/`:^` now works for integers, longs, and floats.
- **C57** Float `:.0` precision now correctly rounds to zero decimals.
- **C58** Empty struct comprehension no longer crashes the compiler.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
