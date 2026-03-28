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

## C6 — Exit code always 0 *(fixed)*

**Fixed.** `main.rs` already calls `std::process::exit(1)` whenever
`p.diagnostics.level() >= Level::Error`.  A missing file produces a
`Level::Fatal` diagnostic (`lexer.switch` → `"Unknown file:{filename}"`),
which is greater than `Level::Error`, so the process exits with code 1.

**Reproducer (now works correctly):**
```sh
loft nonexistent.loft; echo $?   # prints 1
```

**Test:** none (shell-level behaviour; verified by code inspection of `main.rs` lines 348–354 and `lexer.rs` `switch()`).
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


## C16 — Struct-enum local variable: debug assertion fails *(fixed in S19)*

**Fixed.**  The three `stack_trace_*` / `call_frame_*` tests that were
`#[ignore]`-d under Problem #85 now pass without workarounds.  Two root
causes were addressed:

1. **Reused-store garbage in vector fields** — `n_stack_trace` now explicitly
   zeroes the `arguments` and `variables` fields of each `StackFrame` element
   so that reused (non-zeroed) store blocks don't leave garbage data that
   `is_null` misreads as a valid `first_block_rec`.
2. **Synthetic entry frame** — `execute_log_steps` now pushes the same
   synthetic `CallFrame` for the entry function as `execute_argv` does
   (Fix #88 parity), so `stack_trace()` returns a consistent frame count
   regardless of which execution path is used.
3. **Call-site line numbers** — `fn_call` now uses a BTreeMap backward
   range search (`range(..=code_pos).next_back()`) so that the nearest
   source line is returned even when `code_pos` has advanced past the
   `line_numbers` entry for the call instruction.

**Tests:** `tests/expressions.rs` — `stack_trace_returns_frames`,
`stack_trace_function_names`, `call_frame_has_line` (all pass, `#[ignore]` removed).
**Fixed by:** S19 — `n_stack_trace` field zeroing + entry-frame parity + BTreeMap line lookup.

---

## C17 — `stack_trace()` returns empty from parallel workers *(fixed in S21)*

**Fixed.**  `stack_trace()` called inside a `par(...)` worker or any
`run_parallel_*` worker now returns the actual call frames.

Two root causes were addressed:
1. **`stack_trace_lib_nr` not propagated** — `WorkerProgram` now carries the
   resolved library index of `n_stack_trace`; `WorkerProgram::new_state` copies
   it into the worker's `stack_trace_lib_nr` field so the snapshot trigger fires.
2. **Snapshot skipped when `data_ptr` null** — `static_call` now takes the
   snapshot even when `data_ptr` is null (direct `run_parallel_*` path), using
   a `"<worker>"` placeholder for frames whose definition is unavailable.

**Test:** `tests/threading.rs` — `parallel_stack_trace_non_empty` (passes,
no `#[ignore]`).
**Fixed by:** S21 — `WorkerProgram.stack_trace_lib_nr` + null-safe snapshot.

---

## C18 — `init(expr)` circular field dependency silently accepted *(fixed in S20)*

**Fixed.**  Mutually-referencing `init(expr)` fields now produce a compile
error naming the cycle.  A DFS cycle check runs after all struct field
definitions are parsed (second pass only); `$.field` reads inside `init(...)`
are tracked via `init_field_tracking`/`init_field_deps` on the parser, and
`check_circular_init` emits one error per cycle root.

**Test:** `tests/parse_errors.rs` — `circular_init_error` (passes, `#[ignore]` removed).
**Fixed by:** S20 — DFS cycle check in `parse_struct` + `$.`-read dep tracking.

---

## C19 — Native codegen: coroutines interpreter-only

The `--native` backend does not support all language features:

| Feature | Interpreter | `--native` |
|---------|-------------|-----------|
| Tuple types (`(integer, float)`) | Yes | **Fixed** (N8a, 0.8.3) |
| Coroutines (`yield`, `iterator<T>`) | Yes | No |
| Generic functions (`fn f<T>`) | Yes | **Fixed** (N8c, 0.8.3) |

Coroutine scripts remain skipped from the native test suite
(`SCRIPTS_NATIVE_SKIP` in `tests/native.rs`).  Tuples and generics now pass.

**Test:** `tests/scripts/51-coroutines.loft` — passes in interpreter, skipped in native.
`50-tuples.loft` — removed from skip list (N8a, 0.8.3).  `48-generics.loft` — removed from skip list (N8c, 0.8.3).
**Workaround:** Use the interpreter (`cargo run --bin loft`) for programs that use coroutines.
**Planned fix:** N8b.1–N8b.3 (coroutines) in [ROADMAP.md](ROADMAP.md); design in [PLANNING.md](PLANNING.md) § N8.

---

## C20 — Tuple types: function return with text elements *(fixed in T1.8b)*

**Fixed.**  A `(integer, text)` tuple can now be returned from a function.
Text elements are stored as `Str` (16B borrowed reference) in tuple slots, consistent
with loft's existing text-argument and text-return conventions.  The new `OpPutText`
opcode stores a `Str` into a tuple slot (analogous to `OpPutInt`).

**Note:** Returning a locally-constructed text value (e.g. `"a" + "b"`) inside a
tuple has the same lifetime caveat as returning `text` directly from a function —
this is a pre-existing interpreter limitation, not introduced by T1.8b.

**Test:** `tests/expressions.rs` — `tuple_with_text` (passes, `#[ignore]` removed).
**Fixed by:** T1.8b — `OpPutText` opcode + codegen fixes for tuple text slots.

---

## C21 — `yield from` has a slot assignment regression *(fixed in CO1.4-fix)*

**Fixed.**  `yield from inner()` — delegation to a sub-generator — now
produces correct slot assignments.  The two-zone slot redesign (S17/S18)
eliminated the overlap between the `__yf_sub` coroutine handle and inner
loop variables; no additional IR restructuring was required.

**Test:** `tests/expressions.rs` — `coroutine_yield_from` (passes,
`#[ignore]` removed).
**Fixed by:** CO1.4-fix — two-zone slot redesign covered the yield-from case.

## C22 — Parallel workers: silent wrong results in release builds

**Fixed in 0.8.3 (S22).**

`par(...)` workers that write to a `const` input previously silently discarded
the write in release builds.  The `#[cfg(debug_assertions)]` guard on auto-lock
insertion has been removed; `store.claim()` and `store.delete()` now use
`assert!` (not `debug_assert!`) so the panic fires in both debug and release
builds.

**Test:** `claim_on_locked_store_panics` and `delete_on_locked_store_panics`
in `tests/expressions.rs` — both pass.
**Docs:** [SAFE.md](SAFE.md) § P1-R1.

---

## C23 — Coroutine with `text` argument: use-after-free on first resume

**Fixed (S25.1, 0.8.3)**

`State::serialise_text_args` is now called in `coroutine_create` for every text
(`Str`) argument slot.  Each dynamic text arg is cloned into an owned `String` in
`frame.text_owned`; the `Str` in `stack_bytes` is updated to point to the owned
buffer.  On resume, the owned buffer address is re-patched into the cloned bytes
before they are copied to the live stack.  The caller's `OpFreeText` no longer
causes a dangling pointer.

**Test:** `coroutine_text_arg_dynamic_serialised` in `tests/expressions.rs`.
**Docs:** [SAFE.md](SAFE.md) § P2-R1, [COROUTINE.md](COROUTINE.md) § SC-CO-1.

---

## C24 — Coroutine text locals: memory leak on early `break`

**Partially fixed (S25.1/S25.2, 0.8.3) — text args fully serialised; text local
leak narrowed to one path.**

**Precise remaining leak (2026-03-29):**

Text locals (`word = "hello"` inside a generator) are `String` objects (24 bytes)
on the generator's live stack.  The bitwise-copy approach in `coroutine_yield` is
safe for the yield/resume cycle and for exhaustion (C24 was misstated — at
exhaustion, `OpFreeText` is emitted before `OpCoroutineReturn` by `scopes::check`,
so the live-stack String IS freed).

The only remaining leak is the **early-break path**:

1. Generator suspends (yields at least once).
2. Consumer breaks from `for item in gen { ... }`.
3. `OpFreeCoroutine` → `free_coroutine(idx)` → `self.coroutines[idx] = None`.
4. Dropping `Box<CoroutineFrame>` drops `frame.stack_bytes: Vec<u8>` as raw bytes
   — the `String` heap allocations embedded in those bytes are not freed.

**Reproducer (early-break path):**
```loft
fn gen_words() -> iterator<text> {
  word = "hello";
  yield word;         // frame.stack_bytes now holds String("hello")
  word = "world";
  yield word;
}
fn main() {
  for w in gen_words() { break; }  // break → free_coroutine → "hello" leaks
}
```

**Note:** iterating to exhaustion does NOT leak (fixed by S25.1/S25.2).

**Complication:** Zone 2 variables (including text locals) are pre-claimed via
`OpReserveFrame` but not zeroed.  Text locals that are assigned only after the
yield point have garbage bytes in `frame.stack_bytes`; calling `drop_in_place` on
those would be UB.  Fix must zero Zone 2 at generator startup first.

**Test:** `coroutine_text_local_survives_yield` (passes, no leak at exhaustion).
No test for the early-break path yet.
**Planned fix:** PLANNING.md S25.3 — zero Zone 2 at first resume + null-ptr-guarded
drop in `free_coroutine`.  Two-step, must land atomically.
**Workaround:** iterate generators to exhaustion rather than breaking.
**Docs:** [SAFE.md](SAFE.md) § P2-R2/P2-R3, [PLANNING.md](PLANNING.md) § S25.3.

---

## C25 — generator functions as `par()` workers: fixed in 0.8.3 (S23)

**Fixed in 0.8.3 (S23):** The compiler now rejects iterator-returning functions as
`par()` workers at parse time with a clear diagnostic. A runtime bounds guard in
`coroutine_next` panics with an actionable message if an out-of-range coroutine
DbRef is encountered (defence-in-depth for indirect paths).

**Test:** `par_worker_returns_generator` in `tests/parse_errors.rs`.
**Docs:** [SAFE.md](SAFE.md) § P2-R6, [COROUTINE.md](COROUTINE.md) § SC-CO-4.

---

## C26 — `e#remove` on a generator iterator: store corruption in release *(fully fixed)*

**Fixed in 0.8.3 (S24 + assert! upgrade).**

`e#remove` on a generator-typed loop variable is rejected at compile time
(CO1.5c — `loop_value == Null` in `collections.rs`).  A defense-in-depth
runtime guard in `remove()` (`state/io.rs`) and `OpRemove()`
(`codegen_runtime.rs`) checks `store_nr == u16::MAX` and panics if somehow
reached.  The guard has been upgraded from `debug_assert!` to `assert!` so
it fires in both debug and release builds.

**Test:** `generator_remove_rejected` in `tests/parse_errors.rs` — compile-time
rejection passes.
**Docs:** [SAFE.md](SAFE.md) § P2-R9, [COROUTINE.md](COROUTINE.md) § SC-CO-11.

---

## C29 — Native tests: `14-image.loft` PNG width returns 0 in CI (all platforms)

**Fixed in 0.8.3 (S33).**

The root cause was cross-profile rlib selection: `find_loft_rlib()` compared
modification times across `release/` and `debug/` deps directories and selected
the wrong profile's rlib.  After fixing it to use only the current test binary's
own `deps/` directory (`current_exe().parent()`), the correct profile rlib is
always used and the PNG native test compiles and runs correctly.

`14-image.loft` has been removed from `NATIVE_SKIP`.

**Test:** `14-image.loft` runs as part of `native_dir` — passes.
**Docs:** [NATIVE.md](NATIVE.md) § S33.

---

## C27 — Native tests: `rand_core` unavailable in standalone rustc compilation

**Fixed in 0.8.3 (S31).**

`collect_extra_externs()` was added to `tests/native.rs`: it scans all `.rlib`
files in the current test binary's `deps/` directory and passes each as
`--extern crate_name=path` to the standalone `rustc` invocation.  All versions
of each crate are passed (no deduplication), allowing rustc's hash matching to
select the one that `libloft` was compiled against.

`15-random.loft` and `21-random.loft` have been removed from their respective
native skip lists.

**Test:** `15-random.loft` and `21-random.loft` run as part of `native_dir` — pass.

---

## C28 — `20-binary.loft` slot conflict (`rv` vs `_read_34`) *(fully fixed in S32 + S34, 0.8.3)*

**Fixed (native, S32).**  `adjust_first_assignment_slot` now checks for same-scope sibling
overlap (`has_sibling_overlap`) before moving a large variable down to TOS.  This prevents
`rv` and `_read_34` from being assigned the same slot range during native codegen.

**Fixed (interpreter, S34).**  When `adjust_first_assignment_slot` cannot move a variable
(same-scope siblings block it) and Option A fires (moving the variable down to current TOS
anyway), the moved variable is now marked `skip_free`.  This prevents `generate_call` from
emitting an `OpFreeRef` for the aliased variable, eliminating the double-free that caused
the "Double free store" panic in the bytecode interpreter.

**Test (interpreter):** `20-binary.loft` passes in `cargo test --test wrap` (S34).
**Test (native):** in `SCRIPTS_NATIVE_SKIP` — native codegen emits malformed Rust for the
`Set(rv, Insert([Set(_read_34, Null), Block]))` pattern.  Before S34, `validate_slots`
panicked during `byte_code()`, which `catch_unwind` caught and converted to a silent skip.
After S34's bytecode fix the panic is gone, so the pre-existing native codegen bug for the
Insert-return pattern is now visible.  Tracked in `SCRIPTS_NATIVE_SKIP` in `tests/native.rs`.
**Fixed by:** S32 — `has_sibling_overlap` (interpreter + native slot assignment); S34 —
`skip_free` + suppressed `OpFreeRef` in `generate_call` (interpreter double-free only).

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [SAFE.md](SAFE.md) — safety analysis for parallel workers and coroutines
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
