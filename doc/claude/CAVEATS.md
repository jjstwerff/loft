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

## C7 — `spacial<T>` not implemented

The spatial index collection type is declared but all operations panic at
runtime.  A compile-time error is emitted for basic usage, but edge cases
may still reach the runtime panics.

**Test:** `tests/scripts/36-parse-errors.loft` — `@EXPECT_ERROR` for spacial.
**Planned fix:** A4 (spatial index operations), targeted for 1.1+.
**Docs:** [PROBLEMS.md](PROBLEMS.md) § Issue 22.

---

## C31 — Closures in struct fields not yet supported

Closures in **vectors** now work (both capturing and non-capturing).
Storing closures as **struct fields** is not yet supported — requires
`Type::Function` handling in the struct field write path.

**Workaround:** use vectors or function arguments to pass closures.
**Planned fix:** deferred to 1.1+.

---

## C32 — Captured parameter "never read" warning is false for cross-scope closures

When a function parameter is captured by a closure that is returned from the
function, the parameter IS read (by the capture) but the use-analysis does not
track `SetText` on a closure record as a read.  The `Variable.captured` flag
suppresses the "never read" warning, but the dead-assignment analysis still
does not see the capture as a use.

**Reproducer:**
```loft
fn make_greeter(prefix: text) -> fn(text) -> text {
    fn(name: text) -> text { "{prefix} {name}" }
}
// No warning (suppressed by captured flag).
// But: x = "a"; f = fn() { x }; x = "b";
// Dead-assignment warning for x="a" fires even though it was captured.
```

**Impact:** cosmetic — the dead-assignment warning is arguably correct (the
capture happens before the overwrite, and the captured value is `"a"` not `"b"`).
**Planned fix:** none — accepted as a language semantic.  Capture-at-definition
is the intended behaviour and the dead-assignment warning is informative.

---

## C33 — Interfaces: factory methods (`fn zero() -> Self`) not supported

An interface method without a `self` parameter (a factory/constructor) cannot be
declared.  The parser requires at least one `Self`-typed `self` parameter in every
interface method body.

**Reproducer:**
```loft
interface Addable {
    fn OpAdd(self: Self, other: Self) -> Self
    fn zero() -> Self        // ERROR: factory method not yet supported
}
```

**Impact:** `sum`-style generic functions that need an identity element must instead
accept `zero: T` as an explicit argument (the workaround used in the stdlib).
**Workaround:** pass the identity value as an extra parameter:
```loft
fn sum<T: Addable>(v: vector<T>, zero: T) -> T { ... }
```
**0.8.3 mitigation (I12.diag):** the compile error will be extended to include the
workaround hint in its message.
**Full fix:** I12 (factory-method restriction phase 2). Target: 1.1+.

---

## C34 — Interfaces: left-side concrete operand in binary operators not supported

When a bounded generic `T` is the **right** operand of a binary operator and a
concrete type is on the left (`3 * t`), the compiler does not resolve the operator
through the interface.  Only `T op T` and `T op Concrete` (where `T` is on the
left) work.

**Reproducer:**
```loft
interface Scalable { fn scale(self: Self, factor: integer) -> Self }
fn double<T: Scalable>(x: T) -> T { 2 * x }   // ERROR: 2 is concrete, x is T
fn double<T: Scalable>(x: T) -> T { x.scale(2) }  // OK: method call workaround
```

**Impact:** expressions where a primitive literal or concrete value must be the
left operand fail to compile inside a bounded generic.  Rewrite as a method call
on the `T` value or put `T` on the left.
**Workaround:** define and use the operator with `T` on the left: `x * 2` instead
of `2 * x`; or use a named method (`x.scale(2)`).
**0.8.3 mitigation (I8.5.diag):** the compiler will detect the concrete-left/generic-right
pattern specifically and emit an error that names the ordering problem and suggests
the workaround, replacing the generic "requires a concrete type" message.
**Full fix:** I8.5 (mixed-type operator, concrete left side). Target: 1.1+.

---

## C35 — Bounded generic returning `text` from struct type causes memory violation

A bounded generic function whose return type is `text` crashes at runtime with
`unsafe precondition(s) violated: ptr::copy_nonoverlapping` when instantiated with
a struct type.  The crash is in the generic specialisation's text-return path.

**Reproducer:**
```loft
interface Describable { fn describe(self: Self) -> text }
struct Temperature { celsius: float }
fn describe(self: Temperature) -> text { "{self.celsius}C" }
fn show<T: Describable>(item: T) -> text { describe(item) }
fn main() {
    t = Temperature { celsius: 37.0 };
    s = show(t);   // crashes — text return from bounded generic on struct
}
```

**Impact:** bounded generic functions that return `text` (e.g. `show<T: Printable>`)
cannot be used with user-defined struct types.  Primitive types (`integer`, `float`,
`boolean`) are unaffected.
**Workaround:** call the concrete function directly: `describe(t)` instead of `show(t)`.
**Planned fix:** needs investigation — likely in the generic specialisation's text
return handling (`src/state/codegen.rs`).  No milestone assigned yet.

---

## C36 — Generic function with `for` loop: slot assignment panic on struct-type instantiation

A bounded (or unbounded) generic function that contains a `for` loop panics with
`variable 'X' never assigned a slot` when instantiated with a struct type.
The slot assignment algorithm does not correctly handle struct-typed local variables
inside generic specialisations that contain loops.

**Reproducer:**
```loft
struct Score { value: integer }
fn OpLt(self: Score, other: Score) -> boolean { self.value < other.value }
fn largest<T: Ordered>(items: vector<T>) -> T {
    best = items[0];
    for i in 1..len(items) {
        if best < items[i] { best = items[i]; }
    }
    best
}
fn main() {
    scores = [Score{value: 3}, Score{value: 7}];
    w = largest(scores);   // panics — 'best' never assigned a slot
}
```

**Impact:** generic functions over struct types cannot contain `for` loops.
Use direct index access where possible, or write separate concrete functions.
**Workaround:** for two-element cases use a simple `if` comparison.
For larger collections write a concrete (non-generic) function with the loop.
**Planned fix:** needs investigation in slot assignment for generic specialisations
(`src/state/codegen.rs` § slot assignment).  No milestone assigned yet.

---

## C37 — Calling the same generic function with two different struct-based types: slot conflict

When the same generic function (e.g. `max_of<T: Ordered>`) is called with two
different **struct** types in the same file, both specialisations share local variable
names in the flat namespace.  The second specialisation's variables cannot be assigned
slots and the runtime panics: `variable 'result' never assigned a slot`.

**Reproducer:**
```loft
struct Score { value: integer }
fn OpLt(self: Score, other: Score) -> boolean { self.value < other.value }
fn main() {
    a = max_of([4, 1, 9, 2]);              // max_of<integer> — OK alone
    scores = [Score{value: 3}, Score{value: 7}];
    b = max_of(scores);                    // max_of<Score> — slot conflict
}
```

**Impact:** a generic function can only be instantiated with **one** concrete struct
type per loft file.  Instantiation with two or more struct types in the same file panics.
**Workaround:** write separate concrete wrapper functions for each struct type.
**Planned fix:** needs investigation in flat-namespace slot assignment during generic
specialisation (`src/state/codegen.rs`).  No milestone assigned yet.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker with severity and fix paths
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — slot assignment bug classes
- [SAFE.md](SAFE.md) — safety analysis for parallel workers and coroutines
- [LOFT.md](LOFT.md) § Known Limitations — user-facing summary
