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
5. **Developed in small, verified steps** — each feature is complete and tested before the next
   begins.  No half-implementations are shipped.  No feature is added "just in case".  Every
   release must be smaller and better than its estimate, never larger.  This is the primary
   defence against regressions and against the codebase growing beyond one person's ability to
   understand it fully.

The items below are ordered by tier: things that break programs come first, then language-quality
and prototype-friction items, then architectural work.  See [RELEASE.md](RELEASE.md) for the full
release gate criteria, project structure changes, and release artifact checklist.

**Completed items are removed entirely** — this document is strictly for future work.
Completion history lives in git (commit messages and CHANGELOG.md).  Leaving "done" markers
creates noise and makes the document harder to scan for remaining work.

Sources: [PROBLEMS.md](PROBLEMS.md) · [INCONSISTENCIES.md](INCONSISTENCIES.md) · [ASSIGNMENT.md](ASSIGNMENT.md) · [THREADING.md](THREADING.md) · [LOGGER.md](LOGGER.md) · [WEB_IDE.md](WEB_IDE.md) · [RELEASE.md](RELEASE.md) · [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) · [BYTECODE_CACHE.md](BYTECODE_CACHE.md)

---

## Contents
- [Version Milestones](#version-milestones)
  - [Milestone Reevaluation](#milestone-reevaluation)
  - [Recommended Implementation Order](#recommended-implementation-order)
- [L — Language Quality](#l--language-quality)
- [P — Prototype Features](#p--prototype-features)
- [A — Architecture](#a--architecture)
- [N — Native Codegen](#n--native-codegen)
- [R — Repository](#r--repository)
- [W — Web IDE](#w--web-ide)
- [Quick Reference](#quick-reference) → [ROADMAP.md](ROADMAP.md)

---

## Version Milestones

### Version 0.8.1 — Stability patch (2026-03-18)

Three correctness fixes — no new language features.

- **T0-11** — `addr_mut()` on a locked store now panics (replaced the silent DUMMY buffer).
- **T0-12** — `vector_add()` snapshots source bytes before resize; `v += v` is now correct.
- **T1-32** — `write_file`, `read_file`, `seek_file` log errors to stderr instead of silently discarding them.

---

### Version 0.8.0 — Released (2026-03-17)

Match expressions (enum, scalar, or-patterns, guard clauses, range patterns, null/char
patterns, struct destructuring), code formatter, wildcard imports, callable fn-refs,
map/filter/reduce, vector.clear(), mkdir, time functions, logging, parallel execution,
24+ bug fixes, comprehensive user documentation (24 pages + Safety guide + PDF).

---

### Version 0.9.0 — Production-ready standalone executable (planned)

Goal: an interpreter a developer can rely on for real programs.  Every planned language
feature is present; no known crashes or silent wrong results; no obvious performance
cliffs; the binary is lean and ships pre-built.

**Language completeness:**
- **L1** — Error recovery: a single bad token must not cascade into dozens of spurious errors.
- **P1** — Lambda expressions: inline `fn(x: T) -> U { ... }` without a top-level name.
- **P3** — Vector aggregates: `sum`, `min_of`, `max_of`, `any`, `all`, `count_if` (depends on P1).
- **L2** — Nested match patterns: field sub-patterns in struct arms.

**Interpreter correctness and stability:**
- **A9** — Vector slice copy-on-write: mutating a slice must not corrupt the parent vector.
- **A6** — Stack slot `assign_slots` pre-pass: compile-time slot layout replaces the current runtime `claim()` calls, eliminating the remaining category of slot-conflict bugs.

**Efficiency and packaging:**
- **A8** — Destination-passing for string natives: eliminates the double-copy overhead on `replace`, `to_lowercase`, `to_uppercase` and format expressions.
- **A3** — Optional Cargo features: gate `png`, `parallel`, `logging`, `mmap` behind `cfg` features for a lean default binary.
- **Tier N** — Native code generation: fix the ~1500 compile errors in `src/generation.rs` incrementally (N2–N9) so that `loft --native` produces correct compiled Rust output.  Each N step is a small, independent fix with its own test; they can interleave with other 0.9.0 work.  N1 (`--native` CLI flag) lands last, once all fixes pass.

**Parallel execution completeness:**
- **A1** — Parallel workers with extra context arguments and text/reference return types.

**Deferred from 0.9.0:**
- P2 (REPL) — High effort; the browser IDE largely covers the interactive use case. Revisit after 1.0.0.
- A5 (closure capture) — Depends on P1; very high effort; 1.1+.
- A7 (native extension libraries) — Useful after the ecosystem exists; 1.1+.

---

### Version 1.0.0 — Complete IDE + stability contract (planned)

Goal: a fully working, friendly IDE that lets users write and run loft programs in a
browser without installing anything, paired with a stable, feature-complete interpreter.

The **stability contract** — any program valid on 1.0.0 compiles and runs identically on
any 1.0.x or 1.x.0 release — covers both the language surface and the public IDE API.
Full gate criteria in [RELEASE.md](RELEASE.md).

**Prerequisites:**
- **R1** — Workspace split into `loft-core` + `loft-cli` + `loft-gendoc` (enables the `cdylib` WASM target without affecting the CLI binary).

**Web IDE (W1–W6):**
- **W1** — WASM foundation: compile interpreter to WASM, expose typed JS API.
- **W2** — Editor shell: CodeMirror 6 with Loft grammar, diagnostics, toolbar.
- **W3** — Symbol navigation: go-to-definition, find-usages, outline panel.
- **W4** — Multi-file projects: IndexedDB persistence, tab bar, `use` auto-complete.
- **W5** — Documentation and examples browser: embedded HTML docs + one-click example projects.
- **W6** — Export/import ZIP + PWA: offline support, URL sharing, drag-and-drop import.

**Stability gate (same as RELEASE.md §§ 1–9):**
- All INCONSISTENCIES.md entries addressed or documented as accepted behaviour.
- Full documentation review; pre-built binaries for all four platforms; crates.io publish.

**Deferred to 1.1+:**
P2, A5, A7, Tier N (native codegen).

---

### Version 1.x — Minor releases (additive)

New language features that are strictly backward-compatible.  Candidates: P2 (REPL),
A5 (closures), A7 (native extensions), Tier N (native codegen).

---

### Version 2.0 — Breaking changes only

Reserved for language-level breaking changes (sentinel redesign, syntax removal).
Not expected in the near term.

---

### Milestone Reevaluation

The previous plan had 1.0 as a language-stability contract for the interpreter alone,
with the Web IDE deferred indefinitely to "post-1.0".  This reevaluation changes both
milestones and adds the small-steps goal.  The reasoning:

**Why introduce 0.9.0?**
The old plan reached the current state (0.8.1) and declared "L1 is the last blocker
before 1.0", but that understated what "fully featured" actually requires.  Several items
(P1 lambdas, A9 vector CoW, A6 slot pre-pass, A8 string efficiency, A1
parallel completeness) are not optional polish — they close correctness and usability
gaps that a production-ready interpreter must not have.  A 0.9.0 milestone gives these
items a home without inflating the 1.0 scope.

**Why include the IDE in 1.0.0?**
A standalone interpreter 1.0 that is later extended with a breaking IDE integration
produces two separate stability contracts to maintain.  The Web IDE (W1–W6) is already
concretely designed in [WEB_IDE.md](WEB_IDE.md) and is bounded, testable work.  Deferring
it to "post-1.0" without a milestone risks it never shipping.  In 2026, "fully featured"
for a scripting language includes browser-accessible tooling; shipping a 1.0 without it
would require walking back that claim at 1.1.

**Why include native codegen (Tier N) in 0.9.0?**
`src/generation.rs` already translates the loft IR to Rust source; the code exists but
does not compile.  The N items are incremental bug fixes — each is Small or Medium effort,
independent of the others, and each makes more generated tests pass.  Fixing them during
0.9.0 turns an existing but broken feature into a working opt-in performance path for
the first production release, at low marginal cost.  Leaving them for 1.1+ would mean
shipping a 0.9.0 binary that silently generates uncompilable output.

**Why deprioritize REPL (P2)?**
The Web IDE (W2 editor + W1 WASM runtime) covers the interactive "try a snippet"
use case that motivates a REPL.  Building both duplicates effort.  P2 remains in
the backlog for 1.1+ if users ask for a terminal-based interactive mode after 1.0.0.

**The small-steps principle in practice:**
Each milestone above is a strict subset of the next.  0.9.0 ships nothing from the IDE
track; 1.0.0 adds exactly R1 + W1–W6 on top of a complete 0.9.0.  No item moves forward
until the test suite for the previous item is green.  This prevents the "everything at
once" failure mode where half-finished features interact and regressions are hard to pin.

---

### Recommended Implementation Order

Ordered by unblocking impact and the small-steps principle (each item leaves the codebase
in a better state than it found it, with passing tests).

**For 0.9.0:**
1. **L1** — error recovery; standalone UX improvement, no dependencies
2. **P1** — lambdas; unblocks P3, A5; makes the language feel complete
3. **P3** + **L2** — aggregates and nested patterns; depends on P1; batch together
4. **A9** + **A6** — vector CoW + slot pre-pass; correctness; can share a branch
5. **A8** + **A3** — string efficiency + optional features; packaging polish
6. **N2–N9** — native codegen fixes; each is independent, interleave freely with other work
7. **N1** — `--native` CLI flag; lands after all N2–N9 fixes pass
8. **A1** — parallel completeness; isolated change, touches parallel.rs only

**For 1.0.0 (after 0.9.0 is tagged):**
7. **R1** — workspace split; small change, unblocks all Tier W
8. **W1** — WASM foundation; highest risk in the IDE track; do first
9. **W2** + **W4** — editor shell + multi-file projects; can develop in parallel after W1
10. **W3** + **W5** — symbol navigation + docs browser; can follow independently
11. **W6** — export/import + PWA; closes the loop

---

## L — Language Quality

### L1  Error recovery after token failures
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
**Target:** 0.9.0

---

### L2  Nested patterns in field positions
**Sources:** [MATCH.md](MATCH.md) — L2
**Severity:** Low — field-level sub-patterns currently require nested `match` or `if` inside the arm body
**Description:** `Order { status: Paid, amount } => charge(amount)` — a field may carry a sub-pattern (`:` separator) instead of (or in addition to) a binding variable.  Sub-patterns generate additional `&&` conditions on the arm.
**Fix path:** See [MATCH.md § L2](MATCH.md) for full design.
Extend field-binding parser to detect `:`; call recursive `parse_sub_pattern(field_val, field_type)` → returns boolean `Value` added to arm conditions with `&&`.
**Effort:** Medium (parser/control.rs — recursive sub-pattern entry point)
**Target:** 0.9.0

---

## P — Prototype Features

### P1  Lambda / anonymous function expressions
**Sources:** Prototype-friendly goal; callable fn refs already complete (landed in 0.8.0)
**Severity:** Medium — without lambdas, `map` / `filter` require a named top-level function
for every single-use transform, which is verbose for prototyping
**Description:** Allow inline function literals at the expression level:
```loft
doubled = map(items, fn(x: integer) -> integer { x * 2 });
evens   = filter(items, fn(x: integer) -> boolean { x % 2 == 0 });
```
An anonymous function expression produces a `Type::Function` value, exactly like `fn <name>`,
but the body is compiled inline.  No closure capture is required initially (captured variables
can be added in a follow-up, see A5).
**Fix path:**

**Phase 1 — Parser** (`src/parser/expressions.rs`):
Recognise `fn '(' params ')' '->' type block` as a primary expression and produce a new
IR node (e.g. `Value::Lambda`).  Existing `fn <name>` references are unaffected.
*Tests:* parser accepts valid lambda syntax; rejects malformed lambdas with a clear
diagnostic; all existing `fn_ref_*` tests still pass.

**Phase 2 — Compilation** (`src/state/codegen.rs`, `src/compile.rs`):
Synthesise a unique anonymous definition name, compile the body as a top-level function,
and emit the def-nr as `Value::Int` — the same representation as a named `fn <name>` ref.
*Tests:* a basic `fn(x: integer) -> integer { x * 2 }` can be assigned to a variable
and called through it; type checker accepts it wherever a `fn(integer) -> integer` is
expected.

**Phase 3 — Integration with map / filter / reduce**:
Verify that anywhere a named `fn <name>` ref works, an inline `fn(...)` expression also
works.  No compiler changes expected — the def-nr representation is already compatible.
*Tests:* `map(v, fn(x: integer) -> integer { x * 2 })`, `filter` and `reduce` with
inline lambdas; nested lambdas (lambda passed to a lambda).

**Effort:** Medium–High (parser.rs, compile.rs)
**Target:** 0.9.0

---

### P2  REPL / interactive mode
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
**Fix path:**

**Phase 1 — Input completeness detection** (`src/repl.rs`, new):
A pure function `is_complete(input: &str) -> bool` that tracks brace/paren depth to decide
whether to prompt for more input.  No parsing or execution involved.
*Tests:* single-line expressions return `true`; `fn foo() {` returns `false`;
`fn foo() {\n}` returns `true`; unclosed string literal returns `false`.

**Phase 2 — Single-statement execution** (`src/repl.rs`, `src/main.rs`):
Read one complete input, parse and execute it in a persistent `State` and `Stores`; no
output yet.  New type definitions and variable bindings accumulate across iterations.
*Tests:* `x = 42` persists; a subsequent `x + 1` evaluates to `43` in the same session.

**Phase 3 — Value output**:
Non-void expression results are printed automatically after execution; void statements
(assignments, `for` loops) produce no output.
*Tests:* entering `42` prints `42`; `x = 1` prints nothing; `"hello"` prints `hello`.

**Phase 4 — Error recovery**:
A parse or runtime error prints diagnostics and the session continues; the `State` is
left at the last successful checkpoint.
*Tests:* entering `x =` (syntax error) prints one diagnostic and re-prompts;
`x = 1` then succeeds and `x` holds `1`.

**Effort:** High (main.rs, parser.rs, new repl.rs)
**Target:** 1.1+ (browser IDE covers the interactive use case at 1.0.0)

---

### P3  Vector aggregates — `sum`, `min_of`, `max_of`, `any`, `all`, `count_if`
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
Note: naming these `min_of`/`max_of` (not `min`/`max`) avoids collision with the built-in `min`/`max` stdlib functions.
**Fix path:** Typed loft overloads using `reduce` for sum/min_of/max_of; compiler
special-case in `parse_call` for `any`/`all`/`count_if` (same level of effort as similar compiler special-cases).
**Effort:** Low for aggregates (pure loft); Medium for any/all/count_if (compiler)
**Target:** 0.9.0 — batch all variants after P1 lands

---

### P4  Bytecode cache (`.loftc`)
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

## A — Architecture

### A1  Parallel workers: extra arguments and text/reference return types
**Sources:** [THREADING.md](THREADING.md) (deferred items)
**Description:** Current limitation: all worker state must live in the input vector;
returning text or references is unsupported.  These are two independent sub-problems.
**Fix path:**

**Phase 1 — Extra context arguments** (`src/parser/collections.rs`, `src/parallel.rs`):
Synthesise an IR-level wrapper function that closes over the extra arguments and calls
the original worker with `(element, extra_arg_1, extra_arg_2, ...)`.  The wrapper is
generated at compile time; the runtime parallel dispatch is unchanged.
*Tests:* `par([1,2,3], fn worker, threshold)` where `worker(n: integer, t: integer) -> integer`
correctly uses `threshold`; two-arg context test (currently in `tests/threading.rs` as
`parallel_two_context_args`, marked `#[ignore]`) passes.

**Phase 2 — Text/reference return types** (`src/parallel.rs`, `src/store.rs`):
After all worker threads join, merge worker-local stores back into the main `Stores` so
that text values and reference fields in the result vector point into live records.
*Tests:* `par([1,2,3], fn label)` where `label(n: integer) -> text` returns a formatted
string; the result vector contains correct, independent text values with no dangling pointers.

**Effort:** High (parser.rs, parallel.rs, store.rs)
**Target:** 0.9.0

---

### A2  Logger: production mode, source injection, hot-reload
**Sources:** [LOGGER.md](LOGGER.md)
**Description:** Three independent improvements to the logging system.
**Fix path:**

**Phase 1 — Structured production panic handler** (`src/logger.rs`, `src/state/mod.rs`):
In production mode, replace the bare `panic!()` in the runtime error path with a call that
writes a structured JSON log entry (level, file, line, message) and sets `had_fatal`.
*Tests:* extend `production_mode_panic_sets_had_fatal` to verify the log file contains a
parseable JSON entry with the correct message field.

**Phase 2 — Source-location injection** (`src/parser/control.rs`, `src/state/codegen.rs`):
At compile time, `assert()` and `log_info/warn/error/fatal()` calls embed the source
file path and line number as string literals in the emitted bytecode.  No runtime overhead.
*Tests:* a runtime error message includes `file:line` in the expected format; the format
is stable across compiler runs on the same source.

**Phase 3 — Hot-reload of log-level config** (`src/logger.rs`):
The logger polls the config file (or uses `inotify`/`kqueue`) and updates the active log
level on change without restarting the interpreter.
*Tests:* write a config file; run a loft program that logs at multiple levels; change the
config file mid-run; verify subsequent log calls respect the new level.

**Effort:** Medium–High (logger.rs, parser.rs, state.rs)
**Target:** 1.1+

---

### A3  Optional Cargo features
**Sources:** OPTIONAL_FEATURES.md
**Description:** Gate subsystems behind `cfg` features: `png` (image support), `gendoc`
(HTML documentation generation), `parallel` (threading), `logging` (logger), `mmap`
(memory-mapped storage).  Remove `rand_core` / `rand_pcg` dead dependencies.
**Effort:** Medium (Cargo.toml, conditional compilation in store.rs, native.rs, main.rs)
**Target:** 0.9.0

---

### A4  Spatial index operations (full implementation)
**Sources:** PROBLEMS #22
**Description:** `spacial<T>` collection type: insert, lookup, and iteration operations
are not implemented.  The pre-gate (compile error) was added 2026-03-15.
**Fix path:**

**Phase 1 — Insert and exact lookup** (`src/database/`, `src/fill.rs`):
Implement `spacial.insert(elem)` and `spacial[key]` for point queries.  Remove the
compile-error pre-gate for these two operations only; all other `spacial` ops remain gated.
*Tests:* insert 3 points, retrieve each by exact key; null returned for missing key.

**Phase 2 — Bounding-box range query** (`src/database/`, `src/parser/collections.rs`):
Implement `for e in spacial[x1..x2, y1..y2]` returning all elements within a bounding box.
*Tests:* 10 points; query a sub-region; verify count and identity of results.

**Phase 3 — Removal** (`src/database/`):
Implement `spacial[key] = null` and `remove` inside an active iterator.
*Tests:* insert 5, remove 2, verify 3 remain and removed points are never returned.

**Phase 4 — Full iteration** (`src/database/`, `src/state/io.rs`):
Implement `for e in spacial` visiting all elements; compatible with the existing iterator
protocol (sorted/index/vector).  Remove the remaining pre-gate.
*Tests:* insert N points, iterate all, count matches N; reverse iteration produces correct order.

**Effort:** High (new index type in database.rs and vector.rs)

---

### A5  Closure capture for lambda expressions
**Sources:** Depends on P1
**Description:** P1 defines anonymous functions without variable capture.  Full closures
require the compiler to identify captured variables, allocate a closure record, and pass
it as a hidden argument to the lambda body.  This is a significant IR and bytecode change.
**Fix path:**

**Phase 1 — Capture analysis** (`src/scopes.rs`, `src/parser/expressions.rs`):
Walk the lambda body's IR and identify all free variables (variables referenced inside
the body that are defined in an enclosing scope).  No code generation yet.
*Tests:* static analysis correctly identifies free variables in sample lambdas; variables
defined inside the lambda are not flagged; non-capturing lambdas produce an empty set.

**Phase 2 — Closure record layout** (`src/data.rs`, `src/typedef.rs`):
For each capturing lambda, synthesise an anonymous struct type whose fields hold the
captured variables; verify field offsets and total size.
*Tests:* closure struct has the correct field count, types, and sizes; `sizeof` matches
the expected layout.

**Phase 3 — Capture at call site** (`src/state/codegen.rs`):
At the point where a lambda expression is evaluated, emit code to allocate a closure
record and copy the current values of the captured variables into it.  Pass the record
as a hidden trailing argument alongside the def-nr.
*Tests:* captured variable has the correct value when the lambda is called immediately
after its definition.

**Phase 4 — Closure body reads** (`src/state/codegen.rs`, `src/fill.rs`):
Inside the compiled lambda function, redirect reads of captured variables to load from
the closure record argument rather than the (non-existent) enclosing stack frame.
*Tests:* captured variable is correctly read after the enclosing function has returned;
modifying the original variable after capture does not affect the lambda's copy (value
semantics — mutable capture is out of scope for this item).

**Phase 5 — Lifetime and cleanup** (`src/scopes.rs`):
Emit `OpFreeRef` for the closure record at the end of the enclosing scope.
*Tests:* no store leak after a lambda goes out of scope; LIFO free order is respected
when multiple closures are live simultaneously.

**Effort:** Very High (parser.rs, state.rs, scopes.rs, store.rs)
**Depends on:** P1
**Target:** 1.1+

---

### A6  Stack slot `assign_slots` pre-pass
**Sources:** [ASSIGNMENT.md](ASSIGNMENT.md) Steps 3+4
**Severity:** Low — `claim()` at code-generation time is O(n) and couples slot layout to
runtime behaviour; no user-visible correctness impact (the correctness fix was completed
2026-03-13); purely architectural debt
**Description:** Replace the runtime `claim()` call in `byte_code()` with a compile-time
`assign_slots()` pre-pass that uses the precomputed live intervals from `compute_intervals`
to assign stack slots by greedy interval-graph colouring.  Makes slot layout auditable and
removes a source of slot conflicts in long functions with many sequential variable reuses.
**Fix path:**

**Phase 1 — Standalone implementation** (`src/variables.rs`):
Add `assign_slots()` as a standalone function: sort variables by `first_def`, assign each
to the lowest slot not occupied by a live variable of incompatible type.  Do **not** wire
it into the main pipeline yet — `claim()` remains the active mechanism.
*Tests:* unit tests in `variables.rs` verify the greedy colouring produces the correct
slot assignments for a representative set of live-interval patterns; all existing tests
pass unchanged.

**Phase 2 — Shadow mode** (`src/scopes.rs`):
Call `assign_slots()` from `scopes::check` after `compute_intervals`, then assert that its
output agrees with the slots `claim()` produces during code generation.  Mismatches log a
warning but do not abort, making divergences visible without breaking anything.
*Tests:* the full test suite passes; any mismatch is surfaced as a test warning so it can
be investigated before Phase 3.

**Phase 3 — Replace `claim()`** (`src/state/codegen.rs`):
Remove `claim()` calls; `assign_slots()` is now the sole slot-layout mechanism.  The
shadow assertions from Phase 2 become the permanent correctness check.
*Tests:* full test suite passes with zero regressions; `cargo test` green on all platforms.

**Effort:** High (variables.rs, scopes.rs, state/codegen.rs)
**Target:** 0.9.0

---

### A9  Vector slice becomes independent copy on mutation
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
**Target:** 0.9.0

---

### A7  Native extension libraries
**Sources:** [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) Phase 2
**Severity:** Low — core language and stdlib cover most use cases; native extensions target
specialised domains (graphics, audio, database drivers) that cannot be expressed in loft
**Description:** Allow separately-packaged libraries to ship a compiled Rust `cdylib`
alongside their `.loft` API files.  The shared library exports `loft_register_v1()` and
registers native functions via `state.static_fn()`.  A new `#native "name"` annotation in
`.loft` API files references an externally-registered symbol (parallel to the existing
`#rust "..."` inline-code annotation).

Example package: an `opengl` library with `src/opengl.loft` declaring `pub fn gl_clear(c: integer);` `#native "n_gl_clear"` and `native/libloft_opengl.so` containing the Rust implementation.
**Fix path:**
- **Phase 1 — `#native` annotation + symbol registration** (parser, compiler, `state.rs`):
  Parse `#native "symbol_name"` on `pub fn` declarations in `.loft` API files.  In the
  compiler, emit a call to a new `OpCallNative(symbol_id)` opcode that dispatches via a
  `HashMap<String, NativeFn>` registered at startup.  Add `State::register_native()` for
  tests.  Test: register a hand-written Rust function, call it from loft, verify result.
- **Phase 2 — `cdylib` loader** (new optional feature `native-ext`, `libloading` dep):
  Add `State::load_plugin(path)` that `dlopen`s the shared library and calls
  `loft_register_v1(state)`.  Gated behind `--features native-ext` so the default binary
  stays free of `libloading`.  Test: build a minimal `cdylib` in the test suite, load it,
  verify it registers correctly.
- **Phase 3 — package layout + `plugin-api` crate** (new workspace member):
  Introduce `loft-plugin-api/` with the stable C ABI (`loft_register_v1`, `NativeFnCtx`).
  Document the package layout (`src/*.loft` + `native/lib*.so`).  Add an example package
  under `examples/opengl-stub/`.  Update EXTERNAL_LIBS.md to reflect the final API.

Full detail in [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) Phase 2.
**Effort:** High (parser, compiler, extensions loader, plugin API crate)
**Depends on:** —
**Target:** 1.1+ (useful after the ecosystem exists; not needed for 1.0.0)

---

### A8  Destination-passing for text-returning native functions
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
6. Remove `OpClearScratch` emission since scratch is no longer used.

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
10. Remove `OpClearScratch` from `fill.rs` if it was added.

**Files changed:**
| File | Change |
|---|---|
| `src/data.rs` | Add `TextDest` flag to function metadata |
| `src/state/codegen.rs` | Emit destination `DbRef` as trailing argument for `TextDest` calls |
| `src/parser/expressions.rs` | Pass destination through `parse_append_text`; skip `OpAppendText` for `TextDest` calls |
| `src/native.rs` | Rewrite 3 functions to pop destination and write directly |
| `src/database/mod.rs` | Remove `scratch` field |
| `src/fill.rs` | Remove `clear_scratch` handler (scratch buffer removal already complete) |

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
**Note:** scratch buffer removal (OpClearScratch) was completed 2026-03-17 and is a prerequisite; some conditionals in the Fix path above reference it as already done.
**Target:** 0.9.0

---

## N — Native Codegen

`src/generation.rs` already translates the loft IR tree into Rust source files
(`tests/generated/*.rs`), but none compile (~1500 errors).  The steps below fix
these incrementally.  Full design in [NATIVE.md](NATIVE.md).

**Target: 0.9.0** — the generator already exists; N items are incremental fixes that turn
broken generated output into correct compiled Rust.  Each fix is small and independent.
See the 0.9.0 milestone in [PLANNING.md](PLANNING.md#version-090) for rationale.

---

### N2  Fix `output_init` to register all intermediate types
**Description:** `output_init` skips intermediate types (vectors inside structs,
plain enum values, byte/short field types), causing type ID misalignment at runtime.
**Effort:** Medium (generation.rs `output_init`)
**Fixes:** `enums_types`, `enums_enum_field` (2 runtime failures)
**Detail:** [NATIVE.md](NATIVE.md) § N10a

---

### N3  Fix `output_set` for DbRef deep copy
**Description:** `Set(var_b, Var(var_a))` for reference types emits a pointer copy.
Add `OpCopyRecord` call after assignment when both sides are same-type references.
**Effort:** Small (generation.rs `output_set`)
**Fixes:** `objects_independent_strings` (1 runtime failure)
**Detail:** [NATIVE.md](NATIVE.md) § N10b

---

### N4  Fix `OpFormatDatabase` for struct-enum variants
**Description:** Formatting outputs only the enum name, not the full struct fields.
Verify `db_tp` argument is the parent enum type so `ShowDb` can dispatch to variant.
**Effort:** Small (codegen_runtime.rs or generation.rs)
**Fixes:** `enums_define_enum`, `enums_general_json` (2 runtime failures)
**Detail:** [NATIVE.md](NATIVE.md) § N10c

---

### N5  Fix null DbRef handling in vector operations
**Description:** Guard `clear_vector` calls with a null check (`rec != 0`) in
generated code.  `stores.null()` returns a DbRef with a valid `store_nr` that
causes panics when passed to vector operations.
**Effort:** Small (generation.rs `output_call` for `OpClearVector`)
**Fixes:** `vectors_fill_result` (1 runtime failure)
**Detail:** [NATIVE.md](NATIVE.md) § N10d

---

---

### N6  Implement `OpIterate`/`OpStep` in codegen_runtime
**Description:** Add iterate/step state machine for sorted/index/vector collections.
Handle `Value::Iter` in `output_code_inner` by emitting a loop with these functions.
**Fix path:**
- **Phase 1 — vector iteration** (`codegen_runtime.rs`, `generation.rs`):
  Implement `OpIterate`/`OpStep` for `vector<T>`.  Emit an index-based loop: `_iter`
  holds the current index as `i64`; `OpStep` increments and checks bounds.  Test: for-loop
  over a vector literal produces correct values in native-codegen mode.
- **Phase 2 — sorted + index iteration** (`codegen_runtime.rs`):
  Extend `OpIterate`/`OpStep` to `sorted<T>` and `index<K,V>`.  Use the existing
  `iterate()`/`step()` interpreter helpers as the model.  Test: for-loop over a populated
  `sorted` and an `index` each produce all entries in order.
- **Phase 3 — reverse iteration + range sub-expressions** (`generation.rs`):
  Support `for x in vec.reversed()` and `for x in vec[a..b]` by recognising the
  sub-expression shape in `output_code_inner` and emitting appropriate start/end/step
  values.  Test: reversed vector and slice loops produce correct sequences.

Full detail in [NATIVE.md](NATIVE.md) § N10e-2.
**Effort:** High (codegen_runtime.rs + generation.rs)
**Fixes:** 3 compile failures (iterator tests)

---

### N7  Add `OpFormatFloat`/`OpFormatStackLong` handlers
**Description:** Add `output_call` special cases that emit direct calls to
`ops::format_float` / `ops::format_long` with the correct `&mut String` argument.
**Effort:** Small (generation.rs `output_call`)
**Fixes:** 2 compile failures
**Detail:** [NATIVE.md](NATIVE.md) § N10e-3

---

---

### N8  Fix empty pre-eval and prefix issues
**Description:** Skip pre-eval bindings when expression is empty; change `_pre{n}`
naming to `_pre_{n}` to avoid Rust prefix parsing; fix `OpGetRecord` argument count.
**Effort:** Small (generation.rs)
**Fixes:** 3 compile failures
**Detail:** [NATIVE.md](NATIVE.md) § N10e-5

---

### N1  Add `--native` CLI flag
**Description:** Add `--native <file.loft>` to `src/main.rs`: parse, generate Rust
source via `Output::output_native()`, compile with `rustc`, run the binary.
**Effort:** Medium
**Depends on:** N2–N8

---

### N9  Repair fill.rs auto-generation
**Description:** Make `create.rs::generate_code()` produce a `fill.rs` that can
replace the hand-maintained `src/fill.rs`. Add `ops` import, fix formatting,
add CI check for drift, and introduce `#state_call` annotation for the 52
delegation operators. Eliminates manual maintenance when adding new opcodes.
**Effort:** Medium (create.rs + default/*.loft + CI)
**Detail:** [NATIVE.md](NATIVE.md) § N20

---

## R — Repository

Standalone `loft` repository created (2026-03-16).  The remaining R item is the
workspace split needed before starting the Web IDE.

---

### R1  Workspace split (pre-W1 only — defer until IDE work begins)
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
**Depends on:** repo creation (done); gates W1

---

## W — Web IDE

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

See [ROADMAP.md](ROADMAP.md) — items in implementation order, grouped by milestone.

---

## See also
- [ROADMAP.md](ROADMAP.md) — All items in implementation order, grouped by milestone
- [../../CHANGELOG.md](../../CHANGELOG.md) — Completed work history (all fixed bugs and shipped features)
- [PROBLEMS.md](PROBLEMS.md) — Known bugs and workarounds
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design asymmetries and surprises
- [ASSIGNMENT.md](ASSIGNMENT.md) — Stack slot assignment status (A6 detail)
- [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) — External library packaging design (A7 Phase 2)
- [BYTECODE_CACHE.md](BYTECODE_CACHE.md) — Bytecode cache design (P4)
- [../DEVELOPERS.md](../DEVELOPERS.md) — Feature proposal process, quality gates, scope rules, and backwards compatibility
- [THREADING.md](THREADING.md) — Parallel for-loop design (A1 detail)
- [LOGGER.md](LOGGER.md) — Logger design (A2 detail)
- [FORMATTER.md](FORMATTER.md) — Code formatter design (backlog item)
- [NATIVE.md](NATIVE.md) — Native Rust code generation: root cause analysis, step details, verification (Tier N detail)
- [WEB_IDE.md](WEB_IDE.md) — Web IDE full design: architecture, JS API contract, per-milestone deliverables and tests, export ZIP layout (Tier W detail)
- [RELEASE.md](RELEASE.md) — 1.0 gate items, project structure changes, release artifacts checklist, post-1.0 versioning policy
