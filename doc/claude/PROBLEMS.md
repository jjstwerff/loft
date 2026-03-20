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
- [Stack Slot Assignment (In Progress)](#stack-slot-assignment-in-progress)
- [Parser / Compiler Bugs](#parser--compiler-bugs)
- [Web Services Design Constraints](#web-services-design-constraints)

---

## Open Issues — Quick Reference

| # | Issue | Severity | Workaround? |
|---|-------|----------|-------------|
| 22 | Spatial index (`spacial<T>`) operations not implemented | Low | N/A |
| 24 | Compile-time slot assignment incomplete | Low | No user impact yet |
| 44 | Empty vector literal `[]` cannot be passed directly as a mutable vector argument | Medium | Assign to named variable first |
| 54 | `json_items` returns opaque `vector<text>` — no compile-time element type | Low | Accepted limitation; `JsonValue` enum deferred |
| 55 | Thread-local `http_status()` pattern is not parallel-safe | Medium | Use `HttpResponse` struct instead; do not add `http_status()` |
| 56 | `v += extra` via `&vector` ref-param panics in debug / silently fails in release | High | Use a return value instead of a ref-param for vector append |

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

## Stack Slot Assignment (In Progress)

### 24. Stack slot `assign_slots` pre-pass (FIXED — A6.4)

**Done (A6.4):** `claim()` and `assign_slots_safe` removed; `LOFT_DEBUG_SLOTS` debug
blocks deleted from both `variables.rs` and `codegen.rs`.  `claim()` replaced by
`set_stack_pos()`.  The TOS-drop fallback in `generate_set` calls
`set_stack_pos(v, stack.position)` to override the pre-assigned slot to TOS.
All tests pass except the pre-existing `ref_param_append_bug` (Issue 56).

**History:** [ASSIGNMENT.md](ASSIGNMENT.md), [SLOT_FAILURES.md](SLOT_FAILURES.md).

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

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements for new features
