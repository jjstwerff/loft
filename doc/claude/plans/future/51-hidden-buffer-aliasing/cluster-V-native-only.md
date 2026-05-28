# Cluster V — Native-only failures

**Status: 🟡 NOT STARTED.**  Probe 29's native mechanism is VERIFIED (OpFreeRef-after-OpCopyRecord visible in generated Rust at `/tmp/loft_native_*.rs:2260-2261`; see § Probe 29 below).  Probe 30's mechanism still hypothesized — capture its generated Rust via `LOFT_KEEP_NATIVE_RS=1` (tool 1, in tree) and read the lambda dispatch in `src/generation/`.

**Next-session entry point for probe 29 fix:**

1. Find where `rewrite_tail_tuple_to_synthetic_struct` (`src/parser/control.rs:677-720`) emits the IR — specifically the OpCopyRecord pattern that the generated Rust mirrors.
2. Read `OpCopyRecord` runtime semantics in `src/codegen_runtime.rs` (native) vs `src/state/io.rs` (bytecode VM) to see WHY interpret escapes (deep-copy semantics? source-free flag `0x8000`?).
3. Read `src/scopes.rs::get_free_vars` to find where the trailing `OpFreeRef(var_a)` / `OpFreeRef(var_b)` are emitted (scope-exit free vs. tuple-rewrite output).
4. Fix candidates: (b) suppress trailing OpFreeRef when OpCopyRecord adopted them; (c) use OpCopyRecord-with-source-free-flag (`0x8000`) so the source is freed AS PART of the copy.

**Severity:** Mixed.  Native codegen produces invalid or buggy Rust for specific shapes.  Some shapes work on interpret but fail on native (29).  Some fail on both backends differently (30).
**Affected probes:** 29 (tuple-return), 30 (lambda-return)
**Backend asymmetry:** Opposite from clusters II/III — here NATIVE fails (interpret may or may not work).

---

## Probe 29 — tuple-of-heap-structs return

```loft
fn split(p: P) -> (Canvas, Canvas) {
  a = alloc_canvas(4, 5, p.tag);
  b = alloc_canvas(7, 9, p.tag * 2);
  (a, b)
}

fn main() {
  for i in 0..6 {
    (ca, cb) = split(P { tag: i });
    assert(cb.data[0] == i * 2, ...);
  }
}
```

**Interpret:** PASSES.
**Native:** COMPILES + RUNS, but assertion fires at iter 0: `cb[0] = null(oob)`.

The generated Rust at `/tmp/loft_native_*.rs:775:14` panics on the assertion message itself.  The compiled binary executes, but the SECOND tuple element (`cb`) is empty / corrupted on iter 0.

### Hypothesis

The native codegen for tuple-of-heap-structs has a buffer-passing bug.  Tuples in loft are represented internally as synthetic struct types (`__tuple<Canvas, Canvas>`).  ref_return promotes the tuple-typed local to a hidden buffer.  Native lowering of "tuple containing two heap fields" must:

1. Allocate a tuple record.
2. Allocate Canvas a (first field).
3. Allocate Canvas b (second field).
4. Write a and b into the tuple's fields.

Possible bug: native emits writes for the first field (a) but the second field write is silenced, OR the second field is written to the wrong slot.

Interpret handles this correctly via its `Insert`-style codegen for tuple construction (separate field-init opcodes).

### What to investigate

1. Locate the native codegen for `Value::Tuple(elements)` and synthetic-`__tuple` types in `src/generation/`.
2. Capture the generated Rust file from `/tmp/loft_native_*.rs` BEFORE the binary runs (use `LOFT_KEEP_NATIVE_RS=1` if such a flag exists; otherwise run with build-only).
3. Inspect lines around 775 in the generated file to see how the tuple's two Canvas fields are populated.

---

## Probe 30 — lambda returning heap struct

```loft
fn main() {
  make_renderer = fn(p: P) -> Canvas {
    cv = alloc_canvas(4, 5, p.tag);
    cv
  };
  for i in 0..6 {
    p = P { tag: i };
    cv = make_renderer(p);
    assert(cv.data[0] == i, ...);
  }
}
```

**Interpret:** Assertion fails at `iter 65535: data[0] = null(oob)`.  The `65535` is u16::MAX — the LOOP VARIABLE `i` itself has been corrupted to the null sentinel.

**Native:** Panics in the generated Rust at line 2264.

### Symptom analysis — interpret

When the assertion message reads `iter 65535: data[0] = null(oob)`, the loop variable `i` was overwritten to `65535` somewhere during the iteration.  Lambda execution corrupted main's stack frame.

Lambda values in loft are represented as `(d_nr: u32, closure_capture: DbRef)` 16-byte fn-ref slots.  Calling a lambda likely involves:

1. Loading the fn-ref from main's slot.
2. Setting up the lambda's frame (its captured args + locals).
3. Executing the lambda body.
4. Returning, restoring main's stack pointer.

If the lambda's frame setup oversteps and writes into main's frame's slot (where `i` lives), main's loop variable gets corrupted.

### Symptom analysis — native

Native panics at line 2264 of the generated Rust.  Without the file content, the exact bug isn't visible.  Possible causes:
- Native codegen doesn't handle ref_return-promoted return for lambda bodies.
- Native's lambda dispatch ABI doesn't pass the hidden buffer through correctly.

### Hypothesis

Lambdas have a distinct codegen path from named function calls.  The fn-ref dispatch + closure-capture mechanism wasn't updated to handle ref_return-promoted hidden buffer args.  Specifically:

- For a named `fn f(p, __hidden) -> T`, the caller pushes `(p, __hidden)` as args and Call(f) does the work.
- For a lambda `make_renderer = fn(p) -> T { ... }`, the call goes through fn-ref dispatch (`OpCallRef` or similar), which might not have the hidden-buffer arg path wired in.

Result: the lambda body uses some stack location that overlaps with the caller's frame, corrupting the caller's `i`.

### What to investigate

1. Read the fn-ref dispatch codegen — `OpCallRef` or `Value::CallRef` handling in both `src/state/codegen.rs` and `src/generation/`.
2. Check whether lambdas engage `ref_return` and `add_defaults` like named fns do.
3. Trace probe 30 with `LOFT_LOG=ref_debug,type_timeline:i` to see when `i` flips to 65535.

---

## Common thread

Both probes 29 and 30 are "specific value-shape" issues:

- **29:** the value-shape is a TUPLE of heap structs (multiple hidden buffers wrapped in a tuple).
- **30:** the value-shape is a LAMBDA returning a heap struct (different dispatch mechanism).

Native's codegen handles the common cases (named fn calls, struct construction with single heap field) but has gaps for these less-common shapes.

## Fix surface

These are **separate from the runtime-ownership class** that Path C addresses.  They're native-codegen-specific shape gaps.

**(a) Per-shape native codegen fixes.**  Read the generated Rust for each shape, identify the bug, patch the corresponding codegen path.  Effort: S–M per shape; ~M total.

**(b) Bypass — disallow these shapes until fixed.**  Add a parser-level "not yet supported under --native" warning when these shapes appear.  Effort: trivial; ships honesty.

**Most likely best path: (a) per-shape fixes.**  These are native-specific bugs separate from the broader runtime-ownership class; they need their own targeted work.  Probably worth a separate plan slot or absorbing into an existing native plan (`plans/finished/N-native-*` history shows N1-N8 patches).

## What we know vs. don't

| | Status |
|---|---|
| Both probes fail on native | ✅ Verified |
| Probe 29 native runs but assertion fires (codegen-bug-but-not-crash) | ✅ Verified |
| Probe 30 native crashes earlier in Rust panic | ✅ Verified |
| Probe 30 interpret corrupts the caller's stack frame | ✅ Verified (iter=65535 evidence) |
| Probe 29 interpret PASSES | ✅ Verified |
| Slot allocation on INTERPRET side is clean for both probes | ✅ Verified via `LOFT_LOG=slots:` — no unallocated vars |
| **Probe 29 native mechanism — generated Rust analysis** | ✅ Verified via `LOFT_KEEP_NATIVE_RS=1` (tool 1) and reading `/tmp/loft_native_*.rs` |

### Probe 29 — verified native mechanism

With `LOFT_KEEP_NATIVE_RS=1`, the generated Rust at `/tmp/loft_native_242750.rs` shows:

```rust
fn n_split(cell, var_p) -> DbRef {
  let mut var___ref_3: DbRef = DbRef { store_nr: u16::MAX, ... };
  let mut var___ref_2: DbRef = DbRef { store_nr: u16::MAX, ... };
  let mut var___ref_1: DbRef = DbRef { store_nr: u16::MAX, ... };
  let mut var_a: DbRef = n_alloc_canvas(cell, 4, 5, ..., var___ref_1);
  let mut var_b: DbRef = n_alloc_canvas(cell, 7, 9, ..., var___ref_2);
  {  // synthetic_tuple_return: ref(__tuple<Canvas,Canvas>)["__ref_3"]
    var___ref_3 = OpDatabase(cell, var___ref_3, 66_i32);
    OpCopyRecord(cell, var_a, tuple_field_0(var___ref_3), 65_i32);
    OpCopyRecord(cell, var_b, tuple_field_16(var___ref_3), 65_i32);
    OpFreeRef(cell, var_b, "var_b"); var_b.store_nr = u16::MAX;  // <-- BUG
    OpFreeRef(cell, var_a, "var_a"); var_a.store_nr = u16::MAX;  // <-- BUG
    if var___ref_1.store_nr != var_a.store_nr { OpFreeRef(...) };
    if var___ref_2.store_nr != var_b.store_nr { OpFreeRef(...) };
    return var___ref_3
  }
}
```

**The bug:** after `OpCopyRecord` deep-copies var_a and var_b INTO the tuple's fields, the code emits **`OpFreeRef(var_a)` and `OpFreeRef(var_b)`**.  This deep-frees var_a's data vector (a child store of the Canvas record) — but `OpCopyRecord` produced a **shallow copy** (the tuple's Canvas field's `data` field DbRef-aliases var_a's vector store).  When var_a is freed, that vector is freed too, leaving the tuple's first Canvas's `data` field dangling.

Then `var_b.store_nr = u16::MAX` zeroes out var_b's slot.  The subsequent `if var___ref_2.store_nr != var_b.store_nr` is `(real) != u16::MAX = true`, so __ref_2 gets freed too — but the tuple's second Canvas's data still aliases __ref_2's vector.  Result: both Canvas fields in the returned tuple have dangling data vectors.

Interpret escapes this because OpCopyRecord on interpret apparently does a DEEP copy (allocates fresh stores for children) OR doesn't engage the same free sequence.

### Probe 30 — partially verified

The lambda body's generated Rust would show the same OpCopyRecord-then-free pattern with closure-frame-corruption risk.  Not yet inspected; need to capture probe 30's `.rs` file.

| | Status |
|---|---|
| Native codegen path for tuple-of-heap-structs | ✅ Pattern identified in generated Rust |
| Native codegen path for lambda-with-heap-return | 🤔 Not yet captured |
| Why interpret escapes the same bytecode pattern | 🤔 OpCopyRecord interpret-side might do deep-copy where native generates shallow |

## Investigation tasks

1. Find a flag or method to keep the `/tmp/loft_native_*.rs` file after run; inspect lines 775 (probe 29) and 2264 (probe 30) for the bugs.
2. Read `src/generation/` tuple-return path.
3. Read `src/generation/` lambda-dispatch path; specifically how `OpCallRef` (or whatever the native equivalent is) handles a fn-ref's hidden buffer args.
4. Trace probe 30 interpret with `LOFT_LOG=ref_debug,type_timeline:i` to pin which opcode corrupts `i`.
