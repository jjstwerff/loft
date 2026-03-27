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

## C1 — Lambda closure capture not supported

Lambdas cannot capture variables from the enclosing scope.  A lambda that
references an outer variable produces incorrect results or a compile error.

**Reproducer:**
```loft
fn test() {
  count = 0;
  f = fn(x: integer) { count += x; };
  f(10);
  f(32);
  assert(count == 42, "expected 42, got {count}");
}
```

**Test:** `tests/issues.rs` — `p1_1_lambda_void_body` (`#[ignore]`, A5 1.1+)
**Workaround:** pass needed values as explicit function arguments.
**Planned fix:** A5 (closure capture), targeted for 1.1+.
**Docs:** [LOFT.md](LOFT.md) § Lambda expressions — states "cannot capture variables from surrounding scope".

---

## C3 — WASM backend: several features not implemented

The `--native-wasm` backend lacks support for file I/O, threading, random
numbers, time functions, and dynamic function references (`CallRef`).

**Affected files:** `tests/wrap.rs` — `WASM_SKIP` array:

| File | Reason |
|------|--------|
| `06-function.loft` | `CallRef` not implemented (#77) |
| `13-file.loft` | File I/O missing (#74) |
| `18-locks.loft` | `todo!()` |
| `19-threading.loft` | `todo!()` |
| `21-random.loft` | External crate unresolved (#79) |
| `22-time.loft` | `todo!()` |

**Workaround:** use the interpreter (`cargo run --bin loft`) instead of `--native-wasm`.
**Planned fix:** W1 (WASM foundation), targeted for 1.0.0.

---

## C4 — Slot assignment: text below TOS in nested scopes

A text variable inside a nested if-block inside a loop can be pre-assigned a
slot below the actual top-of-stack, causing a codegen panic.

**Test:** `tests/slots.rs` — `text_below_tos_nested_loops` (`#[ignore]`, B-dir class)
**Workaround:** restructure code to avoid deeply nested text assignments inside loops.
**Planned fix:** S17 in [ROADMAP.md](ROADMAP.md) (1.1+).
**Docs:** [SLOT_FAILURES.md](SLOT_FAILURES.md) § B-dir.

---

## C5 — Slot assignment: sequential file blocks conflict

Sequential `{ f = file(...); ... }` blocks can cause a ref-variable slot
override that overlaps with a subsequent variable.

**Test:** `tests/slots.rs` — `sequential_file_blocks_read_conflict` (`#[ignore]`, B-binary class)
**Workaround:** reuse a single file variable across blocks instead of re-declaring.
**Planned fix:** S18 in [ROADMAP.md](ROADMAP.md) (1.1+).
**Docs:** [SLOT_FAILURES.md](SLOT_FAILURES.md) § B-binary.

---

## C6 — Exit code always 0

`loft` exits with code 0 even on parse errors.  Shell scripts that check
`$?` will miss failures.

**Reproducer:**
```sh
loft nonexistent.loft; echo $?   # prints 0
```

**Test:** none (shell-level behaviour).
**Workaround:** capture output and grep for `Error:` or `panicked`.
**Docs:** [LOFT.md](LOFT.md) § Known Limitations.

---

## C7 — `spacial<T>` not implemented

The spatial index collection type is declared but all operations panic at
runtime.  A compile-time error is emitted for basic usage, but edge cases
may still reach the runtime panics.

**Test:** `tests/scripts/36-parse-errors.loft` — `@EXPECT_ERROR` for spacial.
**Planned fix:** A4 (spatial index operations), targeted for 1.1+.
**Docs:** [PROBLEMS.md](PROBLEMS.md) § Issue 22.

---

## C8 — Predicate aggregates (`any`, `all`, `count_if`) not implemented

Only `sum_of`, `min_of`, `max_of` are available for `vector<integer>`.
Lambda-based predicate aggregates require compiler special-casing for
iterator loop generation that is not yet implemented.

**Reproducer:**
```loft
fn main() {
  nums = [1, 2, 3];
  // These would be: any(nums, |x| { x > 2 })  — not available yet
}
```

**Test:** none (feature not implemented).
**Planned fix:** P3 predicate aggregates (deferred — needs compiler loop IR work).
**Docs:** [PLANNING.md](PLANNING.md) § P3; [ROADMAP.md](ROADMAP.md) deferred note.

---

## C11 — No `while` loop

Loft has no `while` keyword. Polling or retry patterns require `for` with a
large upper bound and `break`.

**Reproducer:**
```loft
fn main() {
  // Polling pattern — the only way to loop until a condition:
  found = false;
  for i in 0..1000000 {
    if i * i > 100 { found = true; break }
  }
  assert(found, "found");
}
```

**Test:** `tests/scripts/46-caveats.loft` — `test_c11_no_while`
**Workaround:** `for i in 0..LARGE { if condition { break } }`
**Docs:** [00-vs-rust.html](../00-vs-rust.html) § No while loop; [00-vs-python.html](../00-vs-python.html) § No while loop.

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

## C14 — Format specifier silently ignored on incompatible types

A numeric format specifier like `:05` on a text value is silently ignored
instead of producing a compile error.

**Reproducer:**
```loft
fn main() {
  t = "hello";
  r = "{t:05}";
  // r is "hello", not "0hello" — the :05 specifier is silently dropped.
  assert(r == "hello", "format: {r}");
}
```

**Test:** `tests/scripts/46-caveats.loft` — `test_c14_format_specifier_ignored`
**Workaround:** ensure format specifiers match the value type.
**Docs:** [00-vs-rust.html](../00-vs-rust.html) § String formatting.

---

## C15 — Parallel for: limited return types and no struct references

`par(...)` workers can only return primitive types (`integer`, `long`, `float`,
`boolean`, `text`, plain `enum`). Struct references cannot be returned.
Context must be embedded in the worker's extra arguments.

**Reproducer:**
```loft
struct Point { x: float, y: float }
// This does NOT work — cannot return struct from par worker:
// for p in points par(result = transform(p)) { ... }
```

**Test:** none needed (compile error produced).
**Workaround:** return primitive values; reconstruct structs after the parallel loop.
**Docs:** [THREADING.md](THREADING.md) § Supported return types; [00-vs-rust.html](../00-vs-rust.html) § Parallel for-loops.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
