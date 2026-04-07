
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Bug Fix Plan — All Open Issues

Analysis of all open issues in PROBLEMS.md with fix strategies,
ordered by impact and fixability.

---

## Tier 1: Fix now — user-facing bugs with clear solutions

### P115. Text parameter reassignment segfaults → auto-promote — FIXED

**Fixed.** Text arguments are now auto-promoted to local String on
first mutation. The parser creates a shadow local `__tp_<name>`,
copies the argument at function entry, and redirects all references.

**Files changed:** `src/variables/mod.rs`, `src/parser/expressions.rs`,
`src/parser/definitions.rs`, `src/state/codegen.rs`,
`tests/scripts/31-text-param.loft`

---

### P58. Silent Type::Unknown(0) on unresolved names — FIXED

**Fixed.** Added `known_var_or_type` call on assignment RHS in
`expressions.rs`. The existing `known_var_or_type` function in
`objects.rs` already detects Unknown/undefined variables during
expression parsing, but assignment RHS was a gap — simple variable
references like `y = typo_name` went unchecked.

**Files changed:** `src/parser/expressions.rs` (one line),
`tests/scripts/74-unknown-type-detection.loft`

---

### P103. Inline vector concat in compound assignment — FIXED

**Fixed.** Upgraded the existing warning to an error in
`parse_append_vector()`. Inline vector concat `[a] + [b]` now
produces a compile error instead of silently producing wrong results.
Users must assign the concat to a variable first.

**Files changed:** `src/parser/vectors.rs` (Level::Warning → Level::Error),
`tests/scripts/70-ignored-struct-method-bugs.loft`,
`tests/scripts/06-structs.loft`

---

## Tier 2: Fix with moderate effort — correctness/robustness

### P60. No recursion depth limit — FIXED

**Fixed.** Added `call_depth: u32` counter to `State`. Incremented
in `fn_call`, decremented in `fn_return`. Panics with clear message
at depth > 500. Limit set below the store stack limit so the depth
check fires before store out-of-bounds.

**Files changed:** `src/state/mod.rs`

---

### P64 + P66. Integer overflow in store/vector arithmetic — FIXED

**Fixed.** Added `checked_offset()` helper in `store.rs` and
`checked_vec_pos()` / `checked_vec_cap()` helpers in `vector.rs`.
All address calculations now use u64 intermediate arithmetic with
assert on overflow, preventing silent memory corruption.

**Files changed:** `src/store.rs`, `src/vector.rs`

---

### P108. f#next initial seek on fresh file handle — FIXED

**Fixed.** After `File::open()` / `File::create()`, seek to the
stored `next_pos` if non-zero. Both `read_file()` and `write_file()`
now apply the seek position on first open.

**Files changed:** `src/state/io.rs`

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

## New issues discovered during optimisation work

### P116. Struct return aliasing — **HIGH PRIORITY**

`x = func(s)` where `func` returns a Reference parameter aliases
the store instead of deep copying.  Partially fixed: codegen branch
added for `n_*` functions, needs regression testing.

**Files:** `src/state/codegen.rs` (`gen_set_first_at_tos` new branch)

### P117. Struct return store leak — **MEDIUM**

After `OpCopyRecord` in `gen_set_first_ref_copy`, the callee's
source store is never freed.  O-B2 adoption fixes this for functions
without Reference params.  Remaining: functions WITH Reference params.

### P118. Threading regression — **MEDIUM**

`22-threading.loft` panics "Incomplete record" after P64/P66 changes.
Not yet diagnosed.  May be a pre-existing issue exposed by assertion
ordering change, or a subtle interaction with parallel worker stores.

---

## Summary

| Tier | Issues | Total effort |
|------|--------|-------------|
| **1: Fix now** | ~~P115~~, ~~P58~~, ~~P103~~ | All fixed |
| **2: Moderate** | ~~P60~~, ~~P64+P66~~, ~~P108~~ | All fixed |
| **3: Deferred** | P22, P54, P55, P61, P79, P85, P86, P89, P90, P91, P92 | — |
| **4: Update docs** | P114 | XS |
| **5: New** | P116 (aliasing), P117 (leak), P118 (threading) | S + M + M |

**Status:** Tiers 1–2 complete. P116 partially implemented (codegen
branch exists, needs testing). P117 partially fixed by O-B2 adoption.
P118 needs investigation.
