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

## C43 — Text slot reuse *(fixed)*

**Fixed.** Zone-2 text-to-text slot reuse is now enabled.  Sequential text
variables with non-overlapping lifetimes share the same 24-byte slot.
Restricted to Text-only reuse at the top-of-stack position to avoid
partial overlap with Reference/Vector variables (discovered during
implementation).

**Tests:** `assign_slots_sequential_text_reuse` (unit), `text_slot_reuse_sequential` (integration).
**Fixed by:** C43.1–C43.4 — `find_reusable_zone2_slot` + top-of-stack filter.

---

## C45 — Zone-2 slot reuse limited to Text-only + top-of-stack

Zone-2 slot reuse (C43) is restricted to `Type::Text` variables and only
the slot immediately below `*tos`.  Reference and Vector variables cannot
reuse dead zone-2 slots because:

1. **IR-walk ordering** — zone-2 assigns in IR-walk order, not live-interval
   order.  The conflict scan only sees already-assigned variables, missing
   future assignments that may overlap the reused slot.
2. **Block-return frame sharing** — non-Text zone-2 variables (Reference,
   Vector) use the block-return pattern where the child scope's zone-1
   frame starts at the variable's slot.  Reusing such slots would break
   the frame layout.

**Impact:** Reference and Vector variables still get sequential slots.
Only text reuse saves stack space (24 bytes per reuse).
**Workaround:** none needed — correctness is preserved.
**Docs:** [PLANNING.md](PLANNING.md) § C43.

---

## C46 — Zone-2 text reuse: top-of-stack restriction removed *(fixed)*

**Fixed.** The top-of-stack filter was removed; the full conflict scan in
`find_reusable_zone2_slot` is sufficient to prevent overlaps.  Non-consecutive
text reuse now works (e.g., text, reference, text — the second text reuses
the first's slot).

**Test:** `zone2_text_reuse_non_consecutive` (unit).
**Fixed by:** C46 — removed `.filter(|&slot| slot + v_size == *tos)`.

---

## C47 — Native codegen: CallRef dispatch doesn't pass `__closure`

The native codegen's `output_call_ref` in `src/generation/emit.rs` generates a
`match var_f.0 { d_nr => fn_name(stores, args...) }` dispatch.  When the
matched function has a `__closure` parameter, the dispatch doesn't pass
`var_f.1` (the closure DbRef) as the last argument.

**Impact:** cross-scope closures (functions returning capturing lambdas) and
capturing closures passed to `map`/`filter`/`reduce` crash in `--native` mode
with "this function takes N arguments but N-1 were supplied".

**Reproducer:**
```loft
fn make_adder(n: integer) -> fn(integer) -> integer {
    fn(x: integer) -> integer { n + x }
}
make_adder(5)(10)   // works in interpreter, fails in --native
```

**Fix path:** in `output_call_ref` (`emit.rs:~276`), when a candidate has
`has_closure == true`, emit `var_{fn_ref_name}.1` as the last argument:
```rust
if *has_closure {
    write!(w, ", {var_name}.1")?;  // pass closure DbRef from fn-ref tuple
}
```

**Test:** cross-scope closure doc example should pass in `native_dir`.
**Docs:** [LIFETIME.md](LIFETIME.md) § Caller-side closure free.

---

## C48 — Capturing closures with map/filter/reduce

Capturing closures cannot be passed directly to `map`, `filter`, or `reduce`
in either the interpreter or native codegen.  The error is "function reference
must be a compile-time constant (use fn <name>)".

**Reproducer:**
```loft
factor = 3
scaled = map([1, 2, 3], fn(x: integer) -> integer { x * factor })
// Error: function reference must be a compile-time constant
```

**Root cause:** `map`/`filter`/`reduce` are implemented as built-in operators
that take a `fn <name>` reference, not a fn-ref variable.  The parser
(`parse_call` in `control.rs`) rejects lambda expressions in the function
argument position of these builtins.

**Fix path:** change `map`/`filter`/`reduce` to accept fn-ref variables
(CallRef) in addition to named function references.  This requires:
1. Parser: allow fn-ref variables in the function argument position
2. Codegen: emit CallRef dispatch instead of static Call for the callback
3. Native: use the `output_call_ref` dispatch (requires C47 first)

**Workaround:** store the closure in a variable and call it manually in a loop:
```loft
factor = 3
result = vector<integer>{};
for x in [1, 2, 3] { result += [x * factor] }
```

**Test:** once fixed, `map(nums, fn(x: integer) -> integer { x * factor })`
should work in both interpreter and native.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [SAFE.md](SAFE.md) — safety analysis for parallel workers and coroutines
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
