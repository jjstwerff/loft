
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

## C12 — No exception handling (by design)

Loft uses null returns + `??` coalescing instead of exceptions.  Fallible
operations return null; callers handle with `??`, `!`, or `if`.  `assert`
and `panic` are for bugs, not expected failures.  Production mode logs
asserts instead of aborting.

**Pattern:** `value = fallible_call() ?? default;`

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

## ~~C53~~ — Match arms with library enum variants — FIXED

Match arms now accept bare (`Yay`), enum-qualified (`Status::Yay`), and
library-qualified (`enumlib::Yay`) variant names.  When the bare name is
not visible in the current source, the parser falls back to searching
the matched enum's `children_of` by name.
**Test:** `tests/imports.rs::match_accepts_library_enum_variants`.

---

## C54 — Integer overflow panics in debug builds

Arithmetic that produces `i32::MIN` (the null sentinel) triggers a
`debug_assert` panic in `src/ops.rs` via the `checked_int!` macro.
In release builds the result is silently null.  By design — the debug
check catches accidental sentinel collisions.

**Workaround:** stay within `i32::MIN + 1 .. i32::MAX` or use `long`.
**Test:** stress_test.loft `test_int_overflow`.

---

## C57 — Nested file-scope-only declarations rejected with a clear diagnostic

Putting `struct`, `enum`, `type`, `interface`, `use`, `pub`, or a named
`fn name(...)` inside a function body produces a single, clear diagnostic:

```
Error: 'struct' definitions must be at file scope, not inside a function
       or block at file.loft:2:9
```

Previously the same code triggered a cascade of confusing errors like
`Expect token =`, `Expect constants to be in upper case`, and
`Syntax error: unexpected ...`.

Lambdas (`fn(args) { body }`) are unchanged — they parse as expressions
so the file-scope check does not fire.

**Workaround:** move the declaration to file scope.
**Tests:** `tests/parse_errors.rs::p85c_struct_inside_fn_emits_diagnostic`
plus three siblings (`_enum_`, `_named_fn_`, `_lambda_inside_fn_still_works`).

---

## C56 — Naming a user definition after a stdlib symbol is rejected with a clear diagnostic

Defining a user `enum`, `struct`, `type`, or top-level constant whose
identifier collides with a stdlib symbol (e.g. `E` from `pub E = OpMathEFloat()`,
`PI`, `TAU`) produces a diagnostic naming the conflicting definition's
location:

```
Error: enum 'E' conflicts with a constant of the same name already defined
       at default/01_code.loft:383:24 — pick a different name
```

Previously the same code crashed the compiler — `enum`/`struct` panicked
with `Cannot change returned type on [164]E to float twice was E`, and
`type`/constant panicked with `Dual definition of E`.

**Workaround:** rename the user definition (e.g. `MyE`, `Status`).
**Tests:** `tests/parse_errors.rs::p85b_enum_shadowing_stdlib_constant_emits_diagnostic`
plus three siblings (`_struct_`, `_type_`, `_constant_`).

---

## ~~C55~~ — Interface method in for-loop on struct vector (P136) — FIXED

Fixed: `type_element_size` now computes struct field size from attributes,
and `subst_type` preserves deps during generic specialisation.
**Test:** `tests/scripts/86-interfaces.loft::test_bounded_for_loop_struct`.

---

## Verification log

Last retested: **2026-04-12** against commit `d5c20fd` (main branch).

| Caveat | Status | How verified |
|--------|--------|-------------|
| C3 | Still applies | Design constraint — WASM has no thread pool |
| C7 | Still applies | `--tests 36-parse-errors.loft::test_spacial_not_implemented` → expected error |
| C12 | Still applies | Design choice — null + `??` instead of exceptions |
| C38 | Still applies | `--tests 56-closures.loft::test_capture_timing` → passes |
| C45 | Still applies | Slot allocator still text-only for zone-2 reuse |
| ~~C51~~ | **Removed** | Native extensions now load via `extensions::load_all`; 15 native_loader tests pass |
| ~~C53~~ | **Fixed** | `tests/imports.rs::match_accepts_library_enum_variants` covers bare/qualified library variants |
| C54 | Still applies | Integer overflow debug panic — by design |
| ~~C55~~ | **Fixed** | P136 — `type_element_size` + deps preservation |
| C56 | Documented | Stdlib name collisions now emit a clean diagnostic instead of panicking |
| C57 | Documented | Nested file-scope keywords now emit a single diagnostic instead of cascading errors |

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
