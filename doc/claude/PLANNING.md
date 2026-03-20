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
  - [L4 — Fix empty `[]` literal as mutable vector argument](#l4--fix-empty--literal-as-mutable-vector-argument)
  - [L5 — Fix `v += extra` via `&vector` ref-param](#l5--fix-v--extra-via-vector-ref-param)
- [P — Prototype Features](#p--prototype-features)
- [A — Architecture](#a--architecture)
  - [A12 — Lazy work-variable initialization](#a12--lazy-work-variable-initialization)
  - [A13 — Float and Long dead-slot reuse](#a13--float-and-long-dead-slot-reuse-in-assign_slots)
  - [A14 — `skip_free` flag](#a14--skip_free-flag--replace-clean_work_refs-type-mutation)
  - [A15 — Exhaustive `inline_ref_set_in`](#a15--exhaustive-inline_ref_set_in-and-fallback-assertion)
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

**Efficiency and packaging:**
- **A8** — Destination-passing for string natives: eliminates the double-copy overhead on
  `replace`, `to_lowercase`, `to_uppercase` and format expressions.

**Native code generation (Tier N):**
- N2–N9 and N6.3 (runtime fixes, codegen fixes, fill.rs auto-generation, reverse and
  range-bounded iteration) completed in 0.8.2.  Remaining: **N9** (fill.rs auto-generation
  N20b–N20d), and **N1** (`--native` CLI flag) which lands last.

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
1. **A6** — slot pre-pass; High, independent
4. **A8** — destination-passing; Med–High, independent efficiency win
5. **N9** — native codegen fill.rs auto-generation remaining; independent; interleave freely with items 2–4
6. **N1** — `--native` CLI flag; lands after N9

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

### L4  Fix empty vector literal `[]` as mutable vector argument
**Sources:** [PROBLEMS.md](PROBLEMS.md) #44
**Severity:** Medium — passing `[]` directly as a mutable `vector<T>` argument panics in
debug builds; workaround is to assign to a named variable first
**Description:** `parse_vector` returns `Value::Insert([Null])` for an empty `[]` literal
when the parse context has `Type::Unknown(0)` as the target type (which is always the case
for call-site arguments).  No temporary variable is created and no `vector_db` init opcodes
are emitted.  The mutable-argument handler in `generate_call` expects a 12-byte `DbRef` on
the stack but finds 0 bytes.

**Fix path:** In `parse_vector` (`src/parser/expressions.rs`), when `[]` is parsed with an
unknown element type and `is_var = false`:

1. Create a unique temporary variable with `Type::Unknown(0)` as a placeholder.
2. Emit the `vector_db` initialisation block — same as the non-empty path does when
   `block = true`.  The element type will be inferred on the second pass from the
   call-site context.
3. Return `Value::Var(vec)` wrapped in `v_block`, matching the non-empty path.

The second pass already resolves the element type correctly for assignment targets
(`my_vec = []`); the fix extends this to call-site arguments by ensuring a temporary
variable and init block are always emitted for empty `[]` literals.

**Effort:** Medium (parser change; careful handling of `Type::Unknown` on second pass)
**Target:** 0.8.2

---

### L5  Fix `v += extra` via `&vector` ref-param
**Sources:** [PROBLEMS.md](PROBLEMS.md) #56; `tests/issues.rs::ref_param_append_bug`
**Severity:** High — panics in debug builds; silently does nothing in release builds
**Description:** `v += extra` compiled for a `v: &vector<T>` ref-param emits
`OpAppendVector` with the raw ref-param DbRef (a stack pointer into the caller's frame)
instead of dereferencing it first.  `vector_append` calls `store.get_int(v.rec, v.pos)`
where `v.rec` is the caller's stack record, which is absent from the current function's
store claims, causing "Unknown record" panic.

**Fix path:** In `generate_set` / the vector-append codegen path
(`src/state/codegen.rs`), when the target variable is `Type::RefVar(Vector)`:

1. Emit `OpGetStackRef` to load the actual vector DbRef from the ref-param.
2. Emit `OpAppendVector` on the loaded DbRef.
3. Emit `OpSetStackRef` to write the (possibly reallocated) DbRef back through the ref.

The same dereference-operate-writeback pattern applies to all collection-mutating
operations on ref-params.  Identify them in codegen and apply the fix consistently.

**Effort:** Medium (codegen change; audit all collection ops on `RefVar` targets)
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

*Step 2 — Unify the three insertion loops in `parse_code`*:

Replace the three separate loops that insert null-inits at function start:

```rust
// BEFORE (lines 50–97, three separate loops):
for wt in self.vars.work_texts() {
    ls.insert(0, v_set(wt, Value::Text(String::new())));  // always at index 0
}
for r in self.vars.work_references() {
    if !is_argument && depend.is_empty() && !is_inline_ref {
        ls.insert(0, v_set(r, Value::Null));              // always at index 0
    }
}
// inline_refs loop: already uses first_set_in (correct)

// AFTER (one unified loop over all work variables):
let all_work: Vec<(u16, Value)> = self
    .vars.work_texts().into_iter().map(|v| (v, Value::Text(String::new())))
    .chain(self.vars.work_references().into_iter()
        .filter(|&r| !self.vars.is_argument(r) && self.vars.tp(r).depend().is_empty())
        .map(|r| (r, Value::Null)))
    .collect();
let mut insertions: Vec<(usize, u16, Value)> = Vec::new();
for (r, init) in all_work {
    let pos = ls.iter().position(|stmt| first_set_in(stmt, r))
        .unwrap_or_else(|| { debug_assert!(false, ...); fallback });
    insertions.push((pos, r, init));
}
// Sort by descending position to avoid index invalidation; same-position: higher var_nr first
insertions.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
for (pos, r, init) in insertions {
    ls.insert(pos, v_set(r, init));
}
```

*Step 3 — Remove `inline_ref_vars` distinction if possible*:

After Step 2, inline-refs use the same insertion path as all other work variables.  The
`inline_ref_vars` `BTreeSet` on `Function` and `mark_inline_ref` / `is_inline_ref` /
`inline_ref_references` can be removed if the LIFO ordering behaviour is identical — verify
that the unified loop produces the same `var_order` for currently tested functions before
removing.

*Interval effect:* `first_def` for a work text variable that appears in the third statement
of a five-statement function is now `seq(stmt3)` rather than 0.  Two sequential text
operations — `__work_1` used in stmt 2 and `__work_2` used in stmt 4 — can now share a
single 24-byte slot since their intervals no longer overlap.

*Safety:* `first_set_in` finds the outermost top-level statement in `ls` that contains the
work variable's Set, even if that Set is inside a nested if/loop/block.  The null-init is
therefore inserted before that statement.  At runtime the null-init fires before any
conditional path that might use and then free the variable — the same guarantee as the
current function-start approach, but narrowed to the first-use statement.

**Tests:** Add a unit test for `assign_slots`: two sequential `Type::Text` variables with
non-overlapping intervals receive the same slot (size = 24).  Verify existing test suite
still passes.
**Effort:** Small–Medium (parser change; mostly refactoring the three loops into one)
**Target:** 0.8.2

---

### A13  Float and Long dead-slot reuse in `assign_slots`
**Sources:** Stack efficiency evaluation 2026-03-20
**Description:** `assign_slots` (`src/variables.rs`) restricts dead-slot reuse to
variables with `var_size <= 4`.  Float (`Type::Float`, 8 B) and Long (`Type::Long`, 8 B)
are excluded even though they have no pre-init opcodes and their `first_def` is already
computed late (after traversing the value expression), giving accurate intervals.

The restriction was introduced to prevent reuse of large types whose pre-init opcodes
(`OpText`, `OpConvRefFromNull`) fire at `stack.position` (TOS), making reuse unsafe.
Float and Long have no such opcodes and are already excluded from `needs_early_first_def`
(variables.rs:890–894), so the restriction does not apply to them.

`set_var` in `codegen.rs` already emits `OpPutFloat` and `OpPutLong` with a stack
displacement (`var_pos = stack.position - stack.function.stack(var)`, lines 973–981), so
the dead-slot-reuse path (`pos < stack.position → set_var()` in `generate_set:464`) works
correctly for these types today.

**Fix path:**

*Change in `src/variables.rs:1255`* — relax the reuse guard:
```rust
// BEFORE:
let can_reuse = var_size <= 4;

// AFTER:
// Float (8 B) and Long (8 B) have no pre-init opcodes; OpPutFloat/OpPutLong handle
// the below-TOS store correctly.  The size-mismatch guard (line 1285) still prevents
// reusing a 4-byte slot for an 8-byte variable and vice versa.
let can_reuse = var_size <= 8;
```

No other changes.  The existing exact-size guard at line 1285 (`var_size != j_size →
candidate = j_end; continue`) already ensures a Float cannot reuse a dead Long slot and
vice versa, and that neither can reuse a 4-byte primitive slot.

**Tests:** Add a `assign_slots` unit test: two sequential `Type::Float` variables with
non-overlapping intervals (`last_use_1 < first_def_2`) are assigned the same slot.  Verify
the existing `assign_slots_sequential_reuse` test still passes.
**Effort:** Very Small (one-line change + one unit test)
**Target:** 0.8.2

---

### A14  `skip_free` flag — replace `clean_work_refs` type mutation
**Sources:** Stack efficiency evaluation 2026-03-20
**Description:** `clean_work_refs` (`src/variables.rs:738–745`) prevents `OpFreeRef`
from being emitted for an abandoned work-ref variable by mutating the variable's type to
`Type::Reference(0, vec![0])` — a non-empty `depend` list that causes `get_free_vars`
(scopes.rs:407–413) to skip the variable.  Any code that inspects the variable's type
after `clean_work_refs` sees a phantom dependency on ref 0 and may mishandle it.

**Fix path:**

*Step 1 — Add `skip_free: bool` to `Variable`* (`src/variables.rs:41–61`):
```rust
pub struct Variable {
    // existing fields ...
    pub skip_free: bool,   // if true, get_free_vars omits OpFreeRef/OpFreeText for this var
}
```
Default: `false`.  Set in `Variable::new` and initialise in `Function::copy` / `Function::append`.

*Step 2 — Expose via `Function`*:
```rust
pub fn set_skip_free(&mut self, var: u16) {
    self.variables[var as usize].skip_free = true;
}
pub fn is_skip_free(&self, var: u16) -> bool {
    self.variables[var as usize].skip_free
}
```

*Step 3 — Update `clean_work_refs`*:
```rust
pub fn clean_work_refs(&mut self, work_ref: u16) {
    for w in work_ref..self.work_ref {
        let n = format!("__ref_{}", w + 1);
        let v_nr = self.var(&n);
        self.set_skip_free(v_nr);   // was: mutate type to Reference(0, vec![0])
    }
}
```

*Step 4 — Update `get_free_vars`* (`src/scopes.rs:407–413`):
```rust
if let Type::Reference(_, dep) | Type::Vector(_, dep) | Type::Enum(_, true, dep) =
    function.tp(v)
    && dep.is_empty()
    && !tp.depend().contains(&v)
    && !function.is_skip_free(v)   // ← new guard
{
    ls.push(call("OpFreeRef", v, data));
}
```

**Tests:** Existing tests that exercise `clean_work_refs` code paths (functions with
abandoned ref-returning calls) must continue to pass.  No new tests needed.
**Effort:** Small (add field + update 3 call sites)
**Target:** 0.8.2

---

### A15  Exhaustive `inline_ref_set_in` and fallback assertion
**Sources:** Stack efficiency evaluation 2026-03-20
**Description:** `inline_ref_set_in` (`src/parser/expressions.rs:15–34`) uses `_ => false`
as a catch-all for `Value` variants that contain no nested `Set` nodes.  If a new `Value`
variant is added that CAN contain a nested `Set` but is not listed in the recursive cases,
the function silently returns `false`, the fallback in `parse_code` fires, and the inline-ref
temp's null-init is inserted at the wrong position — producing incorrect LIFO ordering
without any diagnostic.

The fallback position ("after the first non-`Line` statement") places the inline-ref too
early in `var_order` when the real first use is in a later statement, causing `OpFreeRef`
for that inline-ref to fire after variables that were allocated after it.

**Fix path:**

*Change 1 — Replace `_ => false` with an exhaustive leaf match*
(`src/parser/expressions.rs:32`):
```rust
// BEFORE:
_ => false,

// AFTER: list every known leaf variant explicitly so the compiler errors on a new variant
Value::Var(_) | Value::Num(_) | Value::Boolean(_) | Value::Text(_)
| Value::Long(_) | Value::Float(_) | Value::Int(_) | Value::Null
| Value::Line(_) | Value::Break(_) | Value::Continue(_) => false,
// If a new Value variant is added that can contain nested expressions, add it to the
// recursive cases above.  Pure-leaf variants (no nested Value) belong here.
```

*Change 2 — Assert the fallback is unreachable* (`src/parser/expressions.rs:84–87`):
```rust
// BEFORE:
let pos = ls
    .iter()
    .position(|stmt| inline_ref_set_in(stmt, *r))
    .unwrap_or(fallback);

// AFTER:
let pos = ls
    .iter()
    .position(|stmt| inline_ref_set_in(stmt, *r))
    .unwrap_or_else(|| {
        debug_assert!(
            false,
            "inline_ref __ref_{r} not found in any top-level statement; \
             LIFO order may be wrong — add the relevant Value variant to \
             inline_ref_set_in"
        );
        fallback
    });
```

Both changes are defence-in-depth only — no semantic change for any currently tested input.
The assert fires during development if a new `Value` variant causes a regression in the
inline-ref ordering, converting silent LIFO corruption into a visible panic.

**Note:** If A12 is implemented first, `inline_ref_set_in` will be renamed `first_set_in`
and extended to all work variables.  Apply these changes to the renamed function.

**Tests:** No new tests needed.  The change is purely defensive; existing tests cover all
current code paths.
**Effort:** Very Small (two targeted edits in expressions.rs)
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

---

### N1  Add `--native` CLI flag
**Description:** Add `--native` mode to `src/main.rs`: parse a `.loft` file, emit a
self-contained Rust source file via `Output::output_native()`, compile it with `rustc`,
and run the resulting binary.  This is the end-to-end native codegen path.
**Depends on:** N6, N9

**Fix path:**

**Step 1 — CLI argument** (`src/main.rs`):
Extend the argument-parsing loop to recognise `--native`:
```rust
"--native" => { native_mode = true; }
```
When `native_mode` is set, run the native pipeline instead of the interpreter pipeline.

**Step 2 — Parse and compile** (`src/main.rs`):
Re-use the existing interpreter pipeline up through `byte_code()`:
```rust
let mut p = Parser::new();
p.parse(&file_content, &file_name)?;
let start_def = compile::byte_code(&mut p.data, &mut p.database)?;
```
`start_def` is the first definition index of the user program (after the stdlib
definitions).

**Step 3 — Emit Rust source** (`src/main.rs`, `src/generation.rs`):
Write to a temporary file in `std::env::temp_dir()`:
```rust
let tmp = std::env::temp_dir().join("loft_native.rs");
let mut f = File::create(&tmp)?;
let mut out = Output { data: &p.data, stores: &p.database, counter: 0,
                       indent: 0, def_nr: 0, declared: Default::default() };
out.output_native(&mut f, 0, start_def)?;
```

**Step 4 — Compile and run** (`src/main.rs`):
```rust
let binary = std::env::temp_dir().join("loft_native_bin");
let status = std::process::Command::new("rustc")
    .args(["--edition=2024", "-o", binary.to_str().unwrap(),
           tmp.to_str().unwrap()])
    .status()?;
if !status.success() {
    eprintln!("loft: native compilation failed");
    std::process::exit(1);
}
std::process::Command::new(&binary)
    .args(std::env::args().skip_while(|a| a != "--native").skip(2))
    .status()?;
```
The `rustc` invocation needs `--edition=2024` (the project uses Rust 2024 features
including `let` chains).  Linking against the `loft` crate is not needed for
self-contained generated code — `output_native` already emits all required `use` paths
from `codegen_runtime`.

**Step 5 — Error handling:**
- If `rustc` is not in `PATH`: print a clear error (`loft: rustc not found; install
  the Rust toolchain to use --native mode`) and exit 1.
- If the generated source has compile errors (indicates a codegen bug): print the
  `rustc` stderr and suggest `--debug` flag to dump the generated source.
- If the binary exits non-zero: propagate the exit code.

**Step 6 — `--native-emit` flag (optional, for debugging):**
Add `--native-emit <out.rs>` to emit the Rust source to a named file without
compiling.  Useful for inspecting codegen output.

**Files changed:**

| File | Change |
|---|---|
| `src/main.rs` | Add `--native` / `--native-emit` flag; native pipeline |
| `tests/native.rs` | Integration test: compile + run a trivial loft program via `--native` |

**Effort:** Medium
**Target:** 0.8.2

---

### N9  Repair fill.rs auto-generation
**Description:** Make `create.rs::generate_code()` produce a `fill.rs` that byte-for-byte
replaces the hand-maintained `src/fill.rs`.  N20a (`use crate::ops;` import) is done.
Remaining phases: N20b (formatting), N20c (replace src/fill.rs), N20d (`#state_call`
annotation for 52 delegation operators).
**Detail:** [NATIVE.md](NATIVE.md) § N20

**Fix path:**

**Phase N20b — Emit properly formatted code** (`src/create.rs`):
`generate_code()` currently emits single-line bodies (`if x { y }`) but `src/fill.rs`
uses expanded form (`if x {\n    y\n}`).  Two approaches:

*Option A — emit expanded form directly in `create.rs`:*
Replace `writeln!(into, "if {} {{ {} }}", ...)` patterns with multi-line equivalents.
This is preferred — it avoids a subprocess dependency.

*Option B — run `rustfmt` on the output file:*
```rust
std::process::Command::new("rustfmt").arg("tests/generated/fill.rs").status()?;
```
Can be called from the test setup in `tests/testing.rs` after `generate_code()`.

After this phase, `diff tests/generated/fill.rs src/fill.rs` should produce no output
(modulo header comment differences).
*Tests:* `cargo test n9_generated_fill_matches_src` passes.

**Phase N20c — Replace `src/fill.rs` with generated version** (`tests/testing.rs`,
CI):
Add a CI check that:
1. Runs the test that calls `generate_code()` (already happens in debug test runs).
2. Compares `tests/generated/fill.rs` against `src/fill.rs`.
3. Fails the test with a diff excerpt if they differ.

Once this CI check is green on the first run (after N20b produces an identical file),
copy `tests/generated/fill.rs` → `src/fill.rs` and add a note at the top of
`src/fill.rs`: `// Auto-generated by create.rs. Do not edit manually.`

From this point, any new opcode added to `default/*.loft` with a `#rust` template is
automatically included in `src/fill.rs`.

*Tests:* `cargo test n9_fill_is_generated` fails if `src/fill.rs` drifts from
`tests/generated/fill.rs`.

**Phase N20d — Add `#state_call` annotation** (`default/*.loft`, `src/create.rs`):
Currently 52 operators delegate to `State` methods (e.g., `s.iterate()`) but have no
`#rust` template.  `generate_code()` silently skips them.  Add a new loft annotation:
```loft
fn OpIterate(...);
#state_call"iterate"
```
In `create.rs::generate_code()`, recognise `#state_call"method_name"` and emit:
```rust
fn n_op_iterate(s: &mut State) {
    s.iterate();
}
```
The 52 delegation operators are listed in [NATIVE.md](NATIVE.md) § N20d.  Adding them
one by one eliminates the remaining hand-maintained entries in `src/fill.rs`.

*Tests:* after adding `#state_call` for all 52, `n9_fill_is_generated` still passes;
no hand-maintained entries remain in `src/fill.rs`.

**Files changed:**

| File | Change |
|---|---|
| `src/create.rs` | Emit expanded-form Rust in `generate_code()`; handle `#state_call` |
| `default/*.loft` | Add `#state_call"..."` for 52 delegation operators |
| `src/fill.rs` | Replace with auto-generated version after N20c |
| `tests/testing.rs` | Add CI diff check (`n9_fill_is_generated`) |

**Effort:** Medium
**Target:** 0.8.2

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
- [WEB_IDE.md](WEB_IDE.md) — Web IDE full design: architecture, JS API contract, per-milestone deliverables and tests, export ZIP layout (Tier W detail)
- [RELEASE.md](RELEASE.md) — 1.0 gate items, project structure changes, release artifacts checklist, post-1.0 versioning policy
