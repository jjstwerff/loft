
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

**Test:** `tests/scripts/36-parse-errors.loft::test_spacial_not_implemented`.
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

## C53 — Match arms cannot use library enum variants

Match arms do not support `testlib::Ok` or bare `Ok` for library enums.
Only same-file enum variants work in match patterns.

**Workaround:** use if-else with `==` comparisons.
**Test:** `tests/imports.rs` (library enum tests).
**Docs:** [PLANNING.md](PLANNING.md) § C53.

---

## C54 — Integer overflow panics in debug builds

Arithmetic that produces `i32::MIN` (the null sentinel) triggers a
`debug_assert` panic in `src/ops.rs` via the `checked_int!` macro.
In release builds the result is silently null.  By design — the debug
check catches accidental sentinel collisions.

**Workaround:** stay within `i32::MIN + 1 .. i32::MAX` or use `long`.
**Test:** stress_test.loft `test_int_overflow`.

---

## Verification log

Last retested: **2026-04-11** against commit `8761101` (consilidate branch).

| Caveat | Status | How verified |
|--------|--------|-------------|
| C3 | Still applies | Design constraint — WASM has no thread pool |
| C7 | Still applies | `--tests 36-parse-errors.loft::test_spacial_not_implemented` → expected error |
| C12 | Still applies | Design constraint — no `try`/`catch` syntax |
| C38 | Still applies | `--tests 56-closures.loft::test_capture_timing` → passes |
| C45 | Still applies | Slot allocator still text-only for zone-2 reuse |
| ~~C51~~ | **Removed** | Native extensions now load via `extensions::load_all`; 15 native_loader tests pass |
| C53 | Still applies | No library-enum match test exists; workaround documented |
| C54 | **New** | Integer overflow debug panic — by design, documented |

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
