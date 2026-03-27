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

## C1 — Lambda closure capture: partial support, three remaining restrictions

Read-only capture of non-text values (integers, floats, booleans, enums) now
works in release mode (A5.3).  Three restrictions remain:

1. **Debug-mode store leak** — a captured value allocates a closure record that
   is not freed before `fn_return`, triggering the stack-size assertion.
   Tests pass under `--release` but are `#[ignore]`-d in debug builds.
2. **Text capture not supported** — text values inside a closure record
   need text-in-struct serialisation, which is not yet implemented.
3. **Mutable capture not supported** — `count += x` inside a lambda crashes in
   codegen with a self-reference guard error.

**Reproducer (restriction 3):**
```loft
fn test() {
  count = 0;
  f = fn(x: integer) { count += x; };  // mutable capture — codegen panic
  f(10);
}
```

**Tests:** `tests/expressions.rs` — `closure_capture_integer` / `closure_capture_text` / `closure_capture_multiple` (`#[ignore]` until debug leak fixed); `tests/parse_errors.rs` — `capture_detected` (`#[ignore]`, A5.4)
**Workaround:** pass needed values as explicit function arguments.
**Planned fix:** A5.6 in [ROADMAP.md](ROADMAP.md) (1.1+) — mutable capture + text capture; design in [PLANNING.md](PLANNING.md) § A5.6.
**Docs:** [LOFT.md](LOFT.md) § Lambda expressions.

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
**Planned fix:** W1 in [ROADMAP.md](ROADMAP.md) (0.8.3) — interpreter-as-WASM entry point; full feature coverage targeted alongside W1 completion.

---

## C4 — Slot assignment: text below TOS in nested scopes *(fixed in S17)*

**Fixed.** The two-zone slot design (0.8.3) ensures large variables (text, ref, vector)
are placed after the zone-1 frame is pre-claimed, so their stack position at codegen
time always matches the pre-assigned slot.

**Test:** `tests/slots.rs` — `text_below_tos_nested_loops` (passes, ignore removed).
**Fixed by:** S17 — two-zone slot redesign.

---

## C5 — Slot assignment: sequential file blocks conflict *(fixed in S18)*

**Fixed.** The same two-zone redesign eliminates the ref-variable override issue:
zone-1 pre-claim prevents running_tos from overestimating across sequential blocks.

**Test:** `tests/slots.rs` — `sequential_file_blocks_read_conflict` (passes, ignore removed).
**Fixed by:** S18 — two-zone slot redesign.

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
**Planned fix:** L7 in [ROADMAP.md](ROADMAP.md) (0.8.3) — emit non-zero exit code on parse/runtime errors.
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


## C11 — No `while` loop *(fixed in L10)*

**Fixed.** Loft now supports `while cond { body }` as syntax sugar that desugars
to a loop with an `if !cond { break }` guard at the top.

```loft
fn main() {
  i = 0;
  found = false;
  while !found { i += 1; found = i * i > 100; }
  assert(found, "found");
}
```

**Test:** `tests/scripts/46-caveats.loft` — `test_c11_while`
**Fixed by:** L10 — `while` loop syntax sugar.

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

## C14 — Format specifier silently ignored on incompatible types *(fixed in L9)*

**Fixed.** Using a numeric radix specifier (`:x`, `:b`, `:o`) on a `text` or
`boolean` value, or zero-padding (`:05`) on a `text` value, is now a compile
error instead of a silent no-op.

**Fixed by:** L9 — format specifier mismatch escalated to compile error.

---


## C16 — Struct-enum local variable: debug assertion fails

Constructing a struct-enum variant as a local variable and returning a scalar
from the function triggers a debug-mode assertion in release builds of the
test runner:

```
assertion failed: Stack not correctly cleared: 8 != 4
```

**Reproducer:**
```loft
fn get(v: ArgValue) -> integer { match v { IntVal { n } => n, _ => 0 } }
// fails when called as: get(IntVal { n: 42 })
```

**Tests:** `tests/expressions.rs` — `stack_trace_*` and `call_frame_*` (`#[ignore = "TR1.3: blocked by Problem #85"]`)
**Workaround:** Pass the struct-enum value as a function parameter rather than assigning it to a local variable.
**Planned fix:** S19 in [ROADMAP.md](ROADMAP.md) (0.9.0) — extend `scopes.rs::free_vars()` to emit `OpFreeRef` for struct-enum locals; design in [PLANNING.md](PLANNING.md) § S19.

---

## C17 — `stack_trace()` returns empty from parallel workers

Calling `stack_trace()` inside a `par(...)` loop body returns an empty vector
instead of the actual call frames.  The function does not error — it silently
returns zero frames.

**Reproducer:**
```loft
fn check() -> integer {
  total = 0;
  for _ in 0..4 par(total += len(stack_trace())) {}
  total  // always 0, even though calls are active
}
```

**Test:** none (no parallel-worker test for stack_trace yet).
**Workaround:** Call `stack_trace()` from the main thread only.
**Planned fix:** S21 in [ROADMAP.md](ROADMAP.md) (0.9.0) — set `data_ptr` in `execute_at*` parallel entry points; design in [PLANNING.md](PLANNING.md) § S21.

---

## C18 — `init(expr)` circular field dependency silently accepted

Two struct fields that reference each other through `init($.other)` are
not detected at compile time.  At runtime the result is undefined — fields
may read uninitialised memory.

**Reproducer:**
```loft
struct Bad {
  a: integer init($.b),
  b: integer init($.a),
}
// No compile error; runtime behaviour is undefined.
```

**Test:** `tests/parse_errors.rs` — `circular_init_error` (`#[ignore = "#91: circular-init detection not yet implemented"]`)
**Workaround:** Do not write mutually referencing `init` fields.
**Planned fix:** S20 in [ROADMAP.md](ROADMAP.md) (0.9.0) — DFS cycle check after struct field parsing; design in [PLANNING.md](PLANNING.md) § S20.

---

## C19 — Native codegen: tuples, coroutines, and generics interpreter-only

The `--native` backend does not support three language features:

| Feature | Interpreter | `--native` |
|---------|-------------|-----------|
| Tuple types (`(integer, float)`) | Yes | No |
| Coroutines (`yield`, `iterator<T>`) | Yes | No |
| Generic functions (`fn f<T>`) | Yes | No |

Scripts using these features are skipped from the native test suite
(`SCRIPTS_NATIVE_SKIP` in `tests/native.rs`).

**Test:** `tests/scripts/50-tuples.loft`, `51-coroutines.loft`, `48-generics.loft` — all pass in interpreter, all skipped in native.
**Workaround:** Use the interpreter (`cargo run --bin loft`) for programs that use these features.
**Planned fix:** N8 in [ROADMAP.md](ROADMAP.md) (1.1+) — native codegen extensions for tuples (N8a), coroutines (N8b), generics (N8c); design in [PLANNING.md](PLANNING.md) § N8.

---

## C20 — Tuple types: function return and text elements not yet implemented

Tuple literals, element access (`.0`, `.1`), element assignment, and tuple
parameters all work.  Two further cases do not:

1. **Tuple return type** — A function declared `-> (integer, integer)` cannot
   have its return value used: calling the function and accessing a return
   element requires work-variable codegen that is not yet written.
2. **Tuple containing `text`** — A `(integer, text)` tuple cannot be returned
   from a function; text-return calling conventions are not yet wired.

**Tests:** `tests/expressions.rs` — `tuple_type_return`, `tuple_destructure_basic`, `tuple_with_text` (`#[ignore = "T1.4: ..."]`)
**Workaround:** Return structs or individual values instead of tuples with multiple or text elements.
**Planned fix:** T1.8 in [ROADMAP.md](ROADMAP.md) (1.1+) — function return convention (T1.8a) and text element lifetime (T1.8b); design in [PLANNING.md](PLANNING.md) § T1.8.

---

## C21 — `yield from` has a slot assignment regression

`yield from inner()` — delegation to a sub-generator — compiles but produces
wrong slot assignments when the outer generator has variables before the
`yield from`.  The test is `#[ignore]`-d pending an IR restructuring.

**Reproducer:**
```loft
fn inner() -> iterator<integer> { yield 10; yield 20; }
fn outer() -> iterator<integer> { yield 1; yield from inner(); yield 2; }
```

**Tests:** `tests/expressions.rs` — `coroutine_yield_from` (`#[ignore = "CO1.4: yield from slot assignment regression"]`); `tests/scripts/51-coroutines.loft` — exercises the working subset of coroutines
**Workaround:** Inline the sub-generator's yields manually or collect the sub-generator into a vector first.
**Planned fix:** CO1.4-fix in [ROADMAP.md](ROADMAP.md) (1.1+) — slot allocator must treat coroutine frame as live across `yield from` expansion; design in [PLANNING.md](PLANNING.md) § CO1.4-fix.

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
