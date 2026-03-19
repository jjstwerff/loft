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

### 24. Full compile-time slot assignment not yet implemented

**Current:** Stack slot positions are determined at codegen time by `claim()` in
`state.rs`. The final two steps of the planned P2 pass are missing:
- `assign_slots()` — compute optimal positions using precomputed live intervals
- Remove `claim()` calls and `copy_variable()` once the above is done

**Impact:** Non-optimal stack usage; potential conflicts detected by `validate_slots`
in debug builds.

**Best way forward:** Implement `assign_slots()` in `variables.rs` as a greedy
interval-graph colouring: sort variables by `first_def`, assign each to the lowest
slot position not occupied by a live variable of incompatible type. Wire it into
`scopes::check` after `compute_intervals`. Once all tests pass with `assign_slots`,
remove `claim()` from `state.rs`.

**Details:** [ASSIGNMENT.md](ASSIGNMENT.md) §"P2 — Full slot assignment pass".

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
