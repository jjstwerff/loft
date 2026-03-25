// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Caveats

Verifiable edge cases and limitations that affect users or block tests.
Each entry has a reproducer and the test(s) that exercise it, so a release
build can be retested quickly.

**Maintenance rule:** remove an entry when the underlying issue is fully fixed
and the test passes without workarounds.

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

## C2 — Native codegen: `FileResult` enum method `ok()` not supported

The `--native` and `--native-wasm` backends cannot compile programs that call
`.ok()` on a `FileResult` enum value.  The interpreter works correctly.

**Reproducer:** `tests/scripts/42-file-result.loft` (any call to `.ok()`)

**Test:** `tests/native.rs` — `SCRIPTS_NATIVE_SKIP` includes `42-file-result.loft`.
**Workaround:** avoid `--native` for programs using `FileResult.ok()`.
**Planned fix:** native codegen for enum methods (no specific PLANNING item yet).

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
**Planned fix:** scope-exit approximation improvement (no specific PLANNING item).
**Docs:** [SLOT_FAILURES.md](SLOT_FAILURES.md) § B-dir.

---

## C5 — Slot assignment: sequential file blocks conflict

Sequential `{ f = file(...); ... }` blocks can cause a ref-variable slot
override that overlaps with a subsequent variable.

**Test:** `tests/slots.rs` — `sequential_file_blocks_read_conflict` (`#[ignore]`, B-binary class)
**Workaround:** reuse a single file variable across blocks instead of re-declaring.
**Planned fix:** codegen `running_tos` overestimate correction (no specific PLANNING item).
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

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
