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

## C1 — Lambda closure capture: one remaining restriction

Read-only capture of non-text values (integers, floats, booleans, enums) works
in both debug and release builds.  Mutable capture (`count += x`) also works (A5.6a).
One restriction remains:

1. **Text capture not supported** — two runtime bugs block it:
   - **Bug 1 (stack layout):** `OpSetText`/`OpGetText` on the closure record produces a garbage
     `DbRef` in the lambda's stack frame, causing store-bounds panics at runtime.
   - **Bug 2 (text work buffers):** Text-returning lambdas via `CallRef` require the caller to
     pre-allocate a text work buffer (`RefVar(Text)` argument), which `generate_call` does but
     `generate_call_ref` does not.  `codegen.rs:generate_call_ref` has a `debug_assert` that
     fires immediately in debug builds when this case is hit.
   Root cause of Bug 1: `OpCallRef` stack frame setup vs. where the lambda's bytecode expects
   `__closure` to live; needs a dedicated investigation with targeted logging.

**Note:** The current implementation captures variable values at the call site,
not at the point of lambda definition.  `closure_capture_after_change` documents
this: `x = 10; f = fn(y) { x + y }; x = 99; f(5)` returns 104, not 15.
Capture-at-definition-time is a deferred improvement.

**Reproducer (restriction 1):**
```loft
fn test() {
  prefix = "Hello";
  greet = fn(name: text) -> text { "{prefix}, {name}!" };  // text capture — runtime crash
  greet("World");
}
```

**Tests:** `tests/expressions.rs` — `closure_capture_integer` / `closure_capture_multiple` / `closure_capture_after_change` (all pass); `closure_capture_text` (`#[ignore]`, restriction 1); `tests/parse_errors.rs` — `capture_detected` (passes, mutable capture)
**Workaround:** pass captured text values as explicit function arguments.
**Planned fix:** A5.6 in [ROADMAP.md](ROADMAP.md) (1.1+); needs investigation of `OpCallRef` stack layout + `generate_call_ref` text buffer allocation.
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
**Planned fix:** N8a.1–N8a.3 (tuples), N8b.1–N8b.3 (coroutines), N8c.1–N8c.2 (generics) in [ROADMAP.md](ROADMAP.md) (0.8.3); design in [PLANNING.md](PLANNING.md) § N8.

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

`par(...)` workers that write to a `const` input silently discard the write in
release builds (the write lands in a 256-byte thread-local dummy buffer) and
continue with stale data.  The locked-store guard is `debug_assert!`-only, so
only debug builds panic.

**Reproducer:**
```loft
fn worker(const items: vector<integer>, idx: integer) -> integer {
  items.push(0);  // writes to const arg — silently discarded in release
  items[idx]
}
fn main() {
  data = [1, 2, 3];
  result = par(worker(data, 0..3));
  // release: may return wrong values; debug: panics immediately
}
```

**Test:** none (release-build behaviour only).
**Workaround:** never write to `const` parameters; always test `par()` loops in debug mode.
**Planned fix:** S22 in [ROADMAP.md](ROADMAP.md) — remove `#[cfg(debug_assertions)]` guard on auto-lock.
**Docs:** [SAFE.md](SAFE.md) § P1-R1.

---

## C23 — Coroutine with `text` argument: use-after-free on first resume

A generator that takes a `text` parameter stores the `Str` pointer verbatim in
`stack_bytes` at creation time.  If the caller's string goes out of scope before
the first `next()` call, the generator resumes with a dangling pointer and
silently reads freed memory.  No panic is emitted in debug or release builds.

**Reproducer:**
```loft
fn words(sentence: text) -> iterator<text> { yield sentence; }
fn main() {
  gen = words("hello world");
  // if the literal String is freed before next(), first resume reads garbage
  w = next(gen);
}
```

**Test:** none (requires memory sanitizer; no loft-level reproducer triggers a clear failure).
**Workaround:** keep text arguments live for the generator's entire lifetime.
**Planned fix:** S25 in [ROADMAP.md](ROADMAP.md) — CO1.3d `serialise_text_slots` at coroutine create.
**Docs:** [SAFE.md](SAFE.md) § P2-R1, [COROUTINE.md](COROUTINE.md) § SC-CO-1.

---

## C24 — Coroutine with `text` locals: memory leak on exhaustion

Generators that have `text` local variables and yield at least once leak those
`String` allocations when exhausted.  `stack_bytes.clear()` treats its payload
as plain bytes — no `String` destructors are called.  The `text_owned`
serialisation path (CO1.3d) that would own these allocations is not yet
implemented.

**Reproducer:**
```loft
fn gen_texts() -> iterator<integer> {
  greeting = "hello";  // text local on the generator stack
  yield 1;
  // exhaustion: greeting's String heap allocation is leaked
}
```

**Test:** none (requires allocator leak detection).
**Workaround:** none; avoid text locals in generators where memory pressure matters.
**Planned fix:** S25 in [ROADMAP.md](ROADMAP.md) — CO1.3d (must land atomically with C23 fix).
**Docs:** [SAFE.md](SAFE.md) § P2-R2/P2-R3.

---

## C25 — `yield` inside `par()` body: panic or wrong results

The compiler does not reject `yield` expressions or generator calls inside a
`par(...)` body.  At runtime a worker indexes its own (nearly empty) `coroutines`
table with a `rec` from the main thread.  Out-of-bounds `rec` → Rust panic;
in-bounds collision → worker silently advances the wrong generator.

**Reproducer:**
```loft
fn gen() -> iterator<integer> { yield 1; }
fn main() {
  result = par(fn(i: integer) -> integer { next(gen()) }(0..4));
  // panics or returns wrong values depending on worker coroutine table state
}
```

**Test:** none (unsafe to run as a test).
**Workaround:** never use `yield` or generator calls inside `par(...)` bodies.
**Planned fix:** S23 in [ROADMAP.md](ROADMAP.md) — `inside_par_body` compiler flag + runtime bounds guard.
**Docs:** [SAFE.md](SAFE.md) § P2-R6, [COROUTINE.md](COROUTINE.md) § SC-CO-4.

---

## C26 — `e#remove` on a generator iterator: store corruption in release

`e#remove` on a generator-typed loop variable is not rejected at compile time.
At runtime `remove()` receives a DbRef with `store_nr == u16::MAX`.  Debug
builds panic (out-of-bounds).  Release builds compute `u16::MAX % allocations.len()`
as the store index and delete a real record, silently corrupting that store's
free list.

**Reproducer:**
```loft
fn gen_items() -> iterator<integer> { for i in 0..5 { yield i; } }
fn main() {
  for e in gen_items() {
    e#remove;  // release: deletes an arbitrary record in a real store
  }
}
```

**Test:** none (debug panics; release silently corrupts — unsafe to automate).
**Workaround:** only use `e#remove` with store-backed collection iterators.
**Planned fix:** S24 in [ROADMAP.md](ROADMAP.md) — compiler rejection + runtime guard in `remove()`.
**Docs:** [SAFE.md](SAFE.md) § P2-R9, [COROUTINE.md](COROUTINE.md) § SC-CO-11.

---

## C27 — Native tests: `rand_core` unavailable in standalone rustc compilation

`rand_core` and `rand_pcg` are optional feature dependencies of the loft crate
(enabled by the `random` feature).  When the native test harness compiles generated
`.rs` files by invoking `rustc` directly, it passes `--extern loft=libloft.rlib` but
does not pass `--extern rand_core=...` for transitive optional deps.  Any native test
that calls `rand`, `rand_seed`, or `rand_indices` therefore fails to compile with
`error[E0433]: failed to resolve: use of undeclared crate or module 'rand_core'`.

**Reproducer:** run `cargo test --test native` with `tests/scripts/15-random.loft` or
`tests/docs/21-random.loft` included (not skipped).

**Test:** `15-random.loft` and `21-random.loft` are in the native skip lists.
**Workaround:** avoid random functions in native test scripts until the harness is fixed.
**Planned fix:** extend `find_loft_rlib` / rustc invocation in `tests/native.rs` to
collect and pass `--extern` flags for all transitive optional deps from the deps dir.

---

## C28 — Native tests: slot conflict in `20-binary.loft` (`rv` vs `_read_34`)

The slot allocator assigns overlapping slots to `rv` and `_read_34` in `n_main`
within `tests/scripts/20-binary.loft`:

```
=== Slot conflict in function 'n_main' ===
* 'rv'       slot [820, 832)  live [1016, 1110]
* '_read_34' slot [820, 832)  live [1008, 1109]
```

Both variables have overlapping live ranges and are assigned the same slot range.
This is a pre-existing slot assignment regression (category A/B from SLOT_FAILURES.md)
that the native codegen exposes as a compile or runtime error.

**Reproducer:** run `cargo test --test native` with `20-binary.loft` included.

**Test:** `20-binary.loft` is in `SCRIPTS_NATIVE_SKIP`.
**Workaround:** keep `20-binary.loft` skipped in native tests.
**Planned fix:** root-cause the slot conflict in the two-zone allocator (see SLOT_FAILURES.md);
fix and remove from skip list.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [SAFE.md](SAFE.md) — safety analysis for parallel workers and coroutines
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
