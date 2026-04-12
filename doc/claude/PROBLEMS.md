
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
| 86 | Lambda capture: misleading self-reference error | Low | Mitigated — clear error message |
| 90 | `fn_call` HashMap lookup per call | Low | Negligible overhead |
| 91 | `init(expr)` parameter form missing | Low | Pass default explicitly at call site |

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





### 85. Struct-enum local variable leaks stack space — **Fixed**

`scopes.rs::get_free_vars` already emits `OpFreeRef` for `Type::Enum(_, true, dep)`
(struct-enum) locals, so the original `Stack not correctly cleared: 8 != 4`
debug assertion no longer reproduces.
**Test:** `tests/scripts/71-caveats-problems.loft::test_p85_struct_enum_local`
runs the original failing pattern (`v = P85Int { n: 42 }; match v { ... }`) in
both release and debug — passes.

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

### 89. Hard-coded `StackFrame` field offsets in `n_stack_trace` — **Fixed**

`n_stack_trace` and `populate_frame_variables` (in `src/native.rs`) now look
up every field position via `stores.position(<type>, <field>)` instead of
writing to literal byte offsets.  Reordering or renaming a field in
`default/04_stacktrace.loft` updates the lookups automatically; a missing
field name fires a clear `assert_ne!` panic in both debug and release
mode (helper closure `lookup` in both functions).

**Test:** `tests/wrap.rs::p89_stacktrace_schema_fields_exist` validates
that every type and field name read by the native helpers exists in the
loaded schema — guards against future drift between the loft definition
and the Rust helpers.

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

### 92. `stack_trace()` in parallel workers — **Fixed**

`WorkerProgram` now carries `data_ptr`, `fn_positions`, and `line_numbers`
from the spawning State (`src/parallel.rs`, `src/state/mod.rs::worker_program`,
`src/state/mod.rs::parallel_join`, and the three `WorkerProgram { ... }`
construction sites in `src/native.rs`).  `State::new_worker` populates
the worker's matching fields so `static_call` can resolve frames to real
function names + source files instead of falling back to the placeholder
`<worker>`.  Topmost-frame line lookup falls back to `line_numbers[code_pos]`
when the worker's `CallFrame.line` is still 0 (par-block path; the
`n_parallel_for` path reports line 0 because `line_numbers` is not yet
threaded through `ParallelCtx`).

**Test:** `tests/threading.rs::parallel_stack_trace_resolves_worker_name`
runs the full `for ... par(...)` execution path and asserts that
`stack_trace()[0].function == "st_named_worker"` for every worker.

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

### 117, 120, 129–131. Fixed

- **117** Struct-text-param store leak — verified with 2000-iteration GL-pattern tests in debug.
- **120** Struct field overwrite leak — high-bit on CopyRecord type in `copy_ref()`.
  6 isolation tests + 2 GL-pattern tests pass in debug mode.
- **129** Duplicate `extern crate` in native codegen — dedup guard added to
  `lib_path_manifest`. Test: `p129_no_duplicate_native_packages`.
- **130** Headless GL foreign exception panic — `GL_READY` thread-local guard
  on all `gl::*` calls. Test: `p130_gl_functions_noop_without_context` in
  `lib/graphics/native/tests/headless_safety.rs`.
- **131** CLI consumes script arguments — `user_args` stored in `Stores`,
  `os_arguments()` returns them instead of raw `std::env::args`.
  Tests: `p131_cli_forwards_script_dashdash_arg`, `p131_cli_explicit_dashdash_separator`,
  `p131_arguments_returns_only_script_args`.

### 121–127. Fixed

- **121** Tuple-literal heap corruption in interpreter — verified with
  `p121_tuple_sustained_loop`, `p121_tuple_nested_operations`, and the
  original `p121_float_tuple_*` guards in debug mode.
- **122** Store leak in game loops — 100k-iteration stress test plus
  `p122_gl_mat4_vector_field_per_frame`, `p122_gl_collision_struct_api`,
  and `gl_combined_game_loop_stress` pass in debug.
- **123** Per-frame vector literal leak — `p123_gl_vector_per_frame_sustained`
  and `p123_gl_multi_vector_per_frame` pass in debug.
- **124** Native inline array indexing — `--native-emit` no longer emits
  `as DbRef` cast. Tests: `p124_function_returning_inline_array_index`,
  `p124_local_array_index_workaround_works`.
- **125** `use` import sibling packages — `lib_path` walks up to `loft.toml`.
- **126** Negative integer literal as final tail expression —
  `tests/issues.rs::p126_negative_tail_expression_after_returns`.
- **127** File-scope vector constants — pre-built in CONST_STORE via
  `OpConstRef`. See [CONST_STORE.md](CONST_STORE.md).

---

### 128. File-scope constants reject type annotations — **Fixed**

`parse_constant` accepts an optional `: type` annotation between the identifier
and `=`.  The annotation is parsed and discarded; the inferred type from the
initialiser is the source of truth.
**Test:** `tests/parse_errors.rs::p128_constant_with_type_annotation_parses`.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements for new features
