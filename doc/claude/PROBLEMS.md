
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
| 61 | ~~Native codegen IR parsing panics on unhandled patterns~~ | ~~Medium~~ | **Fixed** — `codegen_runtime.rs` implements all critical opcodes |
| 64 | ~~Overflow risk in store offset arithmetic~~ | ~~Medium~~ | **Fixed** — `checked_offset()` uses u64 with assert |
| 66 | ~~Integer cast truncation in vector index/size~~ | ~~Medium~~ | **Fixed** — `checked_vec_pos()`/`checked_vec_cap()` use u64 |
| 79 | ~~Native codegen: `external` crate reference not resolved (random/FFI)~~ | ~~Low~~ | **Fixed** — `external` module emitted in generated code via `codegen_runtime` wrappers |
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
| 116 | ~~`x = func(s)` where func returns a struct param aliases the store~~ | ~~**High**~~ | **Fixed** — codegen deep copies when func has Reference params; adopts when safe |
| 117 | Struct-returning functions leak the callee's store after deep copy | Medium | ⚠️ Appears fixed by P116/P118/P122 wave; regression guard `tests/issues.rs::p117_text_param_struct_return_loop_no_leak` passes. Re-verify with the original `file()`-style pattern before closing |
| 120 | Use-after-free: struct return inside `if` block in loop frees borrowed store | **High** | ⚠️ Appears fixed; `lib/graphics/examples/test_mat4_crash.loft` runs cleanly and `tests/issues.rs::p120_vector_field_in_returned_struct_round_trip` passes. Re-verify with the full GL example suite before closing |
| 121 | Tuple literals crash interpreter with heap corruption | **High** | ⚠️ Appears fixed; reproducer from PROBLEMS.md runs cleanly and `tests/issues.rs::p121_float_tuple_*` regression guards pass. Re-verify in a debug build with valgrind before closing |
| 122 | Store leak: struct/vector allocation inside game loop exhausts store pool | **High** | Use raw-float functions instead of struct-based APIs in loops |
| 123 | Per-frame vector literal allocation leaks stores | Medium | Use scalar variables or bitmasks instead of `[for _ in 0..N { 0 }]` in render loop |
| 124 | Native codegen: inline array indexing `[a,b,c][i]` generates invalid Rust cast | Low | ⚠️ Appears fixed; `tests/issues.rs::p124_function_returning_inline_array_index` passes under interpret. Re-verify under `--native` with `--native-emit` before closing |
| 125 | `use` import can't find sibling packages when script is inside a package | Medium | ~~**Fixed**~~ — `lib_path` now walks up to `loft.toml` and searches siblings |
| 126 | Negative integer literal as final expression parsed as void negation | Low | Use `return -1;` instead of bare `-1` as function tail expression |
| 127 | File-scope `vector<single>` constant passed to `gl_upload_vertices` causes codegen 8B vs 12B stack mismatch | Medium | Move the literal inline into the calling function |
| 128 | File-scope constants reject type annotations with misleading "Expect token =" error | Low | Drop the annotation; let the literal's element type be inferred |
| 129 | Native codegen emits duplicate `extern crate loft_graphics_native` when a script outside the package imports a package that uses graphics | Medium | Run the script in `--interpret` mode, or place it inside the loft repo |
| 130 | Headless GL: panic via `fatal runtime error: Rust cannot catch foreign exceptions` after `gl_create_window` returns false | Medium | Don't run GL examples without a `DISPLAY`; check `gl_create_window` return and `return` immediately — but the panic happens regardless on some paths |
| 131 | Loft CLI consumes script arguments instead of forwarding them (e.g. `loft script.loft --mode glb` → `unknown option: --mode`) | Low | Use a flag the loft CLI doesn't recognise as its own; or hard-code the mode for now |
| 118 | ~~`22-threading.loft` regression~~ | ~~Medium~~ | **Fixed** — O-B2 branch now excludes native/stub functions (`code != Null`) |
| 119 | ~~Native OpenGL programs segfault (heap corruption)~~ | ~~**High**~~ | **Fixed** — `n_` functions registered under `loft_` names so auto-marshaller resolves them |

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

### 116. ~~`x = func(s)` where func returns a struct parameter aliases the store~~

**FIXED** in `src/state/codegen.rs`.

Added new branch in `gen_set_first_at_tos` for `Type::Reference +
Value::Call(n_*, ...)` where the function has a code body (not native).
Functions with Reference parameters emit deep copy via
`gen_set_first_ref_call_copy`. Functions without Reference params adopt
the returned store directly (O-B2 optimisation).

Guard `code != Value::Null` excludes native/stub functions (P118 fix).

---

### 117. Struct-returning functions with text params leak stores

**Severity:** Medium — stores accumulate for functions like `file()`.

**Status (2026-04-09):** ⚠️ **Appears fixed but unverified.** The
regression guard `tests/issues.rs::p117_text_param_struct_return_loop_no_leak`
runs 1000 iterations of a `Wrap { name: text, count: integer }`
construction and assertion loop without any leak warnings or pool
exhaustion. The `file()` API path that originally exhibited the bug has
also changed (`file().exists()` no longer exists in the current API), so
the original repro can't be re-run as-is. Before closing this entry,
re-verify with: (1) a fresh `file()`-style API call in a tight loop with
`LOFT_STORES=warn`, (2) the test scripts listed under "Affected tests"
below.

**Symptom:** `f = file("path")` leaks store because `f`'s type has
`dep=[__ref_1]` (text-return work variable). Scopes.rs sees non-empty
deps and skips OpFreeRef, treating `f` as a borrowed reference.

**Root cause:** `call_dependencies` / `resolve_deps` propagates deps
from text-return work variables (`__ref_N`) into the struct return type.
The File struct COPIES the text into its store (OpSetText deep copy),
so the dep is spurious — but the dep system doesn't distinguish copies
from shared references.

**Affected tests:** `file_write_error`, `file_exists_true/false`,
`file_debug` — all fail with "Database N not correctly freed".

**Attempted fix:** Filtering `__ref_N` deps in `get_free_vars` fixed
the file tests but caused "Double free" in `issue_84_merge_sort` —
the filter was too broad, removing genuine deps for recursive structs.

**Attempted fixes and why they fail:**

1. **Filtering __ref_N deps in get_free_vars (scopes.rs):** Fixed file
   tests but caused double-free in merge_sort (recursive vector returns)
   and double-free in native codegen (which reads the same IR).

2. **Empty deps in add_defaults line 1797:** Fixed file tests but caused
   use-after-free in null-coalescing tests. The `vec![vr]` dep keeps the
   work ref alive while the returned struct is constructed. Removing it
   breaks patterns where the work ref IS the returned store.

3. **Filtering text deps in ref_return:** Doesn't help because the
   spurious dep comes from `add_defaults` (caller side), not `ref_return`
   (callee side). The variables in `ref_return`'s `ls` are struct-typed.

**Root cause:** The dep at `add_defaults:1797` (`vec![vr]`) is
load-bearing — removing it causes use-after-free (stack store freed
during execution) and breaks null-coalescing. The dep keeps the
return-store work ref alive, which is correct when the function
returns THROUGH the work ref. But for O-B2 adoption (no-ref-param
functions), the work ref is unused — the callee's store is adopted
directly. The dep is only spurious in the O-B2 case.

**Correct fix:** In the O-B2 codegen path (`gen_set_first_at_tos`),
after adopting the callee's store, emit `OpFreeRef` for the unused
`__ref_N` work variable. This frees the work ref that was allocated
by `add_defaults` but never used (O-B2 bypasses it). The dep stays
in the type system (keeping the broader lifetime model intact), but
the unused work ref store is explicitly cleaned up.

**Detection:** Runtime warning at program exit (`execute_argv`) and
compile-time `check_ref_leaks` P117 warning are in place.

**Files:** `src/state/codegen.rs` (O-B2 adoption path)

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


### 61. Native codegen IR parsing panics on unhandled patterns *(fixed)*

**Fixed.** All critical opcodes now have implementations in `src/codegen_runtime.rs`:
`OpDatabase`, `OpNewRecord`, `OpFinishRecord`, `OpFreeRef`, `OpCopyRecord`,
`OpGetTextSub`, `OpLengthCharacter`, `OpGetFileText`, `OpTruncateFile`,
`OpInsertVector`, `OpSortVector`, `OpIterate`, `OpStep`, `OpGetRecord`,
`OpSizeofRef`, `OpFormatDatabase`, and `cr_call_push`/`CallGuard` for stack traces.

The generated native code uses `use loft::codegen_runtime::*;` to import all
implementations.  Remaining unimplemented opcodes (parallel blocks, some hash/index
ops) are low-priority and tracked under N9.

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

### 79. `external` crate reference unresolved *(fixed)*

**Fixed.** The native codegen now emits a local `mod external` block
(`src/generation/mod.rs:416-419`) that wraps `codegen_runtime::cr_rand_seed` and
`cr_rand_int`.  The `#rust` templates in `default/01_code.loft` reference
`external::rand_seed()` and `external::rand_int()` which resolve to these wrappers.

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
Fixed in Sprint 8.

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
Fixed in Sprint 8.

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

### 119. Native OpenGL programs segfault (heap corruption) *(fixed)*

**Symptom:** Running `02-hello-triangle.loft` or other OpenGL examples in `--interpret`
mode segfaulted. The crash occurred when calling native functions like
`loft_gl_upload_vertices` and `loft_gl_set_uniform_mat4`.

**Root cause:** Two interpreter-aware native functions (`n_gl_upload_vertices` and
`n_gl_set_uniform_mat4`) were registered in `wire_native_fns` under their `n_` prefix
names, but the `#native` annotations in `.loft` files reference them with the `loft_`
prefix. When the interpreter looked up `loft_gl_upload_vertices` in the registry, it
found no match and fell back to `try_dlsym`, which resolved the raw C-ABI version
(`loft_gl_upload_vertices` — the non-store-aware function taking raw pointers). Calling
a raw-pointer function with store-based interpreter arguments caused heap corruption and
segfault.

The `n_`-prefixed functions exist specifically for the interpreter path: they accept
`LoftStore` + `LoftRef` parameters and use the auto-marshaller to safely access store
data. The `loft_`-prefixed versions are for the compiled/native path and take raw
pointers directly.

**Fix:** In `lib/graphics/native/src/lib.rs`, changed two `reg!()` calls to register
the `n_` implementations under their `loft_` names:
- `reg!(b"loft_gl_upload_vertices", n_gl_upload_vertices);`
- `reg!(b"loft_gl_set_uniform_mat4", n_gl_set_uniform_mat4);`

This ensures the auto-marshaller finds the correct store-aware function before the
`dlsym` fallback is tried.

**Remaining risk:** Verify that the `#native` annotation string for `set_uniform_mat4`
in `graphics.loft` matches the registered name exactly (`loft_gl_set_mat4` vs
`loft_gl_set_uniform_mat4` — a potential secondary mismatch). Any future `n_`-prefixed
functions must also be registered under `loft_` names manually; there is no generic
prefix-fallback in the symbol lookup.

**Test:** `02-hello-triangle.loft` with `--interpret` — renders 300 frames without
crash.

### 120. Struct constructor doesn't deep-copy vector fields into struct store

**Status (2026-04-09):** ⚠️ **Appears fixed but unverified.** The
documented reproducer `lib/graphics/examples/test_mat4_crash.loft` now
runs cleanly:
```
inside make_big: data len=16
after return: data len=16
data[0]=0 data[15]=15
```
The unit-test version `tests/issues.rs::p120_vector_field_in_returned_struct_round_trip`
also passes, asserting the full round-trip (16 elements survive return).
Before closing this entry, re-run the full GL example suite — especially
`19-complete-scene` and `25-breakout`, which the docs cite as blocked by
this bug. If GL textures still come out black or matrices come out
zeroed, the fix is incomplete.

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

**Status (2026-04-09):** ⚠️ **Appears fixed but unverified.** The exact
documented reproducer (`a = (3.0, 2.0); assert(a.0 > 1.0, ...)`) runs
cleanly under `--interpret`. The regression-guard tests
`p121_float_tuple_literal_no_heap_corruption` and
`p121_float_tuple_function_return` in `tests/issues.rs` pass.

Heap corruption is non-deterministic — passing tests don't *prove* the
bug is gone; the corruption might require specific allocator state or
allocation history. Before closing, re-verify with: (1) a debug build
under valgrind, (2) the `tests/scripts/50-tuples.loft` end-to-end script
(currently in `SCRIPTS_NATIVE_SKIP`).

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

### 124. Native codegen: inline array indexing generates invalid Rust

**Severity:** Low (native mode only)

**Status (2026-04-10):** ⚠️ **Appears fixed but unverified.** The
function-tail form `[0.9, 0.2, 0.3][idx]` now compiles cleanly under
`--native`, and the regression guard
`tests/issues.rs::p124_function_returning_inline_array_index` passes
under interpret mode. The local-variable workaround
(`tests/issues.rs::p124_local_array_index_workaround_works`) also keeps
passing.

Note: in interpret mode, the same expression as a *statement-level*
assignment (`v = [10, 20, 30][i];`) now produces a parser-level type
error ("Variable v cannot change type from vector<integer> to integer")
— a different and stricter behaviour than the original codegen panic.
Before closing this entry, re-verify with `--native-emit` to confirm no
`as DbRef` cast appears in the generated Rust source.

**Symptom (historical):** `[0.9, 0.2, 0.3][idx]` in loft generated an
`as DbRef` cast in the Rust output, which failed to compile.

**Workaround:** Assign the array to a variable first:
```loft
// BAD — native codegen error
color = [0.9, 0.2, 0.3][row];

// GOOD
colors = [0.9, 0.2, 0.3];
color = colors[row];
```

---

### 125. ~~`use` import can't find sibling packages from inside a package~~

**Severity:** Medium — **Fixed**

**Symptom:** Running `./25-breakout.loft` from `lib/graphics/examples/`
failed with `"Included file shapes not found"` because `use shapes;`
couldn't locate the sibling `lib/shapes/` package.

**Fix:** `lib_path` in `parser/mod.rs` now walks up from the script's
directory looking for a `loft.toml`. When found, the package's parent
directory is searched for sibling packages in `<name>/src/<name>.loft`
layout.

---

### 126. Negative integer literal as final expression

**Severity:** Low

**Symptom:** A function whose body contains earlier `if X { return Y; }`
statements followed by a tail expression `-1` produces a misleading
parse error:

```
Error: No matching operator '-' on 'void' and 'integer' at .../file.loft:5:1
  |
   5 | }
     | ^
```

A function with bare `-1` as its *only* statement parses fine. The bug
only fires when an earlier statement (typically `if { return; }`) leaves
the parser in a state where the next `-` is treated as a binary operator
on the previous statement's `void` result instead of as a unary prefix
on a new expression.

**Reproducer:**
```loft
fn lookup(idx: integer) -> integer {
  if idx == 0 { return 100; }
  if idx == 1 { return 200; }
  -1                          // ← parsed as `void - 1`
}
```

**Tests:** `tests/issues.rs::p126_negative_tail_expression` (workaround
guard, passes) and `p126_negative_tail_expression_after_returns`
(`#[ignore]`d real bug reproducer).

**Root cause hypothesis:** the statement-vs-expression boundary in
`parse_block`/`parse_expression` uses pratt parsing and tries to extend
the previous statement's value with an infix `-` operator before checking
whether the previous statement actually produced a value.

**Fix path:** in the block parser, force `-` after a void-returning
statement to start a new unary prefix expression. Equivalent to inserting
an implicit `;` boundary when the previous statement's result type is
`Void`. Touch points: `src/parser/expressions.rs` (statement loop) and
`src/parser/operators.rs` (prefix vs infix `-` resolution).

**Workaround:** Use `return -1;` with explicit return, or assign to a
variable first: `result = -1; result`.

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
