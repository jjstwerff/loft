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

## C39 — Native codegen: fn-ref `(u32, DbRef)` tuple + closure free *(fixed)*

**Fixed.** Fn-ref variables in native-compiled code are now `(u32, DbRef)`
tuples.  `OpFreeRef` destructures `.1` and frees the closure if non-null.
The `fn_ref_context` flag ensures if-else branches with bare Int values
produce correct tuples.

**Test:** all 5 native tests pass (`cargo test --test native`).
**Fixed by:** C39 — coordinated changes across dispatch.rs, mod.rs, emit.rs,
calls.rs, pre_eval.rs.

---

## C40 — Debug logger: fn-ref opcode type mismatch *(documented)*

`OpPutFnRef` and `OpVarFnRef` declare their mutable attribute as `text` in
`02_images.loft`, but the stack holds 16 bytes of fn-ref data (`[d_nr:i32]
[closure:DbRef]`).  A guard in `log_step` skips text-typed mutable attributes
for these opcodes, preventing SIGSEGV.  WARNING comments added to the opcode
declarations in `02_images.loft` reference the guard.

**Guard location:** `src/state/debug.rs` — `log_step`, mutable-attribute loop.
**Fixed by:** C40 — documentation + WARNING comments in `02_images.loft`.

---

## C43 — Text slot reuse disabled (Problem #69, A12)

Text variables (24 bytes) are placed by zone 2 in `assign_slots`, which
assigns slots sequentially at TOS without dead-slot reuse.  Sequential text
variables each get their own 24-byte slot, wasting stack space.

A naive same-size reuse attempt caused slot conflicts: the reused slot
can partially overlap with other zone-2 variables placed by the same
`place_large_and_recurse` pass, because the reuse check only compares
against the candidate variable — not all previously assigned variables.
Zone-2 needs the same full conflict scan that zone-1 uses.

**Impact:** wastes stack space when many short-lived text variables are used
sequentially.  No correctness issue.

**Test:** `assign_slots_sequential_text_reuse` in `src/variables/slots.rs`
(`#[ignore]` — A12).
**Docs:** [PLANNING.md](PLANNING.md) § C43, [PROBLEMS.md](PROBLEMS.md) § Issues 69–70.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [SAFE.md](SAFE.md) — safety analysis for parallel workers and coroutines
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
