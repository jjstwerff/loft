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

Sources: [PROBLEMS.md](PROBLEMS.md) · [INCONSISTENCIES.md](INCONSISTENCIES.md) · [ASSIGNMENT.md](ASSIGNMENT.md) · [SLOTS.md](SLOTS.md) · [THREADING.md](THREADING.md) · [LOGGER.md](LOGGER.md) · [WEB_IDE.md](WEB_IDE.md) · [RELEASE.md](RELEASE.md) · [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) · [BYTECODE_CACHE.md](BYTECODE_CACHE.md) · [PERFORMANCE.md](PERFORMANCE.md) · [TUPLES.md](TUPLES.md) · [STACKTRACE.md](STACKTRACE.md) · [COROUTINE.md](COROUTINE.md)

---

## Contents
- [Version Milestones](#version-milestones)
  - [Milestone Reevaluation](#milestone-reevaluation)
  - [Recommended Implementation Order](#recommended-implementation-order)
- [L — Language Quality](#l--language-quality)
  - [L4 — Fix empty `[]` literal as mutable vector argument](#l4--fix-empty--literal-as-mutable-vector-argument)
  - [L5 — Fix `v += extra` via `&vector` ref-param](#l5--fix-v--extra-via-vector-ref-param)
  - [L6 — Prevent double evaluation of `expr ?? default`](#l6--prevent-double-evaluation-of-expr--default)
- [S — Stability Hardening](#s--stability-hardening)
  - [S4 — Binary I/O type coverage (Issue 59, 63)](#s4--binary-io-type-coverage)
  - [S5 — Optional `& text` panic](#s5--fix-optional--text-parameter-subtract-with-overflow-panic) *(0.8.2)*
  - [S6 — `for` loop in recursive function](#s6--fix-for-loop-in-recursive-function----too-few-parameters-panic) *(1.1+)*
- [P — Prototype Features](#p--prototype-features)
  - [T1 — Tuple types](#t1--tuple-types) *(1.1+)*
  - [CO1 — Coroutines](#co1--coroutines) *(1.1+)*
- [A — Architecture](#a--architecture)
  - [A1 — Parallel workers: extra args + value-struct + text/ref returns](#a1--parallel-workers-extra-arguments-value-struct-returns-and-textreference-returns) *(completed 0.8.3)*
  - [A12 — Lazy work-variable initialization](#a12--lazy-work-variable-initialization) *(deferred to 1.1+)*
  - [A13 — Complete two-zone slot assignment](#a13--complete-two-zone-slot-assignment-steps-8-and-10) *(completed 0.8.3)*
  - [TR1 — Stack trace introspection](#tr1--stack-trace-introspection) *(0.9.0)*
- [N — Native Codegen](#n--native-codegen)
- [O — Performance Optimisations](#o--performance-optimisations)
  - [O1–O7 — Interpreter and native performance](#o1--superinstruction-merging) *(O1 deferred indefinitely — opcode table full; O2–O7 deferred to 1.1+)*
- [H — HTTP / Web Services](#h--http--web-services)
- [R — Repository](#r--repository)
- [W — Web IDE](#w--web-ide)
- [Quick Reference](#quick-reference) → [ROADMAP.md](ROADMAP.md)

---

## Version Milestones

### Version 0.8.2 — Stability, native codegen, and slot correctness (in progress)

Goal: harden the interpreter, complete native code generation, fix slot assignment, and
improve runtime efficiency.  No new language syntax.  Most items are independent and can be developed
in parallel.

**Remaining for 0.8.2:** *(none — all items completed or deferred)*

**Deferred from 0.8.2 (too complex / disruptive for stability):**
- **O1** — Superinstruction peephole rewriting pass — deferred indefinitely (opcode table is full: 254/256 used; adding superinstructions would require an opcode-space redesign).
- **A12** — Lazy work-variable initialization — deferred to 1.1+ (also blocked by Issues 68–70).

---

### Version 0.8.3 — Language syntax extensions *(completed)*

All items completed.  See CHANGELOG.md for details.

- **P1** — Lambda expressions ✓ (completed in 0.8.2)
- **P3** — Vector aggregates: `sum_of`, `min_of`, `max_of`, `any`, `all`, `count_if` ✓
- **P5** — Generic functions: parse, instantiate, validate, test+docs ✓
- **L2** — Nested match patterns: field sub-patterns in struct arms ✓
- **A10** — Field iteration (`for f in s#fields`): all 5 phases ✓
- **T1.1–T1.7** — Tuple types: type system, parser, scope, codegen, ref params, mutation guard, `not null` ✓
- **CO1** — Coroutines: opcodes, yield, text serialisation, `for` integration, `yield from` ✓
- **TR1** — Stack trace: shadow call-frame, type declarations, materialisation, line numbers ✓
- **A5** — Closures: capture analysis, record layout, call-site allocation, body reads, lifetime ✓

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
- **A1** — Moved to 0.8.2 (see remaining work above).

**Logging completeness:**
- **A2** — Logger remaining work: hot-reload wiring, `is_production()`/`is_debug()`, `--release` assert elision, `--debug` per-type safety logging.

**Deferred from 0.9.0:**
- O1 (superinstruction merging) — Deferred indefinitely; the opcode table is full (254/256 used) and adding superinstructions requires an opcode-space redesign first.
- A12 (lazy work-variable init) — Too complex and disruptive; also blocked by Issues 68–70; deferred to 1.1+.
- A5 (closure capture) — Depends on P1; very high effort; 1.1+.
- A7 (native extension libraries) — Moved to 0.9.0.

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
`src/generation/` already translates the loft IR to Rust source; the code exists but
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
and tooling; 0.8.3 adds R1 + W1 (WASM runtime); 1.0.0 adds W2–W6 (IDE) on top of a complete 0.9.0.  No item moves
forward until the test suite for the previous item is green.  This prevents the "everything
at once" failure mode where half-finished features interact and regressions are hard to pin.

---

### Recommended Implementation Order

Ordered by unblocking impact and the small-steps principle (each item leaves the codebase
in a better state than it found it, with passing tests).

**Released as 0.8.2 (2026-03-24).**

**For 0.8.3 (after 0.8.2 is tagged):**
1. **P3** + **L2** — aggregates and nested patterns; P3 depends on P1 (done in 0.8.2); batch together
2. **P5** — generic functions; independent of P3/L2; land after data.rs changes settle
3. **A10** — field iteration; independent, medium; can land in parallel with P3

**For 0.8.4 (after 0.8.3 is tagged):**
1. **H1** — `#json` + `to_json`; Small, no new Rust deps; validates annotation parsing
2. **H2** — JSON primitive stdlib; Small–Medium, new `src/database/json.rs` (~80 lines, no new dep); test each extractor in isolation
3. **H3** — `from_json` scalar codegen; Medium, depends on H1 + H2; verify `Type.from_json` as fn-ref
4. **H4** — HTTP client + `HttpResponse`; Medium, adds `ureq`; test against httpbin.org or mock
5. **H5** — nested/array/enum `from_json` + integration tests; Med–High, depends on H3 + H4

**For 0.9.0 (after 0.8.4 is tagged):**
1. **L1** — error recovery; standalone UX improvement, no dependencies; also unblocks P2.4
2. **A2** — logger remaining work; independent, small-medium; can land any time
3. **P2** — REPL; high effort; land after L1 (needed for P2.4 error recovery)

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

---

---

---

### L6  Prevent double evaluation of `expr ?? default` *(completed 0.8.3)*

Implemented: non-trivial LHS expressions are materialised into a temp variable
before building the null-check conditional.  Tests in `25-null-coalescing.loft`.

---

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

### P3  Vector aggregates *(completed 0.8.3)*

`sum_of`/`min_of`/`max_of` implemented as pure-loft reduce wrappers for
`vector<integer>`.  `any(vec, pred)`, `all(vec, pred)`, `count_if(vec, pred)`
implemented as compiler special-cases with short-circuit evaluation and
lambda type inference support.

---

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

### T1  Tuple types
**Sources:** TUPLES.md
**Description:** Multi-value returns and stack-allocated `(A, B, C)` compound values. Enables functions to return more than one value without heap allocation. Seven implementation phases; full design in [TUPLES.md](TUPLES.md).

- **T1.1** — Type system *(completed 0.8.3)*: `Type::Tuple(Vec<Type>)` variant, `element_size`, `element_offsets`, `owned_elements` helpers in `data.rs`.
- **T1.2** — Parser *(completed 0.8.3)*: type notation `(A, B)`, literal syntax `(expr, expr)`, element access `t.0`, LHS destructuring `(a, b) = expr`.  `Value::Tuple` IR variant added.
- **T1.3** — Scope analysis *(completed 0.8.3)*: tuple variable intervals, owned-element cleanup tracking in `scopes.rs`.
- **T1.4** — Bytecode codegen *(completed 0.8.3)*: `Value::TupleGet` IR, element read via `OpVar*` at offset, tuple set via per-element `OpPut*`, tuple parameters.  6 tests passing; function-return convention, text elements, destructuring, and element assignment remain for follow-up.
- **T1.5** — *(completed 0.8.3)* SC-4: `RefVar(Tuple)` element read/write via `OpVarRef` + element offset; `parse_ref_tuple_elem` helper in operators.rs.
- **T1.6** — *(completed 0.8.3)* SC-8: `check_ref_mutations` emits WARNING (not error) for `RefVar(Tuple)` params never written; `find_written_vars` recognises `TuplePut`.
- **T1.7** — *(completed 0.8.3)* SC-7: `Type::Integer` gains a `not_null: bool` third field; `parse_type` accepts `not null` suffix; null assigned to a `not null` tuple element is a compile error.

- **T1.8** — Tuple function return convention + text elements (C20).
  Two sub-issues remain after T1.1–T1.7:

  **T1.8a — Function return convention:** A function declared `-> (A, B)` must write its return value directly into the caller’s pre-allocated slot.  This requires (1) codegen to allocate the tuple on the caller’s stack before the call; (2) a `ReturnTuple` IR variant; (3) `OpReturnTuple(size)` that copies from the callee stack to the pre-allocated slot.
  
  **T1.8b — Text elements:** `Type::Text` inside a `Type::Tuple` needs lifetime tracking and `OpFreeRef`-style cleanup for the text slot on scope exit.  `owned_elements` in `data.rs` must enumerate text positions within a tuple so `get_free_vars` can emit the right cleanup sequence.

  **Effort:** Medium  
  **Target:** 1.1+

**Effort:** Very High
**Target:** 1.1+

---

### CO1  Coroutines
**Sources:** COROUTINE.md
**Description:** Stackful `yield`, `iterator<T>` return type, and `yield from` delegation. Enables lazy sequences and producer/consumer patterns without explicit state machines. Six implementation phases; full design in [COROUTINE.md](COROUTINE.md).

- **CO1.1** — *(completed 0.8.3)* `CoroutineStatus` enum in `default/05_coroutine.loft`; `CoroutineFrame` struct, coroutine storage, and helpers on State.
- **CO1.2** — *(completed 0.8.3)* `OpCoroutineCreate` + `OpCoroutineNext` opcodes: frame construction (argument copy, COROUTINE_STORE DbRef push) and advance (stack restore, call-frame restore, state machine).
- **CO1.3** — `OpYield` + `OpCoroutineReturn` + parser `yield` keyword.  Split into five independently testable sub-steps:

  **CO1.3a — `OpCoroutineReturn` opcode** *(completed 0.8.3)*:
  `coroutine_return(value_size)` on State: clears text_owned/stack_bytes, truncates
  call_stack, marks Exhausted, pops active_coroutines, pushes null, returns to consumer.
  Fixes #96.

  **CO1.3b — `OpCoroutineYield` opcode (integer-only)** *(completed 0.8.3)*:
  `coroutine_yield(value_size)` on State: serialises stack[stack_base..stack_pos] into
  stack_bytes, saves call frames, suspends, slides yielded value to stack_base, returns
  to consumer.  Text serialisation deferred to CO1.3d.  Fixes #95.

  **CO1.3c — Parser: `yield` keyword + codegen emit** *(completed 0.8.3)*:
  `yield` lexer keyword added.  `yield expr` parsed as `Value::Yield(Box<Value>)`.
  `iterator<T>` single-parameter syntax accepted.  Codegen: OpCoroutineCreate for
  generator calls, OpCoroutineYield for yield, OpCoroutineReturn for generator return.
  Remaining: generator body return-type check suppression and `next()` wiring.

  **CO1.3d — Text serialisation** *(completed 0.8.3)* (`src/state/codegen.rs`, `src/state/mod.rs`):
  Two root causes for SIGSEGV in generators with `text` parameters: (1) `coroutine_create`
  now appends a 4-byte return-address slot to `stack_bytes` so `get_var` offsets match the
  codegen-time layout on every resume; (2) `Value::Yield` codegen decrements `stack.position`
  by the yielded value size after emitting `OpCoroutineYield`, so subsequent variable accesses
  use correct offsets on second and later resumes.  Fixes #94.

  **CO1.3e — Nested yield** *(completed 0.8.3)*:
  Call-stack save/restore in `OpCoroutineYield` / `OpCoroutineNext` verified for nested
  helper calls between yields.

- **CO1.4** — *(completed 0.8.3)* `yield from sub_gen` parsed and desugared to
  advance-loop + yield forwarding.

  **CO1.4-fix** — *(completed)* The slot-assignment regression (C21) was resolved
  by the two-zone slot redesign (S17/S18): the `__yf_sub` coroutine handle and
  inner loop temporaries no longer overlap.  Test `coroutine_yield_from` passes
  without `#[ignore]`.
- **CO1.5** — *(completed 0.8.3)* `for item in generator` integration + `e#remove` rejection.
- **CO1.3e** — *(completed 0.8.3)* Nested yield verified — helper call between yields.

- **CO1.6** — *(completed 0.8.3)* `next()` / `exhausted()` stdlib, stack tracking fix,
  null sentinel on exhaustion.  `OpCoroutineNext` and `OpCoroutineExhausted` bypass the
  operator codegen path; stack.position manually adjusted.  `push_null_value` writes
  `i32::MIN` / `i64::MIN` for typed null returns.

**Effort:** Very High
**Depends:** TR1
**Target:** 1.1+

---

## A — Architecture

### A1  Parallel workers: struct/reference return types *(completed 0.8.3)*

All parallel worker return types are now supported: primitives, text, enum, and
struct/reference.  Struct returns use deep-copy (`copy_block` + `copy_claims`) in
the bytecode interpreter and `n_parallel_for_ref_native` in native codegen.
The native skip for `40-par-ref-return` has been removed.

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

**Phase 1 — Capture analysis** *(completed 0.8.3)*:
Parser detects variables from enclosing scopes referenced inside lambdas.  Emits a clear
error ("lambda captures variable 'name' — closure capture is not yet supported") and
creates a placeholder variable so parsing continues without cascading errors.  Capture
context saved/restored in both parse_lambda and parse_lambda_short.

**Phase 2 — Closure record layout** *(completed 0.8.3)*:
For each capturing lambda, the parser synthesizes `__closure_N` with fields matching
the captured variables.  The record def_nr is stored on Definition.closure_record.
Diagnostic emitted with field count/names/types for test verification.

**Phase 3 — Capture at call site** *(completed 0.8.3)*:
Capture diagnostic updated from generic "not yet supported" to specific "closure body
reads not yet implemented (A5.4)".  Closure record struct (A5.2) is still synthesized.
Actual closure record allocation IR and codegen deferred to A5.4.

**Phase 4 — Closure body reads** *(completed 0.8.3)*:
Hidden `__closure` parameter added on second pass.  Captured variable reads redirect
to `get_field` on the closure record.  Read-only captures work; mutable captures
(`count += x`) pending — codegen panics on self-reference for write targets.

**Phase 5 — Lifetime and cleanup** *(completed 0.8.3)*:
Closure record work variable (Type::Reference with empty deps) is already freed by
the existing OpFreeRef scope-exit logic in get_free_vars.  No new code needed.
Per-field text/reference cleanup inside the record is pending — only matters when
text captures become testable.

**Phase 6 — Mutable capture + text capture** (C1 remaining, tracked as A5.6):
Two remaining restrictions after A5.1–A5.5:

**A5.6a — Mutable capture** *(completed 0.8.3)*:
`capture_detected` passes without source changes.  The mutable-capture path
(`count += x`) routes through `call_to_set_op` → `OpSetInt`, which never hits the
`generate_set` self-reference guard.  The earlier plan for a `SetClosureField` IR
variant was not needed.  Test: `tests/parse_errors.rs::capture_detected`.

**A5.6b.1 — Text capture: garbage DbRef in `CallRef` stack frame** (✓ implemented in `safe` branch):
Text-capturing, text-returning lambdas (e.g. `fn(name: text) -> text { "{prefix} {name}" }`)
produce a garbage `__closure` DbRef at runtime, causing panics such as "Unknown record
49745" or "Store write out of bounds".  Integer-only captures work correctly.

**Root cause — `text_return()` adds captured text variables as spurious work-buffer attributes:**

When the lambda body `"{prefix} {name}"` is compiled, the format-string processor calls
`text_return(ls)` (control.rs:1550) where `ls` contains the text variables referenced in
the format string — including the captured variable `prefix`.

`text_return` iterates over `ls` and for each text variable that is NOT already an
attribute of the lambda, it adds it as a `RefVar(Text)` attribute (a hidden work-buffer
argument) and calls `self.vars.become_argument(v)`.  The guard that skips already-registered
attributes (line 1557: `attr_names.get(n)`) does NOT catch captured variables — at the point
`text_return` runs, `prefix` is not yet registered as an attribute (the hidden `__closure`
parameter is added later in `parse_lambda`).

Result: `prefix` is added as a `RefVar(Text)` attribute of the lambda, giving the lambda
an **extra 12-byte argument slot** that the caller knows nothing about.

**Broken argument layout (with the bug):**

The lambda’s `def_code` processes attributes in order:
1. `name: text` → slot 0, 16 bytes (`size_of::<&str>()`)
2. `prefix: RefVar(Text)` → slot 16, 12 bytes ← spurious, added by `text_return`
3. `__closure: Reference` → slot 28, 12 bytes

Total argument area = 40 bytes; `+4` for return addr → TOS at 44.
Reading `__closure`: `var_pos = 44 - 28 = 16`.  At runtime `stack_pos - 16 = args_base + 16`.

But the caller only pushes 28 bytes (`name` 16 + `__closure` DbRef 12):
- `args_base + 0..16`: `name` ✓
- `args_base + 16..28`: closure DbRef ← callee reads this as `prefix` slot
- `args_base + 28..40`: **nothing** ← callee reads this as `__closure` slot → garbage

**Fix (concrete — `src/parser/control.rs`, `text_return`):**

Add a captured-variable guard immediately after the existing `attr_names` check:

```rust
pub(crate) fn text_return(&mut self, ls: &[u16]) {
    if let Type::Text(cur) = &self.data.definitions[self.context as usize].returned {
        let mut dep = cur.clone();
        for v in ls {
            let n = self.vars.name(*v);
            let tp = self.vars.tp(*v);
            // skip related variables that are already attributes
            if let Some(a) = self.data.def(self.context).attr_names.get(n) {
                if !dep.contains(&(*a as u16)) {
                    dep.push(*a as u16);
                }
                continue;
            }
            // A5.6b.1: skip captured variables — they are read from the closure
            // record at runtime, not passed as hidden work-buffer arguments.
            // Adding them as RefVar(Text) attributes shifts __closure to the wrong
            // argument slot, giving the lambda a garbage DbRef.
            if self.captured_names.iter().any(|(name, _)| name == n) {
                continue;
            }
            if matches!(tp, Type::Text(_)) {
                // ... rest unchanged
```

After this fix, the lambda’s argument layout becomes:
1. `name: text` → slot 0, 16 bytes
2. `__closure: Reference` → slot 16, 12 bytes
Total = 28 bytes, matching what the caller pushes. ✓

**Why `name` is still handled correctly:**

`name` IS already an attribute (it’s the declared parameter), so the `attr_names.get(n)`
check catches it and just adds its attribute index to `dep`.  The format string’s text
dependency tracking for `name` still works — only the spurious insertion of `prefix` as
a work-buffer attribute is suppressed.

**Scope of fix:** Only affects lambdas that (a) return text AND (b) capture text variables
from an outer scope.  No other code path is changed.

**Test scope note:** The existing `closure_capture_text` test (`make_greeter("Hello")("world")`)
crosses function scope — the closure is returned from `make_greeter` and called from
outside.  The `last_closure_alloc` block references variable slots in `make_greeter`'s
frame; calling the returned fn-ref from a different scope would access those stale slots.
This pattern requires A5.6 (1.1+) — returning a closure alongside its DbRef.

After this fix, add a **same-scope** test that exercises A5.6b.1 directly:
```
prefix = "Hello";
f = fn(name: text) -> text { "{prefix} {name}" };
f("world")  // expected: "Hello world"
```
Same-scope calls use `last_closure_alloc` correctly (consumed at the call site within
the same definition) and do not require the returning-closure architecture.  The
existing `closure_capture_text` test should remain `#[ignore]` until A5.6 (1.1+).

**A5.6b.2 — `generate_call_ref`: text work buffers not pre-allocated** (✓ implemented in `safe` branch):
Text-returning lambdas called via `CallRef` fail even after the DbRef fix because
`generate_call_ref` does not pre-allocate the text work variables (e.g. `__work_1`)
that `text_return()` in `control.rs` adds to the lambda’s stack frame.  `generate_call`
allocates these with `OpAllocText` before the call; `generate_call_ref` has no
equivalent.  Result: the lambda reads garbage from uninitialized stack positions.

The debug assertion added in this session (codegen.rs:1170–1178) fires for this case
in debug builds, making the problem immediately visible.

**Fix path (concrete — `src/state/codegen.rs`, `generate_call_ref`):**

The fn-type stored in the fn-ref variable is `Type::Function(param_types, ret_type)`
where `ret_type = Type::Text(deps)` and `deps` is the list of hidden `__work_N`
variables required.  `deps.len()` gives the count of text work buffers needed.

After fixing A5.6b.1 (so the DbRef is valid), extend `generate_call_ref` to emit
`OpAllocText` for each work buffer before `OpCallRef`:

```rust
// A5.6b.2: pre-allocate text work buffers required by text-returning lambdas.
// `generate_call` does this via try_text_dest_pass; CallRef must do it manually.
let work_count = if let Type::Text(deps) = &ret_type { deps.len() } else { 0 };
for _ in 0..work_count {
    stack.add_op("OpAllocText", self);
    stack.position += size(&Type::Text(vec![]), &Context::Variable) as u16;
}
```

This block is inserted after generating the declared args but before emitting the
closure-arg generation (so the work buffers are below the closure DbRef on the stack,
matching what `generate_call` produces).  The `total_arg_size` computation must also
account for the added work-buffer bytes:

```rust
let work_size = work_count as u16
    * size(&Type::Text(vec![]), &Context::Variable) as u16;
let total_arg_size = declared_size + extra + work_size;
```

Verify that the lambda’s `def_code` assigns `__work_N` slots AFTER declared params
and BEFORE `__closure`, so the stack layout matches what `generate_call` produces.
Adjust the push order in `generate_call_ref` if needed.

**A5.6c — Mutable capture write-back: void-return lambdas** (✓ implemented in `safe` branch):
A void-return capturing lambda (`fn(x: integer) { count += x; }`) updates the
`count` field inside the closure record, but the outer `count` variable (in the
caller’s stack frame) is never updated.  After `f(10); f(32)`, the outer `count`
remains 0.

The lambda’s IR correctly modifies the closure record field (A5.6a is done — the
`capture_detected` test proves mutable field writes work inside the lambda body).
The missing step is the write-back from closure record to outer variable after each
`CallRef` returns.

**Fix path (concrete — parser `control.rs`, call site generation):**

At the call site where `Value::CallRef(v_nr, args)` is built (control.rs:2000),
after constructing `converted`, emit write-back IR for each mutable captured variable:

```rust
// A5.6c: after CallRef to a closure, write captured mutable fields back to
// the outer variables so the caller sees the updated values.
if let Some(&closure_w) = self.closure_vars.get(&v_nr) {
    // closure_vars maps fn-ref var → closure work var (the __clos DbRef in scope).
    let closure_rec = self.data.def(d_nr).closure_record;
    if closure_rec != u32::MAX {
        for aid in 0..self.data.attributes(closure_rec) {
            let cap_name = self.data.attr_name(closure_rec, aid);
            let outer_v = self.vars.var(&cap_name);
            if outer_v != u16::MAX {
                // Emit: outer_var = get_field(__clos, aid)
                write_back_ops.push(self.get_field(closure_rec, aid, 0,
                    Value::Var(closure_w)));
                write_back_ops.push(Value::Set(outer_v, /* get_field result */));
            }
        }
    }
}
```

The exact IR construction follows the existing `set_field_no_check` / `get_field`
helpers.  The write-back IR is emitted as statements immediately after the
`Value::CallRef(...)` expression in the enclosing block.

**Prerequisite:** `closure_vars` must be populated for the fn-ref variable `v_nr`.
Currently `closure_vars.insert` fires only when `last_closure_work_var != u16::MAX`,
but `last_closure_work_var` is never set.  Fix: in `emit_lambda_code` (vectors.rs),
after creating `w` (the `__clos` work var), set `self.last_closure_work_var = w`.
Then in `parse_assign` (expressions.rs:710–712), `closure_vars.insert(var_nr, w)`
fires correctly.

**Test:** Remove `#[ignore]` from `tests/issues.rs::p1_1_lambda_void_body` and
update the ignore reason from the old "A5, 1.1+" text to "A5.6c" once the fix is
implemented.

**Effort:** A5.6b.1 Medium · A5.6b.2 Small · A5.6c Medium
**Target:** 0.8.3 (A5.6b.1, A5.6b.2, A5.6c); A5.6 (capture-at-definition-time) in 1.1+

**A5.6 — Full capture-at-definition-time semantics** (1.1+, depends on A5.6b.1–A5.6c):
After A5.6b.1, A5.6b.2, and A5.6c are resolved, the remaining gaps in closure
semantics are:

1. **Lambdas as first-class values stored in collections or struct fields.**
   Currently `closure_vars` only maps fn-ref variables created by direct assignment.
   If a lambda is stored in a vector or struct field, the closure record association
   is lost and the hidden `__closure` arg is never injected at call sites.
   Fix: extend `closure_vars` to handle indirect storage, or store the closure DbRef
   alongside the fn-ref `d_nr` value as a single `(d_nr, closure_dbref)` pair in a
   16-byte fn-ref slot (requires opcode changes).

2. **Lambda re-definition (reassignment of a fn-ref variable).**
   If `f = fn(x) { count += x }` is followed by `f = fn(x) { count -= x }`, the
   second assignment clears `closure_vars[f]` and sets a new closure record.  The
   old closure record must be freed.  `get_free_vars` must emit `OpFreeRef` for the
   OLD closure record at the reassignment point.

3. **Returning a closure from a function.**
   `fn make_adder(n: integer) -> fn(integer) -> integer { fn(x: integer) -> integer { n + x } }`
   The closure record must outlive `make_adder`'s stack frame.  Currently the
   `__clos` variable lives on `make_adder`'s stack and is freed when `make_adder`
   returns — before the returned fn-ref can be called.  Fix: the closure record must
   be heap-allocated (already the case via `OpDatabase`) and returned alongside the
   fn-ref as a `(d_nr, closure_dbref)` pair; `make_greeter` in the failing test
   exercises exactly this scenario.

4. **Closure lifetime and sharing.**
   When the same lambda is called multiple times, the closure record is shared across
   all calls.  After A5.6c adds write-back, each call to `f(10)` reads the closure
   record, modifies it, and writes back.  Concurrent use (via `par()`) would require
   locking or per-call copies — deferred to the parallel safety audit.

**Effort:** High (multiple sub-items, some requiring opcode-level changes)
**Depends on:** A5.6b.1, A5.6b.2, A5.6c
**Target:** 1.1+

---

### A7  Native extension libraries *(completed 0.8.3)*
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
**Target:** 0.8.3

---

### A10  Field iteration — `for f in s#fields` *(completed 0.8.3)*
**Sources:** Design evaluation 2026-03-18; syntax decision 2026-03-19
**Description:** Allow iterating over the stored primitive fields of a struct value with
`for f in s#fields`.  The loop variable `f` has type `Field` (defined in
`default/01_code.loft`) with `f.name: text` (the compile-time field name) and
`f.value: FieldValue` (a struct-enum covering all primitive types).  Native type capture
uses existing `match f.value { Float{v} => ... }` pattern syntax.

The loop is a compile-time unroll: the parser expands `for f in s#fields` into one
sequential block per eligible field.  No runtime allocation is needed.  Fields whose
type is a reference, collection, or nested struct are skipped in this version.

**Syntax choice — `s#fields` vs `fields(s)`:**
`s#fields` was chosen over `fields(s)` to avoid reserving `fields` as a keyword.
`fields` is a common English word (it was already used as an identifier in 3 stdlib files
and had to be renamed when L3 added it to KEYWORDS).  The `#` postfix pattern already
avoids keyword reservation for `count`, `first`, `index`, `remove`, etc., and the same
mechanism works here.  Constraint: the source `s` must be a plain identifier; for complex
expressions, assign a temporary first (`let cfg = get_config(); for f in cfg#fields`).

```loft
struct Config { host: text, port: integer not null, debug: boolean }
c = Config{ host: "localhost", port: 8080, debug: true };

for f in c#fields {
    match f.value {
        Text { v } => log_info("{f.name} = '{v}'")
        Int  { v } => log_info("{f.name} = {v}")
        Bool { v } => log_info("{f.name} = {v}")
        _          => {}
    }
}
```

**Fix path:**

**Phase A10.0 — Remove `fields` from `KEYWORDS`** (`src/lexer.rs`):
Delete `"fields"` from the `KEYWORDS` static array (reverting the L3 code change).
The identifier renames made during L3 (`type_fields`, `flds`, `items`) can remain as
they are improvements in their own right.
*Tests:* existing tests pass; `fields` is legal as a variable, function, and field name
in user code again.

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

**Phase A10.2 — `ident#fields` detection in `parse_for`** (`src/parser/collections.rs`,
`src/data.rs`):
In `parse_for`, after reading the source identifier, check `lexer.has_token("#")` followed
by `lexer.has_keyword("fields")`.  If matched, resolve the identifier's type; validate it
is a struct (non-struct → clear compile error: `#fields requires a struct variable, got
<type>`).  Return a new IR node `Value::FieldsOf(struct_def_nr, Box<source_expr>)` with
type `Type::FieldsOf(struct_def_nr)`.

```
// data.rs — add to Value enum
FieldsOf(u32, Box<Value>),   // (struct def_nr, source expression)

// data.rs — add to Type enum
FieldsOf(u32),               // struct def_nr; erased after loop unrolling
```

*Tests:* `for f in point#fields` on a known struct type-checks without error; `for f in
n#fields` where `n: integer` produces one diagnostic naming the offending type.

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

`break` and `continue` inside a `for f in s#fields` body are a compile error in this
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
- Non-struct `n#fields` where `n: integer`: single diagnostic, no crash.

**Phase A10.4 — Error messages and documentation** (`doc/claude/LOFT.md`,
`doc/claude/STDLIB.md`):
Polish pass: verify error messages are clear and point to the right source location.
Add `s#fields` to LOFT.md § Control flow (alongside `for`) and to STDLIB.md § Structs.
Document the skipped-field limitation, the identifier-only constraint, and the future
`A10+` path for non-primitive fields.
*Tests:* `ref_val#fields` (reference type, not the struct it points to) gives a clear
error distinguishing "you have a reference; use a struct variable, not a reference" from
the generic type-mismatch message.

**Files changed:**

| File | Change |
|---|---|
| `src/lexer.rs` | Remove `"fields"` from `KEYWORDS` (A10.0) |
| `default/01_code.loft` | Add `FieldValue` (struct-enum, 8 variants) and `Field` (struct) |
| `src/data.rs` | Add `Value::FieldsOf(u32, Box<Value>)` and `Type::FieldsOf(u32)` |
| `src/parser/collections.rs` | Detect `ident#fields` in `parse_for`; build unrolled block IR |
| `src/typedef.rs` | Erase `Type::FieldsOf` after unrolling (it should not appear in bytecode) |
| `tests/docs/21-field-iter.loft` | New — test coverage |
| `tests/wrap.rs` | Add `field_iteration()` test |
| `doc/claude/LOFT.md` | Document `for f in s#fields` in the For-loop section |
| `doc/claude/STDLIB.md` | Add `s#fields` to the Structs section |

**Limitations (initial version):**
- Only primitive-typed fields are visited; reference, collection, and nested-struct fields
  are silently skipped.
- `break` and `continue` are not supported inside the loop body.
- The source must be a plain identifier, not an arbitrary expression.  Use a temporary:
  `let cfg = get_config(); for f in cfg#fields { ... }`.
- `s#fields` is only valid as the source expression of a `for` loop, not as a standalone
  expression producing a `vector<Field>`.
- `virtual` fields are included (they are read-only computed values, still primitive).

**Effort:** Medium (data.rs + 2 parser files + default library; no bytecode changes)
**Target:** 0.8.3

---

### S14  Struct-enum stdlib field positions *(completed 0.8.3)*

Fixed: `fill_all()` now processes all definitions from 0 (not `start_def`), and the
discriminant field uses `database.byte(0, false)` instead of `database.name("byte")`.

---

### S15  Struct-enum same-name variant field offsets *(completed 0.8.3)*

Fixed: match arm field bindings now use per-arm unique variables via
`create_unique` + temporary name aliasing.  Each arm's variable has the
correct type for its variant, avoiding the type/slot reuse bug.
Field offsets were already correct in the database — the root cause was
`add_variable` reusing the first arm's variable for subsequent arms.
A10 field iteration test now passes.

**Effort:** Medium (requires understanding database field layout)
**Target:** 0.8.3

---

### L7  Non-zero exit code on parse/runtime errors
**Sources:** CAVEATS.md C6, `src/main.rs`, `src/diagnostics.rs`
**Severity:** Medium — shell scripts that use `loft` as a pipeline step check `$?` to detect failures; returning 0 on error silently swallows failures.
**Description:** Two issues in `src/main.rs`:

1. **Parse/compile error path (line 343):** The diagnostic check `if !p.diagnostics.is_empty()` exits with code 1 whenever any diagnostic is present, including warnings-only programs. This is too aggressive: a program with only warnings should execute and exit 0.
2. **Warning-only programs don't run:** Because warnings cause exit 1 at line 343, a program like `46-caveats.loft` (which has a C14 format-specifier warning) would not execute at all when invoked via the CLI — even though the interpreter test harness runs it fine (bypasses `main.rs`).

**Fix path (`src/main.rs` lines 343–348):**
```rust
// Before (exits 1 for any diagnostic including warnings):
if !p.diagnostics.is_empty() {
    for l in p.diagnostics.lines() {
        println!("{l}");
    }
    std::process::exit(1);
}
```
```rust
// After (print all diagnostics, only exit 1 for errors or fatal):
if !p.diagnostics.is_empty() {
    for l in p.diagnostics.lines() {
        println!("{l}");
    }
    if p.diagnostics.level() >= Level::Error {
        std::process::exit(1);
    }
}
```
Import `Level` from `crate::diagnostics::Level` if not already in scope.

**Scope check diagnostics:** `scopes::check` does not produce a separate `Diagnostics`; its errors are printed directly via the parser’s lexer and collected into `p.diagnostics`. Verify with a scope-error test.

**Runtime fatal path (line 553):** `state.database.had_fatal` already correctly exits 1 on `log_fatal()`. No change needed there.

**`--format-check` path:** Already exits 1 on bad format (line 106). No change needed.

**Test plan:**
1. `cargo run --bin loft -- tests/scripts/46-caveats.loft` — should print the C14 warning and then execute, printing `caveats: all ok`, exiting 0.
2. `echo 'fn main() { x = 1' | cargo run --bin loft -- /dev/stdin` — should exit 1.
3. Add shell-level test in `tests/integration.rs` (or a new `tests/exit_codes.rs`) that invokes the binary and checks `$?`.
**Effort:** Small
**Target:** 0.8.3

---

### L8  Warn on format specifier / type mismatch *(completed 0.8.3)*

Implemented: compile-time warnings in `append_data()` for numeric format specifiers
on text/boolean and zero-padding on text.  Tests in `38-parse-warnings.loft`.

---

### L9  Format specifier / type mismatch — escalate to compile error
**Status: completed**
Changed `Level::Warning` → `Level::Error` in `append_data()` for radix specifiers on
text/boolean and zero-padding on text.  Tests updated in `38-parse-warnings.loft`.
CAVEATS.md C14 closed.

---

### L10  `while` loop syntax sugar
**Status: completed**
Added `while` keyword to the lexer and `parse_while()` in `expressions.rs`.
Desugars to `v_loop([if !cond { break }, body])`.  Tests in `46-caveats.loft`.
CAVEATS.md C11 closed.

---

### A12  Lazy work-variable initialization
**Status: deferred to 1.1+ — too complex and disruptive for stability; also blocked by Issues 68–70 (see PROBLEMS.md)**
**Sources:** Stack efficiency evaluation 2026-03-20
**Description:** Work text variables (`__work_N`) are currently initialized at function
start via `Set(wt, Text(""))` inserted at index 0 of the body block.  This forces
`first_def = 0` for every work text variable, making its live interval span the entire
function.  Two sequential, non-overlapping text operations each hold a 24-byte slot for
the full lifetime of the call frame.  The same applies to non-inline work ref variables
(`__ref_N`), which also get function-start null-inits.

Inline-ref temporaries already use lazy insertion (per A6.3a work): their null-init is
placed immediately before the statement that first assigns them, giving accurate intervals.
This item extends that approach to all work variables.

**Fix path:**

*Step 1 — Rename and generalize `inline_ref_set_in`* (`src/parser/expressions.rs`):

Rename `inline_ref_set_in` to `first_set_in` (or add it as a general helper).  No logic
changes — the function already recurses into all relevant `Value` variants and works
correctly for both text and ref work variables.

*Step 2 — Extend insertion loop in `parse_code` to work texts*:

Replace the eager-insert loop for work texts with a lazy-insert using `first_set_in`.
Non-inline work references remain eagerly inserted at position 0 (see blocker below).
Inline-ref variables continue to use the same lazy path as before.

```rust
// BEFORE: for wt in work_texts() { ls.insert(0, v_set(wt, Text(""))) }
// AFTER: find the first top-level statement containing a Set to wt, insert before it.
let mut insertions: Vec<(usize, u16, Value)> = Vec::new();
for wt in self.vars.work_texts() {
    let pos = ls.iter().position(|stmt| first_set_in(stmt, wt, 0)).unwrap_or(fallback);
    insertions.push((pos, wt, Value::Text(String::new())));
}
// work_references: still position 0 (blocker: Issue 68)
for r in self.vars.work_references() {
    if !is_argument && depend.is_empty() && !is_inline_ref {
        insertions.push((0, r, Value::Null));
    }
}
for r in self.vars.inline_ref_references() { ... lazy as before ... }
insertions.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
for (pos, r, init) in insertions { ls.insert(pos, v_set(r, init)); }
```

**Known blockers (found during 2026-03-20 implementation):**

- **Issue 68** — `first_set_in` does not descend into `Block`/`Loop` nodes.  Work
  references used only inside a nested block cannot be found; the fallback position lands
  *after* the block, giving `first_def > last_use`.  Fix: add `Block` and `Loop` arms to
  `first_set_in`.  Until then, non-inline work references stay at position 0.

- **Issue 69** — Extending `can_reuse` in `assign_slots` to `Type::Text` causes slot
  conflicts: two smaller variables can independently claim the first bytes of the same
  dead 24-byte text slot.  The `assign_slots_sequential_text_reuse` unit test passes in
  isolation (with explicit non-overlapping intervals) but the integration suite fails.
  Full text slot sharing also requires OpFreeText to be placed after each variable's last
  use (not at function end), otherwise sequential work texts still have overlapping live
  intervals.  Both issues must be resolved before `can_reuse` is extended.

- **Issue 70** — Adding `Type::Text` to the `pos < TOS` bump-to-TOS override in
  `generate_set` causes SIGSEGV in `append_fn`.  This override was added to handle
  "uninitialized memory if lazy init places a text var below current TOS", but that
  scenario only arises when text slots are reused (Issue 69), which is disabled.  The
  override must be reverted until text slot reuse is safe.

*Interval effect (partial):* `first_def` for work texts is now accurate.  Slot sharing
requires resolving Issues 69 and 70 and moving OpFreeText to after each variable's last
use.

**Tests:** `assign_slots_sequential_text_reuse` in `src/variables/` (currently
`#[ignore]` — pending Issue 69 fix).
**Effort:** Medium (three inter-related blockers; Issues 68–70)
**Target:** 0.8.2

---

### A13  Complete two-zone slot assignment (Steps 8 and 10) *(completed 0.8.3)*

All steps done.  Step 8 was completed earlier.  Step 10:
- **10a** — Full cross-check (2026-03-24) confirmed `build_scope_parents`, `scan_inner`,
  and `compute_intervals` all handle every `Value` variant with nested expressions.
- **10b** — `Value::Iter` arm already present in `scan_inner` (added earlier).
- **Scope-cycle root cause** — Fixed: `build_scope_parents` now uses `entry().or_insert()`
  to keep the first-seen parent and skips self-loops (`bl.scope == parent`).

---

### TR1  Stack trace introspection
**Sources:** STACKTRACE.md
**Description:** `stack_trace()` stdlib function returning `vector<StackFrame>`, where each frame exposes function name, source file, and line number. Full design in [STACKTRACE.md](STACKTRACE.md). Prerequisite for CO1 (coroutines use the frame vector for yield/resume).

- **TR1.1** — Shadow call-frame vector *(completed 0.8.3)*: CallFrame struct and call_stack on State; OpCall encodes d_nr and args_size; fn_call pushes, fn_return pops.
- **TR1.2** — Type declarations *(completed 0.8.3)*: ArgValue, ArgInfo, VarInfo, StackFrame in `default/04_stacktrace.loft`.
- **TR1.3** — Materialisation *(completed 0.8.3)*: `stack_trace()` native function builds `vector<StackFrame>` from snapshot. Tests blocked by Problem #85.
- **TR1.4** — Call-site line numbers *(completed 0.8.3)*: CallFrame stores source line directly; resolved in fn_call. Tests blocked by Problem #85.

**Effort:** Medium
**Target:** 0.9.0

---


## S — Stability Hardening

Items found in a systematic stability audit (2026-03-20).  Each addresses a panic,
silent failure, or missing bound in the interpreter and database engine.  All target 0.8.2.

---

### S6  Fix remaining "recursive call sees stale attribute count" cases
**Sources:** PROBLEMS.md #84
**Severity:** Medium — the merge-sort use-after-free (the primary manifestation) was fixed in 0.8.2.  Complex mutual-recursion patterns that trigger `ref_return` on a function after its recursive call sites were already compiled may still produce wrong attribute counts.
**Description:** `ref_return` adds work-ref attributes to a function's IR while the body is still being parsed.  When the function is recursive, call sites parsed before `ref_return` runs see the old (smaller) attribute count.  The merge-sort case was fixed by guarding `vector_needs_db` with `!is_argument` and injecting the return-ref in `parse_return`.  A general fix would scan the IR tree after the second parse pass and patch under-argument recursive calls via `add_defaults`.
**Fix path:** Post-parse IR scan and call-site patching in `parse_function`.
**Effort:** Medium
**Target:** 1.1+

---

### S19  Fix #85: struct-enum locals not freed in debug mode
**Sources:** PROBLEMS.md #85, CAVEATS.md C16
**Severity:** Low in production (no assertion), critical in debug builds (SIGABRT).
**Description:** `scopes.rs::free_vars()` emits `OpFreeRef` for plain struct local variables but not for struct-enum locals.  In debug builds, the store's allocation assert fires at scope exit because the record is still live.
**Fix path:**
1. In `get_free_vars` (or equivalent), add a branch for `Type::Named(_, _, _)` that is a struct-enum variant — emit `OpFreeRef` exactly as is done for plain structs.
2. Regression test: declare a local struct-enum variable inside a `for` or `if` body; verify no assertion fire in debug, value correct in release.
**Effort:** Small
**Target:** 0.9.0

---

### S20  Fix #91: init(expr) circular dependency silently accepted
**Sources:** PROBLEMS.md #91, CAVEATS.md C18
**Severity:** Medium — silent undefined behaviour at runtime when two store fields form a mutual initialisation cycle.
**Description:** The `init(expr)` attribute on struct fields is evaluated at record creation time.  If field A's init expr reads field B and field B's init expr reads field A, the interpreter reads uninitialised memory.  No cycle check is performed.
**Fix path:**
1. After all struct field defs are parsed, build a dependency graph: edge A→B if field A's init expr contains a read of field B.
2. DFS cycle detection over the graph; emit a compile error naming the cycle.
3. Test: two mutually-referencing `init(...)` fields produce a clear error; acyclic chains are unaffected.
**Effort:** Small
**Target:** 0.9.0

---

### S21  Fix #92: stack_trace() silent empty in parallel workers
**Sources:** PROBLEMS.md #92, CAVEATS.md C17
**Severity:** Medium — debugging parallel code is significantly harder without stack traces.
**Description:** `stack_trace()` reads `state.data_ptr` to walk the call stack.  In parallel workers spawned by `par(...)`, `execute_at` (and `execute_at_ref`) entry points do not set `data_ptr` before dispatch, so the pointer is null and `stack_trace()` returns an empty vec.
**Fix path:**
1. In `execute_at` and `execute_at_ref` in `src/state/mod.rs`, set `self.data_ptr = data as *const Data;` (or equivalent) immediately before the dispatch call, mirroring what the single-threaded `execute` path does.
2. Regression test: call `stack_trace()` inside a `par(...)` worker body; assert the returned vec is non-empty and contains the worker function name.
**Effort:** Small
**Target:** 0.9.0

---

### S22  Fix parallel worker auto-lock in release builds
**Sources:** SAFE.md § P1-R1, CAVEATS.md C22
**Severity:** Medium — release builds silently return wrong results when a worker writes to a `const` argument.
**Description:** The auto-lock insertion (`n_set_store_lock`) for `const` worker arguments is guarded by `#[cfg(debug_assertions)]` in `parser/expressions.rs`.  Release builds never lock the input stores, so a buggy worker that accidentally mutates a `const` argument silently discards the write into a 256-byte dummy buffer and continues with stale data.
**Fix path:**
1. Remove the `#[cfg(debug_assertions)]` guards from the two auto-lock insertion sites in `parse_code` and `expression` that emit `n_set_store_lock` for `const` parameters and local const variables.
2. In `addr_mut` (`store.rs`), change the release-build dummy-buffer path to `panic!("write to locked store")` — no legitimate code path should hit it once auto-lock is unconditional.
3. Add an integration test that runs a `par()` loop whose worker attempts to push to its `const` input in release mode; assert the panic fires with a clear message.
**Effort:** Small
**Target:** 0.8.3

---

### S23  Compiler + runtime: reject `yield` inside `par()` body
**Sources:** SAFE.md § P2-R6, CAVEATS.md C25, COROUTINE.md § SC-CO-4
**Severity:** Medium — `yield` or generator calls inside `par(...)` produce out-of-bounds panics or silent wrong results depending on frame-index collision.
**Description:** No compiler check prevents `yield` or calls to `iterator<T>`-returning functions inside `par(...)` bodies.  Worker `State` instances hold only a null-sentinel `coroutines` table; a DbRef produced by the main thread indexes into it incorrectly.
**Fix path:**
1. In `src/parser/collections.rs` (parallel-for desugaring) and wherever `par(...)` body parsing begins, add an `inside_par_body: bool` flag to the parser context.
2. In `parse_yield` and any site that resolves a function call returning `iterator<T>`, emit a compile error when `inside_par_body` is true.
3. In `coroutine_next` (`state/mod.rs`), add a bounds check: `if idx >= self.coroutines.len() { panic!("iterator<T> DbRef out of range in worker") }`.  This defence-in-depth guard catches the case where the compiler check is missing.
4. Test: a loft program that calls a generator inside `par(...)` produces a compile error; one that bypasses the check triggers the runtime guard in debug.
**Effort:** Small
**Target:** 0.8.3

---

### S24  Compiler + runtime: reject `e#remove` on generator iterator
**Sources:** SAFE.md § P2-R9, CAVEATS.md C26, COROUTINE.md § SC-CO-11
**Severity:** Medium — release builds silently corrupt a real store record; debug builds panic with an uninformative out-of-bounds message.
**Description:** `e#remove` on a generator-typed loop variable passes a DbRef with `store_nr == u16::MAX` (the coroutine sentinel) to `database::remove`.  In debug `u16::MAX` overflows `allocations`; in release `u16::MAX % len` selects a real store and the `rec` (frame index ≈ 1–2) deletes a real record.
**Fix path:**
1. In `src/parser/fields.rs` (or wherever `e#remove` is resolved), check whether the loop's collection type is `iterator<T>` (backed by `OpCoroutineCreate`).  If so, emit: `error: e#remove is not valid on a generator iterator`.
2. In `database::remove` (or the calling opcode), add: `if db.store_nr == COROUTINE_STORE { debug_assert!(false, "remove on coroutine DbRef"); return; }`.  The `return` prevents release-build corruption even if the compiler check is missing.
3. Test: `e#remove` on a generator iterator is a compile error; a debug-only test verifies the runtime guard fires if the check is bypassed.
**Effort:** Extra Small
**Target:** 0.8.3

---

### S25  CO1.3d — coroutine text serialisation (must land atomically)
**Sources:** SAFE.md § P2-R1/R2/R3, CAVEATS.md C23/C24, COROUTINE.md § CO1.3d/SC-CO-1/SC-CO-8/SC-CO-10
**Severity:** Critical (P2-R1 use-after-free) / High (P2-R2 memory leak) — both caused by `text_owned` being permanently empty.
**Description:** `CoroutineFrame.text_owned` is designed to hold owned copies of all dynamic-text locals across suspension.  The serialisation path (`serialise_text_slots`) is specified in COROUTINE.md but not implemented.  Until it lands, text arguments dangle (C23) and text locals leak (C24).  **Must land atomically** — implementing `free_dynamic_str` at yield without simultaneously implementing the pointer-patch at resume and the String drain at exhaustion introduces an explicit use-after-free.
**Fix path:**

#### S25.1 — `serialise_text_slots` at coroutine create + yield
1. Implement `serialise_text_slots(stack_bytes, text_slot_layout, stores) -> Vec<(u32, String)>` per COROUTINE.md spec:
   - Walk each text slot in `stack_bytes` by (offset, Type) from the function definition.
   - Skip null Str and static Str (ptr inside `text_code`).
   - Call `s.str().to_owned()` to make an owned `String`; call `free_dynamic_str` on the original.
   - Patch `stack_bytes` with a Str pointing to the owned buffer; record `(offset, String)` in `text_owned`.
2. Call `serialise_text_slots` from `coroutine_create` (text arg slots) and `coroutine_yield` (all text locals).
3. Add `debug_assert!(def.text_slot_count == 0, …)` to `coroutine_yield` until S25.1 is complete (M8-b from SAFE.md).

#### S25.2 — Pointer-patch on resume + String drain on exhaustion
1. In `coroutine_next`: before copying `stack_bytes` to live stack, write updated `Str` pointers from `text_owned` buffers into the bytes (M6-b from SAFE.md).
2. In `coroutine_return`: drain `text_owned` with `for (_, s) in frame.text_owned.drain(..) { drop(s); }` before `stack_bytes.clear()` (M7-a from SAFE.md).
3. Add a leak-detection test: generator with text local, yields once, loop broken — verify no allocation escapes under Miri or similar.

**Effort:** Large (S25.1 + S25.2 combined; must not be split across releases)
**Target:** 0.8.3

---

### S26  `OpFreeCoroutine` at for-loop exit
**Sources:** SAFE.md § P2-R7, COROUTINE.md § Phase 1
**Severity:** Low — memory growth; `State::coroutines` accumulates one `Box<CoroutineFrame>` per generator invocation forever.
**Description:** `coroutine_return` marks the frame `Exhausted` but never sets the slot to `None`.  The `free_coroutine(idx)` helper is designed but never called.  Programs that create many generators in a loop grow `State::coroutines` without bound.
**Fix path:**
1. In the `for … in gen { }` desugaring codegen, emit `OpFreeCoroutine(gen_slot)` at loop exit (both exhaustion and `break`).
2. Implement `OpFreeCoroutine` in `fill.rs`: call `free_coroutine(idx)` which sets `coroutines[idx] = None`.
3. Optionally, lazily free in `coroutine_exhausted` when it first observes `Exhausted` status (covers the `explicit-advance` API path).
**Effort:** Medium
**Target:** 0.8.3

---

### S27  Coroutine `text_positions` save/restore across yield/resume
**Sources:** SAFE.md § P2-R4
**Severity:** Medium (debug-only) — `text_positions` BTreeSet becomes inconsistent across yield/resume, causing false double-free misses and masking missing `OpFreeText` for unrelated code.
**Description:** `coroutine_yield` rewinds `stack_pos` but does not remove text-local entries from `State::text_positions`.  The orphaned entries interfere with the debug detector for unrelated text frees at the same stack positions.
**Fix path:**
1. In `coroutine_yield` (debug path): collect `text_positions` entries in `[base, locals_end)`, remove them, store in `frame.saved_text_positions: BTreeSet<u32>`.
2. In `coroutine_next` (debug path): re-insert `frame.saved_text_positions` and clear it.
3. In `coroutine_return` (debug path): clear `frame.saved_text_positions` without reinserting.
**Effort:** Small (debug-only path)
**Target:** 0.8.3

---

### S28  Debug generation-counter for stale DbRef detection in coroutines
**Sources:** SAFE.md § P2-R8, COROUTINE.md § SC-CO-2
**Severity:** Medium — a generator resuming after its backing record or store was freed silently reads/writes wrong data with no diagnostic.
**Description:** A `DbRef` live in a generator local at a `yield` point can refer to memory freed or resized by the consumer between iterations.  Worse than ordinary functions: the suspension window spans many `next()` calls.
**Fix path:**
1. Add `generation: u32` to `Store`; increment on every `claim`, `delete`, and `resize`.
2. When `coroutine_create` / `coroutine_yield` saves a `DbRef` to `stack_bytes`, also record `(store_nr, generation_at_save)` in a new `frame.store_generations: Vec<(u16, u32)>`.
3. At `coroutine_next`, verify each saved store's current generation matches; emit a runtime diagnostic on mismatch.
**Effort:** Medium
**Target:** 0.8.3

---

### S29  Parallel store hardening: `thread::scope` + LIFO assert + skip claims
**Sources:** SAFE.md § P1-R2/P1-R3/P1-R4
**Severity:** Low/Medium — three independent low-effort fixes for parallel store infrastructure.
**Description:**
- **P1-R2:** `run_parallel_direct` uses a raw `*mut u8` with a lifetime invariant enforced only by convention; `thread::spawn` + manual join does not give compile-time guarantees.
- **P1-R3:** `clone_locked` copies `self.claims` (all live record offsets) into worker clones that never call `validate()` — wasted O(records) allocation per worker.
- **P1-R4:** `free_named` relies on LIFO store freeing order; out-of-order frees stall `max` and may cause subsequent `database()` to reuse a live slot.
**Fix path:**
1. Replace `thread::spawn` + manual join in `run_parallel_direct` with `std::thread::scope` (Rust 1.63+) to give compile-time lifetime enforcement over `out_ptr`.
2. Add `clone_locked_for_worker` on `Store` that omits `claims: HashSet::new()`; use it in `Stores::clone_for_worker`.
3. Add `debug_assert!(store_nr == self.max - 1, "free() must be called in LIFO order")` in `free_named`.
**Effort:** Small (three independent one-function changes)
**Target:** 0.8.3

---

### S30  `WorkerStores` newtype for type-level non-aliasing
**Sources:** SAFE.md § P1-R5
**Severity:** Low — no current bug; guards against future extensions to the parallel dispatch that could silently allow workers to hold main-thread `DbRef` values.
**Description:** The architecture relies on convention (workers receive cloned stores and may not hold main-thread `DbRef`s) rather than Rust types.  A future refactor extending worker dispatch could silently break the invariant.
**Fix path:**
1. Introduce `WorkerStores(Stores)` newtype, constructible only by `clone_for_worker` (private inner field).
2. Worker closures receive `WorkerStores`; the type is `Send` but not `Sync`, preventing cross-thread sharing.
3. Long-term: add `origin: StoreOrigin` tag to `DbRef` and a debug assert in `copy_from_worker` that all result DbRefs have worker origin, not main-thread origin.
**Effort:** Medium
**Depends:** S29 (clean parallel store state first)
**Target:** 0.8.3

---

## N — Native Codegen

All N-tier items (N1–N9) are completed.  Native test parity achieved 2026-03-23:
all `.loft` tests pass in both interpreter and native mode.
Full design in [NATIVE.md](NATIVE.md).

---

### N8  Native codegen: extend to tuples, coroutines, and generics
**Sources:** CAVEATS.md C19, NATIVE.md, TUPLES.md, COROUTINE.md
**Severity:** Medium — programs using tuples, coroutines, or `maybe<T>` cannot be compiled with `--native`.
**Description:** The native (`--native`) code generator currently falls back to the interpreter for three feature areas (see CAVEATS.md C19): tuples, coroutines, and generic/maybe types.  Each area is split into independently shippable sub-items below.

---

#### N8a.1 — Native: `Type::Tuple` dispatch in code generator
**Effort:** Small · **Depends:** T1
Add `Type::Tuple` to all `output_type`, `output_init`, `output_set`, and variable-declaration paths in `src/generation/`.  Until N8a.2 is done, functions that use tuples should be gracefully skipped (added to `SCRIPTS_NATIVE_SKIP`).
**Tests:** compile without errors for files that don’t use tuple operations; skip gate for `50-tuples.loft`.

#### N8a.2 — Native: tuple construction and element access
**Effort:** Small · **Depends:** N8a.1
Emit a tuple literal as consecutive scalar assignments onto the Rust stack frame.  Emit element reads (`.0`, `.1`, …) as direct field reads from the emitted Rust struct/tuple.  Emit `OpPutInt`/`OpPutText` analogs for element writes.
**Tests:** `tests/scripts/50-tuples.loft` passes in `--native` mode for construction and read sections; element assignment and deconstruction covered by sub-tests.

#### N8a.3 — Native: tuple function return (multi-value Rust struct)
**Effort:** Medium · **Depends:** N8a.2
Tuple-returning functions emit a generated Rust struct (e.g. `struct Ret_foo { f0: i64, f1: String }`) as the return type.  Caller deconstructs the struct into local slots.  LHS deconstruction (`(a, b) = foo()`) handled in the call site template.
**Tests:** `50-tuples.loft` fully passes in `--native` mode (no `SCRIPTS_NATIVE_SKIP` entry).

---

#### N8b.1 — Native: coroutine state-machine transform design
**Effort:** High · **Depends:** CO1
Design and document the Rust enum state machine that represents a suspended coroutine.  Each `yield` point becomes a variant that stores all live locals.  Write the state-machine emitter skeleton in `src/generation/`; no working coroutines yet, but the infrastructure compiles.  Document the design in NATIVE.md § N8b.
**Note:** Using `genawaiter` or `async-std` generators is an alternative; evaluate before committing to the hand-written state machine approach.

#### N8b.2 — Native: basic coroutine emission (yield/resume cycle)
**Effort:** High · **Depends:** N8b.1
Emit `OpCoroutineCreate`, `OpCoroutineNext`, `OpYield`, and `OpCoroutineReturn` using the state machine from N8b.1.  Cover coroutines with integer/float/boolean yields and no text locals (text serialisation adds complexity, tackled as a follow-on).
**Tests:** `tests/scripts/51-coroutines.loft` basic sections pass in `--native`; text-yield sections remain skipped.

#### N8b.3 — Native: `yield from` delegation in native coroutine
**Effort:** Medium · **Depends:** N8b.2
Extend the state machine emitter to handle `yield from inner()` — the sub-generator loop is inlined into the outer state machine as an additional state range.  Requires careful handling of the sub-generator’s exhaustion sentinel.
**Tests:** `51-coroutines.loft` fully passes in `--native` mode (yield-from sections un-skipped).

---

#### N8c.1 — Native: audit which generic instantiations fail and why
**Effort:** Small · **Depends:** none
Generic functions are monomorphized at parse time (`try_generic_instantiation` in
`src/parser/mod.rs`); each call site produces a concrete `DefType::Function` named
`t_<len><type>_<name>` (e.g. `t_4text_identity`).  Native codegen sees only concrete
functions.  The P5 skip is because some monomorphized instantiations produce compile
errors, not because generics are unsupported at codegen level.

Audit procedure:
1. Temporarily remove `"48-generics.loft"` from `SCRIPTS_NATIVE_SKIP`.
2. Run `cargo test --test native 2>&1` and capture the exact compile errors.
3. Inspect the generated `.rs` file for the failing `t_4text_*` functions.
4. Record findings in NATIVE.md § N8c.1 before writing N8c.2.

Expected: text-returning instantiations lack the `Str::new()` return wrapping or have a text-parameter type mismatch.  Full design in NATIVE.md § N8c.
**Output:** Exact error message + root-cause note in NATIVE.md § N8c.1.

#### N8c.2 — Native: fix failing monomorphised instantiations
**Effort:** Small · **Depends:** N8c.1
Apply the fix identified in N8c.1.  If the issue is text-return wrapping: verify
`output_function()` applies the `Str::new()` path for all `Type::Text` return types
including `t_*` functions.  If parameter type: fix the call-site argument emission for
text arguments in monomorphized calls.  Remove `"48-generics.loft"` from
`SCRIPTS_NATIVE_SKIP`; confirm `cargo test --test native` passes.
**Tests:** `48-generics.loft` passes in `--native` mode; all four identity instantiations
(integer, float, text, boolean) and both pick_second instantiations produce correct output.

---

**Overall effort:** N8a Small+Small+Medium; N8b High+High+Medium; N8c Small+Small
**Depends:** T1 (N8a), CO1 (N8b)
**Target:** 0.8.3

---

### S31  Native harness: pass `--extern` for optional feature deps
**Sources:** CAVEATS.md C27
**Severity:** Medium — `rand`, `rand_seed`, `rand_indices` and any future optional-dep functions are silently untested in native mode.
**Description:** The native test harness in `tests/native.rs` compiles generated `.rs` files by invoking `rustc` directly with `--extern loft=libloft.rlib`.  Optional feature dependencies (`rand_core`, `rand_pcg`) are not passed as `--extern` flags, so any generated code that uses the `random` feature fails to compile with `E0433: use of undeclared crate or module 'rand_core'`.  `15-random.loft` and `21-random.loft` are therefore in `SCRIPTS_NATIVE_SKIP` / `NATIVE_SKIP`.

**Fix path:**
1. In `find_loft_rlib()` (`tests/native.rs`), after locating the `deps/` directory, scan it for `.rlib` files matching the optional deps listed in `Cargo.toml` (`rand_core`, `rand_pcg`, `png`, etc.).
2. Build a `Vec<(String, PathBuf)>` of `(crate_name, rlib_path)` pairs.
3. Pass each as an additional `--extern <crate_name>=<path>` argument in the `rustc` invocations inside `run_native_test`.
4. Remove `"15-random.loft"` from `SCRIPTS_NATIVE_SKIP` and `"21-random.loft"` from `NATIVE_SKIP`.
5. Confirm `cargo test --test native` passes for both random files.

**Tests:** `15-random.loft` and `21-random.loft` pass in native mode.
**Effort:** Small
**Target:** 0.8.3

---

### S33  Native: fix `14-image.loft` PNG width=0 in CI
**Sources:** CAVEATS.md C29
**Severity:** Low — PNG functionality is covered by the interpreter tests; only the native CI path is uncovered.
**Description:** The native binary for `tests/docs/14-image.loft` panics in CI (Ubuntu, macOS, Windows) with `width=0`.  Passes locally.  `stores.get_png()` is called with the relative path `"tests/example/map.png"` but silently leaves width=0, suggesting either a working-directory mismatch in CI or a codegen issue where the `get_png` return value is not handled correctly in native mode.

**Fix path:**
1. Print the working directory inside the compiled binary to verify it matches the repo root when run by the native test harness.
2. Check whether `stores.get_png()` returns an error code that the interpreter checks but native codegen ignores (look for a mismatch between the bytecode `get_png` call and the native emission in `dispatch.rs`).
3. Fix the root cause (cwd, ignored return, or path mismatch) and remove `"14-image.loft"` from `NATIVE_SKIP`.
4. Confirm `cargo test --test native native_dir` passes in CI.

**Tests:** `14-image.loft` passes in `native_dir` in CI on all platforms.
**Effort:** Small
**Target:** 0.8.3

---

### S32  Fix slot conflict in `20-binary.loft` (`rv` / `_read_34`)
**Sources:** CAVEATS.md C28
**Severity:** Medium — a binary file I/O test is excluded from both interpreter and native CI.
**Description:** The two-zone slot allocator assigns overlapping slots `[820, 832)` to both `rv` (live `[1016, 1110]`) and `_read_34` (live `[1008, 1109]`) in `n_main` of `tests/scripts/20-binary.loft`.  The live ranges overlap, so the slot validator panics in debug builds.  `20-binary.loft` is in `ignored_scripts()` (wrap), `SCRIPTS_NATIVE_SKIP` (native scripts), and the `binary` test is `#[ignore]`.

**Fix path:**
1. Run `LOFT_LOG=variables cargo test --test wrap binary 2>&1` to dump the full variable table for `n_main`.
2. Identify why `rv` and `_read_34` are assigned the same slot despite overlapping live ranges.  Likely cause: one is a short-lived `_read_*` temp in an inner scope that the zone-2 allocator reuses too aggressively when another variable with a long live range occupies the same zone-2 slot.
3. Apply the minimal fix to the zone-2 reuse logic in `src/variables/slots.rs` to prevent the overlap.
4. Remove `"20-binary.loft"` from `ignored_scripts()` and `SCRIPTS_NATIVE_SKIP`; remove the manual `#[ignore]` from the `binary` test; re-enable.
5. Run `make ci` to confirm no regressions.

**Tests:** `binary` and `loft_suite` (wrap) pass; `20-binary.loft` passes in native mode.
**Effort:** Medium
**Target:** 0.8.3

---

### S34  Interpreter: `20-binary.loft` `pos >= TOS` assertion at codegen.rs:751
**Sources:** `tests/scripts/20-binary.loft`, `wrap::binary` `#[ignore]`, `src/state/codegen.rs:751`
**Severity:** Medium — the interpreter test for `20-binary.loft` has been excluded since S32 only fixed the native path.
**Description:** `generate_set` (codegen.rs) has two branches for first variable allocation:
- `pos == stack.position` → slot is at TOS; place directly.
- `else` → slot is below TOS; reuse dead slot via `OpPutX`. The `debug_assert!(pos < stack.position)` guards this path.

S32 added `has_sibling_overlap` to `adjust_first_assignment_slot`: when a same-scope sibling overlaps the range `[stack.position, stack.position + v_size)`, the downward adjustment is skipped and `pos` retains its pre-assigned value (which is **above** TOS, i.e. `pos > stack.position`). When `generate_set` then evaluates `pos == stack.position` → false, it falls into the `else` branch and the `debug_assert!(pos < stack.position)` fires because `pos > stack.position`.

**Root cause:** The `has_sibling_overlap` check in `adjust_first_assignment_slot` is too conservative for the `pos > stack.position` case. Siblings detected at `[stack.position, ...)` are at TOS or above TOS — exactly where the new variable should go. Blocking the assignment leaves `pos` in an invalid state (`pos > stack.position`) that no branch in `generate_set` handles.

**Fix path:**

*Option A — Short-term: handle `pos > stack.position` in `generate_set`.*

Add a third case between the two existing branches:
```rust
if pos == stack.position {
    // ... existing at-TOS path ...
} else if pos > stack.position {
    // Slot was pre-assigned above TOS but adjust_first_assignment_slot
    // could not move it down due to sibling overlap.
    // Treat as at-TOS: reset the slot to current TOS and place directly.
    stack.function.set_stack_pos(v, stack.position);
    // fall through to at-TOS placement
    self.gen_set_first_at_tos(stack, v, value);
} else {
    debug_assert!(pos < stack.position);
    // ... existing below-TOS reuse path ...
}
```

*Option B — Proper fix: correct `adjust_first_assignment_slot`.*

In the `pos > stack.position` branch of `adjust_first_assignment_slot`, the sibling overlap check should only block the move when siblings occupy slots **below** current TOS (i.e. `js < stack.position`). Siblings at or above TOS do not have existing data to protect. Revise the predicate:
```rust
let has_sibling_overlap = (0..stack.function.count()).any(|j| {
    // Only block if sibling is already allocated BELOW current TOS.
    // Siblings at TOS or above also need space; don't block on them.
    let js = stack.function.stack(j);
    js < stack.position   // sibling is below TOS — real data exists there
    && js + size(...) > stack.position  // its range overlaps TOS
    && /* live range overlap check */ ...
});
if !has_sibling_overlap {
    stack.function.set_stack_pos(v, stack.position);
}
// If has_sibling_overlap: both the new variable and the sibling need TOS;
// assign new variable to TOS + sibling_size (bump past the sibling).
else {
    let next_free = /* find first slot >= stack.position not occupied by a sibling */;
    stack.function.set_stack_pos(v, next_free);
}
```

**Status:** *(completed 0.8.3)* — `skip_free` mechanism added to `src/state/codegen.rs`
and `src/variables/validate.rs`.  During `generate_set` Option A (pos > TOS → alias to TOS
slot), the inner variable `_read_34` is marked `skip_free`.  `generate_call` suppresses
`OpFreeRef` emission for `skip_free` variables, eliminating the double-free.
`validate_slots` skips conflict checks where either variable is `skip_free`.  The
`skip_free` flags are propagated from the codegen-time `stack.function` into
`data.definitions[def_nr].variables` after all `Data` mutations complete.

`wrap::binary` now passes.  `"20-binary.loft"` removed from `ignored_scripts()`.

**Side effect:** Fixing S34's interpreter panic exposed a pre-existing native codegen
bug for the same IR pattern (tracked as S35).  `"20-binary.loft"` added to
`SCRIPTS_NATIVE_SKIP` as a result.

**Tests:** `cargo test --test wrap binary` — passes.
**Effort:** Medium
**Target:** 0.8.3

---

### S35  Native: Insert-return pattern emits malformed Rust
**Sources:** `tests/native.rs` `SCRIPTS_NATIVE_SKIP`, `tests/scripts/20-binary.loft`
**Severity:** Medium — the native codegen path for `20-binary.loft` has been excluded
since S34's interpreter fix exposed it.
**Description:** The native code generator (`src/generation/`) emits malformed Rust for
the IR pattern `Set(rv, Insert([Set(_read_34, Null), Block]))`.  This is a block-return
pattern where the return value `rv` is assigned the result of an `Insert` that contains
a nested `Set`.  The emitted Rust looks like:

```rust
let mut var_rv: DbRef =   let mut var__read_34: DbRef = DbRef::null();
```

The inner `Set(_read_34, Null)` is being emitted inline as a declaration rather than
as a separate statement before the `Insert` call, producing a declaration in the middle
of an expression context.

**Root cause (confirmed):** `output_set` in `src/generation/dispatch.rs` handles
`Value::Set(var, to)` by writing `let mut var_{name}: type = ` and then calling
`output_code_inner(w, to)` for the RHS.  When `to` is `Value::Insert(ops)`, the
`Value::Insert` arm in `output_code_inner` (emit.rs:52–63) iterates over `ops` and
emits each one indented with a trailing semicolon — treating them as statements.
This is correct at the top level but wrong inside an expression context.  The result
is a Rust declaration nested inside another Rust expression, which is a syntax error.

**Fix path (concrete — `src/generation/dispatch.rs`, `output_set`):**

Add a branch for `to = Value::Insert(ops)` before the general `output_code_inner`
call, handling it by hoisting all-but-last ops as statements then assigning the
last op's result:

```rust
// S35: Set(var, Insert([stmt1, ..., last_expr])) — hoist all-but-last ops
// as statements before the declaration, then assign from the final expression.
if let Value::Insert(ops) = to {
    // Emit prefix statements (all except the last op).
    for op in &ops[..ops.len() - 1] {
        self.indent(w)?;
        self.output_code_inner(w, op)?;
        writeln!(w, ";")?;
    }
    self.indent(w)?;
    // Now emit the declaration/assignment with only the last op as the value.
    if self.declared.contains(&var) {
        write!(w, "var_{name} = ")?;
    } else {
        self.declared.insert(var);
        let tp_str = rust_type(variables.tp(var), &Context::Variable);
        write!(w, "let mut var_{name}: {tp_str} = ")?;
    }
    self.output_code_inner(w, &ops[ops.len() - 1])?;
    return Ok(());
}
```

This branch is added after the `Value::Block` pre-declaration handling (line ~73) and
before the general `declared.contains` check (line ~85).

**Tests:** Remove `"20-binary.loft"` from `SCRIPTS_NATIVE_SKIP` in `tests/native.rs`
once fixed.
**Effort:** Medium
**Target:** 0.8.3

---

### O1  Superinstruction merging
**Status: deferred indefinitely — opcode table is full (254/256 used)**
**Sources:** PERFORMANCE.md § P1
**Description:** Peephole pass in `src/compile.rs` merges common 4-opcode sequences (var/var/op/put) into single opcodes.  Originally targeted the 16 "free" slots above opcode 240, but those slots are now taken (T1.8b `OpPutText` + prior additions).  With 254/256 opcodes used, no slots remain for superinstructions without a redesign of the opcode space (e.g. a two-byte opcode escape or a dedicated superinstruction table).
**Expected gain:** 2–4× on tight integer loops — the gain remains attractive but the prerequisite work (opcode-space redesign) is High effort and blocks everything else.
**Effort:** Medium for the peephole pass itself; High to first free up opcode slots.
**Target:** 1.1+

---

### O2  Stack raw pointer cache
**Sources:** PERFORMANCE.md § P2
**Description:** Every `get_stack`/`put_stack` call resolves `database.store(&stack_cur)` then computes a raw pointer from `rec + pos`. Adding `stack_base: *mut u8` to `State` that is refreshed once per function call/return eliminates this lookup on every arithmetic push/pop, reducing the hot path to a single pointer add.
**Expected gain:** 20–50% across all interpreter benchmarks.

**Fix path:**

*Step 1 — Add `stack_base: *mut u8` and `stack_dirty: bool` to `State`.*

*Step 2 — Add `refresh_stack_ptr()`:*
```rust
fn refresh_stack_ptr(&mut self) {
    self.stack_base = self.database
        .store_mut(&self.stack_cur)
        .record_ptr_mut(self.stack_cur.rec, self.stack_cur.pos);
}
```
Call after `fn_call`, `op_return`, and any op that sets `stack_dirty = true`.

*Step 3 — Rewrite `get_stack` / `put_stack` as pointer arithmetic:*
```rust
pub fn get_stack<T: Copy>(&mut self) -> T {
    self.stack_pos -= size_of::<T>() as u32;
    unsafe { *(self.stack_base.add(self.stack_pos as usize) as *const T) }
}
pub fn put_stack<T>(&mut self, val: T) {
    unsafe { *(self.stack_base.add(self.stack_pos as usize) as *mut T) = val; }
    self.stack_pos += size_of::<T>() as u32;
}
```

*Step 4 — Mark allocation ops as dirty.*
In `fill.rs`, ops that allocate new records (`OpDatabase`, `OpNewRecord`, `OpInsertVector`, `OpAppendCopy`) set `self.stack_dirty = true`. The dispatch loop checks `stack_dirty` once per iteration and calls `refresh_stack_ptr()`.

*Step 5 — Benchmark and verify.* Run `bench/run_bench.sh` before/after. Target: ≥20% gain on benchmark 01.

**Safety invariant:** `stack_base` is valid only while no allocation modifies `stack_cur`'s backing store. Collection ops use separate stores, so the invariant holds between `refresh_stack_ptr` calls as long as `stack_dirty` is set by any store-mutating op.

**Effort:** High (`src/state/mod.rs`, `src/fill.rs`)
**Target:** 1.1+

---

**Target:** 0.8.2

---

### O4  Native: direct-emit local collections
**Sources:** PERFORMANCE.md § N1
**Description:** All vector/hash access in generated Rust currently goes through `codegen_runtime` helpers that take `stores: &mut Stores` and decode `DbRef` pointers. For a local `vector<integer>` used only within one function, the correct Rust type is `Vec<i32>` — no stores, no DbRef, no bounds-check overhead.
**Expected gain:** 5–15× on data-structure benchmarks (word frequency 16×, dot product 12×, insertion sort 7×).

**Fix path:**

*Step 1 — Escape analysis pass (`src/generation/escape.rs`, new).*
Before native codegen runs per function, classify each local variable:
- `Local` — declared in this function, never passed by `&ref` to another function, never assigned to a struct field.
- `Escaping` — passed by reference, stored in a field, or returned.
Conservative: any uncertain case is `Escaping`.

*Step 2 — Direct-emit type mapping.*
For `Local` variables of collection type, emit Rust native types:
`vector<integer>` → `Vec<i32>`, `vector<float>` → `Vec<f64>`, `index<text, T>` → `HashMap<String, T>`.
Declaration site: `let mut var_counts: Vec<i32> = Vec::new();` instead of `let mut var_counts: DbRef = stores.null();`.

*Step 3 — Direct-emit operation mapping.*
In `output_code_inner`, when the target variable is `Local`, bypass `codegen_runtime`:
`v[i]` → `v[i as usize]`, `v.length` → `v.len() as i32`, `v.append(x)` → `v.push(x)`, `v.sort()` → `v.sort()`.
For `Escaping` variables, the existing `codegen_runtime` path is unchanged.

*Step 4 — Drop is automatic.*
`Local` `Vec`/`HashMap` values drop at end of scope via RAII — no `OpFreeRef` emission needed.

*Step 5 — Verify.*
All 10 native benchmarks pass; `native_dir` and `native_scripts` test suites pass. New assertion: generated Rust for a known `Local` vector contains `Vec<` not `DbRef`.

**Effort:** High (`src/generation/escape.rs` new, `src/generation/emit.rs`, `src/generation/mod.rs`)
**Target:** 1.1+

---

### O5  Native: omit `stores` param from pure functions
**Sources:** PERFORMANCE.md § N2
**Description:** Every generated function currently receives `stores: &mut Stores` even when it never touches a store. For recursive functions like Fibonacci, `rustc -O` cannot eliminate this parameter across recursive calls, adding a register save/restore pair per call (measured: 1.84× slower than hand-written Rust). Purity analysis emits a `_pure` variant without `stores`; the wrapper delegates to it.
**Expected gain:** 10–30% on recursive compute benchmarks.
**Depends:** O4

**Fix path:**

*Step 1 — Purity analysis (`src/generation/purity.rs`, new).*
Recursively scan `def.code: Value`. A function is **pure** if its IR contains none of:
`Value::Ref`, `Value::Store`, `Value::Format`, `Value::Call` to any op with `stores` in its `#rust` body.
Memoize per `def_nr` to avoid exponential recursion on call graphs.

*Step 2 — Emit `_pure` variant.*
For each pure function, emit two Rust functions:
```rust
fn n_fibonacci_pure(n: i32) -> i32 {   // no stores parameter
    if n <= 1 { return n; }
    n_fibonacci_pure(n - 1) + n_fibonacci_pure(n - 2)
}
fn n_fibonacci(stores: &mut Stores, n: i32) -> i32 {  // wrapper for uniform call interface
    n_fibonacci_pure(n)
}
```

*Step 3 — Call-site dispatch.*
In `output_call`, when emitting a call from a pure context to a pure callee, emit `n_foo_pure(…)` directly, omitting `stores`. This allows `rustc` to inline and tail-call-optimise freely.

*Step 4 — Verify.*
`n_fibonacci_pure` appears in generated Rust for any recursive integer function. All native benchmarks pass.

**Effort:** High (`src/generation/purity.rs` new, `src/generation/emit.rs`, `src/generation/mod.rs`)
**Target:** 1.1+

---

---

### O7  WASM: pre-allocate format-string buffers in native/wasm codegen *(completed 0.8.3)*
**Sources:** PERFORMANCE.md § W1 (Design: W1 — wasm string representation)
**Expected gain:** Reduces wasm/native string-building gap from 2.06× to <1.3× on benchmark 07.
**Background:** Each format string in loft generates a sequence of bytecodes:
1. `OpClearStackText` — resets the work-text variable to `""`
2. N × `Op*Format*` calls — append each segment and value
3. The completed string is used (moved or assigned)

In native/wasm codegen, `OpClearStackText` emits `var_x.clear()` (`src/generation/text.rs::clear_stack_text`).  Each subsequent `OpAppendText` emits `var_x += &*(expr)`, which calls `String::push_str` internally and triggers a reallocation whenever capacity is exceeded.  In the wasm linear-memory allocator each reallocation requires a potential `memory.grow`, making repeated small appends disproportionately slow.

**Fix path:**

**Step 1 — Profile (verify root cause):**
Run `bench/run_bench.sh` targeting benchmark 07 with wasm build and capture a `wasmtime --profile` trace.  Confirm that `String` reallocations (calls to `wasm_bindgen::__wbindgen_malloc` or equivalent) account for the majority of the gap.  If the gap is from function-call overhead instead, revisit the approach.

**Step 2 — Count format operations at codegen time:**
In `src/generation/` the `Output` struct processes bytecodes in order.  Add a pre-scan function `count_format_ops(ops: &[Op]) -> usize` that, for a sequence starting with `OpClearStackText`, counts consecutive `Op*Format*` operations until the next non-format op.  This count is the static upper bound for the number of append calls.

**Step 3 — Emit `with_capacity` in `clear_stack_text`:**
Modify `src/generation/text.rs::clear_stack_text` to accept the pre-scanned count `n`:
```rust
// Before:
write!(w, "var_{s_nr}.clear()")?;

// After (when n > 1):
// avg_element_len = 8 is a conservative estimate for mixed text/integer fields
write!(w, "{{ let _cap = {n} * 8usize; if var_{s_nr}.capacity() < _cap {{ var_{s_nr} = String::with_capacity(_cap); }} else {{ var_{s_nr}.clear(); }} }}")?;
```
Use `with_capacity` only for format strings with 2+ segments; single-segment strings (just `clear()`) are unaffected.

**Step 4 — Verify `append_text` uses `push_str`:**
Confirm line 87 in `text.rs` emits `var_{s_nr} += &*(expr)`.  Rust’s `AddAssign<&str>` on `String` calls `push_str` internally so no allocation is triggered when capacity is sufficient.  No change needed here.

**Step 5 — Feature-gate (optional):**
The `with_capacity` change benefits both native and wasm builds (reducing allocations in both).  No feature gate required.  If profiling shows native is unaffected, gate behind `#[cfg(feature = "wasm")]` to keep the emitted code simple.

**Step 6 — Benchmark and verify:**
Re-run benchmark 07 wasm vs native.  Target: gap < 1.3×.  If gap persists, increase `avg_element_len` or apply the capacity hint to `OpClearText` paths as well.

**Files changed:** `src/generation/text.rs` (10–20 lines), `src/generation/dispatch.rs` (pass count to `clear_stack_text`).

**Effort:** Medium
**Depends:** W1 (W1.9 — WASM entry point; needed to test the wasm build)
**Target:** 0.8.3

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
**Sources:** [WEB_SERVICES.md](WEB_SERVICES.md) § Approach B; CODE.md § Dependencies
**Description:** Add a new stdlib module `default/04_web.loft` with JSON field-extraction
functions.  Functions extract a single typed value from a JSON object body supplied as
a `text` string.  No `serde_json` dependency — the existing parsing primitives in
`src/database/structures.rs` are sufficient; a new `src/database/json.rs` module adds
schema-free navigation on top.
**Fix path:**

**Step 1 — Cargo dependency** (`Cargo.toml`):
Add only `ureq` (used in H4) under a new `http` optional feature.  No `serde_json`.
```toml
[features]
http = ["ureq"]

[dependencies]
ureq = { version = "2", optional = true }
```

**Step 2 — `src/database/json.rs`** (new file, ~80 lines, no new dependency):
Add as a submodule of `src/database/`.  Provides three `pub(crate)` building blocks:

```rust
// Find `key` in a top-level JSON object; return raw value slice (unallocated).
pub(crate) fn json_get_raw<'a>(text: &'a str, key: &str) -> Option<&'a str>

// Return raw JSON text for each element of a top-level JSON array.
pub(crate) fn json_array_items(text: &str) -> Vec<String>

// Parse a raw value slice into a Rust primitive (loft null sentinels on failure):
pub(crate) fn as_text(raw: &str) -> String   // strips quotes + handles \n \t \\
pub(crate) fn as_int(raw: &str) -> i32       // i32::MIN on failure
pub(crate) fn as_long(raw: &str) -> i64      // i64::MIN on failure
pub(crate) fn as_float(raw: &str) -> f64     // f64::NAN on failure
pub(crate) fn as_bool(raw: &str) -> bool     // false on failure
```

Internally `json.rs` uses its own `skip_ws`, `skip_value`, and `extract_string` helpers
(~50 lines combined).  These mirror the primitives in `structures.rs` but operate
schema-free: no `Stores`, no `DbRef`, no type lookup.  The byte-scanning logic is
identical in style to the existing `match_text` / `skip_float` functions.

*Design note:* The primitives in `structures.rs` (`match_text`, `match_integer`, etc.)
are `fn` (module-private) because they are only called by `parsing()` within the same
module.  Rather than widening their visibility, `json.rs` keeps its own small copies
to preserve the clean boundary between schema-driven and schema-free parsing.

**Step 3 — Loft declarations** (`default/04_web.loft`):
```loft
// Extract primitive values from a JSON object body.
// Returns zero/empty/null-sentinel if the key is absent or type does not match.
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

**Step 4 — Rust implementation** (new `src/native_http.rs`, registered in `src/native.rs`):
Each native function calls `json::json_get_raw` then the appropriate `as_*` converter.
All functions return the loft null sentinel (or empty string) on any error — never panic.
- `json_text`: `json_get_raw(body, key).map(as_text).unwrap_or_default()`
- `json_int`: `json_get_raw(body, key).map(as_int).unwrap_or(i32::MIN)`
- `json_long`: `json_get_raw(body, key).map(as_long).unwrap_or(i64::MIN)`
- `json_float`: `json_get_raw(body, key).map(as_float).unwrap_or(f64::NAN)`
- `json_bool`: `json_get_raw(body, key).map(as_bool).unwrap_or(false)`
- `json_items`: `json_array_items(body)` → build a `vector<text>` via `stores.text_vector`
- `json_nested`: `json_get_raw(body, key).unwrap_or_default().to_string()`

**Step 5 — Feature gate** (`src/native.rs` or `src/main.rs`):
Register the H2 natives only when compiled with `--features http`.  Without the feature,
calling any `json_*` function raises a compile-time error:
`"json_text requires the 'http' Cargo feature"`.

*Tests:*
- Valid JSON object: each extractor returns the correct value.
- Missing key: returns zero/empty/null-sentinel without panic.
- Invalid JSON body: returns zero/empty/null-sentinel without panic.
- Nested object value: `json_nested` returns a string parseable by `json_int` etc.
- `json_items` on a 3-element array returns a `vector<text>` of length 3.
- Unicode and `\"` escapes inside string values are handled correctly.

**Effort:** Small–Medium (new `json.rs` ~80 lines + 7 native functions; no new dependency)
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
// Mirror the File read interface so HTTP sources are interchangeable with
// file sources in any function that processes text.
pub fn content(self: HttpResponse) -> text {
    self.body
}
pub fn lines(self: HttpResponse) -> vector<text> {
    self.body.split('\n')  // strips \r so CRLF bodies match LF bodies
}
```
No `#rust` needed; all three methods are plain loft.  `lines()` uses the same
CRLF-stripping logic as `File.lines()` — HTTP/1.1 bodies frequently use CRLF.

**Optical similarity with `File`:** the shared method names let processing
functions accept either source without modification:
```loft
fn process(rows: vector<text>) { ... }
process(file("local/data.txt").lines());
process(http_get("https://example.com/data").lines());
```

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

### R1  Add `cdylib` + `rlib` crate types for WASM compilation
**Sources:** WASM.md § Step 1, W1.1
**Description:** The loft interpreter must be compiled as a `cdylib` (dynamic library) to produce a `.wasm` file via `wasm-bindgen`, and as an `rlib` so the existing native tests and `cargo test` continue to work against the library API.  No workspace split is required for 0.8.3 — the binary targets (`[[bin]] loft`, `[[bin]] gendoc`) are separate compilation units and will not be included in the `cdylib` output.

**Fix path:**

**Step 1 — Add `[lib]` section to `Cargo.toml`:**
```toml
[lib]
name = "loft"
crate-type = ["cdylib", "rlib"]
```
If a `[lib]` section already exists, just add the `crate-type` line.

**Step 2 — Add `src/lib.rs` if not present:**
`src/lib.rs` should already exist and re-export the public API (`pub mod parser`, `pub mod compile`, `pub mod state`, etc.).  Verify it compiles cleanly as a library target with `cargo build --lib`.

**Step 3 — Verify no `main.rs` symbols leak into the `cdylib`:**
`cargo check --target wasm32-unknown-unknown --features wasm --no-default-features` must pass.  Any use of `std::process::exit`, `std::env::args`, or `dirs::home_dir` in `src/lib.rs`-reachable modules must be feature-gated (done in W1.3–W1.6).

**Step 4 — Deferred workspace split (post-1.0):**
A full workspace split into `loft-core / loft-cli / loft-gendoc` reduces incremental build times and isolates CLI from the library API.  This is deferred until the Web IDE (W2+) makes it necessary.  The current single-crate layout is sufficient for 0.8.3.

**Verify:** `cargo check` ✔  `cargo test` ✔  `cargo check --target wasm32-unknown-unknown --features wasm --no-default-features` ✔

**Effort:** Small (one `Cargo.toml` change; no logic changes)
**Depends on:** repo creation (done)
**Target:** 0.8.3

---

## W — Web IDE

A fully serverless, single-origin HTML application that lets users write, run, and
explore Loft programs in a browser without installing anything.  The existing Rust
interpreter is compiled to WebAssembly via a new `wasm` Cargo feature; the IDE shell
is plain ES-module JavaScript with no required build step after the WASM is compiled
once.  Full design in [WEB_IDE.md](WEB_IDE.md).

---

### W1  WASM Foundation *(W1.1–W1.13 all completed 0.8.3)*
**Sources:** [WASM.md](WASM.md) — full design and 14-step implementation plan
**Severity/Value:** High — nothing else in Tier W is possible without this
**Description:** Compile the loft interpreter itself as a WASM module
(`wasm32-unknown-unknown` + `wasm-bindgen`) so it can run in browsers and Node.js.
This is distinct from the existing `--native-wasm` flag (which compiles *loft programs* to WASM).
The WASM module exposes `compile_and_run([{name, content}])` returning
`{output, diagnostics, success}`. The JS host provides filesystem, random, time,
env, and log operations through `globalThis.loftHost`.

**Steps W1.1–W1.9 (Rust):** all behind `#[cfg(feature = "wasm")]`, verifiable with
`cargo check --features wasm --no-default-features` + `cargo test` (native green):
1. **W1.1** `Cargo.toml`: `wasm`/`threading`/`wasm-threads` features + optional deps (`wasm-bindgen`, `serde`, `web-sys`); `crate-type = ["cdylib","rlib"]`
2. **W1.2** `src/fill.rs`: `print()` writes to thread-local buffer under `wasm`, real `print!()` otherwise
3. **W1.3** `src/parallel.rs`: `run_parallel_*` gated on `threading`; sequential fallback when `not(threading)`; `tests/threading.rs` guarded by `#![cfg(feature = "threading")]`
4. **W1.4** `src/logger.rs`: file I/O, rotation, `Instant`/`SystemTime` gated on `not(wasm)`; WASM calls `crate::wasm::host_log_write()`
5. **W1.5** `src/ops.rs`: random functions already gated on `random`; WASM branch calls `host_random_int`/`host_random_seed` when `wasm` and `not(random)`
6. **W1.6** `src/native.rs` + `src/database/format.rs`: `SystemTime`, `std::env`, `dirs` gated; WASM stubs call `time_now`, `time_ticks`, `env_variable`, `arguments`, path bridges
7. **W1.7** `src/state/io.rs`: every `std::fs` call gated on `not(wasm)`; WASM branches call `fs_exists`, `fs_read_text`, `fs_write_text`, `fs_read_binary`, `fs_write_binary`, `fs_delete`, `fs_move`, `fs_mkdir`, `fs_mkdir_all`, `fs_list_dir`, `fs_seek`, `fs_read_bytes`, `fs_write_bytes`, `fs_get_cursor`
8. **W1.8** `src/png_store.rs`: extract `decode_into_store<R: Read>()`; WASM reads bytes via `host_read_binary` + `Cursor<Vec<u8>>`
9. **W1.9** `src/wasm.rs`: implement `#[wasm_bindgen] fn compile_and_run(files_js: JsValue) -> JsValue`; wire parse → scope → codegen → execute → return result

**Step W1.10 (JavaScript):** completed 0.8.3:
10. **W1.10** `tests/wasm/virt-fs.mjs`: full VirtFS class (path resolution, text/binary, cursors, snapshot/restore, JSON roundtrip); `harness.mjs` + `virt-fs.test.mjs` — all 13 unit tests pass under Node.js

**Step W1.11 (JavaScript):** completed 0.8.3:
11. **W1.11** `tests/wasm/host.mjs`: `createHost(tree, options)` wiring VirtFS to `loftHost`; deterministic xoshiro128** PRNG; `bridge.test.mjs` (7 tests, skips if no pkg), `file-io.test.mjs` (14 host-level tests, no WASM needed), `random.test.mjs` (host + optional WASM level)

**Step W1.12 (JavaScript):** completed 0.8.3:
12. **W1.12** `tests/wasm/layered-fs.mjs`: `LayeredFS extends VirtFS` (base + delta overlay); `ide/scripts/build-base-fs.js` generates `ide/assets/base-fs.json`; 20 unit tests in `layered-fs.test.mjs`

**Step W1.13 (JavaScript):** completed 0.8.3:
12. **W1.12** `tests/wasm/layered-fs.mjs`: `LayeredFS extends VirtFS` (base + delta overlay, persistence); `ide/scripts/build-base-fs.js` generates `ide/assets/base-fs.json`
13. **W1.13** `tests/wasm/suite.mjs`: discovers all `fn main()` loft files in `tests/scripts/` and `tests/docs/`; builds a VirtFS pre-populated with fixtures; runs each through WASM; compares output against `cargo run` native reference; skips non-deterministic tests (time, unseeded random, image); exits non-zero on failure

**Host bridge API** (JS → Rust): `fs_*`, `random_*`, `time_*`, `env_*`, `log_*` functions
on `globalThis.loftHost`. Full spec in [WASM.md](WASM.md) § Host Bridge API.

**Effort:** High (13 steps; W1.1–W1.8 are individually small; W1.9–W1.13 are medium)
**Depends on:** R1
**Target:** 0.8.3

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
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark results and implementation designs for O1–O7 (interpreter and native performance improvements)
- [WEB_IDE.md](WEB_IDE.md) — Web IDE full design: architecture, JS API contract, per-milestone deliverables and tests, export ZIP layout (Tier W detail)
- [RELEASE.md](RELEASE.md) — 1.0 gate items, project structure changes, release artifacts checklist, post-1.0 versioning policy
