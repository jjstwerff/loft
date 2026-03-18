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
- [H — HTTP / Web Services](#h--http--web-services)
- [R — Repository](#r--repository)
- [W — Web IDE](#w--web-ide)
- [Quick Reference](#quick-reference) → [ROADMAP.md](ROADMAP.md)

---

## Version Milestones

### Version 0.8.2 — Stability, efficiency, and native codegen (planned)

Goal: harden the interpreter, improve runtime efficiency, and ship working native code
generation.  No new language syntax.  Most items are independent and can be developed
in parallel.

**Interpreter correctness:**
- **A9** — Vector slice copy-on-write: mutating a slice must not corrupt the parent vector.
- **A6** — Stack slot `assign_slots` pre-pass: compile-time slot layout replaces the
  current runtime `claim()` calls, eliminating the remaining category of slot-conflict bugs.

**Efficiency and packaging:**
- **A8** — Destination-passing for string natives: eliminates the double-copy overhead on
  `replace`, `to_lowercase`, `to_uppercase` and format expressions.
- **A3** — Optional Cargo features: gate `png`, `parallel`, `logging`, `mmap` behind `cfg`
  features for a lean default binary; remove dead `rand_core`/`rand_pcg` dependencies.

**Native code generation (Tier N):**
- Fix the ~1500 compile errors in `src/generation.rs` incrementally (N2–N9) so that
  `loft --native` produces correct compiled Rust output.  Each N step is small and
  independent; they can run in parallel with other 0.8.2 work.  N1 (`--native` CLI flag)
  lands last, after all N2–N9 fixes pass.

---

### Version 0.8.3 — Language syntax extensions (planned)

Goal: add all new language syntax before the feature-complete 0.9.0 milestone so that
syntax decisions can be validated and refined independently.  All items change the parser
or type system; 0.8.2 correctness work is a prerequisite.

**Lambda expressions (P1):**
- **P1.1** — Parser: recognise `fn(params) -> type block` as a primary expression.
- **P1.2** — Compilation: synthesise an anonymous `def`, emit a def-number at the call site.
- **P1.3** — Integration: `map`, `filter`, `reduce` accept inline lambdas.
- **P3** — Vector aggregates: `sum`, `min_of`, `max_of`, `any`, `all`, `count_if` (depends on P1).

**Pattern extensions (L2):**
- **L2** — Nested match patterns: field sub-patterns separated by `:` in struct arms.

**Field iteration (A10):**
- **A10.1** — `Field` + `FieldValue` enum types in `default/01_code.loft`.
- **A10.2** — `fields()` special form → `Value::FieldsOf` + `Type::FieldsOf`.
- **A10.3** — Loop unrolling in `parse_for` for `Type::FieldsOf` (compile-time expansion).
- **A10.4** — Error messages, docs, and test coverage.

---

### Version 0.8.4 — HTTP client and JSON (planned)

Goal: add blocking HTTP client access and automatic JSON mapping so loft programs can
consume web services.  Builds on P1 lambdas (0.8.3): `Type.from_json` is a callable
fn-ref that composes naturally with `map` and `filter`.  All items gated behind a new
`http` Cargo feature so binaries that don't need networking stay lean.

**JSON struct annotation (H1):**
- **H1** — Parse `#json` before struct declarations; synthesise `to_json(self) -> text`
  reusing the existing `:j` format flag.  No new runtime dependency.

**JSON primitive stdlib (H2):**
- **H2** — Add `serde_json`-backed extraction functions: `json_text`, `json_int`,
  `json_long`, `json_float`, `json_bool`, `json_items`, `json_nested`.
  Declared in `default/04_web.loft`; implemented in new `src/native_http.rs`.

**JSON deserialization codegen — scalars (H3):**
- **H3** — For each `#json` struct with primitive fields only, synthesise
  `from_json(body: text) -> T` using the H2 primitives.  `Type.from_json` is now a
  valid fn-ref passable to `map`.

**HTTP client (H4):**
- **H4** — `HttpResponse` struct (`status: integer`, `body: text`, `ok()` method) and
  blocking HTTP functions (`http_get`, `http_post`, `http_put`, `http_delete`, plus
  `_h` variants accepting `vector<text>` headers) via `ureq`.

**Nested types and integration (H5):**
- **H5** — Extend `from_json` codegen to nested `#json` struct fields, `vector<T>` array
  fields, and plain enum fields.  Integration test suite against a mock HTTP server.

---

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

Goal: every planned language feature is present and the interpreter ships pre-built.
Interpreter correctness and native codegen are handled by 0.8.2; new syntax by 0.8.3;
HTTP and JSON by 0.8.4; this milestone completes runtime infrastructure and tooling.

**Language completeness:**
- **L1** — Error recovery: a single bad token must not cascade into dozens of spurious errors.
- **P2** — REPL / interactive mode: `loft` with no arguments enters a persistent session.

**Parallel execution completeness:**
- **A1** — Parallel workers with extra context arguments and text/reference return types.

**Logging completeness:**
- **A2** — Logger remaining work: hot-reload wiring, `is_production()`/`is_debug()`, `--release` assert elision, `--debug` per-type safety logging.

**Deferred from 0.9.0:**
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
A5, A7, Tier N (native codegen).

---

### Version 1.x — Minor releases (additive)

New language features that are strictly backward-compatible.  Candidates: A5 (closures),
A7 (native extensions), Tier N (native codegen).

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

**Why include native codegen (Tier N) in 0.8.2?**
`src/generation.rs` already translates the loft IR to Rust source; the code exists but
does not compile.  The N items are incremental bug fixes — each is Small or Medium effort,
independent of each other and of the other 0.8.2 items — they can be interleaved freely.
Fixing them in 0.8.2 means 0.9.0 ships a binary where `--native` actually works, at no
extra milestone cost.  Deferring them would mean shipping a 0.9.0 that silently generates
uncompilable output.

**Why include REPL (P2) in 0.9.0?**
The Web IDE covers the browser-based interactive use case, but a terminal REPL is
independently useful for development workflows where a browser is not available or
convenient.  P2 is self-contained (new `src/repl.rs`, small changes to `main.rs`)
and depends on L1 (error recovery) which is already in 0.9.0.  Including it rounds
out the "prototype-friendly" goal without affecting the IDE track.

**Why split syntax into 0.8.3?**
Lambda expressions, nested patterns, and field iteration all touch the parser and type
system simultaneously.  Grouping them in a dedicated milestone means syntax decisions can
be reviewed and refined in isolation, before runtime infrastructure work in 0.9.0 begins.
It also keeps each milestone small enough to be fully understood in a single pass.

**The small-steps principle in practice:**
Each milestone is a strict subset of the next.  0.8.2 hardens correctness; 0.8.3 adds new
syntax; 0.8.4 adds HTTP and JSON on top of lambdas; 0.9.0 completes runtime infrastructure
and tooling; 1.0.0 adds exactly R1 + W1–W6 on top of a complete 0.9.0.  No item moves
forward until the test suite for the previous item is green.  This prevents the "everything
at once" failure mode where half-finished features interact and regressions are hard to pin.

---

### Recommended Implementation Order

Ordered by unblocking impact and the small-steps principle (each item leaves the codebase
in a better state than it found it, with passing tests).

**For 0.8.2:**
1. **L3** — reserve keywords; Small, must be first (claims `fields` and `debug_assert` before 0.9.0 features land)
2. **A9** — vector slice CoW; Medium, independent correctness fix
3. **A6** — slot pre-pass; High, independent; can share a branch with A9
4. **A8** — destination-passing; Med–High, independent efficiency win
5. **A3** — optional Cargo features; Medium, packaging polish; independent
6. **N2–N9** — native codegen fixes; each is independent and Small–Medium; interleave freely with items 2–5
7. **N1** — `--native` CLI flag; lands after all N2–N9 fixes pass

**For 0.8.3 (after 0.8.2 is tagged):**
1. **P1** — lambdas; unblocks P3, A5; makes the language feel complete
2. **P3** + **L2** — aggregates and nested patterns; P3 depends on P1; batch together
3. **A10** — field iteration; independent, medium; can land in parallel with P1–P3

**For 0.8.4 (after 0.8.3 is tagged):**
1. **H1** — `#json` + `to_json`; Small, no new Rust deps; validates annotation parsing
2. **H2** — JSON primitive stdlib; Medium, adds `serde_json`; test each extractor in isolation
3. **H3** — `from_json` scalar codegen; Medium, depends on H1 + H2; verify `Type.from_json` as fn-ref
4. **H4** — HTTP client + `HttpResponse`; Medium, adds `ureq`; test against httpbin.org or mock
5. **H5** — nested/array/enum `from_json` + integration tests; Med–High, depends on H3 + H4

**For 0.9.0 (after 0.8.4 is tagged):**
1. **L1** — error recovery; standalone UX improvement, no dependencies; also unblocks P2.4
2. **A2** — logger remaining work; independent, small-medium; can land any time
3. **A1** — parallel completeness; isolated change, touches parallel.rs only
4. **P2** — REPL; high effort; land after L1 (needed for P2.4 error recovery)

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
**Target:** 0.8.3

---

### L3  Reserve compile-time intrinsic names as keywords
**Sources:** [PROBLEMS.md](PROBLEMS.md) #53
**Description:** Several names are special-cased in `parse_call` / `parse_single` but are
not in the `KEYWORDS` array, so user code can define functions or variables with the same
names.  The intrinsic always wins silently, making user definitions unreachable dead code.
Two upcoming features introduce new intrinsic names — `fields` (A10) and `debug_assert`
(A2.3) — that must be reserved *before* those features land or existing user code could
break silently.
**Fix path:**
1. Add to `KEYWORDS` in `src/lexer.rs`: `match` (added in 0.8.0 but never put in KEYWORDS),
   `sizeof`, `assert`, `panic`, `fields`, `debug_assert`.
2. Update COMPILER.md KEYWORDS list.
3. Add a parse-error test: `fn sizeof(...) { ... }` produces a single clear diagnostic.

Names intentionally left as identifiers: `log_info/warn/error/fatal` (prefixed, low
collision risk), `parallel_for` (highly specific), `rev` (likely future stdlib function).
**Effort:** Small (`src/lexer.rs` + test)
**Target:** 0.8.2

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
**Target:** 0.8.3

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
**Target:** 0.9.0

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
**Target:** 0.8.3 — batch all variants after P1 lands

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

### A2  Logger: hot-reload, run-mode helpers, release + debug flags
**Sources:** [LOGGER.md](LOGGER.md) § Remaining Work
**Description:** Four independent improvements to the logging system.  The core framework
(production mode, source-location injection, log file rotation, rate limiting) was shipped
in 0.8.0.  These are the remaining pieces.
**Fix path:**

**A2.1 — Wire hot-reload** (`src/native.rs`):
Call `lg.check_reload()` at the top of each `n_log_*`, `n_panic`, and `n_assert` body so
the config file is re-read at most every 5 s.  `check_reload()` is already implemented.
*Tests:* write a config file; change the level mid-run; verify subsequent calls respect the new level.

**A2.2 — `is_production()` and `is_debug()` helpers** (`src/native.rs`, `default/01_code.loft`):
Two new loft natives read `stores.run_mode`.  The `RunMode` enum replaces the current
`production: bool` flag on `RuntimeLogConfig` so all runtime checks share one source of truth.
*Tests:* a loft program calling `is_production()` returns `true` under `--production`/`--release`
and `false` otherwise; `is_debug()` returns `true` only under `--debug`.

**A2.3 — `--release` flag with zero-overhead assert elision** (`src/parser/control.rs`, `src/main.rs`):
`--release` implies `--production` AND strips `assert()` and `debug_assert()` from bytecode
at parse time (replaced by `Value::Null`).  Adds `debug_assert(test, message)` as a
companion to `assert()` that is also elided in release mode.
*Tests:* a `--release` run skips assert; `--release` + failed assert does not log or panic.

**A2.4 — `--debug` flag with per-type runtime safety logging** (`src/fill.rs`, `src/native.rs`):
When `stores.run_mode == Debug`, emit `warn` log entries for silent-null conditions:
integer/long overflow, shift out-of-range, null field dereference, vector OOB.
*Tests:* a deliberate overflow under `--debug` produces a `WARN` entry at the correct file:line.

**Effort:** Medium (logger.rs, native.rs, fill.rs; see LOGGER.md for full design)
**Target:** 0.9.0

---

### A3  Optional Cargo features
**Sources:** OPTIONAL_FEATURES.md
**Description:** Gate subsystems behind `cfg` features: `png` (image support), `gendoc`
(HTML documentation generation), `parallel` (threading), `logging` (logger), `mmap`
(memory-mapped storage).  Remove `rand_core` / `rand_pcg` dead dependencies.
**Effort:** Medium (Cargo.toml, conditional compilation in store.rs, native.rs, main.rs)
**Target:** 0.8.2

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
**Target:** 1.1+

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
**Target:** 0.8.2

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
**Target:** 0.8.2

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
**Target:** 0.8.2

---

### A10  Field iteration — `for f in fields(s)`
**Sources:** Design evaluation 2026-03-18
**Description:** Allow iterating over the stored primitive fields of a struct value with
`for f in fields(s)`.  The loop variable `f` has type `Field` (defined in
`default/01_code.loft`) with `f.name: text` (the compile-time field name) and
`f.value: FieldValue` (a struct-enum covering all primitive types).  Native type capture
uses existing `match f.value { Float{v} => ... }` pattern syntax.

The loop is a compile-time unroll: the parser expands `for f in fields(s)` into one
sequential block per eligible field.  No runtime allocation is needed.  Fields whose
type is a reference, collection, or nested struct are skipped in this version.

```loft
struct Config { host: text, port: integer not null, debug: boolean }
c = Config{ host: "localhost", port: 8080, debug: true };

for f in fields(c) {
    match f.value {
        Text { v } => log_info("{f.name} = '{v}'")
        Int  { v } => log_info("{f.name} = {v}")
        Bool { v } => log_info("{f.name} = {v}")
        _          => {}
    }
}
```

**Fix path:**

**Phase A10.1 — `Field` and `FieldValue` types** (`default/01_code.loft`):
Define the two public types that form the loop variable contract.  No compiler changes in
this phase.

```loft
pub enum FieldValue {
    Bool   { v: boolean },
    Int    { v: integer },
    Long   { v: long },
    Float  { v: float },
    Single { v: single },
    Char   { v: character },
    Text   { v: text },
    Enum   { name: text not null, ordinal: integer not null },
}

pub struct Field {
    name:  text not null,
    value: FieldValue,
}
```

`Enum` carries both the variant name (for display) and the ordinal (for comparison).
Reference, collection, and nested-struct fields are excluded from `FieldValue`; the
compiler will skip those field types silently in Phase A10.3.
*Tests:* `Field` and `FieldValue` are usable in normal loft code; a hand-constructed
`Field{name: "x", value: FieldValue::Float{v: 1.0}}` round-trips through a match arm.

**Phase A10.2 — `fields()` parse-time special form** (`src/parser/control.rs`,
`src/data.rs`):
In `parse_call`, detect `fields(expr)` where `expr` has a struct type (resolved on the
second pass via `typedef::actual_type`).  Validate: non-struct arguments produce a clear
compile error (`fields() requires a struct value, got <type>`).  Return a new IR node
`Value::FieldsOf(struct_def_nr, Box<source_expr>)` with type `Type::FieldsOf(struct_def_nr)`.
On the first pass (types not yet resolved), return a placeholder and defer validation.

```
// data.rs — add to Value enum
FieldsOf(u32, Box<Value>),   // (struct def_nr, source expression)

// data.rs — add to Type enum
FieldsOf(u32),               // struct def_nr; erased after loop unrolling
```

*Tests:* `fields(my_point)` on a known struct type-checks without error; `fields(42)` and
`fields(my_vector)` each produce one diagnostic naming the offending type.

**Phase A10.3 — Loop unrolling** (`src/parser/collections.rs`):
In `parse_for` (or the `parse_in_range` helper that determines iterator type), detect
`Type::FieldsOf(struct_def_nr)` and take the unrolling path instead of the normal
`v_loop` path.

Algorithm:
1. Declare loop variable `f` with type `Field` in the current variable scope.
2. Parse the loop body once (first pass: types still unknown; second pass: body typed
   against `Field`).
3. For each field in `data.structs[struct_def_nr].fields` in declaration order:
   a. Determine the `FieldValue` variant for the field's type:
      - `boolean` → `Bool`, `integer` (all limit variants) → `Int`, `long` → `Long`,
        `float` → `Float`, `single` → `Single`, `character` → `Char`,
        `text` → `Text`, plain enum → `Enum`
      - reference / collection / nested struct → **skip this field**
   b. Build the Field constructor IR:
      ```
      Value::Call(field_ctor_nr, [
          Value::Str(field_name),                         // f.name
          Value::Call(fv_variant_ctor_nr, [               // f.value
              <source_expr>.field_name,                   // actual field read
          ]),
      ])
      ```
      For plain enum fields the variant is `Enum{ name: format_enum(s.variant), ordinal: s.variant as integer }`.
   c. Emit `v_block([v_set(f_var, field_constructor), body_copy])`.
4. Wrap all N blocks in a single `v_block`.  The result replaces the normal loop IR.

`break` and `continue` inside a `for f in fields(s)` body are a compile error in this
version (emit: `break/continue not supported in field loops`).

*Tests:*
- Iterate over `struct Point { x: float not null, y: float not null, z: float not null }`:
  verify three iterations; `f.name` values are `"x"`, `"y"`, `"z"`; `f.value` matches
  `Float{v}` with the correct values.
- Iterate over a mixed-type struct (`integer`, `text`, `boolean`, `float` fields): all four
  `FieldValue` variants are matched correctly in the same loop body.
- Null field value: a nullable text field holding `null` produces `Text{v: null}`; the match
  arm `Text{v}` binds `v = null`.
- Plain enum field: produces `Enum{name: "Red", ordinal: 0}` for a `Color::Red` value.
- Struct with a reference field and a vector field: those fields are skipped; only the
  primitive fields are visited.
- `break` inside the body: compile error with message naming the field loop restriction.
- `fields(42)`: single diagnostic, no crash.

**Phase A10.4 — Error messages and documentation** (`src/parser/control.rs`,
`doc/claude/LOFT.md`, `doc/claude/STDLIB.md`):
Polish pass: verify error messages are clear and point to the right source location.
Add `fields()` to LOFT.md § Control flow (alongside `for`) and to STDLIB.md § Structs.
Document the skipped-field limitation and note the future `A10+` path for non-primitive
fields.
*Tests:* `fields(ref_val)` (reference type, not the struct it points to) gives a clear
error distinguishing "you have a reference; dereference it first with `.field` access or
pass a struct value" from the generic type-mismatch message.

**Files changed:**

| File | Change |
|---|---|
| `default/01_code.loft` | Add `FieldValue` (struct-enum, 8 variants) and `Field` (struct) |
| `src/data.rs` | Add `Value::FieldsOf(u32, Box<Value>)` and `Type::FieldsOf(u32)` |
| `src/parser/control.rs` | Special-case `fields(expr)` in `parse_call`; type-check on second pass |
| `src/parser/collections.rs` | Detect `Type::FieldsOf` in `parse_for`; build unrolled block IR |
| `src/typedef.rs` | Erase `Type::FieldsOf` after unrolling (it should not appear in bytecode) |
| `tests/docs/21-field-iter.loft` | New — test coverage |
| `tests/wrap.rs` | Add `field_iteration()` test |
| `doc/claude/LOFT.md` | Document `for f in fields(s)` in the For-loop section |
| `doc/claude/STDLIB.md` | Add `fields()` to the Structs section |

**Limitations (initial version):**
- Only primitive-typed fields are visited; reference, collection, and nested-struct fields
  are silently skipped.
- `break` and `continue` are not supported inside the loop body.
- `fields(s)` is only valid as the source expression of a `for` loop, not as a standalone
  expression producing a `vector<Field>`.  (A follow-on can implement that using an
  actual runtime vector construction via the same boxing logic.)
- `virtual` fields are included (they are read-only computed values, still primitive).

**Effort:** Medium (data.rs + 2 parser files + default library; no bytecode changes)
**Target:** 0.8.3

---

### A11  Fix hash table load-factor threshold
**Sources:** Code review 2026-03-18
**Severity:** Low — current threshold triggers rehash at ~57% load; intended threshold is
75% (standard for open-addressing with linear probing); DEVELOPERS.md comment incorrectly
stated 87.5%
**Description:** `src/hash.rs:25` contains `(length * 14 / 16) + 1 >= room`.  Because
`elms = (room - 1) * 2`, the effective load fraction at rehash is not `14/16 = 87.5%`
but `(14/16) × (1/2) ≈ 4/7 ≈ 57%`.  The fix is `(length * 2 / 3) + 1 >= room`, which
gives the standard 75% threshold at all table sizes.  Update DEVELOPERS.md accordingly.
**Fix path:**
1. `src/hash.rs:25`: `(length * 14 / 16) + 1` → `(length * 2 / 3) + 1`
2. `doc/DEVELOPERS.md`: update load-factor description from "87.5%" to "75%"
**Effort:** Trivial (one arithmetic expression + one doc line)
**Target:** 0.8.2

---

## N — Native Codegen

`src/generation.rs` already translates the loft IR tree into Rust source files
(`tests/generated/*.rs`), but none compile (~1500 errors).  The steps below fix
these incrementally.  Full design in [NATIVE.md](NATIVE.md).

**Target: 0.8.2** — the generator already exists; N items are incremental fixes that turn
broken generated output into correct compiled Rust.  Each fix is small and independent.
See the 0.8.2 milestone in [PLANNING.md](PLANNING.md#version-082) for rationale.

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

## H — HTTP / Web Services

Full design rationale and approach comparison: [WEB_SERVICES.md](WEB_SERVICES.md).

The `#json` annotation is the key enabler: it synthesises `to_json` and `from_json` for a
struct, making `Type.from_json` a first-class callable fn-ref that composes with `map` and
`filter`.  The HTTP client is a thin blocking wrapper (via `ureq`) returning a plain
`HttpResponse` struct — no thread-local state, parallel-safe.  All web functionality is
gated behind an `http` Cargo feature.

---

### H1  `#json` annotation — parser and `to_json` synthesis
**Sources:** [WEB_SERVICES.md](WEB_SERVICES.md) § Approach B, Phase 1
**Description:** Extend the annotation parser to accept `#json` (no value) before a struct
declaration.  For every annotated struct, the compiler synthesises a `to_json` method that
reuses the existing `:j` JSON format flag.  No new Rust dependencies are needed.
**Fix path:**

**Step 1 — Parser** (`src/parser/parser.rs` or `src/parser/expressions.rs`):
Extend the annotation-parsing path that currently handles `#rust "..."` to also accept
bare `#json`.  Store a `json: bool` flag on the struct definition node (parallel to how
`#rust` stores its string).  Emit a clear parse error if `#json` is placed on anything
other than a struct.
*Test:* `#json` before a struct compiles without error; `#json` before a `fn` produces a
single clear diagnostic.

**Step 2 — Synthesis** (`src/state/typedef.rs`):
During type registration, for each struct with `json: true`, synthesise an implicit `pub fn`
definition equivalent to:
```loft
pub fn to_json(self: T) -> text { "{self:j}" }
```
The synthesised def shares the struct's source location for error messages.
*Test:* `"{user:j}"` and `user.to_json()` produce identical output for a `#json` struct.

**Step 3 — Error for missing annotation** (`src/state/typedef.rs`):
If `to_json` is called on a struct without `#json`, emit a compile error:
`"to_json requires #json annotation on struct T"`.
*Test:* Unannotated struct calling `.to_json()` produces a single clear diagnostic.

**Effort:** Small (parser annotation extension + typedef synthesiser)
**Target:** 0.8.4
**Depends on:** —

---

### H2  JSON primitive extraction stdlib
**Sources:** [WEB_SERVICES.md](WEB_SERVICES.md) § Approach B
**Description:** Add a new stdlib module `default/04_web.loft` with JSON field-extraction
functions backed by `serde_json`.  Functions extract a single typed value from a JSON
object body supplied as a `text` string.
**Fix path:**

**Step 1 — Cargo dependency** (`Cargo.toml`):
Add `serde_json = "1"` (and `ureq` placeholder, used in H4) under a new `http` optional
feature.  The feature is not enabled by default:
```toml
[features]
http = ["serde_json", "ureq"]

[dependencies]
serde_json = { version = "1", optional = true }
ureq       = { version = "2", optional = true }
```

**Step 2 — Loft declarations** (`default/04_web.loft`):
```loft
// Extract primitive values from a JSON object body.
// Returns zero/empty if the key is absent or the type does not match.
pub fn json_text(body: text, key: text) -> text;
pub fn json_int(body: text, key: text) -> integer;
pub fn json_long(body: text, key: text) -> long;
pub fn json_float(body: text, key: text) -> float;
pub fn json_bool(body: text, key: text) -> boolean;

// Split a JSON array body into element bodies (each element as raw JSON text).
pub fn json_items(array_body: text) -> vector<text>;

// Extract a named field as raw JSON text (object, array, or primitive).
// Use for nested structs and array fields: json_nested(body, "field").
pub fn json_nested(body: text, key: text) -> text;
```

**Step 3 — Rust implementation** (new `src/native_http.rs`, registered in `src/native.rs`):
Implement each function using `serde_json::from_str` to parse the body, then navigate to
the key.  All functions return the zero value on any error (missing key, type mismatch,
invalid JSON) — never panic.
- `json_text`: `value.get(key)?.as_str()? .to_owned()`
- `json_int`: `value.get(key)?.as_i64()? as i32`
- `json_long`: `value.get(key)?.as_i64()?`
- `json_float`: `value.get(key)?.as_f64()? as f32`
- `json_bool`: `value.get(key)?.as_bool()?`
- `json_items`: parse as array, `serde_json::to_string` each element
- `json_nested`: `serde_json::to_string(value.get(key)?)`

**Step 4 — Feature gate** (`src/native.rs` or `src/main.rs`):
Register the H2 natives only when compiled with `--features http`.  Without the feature,
calling any `json_*` function raises a compile-time error:
`"json_text requires the 'http' Cargo feature"`.

*Tests:*
- Valid JSON object: each extractor returns the correct value.
- Missing key: returns zero/empty without panic.
- Invalid JSON body: returns zero/empty without panic.
- `json_items` on a 3-element array returns a `vector<text>` of length 3.
- `json_nested` on a nested object returns parseable JSON text.

**Effort:** Medium (`serde_json` integration + 7 native functions)
**Target:** 0.8.4
**Depends on:** H1 (for the `http` feature gate pattern)

---

### H3  `from_json` codegen — scalar struct fields
**Sources:** [WEB_SERVICES.md](WEB_SERVICES.md) § Approach B, Phase 2
**Description:** For each `#json`-annotated struct whose fields are all primitive types
(`text`, `integer`, `long`, `float`, `single`, `boolean`, `character`), the compiler
synthesises a `from_json(body: text) -> T` function.  The result is a normal callable
fn-ref: `User.from_json` can be passed to `map` without any special syntax.
**Fix path:**

**Step 1 — Synthesis** (`src/state/typedef.rs`):
After H2 is in place, extend the `#json` synthesis pass (H1 Step 2) to also emit
`from_json`.  For each field, select the extractor by type:

| Loft type | Extractor call |
|-----------|---------------|
| `text` | `json_text(body, "field_name")` |
| `integer` | `json_int(body, "field_name")` |
| `long` | `json_long(body, "field_name")` |
| `float` / `single` | `json_float(body, "field_name")` |
| `boolean` | `json_bool(body, "field_name")` |
| `character` | first char of `json_text(body, "field_name")` |

The synthesised `from_json` body is a struct-literal expression using the above calls.
Fields not in the table (nested structs, enums, vectors) are silently skipped in this
phase (H5 adds them).

**Step 2 — fn-ref validation** (`src/state/compile.rs` or `src/state/codegen.rs`):
Verify that `Type.from_json` resolves as a callable fn-ref with type
`fn(text) -> Type`, so it can be passed directly to `json_items(...).map(...)` and
`json_items(...).filter(...)`.

*Tests:*
- `User.from_json(body)` returns a struct with all fields set from JSON.
- `json_items(resp.body).map(User.from_json)` returns a `vector<User>`.
- Absent JSON key sets the field to its zero value (0, "", false).
- Struct with a nested `#json` struct field compiles without error (nested field gets zero value until H5).

**Effort:** Medium (typedef synthesiser + fn-ref type check)
**Target:** 0.8.4
**Depends on:** H1, H2

---

### H4  HTTP client stdlib and `HttpResponse`
**Sources:** [WEB_SERVICES.md](WEB_SERVICES.md) § Approach B, stdlib additions; PROBLEMS #55
**Description:** Add blocking HTTP functions to `default/04_web.loft` backed by `ureq`.
All functions return `HttpResponse` — a plain struct — so there is no thread-local status
state and the API is parallel-safe (see PROBLEMS #55).
**Fix path:**

**Step 1 — `HttpResponse` struct** (`default/04_web.loft`):
```loft
pub struct HttpResponse {
    status: integer
    body:   text
}

pub fn ok(self: HttpResponse) -> boolean {
    self.status >= 200 and self.status < 300
}
```
No `#rust` needed; `ok()` is a plain loft method.

**Step 2 — HTTP functions declaration** (`default/04_web.loft`):
```loft
// Body-less requests
pub fn http_get(url: text) -> HttpResponse;
pub fn http_delete(url: text) -> HttpResponse;

// Body requests (body is a text string, typically to_json() output)
pub fn http_post(url: text, body: text) -> HttpResponse;
pub fn http_put(url: text, body: text) -> HttpResponse;
pub fn http_patch(url: text, body: text) -> HttpResponse;

// With explicit headers (each entry: "Name: Value")
pub fn http_get_h(url: text, headers: vector<text>) -> HttpResponse;
pub fn http_post_h(url: text, body: text, headers: vector<text>) -> HttpResponse;
pub fn http_put_h(url: text, body: text, headers: vector<text>) -> HttpResponse;
```

**Step 3 — Rust implementation** (`src/native_http.rs`):
Use `ureq::get(url).call()` / `.send_string(body)`.  Parse each `"Name: Value"` header
entry by splitting at the first `:`.  On network error, connection refused, or timeout,
return `HttpResponse { status: 0, body: "" }` — never panic.  Set a default timeout of
30 seconds.
```rust
fn http_get(url: &str) -> HttpResponse {
    match ureq::get(url).call() {
        Ok(resp) => HttpResponse {
            status: resp.status() as i32,
            body:   resp.into_string().unwrap_or_default(),
        },
        Err(_) => HttpResponse { status: 0, body: String::new() },
    }
}
```

**Step 4 — Content-Type default**:
`http_post` and `http_put` set `Content-Type: application/json` automatically when the
body is non-empty (the common case).  Callers who need a different content type use the
`_h` variants to supply their own `Content-Type` header.

*Tests (run with a local mock server or httpbin.org):*
- `http_get("https://httpbin.org/get").ok()` is `true`.
- `http_get("https://httpbin.org/status/404").status` is `404`.
- `http_post` with a JSON body returns the echoed body from `/post`.
- Network failure (bad URL) returns `HttpResponse { status: 0, body: "" }`.
- Header variants set the supplied headers (verify via httpbin.org `/headers`).

**Effort:** Medium (`ureq` integration + 8 native functions)
**Target:** 0.8.4
**Depends on:** H2 (for the `http` Cargo feature; `ureq` added there)

---

### H5  Nested/array/enum `from_json` and integration tests
**Sources:** [WEB_SERVICES.md](WEB_SERVICES.md) § Approach B, Phases 3–4
**Description:** Extend the H3 `from_json` synthesiser to handle nested `#json` structs,
`vector<T>` array fields, and plain enum fields.  Add an integration test suite that calls
real HTTP endpoints and verifies the full round-trip.
**Fix path:**

**Step 1 — Nested `#json` struct fields** (`src/state/typedef.rs`):
For a field `addr: Address` where `Address` is `#json`-annotated, emit:
```loft
addr: Address.from_json(json_nested(body, "addr"))
```
The compiler must verify that `Address` is `#json` at the point of synthesis; if not,
emit: `"field 'addr' has type Address which is not annotated with #json"`.

**Step 2 — `vector<T>` array fields** (`src/state/typedef.rs`):
For a field `items: vector<Item>` where `Item` is `#json`, emit:
```loft
items: json_items(json_nested(body, "items")).map(Item.from_json)
```
This relies on `map` with fn-refs, which already works.  If `Item` is not `#json`, emit
a compile error.

**Step 3 — Plain enum fields** (`src/state/typedef.rs`):
For a field `status: Status` where `Status` is a plain (non-struct) enum, emit a `match`
on the string value:
```loft
status: match json_text(body, "status") {
    "Active"   => Status::Active,
    "Inactive" => Status::Inactive,
    _          => Status::Active,   // first variant as default
}
```
The default fallback uses the first variant; a compile-time warning notes it.
Struct-enum variants in JSON (e.g. `{"type": "Paid", "amount": 42}`) are not supported
in this phase — a compile error is emitted if a struct-enum field appears in a `#json` struct.

**Step 4 — `not null` field validation** (`src/state/typedef.rs`):
Fields declared `not null` whose JSON key is absent should emit a runtime warning (via the
logger) and keep the zero value rather than panicking.  This matches loft's general approach
of never crashing on bad data.

**Step 5 — Integration test suite** (`tests/web/`):
Write loft programs that call public stable APIs and assert on the response.  Tests should
be skipped if the `http` feature is not compiled in or if the network is unavailable:
- `GET https://httpbin.org/json` → parse known struct, assert fields.
- `POST https://httpbin.org/post` with JSON body → assert echoed body round-trips.
- `GET https://httpbin.org/status/500` → `resp.ok()` is `false`, `resp.status` is `500`.
- Nested struct: `GET https://httpbin.org/json` contains a nested `slideshow` object.
- Array field: `GET https://httpbin.org/json` contains a `slides` array.

**Effort:** Medium–High (3 codegen extensions + integration test infrastructure)
**Target:** 0.8.4
**Depends on:** H3, H4

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
**Target:** 1.0.0

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
**Depends on:** R1
**Target:** 1.0.0

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
**Target:** 1.0.0

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
**Target:** 1.0.0

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
**Target:** 1.0.0

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
**Target:** 1.0.0

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
**Target:** 1.0.0

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
