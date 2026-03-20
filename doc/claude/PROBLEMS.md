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
| 57 | Database type-dispatch panics on unrecognized types in search/io | Medium | N/A — defensive panics for schema corruption |
| 58 | Silent `Type::Unknown(0)` variable creation on unresolved names | High | N/A — check carefully for typos in Loft code |
| 59 | Unimplemented type combinations in binary file I/O | Medium | Avoid schema types not yet covered by `read_data`/`write_data` |
| 60 | No recursion depth limit in codegen and parser traversals | Medium | N/A — only affects adversarially deep ASTs |
| 61 | Native codegen IR parsing panics on unhandled patterns | Medium | N/A — only affects `--native` path (not yet default) |
| 63 | `todo!()` for sub-record type traversal in `format.rs` | Medium | Avoid sub-record schemas until implemented |
| 64 | Overflow risk in store offset arithmetic (`i32`/`usize` casts) | Medium | N/A — only affects extremely large records |
| 65 | Type index out-of-bounds (`[]` indexing in `data.rs`) | Medium | N/A — only triggered by corrupted/invalid type numbers |
| 66 | Integer cast truncation in vector index/size computations | Medium | N/A — only affects very large vectors |
| 67 | Silent early-return on store resize limit (no diagnostic) | Medium | N/A — large-dataset failures are invisible |

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

### 59. Unimplemented type combinations in binary file I/O

**Severity:** Medium — triggers a panic on schemas using types not yet covered.

**Location:** `src/database/io.rs:101`, `src/database/allocation.rs:399,461`

**Symptom:** `panic!("Not implemented type for file writing ...")` or `todo!()` in
secondary-structure duplication paths when a schema uses a record type not yet handled by
`read_data`/`write_data` or the allocation copy code.

**Fix path:** Implement the missing arms one type at a time, following the pattern of
the existing arms.  Secondary structure copy (`allocation.rs:399,461`) is a TODO for
sub-record types that were deferred; implement analogously to the primary record copy path.

**Effort:** Medium (several arms; requires careful binary layout knowledge)

---

## Parser / Compiler Bugs

### 58. Silent `Type::Unknown(0)` variable creation on unresolved names

**Severity:** High — silently masks typos; produces confusing downstream type errors.

**Location:** `src/parser/expressions.rs:2641`

**Symptom:** When a name lookup fails (not a local variable, not an attribute, not an
enum member), the parser silently creates a new `Type::Unknown(0)` variable with that
name instead of emitting a diagnostic:
```rust
} else {
    *code = Value::Var(self.create_var(name, &Type::Unknown(0)));
    t = Type::Unknown(0);
}
```
A typo in a variable name (e.g. `totla` instead of `total`) creates a fresh variable
with unknown type.  The error surface is the subsequent type-mismatch downstream, not
the unresolved name.

**Root cause:** The two-pass parser uses unknown-type variables as placeholders for
names that will be resolved on the second pass (forward references within a function,
cross-function references resolved in the type-pass).  The silent creation is intentional
for first-pass, but the same code path runs on the second pass where the name really
should be resolved.

**Fix path:** On the second pass (`self.pass == Pass::Second`), emit a "undefined
variable" diagnostic instead of creating a new unknown-type var.  Requires confirming
that all legitimate forward-reference cases are resolved before the second pass begins.

**Effort:** Medium (requires understanding the two-pass resolution protocol)

---

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

### 60. No recursion depth limit in codegen and parser traversals

**Severity:** Medium — adversarially or pathologically deep ASTs cause stack overflow.

**Location:** `generate` (`src/state/codegen.rs`), `inline_ref_set_in`
(`src/scopes.rs`), `compute_intervals` (`src/variables.rs`), `scan` (expressions)

**Symptom:** A deeply nested Loft expression (e.g., `1+(1+(1+(1+...)))` with thousands
of levels) causes a Rust stack overflow (SIGSEGV / `thread stack exhausted`).

**Root cause:** All four traversal functions are directly recursive with no depth counter.
Normal Loft programs stay well within the default stack; only adversarial or machine-
generated code can reach the limit.

**Fix path:** Add a `depth: usize` parameter and return an error (or panic with a helpful
message) beyond a configurable threshold (e.g. 1000).  Alternatively, convert the most
critical path (`generate`) to an iterative design using an explicit stack.

**Effort:** Small per function; changing the `generate` signature is the most invasive part.

---

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

### 63. `todo!()` for sub-record type traversal in `format.rs`

**Severity:** Medium — triggered by schemas that contain sub-records (nested struct types
as fields).

**Location:** `src/database/format.rs:109`

**Symptom:** `todo!()` when `format_record` encounters a field whose type is a nested
record (sub-record).  The primary-record format path handles scalars and known compound
types, but sub-record traversal is deferred.

**Fix path:** Implement the sub-record branch by recursing into `format_record` for the
nested type, following the pattern of the existing scalar arms.

**Effort:** Small–Medium (recursive traversal; requires correct byte-offset computation
for the nested record's fields)

---

### 64. Overflow risk in store offset arithmetic

**Severity:** Medium — only affects records larger than ~2 GB; silent wrap in release.

**Location:** `src/store.rs` — offset arithmetic using `i32`/`usize` casts

**Symptom:** Store byte-offset computations cast between `i32` and `usize` without
overflow checks.  Extremely large records (exceeding `i32::MAX` bytes) would wrap
silently in release builds.

**Fix path:** Replace unchecked casts with checked arithmetic (`checked_add`, `try_into`)
and return an error or panic with a clear message rather than wrapping.  Long-term, the
store's index type should consistently be `u32` or `usize` throughout to eliminate the
cast surface.

**Effort:** Small per site; Medium for a consistent audit of all cast points in `store.rs`.

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

### 66. Integer cast truncation in vector index/size computations

**Severity:** Medium — silent truncation on very large vectors (>2^31 elements for i32,
>2^16 for u16 casts).

**Location:** Vector ops in `src/state/codegen.rs` and `src/database/database.rs` —
`as i32`, `as u16` casts on element counts

**Symptom:** A very large vector (element count exceeding the cast target width) silently
produces a truncated index or size, leading to incorrect iteration or OOB access.

**Fix path:** Add `debug_assert!(count <= MAX)` guards before the narrowing casts, or
use `try_into().expect(...)` to make truncation a panic in both debug and release.

**Effort:** Small per site; Small–Medium for a full audit of all narrowing casts.

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

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements for new features
