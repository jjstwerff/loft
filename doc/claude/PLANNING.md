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
- [S — Stability Hardening](#s--stability-hardening)
  - [S4 — Binary I/O type coverage (Issue 59, 63)](#s4--binary-io-type-coverage)
  - [S5 — Optional `& text` panic](#s5--fix-optional--text-parameter-subtract-with-overflow-panic) *(0.8.2)*
  - [S6 — `for` loop in recursive function](#s6--fix-for-loop-in-recursive-function----too-few-parameters-panic) *(1.1+)*
  - [S7 — `string` type name diagnostic](#s7--add-diagnostic-error-for-string-type-name----should-be-text) *(0.8.2)*
  - [S8 — `key` field in hash struct](#s8--compile-time-error-when-hash-value-struct-has-field-named-key) *(0.8.2)*
- [P — Prototype Features](#p--prototype-features)
  - [T1 — Tuple types](#t1--tuple-types) *(1.1+)*
  - [CO1 — Coroutines](#co1--coroutines) *(1.1+)*
- [A — Architecture](#a--architecture)
  - [A1 — Parallel workers: extra args + value-struct + text/ref returns](#a1--parallel-workers-extra-arguments-value-struct-returns-and-textreference-returns) *(0.8.2)*
  - [A12 — Lazy work-variable initialization](#a12--lazy-work-variable-initialization) *(1.1+ backlog)*
  - [A13 — Complete two-zone slot assignment](#a13--complete-two-zone-slot-assignment-steps-8-and-10) *(1.1+)*
  - [TR1 — Stack trace introspection](#tr1--stack-trace-introspection) *(1.1+)*
- [N — Native Codegen](#n--native-codegen)
  - [N2–N7 — Native codegen bug fixes](#n2--implement-callref--function-pointer-calls-in-native-codegen) *(0.8.2 / 1.1+)*
- [O — Performance Optimisations](#o--performance-optimisations)
  - [O1–O7 — Interpreter and native performance](#o1--superinstruction-merging) *(0.8.2 / 1.1+)*
- [H — HTTP / Web Services](#h--http--web-services)
- [R — Repository](#r--repository)
- [W — Web IDE](#w--web-ide)
- [Quick Reference](#quick-reference) → [ROADMAP.md](ROADMAP.md)

---

## Version Milestones

### Version 0.8.2 — Stability, efficiency, and native codegen (in progress)

Goal: harden the interpreter, improve runtime efficiency, and ship working native code
generation.  No new language syntax.  Most items are independent and can be developed
in parallel.

**Completed in 0.8.2:**
- **L4** — Empty `[]` literal as mutable vector argument. ✓
- **L5** — `v += extra` via `&vector` ref-param. ✓
- **A13/A14/A15** *(old numbering)* — slot efficiency (can_reuse guard, skip_free flag, exhaustive inline_ref_set_in). ✓
- **S3** — Database dispatch exhaustiveness. ✓
- **S4** — Binary I/O type coverage (Issues 59, 63). ✓
- **P1** — Lambda expressions (`fn(params) -> ret { body }`). ✓
- **N9** — fill.rs auto-generation (rustfmt, six `#rust` templates, byte-exact match). ✓
- **N1** — `--native` / `--native-emit` CLI flags. ✓

**Remaining for 0.8.2:**

*Stability:*
- **S5** — Fix optional `& text` parameter subtract-with-overflow panic (Issue 89).
- **S7** — Add diagnostic error for `string` type name (Issue 82).
- **S8** — Compile-time error when hash-value struct has field named `key` (Issue 83).

*Native codegen fixes (small):*
- **N6** — Text method in format interpolation: emit `&str` not `String` (Issue 87).
- **N7** — `directory()` / `user_directory()` / `program_directory()` scratch buffer (Issue 88).

*Interpreter performance:*
- **O1** — Superinstruction merging: peephole pass, 6 merged opcodes 240–245.
- **O3** — Verify integer paths carry no `long` null-sentinel.
- **O6** — Native: `_nn` variants remove `long` null-sentinel from local arithmetic.

*Parallel execution:*
- **A1.1** — Extra context args + value-struct returns: extend `execute_at_raw`, output buffer.
- **A1.2** — Text/reference returns: dedicated result store per dispatch (depends on A1.1).

---

### Version 0.8.3 — Language syntax extensions (planned)

Goal: add all new language syntax before the feature-complete 0.9.0 milestone so that
syntax decisions can be validated and refined independently.  All items change the parser
or type system; 0.8.2 correctness work is a prerequisite.

**Lambda expressions (P1):**
- **P1.1** — Parser: ✓ completed in 0.8.2.
- **P1.2** — Compilation: ✓ completed in 0.8.2 (short-form `|x| {…}` and `||` with hint inference).
- **P1.3** — Integration: ✓ completed in 0.8.2 (`map`, `filter`, `reduce` accept inline lambdas).
- **P3** — Vector aggregates: `sum`, `min_of`, `max_of`, `any`, `all`, `count_if` (depends on P1).

**Pattern extensions (L2):**
- **L2** — Nested match patterns: field sub-patterns separated by `:` in struct arms.

**Field iteration (A10):**
- **A10.0** — Remove `fields` from `KEYWORDS` (revert L3 code change; keep identifier renames).
- **A10.1** — `Field` + `FieldValue` enum types in `default/01_code.loft`.
- **A10.2** — `ident#fields` detection in `parse_for` → `Value::FieldsOf` + `Type::FieldsOf`.
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
- **A1** — Moved to 0.8.2 (see remaining work above).

**Logging completeness:**
- **A2** — Logger remaining work: hot-reload wiring, `is_production()`/`is_debug()`, `--release` assert elision, `--debug` per-type safety logging.

**Deferred from 0.9.0:**
- A12 (lazy work-variable init) — Blocked by Issues 68–70; deferred to 1.1+.
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

**For 0.8.2 (remaining):**
1. **S5**, **S7**, **S8** — small stability fixes; independent, no dependencies
2. **O3** — integer sentinel verification; Low effort, zero risk
3. **N6**, **N7** — small native codegen fixes; independent
4. **O6** — `_nn` long arithmetic; Low effort, only touches `ops.rs` + `generation.rs`
5. **O1** — superinstruction merging; Medium effort, highest impact

**For 0.8.3 (after 0.8.2 is tagged):**
1. **P1** — lambdas; unblocks P3, A5; makes the language feel complete
2. **P3** + **L2** — aggregates and nested patterns; P3 depends on P1; batch together
3. **A10** — field iteration; independent, medium; can land in parallel with P1–P3

**For 0.8.4 (after 0.8.3 is tagged):**
1. **H1** — `#json` + `to_json`; Small, no new Rust deps; validates annotation parsing
2. **H2** — JSON primitive stdlib; Small–Medium, new `src/database/json.rs` (~80 lines, no new dep); test each extractor in isolation
3. **H3** — `from_json` scalar codegen; Medium, depends on H1 + H2; verify `Type.from_json` as fn-ref
4. **H4** — HTTP client + `HttpResponse`; Medium, adds `ureq`; test against httpbin.org or mock
5. **H5** — nested/array/enum `from_json` + integration tests; Med–High, depends on H3 + H4

**For 0.8.2 (remaining — parallel):**
6. **A1.1** — extra args + value-struct returns; Medium; extend `execute_at_raw`, add output buffer
7. **A1.2** — text/ref returns; Medium; dedicated result store; depends on A1.1

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

### L2  Nested patterns in field positions
**Sources:** [MATCH.md](MATCH.md) — L2
**Severity:** Low — field-level sub-patterns currently require nested `match` or `if` inside the arm body
**Description:** `Order { status: Paid, amount } => charge(amount)` — a field may carry a sub-pattern (`:` separator) instead of (or in addition to) a binding variable.  Sub-patterns generate additional `&&` conditions on the arm.
**Fix path:** See [MATCH.md § L2](MATCH.md) for full design.
Extend field-binding parser to detect `:`; call recursive `parse_sub_pattern(field_val, field_type)` → returns boolean `Value` added to arm conditions with `&&`.
**Effort:** Medium (parser/control.rs — recursive sub-pattern entry point)
**Target:** 0.8.3

---

### L3  `FileResult` enum — replace filesystem boolean returns

**Sources:** User request 2026-03-19; [PROBLEMS.md](PROBLEMS.md)
**Severity:** Low — file I/O failures (permission denied, wrong path type) are silently
collapsed into `false`, making error handling impossible without a second `file()` call
**Description:** All filesystem-mutating ops currently return `boolean`.  A failed
`delete()` returns `false` whether the file was absent, the path outside the project, or
a permission was denied.  Expanding this to an enum lets callers distinguish error causes
without extra queries.

**Design — `FileResult` enum** (variant index matches the stored byte):

```loft
pub enum FileResult {
  Ok,               // 0 — succeeded
  NotFound,         // 1 — path does not exist (also: path outside project)
  PermissionDenied, // 2 — OS permission denied
  IsDirectory,      // 3 — expected a file, got a directory
  NotDirectory,     // 4 — expected a directory, got a file
  Other             // 5 — any other OS error (incl. bad arguments, invalid PNG, etc.)
}
```

`AlreadyExists` was dropped: it cannot be returned by any current public API function
(`move` pre-checks with `exists(to)`, the others never create files that could conflict).
Adding an unreachable variant would mislead callers matching on the result.

**Design — Rust helper** (placed in `src/database/io.rs`, used everywhere):

```rust
fn io_result<T>(r: std::io::Result<T>) -> u8 {
    match r {
        Ok(_) => 0,
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound         => 1,
            std::io::ErrorKind::PermissionDenied => 2,
            std::io::ErrorKind::IsADirectory     => 3,
            std::io::ErrorKind::NotADirectory    => 4,
            _                                    => 5,
        },
    }
}
```

**Ops changed** (`default/02_images.loft`):

`OpGetFile`, `OpGetDir`, and `OpGetPngImage` are **excluded from scope** — their return
value is always discarded by the loft wrappers (`file()`, `files()`, `png()`), so
changing them adds Rust complexity with no benefit to callers.  They remain `boolean`.

| Op | Old return | New return | `#rust` body change |
|---|---|---|---|
| `OpGetFile` | `boolean` | unchanged | — |
| `OpGetDir` | `boolean` | unchanged | — |
| `OpGetPngImage` | `boolean` | unchanged | — |
| `OpDelete` | `boolean` | `FileResult` | `io_result(std::fs::remove_file(@path))` |
| `OpMoveFile` | `boolean` | `FileResult` | `io_result(std::fs::rename(@from, @to))` |
| `OpTruncateFile` | `boolean` | `FileResult` | — (no `#rust`) |
| `OpMkdir` | `boolean` | `FileResult` | `io_result(std::fs::create_dir(@path))` |
| `OpMkdirAll` | `boolean` | `FileResult` | `io_result(std::fs::create_dir_all(@path))` |

**Public API changed** (`default/02_images.loft`):

| Function | Old | New | Notes |
|---|---|---|---|
| `delete(path)` | `-> boolean` | `-> FileResult` | `valid_path` guard → `NotFound` |
| `move(from, to)` | `-> boolean` | `-> FileResult` | `valid_path` guards → `NotFound` |
| `mkdir(path)` | `-> boolean` | `-> FileResult` | `valid_path` guard → `NotFound` |
| `mkdir_all(path)` | `-> boolean` | `-> FileResult` | `valid_path` guard → `NotFound` |
| `set_file_size(self, n)` | `-> boolean` | `-> FileResult` | bad format/negative size → `Other` |
| `exists(path)` | `-> boolean` | unchanged | Boolean question; unaffected |
| `file(path)` | `-> File` | unchanged | `format` field already encodes state |
| `FileResult.ok()` | — | `-> boolean` | New — `self == FileResult.Ok`; preserves boolean idiom |

**`valid_path` boundary:** A path that fails `valid_path()` is inaccessible from within
the project namespace — from the caller's perspective, it does not exist.  The guard
returns `FileResult.NotFound`.  This avoids the false implication that a `chmod` or
ownership change would help.

**`set_file_size` note:** Pre-condition violations (negative size, wrong file format) are
caller errors, not OS errors, but they share the `Other` variant with unusual OS
conditions.  This is acceptable: `set_file_size` is called on a `File` value the caller
already has, so the format check is a defensive guard rather than a user-facing branch.
If distinguishing these ever matters, a dedicated `InvalidInput` variant can be added
without renumbering.

**`truncate_file` change** (`src/state/io.rs`): `put_stack(bool)` → `put_stack(u8)`;
open + set-len error mapped via `io_result`.

**Boolean conversion — `ok()` method:**
`FileResult` exposes `ok() -> boolean` so existing call sites need only append `.ok()`
rather than rewriting to an enum comparison:

```loft
pub fn ok(self: FileResult) -> boolean {
  self == FileResult.Ok
}
```

This keeps the migration mechanical and preserves the boolean idiom for callers that only
care about success vs. failure.  Callers that need the specific error reason use the enum
value directly.

**Breaking change:** Minimal.  Every existing boolean use of `delete`, `move`, `mkdir`,
`mkdir_all`, or `set_file_size` appends `.ok()`.  Tests in `11-files.loft` and
`13-file.loft` are updated as part of L3.3.

**Test migration pattern:**
```loft
// Before
assert(delete(f), "removed");
assert(!delete(f), "not there");
// After — success/failure only
assert(delete(f).ok(), "removed");
assert(!delete(f).ok(), "not there");
// After — specific error reason
assert(delete(f) == FileResult.NotFound, "not there");
```

**Fix path:**

**Phase 1 — Enum definition** (`default/02_images.loft`, `src/database/io.rs`):
Add `FileResult` enum immediately after the existing `Format` enum in
`02_images.loft`. Add `io_result<T>(r: std::io::Result<T>) -> u8` as a private
function in `src/database/io.rs`. No other changes yet; verify the project compiles.

**Phase 2 — Op signatures and Rust internals:**
- Change the five in-scope `Op*` return types (`OpDelete`, `OpMoveFile`, `OpTruncateFile`,
  `OpMkdir`, `OpMkdirAll`) from `boolean` to `FileResult` in `default/02_images.loft`.
- Update `#rust` bodies for the four annotated ops (OpDelete, OpMoveFile, OpMkdir,
  OpMkdirAll) to call `io_result(...)`.
- `src/database/io.rs`: add `io_result` helper; no changes to `fill_file`, `get_file`,
  `get_dir`, or `get_png` (those ops remain `boolean`).
- `src/state/io.rs`: change `truncate_file` to `put_stack(u8)` using `io_result`.
- `src/fill.rs`: update `delete`, `move_file`, `mkdir`, `mkdir_all` to `put_stack(u8)`
  via `io_result`.  Leave `get_file`, `get_dir`, `get_png_image` unchanged.

**Phase 3 — Public API wrappers and tests:**
- Add `ok() -> boolean` method to `FileResult` in `default/02_images.loft`.
- Rewrite `delete`, `move`, `mkdir`, `mkdir_all`, `set_file_size` in
  `default/02_images.loft` to return `FileResult`, replacing `&&`-chains with
  explicit `if` guards.
- Update all assertions in `tests/scripts/11-files.loft` and
  `tests/docs/13-file.loft`: simple success/failure checks become `.ok()` / `!.ok()`;
  checks that verify a specific failure reason use `== FileResult.<Variant>`.
- Run full test suite; verify no regressions.

**Effort:** Small (3 phases; no parser changes; all changes are mechanical)
**Target:** 0.8.3

---

## P — Prototype Features

### P1  Lambda / anonymous function expressions
**Sources:** Prototype-friendly goal; callable fn refs already complete (landed in 0.8.0)
**Severity:** Medium — without lambdas, `map` / `filter` require a named top-level function
for every single-use transform, which is verbose for prototyping
**Description:** Two syntactic forms for inline function literals:

```loft
// Long form — fully explicit types (always valid)
doubled = map(items, fn(x: integer) -> integer { x * 2 });
evens   = filter(items, fn(x: integer) -> boolean { x % 2 == 0 });

// Short form — types inferred from call-site context
doubled = map(items, |x| { x * 2 });
evens   = filter(items, |x| { x > 0 });
(a, b) = reduce(pairs, (0, 0), |acc, x| { (acc.0 + x.0, acc.1 + x.1) });

// Short form with explicit annotations (when no context is available)
transform: fn(integer) -> integer = |x: integer| -> integer { x * 2 };

// Zero-parameter short form uses the existing || token
run(|| { println("hello") });
```

Both forms produce a `Type::Function` value with the same d_nr representation as
`fn <name>`.  No closure capture is required initially (see A5 for full closures).

**Grammar additions:**
```
single ::= ...
         | 'fn' '(' [ param_list ] ')' [ '->' type ] block     // long form
         | '|' [ short_param { ',' short_param } ] '|'
               [ '->' type ] block                              // short form ≥1 param
         | '||' [ '->' type ] block                             // short form 0 params

short_param ::= lower_ident [ ':' type ]    // type optional when context supplies it
```

The `||` token already exists in the lexer; inside `parse_primary` it is re-interpreted
as a zero-parameter closure opener rather than logical-OR (which is a binary operator
and cannot appear at a primary-expression position).  A closing `|` token after the
parameter list is consumed before `->` and the body block.

**Phase 1 — Parser** ✓ completed (0.8.2): `parse_lambda` added to `parse_primary`;
`lambda_counter: u32` on `Parser` guarantees consistent `__lambda_N` naming across both
passes.  Emits `Value::Int(d_nr)` — same representation as a named fn-ref.

**Phase 2 — Short-form parser** (`src/parser/expressions.rs`):
Add a `parse_lambda_short` helper called from `parse_primary` when the current token is
`|` or `||`:
1. Consume the opening delimiter; collect `(name, Option<Type>)` pairs up to the
   closing `|`.
2. Optionally consume `->` and a return type.
3. First pass: register `__lambda_N` with placeholder types for unannotated params;
   skip the body.
4. Second pass: fill in any `None` parameter types from `hint`; error if hint is absent
   and a type is still `None`; compile the body.
5. Emit `Value::Int(d_nr as i32)`.
*Tests:* `|x| { x * 2 }` as argument to `fn(integer)->integer`; `|x, y| { x + y }`;
`|| { 0 }` for zero params; explicit annotation `|x: integer| -> integer { x }`;
error when no context and no annotation.

**Phase 3 — Hint propagation** (`src/parser/control.rs`):
Extend `parse_call` to compute the expected type of each argument position and pass it
as a hint to `parse_expression` → `parse_primary`.  When the expected type is
`Type::Function(...)`, pass it as the hint for the short-lambda parser.  No change
needed for the long form or non-lambda arguments.
*Tests:* `map(v, |x| { x * 2 })` compiles and runs; `filter(v, |x| { x > 0 })`; nested
call `map(filter(v, |x| { x > 0 }), |x| { x * 3 })`.

**Phase 4 — Compilation** (`src/state/codegen.rs`, `src/compile.rs`):
No changes expected.  Both lambda forms emit `Value::Int(d_nr)`, which is already handled
identically to named fn-refs.
*Tests:* both forms callable through a `fn(T) -> R` variable; `reduce` with a two-param
short lambda.

**Phase 5 — Integration with map / filter / reduce**:
Verify that anywhere a named `fn <name>` ref works, both lambda forms also work.
*Tests:* `map`, `filter`, `reduce` with short-form lambdas; named fn-ref alongside
short lambda in the same expression; nested lambdas.

**Effort:** Medium–High (parser.rs, compile.rs)
**Completed:** 0.8.2 (P1.1 long-form, P1.2 short-form with hint inference, P1.3 map/filter/reduce integration)

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

### T1  Tuple types
**Sources:** TUPLES.md
**Description:** Multi-value returns and stack-allocated `(A, B, C)` compound values. Enables functions to return more than one value without heap allocation. Seven implementation phases; full design in [TUPLES.md](TUPLES.md).

- **T1.1** — Type system: `Type::Tuple`, element offsets, `element_size` helpers (`src/data.rs`, `src/typedef.rs`).
- **T1.2** — Parser: type notation `(A, B)`, literal syntax, destructuring assignment (`src/parser/`).
- **T1.3** — Scope analysis: tuple variable intervals, text/ref element lifetimes (`src/scopes.rs`).
- **T1.4** — Bytecode codegen: slot allocation, element read/write opcodes (`src/state/codegen.rs`).
- **T1.5** — SC-4: Reference-tuple parameters with owned elements.
- **T1.6** — SC-8: Tuple-aware mutation guard.
- **T1.7** — SC-7: `not null` annotation for tuple integer elements.

**Effort:** Very High
**Target:** 1.1+

---

### CO1  Coroutines
**Sources:** COROUTINE.md
**Description:** Stackful `yield`, `iterator<T>` return type, and `yield from` delegation. Enables lazy sequences and producer/consumer patterns without explicit state machines. Six implementation phases; full design in [COROUTINE.md](COROUTINE.md).

- **CO1.1** — `iterator<T>` type + `CoroutineStatus` enum in `default/05_coroutine.loft`.
- **CO1.2** — `OpCoroutineCreate` + `OpCoroutineNext`: frame construction and advance.
- **CO1.3** — `OpYield`: serialise live stack to heap frame, return to caller.
- **CO1.4** — `yield from`: sub-generator delegation.
- **CO1.5** — `for item in generator`: iterator protocol integration.
- **CO1.6** — `next()` / `exhausted()` stdlib functions.

**Effort:** Very High
**Depends:** TR1
**Target:** 1.1+

---

## A — Architecture

### A1  Parallel workers: extra arguments, value-struct returns, and text/reference returns
**Sources:** [THREADING.md](THREADING.md) (deferred items)
**Description:** Three related extensions to `par(...)` parallel for-loops.
All worker state must currently live in the input vector; extra parameters, value-struct
returns larger than 8 bytes, and text/reference returns are all unsupported.
The three sub-problems share infrastructure but have different complexity.
**Fix path:**

**Phase 1 — Extra context args and value-struct returns** (`src/parser/collections.rs`, `src/parallel.rs`, `src/state/mod.rs`):

*Extra context arguments (primitives and const struct refs):*
Extend `execute_at_raw(fn_pos, arg, return_size) -> u64` to accept an extra
`extra_args: &[u64]` slice; push those values onto the call stack before the row ref
(in declaration order).  `run_parallel_raw` receives the captured extra arg values
(cloned to every worker — they are read-only constants).  The compiler emits the
extras as part of the `n_parallel_for` call.  No IR wrapper synthesis is needed for
primitive extras.  For `const Struct` extras (DbRef, 12 bytes) add an
`Option<DbRef>` context parameter alongside the row ref rather than folding it into
`u64`.
*Supported extra arg types:* `integer`, `long`, `float`, `boolean` (fit in u64);
`const Struct` (passed as `Option<DbRef>` context).  Text extras are already readable
from cloned stores via their DbRef — no special handling needed.

*Value-struct returns (no heap pointers):*
For worker return types where all fields are primitives (no `text`, no `reference`
fields), replace the `Vec<u64>` result channel with a pre-allocated
`Vec<u8>` output buffer of size `n_rows × result_byte_size`.  Divide it into
non-overlapping per-row slices; each worker writes directly via
`execute_at_struct(fn_pos, row_ref, out_slice: &mut [u8])`.  After join, interpret the
buffer as a typed vector record in the store.  The compiler checks that the return type
is "all-value" and computes `result_byte_size`.  DbRef (12 bytes) and any struct
containing text/reference fields fall through to Phase 2.

*Tests:* `par([1,2,3], fn worker, threshold)` where `worker(n: integer, t: integer) -> integer`
correctly uses `threshold`; value-struct return test where `worker(s: Score) -> Pair`
returns `Pair{lo: s.value, hi: s.value * 2}`; both marked `#[ignore]` in
`tests/threading.rs` until this phase ships.

**Phase 2 — Text/reference return types** (`src/parallel.rs`, `src/store.rs`):
Text and reference values are DbRefs pointing into a specific store.  Workers get
locked store snapshots; new allocations in a worker are invisible to the main thread
after join.  LIFO freeing makes ad-hoc store merging unsafe.

*Approach — dedicated result store:*
Before dispatch, the main thread calls `Stores::new_result_store()` which allocates a
fresh, writable store not included in the input snapshot.  `clone_for_worker` gives
each worker a reference to this result store (mutable, range-partitioned by row).
Workers write text/ref results into the result store via their local `State`'s text
allocator redirected to the result store index.  After join, `Stores::adopt_result_store(idx)`
unlocks the result store for use by the main thread; `n_parallel_for` builds the
result vector from the result-store DbRefs.  Since the result store did not exist in
any worker's input snapshot, there are no LIFO conflicts.

*Tests:* `par([1,2,3], fn label)` where `label(n: integer) -> text` returns a formatted
string; the result vector contains correct, independent text values with no dangling
pointers.

**Effort:** Med–High (parser.rs, parallel.rs, store.rs, state/mod.rs)
**Target:** 0.8.2

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

**Phase 1 — Capture analysis** (`src/scopes.rs`, `src/parser/expressions.rs`):
Walk the lambda body's IR and identify all free variables (variables referenced inside
the body that are defined in an enclosing scope).  No code generation yet.
*Tests:* static analysis correctly identifies free variables in sample lambdas; variables
defined inside the lambda are not flagged; non-capturing lambdas produce an empty set.

**Phase 2 — Closure record layout** (`src/data.rs`, `src/typedef.rs`):
For each capturing lambda, synthesise an anonymous struct type whose fields hold the
captured variables; verify field offsets and total size.
The element-size table and offset arithmetic introduced for tuples (see
[TUPLES.md](TUPLES.md) § Memory Layout) are identical for closure record fields; use
the shared helpers `element_size`, `element_offsets`, and `owned_elements` from
`data.rs` rather than duplicating the logic.  The closure record is heap-allocated
(a store record) and passed as a hidden trailing argument alongside the def-nr — it
does not use the stack-only tuple layout.
*Tests:* closure struct has the correct field count, types, and sizes; `sizeof` matches
the expected layout; a record containing a `text` capture has `owned_elements` count 1.

**Phase 3 — Capture at call site** (`src/state/codegen.rs`):
At the point where a lambda expression is evaluated, emit code to allocate a closure
record and copy the current values of the captured variables into it.  Pass the record
as a hidden trailing argument alongside the def-nr.  Copying a captured `text`
variable into the record requires a deep copy (same rule as tuple text elements —
see [TUPLES.md](TUPLES.md) § Copy Semantics).
*Tests:* captured variable has the correct value when the lambda is called immediately
after its definition; captured `text` is independent of the original after capture.

**Phase 4 — Closure body reads** (`src/state/codegen.rs`, `src/fill.rs`):
Inside the compiled lambda function, redirect reads of captured variables to load from
the closure record argument rather than the (non-existent) enclosing stack frame.
*Tests:* captured variable is correctly read after the enclosing function has returned;
modifying the original variable after capture does not affect the lambda's copy (value
semantics — mutable capture is out of scope for this item).

**Phase 5 — Lifetime and cleanup** (`src/scopes.rs`):
Emit `OpFreeRef` for the closure record at the end of the enclosing scope.  When the
record contains `text` or `reference` captures, free them in **reverse field index
order** before releasing the record itself — the same LIFO invariant required by tuple
scope exit (see [TUPLES.md](TUPLES.md) § Calling Convention, Scope exit order).  Use
`owned_elements` from Phase 2 to enumerate the fields that need freeing.
*Tests:* no store leak after a lambda goes out of scope; LIFO free order is respected
when multiple closures are live simultaneously; a `text` capture is freed exactly once.

**Effort:** Very High (parser.rs, state.rs, scopes.rs, store.rs)
**Depends on:** P1
**Target:** 1.1+

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

### A10  Field iteration — `for f in s#fields`
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

### A12  Lazy work-variable initialization
**Status: deferred to 1.1+ — blocked by Issues 68–70 (see PROBLEMS.md)**
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

**Tests:** `assign_slots_sequential_text_reuse` in `src/variables.rs` (currently
`#[ignore]` — pending Issue 69 fix).
**Effort:** Medium (three inter-related blockers; Issues 68–70)
**Target:** 0.8.3

---

### A13  Complete two-zone slot assignment (Steps 8 and 10)
**Sources:** SLOTS.md Steps 8, 10 and § Open Issues
**Description:** Three remaining steps to finish the two-zone design. Full details in [SLOTS.md](SLOTS.md).

- **Step 8** — Fix `Set(v, Block)` ordering in `place_large_and_recurse`: when the inner block evaluates first (e.g. `outer = { inner = …; inner }`), process the inner block before placing `outer` so both share the block's result slot legally. After this, replace the `pos > TOS` override branch in `generate_set` with a `debug_assert`.
- **Step 10a** — Audit `build_scope_parents` against `scan_inner`: every IR variant containing a nested block should be handled in both. Missing arms cause `scopes_can_conflict` false-positives in `validate_slots`. Also investigate why any scope maps to itself in the parent map (root cause of the `is_scope_ancestor` cycle guard).
- **Step 10b** — Add a `Value::Iter` arm to `scan_inner` in `src/scopes.rs`, recursing into all three sub-expressions, mirroring the existing arm in `compute_intervals`. Currently safe because parser-synthesised Iter nodes contain no user-defined `Set`; becomes a latent false-positive risk if a parser change ever places a `Set` inside an Iter sub-expression.

**Effort:** Medium per step
**Target:** 1.1+

---

### TR1  Stack trace introspection
**Sources:** STACKTRACE.md
**Description:** `stack_trace()` stdlib function returning `vector<StackFrame>`, where each frame exposes function name, source file, and line number. Full design in [STACKTRACE.md](STACKTRACE.md). Prerequisite for CO1 (coroutines use the frame vector for yield/resume).

- **TR1.1** — Shadow call-frame vector: push/pop a `(fn_name, line)` entry on each function call/return in `src/state/mod.rs`.
- **TR1.2** — Type declarations: `ArgValue` enum and `StackFrame` struct in `default/04_stacktrace.loft`.
- **TR1.3** — Materialisation: `stack_trace()` native function builds `vector<StackFrame>` from the shadow vector.
- **TR1.4** — Call-site line numbers: track source position in the call frame for accurate per-frame line reporting.

**Effort:** Medium
**Target:** 1.1+

---


## S — Stability Hardening

Items found in a systematic stability audit (2026-03-20).  Each addresses a panic,
silent failure, or missing bound in the interpreter and database engine.  All target 0.8.2.

---

### S4 — Binary I/O type coverage

**Source:** PROBLEMS.md Issues 59, 63 · `src/database/io.rs:101`,
`src/database/allocation.rs:399,461`, `src/database/format.rs:109`

**Completed (0.8.2):**
- `read_data`: `Parts::Array` implemented (loop over element count, recurse per element).
  `Parts::Sorted | Ordered | Hash | Index | Spacial` now panics with a clear message
  ("binary I/O not supported for keyed collection fields").  `Parts::Base` → `unreachable!`.
- `write_data`: same messaging improvements for Sorted/etc. and `Parts::Base`.

**Remaining:**
- `write_data` `Parts::Array` — write support requires allocating new records in the store;
  deferred.  Currently panics with a clear message.
- `format_record` Issue 63 — `src/database/format.rs:109` still has a `TODO` for
  `Parts::Struct(_)` and `Parts::EnumValue(_, _)` sub-record fields.  A struct with a
  nested struct field prints `{}` instead of field contents.  Fix: call
  `self.write_field(s, field_index, indent + 1)` recursively, as `write_struct` already
  does for `Parts::Struct` at line 351.

**Effort:** Small (each remaining arm is isolated)
**Target:** 0.8.2

---

### S5  Fix optional `& text` parameter subtract-with-overflow panic
**Sources:** PROBLEMS.md #89
**Severity:** High — interpreter panics at call site when any optional `& text` argument is supplied
**Description:** `create_stack` size calculation for optional `& text` parameters underflows when the argument is provided, causing a subtract-with-overflow panic before the function body is entered.
**Fix path:** Audit `create_stack` in `src/state/codegen.rs` for the optional-reference slot size/offset calculation; ensure the slot reserved for `& T` matches the stack layout.
**Effort:** Small
**Target:** 0.8.2

---

### S6  Fix `for` loop in recursive function — "Too few parameters" panic
**Sources:** PROBLEMS.md #84
**Severity:** High — any algorithm combining recursion with a helper that contains a `for` loop panics
**Description:** `ref_return` adds work-ref attributes to a function's IR while the body is still being parsed. When the function is recursive, call sites seen earlier in the body were compiled with the old (lower) attribute count. Codegen then asserts `parameters.len() >= expected` and panics.
**Fix path:** After the second parse pass completes, scan the IR tree for recursive calls with fewer arguments than the now-finalised attribute count and patch them via `add_defaults`. Significantly simpler than a full per-function variable scoping refactor.
**Effort:** Medium
**Target:** 1.1+

---

### S7  Add diagnostic error for `string` type name — should be `text`
**Sources:** PROBLEMS.md #82
**Severity:** Trivial — users from other languages write `string` and receive confusing cascading errors
**Description:** `string` is not a valid loft type. Writing it produces "Undefined type string" + "Invalid index key" rather than a helpful suggestion to use `text`.
**Fix path:** In the type resolver (`src/typedef.rs`) or early parse pass, detect `string` used as a type name and emit "Unknown type 'string' — did you mean 'text'?" before any other error.
**Effort:** Trivial
**Target:** 0.8.2

---

### S8  Compile-time error when hash-value struct has field named `key`
**Sources:** PROBLEMS.md #83
**Severity:** High — silent store-allocation corruption at runtime with no useful error message
**Description:** `key` is a reserved pseudo-field for hash iteration (`for kv in h { kv.key }`). When a user struct used as a hash value type has a real field named `key`, the name clash corrupts store allocation, producing a late "Allocating a used store" panic.
**Fix path:** In the struct-declaration validator, detect any field named `key` in a type used as a hash value; emit a compile-time error pointing to the field declaration.
**Effort:** Small
**Target:** 0.8.2

---

## N — Native Codegen

`src/generation.rs` already translates the loft IR tree into Rust source files
(`tests/generated/*.rs`).  As of 2026-03-18, **76 of 115 files compile and pass**
(66%).  The remaining 39 failures fall into the categories tracked by the items
below.  Full design in [NATIVE.md](NATIVE.md).

**Target: 0.8.2** — the generator already exists; N items are incremental fixes that turn
broken generated output into correct compiled Rust.  Each fix is small and independent.
See the 0.8.2 milestone in [PLANNING.md](PLANNING.md#version-082) for rationale.

---

### N9  Repair fill.rs auto-generation ✓ DONE
**Description:** Make `create.rs::generate_code()` produce a `fill.rs` that byte-for-byte
replaces the hand-maintained `src/fill.rs`.

**Completed phases:**
- **N20a** — `use crate::ops;` import added to generated header.
- **N20b** — `rustfmt` called on `tests/generated/fill.rs` after each write;
  `#![allow(unused_parens)]` added to suppress double-paren template warnings.
- **N20c** — `n9_generated_fill_matches_src` test enforces byte-exact match;
  `src/fill.rs` replaced with auto-generated version.
- **N20d** — Six operators that previously generated `s.{op}()` delegation stubs now
  have `#rust` templates:
  `OpMathFuncSingle`, `OpMathFunc2Single` (f32 match dispatch),
  `OpMathFuncFloat`, `OpMathFunc2Float` (f64 match dispatch),
  `OpClearScratch` (`stores.scratch.clear()`),
  `OpSortVector` (inlined elem\_size + is\_float + `vector::sort_vector`).
  The remaining delegation operators (e.g. `s.iterate()`) are correctly handled by
  the `s.{name}()` fallback path in `generate_code()` — they still call into State
  methods and require those methods to exist, but the generated code is functionally
  correct.  No `#state_call` annotation was needed.

**Effort:** Medium (completed)
**Target:** 0.8.2 ✓

---

### N2  Implement `CallRef` / function-pointer calls in native codegen
**Sources:** PROBLEMS.md #77
**Severity:** Medium — function-pointer call sites produce invalid generated Rust
**Description:** `Value::CallRef` is not handled in `output_code_inner`; any loft program that calls a function by reference fails to produce correct native output. Affects `tests/scripts/06-function.loft`.
**Fix path:** Add a `Value::CallRef` arm to `output_code_inner` in `src/generation.rs`; emit an indirect call using the function-pointer value, following the same ABI as direct calls.
**Effort:** Medium
**Target:** 1.1+

---

### N3  Resolve `external` crate reference in native codegen
**Sources:** PROBLEMS.md #79
**Severity:** Low — random/FFI extensions produce unresolved-module compile errors
**Description:** The native codegen emits references to an `external` module for random/FFI functions that has no corresponding crate in the generated build.
**Fix path:** Bundle the required symbols into `codegen_runtime`, or emit a proper `extern` block; update `src/generation.rs` accordingly.
**Effort:** Low
**Target:** 1.1+

---

### N4  Fix LIFO store-free order in generated native code
**Sources:** PROBLEMS.md #80
**Severity:** Medium — stores freed in declaration order; allocator requires LIFO
**Description:** `OpFreeRef` calls are emitted in declaration order. The loft store allocator requires LIFO deallocation; out-of-order frees corrupt the heap on the third call to any function with multiple stores.
**Fix path:** In `output_block` in `src/generation.rs`, collect all `OpFreeRef` emissions and sort them by `store_nr` descending before writing to the output.
**Effort:** Medium
**Target:** 1.1+

---

### N5  Implement `file_from_bytes` for `DbRef` vector types in `codegen_runtime.rs`
**Sources:** PROBLEMS.md #86
**Severity:** Medium — `f#read(n) as vector<T>` returns empty vector in native-compiled programs
**Description:** The interpreter fix for `read_file` is in place; the native-path `FileVal::file_from_bytes` implementation in `src/codegen_runtime.rs` remains a stub returning an empty vector.
**Fix path:** Port the interpreter fix: iterate `data.len() / elem_size` elements, call `vector_append` + `write_data` for each element. Until fixed, `12-binary.loft` is in `SCRIPTS_NATIVE_SKIP`.
**Effort:** Medium
**Target:** 1.1+

---

### N6  Fix text method call in format interpolation — emit `&str` not `String`
**Sources:** PROBLEMS.md #87
**Severity:** Small — format strings containing text method calls produce a type-mismatch compile error in generated Rust
**Description:** Text methods return `String`, but `format_text` expects `&str`. The emitter passes the `String` value directly, which the Rust compiler rejects.
**Fix path:** In the format-string emission logic in `src/generation.rs`, bind the method result to a `let _tmp = ...;` temporary and pass `&_tmp` to `format_text`.
**Effort:** Small
**Target:** 0.8.2

---

### N7  Fix `directory()`/`user_directory()`/`program_directory()` scratch buffer argument
**Sources:** PROBLEMS.md #88
**Severity:** Small — directory query functions emit `()` instead of `&mut work_N` in generated Rust
**Description:** Destination-passing text functions require a `&mut String` scratch buffer. The native emitter generates an empty block `()` instead.
**Fix path:** In `src/generation.rs`, detect destination-passing text function calls and emit the pre-allocated scratch buffer `&mut work_N` as the first argument.
**Effort:** Small
**Target:** 0.8.2

---

## O — Performance Optimisations

Planned interpreter and native-codegen performance improvements. Full designs with benchmark data and expected gains in [PERFORMANCE.md](PERFORMANCE.md). Priority order per that file: O1 > O4 > O2 > O5 > O6 > O3 > O7.

---

### O1  Superinstruction merging
**Sources:** PERFORMANCE.md § P1
**Description:** Peephole pass in `src/compile.rs` merges common 4-opcode sequences (var/var/op/put) into single opcodes 240–245. Six new entries added to the `OPERATORS` array in `src/fill.rs`. Operands encoded in the same byte count as the replaced sequence, so branch targets need no relocation.
**Expected gain:** 2–4× on tight integer loops; benefits every loop in the interpreter.
**Effort:** Medium
**Target:** 0.8.2

---

### O2  Stack raw pointer cache
**Sources:** PERFORMANCE.md § P2
**Description:** Add `stack_base: *mut u8` to `State`; refresh once per function call/return; eliminate the `database.store()` lookup on every push/pop. A `stack_dirty` flag, set by allocation ops, triggers a refresh at the top of the dispatch loop.
**Expected gain:** 20–50% across all interpreter benchmarks.
**Effort:** High
**Target:** 1.1+

---

### O3  Verify integer paths carry no `long` null-sentinel checks
**Sources:** PERFORMANCE.md § P3
**Description:** Grep audit of `src/ops.rs` confirming that `*_int` functions never check `i64::MIN`. Add a compile-time string-search test. If violations exist, move the check to `*_long` paths only.
**Expected gain:** 2–5% on pure integer benchmarks if violations exist; zero cost if already clean.
**Effort:** Low
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

### O6  Native: remove `long` null-sentinel from local arithmetic
**Sources:** PERFORMANCE.md § N3
**Description:** Add `op_add_long_nn`, `op_sub_long_nn`, etc. (`_nn` = non-nullable) to `src/ops.rs`. When escape analysis confirms both operands are local (definitely assigned) `long` variables, the native emitter uses `_nn` variants, eliminating the `i64::MIN` check on the hot path.
**Expected gain:** 1.3–1.5× on Collatz and `long`-heavy native benchmarks.
**Effort:** Low
**Target:** 0.8.2

---

### O7  wasm: pre-allocate string buffers in format path
**Sources:** PERFORMANCE.md § W1
**Description:** Pre-allocate the result string with `String::with_capacity` before format-string loops in generated wasm code, and use `push_str` instead of `+` to avoid intermediate allocations through wasm's linear-memory allocator.
**Expected gain:** Reduces wasm/native string-building gap from 2× to <1.3×.
**Effort:** Medium
**Depends:** W1
**Target:** 1.1+

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
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark results and implementation designs for O1–O7 (interpreter and native performance improvements)
- [WEB_IDE.md](WEB_IDE.md) — Web IDE full design: architecture, JS API contract, per-milestone deliverables and tests, export ZIP layout (Tier W detail)
- [RELEASE.md](RELEASE.md) — 1.0 gate items, project structure changes, release artifacts checklist, post-1.0 versioning policy
