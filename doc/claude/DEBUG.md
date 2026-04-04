---
render_with_liquid: false
---
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Debugging Strategy

The primary debug surface is the `LOFT_LOG` environment variable, which selects a
preset defined in `src/log_config.rs`. Set it before running a test:

```bash
LOFT_LOG=minimal cargo test -- my_test 2>&1 | head -200
LOFT_LOG=ref_debug cargo test -- my_test 2>&1 | head -500
LOFT_LOG=full cargo test -- my_test 2>&1
```

---

## Contents
- [Preset Guide](#preset-guide)
- [Debugging a Parse Error or Wrong IR](#debugging-a-parse-error-or-wrong-ir)
- [Debugging a Runtime Crash or Wrong Result](#debugging-a-runtime-crash-or-wrong-result)
- [Debugging a validate_slots Panic](#debugging-a-validate_slots-panic)
- [Debugging a Scope Analysis Bug](#debugging-a-scope-analysis-bug)
- [Using the Test Framework for Quick Iteration](#using-the-test-framework-for-quick-iteration)

---

## Preset Guide

| Preset | What it shows | When to use |
|--------|---------------|-------------|
| `minimal` | Bytecode execution trace (opcode + stack state per step) | Stack corruption, wrong opcode, wrong result |
| `ref_debug` | Reference allocation and free events | Double-free, use-after-free, wrong store_nr |
| `full` | IR tree + bytecode + execution | Everything at once; output is very large |
| `static` | IR tree and bytecode only (no execution) | Codegen bugs, wrong IR, wrong opcode selection |
| `crash_tail:N` | Last N lines before panic | Crash triage when full output is too large |

---

## Database / Struct Debug Dumps in the Trace

Every opcode that produces or consumes a `DbRef` (struct, enum, or vector) shows a
compact inline dump of the pointed-to record in the execution trace.  The format is:

```
   8:[44] VarRef(var[32]=l) -> #3.1 { name: "diagonal", start: #2.1 { x: 1.5, y: 2.5 }, end_p: #1.1 { x: 10, y: 20 } }[44]
  65:[68] GetField(v1=ref(3,1,8)[56], fld=0) -> #3.1 { }[56]
```

**Reference prefix** `#store.record` — e.g. `#3.1` means store 3, record 1.
This tells you which allocation each struct lives in, making it easy to track
aliasing and double-free issues across opcodes.

**Depth limit** — nested structs expand up to depth 2 by default.  Deeper records
are shown as `{...}`:

```
#3.1 { inner: #5.7 { val: 42, nested: #6.2 {...} } }
```

**Element limit** — vectors show up to 8 elements by default, then `...N more`:

```
#4.3 [ #2.1 { x: 0 }, #2.2 { x: 1 }, ...6 more ]
```

**Depth limit at a vector** — if the depth limit is reached at a vector, shows
the element count instead of expanding: `#4.3 [10 items...]`

**Null fields are hidden** — fields holding the null sentinel are omitted, so a
freshly allocated struct with only one field set shows only that field.  This keeps
traces compact even for large structs.

### Tuning the dump limits

```bash
LOFT_DUMP_DEPTH=3    # expand up to 3 levels of nesting (default 2)
LOFT_DUMP_ELEMENTS=4 # show at most 4 vector elements (default 8)
```

These are read from the environment at runtime; no recompile needed.

### Accessing dumps directly via `cargo run`

When `LOFT_LOG` is set, `cargo run --bin loft` routes execution through
`execute_log` and writes the full trace (including struct dumps) to stderr:

```bash
LOFT_LOG=full  cargo run --bin loft -- myprog.loft 2>trace.txt
LOFT_LOG=minimal cargo run --bin loft -- myprog.loft 2>trace.txt
LOFT_DUMP_DEPTH=3 LOFT_LOG=full cargo run --bin loft -- myprog.loft 2>trace.txt
```

Without `LOFT_LOG`, the program runs without any trace output (production mode).

### Implementation

| File | Role |
|------|------|
| `src/database/mod.rs` | `DumpDb` struct — stores, depth/element limits, compact flag |
| `src/database/format.rs` | `Stores::dump_compact()`, `DumpDb::write()`, `write_struct()`, `write_list()` |
| `src/state/debug.rs` | `dump_limits()`, `dump_result()`, `dump_stack()` — calls `dump_compact()` for inline trace |
| `src/main.rs` | Routes `LOFT_LOG`-enabled runs through `execute_log` instead of `execute_argv` |

---

## Debugging a Parse Error or Wrong IR

1. Add `LOFT_LOG=static` and run the failing test.
2. In the output, find the function that contains the wrong code.
3. Compare the emitted IR (`Value` tree) against what you expect.
4. If the IR is wrong: the bug is in the parser. Search for the relevant `Value`
   variant in `src/parser/` and trace through `parse_single` or `parse_operators`.
5. If the IR is correct but the bytecode is wrong: the bug is in `src/state/codegen.rs`,
   in the `value_code` branch for the relevant `Value` variant.

---

## Debugging a Runtime Crash or Wrong Result

1. Reproduce with the smallest possible loft program (isolate to a single function).
2. Add `LOFT_LOG=minimal` and run. Find the last opcode executed before the crash or
   wrong result.
3. If the opcode is a memory access (`set_int`, `get_int`, `set_long`, etc.) and the
   `store_nr` is a large or unexpected value (like 60 or 0x3C), the DbRef on the
   stack is garbage — the bug is in scope analysis or codegen, not in the opcode.
   Switch to `LOFT_LOG=ref_debug` to find where the bad DbRef was created.
4. If the opcode itself is wrong (wrong opcode for the operation), check
   `src/state/codegen.rs` and the `Stack::operator` delta table in `src/stack.rs`.

---

## Debugging a validate_slots Panic

`validate_slots` panics in debug builds when two variables with overlapping live
intervals share the same stack slot. The panic message includes both variable names,
their slot range, and their live intervals.

1. Identify which function and which two variables conflict.
2. Add a minimal reproducer to `tests/slot_assign.rs`.
3. Check whether the live intervals truly overlap (can both variables be live at the
   same time?) or whether `compute_intervals` is computing a conservatively wide range.
4. If the overlap is real: a bug in scope analysis assigned the same slot to two
   simultaneously-live variables. Check `scopes.rs::copy_variable`.
5. If the overlap is spurious (a sequential block reuse): the exemption in
   `find_conflict` may need to be extended.

---

## Debugging a Tricky Compiler Bug (use logging first)

For non-obvious bugs — wrong use counts, unexpected variable lifetimes, closure leaks,
dead-assignment warnings that fire or don't fire — **always add targeted debug logging
before attempting a fix**.

Reasoning alone about multi-pass parser/compiler state is unreliable; logging shows
exactly what is happening.

Pattern:
1. Add `eprintln!` to the tracking function closest to the symptom (e.g. `in_use`,
   `track_write`, slot-assignment helpers).
2. Run the failing test and read the output to confirm your hypothesis.
3. If the call site is still unclear, add `std::backtrace::Backtrace::capture()` at the
   suspicious point and print it. This pinpoints the exact source location.
4. Fix the root cause, then **remove all debug prints before committing**.

Example: when investigating why a dead-assignment warning stopped firing, adding
`eprintln!` to `in_use` and `track_write` immediately revealed an extra `uses` increment
from a captured variable re-read, and the backtrace pointed to the exact `parse_var` call.

---

## Debugging a Scope Analysis Bug

Scope analysis bugs are the hardest to diagnose. The gap between the wrong IR
insertion and the runtime crash is large.

Strategy:
1. Use `LOFT_LOG=ref_debug` to capture all allocation and free events.
2. Look for a `free` event on a DbRef whose `store_nr` does not match any live
   allocation — that is the double-free or wrong-store free.
3. Search backwards in the log for the `alloc` event for that DbRef. The function and
   variable name tell you where the wrong free was inserted.
4. In `src/scopes.rs`, find the `get_free_vars` or `exit_scope` call that produced
   the wrong `OpFreeRef` / `OpFreeText`, and fix the scope assignment for that variable.

---

## Using the Test Framework for Quick Iteration

The `code!` and `expr!` macros in `tests/testing.rs` let you write a loft program
inline in a Rust test:

```rust
#[test]
fn my_feature() {
    expr!("my_expr_result").result(Value::Int(42)).run();
    code!("fn main() { assert(1 + 1 == 2, \"math\"); }").run();
}
```

Use `.error("expected error message")` to assert on compile-time diagnostics.
Use `.warning("expected warning")` for non-fatal diagnostics.

For end-to-end tests on `.loft` files, add to `tests/docs/` and the `wrap.rs`
runner will pick it up automatically.

---

## See also
- [../DEVELOPERS.md](../DEVELOPERS.md) — Developer guide: pipeline overview, quality requirements, feature proposals
- [TESTING.md](TESTING.md) — Test framework, `code!` / `expr!` macros, LogConfig debug presets
- [PROBLEMS.md](PROBLEMS.md) — Known bugs with severity, workarounds, and fix paths
- [ASSIGNMENT.md](ASSIGNMENT.md) — Variable scoping and slot assignment details
