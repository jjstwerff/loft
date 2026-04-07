
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Bug Fix Plan — All Open Issues

Analysis of all open issues in PROBLEMS.md with fix strategies,
ordered by impact and fixability.

---

## Tier 1: Fix now — user-facing bugs with clear solutions

### P115. Text parameter reassignment segfaults → auto-promote

**Current state:** Compile error. User must manually copy to local.

**Fix:** Auto-promote text argument to local String on first mutation.
The parser already detects the mutation point. Needs:
1. Create a shadow local variable `__targ_N` with String type
2. Emit `OpText + OpAppendText` to copy argument content into it
3. Redirect all subsequent references of the original parameter to
   the shadow variable

**Complexity:** M — needs variable redirection in the parser's
variable table during second pass. The first pass establishes the
variable; the second pass can substitute reads.

**Files:** `src/parser/expressions.rs`, `src/variables/mod.rs`

---

### P58. Silent Type::Unknown(0) on unresolved names

**Current state:** Typos in variable names silently create new variables
with unknown type, leading to confusing errors later.

**Fix:** After parsing each function, check for variables that remain
`Type::Unknown(0)` and were never assigned. Emit a warning:
```
Warning: variable 'nme' is used but never defined — possible typo?
```

**Complexity:** S — the variable table already tracks `uses` and
`type_def`. A post-parse scan for `Unknown(0)` with `uses > 0` is
straightforward.

**Files:** `src/parser/mod.rs` (end of `parse_file`),
`src/variables/mod.rs`

---

### P103. Inline vector concat in compound assignment

**Current state:** Warning emitted. `v += [a] + [b]` produces wrong
result.

**Fix:** In the parser, when `+=` RHS is a vector concat expression,
materialize the concat into a temp before appending. Or: detect
and emit a clear error instead of warning.

**Complexity:** S — the warning infrastructure is already in place.
Converting to an error or fixing the concat materialization.

**Files:** `src/parser/expressions.rs`

---

## Tier 2: Fix with moderate effort — correctness/robustness

### P60. No recursion depth limit

**Current state:** Deeply recursive loft code can stack overflow the
Rust process.

**Fix:** Add a `depth: u32` counter to `State`. Increment on
`fn_call`, decrement on `fn_return`. Panic with a clear message
at depth > 1000 (configurable).

**Complexity:** S — two lines in `fn_call` and `fn_return`.

**Files:** `src/state/mod.rs`

---

### P64 + P66. Integer overflow in store/vector arithmetic

**Current state:** `i32` offsets can overflow for very large records
or vectors. Theoretical risk, no known reproducer.

**Fix:** Use `u32::checked_mul` / `checked_add` in critical paths:
- `store.rs:addr()` — `rec * 8 + fld`
- `vector.rs:get_vector` — `8 + size * index`
- `vector.rs:vector_append` — `(length + 1) * size`

On overflow, return null / panic with a clear "record too large" error.

**Complexity:** S — mechanical, many callsites but each is a one-line
change.

**Files:** `src/store.rs`, `src/vector.rs`

---

### P108. f#next initial seek on fresh file handle

**Current state:** Seeking before reading fails.

**Fix:** In the File iterator implementation, track whether the first
read has occurred. If `f#next` is called before any read, perform
an implicit first read.

**Complexity:** S — add a `first_read: bool` flag to the file handle
state.

**Files:** `src/state/io.rs` (file iteration)

---

## Tier 3: Low priority — design limitations or native-only

### P22. Spatial index not implemented

**Status:** Compile error emitted. No user demand yet. Deferred.

### P54. json_items returns opaque vector<text>

**Status:** Accepted design limitation. JsonValue enum deferred to
a future language version.

### P55. http_status() thread-local not parallel-safe

**Status:** Already documented to not use. HttpResponse struct works.

### P61. Native codegen panics on unhandled IR patterns

**Status:** Native path (`--native`) is not the default. Fix as
patterns are encountered during native codegen development.

### P79. Native codegen external crate reference

**Status:** Native-only. Fix when native FFI matures.

### P85. Struct-enum local variable leaks stack space

**Status:** Low impact. Debug assertion only. Workaround available.

### P86. Lambda capture self-reference error message

**Status:** Already mitigated — error is now clear. No fix needed.

### P89. Hard-coded StackFrame field offsets

**Status:** By design — must match `04_stacktrace.loft`. Document
and add a compile-time assert.

### P90. fn_call HashMap lookup per call

**Status:** Performance overhead. Consider caching or using a Vec
lookup instead of HashMap in a performance pass.

### P91. init(expr) parameter form

**Status:** Feature request, not a bug. Low demand.

### P92. stack_trace() empty in parallel workers

**Status:** By design — workers don't have the full call stack.
Document limitation.

---

## Tier 4: Already fixed but detail section needs update

### P114. `h = h + expr` — fully fixed

The struct field case now works. Update detail section from
"partially fixed" to "fixed".

---

## Summary

| Tier | Issues | Total effort |
|------|--------|-------------|
| **1: Fix now** | P115, P58, P103 | S + S + S = Small |
| **2: Moderate** | P60, P64+P66, P108 | S + S + S = Small |
| **3: Deferred** | P22, P54, P55, P61, P79, P85, P86, P89, P90, P91, P92 | — |
| **4: Update docs** | P114 | XS |

**Recommended sprint:** Fix Tier 1 + Tier 2 (6 issues, all S effort)
in one branch. Update P114 docs. Leave Tier 3 as documented limitations.
