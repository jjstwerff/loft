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
  Fix designs, root-cause analysis, and implementation plans belong in
  [PLANNING.md](PLANNING.md), [PROBLEMS.md](PROBLEMS.md), or the relevant
  design doc.  Each entry here should have at most: one-line description,
  reproducer, test reference, workaround, and a pointer to where the fix is
  planned.

---

## C1 — Lambda closure capture: cross-scope restriction (A5.6)

Same-scope closure capture (all types — integers, text, mutable) works (A5.6a–c ✓).
One restriction remains: **cross-scope closures**, where a capturing lambda is
returned from a function and called from a different scope.

Root cause (two parts, detailed design in PLANNING.md § A5.6):

1. **`Type::Function` is 4 bytes (d_nr only).** When `make_greeter` returns the inner
   lambda, only the 4-byte d_nr fits in the return slot; the closure record's 12-byte
   DbRef is lost.  Fix: extend `Type::Function` to 16 bytes; `fn_call_ref` reads the
   embedded DbRef and pushes it as `__closure` at runtime.

2. **Parser does not handle `expr(args)` chained calls.** `parse_part` only loops on
   `.` and `[`; `make_greeter("Hello")("world")` parses the `("world")` as a separate
   expression rather than a chained call.  Fix: extend `parse_part` to handle `(` when
   the current type is `Type::Function`.

**Note:** Capture-at-definition-time is not guaranteed — `closure_capture_after_change`
shows `x = 10; f = fn(y) { x + y }; x = 99; f(5)` returns 104.

**Reproducer:**
```loft
fn make_greeter(prefix: text) -> fn(text) -> text {
    fn(name: text) -> text { "{prefix} {name}" }
}
make_greeter("Hello")("world")  // should be "Hello world" — crashes/wrong output
```

**Tests:** `closure_capture_integer` / `closure_capture_multiple` / `closure_capture_after_change` / `closure_capture_text_integer_return` / `closure_capture_text_return` (all pass); `closure_capture_text` (`#[ignore]` — A5.6 cross-scope, 0.8.3).
**Workaround:** pass captured values as explicit function arguments.
**Planned fix:** A5.6 in [PLANNING.md](PLANNING.md) and [ROADMAP.md](ROADMAP.md) (0.8.3) — 16-byte fn-ref + parser `parse_part` fix.
**Docs:** [LOFT.md](LOFT.md) § Lambda expressions.

---

## C3 — WASM backend: threading not implemented

The `--native-wasm` backend currently lacks support for threading.
File I/O, random numbers, time functions, and dynamic function references (`CallRef`) are
now all implemented (W1.15, W1.16, W1.17, W1.19, W1.20 — all 0.8.3).

**Affected files:** `tests/wrap.rs` — `WASM_SKIP` array:

| File | Reason |
|------|--------|
| `19-threading.loft` | WASM threading model differs; W1.18 not yet landed |

**Workaround:** use the interpreter (`cargo run --bin loft`) instead of `--native-wasm`.
**Remaining work:** W1.18 (threading) in [ROADMAP.md](ROADMAP.md) (0.8.3).

---

## C7 — `spacial<T>` not implemented

The spatial index collection type is declared but all operations panic at
runtime.  A compile-time error is emitted for basic usage, but edge cases
may still reach the runtime panics.

**Test:** `tests/scripts/36-parse-errors.loft` — `@EXPECT_ERROR` for spacial.
**Planned fix:** A4 (spatial index operations), targeted for 1.1+.
**Docs:** [PROBLEMS.md](PROBLEMS.md) § Issue 22.

---

## C12 — No exception handling

Runtime errors from `assert` and `panic` abort the program. There is no
`try`/`catch` or `Result` mechanism for structured error recovery.

**Reproducer:**
```loft
fn main() {
  // This aborts — no way to catch it:
  assert(false, "deliberate failure");
  // This line is never reached.
}
```

**Test:** none (cannot test abort from inside loft).
**Workaround:** validate inputs before operations; use `FileResult` for file I/O errors.
**Docs:** [00-vs-python.html](../00-vs-python.html) § No exception handling.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [SAFE.md](SAFE.md) — safety analysis for parallel workers and coroutines
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
