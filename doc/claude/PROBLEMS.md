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

### 24. Optimised slot assignment not yet stable

**Current:** The safe pre-pass (`assign_slots_safe`) is now the default — variables
receive sequential slots in `first_def` order with no reuse.  `claim()` in codegen
is retained but skipped for already-allocated variables.  The legacy `claim()`-only
path is still reachable via `LOFT_LEGACY_SLOTS=1`; the greedy-coloring path via
`LOFT_ASSIGN_SLOTS=1`.

**Two known bugs block the optimised mode** (documented in `doc/claude/SLOT_FAILURES.md`):

- **Bug A — Vector comprehension aliasing** (`compute_intervals`): **Fixed by A6.3a.**
  The `needs_early_first_def` predicate now excludes `Type::Vector`; filter/map tests
  pass in all modes.

- **Bug B — Narrow→wide slot reuse** (`assign_slots`, optimised only):
  A 4-byte fn-ref is allowed to reuse a dead 1-byte slot; the `OpPutX` displacement
  is off by the size difference, corrupting data at runtime.
  Fix: add `|| var_size != j_size` to the dead-slot-overlap guard (inner loop).

- **Bug C — `Value::Iter` not traversed** (`compute_intervals`, optimised only):
  Variables read only inside an iterator's `create`/`next`/`extra_init` keep
  `last_use = 0` and appear dead at birth, allowing slot reuse that corrupts the
  loop counter at runtime.
  Fix: add a `Value::Iter` arm that recurses into all three sub-expressions.

**Impact:** With the safe default, all tests pass (except the pre-existing
`ref_param_append_bug`).  The optimised mode is gated behind `LOFT_ASSIGN_SLOTS=1`
and should not be used until Bugs B and C are fixed.

**Next steps:** Fix Bugs B and C (each a one- or two-line change); enable the ignored
unit tests; make greedy the default; remove `LOFT_LEGACY_SLOTS` and `LOFT_ASSIGN_SLOTS`
env-var gates.  Then A6.4: remove `claim()` and `assign_slots_safe`.

**Details:** [ASSIGNMENT.md](ASSIGNMENT.md), [SLOT_FAILURES.md](SLOT_FAILURES.md).

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
