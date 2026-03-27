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
  - [O1–O7 — Interpreter and native performance](#o1--superinstruction-merging) *(deferred to 1.1+)*
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
- **O1** — Superinstruction peephole rewriting pass — deferred to 1.1+.
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
- O1 (superinstruction merging) — Too complex and disruptive for stability; deferred to 1.1+.
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
  advance-loop + yield forwarding.  Test `#[ignore]` pending slot-assignment fix.

  **CO1.4-fix — Slot-assignment regression in `yield from`** (C21):
  The desugared advance-loop introduces a temporary coroutine handle variable whose
  slot overlaps with the generator’s own stack frame on second resume.  Root cause:
  the loop-body slot for the `__next` temp is assigned before the generator frame
  is taken into account.  Fix requires the slot allocator to treat the coroutine
  frame as live across the entire `yield from` expansion, not just the yield site.
  **Target:** 1.1+
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

**A5.6a — Mutable capture:** A captured variable used as the target of `+= / -=`
causes `generate_set` to panic at "self-reference in SetRef target".  The closure
record’s field must be treated as an `OpVarRef`-relative write target rather than a
plain slot write.  Requires a new `SetClosureField(field_idx)` IR variant emitted
by `parse_assign` when the LHS resolves to a captured variable.

**A5.6b — Text capture:** The per-field text cleanup note in Phase 5 is not yet
implemented.  When a text variable is captured, the closure record holds a
`Type::Text` field; `get_free_vars` must emit `OpFreeRef` for that field at the
point where the closure record itself goes out of scope.  Without this, text memory
leaks in debug mode (assertion fires) and is double-freed in release.

**Effort:** Medium  
**Target:** 1.1+

**Effort:** Very High (parser.rs, state.rs, scopes.rs, store.rs)
**Depends on:** P1 (done)
**Target:** 0.8.3

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

## N — Native Codegen

All N-tier items (N1–N9) are completed.  Native test parity achieved 2026-03-23:
all `.loft` tests pass in both interpreter and native mode.
Full design in [NATIVE.md](NATIVE.md).

---

### N8  Native codegen: extend to tuples, coroutines, and generics
**Sources:** CAVEATS.md C19, NATIVE.md, PLANNING.md T1/CO1
**Severity:** Medium — programs using tuples, coroutines, or `maybe<T>` cannot be compiled with `--native`.
**Description:** The native (`--native`) code generator currently falls back to the interpreter for three feature areas:
- **Tuples:** `Type::Tuple` values: construction, element reads/writes, function return.
- **Coroutines:** `OpCoroutineCreate`, `OpCoroutineYield`, `OpCoroutineNext`, `OpCoroutineReturn` — require stack serialisation that has no direct Rust equivalent without the interpreter’s store-backed stack.
- **Generics / `maybe<T>`:** parametric types like `maybe<integer>` are partially supported but edge cases remain (null propagation paths, ref-counted text inside maybe).

**Fix path:** Each area is an independent sub-item:
- **N8a — Tuple native codegen:** Emit tuple as multiple stack variables; element access as direct offset read; function return as multiple return values via a struct.
- **N8b — Coroutine native codegen:** Requires a Rust generator or state-machine transform (e.g., via `genawaiter` or hand-written enum state machine).  High complexity; likely 1.2+.
- **N8c — Generic/maybe native codegen:** Audit null-path branches in `generate_*` for `Type::Named` with type parameters; add missing cases.

**Effort:** High (N8a Medium, N8b Very High, N8c Small)
**Depends:** T1 (for N8a), CO1 (for N8b)
**Target:** 1.1+

---

### O1  Superinstruction merging
**Status: deferred to 1.1+ — too complex and disruptive for current release stability**
**Sources:** PERFORMANCE.md § P1
**Description:** Peephole pass in `src/compile.rs` merges common 4-opcode sequences (var/var/op/put) into single opcodes 240–245. Six new entries added to the `OPERATORS` array in `src/fill.rs`. Operands encoded in the same byte count as the replaced sequence, so branch targets need no relocation.
**Expected gain:** 2–4× on tight integer loops; benefits every loop in the interpreter.
**Effort:** Medium
**Target:** 1.1+

---

### O2  Stack raw pointer cache
**Sources:** PERFORMANCE.md § P2
**Description:** Add `stack_base: *mut u8` to `State`; refresh once per function call/return; eliminate the `database.store()` lookup on every push/pop. A `stack_dirty` flag, set by allocation ops, triggers a refresh at the top of the dispatch loop.
**Expected gain:** 20–50% across all interpreter benchmarks.
**Effort:** High
**Target:** 1.1+

---

**Target:** 0.8.2

---

### O4  Native: direct-emit local collections
**Sources:** PERFORMANCE.md § N1
**Description:** Escape analysis pass marks collection variables as `Local` when they never leave the function (not ref-passed, not stored in a struct field). For `Local` variables, emit `Vec<T>` / `HashMap` directly, bypassing `codegen_runtime` helpers and `DbRef` indirection entirely.
**Expected gain:** 5–15× on data-structure benchmarks (word frequency 16×, dot product 12×, insertion sort 7×).
**Effort:** High
**Target:** 1.1+

---

### O5  Native: omit `stores` param from pure functions
**Sources:** PERFORMANCE.md § N2
**Description:** Purity analysis identifies functions whose IR contains no store reads or writes, no IO, no format ops. These emit a `_pure` variant without the `stores: &mut Stores` parameter; the outer wrapper with `stores` delegates to `_pure`. Enables `rustc -O` to inline across recursive calls.
**Expected gain:** 10–30% on recursive compute benchmarks.
**Effort:** High
**Depends:** O4
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
