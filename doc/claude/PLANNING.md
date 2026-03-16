// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Enhancement Planning

## Goals

Loft aims to be:

1. **Correct** — programs produce the right answer or a clear error, never silent wrong results.
2. **Prototype-friendly** — a new developer should be able to express an idea in loft with minimal
   ceremony: imports that don't require prefixing every name, functions that can be passed and
   called like values, concise pattern matching, and a runtime that reports errors clearly and
   exits with a meaningful code.
3. **Performant at scale** — allocation, collection lookups, and parallel execution should stay
   efficient as data grows.
4. **Architecturally clean** — the compiler and interpreter internals should be free of technical
   debt that makes future features hard to add.

The items below are ordered by tier: things that break programs come first, then language-quality
and prototype-friction items, then architectural work.  See [RELEASE.md](RELEASE.md) for the full
1.0 gate criteria, project structure changes, and release artifact checklist.

Sources: [PROBLEMS.md](PROBLEMS.md) · [INCONSISTENCIES.md](INCONSISTENCIES.md) · [ASSIGNMENT.md](ASSIGNMENT.md) · [THREADING.md](THREADING.md) · [LOGGER.md](LOGGER.md) · [WEB_IDE.md](WEB_IDE.md) · [RELEASE.md](RELEASE.md) · [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) · [BYTECODE_CACHE.md](BYTECODE_CACHE.md)

---

## Contents
- [Version Milestones](#version-milestones)
- [Tier 0 — Crashes / Silent Wrong Results](#tier-0--crashes--silent-wrong-results)
- [Tier 1 — Language Quality & Consistency](#tier-1--language-quality--consistency)
- [Tier 2 — Prototype-Friendly Features](#tier-2--prototype-friendly-features)
- [Tier 3 — Architectural / Future Work](#tier-3--architectural--future-work)
- [Tier R — Repository Extraction](#tier-r--repository-extraction)
- [Tier W — Web IDE](#tier-w--web-ide)
- [Quick Reference](#quick-reference)

---

## Version Milestones

### Version 1.0 — Language Stability

1.0 is a **stability contract**: any program valid on 1.0 compiles and runs identically on any 1.x
release.  Full criteria and release checklist in [RELEASE.md](RELEASE.md).

**Hard gate items** (must be resolved before tagging 1.0):
R1 — see Quick Reference for full details

**1.0 target items** (include if time allows; 1.1 if not):
T2-0 — see Quick Reference for full details

**Explicitly 1.1+**:
T2-1 (lambdas), T2-2 (REPL), T2-4, T2-5, T2-7, T2-8, T2-12, T3-1..T3-5, T3-7, T3-8, W1..W6 (Web IDE; starts after R6)

### Version 1.x — Minor releases (additive)

New language features that are strictly backward-compatible: T2-0, T2-1, T2-2.
Roughly monthly cadence.  Web IDE (Tier W) is a parallel track independent of interpreter versions.

### Version 2.0 — Breaking changes only

Reserved for language-level breaking changes (syntax removal, sentinel redesign).
Not expected in the near term.

---

## Tier 0 — Crashes / Silent Wrong Results


## Tier 1 — Language Quality & Consistency

---

### T1-14  Scalar patterns in `match` expressions
**Sources:** [MATCH.md](MATCH.md) — T1-14
**Severity:** Medium — `match` currently only handles enum subjects; scalar dispatch requires if/else chains
**Description:** Allow `match` on `integer`, `long`, `float`, `single`, `text`, `boolean`, and `character` values.  Arm patterns are literals; boolean is exhaustive (two values); float arms warn about NaN equality.
**Fix path:** See [MATCH.md#t1-14](MATCH.md#t1-14--scalar-patterns) for full design.
Extend `parse_match` subject-type dispatch; add scalar literal parsing in the arm loop; reuse `OpEqInt` / `OpEqText` / `OpEqBool` etc.
**Effort:** Medium (parser/control.rs — subject dispatch + literal pattern parsing)
**Target:** 1.1

---

### T1-9  Dead assignment — variable overwritten before first read
**Sources:** Compiler warnings audit 2026-03-15
**Severity:** Medium — a value assigned but never read before being overwritten is silently
discarded; the most common form is a copy-paste bug (wrong variable on the left-hand side)
**Description:** Extend the existing "Variable is never read" infrastructure to detect when
a variable is assigned, then assigned again without any intervening read:
```loft
fn compute(a: integer, b: integer) -> integer {
    result = a + b    // Warning: dead assignment — 'result' overwritten before first read
    result = a * b
    result
}
```
**Fix path:**
1. Add a `last_write: Option<Source>` field to `Variable` alongside the existing `uses` counter.
2. On each assignment, if `uses` has not grown since the previous write, emit the warning at `last_write`.
3. Update `last_write` to the current assignment source position.
4. `_`-prefixed variables are exempt (consistent with "Variable is never read").
**Effort:** Small (variables.rs — extends existing write-tracking)
**Target:** 1.1

---

### T1-10  Unused loop variable
**Sources:** Compiler warnings audit 2026-03-15
**Severity:** Low–Medium — a loop variable that is never read in the body usually indicates
iterating only to count, but accidentally using the wrong name inside the loop body
**Description:** The existing "Variable is never read" check skips names containing `#`
(synthetic loop metadata) but not the primary loop iteration variable.  Extend the check:
```loft
for item in items {
    total += 1   // Warning: loop variable 'item' is never read
}
```
`_`-prefixed loop variables (e.g. `for _ in items`) suppress the warning, consistent with
the existing convention for unused variables.
**Fix path:** In `variables.rs:test_used`, narrow the `contains('#')` exemption so it only
skips synthetic internal names; apply the same `uses == 0` check to loop iteration variables.
**Effort:** Trivial (variables.rs — one-line scope change of the existing exemption)
**Target:** 1.1

---

### T1-13  Unreachable code after unconditional terminator
**Sources:** Compiler warnings audit 2026-03-15
**Severity:** Medium — any statement after an unconditional `return`, `break`, or `continue`
will never execute; this usually indicates dead leftover code or a missing conditional
**Description:** Track a "flow terminated" flag through the statement list in each block.
Set it on `return`, `break`, or `continue` at the top level of a block (not inside a nested
`if`); emit a warning for every subsequent statement in the same block:
```loft
fn f() -> integer {
    return 1
    x = compute()    // Warning: unreachable code
}
```
**Fix path:**
1. Add a `terminated: bool` flag to the parser's statement-loop state.
2. Set `terminated = true` after parsing `return` / `break` / `continue` at block scope.
3. At the start of each statement iteration, if `terminated`, emit the warning, continue
   parsing (to avoid cascading errors) but discard the generated IR.
4. Clear `terminated` at if/else merge points.
**Effort:** Medium (parser/control.rs — new flag threaded through the statement loop)
**Target:** 1.1

---

### T1-16  Guard clauses (`if`) in `match` arms
**Sources:** [MATCH.md](MATCH.md) — T1-16
**Severity:** Medium — without guards, per-arm conditions require a nested `if` inside the arm body and cannot affect exhaustiveness
**Description:** `Circle { r } if r > 0.0 => ...` — optional boolean guard after a pattern.  Guard failure falls through to the next arm.  Guarded arms do not contribute to exhaustiveness coverage.
**Fix path:** See [MATCH.md#t1-16](MATCH.md#t1-16--guard-clauses-if) for full design.
Parse optional `if expr` after pattern; emit `If(pattern_cmp, If(guard, body, chain_rest), chain_rest)` with chain_rest cloned.
**Effort:** Small–Medium (parser/control.rs — guard parsing + chain-building change)
**Depends on:** T1-14
**Target:** 1.1

---

### T1-15  Or-patterns (`|`) in `match` arms
**Sources:** [MATCH.md](MATCH.md) — T1-15
**Severity:** Low–Medium — disjunction over patterns requires duplicating arm bodies today
**Description:** `North | South => "vertical"` — multiple patterns per arm, combined with `||`.  Works for enum variants, scalars, and ranges.
**Fix path:** See [MATCH.md#t1-15](MATCH.md#t1-15--or-patterns-) for full design.
Refactor `arms` storage from `(Option<i32>, ...)` to `(Option<Value>, ...)` (pre-built condition); add `|`-loop in pattern parser.
**Effort:** Medium (parser/control.rs — structural refactor of arms vec + pattern loop)
**Depends on:** T1-14
**Target:** 1.1

---

### T1-17  Range patterns in `match` arms
**Sources:** [MATCH.md](MATCH.md) — T1-17
**Severity:** Low–Medium — range dispatch currently requires chained `if`/`else if` comparisons
**Description:** `1..=10 =>` (inclusive) and `1..100 =>` (exclusive) patterns for integer, long, float, single, text, and character subjects.  Open-start `..=hi` supported; open-end `lo..` is an error in pattern position.
**Fix path:** See [MATCH.md#t1-17](MATCH.md#t1-17--range-patterns) for full design.
After parsing scalar literal, check for `..` + optional `=`; build `OpLeXxx(lo, subj) && OpLeXxx/OpLtXxx(subj, hi)`.
**Effort:** Small (parser/control.rs — extends scalar pattern parser)
**Depends on:** T1-14
**Target:** 1.1

---

### T1-18  Plain struct destructuring in `match`
**Sources:** [MATCH.md](MATCH.md) — T1-18
**Severity:** Low–Medium — struct field extraction currently requires separate field-access statements
**Description:** `match p { Point { x, y } => x + y }` — bind struct fields directly in a match arm.  No discriminant comparison (one shape); exhaustive once any unconditional arm appears.
**Fix path:** See [MATCH.md#t1-18](MATCH.md#t1-18--plain-struct-destructuring) for full design.
Extend subject-type dispatch to `Type::Reference(d_nr)` with `DefType::Struct`; reuse field-binding mechanism from T1-4 struct-enum.
**Effort:** Small (parser/control.rs — subject dispatch + reuse existing field-bind code)
**Target:** 1.1

---

### T1-12  Redundant null check on `not null` type
**Sources:** Compiler warnings audit 2026-03-15
**Severity:** Low–Medium — comparing a `not null` value to `null` is always false or always
true; using `//` (null-coalescing) on a `not null` value makes the default branch unreachable;
both indicate a misunderstood type annotation
**Description:** When the type of an expression is statically known to be non-nullable, flag
null-check patterns whose result is constant:
```loft
fn f(x: integer not null) {
    if x == null { ... }     // Warning: 'x' is 'not null' — comparison is always false
    y = x // default_value   // Warning: 'x' is 'not null' — null-coalescing is redundant
}
```
**Fix path:**
1. In the equality expression parser: when one operand is the `null` literal and the other
   has a non-nullable type, emit the warning.
2. In the `//` operator handler: when the left-hand operand has a non-nullable type, emit
   the warning and still emit the code (preserve semantics; let optimiser remove the branch).
**Effort:** Small (parser/expressions.rs — type-driven checks, no flow analysis required)
**Target:** 1.1

---

### T1-22  Missing return path for functions with a non-null return type
(Moved from Tier 2 — this is a language correctness item.)
**Sources:** Compiler warnings audit 2026-03-15
**Severity:** Medium — a function declared to return `integer not null` that falls off the
end without a `return` silently returns null, violating the declared contract
**Description:** After parsing a function body, check whether every exit path has an explicit
`return`.  Warn only when the declared return type is non-nullable:
```loft
fn classify(n: integer) -> text not null {
    if n > 0 { return "pos" }
    // Warning: not all code paths return a value; function may return null
}
```
A nullable return type (`-> text`, without `not null`) is exempt — falling off the end is
then intentional.
**Fix path:**
1. Define a `definitely_returns(block) -> bool` predicate: a block definitely-returns if
   its last statement is a `return`, or it is an `if` with an `else` where both branches
   definitely-return (recursive).
2. After parsing each function body, if the return type is `not null` and
   `!definitely_returns(body)`, emit the warning at the closing `}`.
**Effort:** Medium (parser/control.rs — return-path analysis after function body)
**Target:** 1.1

---

### T1-23  Variable shadowing
(Moved from Tier 2 — this is a language quality item.)
**Sources:** Compiler warnings audit 2026-03-15
**Severity:** Low — a variable in an inner scope silently shadows an outer-scope variable
of the same name; the outer variable is unchanged, which is often surprising in loops
**Description:** Before adding a new variable to a scope, check whether the same name exists
in any enclosing scope.  If it does, emit a warning:
```loft
x = 10
for x in items {    // Warning: loop variable 'x' shadows outer variable 'x'
    ...
}
// outer x is still 10 — the loop variable was distinct
```
**Fix path:**
1. In the variable creation path (`variables.rs:add_variable`), walk the enclosing scope
   chain checking for a name collision before registering the new variable.
2. Emit the warning at the inner declaration site, referencing both positions.
3. `_`-prefixed names are exempt.
**Effort:** Small (variables.rs + scopes.rs — scope-chain walk at variable creation)
**Target:** 1.1+

---

### T1-19  Nested patterns in field positions
**Sources:** [MATCH.md](MATCH.md) — T1-19
**Severity:** Low — field-level sub-patterns currently require nested `match` or `if` inside the arm body
**Description:** `Order { status: Paid, amount } => charge(amount)` — a field may carry a sub-pattern (`:` separator) instead of (or in addition to) a binding variable.  Sub-patterns generate additional `&&` conditions on the arm.
**Fix path:** See [MATCH.md#t1-19](MATCH.md#t1-19--nested-patterns-in-field-positions) for full design.
Extend field-binding parser to detect `:`; call recursive `parse_sub_pattern(field_val, field_type)` → returns boolean `Value` added to arm conditions with `&&`.
**Effort:** Medium (parser/control.rs — recursive sub-pattern entry point)
**Depends on:** T1-14, T1-18
**Target:** 1.1+

---

### T1-20  Remaining patterns (null, binding `@`)
**Sources:** [MATCH.md](MATCH.md) — T1-20
**Severity:** Low
**Description:** `null` pattern; wildcard-binding (`x => body`); explicit `name @ pattern` binding; character literal patterns.
**Fix path:** See [MATCH.md#t1-20](MATCH.md#t1-20--remaining-patterns-null-binding) for full design.
`null`: detect `has_token("null")`; emit null-equality condition.  Wildcard binding: unrecognised identifier in scalar arm creates a variable.  `@`: add `"@"` to TOKENS; parse `name @ pattern`.
**Effort:** Small (parser/control.rs — a few new checks in arm parsing; one TOKENS addition)
**Depends on:** T1-14
**Target:** 1.1+

---

### T1-21  Slice and vector patterns
**Sources:** [MATCH.md](MATCH.md) — T1-21
**Severity:** Low — vector/text structural dispatch requires manual length checks and element access today
**Description:** `[first, ..] =>`, `[.., last] =>`, `[a, b] =>` and similar patterns for `vector<T>` and `text` subjects.  Binds elements by position; `..` skips the rest.  Rest binding (`rest..`) deferred to a follow-up.
**Fix path:** See [MATCH.md#t1-21](MATCH.md#t1-21--slice-and-vector-patterns) for full design.
Detect `has_token("[")` in arm; parse slice elements; emit `OpLengthVector` length test + `OpGetVector` element bindings.
**Effort:** Medium (parser/control.rs — new `parse_slice_pattern` helper)
**Depends on:** T1-14, T1-15
**Target:** 1.1+

---

## Tier 2 — Prototype-Friendly Features

### T2-0  Code formatter (`loft --format`)
**Sources:** [FORMATTER.md](FORMATTER.md)
**Severity:** Low — no correctness impact; quality-of-life
**Description:** Token-stream formatter imposing one canonical loft style (no configuration).
Key rules: 2-space indent, opening brace on same line, every block body multi-line, spaces
around operators, fields on separate lines in struct/enum definitions, param/call/array lists
wrapped at 80 cols, consecutive `use` lines sorted alphabetically, trailing commas stripped.
Invoked as `loft --format file.loft` (in-place) or `--format-check` (CI exit 1 if differs).
Works via a new `Mode::Raw` lexer pass that preserves `LineComment` tokens; ~400 lines in
`src/formatter.rs`.
**Effort:** Small–Medium (new `src/formatter.rs`; minor additions to `src/lexer.rs`, `src/main.rs`)

---

### T2-13  Empty `[]` literal unusable as a direct mutable vector argument
**Sources:** PROBLEMS #44
**Severity:** Low — passing `[]` directly to a function that takes `&vector<T>` fails with
a codegen assertion; the workaround is trivial but surprising
**Description:** Writing `join([], "-")` when `join` expects a mutable vector triggers a
debug-build assertion in `generate_call` ("expected 12B on stack but generate(Insert([Null])) pushed 0B") because `parse_vector` returns `Value::Insert([Null])` — zero stack bytes — when `[]` appears in call context with an unknown element type.  In assignment context (`v = []`) the second pass knows the type and works correctly.
**Fix path:** In the `else` branch of `parse_vector` (the early-return path for empty `[]`
when `is_var = false`), synthesise an anonymous temporary variable, call `vector_db` to emit
the initialisation ops, and return `Value::Var(tmp)` wrapped in a `v_block` — exactly as the
non-empty path does when `block = true`.  The catch: `assign_tp` is `Type::Unknown(0)` at
this point, so `vector_db` must tolerate `Unknown` on the first pass and be called again on
the second pass once the callee's parameter type is known.
**Effort:** Medium (parser/expressions.rs — deferred type resolution for empty vector in call context)
**Target:** 1.1

---

### T2-1  Lambda / anonymous function expressions
**Sources:** Prototype-friendly goal; T1-1 (callable fn refs) already complete
**Severity:** Medium — without lambdas, `map` / `filter` require a named top-level function
for every single-use transform, which is verbose for prototyping
**Description:** Allow inline function literals at the expression level:
```loft
doubled = map(items, fn(x: integer) -> integer { x * 2 });
evens   = filter(items, fn(x: integer) -> boolean { x % 2 == 0 });
```
An anonymous function expression produces a `Type::Function` value, exactly like `fn <name>`,
but the body is compiled inline.  No closure capture is required initially (captured variables
can be added in a follow-up, see T3-2).
**Fix path:**
1. Parser: recognise `fn '(' params ')' '->' type block` as an expression.
2. Compilation: synthesise a unique def-nr, compile the body as a top-level function.
3. Runtime: the resulting value is the def-nr — identical to a named `fn <name>` ref.
**Effort:** Medium–High (parser.rs, state.rs)

---

### T2-2  REPL / interactive mode
**Sources:** Prototype-friendly goal
**Severity:** Low–Medium — a REPL dramatically reduces iteration time when exploring data
or testing small snippets
**Description:** Running `loft` with no arguments (or `loft --repl`) enters an
interactive session where each line or block is parsed, compiled, and executed immediately.
State accumulates across lines (variables and type definitions persist).
```
$ loft
> x = 42
> "{x * 2}"
84
> struct Point { x: float, y: float }
> p = Point { x: 1.0, y: 2.0 }
> p.x + p.y
3.0
```
A basic REPL does not require T1-1 or T1-2; those features simply make the REPL more
ergonomic once available.
**Fix path:**
1. Implement an incremental `Parser` mode that accepts a single statement and returns when
   complete (tracking open braces to handle multi-line blocks).
2. Maintain a persistent `State` and `Stores` across iterations.
3. Print expression results automatically (non-void expressions print their value).
4. On parse error, discard the failed line and continue the session.
**Effort:** High (main.rs, parser.rs, new repl.rs)

---

### T2-5  In-place sort for primitive vectors
**Sources:** Standard library audit 2026-03-15
**Severity:** Medium — sorting an existing `vector<integer>` or `vector<text>` in-place is a
fundamental operation with no current solution; `sorted<T>` is insertion-ordered, not a sort
**Description:**
```loft
pub fn sort(v: &vector<integer>);           // ascending
pub fn sort(v: &vector<text>);
pub fn sort_desc(v: &vector<integer>);      // descending
// + long, float, single, character variants
```
The `&` modifier makes the sort visible to the caller (modifies in-place).
A custom comparator variant (`sort(v, fn cmp)`) can follow in a later release.
**Fix path:** Native Rust implementation in `src/native.rs` per element type; declaration in
`default/01_code.loft`.  Uses `Store::get_int` / `set_int` to swap elements directly in
the vector storage, or copies to a `Vec<T>`, sorts, writes back.
**Effort:** Medium (native Rust per type; ~50 lines per overload)
**Target:** 1.1 — important but not blocking; implement after 1.0 is tagged

---


### T2-7  File system — `mkdir` and `mkdir_all`
**Sources:** Standard library audit 2026-03-15
**Severity:** Low — files can be read, written, deleted, and listed, but directories cannot
be created; output pipelines that write to a new subdirectory require a shell workaround
**Description:**
```loft
// Create one directory level (fails if parent does not exist).
pub fn mkdir(path: text) -> boolean;

// Create directory and all missing parents (like Unix mkdir -p).
pub fn mkdir_all(path: text) -> boolean;
```
Returns `true` on success, `false` (not null) on failure so callers can check without
null-testing.
**Fix path:** Native Rust using `std::fs::create_dir` / `create_dir_all`; declaration
alongside `delete` and `move` in `default/02_images.loft`.
**Effort:** Small (native Rust ~15 lines)
**Target:** 1.1 — useful but not blocking

---

### T2-8  Expose hidden vector operations — `reverse`, `clear`, `insert`
**Sources:** Standard library audit 2026-03-15
**Severity:** Low — `OpClearVector` and `OpInsertVector` exist in the bytecode but have no
public loft wrappers; `reverse` has no operator at all
**Description:**
```loft
pub fn clear(v: &vector);                           // set length to 0; O(1)
pub fn insert(v: &vector<integer>, idx: integer, elem: integer);  // insert at position
pub fn reverse(v: &vector<integer>);                // reverse in-place; O(n)
// + typed overloads per element type for insert/reverse
```
`clear` wraps `OpClearVector` (trivial).  `insert` wraps `OpInsertVector`.
`reverse` has no existing operator; needs a native implementation or an O(n) loft loop.
**Fix path:**
- `clear`: pure loft using `OpClearVector` (or in-place loop if that's cleaner).
- `insert`: expose existing `OpInsertVector` via a public loft declaration.
- `reverse`: native Rust for efficiency, or O(n) swap loop in loft for each type.
**Effort:** Low–Medium (clear and insert low; reverse medium per type)
**Target:** 1.1 — nice to have, no urgency

---

### T2-4  Vector aggregates — `sum`, `min_of`, `max_of`, `any`, `all`, `count_if`
**Sources:** Standard library audit 2026-03-15
**Severity:** Low–Medium — common operations currently require manual `reduce`/loop boilerplate;
the building blocks (`map`, `filter`, `reduce`) are already present
**Description:** Typed overloads for each primitive element type:
```loft
// Sum (integer overload shown; long/float/single analogous)
pub fn sum(v: vector<integer>) -> integer { reduce(v, 0, fn __add_int) }

// Range min/max (avoids shadowing scalar min/max by using longer names)
pub fn min_of(v: vector<integer>) -> integer { ... }
pub fn max_of(v: vector<integer>) -> integer { ... }

// Predicates — require compiler special-casing (like map/filter) because fn-ref
// types are not generic; each overload hardcodes the element type
pub fn any(v: vector<integer>, pred: fn(integer)->boolean) -> boolean { ... }
pub fn all(v: vector<integer>, pred: fn(integer)->boolean) -> boolean { ... }
pub fn count_if(v: vector<integer>, pred: fn(integer)->boolean) -> integer { ... }
```
`sum`/`min_of`/`max_of` are straightforward reduce wrappers; `any`/`all`/`count_if`
are short-circuit loops that need a named helper or compiler special-casing.
Note: naming these `min_of`/`max_of` (not `min`/`max`) avoids collision with T1-7.
**Fix path:** Typed loft overloads using `reduce` for sum/min_of/max_of; compiler
special-case in `parse_call` for `any`/`all`/`count_if` (same tier of effort as T1-3).
**Effort:** Low for aggregates (pure loft); Medium for any/all/count_if (compiler)
**Target:** 1.1 — batch all variants; defer until after T2-1 (lambdas) makes them ergonomic

---

### T2-12  Bytecode cache (`.loftc`)
**Sources:** [BYTECODE_CACHE.md](BYTECODE_CACHE.md)
**Severity:** Medium — repeated runs of an unchanged script re-parse and re-compile every
time; for scripts with many `use`-imported libraries this is measurably slow
**Description:** On first run, write a `.loftc` cache file next to the script containing
the compiled bytecode, type schema, function-position table, and source mtimes.  On
subsequent runs, if all mtimes and the binary hash match, skip the entire parse/compile
pipeline and execute directly from cache.
```
script.loft   →   script.loftc    (next to source; --cache-dir for override)
```
Phases:
- **C1** — single-file cache (4 files changed, no new dependencies)
- **C2** — library file invalidation (`Parser.imported_sources`)
- **C3** — debug info preserved (error messages still show file:line after cache hit)
- **C4** — `--cache-dir xdg` and `--no-cache` / `--invalidate-cache` flags
**Fix path:** See [BYTECODE_CACHE.md](BYTECODE_CACHE.md) for full detail.
**Effort:** Medium (C1 is Small; full C1–C4 is Medium)
**Target:** 1.1

---

## Tier 3 — Architectural / Future Work

### T3-1  Parallel workers: extra arguments and text/reference return types
**Sources:** [THREADING.md](THREADING.md) (deferred items)
**Description:** Current limitation: all worker state must live in the input vector;
returning text or references is unsupported.
**Fix path:**
1. Extra args: synthesise an IR-level wrapper function that captures the extra args as
   closure variables and passes them alongside the element.
2. Text/reference returns: merge worker-local stores back into the main `Stores` after all
   threads join.
**Effort:** High (parser.rs, parallel.rs, store.rs)

---

### T3-2  Logger: production mode, source injection, hot-reload
**Sources:** [LOGGER.md](LOGGER.md)
**Description:**
- Production panic handler writes structured log entry instead of aborting.
- Source-location metadata injected at compile time into assert/log calls.
- Hot-reload of log-level config without restarting the interpreter.
**Effort:** Medium–High (logger.rs, parser.rs, state.rs)

---

### T3-3  Optional Cargo features
**Sources:** OPTIONAL_FEATURES.md
**Description:** Gate subsystems behind `cfg` features: `png` (image support), `gendoc`
(HTML documentation generation), `parallel` (threading), `logging` (logger), `mmap`
(memory-mapped storage).  Remove `rand_core` / `rand_pcg` dead dependencies.
**Effort:** Medium (Cargo.toml, conditional compilation in store.rs, native.rs, main.rs)

---

### T3-4  Spatial index operations (full implementation)
**Sources:** PROBLEMS #22
**Description:** `spacial<T>` collection type: insert, lookup, and iteration operations
are not implemented.  The pre-gate (compile error) was added 2026-03-15.
**Effort:** High (new index type in database.rs and vector.rs)

---

### T3-5  Closure capture for lambda expressions
**Sources:** Depends on T2-1
**Description:** T2-1 defines anonymous functions without variable capture.  Full closures
require the compiler to identify captured variables, allocate a closure record, and pass
it as a hidden argument to the lambda body.  This is a significant IR and bytecode change.
**Effort:** Very High (parser.rs, state.rs, scopes.rs, store.rs)

---

### T3-6  Redundant `const` parameter annotation
**Sources:** Compiler warnings audit 2026-03-15
**Severity:** Low — a `const`-annotated parameter that is never written to inside the
function body does not benefit from the annotation; it is noise that implies a mutation
risk that does not exist
**Description:** After analysing a function body, if a `const_param` variable has no
write operations, the `const` annotation is redundant:
```loft
fn sum(v: const vector<integer>) -> integer {
    // v is never written to — 'const' annotation is redundant but harmless
    total = 0
    for x in v { total += x }
    total
}
```
Note: this is the inverse of a const-violation (writing to a `const` param, which is
already a debug-mode runtime error).  This warning targets unnecessary annotations.
**Fix path:**
1. Add a `writes: u32` counter alongside `uses` in `Variable`; increment on every
   assignment to that variable during second-pass parsing.
2. After parsing the function body, if `writes == 0` for a `const_param` variable, emit
   the warning at the parameter declaration site.
**Effort:** Small–Medium (variables.rs — write counter; warning after function body)
**Target:** 1.1+

---

### T3-7  Stack slot `assign_slots` pre-pass
**Sources:** [ASSIGNMENT.md](ASSIGNMENT.md) Steps 3+4; formerly T1-5 arch
**Severity:** Low — `claim()` at code-generation time is O(n) and couples slot layout to
runtime behaviour; no user-visible correctness impact (the correctness fix was completed
2026-03-13); purely architectural debt
**Description:** Replace the runtime `claim()` call in `byte_code()` with a compile-time
`assign_slots()` pre-pass that uses the precomputed live intervals from `compute_intervals`
to assign stack slots by greedy interval-graph colouring.  Makes slot layout auditable and
removes a source of slot conflicts in long functions with many sequential variable reuses.
**Fix path:**
1. Implement `assign_slots()` in `variables.rs` — sort variables by `first_def`, assign
   each to the lowest slot not occupied by a live variable of incompatible type.
2. Wire into `scopes::check` after `compute_intervals`.
3. Remove `claim()` calls from `src/state/codegen.rs` once all tests pass.
**Effort:** High (variables.rs, scopes.rs, state/codegen.rs)
**Target:** 1.1+

---

### T3-8  Native extension libraries
**Sources:** [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) Phase 2
**Severity:** Low — core language and stdlib cover most use cases; native extensions target
specialised domains (graphics, audio, database drivers) that cannot be expressed in loft
**Description:** Allow separately-packaged libraries to ship a compiled Rust `cdylib`
alongside their `.loft` API files.  The shared library exports `loft_register_v1()` and
registers native functions via `state.static_fn()`.  A new `#native "name"` annotation in
`.loft` API files references an externally-registered symbol (parallel to the existing
`#rust "..."` inline-code annotation).

Example package: an `opengl` library with `src/opengl.loft` declaring `pub fn gl_clear(c: integer);` `#native "n_gl_clear"` and `native/libloft_opengl.so` containing the Rust implementation.
**Fix path:** See [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) Phase 2 (7 files; new `libloading`
optional dependency; new `plugin-api` workspace member).
**Effort:** High (parser, compiler, extensions loader, plugin API crate)
**Depends on:** —
**Target:** 1.1+

---

## Tier R — Repository Extraction

The interpreter lives inside the Dryopea game-engine repository, which gives it the wrong
identity in every public artifact (Cargo.toml, crates.io, README, generated Rust).  All R items
must be complete before tagging 1.0.  None requires language changes; they are purely
packaging and naming work.  The IDE (Tier W) is the continuation after extraction.

**Finding:** Every `.rs` file in `src/` is language-core — there are no game-engine modules.
The only "game" references are ~10 text strings and the `Cargo.toml` identity.

---

### R1  Create standalone repository
**Description:** Create a new public GitHub repository named `loft` (matches binary name
and planned crates.io crate name).  Description: `loft — interpreter for the loft scripting language`.
Before copying, audit these directories that may contain game content and do not belong in
the language repo: `archive/`, `code/`, `work/`, `webassembly/`, `example/`, `todo`.
Drop `Dryopea.iml` (IntelliJ project file).
Everything else copies cleanly: `src/`, `default/`, `doc/`, `tests/`, `Cargo.toml`,
`clippy.toml`, `Makefile`, `LICENSE`.
**Effort:** Trivial

---

### R6  Workspace split (pre-W1 only — defer until IDE work begins)
**Description:** When W1 (WASM Foundation) is started, split the single crate into a Cargo
workspace so `loft-core` can be compiled to both native and `cdylib` (WASM) targets
without pulling CLI code into the WASM bundle:
```
Cargo.toml                     (workspace root)
loft-core/                 (all src/ except main.rs, gendoc.rs; crate-type = ["cdylib","rlib"])
loft-cli/                  ([[bin]] loft; depends on loft-core)
loft-gendoc/               ([[bin]] gendoc; depends on loft-core)
ide/                           (W2+: index.html, src/*.js, sw.js, manifest.json)
```
This change is a **prerequisite for W1** and should happen at the same time, not before.
For 1.0 the single-crate layout is correct and should not be changed early.
**Effort:** Small (Cargo workspace wiring; no logic changes)
**Depends on:** R1–R5; gates W1

---

## Tier W — Web IDE

A fully serverless, single-origin HTML application that lets users write, run, and
explore Loft programs in a browser without installing anything.  The existing Rust
interpreter is compiled to WebAssembly via a new `wasm` Cargo feature; the IDE shell
is plain ES-module JavaScript with no required build step after the WASM is compiled
once.  Full design in [WEB_IDE.md](WEB_IDE.md).

---

### W1  WASM Foundation
**Sources:** [WEB_IDE.md](WEB_IDE.md) — M1
**Severity/Value:** High — nothing else in Tier W is possible without this
**Description:** Compile the interpreter to WASM and expose a typed JS API.
Requires four bounded Rust changes, all behind `#[cfg(feature="wasm")]`:
1. `Cargo.toml` — `wasm` feature gating `wasm-bindgen`, `serde`, `serde-wasm-bindgen`; add `crate-type = ["cdylib","rlib"]`
2. `src/diagnostics.rs` — add `DiagEntry { level, file, line, col, message }` and `structured: Vec<DiagEntry>`; populate from `Lexer::diagnostic()` which already has `position: Position`
3. `src/fill.rs` — `op_print` writes to a `thread_local` `String` buffer instead of `print!()`
4. `src/parser/mod.rs` — virtual FS `thread_local HashMap<String,String>` checked before the real filesystem so `use` statements resolve from browser-supplied files
5. `src/wasm.rs` (new) — `compile_and_run(files: JsValue) -> JsValue` and `get_symbols(files: JsValue) -> JsValue`

JS deliverable: `ide/src/wasm-bridge.js` with `initWasm()` + `compileAndRun()`.
JS tests (4): hello-world, compile-error with position, multi-file `use`, runtime output capture.
**Effort:** Medium (Rust changes bounded; most risk is in virtual-FS wiring)
**Depends on:** —

---

### W2  Editor Shell
**Sources:** [WEB_IDE.md](WEB_IDE.md) — M2
**Severity/Value:** High — the visible IDE; needed by all later W items
**Description:** A single `index.html` users can open directly (no bundler).
- `ide/src/loft-language.js` — CodeMirror 6 `StreamLanguage` tokenizer: keywords, types, string interpolation `{...}`, line/block comments, numbers
- `ide/src/editor.js` — CodeMirror 6 instance with line numbers, bracket matching, `setDiagnostics()` for gutter icons and underlines
- Layout: toolbar (project switcher + Run button), editor left, Console + Problems panels bottom

JS tests (5): keyword token, string interpolation span, line comment, type names, number literal.
**Effort:** Medium (CodeMirror 6 setup + Loft grammar)
**Depends on:** W1

---

### W3  Symbol Navigation
**Sources:** [WEB_IDE.md](WEB_IDE.md) — M3
**Severity/Value:** Medium — go-to-definition and find-usages; significant IDE quality uplift
**Description:**
- `src/wasm.rs`: implement `get_symbols()` — walk `parser.data.def_names` and variable tables; return `[{name, kind, file, line, col, usages:[{file,line,col}]}]`
- `ide/src/symbols.js`: `buildIndex()`, `findAtPosition()`, `formatUsageList()`
- Editor: Ctrl+click → jump to definition; hover tooltip showing kind + file
- Outline panel (sidebar): lists all functions, structs, enums; clicking navigates

JS tests (3): find function definition, format usage list, no-match returns null.
**Effort:** Medium (Rust symbol walk + JS index)
**Depends on:** W1, W2

---

### W4  Multi-File Projects
**Sources:** [WEB_IDE.md](WEB_IDE.md) — M4
**Severity/Value:** High — essential for any real program; single-file is a toy
**Description:** All projects persist in IndexedDB.  Project schema: `{id, name, modified, files:[{name,content}]}`.
- `ide/src/projects.js` — `listProjects()`, `loadProject(id)`, `saveProject(project)`, `deleteProject(id)`; auto-save on edit (debounced 2 s)
- UI: project-switcher dropdown, "New project" dialog, file-tree panel, tab bar, `use` filename auto-complete

JS tests (4): save/load roundtrip, list all projects, delete removes entry, auto-save updates timestamp.
**Effort:** Medium (IndexedDB wrapper + UI wiring)
**Depends on:** W2

---

### W5  Documentation & Examples Browser
**Sources:** [WEB_IDE.md](WEB_IDE.md) — M5
**Severity/Value:** Medium — documentation access without leaving the IDE; example projects lower barrier to entry
**Description:**
- Build-time script `ide/scripts/bundle-docs.js`: parse `doc/*.html` → `assets/docs-bundle.json` (headings + prose + code blocks)
- `ide/src/docs.js` — renders bundle with substring search
- `ide/src/examples.js` — registers `tests/docs/*.loft` as one-click example projects ("Open as project")
- Right-sidebar tabs: **Docs** | **Examples** | **Outline**

Run the bundler automatically from `build.sh` after `cargo run --bin gendoc`.
**Effort:** Small–Medium (bundler script + panel UI)
**Depends on:** W2

---

### W6  Export, Import & PWA
**Sources:** [WEB_IDE.md](WEB_IDE.md) — M6
**Severity/Value:** Medium — closes the loop between browser and local development
**Description:**
- `ide/src/export.js`: `exportZip(project)` → `Blob` (JSZip); `importZip(blob)` → project object; drag-and-drop import
- Export ZIP layout: `<name>/src/*.loft`, `<name>/lib/*.loft` (if any), `README.md`, `run.sh`, `run.bat` — matches `loft`'s `use` resolution path so unzip + run works locally
- `ide/sw.js` — service worker pre-caches all IDE assets; offline after first load
- `ide/manifest.json` — PWA manifest
- URL sharing: single-file programs encoded as `#code=<base64>` in URL

JS tests (4): ZIP contains `src/main.loft`, `run.sh` invokes `loft`, import roundtrip preserves content, URL encode/decode.
**Effort:** Small–Medium (JSZip + service worker)
**Depends on:** W4

---

## Quick Reference

| ID   | Title                                                       | Tier | Effort    | Target  | Depends on  | Source                     |
|------|-------------------------------------------------------------|------|-----------|---------|-------------|----------------------------|
| T1-14 | Scalar patterns in `match` (int, text, bool, …)           | 1    | Medium    | 1.1     |             | MATCH.md T1-14             |
| T1-9  | Dead assignment (overwritten before first read)            | 1    | Small     | 1.1     |             | Warnings audit 2026-03-15  |
| T1-10 | Unused loop variable                                      | 1    | Trivial   | 1.1     |             | Warnings audit 2026-03-15  |
| T1-13 | Unreachable code after return/break/continue              | 1    | Medium    | 1.1     |             | Warnings audit 2026-03-15  |
| T1-16 | Guard clauses (`if`) in `match` arms                     | 1    | Small–Med | 1.1     | T1-14       | MATCH.md T1-16             |
| T1-15 | Or-patterns (`\|`) in `match` arms                       | 1    | Medium    | 1.1     | T1-14       | MATCH.md T1-15             |
| T1-17 | Range patterns in `match` (`lo..=hi`)                    | 1    | Small     | 1.1     | T1-14       | MATCH.md T1-17             |
| T1-18 | Plain struct destructuring in `match`                    | 1    | Small     | 1.1     |             | MATCH.md T1-18             |
| T1-12 | Redundant null check on `not null` type                  | 1    | Small     | 1.1     |             | Warnings audit 2026-03-15  |
| T1-22 | Missing return path for non-null functions               | 1    | Medium    | 1.1     |             | Warnings audit 2026-03-15  |
| T1-23 | Variable shadowing                                       | 1    | Small     | 1.1+    |             | Warnings audit 2026-03-15  |
| T1-19 | Nested patterns in field positions                       | 1    | Medium    | 1.1+    | T1-14,T1-18 | MATCH.md T1-19             |
| T1-20 | Remaining patterns (null, binding `@`)                   | 1    | Small     | 1.1+    | T1-14       | MATCH.md T1-20             |
| T1-21 | Slice and vector patterns                                | 1    | Medium    | 1.1+    | T1-14,T1-15 | MATCH.md T1-21             |
| T2-0  | Code formatter (`loft --format`)                        | 2    | Small–Med | 1.0 tgt |             | FORMATTER.md               |
| T2-13 | Empty `[]` literal unusable as direct mutable vector arg | 2   | Medium    | 1.1     |             | PROBLEMS #44               |
| T2-1  | Lambda / anonymous function expressions                  | 2    | Med–High  | 1.1     | T1-1        | Prototype goal             |
| T2-2  | REPL / interactive mode                                  | 2    | High      | 1.1     |             | Prototype goal             |
| T2-5  | In-place sort for primitive vectors                      | 2    | Medium    | 1.1     |             | Stdlib audit 2026-03-15    |
| T2-7  | File system: `mkdir`, `mkdir_all`                        | 2    | Small     | 1.1     |             | Stdlib audit 2026-03-15    |
| T2-8  | Expose `reverse`, `clear`, `insert` on vectors          | 2    | Low–Med   | 1.1     |             | Stdlib audit 2026-03-15    |
| T2-4  | Vector aggregates (sum, min_of, any, all, count_if)      | 2    | Low–Med   | 1.1     | T2-1        | Stdlib audit 2026-03-15    |
| T2-12 | Bytecode cache (`.loftc`, skip recompile on rerun)      | 2    | Medium    | 1.1     |             | BYTECODE_CACHE.md          |
| T3-1  | Parallel workers: extra args + text/ref returns          | 3    | High      | 1.1+    |             | THREADING deferred         |
| T3-2  | Logger: production mode, source injection               | 3    | Med–High  | 1.1+    |             | LOGGER.md                  |
| T3-3  | Optional Cargo features                                  | 3    | Medium    | 1.1+    |             | OPTIONAL_FEATURES.md       |
| T3-4  | Spatial index operations (full implementation)           | 3    | High      | 1.1+    |             | PROBLEMS #22               |
| T3-5  | Closure capture for lambdas                              | 3    | Very High | 2.0     | T2-1        | Depends on T2-1            |
| T3-6  | Redundant `const` parameter annotation                   | 3    | Small–Med | 1.1+    |             | Warnings audit 2026-03-15  |
| T3-7  | Stack slot `assign_slots` pre-pass (arch cleanup)        | 3    | High      | 1.1+    |             | ASSIGNMENT.md Steps 3+4    |
| T3-8  | Native extension libraries (`cdylib` + `#native`)        | 3    | High      | 1.1+    | —           | EXTERNAL_LIBS.md Ph2       |
| R1    | Create standalone `loft` GitHub repository              | R    | Trivial   | **1.0** |             | Extraction plan            |
| R6    | Workspace split (prerequisite for W1 only)              | R    | Small     | pre-W1  | R1–R5       | Extraction plan            |
| W1    | WASM foundation (Rust feature + wasm-bridge.js)         | W    | Medium    | post-1.0 | R6         | WEB_IDE.md M1              |
| W2    | Editor shell (CodeMirror 6 + Loft grammar)              | W    | Medium    | post-1.0 | W1         | WEB_IDE.md M2              |
| W3    | Symbol navigation (go-to-def, find-usages)              | W    | Medium    | post-1.0 | W1, W2     | WEB_IDE.md M3              |
| W4    | Multi-file projects (IndexedDB)                         | W    | Medium    | post-1.0 | W2         | WEB_IDE.md M4              |
| W5    | Docs & examples browser                                 | W    | Small–Med | post-1.0 | W2         | WEB_IDE.md M5              |
| W6    | Export/import ZIP + PWA offline                         | W    | Small–Med | post-1.0 | W4         | WEB_IDE.md M6              |

**Target key:** **1.0** = hard gate · **1.0 tgt** = target, not blocking · **1.1** = first post-1.0 minor · **1.1+** = later minor · **post-1.0** = independent track · **pre-W1** = must precede W1

_Note: T1-3 requires compiler special-casing (not loft-only) — loft has no generic type parameters._
_Note: W2 and W4 can be developed in parallel once W1 is complete; W3 and W5 can follow independently._

---

## See also
- [../../CHANGELOG.md](../../CHANGELOG.md) — Completed work history (all fixed bugs and shipped features)
- [PROBLEMS.md](PROBLEMS.md) — Known bugs and workarounds
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design asymmetries and surprises
- [ASSIGNMENT.md](ASSIGNMENT.md) — Stack slot assignment status (T3-7 detail)
- [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) — External library packaging design (T3-8 Phase 2)
- [BYTECODE_CACHE.md](BYTECODE_CACHE.md) — Bytecode cache design (T2-12)
- [../DEVELOPERS.md](../DEVELOPERS.md) — Feature proposal process, quality gates, scope rules, and backwards compatibility
- [THREADING.md](THREADING.md) — Parallel for-loop design (T3-1 detail)
- [LOGGER.md](LOGGER.md) — Logger design (T3-2 detail)
- [FORMATTER.md](FORMATTER.md) — Code formatter design (T2-0 detail)
- [WEB_IDE.md](WEB_IDE.md) — Web IDE full design: architecture, JS API contract, per-milestone deliverables and tests, export ZIP layout (Tier W detail)
- [RELEASE.md](RELEASE.md) — 1.0 gate items, project structure changes, release artifacts checklist, post-1.0 versioning policy
