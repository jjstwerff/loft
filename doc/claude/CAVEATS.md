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

**Workaround:** use the interpreter (`cargo run --bin loft`) instead of `--native-wasm`.
**Remaining work:** W1.18 (threading) in [ROADMAP.md](ROADMAP.md) (1.1+).

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

## C38 — Closure capture is copy-at-definition-time

When a lambda captures a variable, the value is **copied** into the closure
record at definition time.  Subsequent mutations of the original variable are
not visible inside the lambda (and vice versa).

**Reproducer:**
```loft
x = 10
f = fn(y: integer) -> integer { x + y }
x = 99
f(5)   // returns 15, not 104 — x=10 was captured at definition time
```

**Test:** `closure_capture_after_change` (passes — documents the behaviour).
**Note:** this is by design (value semantics), not a bug.  It matches Rust's
`move` closure semantics.  Shared-reference captures would require a
reference-counting or borrow-checking scheme.

---

## C39 — Native codegen: closure records not freed for fn-ref variables

The `--native` backend skips `OpFreeRef` for `Type::Function` variables
(`src/generation/dispatch.rs`).  The interpreter correctly frees closure
records at offset+4 in the fn-ref slot via a codegen special case, but the
native codegen does not yet replicate this logic.

**Impact:** closure store records leak in native-compiled programs when fn-ref
variables go out of scope.  For short-lived programs or chained calls
(`make_greeter("Hello")("world")`) the impact is negligible.

**Workaround:** use the interpreter for programs with many stored fn-refs.
**Planned fix:** extend native codegen to emit `(u32, DbRef)` destructuring
and explicit `OpFreeRef` for the closure component.
**Docs:** [LIFETIME.md](LIFETIME.md) § Caller-side closure free.

---

## C40 — Debug logger: fn-ref opcodes require C30 guard

`OpPutFnRef` and `OpVarFnRef` declare their mutable attribute as `text` in
`02_images.loft`, but the stack holds 16 bytes of fn-ref data (`[d_nr:i32]
[closure:DbRef]`).  The debug logger's `log_step` would interpret these bytes
as a `Str` pointer and SIGSEGV.  A guard in `log_step` skips text-typed
mutable attributes for these opcodes.

**Risk:** if new fn-ref opcodes are added without updating the guard, the
SIGSEGV will return.  The root cause (type mismatch in opcode declarations)
remains unfixed.
**Guard location:** `src/state/debug.rs` — `log_step`, mutable-attribute loop.
**History:** originally fixed in C30 (commit f0b6362), accidentally removed in
commit 9420be9, restored in the A5.6-text branch.

---

## C41 — Struct-enum local variable leaks stack space (Problem #85)

Creating a struct-enum variant as a local variable and returning a scalar
causes a debug assertion "Stack not correctly cleared" because the enum's
store record is never freed.  Release builds silently leak.

**Reproducer:**
```loft
enum Value { IntVal(n: integer), FloatVal(f: float) }
fn test() -> integer {
    v = IntVal { n: 42 }
    match v { IntVal { n } => n, _ => 0 }
}
```

**Test:** none yet (debug assertion fires).
**Workaround:** pass enum values as parameters instead of storing locally.
**Fix path:** emit `OpFreeRef` for struct-enum locals at scope exit (small effort).
**Docs:** [PROBLEMS.md](PROBLEMS.md) § Issue 85.

---

## C42 — `Type::Unknown(0)` silently created for unresolved names (Problem #58)

When the parser encounters an undefined name or typo, it silently creates a
`Type::Unknown(0)` variable instead of emitting a diagnostic.  This masks
user errors that would otherwise be caught at compile time.

**Reproducer:**
```loft
fn test() -> integer {
    reuslt = 42    // typo: 'reuslt' instead of 'result'
    result         // silently creates a new Unknown(0) variable
}
```

**Test:** none yet.
**Workaround:** check loft code carefully for typos.
**Fix path:** add early name-validation pass or emit diagnostic when
`Unknown(0)` is created (medium effort).
**Docs:** [PROBLEMS.md](PROBLEMS.md) § Issue 58.

---

## C43 — Text slot reuse disabled (Problem #69, A12)

Extending `can_reuse` in the slot allocator to allow text-slot reuse caused
overlapping live intervals between variables of different sizes sharing a
dead 24-byte text slot.  Text slot reuse is disabled; sequential text
variables each get their own 24-byte slot.

**Impact:** wastes stack space when many short-lived text variables are used
sequentially.  No correctness issue.

**Test:** `assign_slots_sequential_text_reuse` in `src/variables/slots.rs`
(`#[ignore]` — A12).
**Blocked by:** Problem #70 (text TOS override).  Problem #68 is fixed
(`inline_ref_set_in` now handles Block/Loop nodes).
**Docs:** [PROBLEMS.md](PROBLEMS.md) § Issues 68–70.

---

## C44 — Native codegen: `external` crate reference unresolved (Problem #79)

The `--native` backend does not resolve the `external` FFI crate used by
the random number extension.  `21-random.loft` fails to compile natively.

**Reproducer:** `cargo run --bin loft -- --native tests/docs/21-random.loft`
**Impact:** `--native` only; interpreter works correctly.
**Fix path:** bundle FFI in `codegen_runtime` or emit `extern` block.
**Docs:** [PROBLEMS.md](PROBLEMS.md) § Issue 79.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [SAFE.md](SAFE.md) — safety analysis for parallel workers and coroutines
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
