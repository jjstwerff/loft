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
- [Database Engine](#database-engine)
- [Parser / Compiler Bugs](#parser--compiler-bugs)
- [Interpreter Robustness](#interpreter-robustness)
- [Web Services Design Constraints](#web-services-design-constraints)

---

## Open Issues — Quick Reference

| # | Issue | Severity | Workaround? |
|---|-------|----------|-------------|
| 22 | Spatial index (`spacial<T>`) operations not implemented | Low | N/A |
| 44 | Empty vector literal `[]` cannot be passed directly as a mutable vector argument | Medium | Assign to named variable first |
| 54 | `json_items` returns opaque `vector<text>` — no compile-time element type | Low | Accepted limitation; `JsonValue` enum deferred |
| 55 | Thread-local `http_status()` pattern is not parallel-safe | Medium | Use `HttpResponse` struct instead; do not add `http_status()` |
| 56 | `v += extra` via `&vector` ref-param panics in debug / silently fails in release | High | Use a return value instead of a ref-param for vector append |
| 57 | Database type-dispatch panics on unrecognized types in search/io | ~~Fixed~~ | S3: exhaustive match arms in search.rs and io.rs |
| 58 | Silent `Type::Unknown(0)` variable creation on unresolved names | High | N/A — check carefully for typos in Loft code |
| 59 | Unimplemented type combinations in binary file I/O | ~~Fixed~~ | S4: `read_data`/`write_data` now use `r.pos + f.position`; `binary_size()` helper advances slice; collection arms are `unreachable!()` |
| 60 | No recursion depth limit in codegen and parser traversals | Medium | N/A — only affects adversarially deep ASTs |
| 61 | Native codegen IR parsing panics on unhandled patterns | Medium | N/A — only affects `--native` path (not yet default) |
| 63 | `todo!()` for sub-record type traversal in `format.rs` | ~~Fixed~~ | S4: `path()` now handles one-level nested structs, builds `"field.subfield[index]"` |
| 64 | Overflow risk in store offset arithmetic (`i32`/`usize` casts) | Medium | N/A — only affects extremely large records |
| 65 | Type index out-of-bounds (`[]` indexing in `data.rs`) | ~~Fixed~~ | S6-65: get_type() helper in Stores panics with diagnostic |
| 66 | Integer cast truncation in vector index/size computations | Medium | N/A — only affects very large vectors |
| 67 | Silent early-return on store resize limit (no diagnostic) | ~~Fixed~~ | S6-67: saturating_mul + checked_add panic in store.rs |
| 74 | Native codegen: `OpGetFileText`/`OpTruncateFile`/`OpSeekFile`/`OpSizeFile` missing from `codegen_runtime` | High | `--native` only; no workaround |
| 75 | Native codegen: text variable (`String`) passed where `Str` expected in function calls | ~~Fixed~~ | N1: `append_text` uses `&*(val)`, `emit_content` uses `Str::new(&*(expr))` |
| 76 | Native codegen: `OpFormatSingle`, `OpFormatStackText`, `OpRemove`, `OpHashRemove`, `OpAppendCopy` missing from `codegen_runtime` | ~~Fixed~~ | N1: `format_single` uses `ops::format_single`; others implemented in prior sessions |
| 77 | Native codegen: `CallRef`/function-pointer calls not implemented | Medium | `--native` only; affects `06-function.loft` |
| 78 | Native codegen: double-borrow of `stores` in some generated code | ~~Fixed~~ | N1: `output_if_with_subst` applies pre-eval substitution only to conditions |
| 79 | Native codegen: `external` crate reference not resolved (random/FFI) | Low | `--native` only; affects `21-random.loft` |
| 80 | Native codegen: 16-parser runtime panic "Allocating a used store" — LIFO store-free order | Medium | `--native` only; loft code frees ref stores in wrong order (allocation order instead of LIFO) |

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

---

## Database Engine

### 57. Database type-dispatch panics on unrecognized types

**Severity:** Medium — triggered by schema evolution without corresponding code update.

**Location:** `src/database/search.rs:30,93,224,401,455`, `src/database/io.rs:145,243`

**Symptom:** Runtime panic if the database schema contains a type tag not covered by
the match arms in the comparison, iteration, or I/O dispatch functions:
```
panic!("Undefined compare {a:?} vs {b:?}")
panic!("Incorrect search")
panic!("Undefined iterate on '{}'", ...)
panic!("Not implemented type for file writing ...")
```

**Root cause:** Each dispatch function has a `_ => panic!(...)` catch-all arm.  Adding a
new primitive type to the schema without updating all dispatch sites causes a panic at the
first database operation that touches the new type.

**Fix path:** When adding any new schema type, audit all catch-all arms in `search.rs`
and `io.rs` and add the new case.  A compile-time exhaustiveness check (converting the
`_` arms to explicit lists) would make omissions a compile error rather than a runtime panic.

**Effort:** Low per type; Medium to convert all arms to exhaustive lists.

---

### 59. Unimplemented type combinations in binary file I/O — ~~Fixed in S4~~

**Severity:** Medium — triggers a panic on schemas using types not yet covered.

**Location:** `src/database/io.rs` (fixed in commit S4)

**Fix applied:** `read_data` and `write_data` now use `r.pos + u32::from(f.position)` for
each struct field instead of the same `r.pos` for all fields.  A `binary_size()` helper
advances the data slice between fields in `write_data`.  Collection-type arms
(`Sorted`, `Hash`, `Index`, `Spacial`, `Array`, `Ordered`) are now `unreachable!()` with
a message referencing the F57 compile-time guard; `Parts::Base` is likewise `unreachable!()`.
Unit tests `read_data_struct_field_positions` and `write_data_struct_field_positions` in
`database::io::tests` verify the fix.

---

## Parser / Compiler Bugs


### 44. Empty vector literal `[]` cannot be passed directly as a mutable vector argument

**Symptom:** Passing `[]` directly as a function argument where the parameter is a mutable
vector (`vector<T>`) fails at compile time or produces incorrect codegen:
```loft
assert(join([], "-") == "", ...);  // ← fails
```
In a debug build, the `generate_call` debug assertion fires:
```
mutable arg 0 (parts: Vector(...)) expected 12B on stack but generate(Insert([Null])) pushed 0B
```

**Root cause:** `parse_vector` in `expressions.rs` returns early with `Value::Insert([Null])`
for an empty `[]` literal when the target is not an existing variable (`is_var = false`).  This
path is reached when `[]` appears as a function call argument, because expression parsing uses
`Type::Unknown(0)` as the initial type context.  With an unknown element type, no temporary
variable is created and no `vector_db` initialisation ops are emitted.  The `Value::Insert([Null])`
carries no stack presence, so the mutable-arg handler finds 0 bytes instead of the 12-byte
`DbRef` it expects.

By contrast, `my_vec = []` in an assignment statement works: on the second pass `my_vec` already
has an inferred type, and `create_vector` (called from `parse_assign_op`) inserts the correct
`vector_db` initialisation ops into the Insert.

**Workaround:** Assign the empty vector to a named variable first; the type is then inferred
from the subsequent call, and the second pass initialises the variable correctly:
```loft
join_empty = [];
assert(join(join_empty, "-") == "", ...);
```

**Fix path:** In `parse_vector`, when the early-return `else` branch is taken (empty `[]`,
not a Var, not a field), create a unique temporary variable, call `vector_db` to initialise it,
push `Value::Var(vec)` as the block result, and wrap in `v_block` — matching what the non-empty
path does when `block = true`.  The difficulty is that `assign_tp` (the element type) is
`Type::Unknown(0)` at this point; `vector_db` must either tolerate Unknown gracefully on the
second pass or this path must be deferred until the call-site type is known.

**Effort:** Medium (parser change; requires careful handling of the Unknown element type)

---

### 56. `v += extra` via `&vector` ref-param panics (debug) / silently fails (release)

**Symptom:** A function that appends to a `&vector<T>` ref-param using `v += extra`
panics in debug builds:
```
thread panicked at src/store.rs:785: Unknown record 5
```
In release builds the operation silently does nothing — the caller's vector is unchanged.
Test: `ref_param_append_bug` in `tests/issues.rs`.

**Root cause:** When `v += extra` is compiled for a `v: &vector<T>` ref-param, codegen
emits `OpAppendVector` with the raw ref-param DbRef (a stack pointer into the caller's
frame via `OpCreateStack`) rather than the actual vector DbRef.  `vector_append` calls
`store.get_int(v.rec, v.pos)` to read the vector header; `v.rec` is the caller's stack
frame record, which is not present in the current function's store `claims` — hence the
`Unknown record` panic.  In release builds the `debug_assert` is elided, producing
corrupt or no-op behaviour.

**Fix path:** In codegen for `v += extra` where `v: RefVar(Vector)`:

1. Emit `OpGetStackRef` to dereference the ref-param and load the actual vector DbRef.
2. Emit `OpAppendVector` with the loaded DbRef.
3. Emit `OpSetStackRef` to write back the (possibly reallocated) DbRef through the ref.

The write-back is required because `vector_append` may resize the backing record and
update the DbRef in-place; without writing back, the caller's variable would reference a
stale record after the append.

The same pattern is needed for any mutable collection operation on a ref-param
(e.g. `v += [item]` for a single element, hash insert, etc.).

**Workaround:** Return the modified vector and assign at the call site, or pass the
vector by-value and reassign:
```loft
fn fill_ret(v: vector<Item>, extra: vector<Item>) -> vector<Item> { v += extra; v }
buf = fill_ret(buf, extra);
```

**Effort:** Medium (codegen change in `src/state/codegen.rs`; requires identifying all
collection-mutating operations on ref-params)
**Target:** 0.8.2

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

### 63. `todo!()` for sub-record type traversal in `format.rs` — ~~Fixed in S4~~

**Severity:** Medium — triggered by schemas that contain sub-records (nested struct types
as fields).

**Location:** `src/database/format.rs` (fixed in commit S4)

**Fix applied:** The `// TODO if f_tp is sub_record/vector/sorted/enum` at line 109 is
replaced with a one-level nested struct traversal.  When a parent struct's field has
`Parts::Struct(_)` or `Parts::EnumValue(_, _)` and the sub-struct contains a collection
of the child type, `path()` now builds `"field.subfield[index]"` by iterating the
sub-fields, computing the nested `DbRef` at
`8 + u32::from(f.position) + u32::from(sf.position)`, and walking the collection to find
the index.

---


### 65. Type index out-of-bounds access in `data.rs`

**Severity:** Medium — triggered by an invalid type number; would panic with an
unhelpful index-out-of-bounds message.

**Location:** `src/data.rs` — `self.types[tp as usize]` and similar unchecked indexing

**Symptom:** If a type number coming from a deserialized schema or codegen path is
out of range, Rust's bounds check panics with a generic index-OOB message rather than a
diagnostic naming the invalid type.

**Fix path:** Introduce a `get_type(nr)` helper that panics with a message including
the invalid index and the function name.  Replace all bare `self.types[tp as usize]`
calls with `self.get_type(tp)`.

**Effort:** Small (helper + search-replace; no logic change)

---


### 67. Silent early-return on store resize limit

**Severity:** Medium — large-dataset failures are invisible; the program continues with
a truncated store rather than reporting an error.

**Location:** `src/store.rs` — resize path

**Symptom:** When the store reaches its size limit it returns early without allocating
more space and without emitting a diagnostic.  Callers that check the return value may
handle this, but callers that assume allocation always succeeds proceed silently with
corrupt state.

**Fix path:** Either return a `Result` from the resize path (propagating to all callers)
or panic with a clear "store limit exceeded" message.  The latter is simpler and
appropriate as long as the limit is large enough to never be reached in normal use.

**Effort:** Small (change early-return to panic or Result propagation)

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

## Native Codegen Blockers (discovered 2026-03-21 via `make test-native`)

All 24 `tests/docs/*.loft` files fail to compile under `--native`.  The root causes are:

### 74. `OpGetFileText` / `OpTruncateFile` / `OpSeekFile` / `OpSizeFile` missing from `codegen_runtime`

**Symptom:** `error[E0425]: cannot find function OpGetFileText` in every generated file that
touches the `File` type (all 24 tests use the stdlib which transitively includes file ops).

**Root cause:** These four Op functions are defined in `default/02_images.loft` with no `#rust`
template.  The bytecode interpreter uses them via `fill.rs` stack-based wrappers.  Native
codegen emits them as direct function calls but no implementation is provided.

**Fix path:** Add `pub fn OpGetFileText`, `pub fn OpTruncateFile`, `pub fn OpSeekFile`, and
`pub fn OpSizeFile` to `src/codegen_runtime.rs` with direct-call signatures matching how the
generator emits them (e.g. `fn OpGetFileText(stores: &mut Stores, file: DbRef, content: &mut String)`).
The implementations can delegate to the same logic already in `src/state/io.rs`.

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

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements for new features
