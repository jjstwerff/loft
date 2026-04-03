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

## C54 — Exponentiation `** 0.5` *(FIXED)*

**Fixed in Sprint 8:** Added `**` to lexer TOKENS and parser OPERATORS table,
mapped to `pow()` via `rename()`.  `x ** 0.5`, `x ** 2.0`, etc. all work.
**Test:** `tests/scripts/77-ignored-exponentiation.loft` (5 passing tests).

---

## C55 — Struct return with inline vector literal *(FIXED)*

**Fixed by P104:** The "Unknown definition" error was caused by the test
runner executing library functions (like `mat4_identity()`) as tests.
Direct struct return with vector literal works correctly.
**Test:** `tests/scripts/76-ignored-struct-vector-return.loft::test_p104_direct_return`.

---

## C56 — Flat namespace requires split test files *(FIXED)*

**Fixed by P104:** The test runner now filters library functions by source
file.  Tests and library functions no longer collide.  All math tests
consolidated in a single file (9 tests).
**Test:** `lib/graphics/tests/math.loft`.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
