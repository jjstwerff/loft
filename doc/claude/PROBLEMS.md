
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Problems in Loft

This document lists known bugs, unimplemented features, and limitations in the loft
language and its interpreter (`loft`). For each issue the workaround and the
recommended fix path are described.

Completed fixes are removed — history lives in git and CHANGELOG.md.

## Contents
- [Open Issues — Quick Reference](#open-issues--quick-reference)
- [Unimplemented Features](#unimplemented-features)
- [Interpreter Robustness](#interpreter-robustness)
- [Web Services Design Constraints](#web-services-design-constraints)

---

## Open Issues — Quick Reference

| # | Issue | Severity | Workaround? |
|---|-------|----------|-------------|
| 22 | Spatial index (`spacial<T>`) operations not implemented | Low | Compile-time error; use `sorted<T>` or `index<T>` |
| 54 | `json_items` returns opaque `vector<text>` | Low | Accepted limitation; `JsonValue` enum deferred |
| 55 | Thread-local `http_status()` not parallel-safe | Medium | Design constraint — use `HttpResponse` struct |
| 85 | Struct-enum local variable leaks stack space | Low | Pass as parameter instead of local |
| 86 | Lambda capture: misleading self-reference error | Low | Mitigated — clear error message |
| 89 | Hard-coded StackFrame field offsets | Low | Do not reorder `04_stacktrace.loft` fields |
| 90 | `fn_call` HashMap lookup per call | Low | Negligible overhead |
| 91 | `init(expr)` parameter form missing | Low | Pass default explicitly at call site |
| 92 | `stack_trace()` empty in parallel workers | Low | Call from main thread only |
| 128 | File-scope constant type annotations rejected | Low | Drop the annotation |
| 129 | Native codegen: duplicate `extern crate` | Medium | Use `--interpret` mode |
| 130 | Headless GL: foreign exception panic | Medium | Check `gl_create_window` return |
| 131 | CLI consumes script arguments | Low | Hard-code the mode for now |
| 133 | RGB/BGR channel swap in GL output | Low | Pre-swap channels at call sites |

---

---

## Unimplemented Features

### 22. Spatial index operations are not implemented

**Pre-gate fix (2026-03-15):** `spacial<T>` in any field or variable type now emits a
compile-time error:
```
spacial<T> is not yet implemented; use sorted<T> or index<T> for ordered lookups
```
Both first-pass and second-pass are covered; no program can reach the runtime `Not implemented`
panics via `spacial<T>` anymore.  Test: `spacial_not_implemented` in `tests/parse_errors.rs`.

**Remaining work:** The full implementation (insert, lookup, copy, remove, iteration) is still
missing.  Best way forward: implement one operation at a time in `database.rs` and `fill.rs`,
starting with iteration, then remove, then copy.  The spacial index structure (radix tree or
R-tree) is already allocated in the schema; the iteration traversal is the main missing piece.

---

## Web Services Design Constraints

### 54. `json_items` returns opaque `vector<text>` — no compile-time element type
**Severity:** Low — accepted design limitation
**Description:** `json_items(body)` parses a JSON array and returns the element bodies as
`vector<text>`.  There is no way for the compiler to verify that the caller's parse function
(e.g. `User.from_json`) receives a valid JSON object body rather than an arbitrary string.
A parse error at runtime produces a partial zero-value struct, not a diagnostic.
**Workaround:** Validate the HTTP response status before parsing (`if resp.ok()`).
**Fix path:** Deferred.  A `JsonValue` enum (covering Object, Array, String, Number, Boolean,
Null variants) would give compile-time structure, but the design cost is high.
**Effort:** Very High (deferred)
**Target:** 1.1+
**See also:** [WEB_SERVICES.md](WEB_SERVICES.md)

---

### 55. Thread-local `http_status()` is not parallel-safe
**Severity:** Medium — design trap; do not introduce this API
**Description:** An `http_status()` function returning the status of the most recent HTTP
call as a thread-local integer (the pattern used by C's `errno`) is tempting but incorrect
in loft's parallel execution model.  A `parallel_for` worker calling `http_get` would
corrupt the thread-local of the calling thread.
**Fix path:** Return an `HttpResponse` struct directly from all HTTP functions.  The status
is a field on the returned value, not global state.  See WEB_SERVICES.md Approach B.
**Effort:** N/A — this is a design constraint, not a bug to fix.  Simply do not add `http_status()`.
**Target:** Avoided by design

---

## Interpreter Robustness

### 61, 68–70, 79. Fixed

- **61** Native codegen IR parsing — all critical opcodes in `codegen_runtime.rs`.
- **68** `first_set_in` renamed to `inline_ref_set_in`, handles `Block`/`Loop` exhaustively.
- **69** Text slot reuse reverted; `can_reuse` restricted to `var_size <= 8`. Workaround applied.
- **70** `Type::Text` in `generate_set` pos-override reverted. Workaround applied.
- **79** `external` crate reference — `mod external` block emitted by native codegen.

---





### 85. Struct-enum local variable leaks stack space *(fixed, C41)*

**Test:** `tests/scripts/71-caveats-problems.loft::test_p85_struct_enum_local` (passes — guard).

**Symptom:** Constructing a struct-enum variant as a local variable and returning a
scalar from the function triggers a debug-mode assertion in `fn_return`:

```
assertion `left == right` failed: Stack not correctly cleared: 8 != 4
```

**Reproducer:**
```loft
fn test() -> integer {
    v = IntVal { n: 42 };
    match v { IntVal { n } => n, _ => 0 }
}
```

The struct-enum reference (12-byte `DbRef`) is allocated on the stack but not freed
before return.  The assertion is gated by `cfg!(debug_assertions)` so release builds
are unaffected, but the leaked stack space is real in both modes.

**Workaround:** Pass the enum value as a function parameter instead of storing it in a
local:
```loft
fn check(v: ArgValue) -> integer { match v { IntVal { n } => n, _ => 0 } }
check(IntVal { n: 42 })
```

**Root cause:** Scope analysis (`scopes.rs`) does not emit `OpFreeRef` for struct-enum
locals whose lifetime ends at the function return.  The `text_positions` cleanup in
`fn_return` handles orphaned text values but not reference-type values.

**Fix path:** Extend scope exit in `scopes.rs::free_vars()` to emit `OpFreeRef` for
struct-enum locals, or ensure the codegen marks such variables with a correct live
interval so the existing cleanup path handles them.

**Discovered:** 2026-03-26, during TR1.2 testing.

---

### 86. Lambda capture produced misleading codegen self-reference error

**Symptom:** A lambda that referenced an outer-scope variable crashed in codegen with:

```
[generate_set] first-assignment of 'count' (var_nr=1) in 'n___lambda_0'
contains a Var(1) self-reference — storage not yet allocated
```

**Reproducer:**
```loft
fn test() {
    count = 0;
    f = fn(x: integer) { count += x; };
    f(1);
}
```

The parser created a new local `count` inside the lambda, but `count += x` desugars to
`count = count + x` — the RHS reads the same uninitialized variable, triggering the
self-reference guard in `generate_set`.

**Status:** *(mitigated by A5.1)* — The parser now detects the outer-scope reference
and emits a clear error ("lambda captures variable 'count' — closure capture is not yet
supported") before codegen runs.  The underlying issue (no actual closure capture) is
tracked as A5.2–A5.5.

**Discovered:** 2026-03-26, during A5.1 testing.

---

### 89. Hard-coded `StackFrame` field offsets in `n_stack_trace`

**Symptom:** `n_stack_trace` in native.rs writes StackFrame fields at hard-coded byte
offsets (0, 4, 8) that must match the field order in `default/04_stacktrace.loft`.
If the struct definition is reordered, fields are renamed, or types change, the
native function silently writes to wrong positions — producing garbage values at
runtime with no compile-time or startup check.

**Root cause:** Native functions cannot call into the type system at runtime.  The
field layout is determined by `calc::calculate_positions` during compilation, but
`n_stack_trace` hard-codes the result.

**Workaround:** Do not modify the StackFrame struct without updating the offsets in
`n_stack_trace`.

**Fix path:** At startup (in `native::init` or `compile::byte_code`), look up the
StackFrame type's field positions from the database schema and store them in a
struct on State.  The native function reads positions from that struct instead of
using literals.  Alternatively, assert the expected layout at startup and panic
with a clear message if it doesn't match.

**Discovered:** 2026-03-26, during TR1.3 implementation.

---

### 90. `fn_call` HashMap lookup for line number on every call

**Symptom:** TR1.4 added `self.line_numbers.get(&self.code_pos)` to `fn_call`,
which runs on every loft function call.  Before TR1.4, the line lookup only happened
during the rare `stack_trace()` snapshot.  This adds a HashMap probe to the hot path.

**Root cause:** The source line is not encoded in the OpCall bytecode operands.
It is stored in a separate `line_numbers: HashMap<u32, u32>` keyed by bytecode
position, and must be looked up at runtime.

**Workaround:** None needed — the overhead is small (O(1) amortised HashMap lookup)
relative to the `Vec::push` and function dispatch already in `fn_call`.

**Fix path (if measured as significant):** Encode the source line as an additional
OpCall bytecode operand (u32) in codegen.  The `call` handler in fill.rs reads it
and passes it to `fn_call`, eliminating the runtime lookup entirely.  This would
increase each OpCall instruction by 4 bytes.

**Discovered:** 2026-03-26, during TR1.4 implementation.

---

### 91. L7 `init(expr)` missing circular-init detection and parameter form

**Symptom:** Two `init` fields that reference each other via `$` (e.g. `a: integer
init($.b)` and `b: integer init($.a)`) are not detected at compile time.  At runtime
the behaviour is undefined — the fields may read uninitialised memory or produce
garbage values.  Additionally, `init(expr)` on function parameters (dynamic defaults
computed from earlier parameters) is not implemented.

**Scope:** The core struct-field `init(expr)` works correctly: evaluated once at
creation, `$` references resolved, writable after construction.  Only the safety
guard (circular detection) and the convenience extension (parameter form) are missing.

**Workaround:** Do not write two `init` fields that reference each other.  For
dynamic parameter defaults, compute the default at the call site and pass it
explicitly.

**Fix path:**
1. Circular detection: after parsing all struct fields, collect `init` fields, walk
   each init expression for `$.<field>` accesses, build a directed graph, DFS for
   cycles, emit `diagnostic!(Level::Error, ...)`.
2. Parameter form: in `parse_arguments`, accept `init(expr)` alongside `= expr`;
   store the expression in `Attribute.value`; at the call site, emit the expression
   when no argument is supplied.

**Circular detection:** Fixed — `= expr` shorthand now enables `init_field_tracking`,
matching the `init(expr)` path.  **Test:** `tests/scripts/72-parse-error-caveats.loft` (`@EXPECT_ERROR`).
**Parameter form:** Still missing.

**Discovered:** 2026-03-26, during L7 implementation.

---

### 92. `stack_trace()` in parallel workers returns empty

**Symptom:** Calling `stack_trace()` from inside a parallel `for` loop body returns
an empty vector.  The function does not panic — it silently produces zero frames.

**Root cause:** The `execute_at` / `execute_at_raw` / `execute_at_ref` functions used
by parallel workers do not set `State.data_ptr`.  The `static_call` snapshot check
sees `data_ptr.is_null()` and skips the snapshot.

**Workaround:** Call `stack_trace()` from the main thread only.

**Fix path:** Set `data_ptr` from the `ParallelCtx.data` pointer at the start of each
`execute_at` variant, or pass it through the `WorkerProgram` struct.

**Discovered:** 2026-03-26, during fix #87/#88 implementation review.

---

### 93, 97–109, 119, 134. Fixed

- **93** Tuple-in-struct-field rejected at compile time. **Test:** `72-parse-error-caveats.loft`.
- **97** Compound assignment on tuple destructuring rejected. **Test:** `72-parse-error-caveats.loft`.
- **98** Index range query with descending key. **Test:** `71-caveats-problems.loft::test_p98_*`.
- **99** Empty struct comprehension + hash types crash. **Test:** `69-ignored-empty-comprehension-hash.loft`.
- **100** Format alignment ignored for numbers. **Test:** `67-ignored-format-align.loft`.
- **101** Float `:.0` precision ignored. **Test:** `68-ignored-float-precision-zero.loft`.
- **102** `rev(vector)` compile error. **Test:** `66-ignored-rev-vector.loft`.
- **103** Inline vector concat in compound assignment — now a compile error.
- **104** Test runner filtered by source file. **Test:** `76-ignored-struct-vector-return.loft`.
- **105** Nested struct field access on vector elements. **Test:** `76-ignored-struct-vector-return.loft`.
- **106** Store corruption with nested struct assignments. Same fix as P105.
- **107** `++` rejected at parse time with clear error. **Test:** `72-parse-error-caveats.loft`.
- **108** `f#next` initial seek applied on first file open.
- **109** Struct field reassignment with nested vector: `remove_claims` + `set_skip_free(elm)`.
- **119** Native OpenGL `n_` functions registered under `loft_` names for auto-marshaller.
- **134** `gl_load_font` sentinel mismatch — now returns `i32::MIN` on failure.

### 117, 120–127. Fixed

- **117** Struct-text-param store leak — verified with 2000-iteration GL-pattern tests in debug.
- **120** Struct field overwrite leak — high-bit on CopyRecord type in `copy_ref()`.
  6 isolation tests + 2 GL-pattern tests pass in debug mode.
- **121** Tuple heap corruption — verified with sustained-loop + nested-ops tests in debug.
- **122** Store leak in game loops — 100k-iteration stress test + GL collision/mat4 tests pass.
- **123** Per-frame vector literal leak — 1000-frame + multi-vector tests pass in debug.
- **124** Native inline array indexing — verified via `--native-emit`, no `as DbRef` cast.
- **125** `use` import sibling packages — `lib_path` walks up to `loft.toml`.
- **126** Negative tail expression — test un-ignored, passes.
- **127** File-scope vector constants — pre-built in CONST_STORE via `OpConstRef`.
  See [CONST_STORE.md](CONST_STORE.md).


### 128. File-scope constants reject type annotations with misleading error

**Status (2026-04-11):** ❌ **Open — confirmed with pure-loft unit tests.**

Two new GL-pattern tests reproduce the bug *without* GL or Xvfb:
- `p120_struct_return_in_conditional_in_loop` — overwriting `node.xform`
  with `make_transform(t)` inside a conditional inside a 1000-iteration
  loop. **Passes in release, fails in debug:** `Database 3 not correctly freed`.
- `p120_multi_node_transform_update` — overwriting `.pos` on 4 scene
  nodes per frame for 500 frames. **Passes in release, fails in debug:**
  `Database 9 not correctly freed`.

The root cause is now clear: when a struct field is overwritten with the
result of a struct-returning function call, the old store (holding the
previous field value) is not freed. Each overwrite leaks one store. In
release mode this is silent; in debug mode the cleanup check at program
exit detects the orphaned stores.

This is the same bug as the GL renderer crashes (11 of 26 examples
failing under Xvfb) — the renderer overwrites node transforms per frame
via `mat4_trs()` / `mat4_look_at()`, leaking stores until the pool
exhausts or a locked-store assertion fires.

**Minimal reproducer (no loop needed):**
```loft
struct Inner { ix: float not null, iy: float not null }
struct Outer { pos: Inner }
fn make_inner(v: float) -> Inner { Inner { ix: v, iy: v * 2.0 } }
fn test() {
    o = Outer { pos: Inner { ix: 0.0, iy: 0.0 } };
    o.pos = make_inner(5.0);   // ← leaks store 3
}
```

**Execution trace analysis:** `make_inner` allocates store 3 for the
returned `Inner { ix: 5, iy: 10 }`. The caller emits `CopyRecord(data=
ref(3,1,8), to=ref(2,1,8), tp=48)` which copies the data into store 2
(the Outer's field). But no `OpFreeRef` is emitted for store 3 after the
copy — it becomes orphaned. The program exits with store 3 still
allocated. Debug mode catches this: `Database 3 not correctly freed`.

**Isolation tests:** 5 tests in `tests/issues.rs` (`p120_field_overwrite_
once/twice/short_loop/with_text` + `p120_local_overwrite_in_loop`).
Local variable overwrite (`x = make_inner(v)`) works correctly — only
the **struct field** path leaks.

**Fix needed:** After `CopyRecord` in the field-assignment codegen path,
emit `OpFreeRef` for the source reference (the function return's store).
The copy has already moved the data into the destination store, so the
source store can be safely freed. Touch points: `src/state/codegen.rs`
(`gen_set_first_ref_call_copy` or the field-set branch that calls
`CopyRecord`).

**Historical context:** 11 GL examples also panic with `Delete on locked
store (rec=360)` under Xvfb — that is a secondary symptom where the
leaked stores eventually trigger a locked-store assertion in `copy_record`.

**Fix needed:** `copy_record` (in `src/state/io.rs`) should detect when
the destination store is locked and either defer the delete or copy
into an unlocked scratch store. Touch points: `src/state/io.rs::copy_record`,
`src/database/allocation.rs::remove_claims`, `src/store.rs::Store::delete`.

**Historical reproducer (`lib/graphics/examples/test_mat4_crash.loft`)
still passes:**
```
inside make_big: data len=16
after return: data len=16
data[0]=0 data[15]=15
```
This simpler case is unaffected — proving the simplified test alone
isn't sufficient to validate the fix.

**Symptom:** Vector fields in returned structs are empty (length=0) or contain
garbage. Causes black textures in GL examples and use-after-free crashes when
the struct is used inside loops.

**Minimal reproducer:** `lib/graphics/examples/test_mat4_crash.loft`:
```loft
struct BigBox {
    width: integer,
    height: integer,
    data: vector<integer>
}
fn make_big() -> BigBox {
    d: vector<integer> = [];
    for y in 0..4 {
        for x in 0..4 { d += [x + y * 4]; }
    }
    BigBox { width: 4, height: 4, data: d }
}
fn main() {
    b = make_big();
    println("data len={b.data.len()}");   // prints 0, should be 16
}
```

**Also:** `tests/native_loader.rs::vec_from_returned_struct_heavy` — headless test.

**Root cause:** When `BigBox { width: 4, height: 4, data: d }` constructs the
struct, it allocates a new store (store 2) for the BigBox record and copies the
scalar fields (`width`, `height`) by value. But the `data` field is a
`vector<integer>` — the constructor only copies the **vector record pointer** (an
i32) from the stack store into the struct store. The actual vector data remains
in the stack store (store 1000/1).

When `make_big()` returns:
1. The callee's stack is unwound → vector data in the stack store is lost
2. `gen_set_first_ref_call_copy` deep-copies the struct from store 2 to the
   caller's store via `OpCopyRecord`
3. `copy_claims_seq_vector` reads the vector pointer from store 2, but it
   points to data in the (now-unwound) stack store → `length=0`

Scalar fields survive because they're copied by value. Vector fields are
pointers that become dangling after stack unwind.

**Blocks:** All OpenGL examples using struct returns with vector fields
(textured cube, breakout, scene graph). Also `Mat4 { m: vector<float> }` in
`math::mat4_mul`, `ortho()`, etc.

**Related:** Issue #117 (store leaks) and #119 (native registration).

**Fix direction:** The struct constructor must deep-copy vector field data
into the struct's own store, not just copy the pointer. This should happen at
the `FinishRecord` or `SetField` level — when a vector-typed field is assigned,
the vector data should be `copy_claims_seq_vector`'d from the source store
into the struct's store.

**Mitigations applied (partial):**
- Tolerate double-free in `free_ref` (skip already-freed stores)
- Loop pre-init hoists Reference variables to pre-loop scope
- `is_ret_work_ref` suppresses FreeRef for `__ref_N` in return path
- `gen_set_first_ref_call_copy` always deep-copies struct returns

---

### 121. Tuple literals crash interpreter with heap corruption

**Severity:** High (interpreter only; native codegen works)

**Status (2026-04-11):** **Fixed.** Verified with GL-pattern stress tests
in both release and debug mode:
- `p121_tuple_sustained_loop`: 1000-iteration loop creating and
  accessing tuple pairs — passes in debug with all assertions
- `p121_tuple_nested_operations`: swap operations on tuple elements,
  arithmetic on `.0` / `.1` fields, 500 iterations — passes in debug
- Original regression guards (`p121_float_tuple_literal_no_heap_corruption`,
  `p121_float_tuple_function_return`) also pass in debug

**Symptom:** Creating a tuple literal such as `a = (3.0, 2.0)` in interpreter
mode causes `corrupted size vs. prev_size` (glibc abort) or SIGSEGV.  Tuple
element access (`a.0`, `a.1`) also crashes.

**Reproducer:**
```loft
fn test() {
    a = (3.0, 2.0);
    assert(a.0 > 1.0, "tuple element");
}
```
Run with `loft --interpret test.loft` — aborts with heap corruption.

**Native mode:** `loft --native test.loft` works correctly.  Tuples in
`tests/docs/28-tuples.loft` pass in native mode.

**Workaround:** Use a struct instead of a tuple:
```loft
struct FloatPair { fx: float not null, fy: float not null }
a = FloatPair { fx: 3.0, fy: 2.0 };
```

**Root cause:** Likely a stack layout issue in the interpreter's tuple
codegen — the 16-byte (two floats) tuple allocation corrupts the heap
allocator metadata, suggesting an off-by-one or alignment error in
`OpTupleLiteral` or the stack reservation for tuple temporaries.

**Impact:** Tuples cannot be used in library code that must run in both
interpreter and native modes.  The `rect_overlap_depth` function in
`lib/shapes` uses a struct (`Overlap`) instead of a tuple for this reason.

---

### 122. Store leak: struct allocation inside game loop exhausts store pool

**Severity:** High (interpreter and native)

**Status (2026-04-11):** **Fixed.** Verified with GL-pattern stress tests
in both release and debug mode:
- `p122_gl_mat4_vector_field_per_frame`: 500 frames × 5 Mat4 allocations
  (struct with `vector<float>` field) = 2500 struct-with-vector allocs —
  passes in debug (67s) with all assertions
- `p122_gl_collision_struct_api`: 200 frames × 8 bricks with
  `Rect` + `Overlap` struct construction per collision check = 1600 checks —
  passes in debug (3.6s)
- `gl_combined_game_loop_stress`: full game-loop pattern combining Ball,
  Brick structs + vector + text per frame for 300 frames — passes in debug
- 100k-iteration stress test also passes (0.05s release, ~10min debug)

**Symptom:** After running for 30-60 seconds, the game panics with
`"Allocating a used store"`. Occurs in any tight loop that creates
struct instances (e.g. collision detection shapes).

**Root cause:** Each `shapes::Rect { ... }` or struct-returning function
call allocates a store. Inside a 60fps game loop with ~50 bricks checked
per frame, this exhausts the store pool within seconds. The stores are
not freed because the struct temporaries are created inside a loop body
(related to P117 store leak on struct returns).

**Workaround:** Use raw-float functions instead of struct-based APIs
in game loops. The `shapes` library provides `aabb_overlap(ax,ay,aw,ah,
bx,by,bw,bh)` and `aabb_depth_x`/`aabb_depth_y` for this purpose.

**Fix direction:** The interpreter should free struct temporaries at the
end of each loop iteration, not at function exit. This requires tracking
which stores were allocated within the loop body.

---

### 123. Per-frame vector literal allocation leaks stores

**Severity:** Medium (interpreter)

**Status (2026-04-11):** **Fixed.** Verified with GL-pattern stress tests
in both release and debug mode:
- `p123_gl_vector_per_frame_sustained`: 1000 frames, each building a
  12-element vector literal — passes in debug
- `p123_gl_multi_vector_per_frame`: 500 frames × 4 vectors (positions,
  normals, colors, indices) = 2000 vector allocations — passes in debug

**Symptom:** Code like `br_shown = [for _ in 0..8 { 0 }]` inside a
render loop allocates a new vector store every frame. After ~1000 frames
the store pool is exhausted.

**Workaround:** Use scalar variables, integer bitmasks, or pre-allocated
arrays initialized once outside the loop.

```loft
// BAD — allocates every frame
for _ in 0..1000000 {
    flags = [for _ in 0..8 { 0 }];  // store leak!
}

// GOOD — use bitmask
for _ in 0..1000000 {
    flags = 0;
    // set bit: flags = flags | (1 << i)
    // test bit: (flags & (1 << i)) != 0
}
```

---

### 124. Native codegen: inline array indexing generates invalid Rust *(fixed)*

**Fixed (2026-04-11).** Verified with `--native-emit`: the generated Rust
source for `[0.9, 0.2, 0.3][idx]` contains no `as DbRef` cast. The
function compiles and runs correctly in both `--native` and interpret modes.
**Tests:** `p124_function_returning_inline_array_index`,
`p124_local_array_index_workaround_works`.

---

### 125–126. Fixed

- **125** `use` import sibling packages — `lib_path` walks up to `loft.toml`.
- **126** Negative integer literal as final tail expression — bare `-1` after
  `if { return; }` no longer parsed as `void - 1`. **Test:**
  `tests/issues.rs::p126_negative_tail_expression_after_returns` (passes,
  verified 2026-04-11; previously `#[ignore]`d).

---

### 127. File-scope vector constant inlined into function call corrupts slots

**Severity:** Medium

**Symptom:** A file-scope constant holding a vector literal, when
referenced inside a function and passed as an argument, panics in
codegen with one of two flavours depending on context:

```
[generate_set] first-assignment of 'n' (var_nr=0) in 'n_test'
contains a Var(0) self-reference — storage not yet allocated, will
produce a garbage DbRef at runtime. This is a parser bug.
value=Call(502, [Block(Block { name: "Vector",
  operators: [Set(1, Call(312, [Var(0), Int(20), Int(65535)])), ...,
              Var(0)],
  result: Vector(Integer(...), []), scope: 2, var_size: 0 })])
```

Or, in a different reference site:

```
generate_call [n_F]: mutable arg 0 (data: Reference(265, []))
expected 12B on stack but generate(Var(0)) pushed 8B —
Value::Null in a typed slot? Missing convert() call in the parser?
```

Both errors come from sanity checks in `src/state/codegen.rs` and surface
the same underlying problem.

**Reproducer:**
```loft
QUAD = [1, 2, 3];
fn count(v: const vector<integer>) -> integer { v.len() }
fn test() {
  n = count(QUAD);              // panics in generate_set
  assert(n == 3, "got {n}");
}
```

The bug fires for `vector<integer>` and `vector<single>` constants alike.
The same literal works when declared as a local variable inside the
function instead of a file-scope constant.

**Tests:** `tests/issues.rs::p127_file_scope_vector_constant_in_call`
and `p127_file_scope_single_vector_constant` (both `#[ignore]`d).
`p127_inline_vector_literal_in_call_works` is the regression guard for
the working local-variable form.

**Root cause:** `parse_vector` (`src/parser/vectors.rs:1082`) builds a
vector literal as a `Value::Block` via `v_block()` (`src/data.rs:798`),
which sets `var_size: 0` and `scope: u16::MAX`. The Block's operator
list uses `Var(0)` and `Var(1)` for the temporary "current vector slot"
and "current element slot" used during the construction loop. When the
literal is parsed as the value of a `DefType::Constant`
(`src/parser/definitions.rs:407`), the IR is stored as-is. Each later
reference to the constant inlines the Block into the calling function's
IR — but the `Var(N)` indices are *not* rewritten and *not* offset, so
they collide with the calling function's local slots 0 and 1.

When the caller's slot 0 or 1 happens to be the variable being assigned
(`n = count(QUAD)` puts `n` at slot 0), the codegen sanity check spots
the self-reference and panics. When the caller's slots happen to be
unrelated locals, the check might not fire and the code would silently
write to the wrong slots — making this potentially a *latent* memory
corruption bug, not just a panic.

**Fix path (preferred order):**
1. **Re-emit the literal at every reference site.** Cleanest fix —
   when a constant of vector type is referenced, call back into
   `parse_vector` to emit a fresh Block with the caller's current
   `var_size` baseline. Loses constant deduplication but vector
   literals are large and rarely shared.
2. **Remap `Var(N)` indices when inlining.** Walk the constant's IR
   and replace each `Var(i)` with `Var(i + caller.var_size)`, then
   bump `caller.var_size` by `constant_block.var_size`. Requires
   tracking the *correct* `var_size` on the constant Block, which
   currently is 0 — also needs a fix.
3. **Constant-fold simple literal vectors at parse time.** If a vector
   literal is fully static (no side-effecting expressions), pre-allocate
   the storage at constant-init time and reference it as a static IR
   node that doesn't need temporaries at all. Best for performance but
   biggest implementation effort.

**Touch points:** `src/data.rs` (`v_block` and `Block::var_size`),
`src/parser/vectors.rs` (vector literal emission), `src/parser/objects.rs`
(`replace_record_ref` analogue for inlining), `src/parser/mod.rs` (constant
resolution path).

**Workaround:** Move the literal inline into the function that uses it.

**Found:** 2026-04-09 while declaring `UNIT_QUAD_2D` in
`lib/graphics/src/graphics.loft` for the `Painter2D` API.

---

### 128. File-scope constants reject type annotations with misleading error

**Severity:** Low

**Symptom:** Adding a type annotation to a file-scope constant:
```loft
QUAD: vector<integer> = [1, 2, 3];
```
produces three cascading parse errors at the colon position:
```
Error: Expect token = at file.loft:1:6
Error: Expect token ; at file.loft:1:6
Error: Syntax error: unexpected ':' at file.loft:1:6
```

Local variables accept the same annotation (`x: integer = 42;`), so the
asymmetry is surprising and the error chain is misleading — the colon is
actually the problem, but the diagnostic blames a missing `=`.

**Test:** `tests/parse_errors.rs::p128_constant_with_type_annotation`
locks in the current 3-error output.

**Root cause:** `parse_constant` (`src/parser/definitions.rs:392`) does:
```rust
if let Some(id) = self.lexer.has_identifier() {
    self.lexer.token("=");           // ← hard-codes `=` immediately
    ...
}
```
There is no provision for a `: type` annotation between the identifier
and the `=`.

**Fix path:** insert a `has_token(":")` branch in `parse_constant`,
parse the annotation via `parse_type`, and either (a) discard it and
continue with full inference, or (b) use it to constrain the inferred
type and error if the literal's element type is incompatible. Option
(b) gives better diagnostics for typos like `vector<single>` vs `vector<float>`.

```rust
pub(crate) fn parse_constant(&mut self) -> bool {
    if let Some(id) = self.lexer.has_identifier() {
        let mut declared_tp = Type::Null;
        if self.lexer.has_token(":") {
            declared_tp = self.parse_type();
        }
        self.lexer.token("=");
        // ... existing logic, optionally check inferred tp matches declared_tp
    }
}
```

**Workaround:** Drop the annotation; the literal's element type
(e.g. `f` suffix → `single`) is sufficient for inference.

**Found:** 2026-04-09 while declaring `UNIT_QUAD_2D` in graphics.loft.

---

### 129. Native codegen emits duplicate `extern crate` for graphics-using packages

**Severity:** Medium

**Symptom:** Running a script *outside* the loft repo that does
`use graphics;` (or transitively uses it) fails native compilation with
Rust E0259:

```
error[E0259]: the name `loft_graphics_native` is defined multiple times
  --> /tmp/loft_native.rs:18:1
   |
17 | extern crate loft_graphics_native;
   | ----- previous import of the extern crate `loft_graphics_native` here
18 | extern crate loft_graphics_native;
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ `loft_graphics_native` reimported here
```

The generated `/tmp/loft_native_*.rs` has the same `extern crate`
declaration emitted twice. The same error happens with `--check` because
`--check` runs through the native codegen path.

**Reproducer:**
```bash
cat > /tmp/test.loft << 'EOF'
use graphics;
fn main() { println("hi"); }
EOF
cargo run --bin loft -- --lib /home/ubuntu/loft/lib/ /tmp/test.loft
```

**Test:** Not added — would require a working out-of-repo native
toolchain in CI. The `--native-emit /tmp/foo.rs` flag is the easiest way
to inspect the generated source.

**Root cause hypothesis:** the native codegen walks the package
dependency graph to emit `extern crate` declarations and either visits
the `graphics` package twice (transitive + direct), or the first emission
happens during one phase (e.g. dependency scan) and the second during
another (e.g. native function lookup), with no de-duplication step in
between.

**Fix path:** maintain a `HashSet<String>` of already-emitted crate
names in `src/generation/mod.rs` (or wherever `extern crate` lines are
written), and skip subsequent emissions. Touch points: `src/generation/`
(emit logic) and `src/generation/dispatch.rs` (per-package walking).

**Workaround:** Run the script with `--interpret` instead (parser still
runs and types are still resolved), or place the script inside the loft
repo so it doesn't trigger the cross-package native build path.

**Found:** 2026-04-09 while parse-checking `Painter2D` additions outside
the repo.

---

### 130. Headless GL aborts via "Rust cannot catch foreign exceptions"

**Severity:** Medium (only affects headless test environments)

**Symptom:** Running any GL example without a display panics during
window creation, then a *second* panic happens that the runtime can't
catch and aborts the process:

```
loft_gl_create_window: EventLoop: os error ... :
  neither WAYLAND_DISPLAY nor WAYLAND_SOCKET nor DISPLAY is set.

thread '<unnamed>' panicked at .../gl-fe1d8.../bindings.rs:20624:13:
gl function was not loaded
fatal runtime error: Rust cannot catch foreign exceptions, aborting
```

The first panic is winit's event-loop creation failing — that one is
caught and `gl_create_window` returns `false`. The second panic happens
during cleanup or during a subsequent GL call: the gl bindings are
dispatched through function pointers that remain null when context
creation failed, and calling through a null pointer is C-side undefined
behaviour that the Rust runtime then refuses to unwind across.

**Test:** Not added — running the test would crash the test harness
itself. Reproduction requires running any of the 3D examples in
`lib/graphics/examples/` without `DISPLAY` set.

**Root cause hypothesis:** `loft_gl_create_window` (in
`lib/graphics/native/src/window.rs` or `src/lib.rs`) catches the winit
error and returns false, but the global GL context state in
`lib/graphics/native/src/lib.rs` is partially initialised and a
subsequent function call (perhaps during the script's `gl_destroy_window`
or during process exit's `Drop` impls) re-enters the gl bindings.

**Fix path:**
1. Initialise the global GL context as `None` and gate every native
   `loft_gl_*` function on `if context.is_none() { return Default::default(); }`.
2. Wrap winit window creation and the early GL function-pointer load in
   `std::panic::catch_unwind` so the foreign-exception path can never
   trigger.
3. As a smaller fix, audit `Drop` impls on the `Renderer` and any
   global `OnceCell`/`Lazy` GL state to make sure they no-op when GL was
   never initialised.

Touch points: `lib/graphics/native/src/lib.rs`, `lib/graphics/native/src/window.rs`.

**Workaround:** Don't run GL examples without a display. The script-side
`if !gl_create_window { return; }` guard fires correctly but doesn't
prevent the second panic.

**Found:** 2026-04-09 while parse-checking the rewritten 3D examples in
the headless sandbox.

---

### 131. Loft CLI consumes script-level arguments

**Severity:** Low

**Symptom:** Many graphics examples parse `arguments()` for `--mode glb`,
but invoking them as

```bash
loft 19-complete-scene.loft --mode glb
```

produces:
```
unknown option: --mode
usage: loft [options] <file>
```

The loft CLI parses `--mode` as one of its own options, sees nothing,
and exits before the script runs. As a related quirk, `arguments()`
called from inside the script returns the *full* loft argv including
loft's own flags (`--interpret`, `--path`, etc.), not just the
script-level args — so the example pattern of `for a in arguments() { … }`
to find `--mode` is broken even when no `--` is involved.

**Test:** `tests/exit_codes.rs::p131_cli_consumes_script_dashdash_arg`
locks in the current "exits non-zero with 'unknown option'" behaviour
so the fix can flip it cleanly.

**Root cause:** `src/main.rs` argument parser doesn't distinguish "loft
options" from "script arguments". Anything matching `--*` is treated
as a loft option, even after the script path has already been seen.
And `arguments()` is implemented as a thin wrapper over `std::env::args`
without filtering out the loft binary name and loft-recognised flags.

**Fix path:**
1. **Option parser:** in `src/main.rs`, once the positional script-path
   argument is consumed, treat every subsequent token as a script
   argument and stop interpreting `--*` as a loft option. Optionally
   also support an explicit `--` separator before script args, matching
   common Unix convention.
2. **`arguments()` builtin:** filter out the loft binary path and any
   tokens consumed by the loft CLI itself, so the script only sees
   what was passed *after* the script path.

Touch points: `src/main.rs` (CLI parser), `src/native.rs` or wherever
`n_arguments` is implemented.

**Workaround:** None at the CLI level. Hard-code the mode in the script,
or invoke a different entry function from the shebang.

**Found:** 2026-04-09 while trying to parse-check the GLB export path
of rewritten examples.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements for new features
