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

**Completed items are removed entirely** — this document is strictly for future work.
Completion history lives in git (commit messages and CHANGELOG.md).  Leaving "done" markers
creates noise and makes the document harder to scan for remaining work.

Sources: [PROBLEMS.md](PROBLEMS.md) · [INCONSISTENCIES.md](INCONSISTENCIES.md) · [ASSIGNMENT.md](ASSIGNMENT.md) · [THREADING.md](THREADING.md) · [LOGGER.md](LOGGER.md) · [WEB_IDE.md](WEB_IDE.md) · [RELEASE.md](RELEASE.md) · [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) · [BYTECODE_CACHE.md](BYTECODE_CACHE.md)

---

## Contents
- [Version Milestones](#version-milestones)
- [Tier 0 — Crashes / Silent Wrong Results](#tier-0--crashes--silent-wrong-results)
- [Tier 1 — Language Quality & Consistency](#tier-1--language-quality--consistency)
- [Tier 2 — Prototype-Friendly Features](#tier-2--prototype-friendly-features)
- [Tier 3 — Architectural / Future Work](#tier-3--architectural--future-work)
- [Tier N — Native Rust Code Generation](#tier-n--native-rust-code-generation)
- [Tier R — Repository Extraction](#tier-r--repository-extraction)
- [Tier W — Web IDE](#tier-w--web-ide)
- [Quick Reference](#quick-reference)

---

## Version Milestones

### Version 0.8.1 — Stability patch (planned)

Fix the three remaining pre-1.0 blockers — no new language features.

1. **T0-11** — Eliminate the unsound DUMMY buffer in release-mode locked stores.
2. **T0-12** — Guard vector slice mutation so it cannot corrupt the parent vector.
3. **T1-32** — Surface file I/O errors instead of silently discarding them.

Pre-release: run the documentation review checklist from [RELEASE.md](RELEASE.md) §§ 1–9.

---

### Version 0.8.0 — Current release (2026-03-17)

Match expressions (enum, scalar, or-patterns, guard clauses, range patterns, null/char
patterns, struct destructuring), code formatter, wildcard imports, callable fn-refs,
map/filter/reduce, vector.clear(), mkdir, time functions, logging, parallel execution,
24+ bug fixes, comprehensive user documentation (24 pages + Safety guide + PDF).

### Version 1.0 — Language Stability

1.0 is a **stability contract**: any program valid on 1.0 compiles and runs identically on any 1.x
release.  Full criteria and release checklist in [RELEASE.md](RELEASE.md).

**Remaining items before 1.0:** T1-28 (error recovery).  T0-11, T0-12, T1-32 ship in 0.8.1.

**Deferred to 1.1+:**
T2-1 (lambdas), T2-2 (REPL), T2-4, T3-1..T3-5, T3-7, T3-8,
N1..N7 (native codegen), W1..W6 (Web IDE; starts after R6)

### Version 1.x — Minor releases (additive)

New language features that are strictly backward-compatible: T2-1, T2-2.
Roughly monthly cadence.  Web IDE (Tier W) is a parallel track independent of interpreter versions.

### Version 2.0 — Breaking changes only

Reserved for language-level breaking changes (syntax removal, sentinel redesign).
Not expected in the near term.

---

### Recommended Implementation Order

Ordered by unblocking impact, batching efficiency, and value-to-effort ratio.
Items on the same line can be done in a single PR.

1. **T1-28** (error recovery) — medium effort, high UX impact
2. **N3** (codegen_runtime) — largest Tier N piece, enables most generated files
3. **T2-1** (lambdas) — unblocks T2-4 and T3-5; makes the language feel modern
4. **N4** (iterators) + **N6** (compile gate) — completes native codegen

Tier W (Web IDE) is an independent parallel track that can start any time after R6.

---

---

## Tier 0 — Crashes / Silent Wrong Results

### T0-11  Unsound DUMMY buffer in release-mode locked stores
**Sources:** store lifecycle; `const` store-lock implementation
**Severity:** High — in release builds, `const`-locked stores reference a DUMMY
sentinel buffer; any write through the lock silently corrupts adjacent heap memory
or produces wrong results instead of failing cleanly.
**Fix path:**
1. Locate the `DUMMY` / sentinel buffer allocated in `allocation.rs` for locked stores.
2. In release builds, replace writes to a locked store with a runtime error or
   `debug_assert!` + safe no-op instead of writing through the dummy.
3. Add a test that verifies locked-store writes produce a clear error in debug builds
   and are safe (no corruption) in release builds.
**Effort:** Small–Medium (allocation.rs; affects const-param code paths)
**Target:** 0.8.1

---

### T0-12  Vector slice mutation corrupts parent vector
**Sources:** T3-11; `src/vector.rs:13` TODO
**Severity:** High — `v[a..b]` returns a slice sharing storage with the parent;
any mutation of that slice (`slice += [x]`, `clear`, `remove`) writes into the
parent's backing store, silently corrupting it.
**Fix path (minimal — runtime guard):**
1. Add an `is_slice: bool` flag to the vector header in `vector.rs`.
2. In every mutating vector op (`OpAppendVector`, `OpInsertVector`,
   `OpClearVector`, `OpRemoveVector`), assert `!is_slice` in debug builds and emit
   a runtime error in release builds.
**Fix path (full CoW — see T3-11):** copy the slice elements to a new allocation on
first mutation.  Prefer the guard for 0.8.1; promote to full CoW (T3-11) in 1.1.
**Effort:** Small for guard; Medium for full CoW (see T3-11)
**Target:** 0.8.1

---

## Tier 1 — Language Quality & Consistency

### T1-28  Error recovery after token failures
**Sources:** [DEVELOPERS.md](../DEVELOPERS.md) § "Diagnostic message quality" Step 5
**Severity:** Medium — a single missing `)` or `}` produces a flood of cascading errors
**Description:** Add `Lexer::recover_to(tokens: &[&str])` that skips tokens until one
of the given delimiters is found.  Call it after `token()` failures in contexts where
cascading is likely: missing `)` skips to `)` or `{`; missing `}` skips to `}` at same
brace depth; missing `=>` in match skips to `=>` or `,`.
**Fix path:**
1. Add `recover_to()` to `lexer.rs` — linear scan forward, stop at matching token or EOF.
2. Modify `token()` to call `recover_to` with context-appropriate delimiters.
3. Add tests that verify a single-error input produces at most 2 diagnostics.
**Effort:** Medium (lexer.rs + parser call sites; needs per-construct recovery targets)
**Target:** 1.1+

---

### T1-19  Nested patterns in field positions
**Sources:** [MATCH.md](MATCH.md) — T1-19
**Severity:** Low — field-level sub-patterns currently require nested `match` or `if` inside the arm body
**Description:** `Order { status: Paid, amount } => charge(amount)` — a field may carry a sub-pattern (`:` separator) instead of (or in addition to) a binding variable.  Sub-patterns generate additional `&&` conditions on the arm.
**Fix path:** See [MATCH.md#t1-19](MATCH.md#t1-19-nested-patterns-in-field-positions) for full design.
Extend field-binding parser to detect `:`; call recursive `parse_sub_pattern(field_val, field_type)` → returns boolean `Value` added to arm conditions with `&&`.
**Effort:** Medium (parser/control.rs — recursive sub-pattern entry point)
**Depends on:** T1-14, T1-18
**Target:** 1.1+

---

### T1-32  File I/O errors are silently discarded
**Sources:** `src/state/io.rs`; pre-1.0 gate item
**Severity:** Medium — `read`, `write`, and related file operations that fail
(permission denied, disk full, path not found) return a default/empty value with
no diagnostic; programs cannot distinguish a successful empty read from a failure.
**Fix path:**
1. Audit every `?`-eliding `unwrap_or` / `unwrap_or_default` in `state/io.rs`
   and `fill.rs` file ops.
2. On failure, push a diagnostic via `lexer.diagnostic()` (level `Error`) and
   return the appropriate null sentinel so the calling loft code can test for null.
3. Expose a `f#error` boolean attribute (analogous to `f#exists`) that loft code
   can query after a read/write.
4. Add tests covering: read non-existent file, write to read-only path, read
   binary file as text — each should produce a null result and a diagnostic.
**Effort:** Medium (io.rs + parser file_op + new attribute)
**Target:** 0.8.1

---

## Tier 2 — Prototype-Friendly Features

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
**Target:** Deferred — superseded by Tier N (native Rust code generation eliminates
the recompile overhead that caching was designed to address)

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

### T3-11  Vector slice becomes independent copy on mutation
**Sources:** TODO in `src/vector.rs:13`
**Severity:** Low — currently a vector slice shares storage with the parent; mutating
the slice can corrupt the parent vector's data
**Description:** `v[a..b]` returns a lightweight slice (same store, different offset/length).
If the slice is subsequently mutated (`slice += [x]`), the mutation writes into the parent's
storage. The fix is copy-on-write: when a slice-derived vector is first mutated, copy its
elements to a new allocation before applying the mutation.
**Fix path:**
1. Add a `is_slice: bool` flag (or `parent_ref: DbRef`) to the vector header.
2. In every mutating vector operation (`OpAppendVector`, `OpInsertVector`, `OpClearVector`,
   `OpRemoveVector`), check the flag and call `vector_copy_to_own(v)` before proceeding.
3. `vector_copy_to_own` allocates a fresh vector, copies elements (with `copy_claims`),
   and updates the DbRef.
**Effort:** Medium (vector.rs, fill.rs — CoW flag + copy-on-first-write)
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

### T3-10  Destination-passing for text-returning native functions
**Sources:** String architecture review 2026-03-16
**Severity:** Low — eliminates the scratch buffer entirely; also removes one intermediate
`String` allocation per format-string expression by letting natives write directly into the
caller's mutable `String`
**Description:** Currently, text-returning natives (`replace`, `to_lowercase`, `to_uppercase`)
create an owned `String`, push it to `scratch`, and return a `Str` pointing into it.  The
caller then copies the `Str` content into a mutable `String` via `OpAppendText`.  This is
two copies: native → scratch → destination.

With destination-passing, the native receives a mutable reference to the caller's `String`
and writes directly into it.  One copy: native → destination.

**Current calling convention:**
```
Stack before call:  [ self:Str, arg1:Str, ... ]
Native executes:    new_value = self.replace(arg1, arg2)
                    scratch.push(new_value)
                    push Str → stack
Stack after call:   [ result:Str ]
Caller:             OpAppendText(dest_var, result)   // copies again
```

**Proposed calling convention:**
```
Stack before call:  [ self:Str, arg1:Str, ..., dest:DbRef ]
Native executes:    let dest: &mut String = stores.get_string_mut(stack)
                    dest.push_str(&self.replace(arg1, arg2))
Stack after call:   [ ]   // result already written to dest
```

**Fix path:**

**Phase 1 — Compiler changes (`state/codegen.rs`, `parser/expressions.rs`):**
1. Add a `TextDest` calling convention flag to text-returning native function definitions
   in `data.rs`.  When the compiler sees a call to a `TextDest` native, it emits an
   `OpCreateStack` pointing to the destination `String` variable as an extra trailing
   argument.
2. Identify the destination variable:
   - If the call is inside `parse_append_text` (format string building), the destination
     is the `__work_N` variable (already known at `expressions.rs:1079`).
   - If the call is in a `v = text.replace(...)` assignment, the destination is `v`
     (if `v` is a mutable `String`).
   - If the call is in a struct field assignment (`obj.name = text.to_uppercase()`), the
     result must go through a work-text and then `set_str()` — no change from current
     behaviour for this case (Phase 2 optimises it).
3. Stop emitting `OpAppendText` after the call — the native already wrote the result.

**Phase 2 — Native function changes (`native.rs`):**
4. Change the signature of `t_4text_replace`, `t_4text_to_lowercase`,
   `t_4text_to_uppercase` to pop the trailing `DbRef` destination argument, resolve it
   to `&mut String`, and `push_str()` into it.
5. Remove `stores.scratch.push(...)` and the `Str` return.  These functions now return
   nothing (void on the stack).
6. If T3-9 was implemented first, remove `OpClearScratch` emission since scratch is no
   longer used.

**Phase 3 — Extend to format expressions (`parser/expressions.rs`):**
7. In `parse_append_text` (`expressions.rs:1070-1119`), the `__work_N` variable is
   currently:
   ```
   OpClearText(work)        // allocate empty String
   OpAppendText(work, lhs)  // copy left fragment
   OpAppendText(work, rhs)  // copy right fragment
   Value::Var(work)         // read as Str
   ```
   With destination-passing, when a text-returning native appears as a fragment, skip
   the intermediate `Str` → `OpAppendText` hop: pass `work` directly as the destination
   to the native call.  This saves one copy per native-call fragment in format strings.
8. When the *entire* expression is a single native call assigned to a text variable
   (`result = text.replace(...)`) and `result` is a mutable `String`, pass `result`
   directly as the destination — eliminating the `__work_N` temporary entirely.

**Phase 4 — Remove scratch buffer:**
9. Once all three natives use destination-passing, remove `Stores.scratch` field
   (`database/mod.rs:118`) and the `scratch.clear()` call (`database/mod.rs:360`).
10. Remove T3-9's `OpClearScratch` if it was added.

**Files changed:**
| File | Change |
|---|---|
| `src/data.rs` | Add `TextDest` flag to function metadata |
| `src/state/codegen.rs` | Emit destination `DbRef` as trailing argument for `TextDest` calls |
| `src/parser/expressions.rs` | Pass destination through `parse_append_text`; skip `OpAppendText` for `TextDest` calls |
| `src/native.rs` | Rewrite 3 functions to pop destination and write directly |
| `src/database/mod.rs` | Remove `scratch` field |
| `src/fill.rs` | Remove `clear_scratch` handler (if T3-9 was done first) |

**Edge cases:**
- **Chained calls** (`text.replace("a","b").replace("c","d")`): the first `replace` writes
  into a work-text; the second reads from it as `Str` self-argument and writes into
  another work-text (or the same one after clear).  Ensure the compiler doesn't pass the
  same `String` as both source and destination — the intermediate work-text is still needed.
- **Parallel workers**: `clone_for_worker()` currently clones `scratch`; with
  destination-passing, no clone needed (workers have their own stack `String` variables).
- **Future text-returning natives** (e.g. `trim`, `repeat`, `join`): any new native
  returning text should use `TextDest` from the start.

**Effort:** Medium–High (compiler calling-convention change + 3 native rewrites + codegen)
**Depends on:** T3-9 (done 2026-03-17)
**Target:** 1.1+

---

## Tier N — Native Rust Code Generation

`src/generation.rs` already translates the loft IR tree into Rust source files
(`tests/generated/*.rs`), but none compile (~1500 errors).  The steps below fix
these incrementally.  Full design in [NATIVE.md](NATIVE.md).

---

### N11  Fix `output_init` to register all intermediate types
**Description:** `output_init` skips intermediate types (vectors inside structs,
plain enum values, byte/short field types), causing type ID misalignment at runtime.
**Effort:** Medium (generation.rs `output_init`)
**Fixes:** `enums_types`, `enums_enum_field` (2 runtime failures)
**Detail:** [NATIVE.md](NATIVE.md) § N10a

---

### N12  Fix `output_set` for DbRef deep copy
**Description:** `Set(var_b, Var(var_a))` for reference types emits a pointer copy.
Add `OpCopyRecord` call after assignment when both sides are same-type references.
**Effort:** Small (generation.rs `output_set`)
**Fixes:** `objects_independent_strings` (1 runtime failure)
**Detail:** [NATIVE.md](NATIVE.md) § N10b

---

### N13  Fix `OpFormatDatabase` for struct-enum variants
**Description:** Formatting outputs only the enum name, not the full struct fields.
Verify `db_tp` argument is the parent enum type so `ShowDb` can dispatch to variant.
**Effort:** Small (codegen_runtime.rs or generation.rs)
**Fixes:** `enums_define_enum`, `enums_general_json` (2 runtime failures)
**Detail:** [NATIVE.md](NATIVE.md) § N10c

---

### N14  Fix null DbRef handling in vector operations
**Description:** Guard `clear_vector` calls with a null check (`rec != 0`) in
generated code.  `stores.null()` returns a DbRef with a valid `store_nr` that
causes panics when passed to vector operations.
**Effort:** Small (generation.rs `output_call` for `OpClearVector`)
**Fixes:** `vectors_fill_result` (1 runtime failure)
**Detail:** [NATIVE.md](NATIVE.md) § N10d

---

---

### N16  Implement `OpIterate`/`OpStep` in codegen_runtime
**Description:** Add iterate/step state machine for sorted/index/vector collections.
Handle `Value::Iter` in `output_code_inner` by emitting a loop with these functions.
**Effort:** High (codegen_runtime.rs + generation.rs)
**Fixes:** 3 compile failures (iterator tests)
**Detail:** [NATIVE.md](NATIVE.md) § N10e-2

---

### N17  Add `OpFormatFloat`/`OpFormatStackLong` handlers
**Description:** Add `output_call` special cases that emit direct calls to
`ops::format_float` / `ops::format_long` with the correct `&mut String` argument.
**Effort:** Small (generation.rs `output_call`)
**Fixes:** 2 compile failures
**Detail:** [NATIVE.md](NATIVE.md) § N10e-3

---

---

### N19  Fix empty pre-eval and prefix issues
**Description:** Skip pre-eval bindings when expression is empty; change `_pre{n}`
naming to `_pre_{n}` to avoid Rust prefix parsing; fix `OpGetRecord` argument count.
**Effort:** Small (generation.rs)
**Fixes:** 3 compile failures
**Detail:** [NATIVE.md](NATIVE.md) § N10e-5

---

### N8  Add `--native` CLI flag
**Description:** Add `--native <file.loft>` to `src/main.rs`: parse, generate Rust
source via `Output::output_native()`, compile with `rustc`, run the binary.
**Effort:** Medium
**Depends on:** N7

---

### N20  Repair fill.rs auto-generation
**Description:** Make `create.rs::generate_code()` produce a `fill.rs` that can
replace the hand-maintained `src/fill.rs`. Add `ops` import, fix formatting,
add CI check for drift, and introduce `#state_call` annotation for the 52
delegation operators. Eliminates manual maintenance when adding new opcodes.
**Effort:** Medium (create.rs + default/*.loft + CI)
**Detail:** [NATIVE.md](NATIVE.md) § N20

---

## Tier R — Repository Extraction

Standalone `loft` repository created (R1, 2026-03-16).  The remaining R item is the
workspace split needed before starting the Web IDE.

---

### R6  Workspace split (pre-W1 only — defer until IDE work begins)
**Description:** When W1 (WASM Foundation) is started, split the single crate into a Cargo
workspace so `loft-core` can be compiled to both native and `cdylib` (WA1SM) targets
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
**Depends on:** R1 (done); gates W1

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
| T0-11 | DUMMY buffer unsound in release-mode locked stores       | 0    | Sm–Med    | 0.8.1   |             | Store lifecycle            |
| T0-12 | Vector slice mutation corrupts parent                    | 0    | Small     | 0.8.1   |             | vector.rs TODO             |
| T1-32 | File I/O errors silently discarded                      | 1    | Medium    | 0.8.1   |             | state/io.rs                |
| T1-28 | Error recovery after token failures                      | 1    | Medium    | 1.1+    |             | DEVELOPERS.md Step 5       |
| T1-19 | Nested patterns in field positions                       | 1    | Medium    | 1.1+    | T1-14,T1-18 | MATCH.md T1-19             |
| T2-1  | Lambda / anonymous function expressions                  | 2    | Med–High  | 1.1     | T1-1        | Prototype goal             |
| T2-2  | REPL / interactive mode                                  | 2    | High      | 1.1     |             | Prototype goal             |
| T2-4  | Vector aggregates (sum, min_of, any, all, count_if)      | 2    | Low–Med   | 1.1     | T2-1        | Stdlib audit 2026-03-15    |
| T2-12 | Bytecode cache (`.loftc`) — deferred, superseded by Tier N | 2  | Medium    | deferred |            | BYTECODE_CACHE.md          |
| T3-1  | Parallel workers: extra args + text/ref returns          | 3    | High      | 1.1+    |             | THREADING deferred         |
| T3-2  | Logger: production mode, source injection, hot-reload   | 3    | Med–High  | 1.1+    |             | LOGGER.md                  |
| T3-3  | Optional Cargo features                                  | 3    | Medium    | 1.1+    |             | OPTIONAL_FEATURES.md       |
| T3-4  | Spatial index operations (full implementation)           | 3    | High      | 1.1+    |             | PROBLEMS #22               |
| T3-5  | Closure capture for lambdas                              | 3    | Very High | 2.0     | T2-1        | Depends on T2-1            |
| T3-10 | Destination-passing for text-returning natives            | 3    | Med–High  | 1.1+    | T3-9 (done) | String arch review         |
| T3-11 | Vector slice becomes independent copy on mutation        | 3    | Medium    | 1.1+    |             | TODO in vector.rs          |
| T3-7  | Stack slot `assign_slots` pre-pass (arch cleanup)        | 3    | High      | 1.1+    |             | ASSIGNMENT.md Steps 3+4    |
| T3-8  | Native extension libraries (`cdylib` + `#native`)        | 3    | High      | 1.1+    | —           | EXTERNAL_LIBS.md Ph2       |
| N8    | `--native` CLI flag                                     | N    | Medium    | 1.1+    | N11–N19     | NATIVE.md                  |
| N11   | Fix `output_init` intermediate type registration         | N    | Medium    | 1.1     |             | NATIVE.md N10a             |
| N12   | Fix `output_set` DbRef deep copy                        | N    | Small     | 1.1     |             | NATIVE.md N10b             |
| N13   | Fix `OpFormatDatabase` for struct-enum variants          | N    | Small     | 1.1     |             | NATIVE.md N10c             |
| N14   | Fix null DbRef in vector operations                     | N    | Small     | 1.1     |             | NATIVE.md N10d             |
| N16   | Implement `OpIterate`/`OpStep` in codegen_runtime        | N    | High      | 1.1     |             | NATIVE.md N10e-2           |
| N17   | Add `OpFormatFloat`/`OpFormatStackLong` handlers         | N    | Small     | 1.1     |             | NATIVE.md N10e-3           |
| N19   | Fix empty pre-eval and prefix issues                    | N    | Small     | 1.1     |             | NATIVE.md N10e-5           |
| N20   | Repair fill.rs auto-generation                          | N    | Medium    | 1.1     |             | NATIVE.md N20              |
| R6    | Workspace split (prerequisite for W1 only)              | R    | Small     | pre-W1  | R1 (done)   | Extraction plan            |
| W1    | WASM foundation (Rust feature + wasm-bridge.js)         | W    | Medium    | post-1.0 | R6         | WEB_IDE.md M1              |
| W2    | Editor shell (CodeMirror 6 + Loft grammar)              | W    | Medium    | post-1.0 | W1         | WEB_IDE.md M2              |
| W3    | Symbol navigation (go-to-def, find-usages)              | W    | Medium    | post-1.0 | W1, W2     | WEB_IDE.md M3              |
| W4    | Multi-file projects (IndexedDB)                         | W    | Medium    | post-1.0 | W2         | WEB_IDE.md M4              |
| W5    | Docs & examples browser                                 | W    | Small–Med | post-1.0 | W2         | WEB_IDE.md M5              |
| W6    | Export/import ZIP + PWA offline                         | W    | Small–Med | post-1.0 | W4         | WEB_IDE.md M6              |

**Target key:** **1.1** = first post-1.0 minor · **1.1+** = later minor · **post-1.0** = independent track · **pre-W1** = must precede W1

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
- [NATIVE.md](NATIVE.md) — Native Rust code generation: root cause analysis, step details, verification (Tier N detail)
- [WEB_IDE.md](WEB_IDE.md) — Web IDE full design: architecture, JS API contract, per-milestone deliverables and tests, export ZIP layout (Tier W detail)
- [RELEASE.md](RELEASE.md) — 1.0 gate items, project structure changes, release artifacts checklist, post-1.0 versioning policy
