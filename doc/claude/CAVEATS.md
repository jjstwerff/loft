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

## C3 — WASM backend: threading not implemented

The `--native-wasm` backend currently lacks support for threading.
File I/O, random numbers, time functions, and dynamic function references (`CallRef`) are
now all implemented (W1.15, W1.16, W1.17, W1.19, W1.20 — all 0.8.3).

**Affected files:** `tests/wrap.rs` — `WASM_SKIP` array:

| File | Reason |
|------|--------|
| `19-threading.loft` | WASM threading model differs; W1.18 not yet landed |

**Previously skipped — now passing:**

| File | Fixed by |
|------|----------|
| `06-function.loft` | W1.15 — `output_call_ref` dispatch table |
| `13-file.loft` | W1.16 — `OpDelete`/`OpMoveFile`/`OpMkdir`/`OpMkdirAll` in `codegen_runtime` |
| `18-locks.loft` | W1.17 — lock functions in `CODEGEN_RUNTIME_FNS` |
| `21-random.loft` | W1.19 — WASM `rand`/`rand_indices` bridge |
| `22-time.loft` | W1.20 — `host_time_now()` via `std::time::SystemTime` |

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

## C30 — Lambda re-definition leaks the old closure record

Reassigning a variable that holds a capturing lambda does not free the
previous closure.  The old closure's store record is orphaned.

**Reproducer:**
```loft
fn test() {
    x = 10;
    f = fn(y: integer) -> integer { x + y };
    // f now holds closure with x=10
    x = 20;
    f = fn(y: integer) -> integer { x + y };
    // old closure leaked — new closure overwrites fn-ref slot
}
```

**Impact:** SIGSEGV in debug builds; passes in release (leak only).  The crash
is from the work-var system: two DbRef copies (fn-ref slot + closure work-var)
point to the same store; freeing either one leaves the other dangling.
**Test:** `closure_redefine_frees_old` (`#[ignore]` in `tests/expressions.rs`).
**Workaround:** avoid reassigning lambda variables that capture values.
**Planned fix:** A5.6 deferred item 1 in [PLANNING.md](PLANNING.md) (0.8.3).
Requires rethinking closure ownership — either the fn-ref slot or the work-var
should own the closure, not both.

---

## C31 — Closures in collections or struct fields not supported

Storing a capturing lambda in a `vector<fn(...)>` or as a struct field
does not work.  Two issues: (1) `element_store_size` now correctly returns
16 for `Type::Function` (fixed), but (2) the parser's vector literal code
requires a database type for element registration, and `Type::Function` has
no database type → assertion "Unknown type" at parse time.

**Workaround:** pass closures as function arguments or return values, not
through collections or struct fields.
**Planned fix:** A5.6 deferred item 2 in [PLANNING.md](PLANNING.md) (1.1+).

---

## C32 — Captured parameter "never read" warning is false for cross-scope closures

When a function parameter is captured by a closure that is returned from the
function, the parameter IS read (by the capture) but the use-analysis does not
track `SetText` on a closure record as a read.  The `Variable.captured` flag
suppresses the "never read" warning, but the dead-assignment analysis still
does not see the capture as a use.

**Reproducer:**
```loft
fn make_greeter(prefix: text) -> fn(text) -> text {
    fn(name: text) -> text { "{prefix} {name}" }
}
// No warning (suppressed by captured flag).
// But: x = "a"; f = fn() { x }; x = "b";
// Dead-assignment warning for x="a" fires even though it was captured.
```

**Impact:** cosmetic — the dead-assignment warning is arguably correct (the
capture happens before the overwrite, and the captured value is `"a"` not `"b"`).
**Planned fix:** none — accepted as a language semantic.  Capture-at-definition
is the intended behaviour and the dead-assignment warning is informative.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [SAFE.md](SAFE.md) — safety analysis for parallel workers and coroutines
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
