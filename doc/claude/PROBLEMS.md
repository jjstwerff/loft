
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
| 58 | ~~Silent `Type::Unknown(0)` variable creation on unresolved names~~ | ~~High~~ | **Fixed** — `known_var_or_type` now called on assignment RHS |
| 60 | ~~No recursion depth limit~~ | ~~Medium~~ | **Fixed** — runtime call depth limit (500) with clear panic |
| 61 | Native codegen IR parsing panics on unhandled patterns | Medium | N/A — only affects `--native` path (not yet default) |
| 64 | ~~Overflow risk in store offset arithmetic~~ | ~~Medium~~ | **Fixed** — `checked_offset()` uses u64 with assert |
| 66 | ~~Integer cast truncation in vector index/size~~ | ~~Medium~~ | **Fixed** — `checked_vec_pos()`/`checked_vec_cap()` use u64 |
| 79 | Native codegen: `external` crate reference not resolved (random/FFI) | Low | `--native` only; affects `21-random.loft` |
| 85 | Struct-enum local variable leaks stack space (debug assertion) | Low | Pass as parameter instead of local |
| 86 | Lambda capture produced misleading codegen self-reference error | Low | *(mitigated by A5.1)* — clear error now |
| 89 | Hard-coded StackFrame field offsets in `n_stack_trace` | Low | N/A — offsets must match `04_stacktrace.loft` |
| 90 | `fn_call` HashMap lookup for line number on every call | Low | N/A — small overhead relative to dispatch |
| 91 | L7 `init(expr)` parameter form not implemented | Low | Pass default explicitly at call site |
| 92 | `stack_trace()` in parallel workers returns empty | Low | Call from main thread only |
| 103 | ~~Inline vector concat in compound assignment~~ | ~~Medium~~ | **Fixed** — now a compile error instead of warning |
| 107 | ~~`++` return expression + struct parameter bug~~ | ~~Medium~~ | **Fixed** — parser now rejects `++` with a clear error |
| 108 | ~~`f#next` initial seek on fresh read handle~~ | ~~Low~~ | **Fixed** — seek applied on first file open |
| 109 | ~~Struct field reassignment corrupts store when field contains nested vector~~ | ~~High~~ | **Fixed** — `set_skip_free(elm)` in `parse_vector` + `remove_claims` in `copy_record` |
| 110 | ~~Vector push corrupts sibling struct fields~~ | ~~**High**~~ | **Fixed** — `other_indexes` no longer links plain vector fields |
| 111 | ~~`character == text` comparison always returns true~~ | ~~Medium~~ | **Fixed** — now produces compile error; use `"{c}" == t` |
| 112 | ~~Text return accumulation in text-returning functions~~ | ~~Medium~~ | **Fixed** — always clear RefVar(Text) before append |
| 113 | ~~`t = t[N..]` self-slice produces empty string~~ | ~~Medium~~ | **Fixed** — work text used for self-referencing assignments |
| 114 | ~~`h = h + expr` clears h before reading~~ | ~~Medium~~ | **Fixed** — self-append detection + self-reference detection |
| 115 | ~~Text parameter reassignment/append segfaults~~ | ~~Medium~~ | **Fixed** — auto-promotes text argument to local String on mutation |
| 116 | `x = func(s)` where func returns a struct param aliases the store | **High** | Wrap in explicit local: `tmp = func(s); x = tmp` forces copy via O-B1 |
| 117 | Struct-returning functions leak the callee's store after deep copy | Medium | N/A — stores accumulate; no user workaround |
| 118 | ~~`22-threading.loft` regression~~ | ~~Medium~~ | **Fixed** — O-B2 branch now excludes native/stub functions (`code != Null`) |

---

## Text and Vector Bugs (110–115)

All five bugs share a root cause area: the parser's handling of text and vector
operations inside loops and across function returns.  They should be investigated
and fixed together.

### 110. ~~Multiple struct field vector appends produce extra/garbage elements~~

**FIXED** in `src/database/types.rs`.

**Root cause:** When adding a vector field to a struct type definition, the
database linked ALL fields with the same content type via `other_indexes`.
Two `vector<integer>` fields got linked, so `record_finish` for one field
propagated `vector_finish` (length increment) to the other — corrupting
the sibling vector's length.

**Fix:** Only link fields that are indexing types (`sorted`, `hash`, `index`)
where cross-field propagation is needed.  Plain vectors are excluded.

**Test:**  Add to `tests/scripts/` — vector push inside for loop on a struct
field, verify values match.

**Workaround:** Pre-allocate with comprehension, assign by index:
```loft
s.vals = [for _ in 0..3 { 0 }];
for vi in 0..3 { s.vals[vi] = vi * 10; }
```

---

### 111. `character == text` comparison always returns true

**Severity:** Medium — logic bug, silent wrong result.

**Reproducer:**
```loft
fn main() {
  assert('a' == "b", "should be false but is true");
}
```

**Root cause:** The operator resolver in `src/parser/mod.rs` `call_op` tries
each `OpEq*` operator.  `OpEqText` fails (no character→text conversion).
`OpEqInt` fails (text→integer doesn't exist).  `OpEqBool` succeeds because
both character and text can be converted to boolean via `OpConvBoolFromCharacter`
and `OpConvBoolFromText`.  Any non-null character is true, any non-empty text
is true, so `true == true` → `true`.

**Fix strategy:** In `src/parser/mod.rs` `call_op`, when testing operator
candidates, skip `OpEqBool` if neither original operand is boolean.  This
prevents the fallback to truthiness comparison for incompatible types.
Alternatively, add `OpConvTextFromCharacter` to `default/01_code.loft` so
`OpEqText` matches (character converts to single-char text, then text==text).

**Test:** `assert(!('a' == "b"))` and `assert('x' == "x")`.

**Workaround:** `"{c}" == text_value`

---

### 112. ~~Text return accumulation in text-returning functions~~

**FIXED** in `src/state/codegen.rs`.

**Root cause:** In text-returning functions, variables become `RefVar(Text)`
implicit parameters.  `set_var()` always used `OpAppendStackText` but only
cleared (`OpClearStackText`) for loop variables.  Non-loop reassignment
appended instead of replacing.

**Fix:** Always emit `OpClearStackText` before `OpAppendStackText` for
`RefVar(Text)` variables, regardless of loop context.

---

### 113. `t = t[N..]` self-slice produces empty string

**Severity:** Medium — silent data loss.

**Reproducer:**
```loft
fn main() {
  t = "hello world";
  t = t[6..];
  // Expected: "world"
  // Actual:   ""
  assert(t == "world", "got: '{t}'");
}
```

**Root cause:** Same as #114.  The parser converts `t = t[6..]` to: clear `t`,
then read `t[6..]` (which is now empty), assign to `t`.  The clear happens
before the slice reads the original value.

**Fix strategy:** Same fix as #114 — detect self-reference in the RHS and
skip the clear.  The self-append detection added for `h = h + expr` does not
cover slice operations because the RHS is not an `Insert` list but a
`Value::Call(OpSlice, ...)`.

Extend the detection in `assign_text` to recognize `Value::Call` where any
argument is `Value::Var(var_nr)` — if the assignment target appears anywhere
in the RHS expression, use a work text for the intermediate result.

**Test:** `t = t[N..]` and `t = t[..N]` produce correct substrings.

**Workaround:** `s = t[6..]; t = s;`

---

### 114. ~~`h = h + expr` clears h before reading~~

**FIXED** in `src/parser/operators.rs`.

Self-append detection (line 59-68) and self-reference detection
(`code_references_var`, line 89) handle both plain variables and struct
fields.  Verified: `b.buf = b.buf + " world"` produces `"hello world"`.

---

### 115. ~~Text parameter reassignment/append segfaults~~

**FIXED** in `src/parser/expressions.rs`, `src/variables/mod.rs`,
`src/parser/definitions.rs`, `src/state/codegen.rs`.

Text arguments are now auto-promoted to local String on first mutation.
The parser creates a shadow local `__tp_<name>`, copies the argument at
function entry, and redirects all references.  No manual workaround needed.

---

### 116. `x = func(s)` where func returns a struct parameter aliases the store

**Severity:** High — mutation of `x` silently corrupts `s`.

**Reproducer:**
```loft
struct Point { x: float not null, y: float not null }
fn identity(p: Point) -> Point { p }
fn test() {
  orig = Point { x: 1.0, y: 2.0 };
  copy = identity(orig);
  copy.x = 99.0;
  assert(orig.x == 1.0, "FAILS: orig.x is 99.0");
}
```

**Root cause:** `gen_set_first_at_tos` in `codegen.rs` has branches for
`Call(OpCopyRecord, ...)`, `Var(src)`, and `TupleGet(...)` — all emit deep
copies.  But `Call(user_func, ...)` returning a Reference falls to the
catch-all `generate(value)`, which just executes the call and uses the
returned DbRef directly — aliasing the parameter's store.

The parser's `copy_ref` at `collections.rs:287` only wraps NON-variable
targets: `!matches!(to, Value::Var(_))`.  Variable-target assignments skip
the OpCopyRecord wrapper entirely.

**Fix path:** Add a new branch in `gen_set_first_at_tos` for
`Type::Reference + Value::Call(n_*, ...)` where the function name starts
with `n_` (user functions).  If the function has Reference parameters,
emit `OpConvRefFromNull` + `OpDatabase` + `OpCopyRecord` (deep copy).
If no Reference parameters, adopt the returned store (O-B2 optimisation).

Partially implemented in the current code but needs regression testing.

**Files:** `src/state/codegen.rs` (new branch + `gen_set_first_ref_call_copy`)

---

### 117. Struct-returning functions leak the callee's store after deep copy

**Severity:** Medium — stores accumulate linearly with struct-returning calls.

**Symptom:** Each call to a struct-returning function (e.g. `make_point()`)
allocates a store in the callee that is never freed.  The `in_ret` check in
`scopes.rs:662` prevents `OpFreeRef` on the return variable.  After the caller
deep-copies via `OpCopyRecord`, the source store is orphaned.

**Reproducer:** Run any program with many struct-returning calls and observe
store count growing via `LOFT_STORE_LOG=1`.

**Fix path:** O-B2 (return store adoption) fixes this for functions without
Reference parameters by adopting the store instead of copying.  For functions
WITH Reference parameters, the source store after OpCopyRecord must be explicitly
freed — requires preserving the source DbRef across the copy or adding an
`OpMoveRecord` variant.

**Files:** `src/state/codegen.rs` (`gen_set_first_ref_copy`)

---

### 118. ~~`22-threading.loft` regression~~

**FIXED.** The O-B2 codegen branch matched `n_parallel_for` and
`n_parallel_for_light` — native runtime functions with `code == Value::Null`.
The adoption path generated bytecode that skipped the store allocation these
functions depend on. Fix: added `code != Value::Null` guard to exclude
stub/native function definitions from the O-B2 branch.

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

### 93. T1.1 tuple-in-struct-field rejection *(fixed)*

The compiler now emits a clear error: *"struct field cannot have a tuple type"*.

**Test:** `tests/scripts/72-parse-error-caveats.loft` (`@EXPECT_ERROR`).

**Impact:** Low — T1.2 parser support has landed, so users can now write tuple type
notation.  The rejection rules should be added before T1.4 (codegen) to prevent
struct fields with tuple types from reaching the runtime.

**Fix path:** Add checks in `typedef.rs::fill_all()`: when processing struct fields,
emit an error if `attribute.typedef` is `Type::Tuple`.  Similarly reject `RefVar`
inside tuple elements.

**Discovered:** 2026-03-26, during T1.1 implementation.

---

### 97. T1.2 compound assignment on tuple destructuring *(fixed)*

The compiler now emits: *"compound assignment is not supported for tuple
destructuring — use (a, b) = expr instead"*.

**Test:** `tests/scripts/72-parse-error-caveats.loft` (`@EXPECT_ERROR`).

**Impact:** Low — confusing error message; no silent wrong behaviour.

**Fix path:** Before the regular assignment loop, check if the LHS is
`Value::Tuple` and the operator is a compound one (`+=`, `-=`, etc.); emit a
targeted diagnostic.

**Discovered:** 2026-03-26, during T1.2 regression evaluation.

---

### 98. Index range query wrong results with descending key *(fixed)*

**Symptom:** Range iteration on `index<T[-key]>` (descending primary key) yields wrong
elements.  Ascending-key indexes work correctly.

```loft
struct Item { cat: text, score: integer }
struct Db { idx: index<Item[-cat]> }

db = Db { idx: [Item{cat:"a", score:1}, Item{cat:"b", score:2}, Item{cat:"c", score:3}] };
sum = 0;
for e in db.idx["a".."c"] { sum += e.score; }
// Expected: sum == 3 (a + b), Actual: sum == 1 (only "a")
```

Ascending-key indexes (`index<T[key]>`) and sorted collections are not affected.

**Impact:** Medium — descending-key index range queries produce silently wrong results.

**Root cause:** The `iterate()` function in `src/state/io.rs:583` computes `start` and
`finish` tree nodes using `tree::find(before, key)`.  For ascending keys, `find(true, from)`
returns `previous(from)` in tree-order, which is correct — the tree walk via `next()` then
starts at `from`.  For descending keys, the tree in-order is reversed from user-logical
order: "c" > "b" > "a".  `find(true, "a")` returns `previous("a")` = "b" in tree order,
causing the walk to start at "b" and only reach "a" before the tree ends.

**Fix path:** In `fill_iter` (`src/parser/fields.rs:575`), detect when the index's primary
key is descending (`Keys[0].type_nr < 0`) and XOR the reverse bit (64) into the `on` byte.
This makes the `step()` function use `previous()` instead of `next()` for the tree walk,
and makes `iterate()` use the existing reverse-path logic (lines 562–582) which already
swaps from/till correctly.  When the user also applies `rev()`, the XOR cancels out,
restoring the ascending walk direction — which is correct for a reversed descending key.

**Test:** `tests/scripts/71-caveats-problems.loft::test_p98_index_range_descending_key` (passes).

**Discovered:** 2026-04-02, during test coverage gap analysis.

---

### 99–102. Fixed

- **99** Empty struct comprehension + hash types crash — field comprehensions used
  `u16::MAX` as variable reference; now passes field expression.  **Test:** `69-ignored-empty-comprehension-hash.loft`.
- **100** Format `:<`/`:^` ignored for numbers — added `dir` parameter to
  `format_long`/`format_float`/`format_single`.  **Test:** `67-ignored-format-align.loft`.
- **101** Float `:.0` precision ignored — changed sentinel from `0` to `-1` for
  "no precision specified".  **Test:** `68-ignored-float-precision-zero.loft`.
- **102** `rev(vector)` compile error — parser now accepts `Type::Vector` and emits
  decrement-with-clamp loop.  **Test:** `66-ignored-rev-vector.loft`.

---

### 103. ~~Inline vector concat in compound assignment expression~~ *(fixed)*

**Symptom:** `result = f([1,2,3,4,5]) + 100 * f([1,2,3] + [4,5])` returns wrong
value.  Each call works correctly in isolation.

**Root cause:** The vector concat `[a] + [b]` creates a Block with `OpDatabase`
that temporarily grows the stack.  When this Block appears inside an assignment
expression, `gen_set_first_at_tos` / `OpFreeStack` miscomputes the stack offset,
placing the result at the wrong position.

**FIXED** in `src/parser/vectors.rs` — upgraded from warning to compile error.
Inline vector concat `[a] + [b]` now produces a compile error. Users must
assign the concat to a variable first:
```loft
combined = [1,2,3] + [4,5];
result = f([1,2,3,4,5]) + 100 * f(combined);  // correct
```

---

### 104. Test runner executes library functions as tests *(fixed)*

**Symptom:** Library functions with zero parameters (e.g. `mat4_identity()`)
were picked up by the `--tests` runner as test entry points, causing crashes
when `execute_argv` looked them up in the wrong source context.

```loft
pub fn mat4_identity() -> Mat4 {
  Mat4 { m: [1.0, 0.0, ...] }   // FAILS — "Unknown definition"
}
```

**Fix:** Filter test function discovery by source file — only functions
defined in the test file itself are treated as entry points.

**File:** `src/test_runner.rs` — added `def.position.file != abs_file` check.
**Test:** `tests/scripts/76-ignored-struct-vector-return.loft::test_p104_direct_return`.

---

### 105. Nested struct field access on vector elements crashes *(fixed)*

**Symptom:** Accessing a struct field on a vector element that itself contains
a struct caused "Unknown record N" runtime error:

```loft
mesh.vertices[0].pos.x   // "Unknown record 0"  — fixed
```

**Root cause:** `get_val()` in `src/parser/mod.rs` emitted `OpGetRef` for
all `Value::Call` nodes when accessing `Type::Reference` fields.  Inline
struct fields in vectors are at a byte offset, not a record pointer — so
`OpGetRef` read garbage and crashed.

**Fix:** Two-part change in `src/parser/fields.rs` `parse_vector_index()`:
1. For **linked** struct types (`is_linked` — struct used in both a vector
   and a hash/sorted, causing the vector to store 4-byte record pointers):
   emit `OpVectorRef` directly.  `OpVectorRef` hardcodes elm_size=4 and
   dereferences the pointer, giving correct results for all indices.
2. For **inline** structs in plain vectors: keep `OpGetVector(elm_size)`.
   No dereference call after — field access happens at the next `.` level.
In `src/parser/mod.rs` `get_val()`: `Type::Reference` now always emits
`OpGetField` (offset addition).  The linked-type dereference is handled
by `OpVectorRef` at the call site.

**Tests:** `tests/scripts/76-ignored-struct-vector-return.loft` —
`test_p105_inline_struct_in_vector`, `test_p105_nested_struct_in_vector`.
See [P104_P105_P106_C54.md](P104_P105_P106_C54.md).

---

### 106. Store corruption with complex nested struct assignments *(fixed)*

**Symptom:** A nested vector inside a vector element had zero length after
append — e.g., `t.items[0].inner.vals.len()` returned 0 instead of the
expected count.

**Root cause:** Same as P105.  `get_val()` emitted `OpGetRef` instead of
`OpGetField` when reading inline `Type::Reference` fields on vector elements,
causing reads from wrong memory locations and silently returning empty vectors.

**Fix:** Same two-part fix as P105 — `OpVectorRef` for linked types,
`OpGetField` for all `Type::Reference` accesses in `get_val()`.

**Test:** `tests/scripts/76-ignored-struct-vector-return.loft::test_p106_nested_vector_in_vector_element`.
See [P104_P105_P106_C54.md](P104_P105_P106_C54.md).

---

### 107. `++` return expression + struct parameter bug *(fixed)*

**Symptom:** `"str1" ++ "str2"` crashed in codegen with a type mismatch assertion.

**Root cause:** `++` is not a valid operator in loft.  The lexer tokenized it as two
`+` tokens.  The first `+` was consumed as binary addition/concat; the second `+`
could not start an expression (no unary `+` in `parse_single`), producing a
`Value::Null` / `Type::Unknown(0)` that corrupted the function's attribute list via
`text_return`.  The original bug report attributed the crash to struct parameters,
but the real trigger was `++` in any context.

**Fix:** The parser now detects `++` (two consecutive `+` tokens) and emits a clear
error: *"'++' is not a valid operator — use '+' for concatenation or addition"*.
The extra `+` is consumed so parsing recovers cleanly.

**Test:** `tests/scripts/72-parse-error-caveats.loft` (`@EXPECT_ERROR`).

---

### 108. ~~`f#next` initial seek on fresh read-only file handle~~

**FIXED** in `src/state/io.rs`.

After `File::open()` / `File::create()`, the stored `next_pos` is now applied
via `seek(SeekFrom::Start(next_pos))` on first open.  Both `read_file()` and
`write_file()` are fixed.

---

### 109. Struct field reassignment corrupts store when field contains nested vector *(fixed)*

**Symptom:** Reassigning a struct field whose type is a struct-with-vector (e.g.,
`math::Mat4` which contains `m: vector<float>`) causes `fl_validate: node at N has
positive header 0 (should be free)` followed by a crash.

**Root cause (two parts):**
1. `copy_record` did not call `remove_claims` before overwriting, leaking the old vector.
2. When building `Inner { vals: [...] }` into an existing field (`is_field = true`),
   the `elm` (_elm_1) variable had an empty dep list → `get_free_vars` emitted
   `OpFreeRef(elm)` → freed the entire store that the outer struct lived in.

**Fix:** Two commits in sprint 8:
- `src/state/io.rs` `copy_record`: added `remove_claims(&to, tp)` before `copy_block`.
- `src/parser/vectors.rs` `parse_vector`: added `set_skip_free(elm)` when `is_field = true`.

**Discovered:** Sprint 8 GLB transform test.  **Test:** `/tmp/p109_repro.loft`.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements for new features
