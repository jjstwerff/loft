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
| 22 | Spatial index (`spacial<T>`) operations not implemented | Low | N/A |
| 54 | `json_items` returns opaque `vector<text>` — no compile-time element type | Low | Accepted limitation; `JsonValue` enum deferred |
| 55 | Thread-local `http_status()` pattern is not parallel-safe | Medium | Use `HttpResponse` struct instead; do not add `http_status()` |
| 58 | Silent `Type::Unknown(0)` variable creation on unresolved names | High | N/A — check carefully for typos in Loft code |
| 60 | No recursion depth limit in codegen and parser traversals | Medium | N/A — only affects adversarially deep ASTs |
| 61 | Native codegen IR parsing panics on unhandled patterns | Medium | N/A — only affects `--native` path (not yet default) |
| 64 | Overflow risk in store offset arithmetic (`i32`/`usize` casts) | Medium | N/A — only affects extremely large records |
| 66 | Integer cast truncation in vector index/size computations | Medium | N/A — only affects very large vectors |
| 79 | Native codegen: `external` crate reference not resolved (random/FFI) | Low | `--native` only; affects `21-random.loft` |
| 85 | Struct-enum local variable leaks stack space (debug assertion) | Low | Pass as parameter instead of local |
| 86 | Lambda capture produced misleading codegen self-reference error | Low | *(mitigated by A5.1)* — clear error now |
| 89 | Hard-coded StackFrame field offsets in `n_stack_trace` | Low | N/A — offsets must match `04_stacktrace.loft` |
| 90 | `fn_call` HashMap lookup for line number on every call | Low | N/A — small overhead relative to dispatch |
| 91 | L7 `init(expr)` missing circular-init detection and parameter form | Low | Avoid circular `$` references between init fields |
| 92 | `stack_trace()` in parallel workers returns empty | Low | Call from main thread only |
| 93 | T1.1 missing tuple-in-struct-field rejection rule | Low | Add checks before T1.4 codegen |
| 97 | T1.2 `(a, b) += expr` falls through to generic error | Low | Use separate assignment statements |

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


### 61. Native codegen IR parsing panics on unhandled patterns

**Severity:** Medium — only affects the `--native` code path, which is not yet the default.

**Location:** `src/generation/:1396,1422,1437,1448,1470,1500`

**Symptom:** `panic!("Could not parse {vals:?}")` when the native code generator
encounters an IR pattern it does not recognise.  This is an exhaustiveness gap in the
native emitter, not in the interpreter.

**Root cause:** The IR → Rust source emitter has `panic!` catch-alls for value patterns
that have not been implemented yet.  Adding new IR opcodes or IR value shapes without
updating the emitter leaves silent coverage gaps that manifest as panics at native
codegen time (i.e., compile time for the `--native` path, not interpreter runtime).

**Fix path:** When implementing native codegen for a new opcode or value kind (N9 in the
roadmap), add the corresponding arm to every dispatch site in `generation/`.  An
exhaustive match (replacing `_ => panic!`) would be cleaner but requires all arms first.

**Effort:** Low per opcode; Medium to reach full coverage (tracked as N9).

---


### 68. `first_set_in` does not descend into `Block` nodes *(fixed)*

**Fixed.** The function was renamed to `inline_ref_set_in` and now handles `Block`
and `Loop` nodes (plus all other Value variants exhaustively).

**Severity:** High (was) — causes `add_const` overflow (subtract with overflow panic) or wrong
slot computation for reference variables whose first use is inside a nested block.

**Location:** `src/parser/expressions.rs` — `first_set_in` helper; `parse_code` insertion
loop for `work_references()`.

**Symptom (A12 investigation, 2026-03-20):** When the unified lazy-insertion loop was
applied to non-inline work references (`__ref_N`), references whose first assignment is
inside a `Value::Block` could not be found by `first_set_in` (which does not match the
`Block` variant).  The fallback position placed the null-init *after* the block that
uses the reference.  This produced `first_def > last_use`, giving `assign_slots` a
corrupt live interval that placed the reference's shadow slot above the current stack
top.  At codegen time `add_const` computed `before_stack − stack(ref)` and panicked
with "attempt to subtract with overflow".  Repro: `cargo test --test enums polymorph`.

**Root cause:** `first_set_in` handles `Set`, `Call`, `Insert`, `If`, `Return`, `Drop`,
and `Triple` but has no arm for `Block(Box<Block>)` or `Loop(Box<Block>)`.  A statement
like `result = { __ref_N = null; … }` is `Set(result, Block(…))`; the recursive call on
the `Block` falls through to `_ => false`.

**Workaround (applied):** Non-inline work references are kept at eager position 0 (the
pre-A12 behaviour).  Only work texts and inline-ref variables use lazy insertion.

**Full fix path:** Add `Block` and `Loop` arms to `first_set_in` that iterate the block's
`operators` list and recurse.  Then work references can also be lazily inserted.  Verify
that the `polymorph` test and all vector tests pass after the change.

**Effort:** Small (two `match` arms + tests)

---

### 69. `can_reuse` extension to `Type::Text` in `assign_slots` causes slot conflicts

**Severity:** High — multiple variables assigned to overlapping regions of a dead 24-byte
text slot; debug assertion fires; release builds produce undefined behaviour.

**Location:** `src/variables/` — `assign_slots`, `can_reuse` predicate.

**Symptom (A12 investigation, 2026-03-20):** Extending `can_reuse` from `var_size <= 8`
to also include `Type::Text` allowed two smaller variables (e.g., a 4-byte `total` and
an 8-byte `e#iter_state`) to each reuse the first bytes of the same dead 24-byte text
slot independently.  Both received `stack_pos = 52`; their live intervals overlap;
`find_conflict` fires.  Repro: multiple `vectors` integration tests (`sorted_remove`,
`growing_vector`, etc.).

**Root cause:** `assign_slots` reuses a dead variable's *slot position* (its
`stack_pos`).  When a 24-byte text slot is reused by a 4-byte variable, the remaining
20 bytes look free to another variable.  There is no mechanism to mark the whole dead
text slot as consumed once part of it is reused.

**Fix path:** Text slot reuse requires one of:
1. Reuse only when the reusing variable is also 24 bytes (same-size restriction).
2. Mark the entire dead slot's byte range as consumed after the first reuse.
3. Implement size-aware reuse: track (position, size) pairs for dead slots and only
   reuse when the reusing variable fits exactly or is the same size.

Approach 1 is the simplest (add `&& var_size == dead_size` guard when the dead type is
`Text`).  Approach 3 is the most general but requires restructuring `assign_slots`.

**Workaround (applied):** The `can_reuse` extension has been reverted; text slot reuse
remains disabled.  The unit test `assign_slots_sequential_text_reuse` stays `#[ignore]`.

**Effort:** Small for approach 1; Medium for approach 3.

---

### 70. `Type::Text` in `generate_set` pos-override causes SIGSEGV (`append_fn`)

**Severity:** High — runtime SIGSEGV in tests that use functions returning text.

**Location:** `src/state/codegen.rs` — `generate_set`, `pos < stack.position` branch.

**Symptom (A12 investigation, 2026-03-20):** Adding `Type::Text(_)` to the large-type
override (the `pos < stack.position` bump-to-TOS path in `generate_set`) causes
`tests/expressions.rs::append_fn` to crash with SIGSEGV.  The function under test is
`fn append(ch: character) -> text { "abc_de" + ch }`.

**Root cause (preliminary):** When a text variable's pre-assigned slot is at or above the
current TOS and gets bumped to TOS by the override, `set_stack_allocated` records the new
position.  A later `OpFreeText` reads the variable's original slot (from
`stack.function.stack(v)`) to compute the relative offset, but that slot was reassigned
to TOS.  If TOS has since grown past the original slot, `string_mut` accesses an
incorrect address — likely an uninitialised or already-freed `String`, causing SIGSEGV.
Full root-cause analysis pending.

**Workaround (applied):** `Type::Text` was added to the override to fix an
"uninitialized memory" concern with lazy init, but that concern only arises if Text slots
can be reused (Issue 69), which is currently disabled.  Removing `Type::Text` from the
override restores original behaviour without risk, since there are no reused text slots
to worry about.

**Fix path:** Revert the `Type::Text` arm in `generate_set`.  If text slot reuse (Issue
69) is later enabled, revisit whether a TOS override is still needed and whether
`OpFreeText` correctly uses the updated slot position.

**Effort:** Trivial revert; investigation of the SIGSEGV root cause is Small.

---


## Native Codegen Blockers

### 79. `external` crate reference unresolved

**Symptom:** `error[E0433]: failed to resolve: use of unresolved module external` in
`21-random.loft`.

**Fix path:** The random number extension uses an `external` FFI crate that is not included in
the native codegen output.  Either bundle the implementation in `codegen_runtime` or emit the
necessary `extern` block in the generated file.

---





### 85. Struct-enum local variable leaks stack space (debug assertion)

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

### 93. T1.1 missing tuple-in-struct-field rejection rule

**Symptom:** The TUPLES.md Phase 1 design specifies that `Type::Tuple` should be
rejected in struct field positions and `Type::RefVar` in tuple element positions.
These compile-time rejection rules are not implemented.

**Impact:** Low — T1.2 parser support has landed, so users can now write tuple type
notation.  The rejection rules should be added before T1.4 (codegen) to prevent
struct fields with tuple types from reaching the runtime.

**Fix path:** Add checks in `typedef.rs::fill_all()`: when processing struct fields,
emit an error if `attribute.typedef` is `Type::Tuple`.  Similarly reject `RefVar`
inside tuple elements.

**Discovered:** 2026-03-26, during T1.1 implementation.

---

### 97. T1.2 compound assignment on tuple destructuring not rejected

**Symptom:** `(a, b) += expr` does not trigger the tuple destructuring path (which
only checks for `=`).  It falls through to the regular assignment loop, which fails
with a generic error instead of a clear "compound assignment not supported on tuple
destructuring" diagnostic.

**Impact:** Low — confusing error message; no silent wrong behaviour.

**Fix path:** Before the regular assignment loop, check if the LHS is
`Value::Tuple` and the operator is a compound one (`+=`, `-=`, etc.); emit a
targeted diagnostic.

**Discovered:** 2026-03-26, during T1.2 regression evaluation.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements for new features
