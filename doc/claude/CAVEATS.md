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

## C47 — Native codegen: cross-scope closures *(fixed)*

**Fixed.** Five sub-issues resolved: FnRef emits closure DbRef, CallRef passes
`.1` as `__closure`, scope bounds check, `last_closure_work_var` reset after
function body, FnRef added to reachable set via `collect_fn_ref_literals`.

**Test:** `make_adder` in `26-closures.loft` passes in both interpreter and native.

---

## C48 — Capturing closures with map/filter *(fixed)*

**Fixed.** `map()` and `filter()` accept capturing lambdas.  The collections
parser stores the fn-ref in a local variable and emits `CallRef` in the
desugared loop body.

**Test:** `map_with_capturing_closure` in `tests/expressions.rs`.

---

## C49 — Libraries: wildcard and selective import *(fixed)*

**Fixed.** `use lib::*` and `use lib::Name, Other` now work.  Only `pub`-marked
definitions are imported into the namespace.  Non-pub definitions remain
accessible via `lib::name` prefix.

The fix: add `pub` to library definitions.  The import machinery was already
implemented but testlib.loft lacked `pub` markers.

**Test:** `17-libraries.loft` doc test.

---

## C50 — Libraries: `pub` controls import visibility *(fixed)*

**Fixed.** `pub` on library definitions controls which names are importable
via wildcard (`use lib::*`) or selective import.  Non-pub definitions are
NOT imported but remain accessible with the `lib::name` prefix.  This gives
library authors control over their public API surface.

---

## C51 — Libraries: no native extension loading

Libraries are pure `.loft` files.  The `loft.toml` manifest `native = "..."`
field is parsed but native shared libraries (`.so`/`.dylib`/`.dll`) are not
loaded at runtime.  Native extensions only work via the interpreter's built-in
`--lib` flag or `LOFT_LIB` environment variable pointing to a directory with
pre-compiled extensions.

---

## C53 — Match arms cannot use library enum variants

Match arms do not support namespaced variant names (`testlib::Ok`) or
bare variant names (`Ok`) for library enums.  Only same-file enum variants
work in match patterns.

**Reproducer:**
```loft
use testlib;
fn check(s: testlib::Status) -> text {
  match s { testlib::Ok => "ok", _ => "other" }
  // Error: unexpected '::'
}
```

**Workaround:** use if-else chains with `==` comparisons:
```loft
if s == testlib::Ok { "ok" } else { "other" }
```

**Fix path:** 4 changes in `src/parser/control.rs:parse_match` — handle
`::` in arm identifier, use resolved name for discriminant lookup, fall
back to `children_of` for bare names, update or-pattern loop.
**Docs:** [PLANNING.md](PLANNING.md) § C53.

---

## C54 — `file.lines()` returns empty for files without trailing newline

`lines()` splits by newline. A file with content but no trailing `\n`
returns 0 lines instead of 1 line.

**Reproducer:**
```loft
{f = file("test.txt"); f += "hello"; }
lines = file("test.txt").lines();
assert(lines.len() == 1, "should have 1 line");  // FAILS: len() == 0
```

Adding a newline works: `f += "hello\n"` → `lines.len() == 1`.

**Fix path:** `lines()` should return the last segment even without a
trailing newline — matching Python's `readlines()` and Rust's `lines()`.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [SAFE.md](SAFE.md) — safety analysis for parallel workers and coroutines
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
