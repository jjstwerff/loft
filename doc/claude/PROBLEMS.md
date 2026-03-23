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
| 77 | Native codegen: `CallRef`/function-pointer calls not implemented | Medium | `--native` only; affects `06-function.loft` |
| 79 | Native codegen: `external` crate reference not resolved (random/FFI) | Low | `--native` only; affects `21-random.loft` |
| 80 | Native codegen: 16-parser runtime panic "Allocating a used store" — LIFO store-free order | Medium | `--native` only; loft code frees ref stores in wrong order (allocation order instead of LIFO) |
| 82 | `string` is not a valid type name — use `text` | ~~Fixed~~ | S7: typedef.rs now emits "did you mean 'text'?" |
| 83 | Struct field named `key` in a hash collection causes "Allocating a used store" panic | ~~Fixed~~ | S8: compile-time error in typedef.rs; runtime fix via Issue 85 |
| 84 | Any function with a `for` loop called from a mutually-recursive or recursive chain panics with "Too few parameters on n_xxx" | ~~Fixed~~ | Root cause: `vector_needs_db` created a new local store for argument vectors on second pass; `parse_return` emitted dangling return for locally-backed work-refs. Fixed by checking `is_argument` in `vector_needs_db` and injecting `OpAppendVector` + `Return(Var(__ref_1))` in `parse_return` for explicit returns. |
| 85 | Null-returning hash lookup before insert causes subsequent lookup to return null / "Allocating a used store" panic | ~~Fixed~~ | Root cause: `convert()` used `OpConvRefFromNull` (allocates a store) for `null`→`Reference` in comparisons; `eq_ref`/`ne_ref` did full `DbRef` comparison (not rec-only). Fix: `convert()` uses `OpNullRefSentinel` (no allocation, sentinel `{u16::MAX,0,0}`); `eq_ref`/`ne_ref` treat `rec==0` as null |
| 86 | `f#read(n) as vector<T>` silently returned an empty vector | Medium | **Fixed** — interpreter and native both fixed in 0.8.2 |
| 87 | Native codegen: text method call in format interpolation emits `String` not `&str` | Medium | **Fixed** — native codegen fixed in 0.8.2 (03-text.loft passes) |
| 88 | Native codegen: `directory()` / `user_directory()` / `program_directory()` generate wrong argument | Medium | **Fixed** — native codegen fixed in 0.8.2 (11-files.loft passes) |
| 89 | Optional `& text` parameter causes subtract-with-overflow panic at call site | High | Call without the optional argument |

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

**Location:** `src/generation.rs:1396,1422,1437,1448,1470,1500`

**Symptom:** `panic!("Could not parse {vals:?}")` when the native code generator
encounters an IR pattern it does not recognise.  This is an exhaustiveness gap in the
native emitter, not in the interpreter.

**Root cause:** The IR → Rust source emitter has `panic!` catch-alls for value patterns
that have not been implemented yet.  Adding new IR opcodes or IR value shapes without
updating the emitter leaves silent coverage gaps that manifest as panics at native
codegen time (i.e., compile time for the `--native` path, not interpreter runtime).

**Fix path:** When implementing native codegen for a new opcode or value kind (N9 in the
roadmap), add the corresponding arm to every dispatch site in `generation.rs`.  An
exhaustive match (replacing `_ => panic!`) would be cleaner but requires all arms first.

**Effort:** Low per opcode; Medium to reach full coverage (tracked as N9).

---


### 68. `first_set_in` does not descend into `Block` nodes — work-ref lazy init places null after first use

**Severity:** High — causes `add_const` overflow (subtract with overflow panic) or wrong
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

**Location:** `src/variables.rs` — `assign_slots`, `can_reuse` predicate.

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

### 71. `assign_slots` places variables above TOS causing codegen slot conflicts (FIXED 2026-03-20)

**Tests:** `growing_vector`, `sorted_remove`, `index_iterator`, `index_key_null_removes_all`,
`index_loop_remove_small`, `index_loop_remove_large`, `sorted_filtered_remove_large` — all
produced "Slot conflict" panics in `validate_slots`.

**Root cause:** `assign_slots` skipped dead slots when their size didn't match the candidate
variable (`!can_reuse || var_size != j_size`).  This pushed the chosen slot above every dead
slot from variables that lived inside a previous for-loop body.  Those loop-body slots are
above the physical TOS after the loop exits (OpFreeStack restores TOS to the pre-loop
position).  Codegen's `pos > stack.position` guard then overrode the pre-assigned slot to the
current TOS, placing the variable at the same slot as another variable already pre-assigned
there by `assign_slots`.

**Example (sorted_remove):** `e#index` (int, slot 60) died at seq 129 inside loop 1.  `total`
(int) had first_def=131.  `assign_slots` gave `total` slot 60 (sequential reuse of dead int).
After loop 1 exited, TOS=52.  Codegen saw pos(60) > TOS(52) → overrode to 52.  `e#iter_state`
(var 7) was also pre-assigned slot 52 for loop 2. Conflict.

**Fix (`src/variables.rs` — `assign_slots`):**
1. Before the slot-search loop, compute `tos_estimate` = the maximum slot-end of all
   already-assigned variables that are live at `first_def`.  This is the guaranteed-reachable
   TOS at the variable's first allocation.
2. When skipping a dead slot due to `!can_reuse || var_size != j_size`, clamp the next
   candidate to `tos_estimate` instead of jumping past the dead slot.  This prevents the
   search from ever choosing a slot above TOS.
3. When `candidate == tos_estimate` (fresh allocation at the expected TOS), skip the
   `!can_reuse || var_size != j_size` check — dead slots at TOS are overwritten by direct
   placement, so size compatibility doesn't matter there.

**Also updated:** The `assign_slots_no_narrow_to_wide_reuse` unit test expected `fnref` to
avoid slot 0 (dead 1-byte flag).  With the fix, `fnref` IS placed at slot 0 via direct
placement (tos_estimate=0, no live vars), which is safe.  The test now asserts slot 0.

---

### 72. `assign_slots` places block-return outer variable above TOS causing slot conflict (FIXED 2026-03-21)

**Tests:** `map_integers`, `filter_integers` (issues.rs); `dir`, `loft_suite`, `map_filter_reduce`
(wrap tests) — all produced "Slot conflict" panics in `validate_slots`.

**Root cause:** In the pattern `Set(outer_var, Block([Set(inner_var, ...), Var(inner_var)]))`,
`place_large_and_recurse` placed `outer_var` first (advancing `*tos` by `outer_var.size`), then
recursed into the inner Block at the resulting higher TOS.  At codegen time the block evaluates
first, so `inner_var` is allocated at the lower TOS (= `outer_var.stack_pos`).  `outer_var`'s
pre-assigned slot was above the real stack top; codegen's `pos > stack.position` override fired,
placing `outer_var` at the same slot as `inner_var`.

**Why this happens:** `generate_block` is called with `to = outer_var.stack_pos`, so the block
runs in-place at `outer_var`'s slot.  `outer_var` and `inner_var` share the block's result slot
legally (non-overlapping live intervals in parent/child scopes), but `assign_slots` didn't model
this sharing.

**Fix (`src/variables.rs` — `place_large_and_recurse`):**
In the `Value::Set(v_nr, inner)` arm, when `inner` is a `Value::Block` and `v` is a large
non-Text type, call `process_scope(function, inner, v_slot, depth + 1)` with `frame_base = v_slot`
(the outer var's pre-computed slot), then set `v.stack_pos = v_slot` and return without recursing
further.  Text is excluded: `gen_set_first_text` emits `OpText` *before* the block runs, advancing
`stack.position` by `v_size`, so for Text the frame_base at codegen is already `v_slot + v_size`.

**Also:** `debug_assert!(pos <= stack.position)` added to `generate_set` before the override block
as a regression guard (Step 8 guard).  Unconditional `assert!(pos != u16::MAX)` added after
computing `pos` to catch variables that escape `assign_slots` entirely (Step 9 guard).

---

## Native Codegen Blockers (discovered 2026-03-21 via `make test-native`)

All 24 `tests/docs/*.loft` files fail to compile under `--native`.  The root causes are:

### 77. Function-pointer calls (`CallRef`) not implemented

**Symptom:** `cannot find function CallRef` in `06-function.loft`.  Also `Int`/`Var` emitted
as bare identifiers from lambda/routine call codegen.

**Fix path:** Implement the `Value::CallRef` (or equivalent) case in `output_code_inner` so
that calling a function by stored `u32` def-number generates a Rust indirect call or a match
dispatch table.

### 79. `external` crate reference unresolved

**Symptom:** `error[E0433]: failed to resolve: use of unresolved module external` in
`21-random.loft`.

**Fix path:** The random number extension uses an `external` FFI crate that is not included in
the native codegen output.  Either bundle the implementation in `codegen_runtime` or emit the
necessary `extern` block in the generated file.

### 80. 16-parser native runtime panic: "Allocating a used store" (LIFO free order)

**Symptom:** `thread 'main' panicked at src/database/allocation.rs:24:9: Allocating a used store`
when running `--native tests/docs/16-parser.loft` on the third call to `n_parse`.

**Root cause:** Inside `n_parse`, three stores are allocated (`var_p`, `var___ref_1`,
`var___ref_2`) in LIFO stack order.  They are freed at the end of the function in allocation
order (var_p first), which violates the LIFO contract.  When an intermediate function call
(like `t_4Code_define`) allocates and does not free its own stores, `max` advances beyond 3,
so freeing var_p (store 0) sets `max = max - 1` to an index that points at an in-use store.
On the next call to `n_parse`, `OpDatabase` tries to allocate at that index and panics.

**Fix path:** Change the generated `OpFreeRef` order at the end of `n_parse` to LIFO (free
`var___ref_2` first, then `var___ref_1`, then `var_p`).  This is a codegen issue: the loft
compiler emits frees in declaration order, but the store allocator requires LIFO.  Fix in
`output_block` to sort free calls by store_nr descending, or fix in `allocation.rs` to accept
non-LIFO frees (would require a free-list instead of a stack pointer).

---

## Bugs Found During Benchmark Development (2026-03-22)

### 82. `string` is not a valid type name — use `text`

**Severity:** Medium — silent or misleading error.

**Symptom:** Using `string` as a type name in a struct field produces:
```
Error: Undefined type string
Error: Invalid index key
Error: Cannot write unknown(423) on field Foo.bar:text["..."]
```

**Root cause:** The canonical UTF-8 string type in loft is `text`. The name `string` is not
defined anywhere in the stdlib or interpreter. Code coming from other languages (Python, Rust,
Java) naturally reaches for `string`, which fails at runtime.

**Workaround / Fix:** Replace every occurrence of `string` with `text` in struct field
definitions and function signatures. The type behaves identically to what other languages call
`string`.

**Effort:** Trivial (rename).

---

### 83. Struct field named `key` in a hash collection causes "Allocating a used store" panic (~~Fixed~~)

**Severity:** ~~High~~ — ~~Fixed by Issue 85 (2026-03-22).~~

**Fix (2026-03-22):** `convert()` in `src/fill.rs` now uses `OpNullRefSentinel` instead of
`OpConvRefFromNull` for `null`→`Reference` coercions (no store allocation, uses sentinel
`{u16::MAX,0,0}`). `eq_ref`/`ne_ref` treat `rec==0` as null (reference-only comparison).
This eliminated the spurious store allocation that triggered the panic.

**Original root cause:** `key` is a pseudo-field name reserved for hash iteration (`for kv in h { kv.key }`).
When a real struct field is also named `key`, the name clashed with the hash machinery's internal
field reference. A null-returning hash lookup before the first insert triggered `OpConvRefFromNull`
in a comparison path, allocating an extra store — the allocation order then violated LIFO, causing
"Allocating a used store".

**Remaining work (S8):** Add a compile-time error when a hash-value struct has a field named `key`,
so the root naming conflict is caught at compile time rather than producing confusing runtime errors.

---

### 84. `for` loop in a function called from a recursive function panics: "Too few parameters on n_xxx" (~~Fixed~~)

**Severity:** ~~High~~ — ~~Fixed 2026-03-23.~~

**Fix:** Two complementary fixes in the parser:

1. **`vector_needs_db` argument guard** (`src/parser/expressions.rs`): On the second parse
   pass, `vector_needs_db` was returning `true` for argument vectors (those promoted to
   function attributes by `ref_return` on the first pass), causing a new local `__vdb_N`
   backing store to be allocated for them.  This store was freed at function exit before
   the caller could read the result — use-after-free producing wrong results.  Fixed by
   adding `&& !self.vars.is_argument(vec)` to `vector_needs_db`.

2. **`parse_return` return-ref injection** (`src/parser/control.rs`): An explicit
   `return base` where `base` was backed by a local `__vdb_N` store (allocated inside
   `msort`'s base-case branch) returned a dangling DbRef after `OpFreeRef(__vdb_N)`.
   Fixed by injecting `OpAppendVector(__ref_1, base, rec_tp)` + `Return(Var(__ref_1))`
   when the return variable's dep does not already contain `__ref_1`.

**Test:** `tests/issues.rs::issue_84_merge_sort_too_few_parameters` — merge sort with
eight elements now produces the correct sorted order.

**Original root cause (for reference):** `ref_return` added work-ref attributes to a
function while its body was being parsed.  In a recursive function, call sites parsed
before `ref_return` ran saw the old (smaller) attribute count.  Codegen asserted
`parameters.len() >= expected` and panicked.  The fix above addresses the specific
use-after-free / wrong-results manifestation for the merge-sort pattern; see S6 in
PLANNING.md for the remaining general "recursive call sees stale attribute count" case.

---

### 85. Null-returning hash lookup before insert causes subsequent lookup to return null / "Allocating a used store" panic (~~Fixed~~)

**Severity:** ~~High~~ — ~~Fixed 2026-03-22.~~

**Fix:** `convert()` in `src/fill.rs` now uses `OpNullRefSentinel` (sentinel `{u16::MAX,0,0}`,
no store allocation) for `null`→`Reference` coercions instead of `OpConvRefFromNull` (which
allocated a new store). `eq_ref`/`ne_ref` were updated to treat `rec==0` as null so the
sentinel compares correctly.

**Original root cause:** `convert()` used `OpConvRefFromNull` for `null`→`Reference` comparisons,
which allocated a new store for the sentinel reference. `eq_ref`/`ne_ref` did full `DbRef`
comparison (store+rec+pos), so the sentinel was not equal to null. Together, a null hash lookup
before the first insert triggered an extra store allocation; the LIFO free order was then violated
on the next allocation, causing "Allocating a used store".

---

## Bugs Found During Script Test Development (2026-03-22)

### 86. `f#read(n) as vector<T>` silently produced an empty vector — **FIXED in 0.8.2**

**Severity:** High — now fixed in both interpreter and native paths.

**Location:** `src/state/io.rs` — `read_file` (interpreter); `src/codegen_runtime.rs` — `FileVal for DbRef::file_from_bytes` (native).

**Symptom:** Reading binary data from a file into a vector variable via `rv = f#read(n) as vector<T>`
returned a vector of length 0 regardless of the byte count requested. The write direction
(`f += vector_value`) worked correctly.

**Root cause:** `read_file` called `self.database.write_data(&val, db_tp, little_endian, &data)` where
`val` is the stack DbRef for the destination variable. For vector types, the stack slot does not hold
the vector record pointer directly — it holds an inner DbRef (same two-level indirection as
`write_file`). `write_data` for `Parts::Vector` calls `vector_append(&val, ...)` which reads
`store.get_int(val.rec, val.pos)` expecting a vector record int, but at that location there was the
first 4 bytes of the inner DbRef (store_nr + padding), which resolved to 0. `vector_append` with
`vec_rec == 0` claimed a new record but the variable was never connected to it.

**Interpreter fix:** Before calling `write_data` for a vector type, dereference `val` to
the inner DbRef with `*self.database.store(&val).addr::<DbRef>(val.rec, val.pos)` — matching the
same pattern already used in `write_file`.

**Native fix:** In `file_from_bytes` (`src/codegen_runtime.rs`), when the destination DbRef is the
null sentinel (`rec==0, store_nr==u16::MAX`), allocate a real 12-byte store record and zero-initialize
the vector header slot before calling `vector_append`. The generated code passes the null sentinel
as the initial destination; `file_from_bytes` now initialises it to an empty vector before
appending elements. `12-binary.loft` removed from `SCRIPTS_NATIVE_SKIP`.

---

### 87. Native codegen: text method call inside format interpolation emits `String` instead of `&str` — **FIXED in 0.8.2**

**Severity:** Medium — now fixed.

**Location:** `src/generation.rs` — format-string expression emission.

**Symptom:** A format string containing a text method call such as `"{tag.to_lowercase()}"` caused a
`rustc` type error in the generated `.rs` file:
```
error[E0308]: mismatched types
    ops::format_text(&mut work, (&var_tag).to_lowercase(), ...)
                                ^^^^^^^^^^^^^^^^^^^^^^^^^
    expected `&str`, found `String`
```

**Root cause:** The native emitter generated the method call inline as `(&var_tag).to_lowercase()`,
which returns a `String`. The `ops::format_text` function expects `&str`. For pre-assigned variables,
`&var_x` coerces `&String` → `&str`, but a temporary `String` cannot be implicitly borrowed to `&str`
in the same expression.

**Fix:** In `generation.rs`, text-returning method calls appearing in format interpolation now emit a
`let _tmp_N = ...; ` let-binding before the format expression and use `&_tmp_N` in the
`format_text` call. `03-text.loft` removed from `SCRIPTS_NATIVE_SKIP`.

---

### 88. Native codegen: `directory()` / `user_directory()` / `program_directory()` generate wrong argument — **FIXED in 0.8.2**

**Severity:** Medium — now fixed.

**Location:** `src/generation.rs` — native emission for `Stores::os_directory`, `Stores::os_home`,
`Stores::os_executable`.

**Symptom:** Calling `directory()`, `user_directory()`, or `program_directory()` caused a `rustc`
type error in the generated `.rs` file:
```
error[E0308]: mismatched types
    let mut var_cwd: String = Stores::os_directory((_pre_N)).to_string();
                              ------------------- ^^^^^^^^^ expected `&mut String`, found `()`
```

**Root cause:** These functions use the A8 destination-passing convention (they write into a
`&mut String` provided by the caller and return a `Str` view). The native emitter did not correctly
generate the scratch-buffer setup for them.

**Fix:** `generation.rs` now generates `&mut work_N` (the pre-allocated scratch string) as the first
argument for these destination-passing functions. `11-files.loft` removed from `SCRIPTS_NATIVE_SKIP`.

---

### 89. Optional `& text` parameter causes subtract-with-overflow panic at call site

**Severity:** High — interpreter panics when any function with an optional `& text` parameter is
called with an argument.

**Location:** `src/state/codegen.rs` — `create_stack` arithmetic for optional reference parameters.

**Symptom:** Calling `directory("sub")` (where `directory` has an optional `& text` path parameter)
panics with:
```
thread 'main' panicked ... attempt to subtract with overflow
```
The panic occurs at call-site stack setup, not at function entry.

**Root cause:** The `create_stack` size calculation for optional reference (`& text`) parameters
underflows when the optional argument is supplied. The stack slot for an optional reference is
sized or offset incorrectly compared to what `create_stack` expects.

**Workaround:** Do not pass arguments to functions with optional `& text` parameters; call them
without the optional argument (e.g., `directory()` instead of `directory("path")`).

**Fix path:** Audit `create_stack` in `src/state/codegen.rs` for the optional-reference parameter
size/offset calculation and ensure the slot reserved for an optional `& T` argument matches the
expected stack layout.

**Effort:** Small.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements for new features
