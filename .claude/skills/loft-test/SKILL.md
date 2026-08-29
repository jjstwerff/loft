---
name: loft-test
description: Reference for writing tests against the loft interpreter, --native backend, and WASM build. Apply whenever adding, editing, or reviewing tests/*.rs or tests/scripts/*.loft / tests/docs/*.loft. Covers test-binary layout, the `code!` and `cross_mode!` macros, the @EXPECT_ERROR / @EXPECT_FAIL / @ARGS / @NAME / @TITLE annotations on `.loft` files, ignore conventions, P-id rules, and the targeted-suite map for subsystem changes.
user-invocable: false
---

# Loft Testing Reference

Always consult this before adding or modifying tests under `tests/`.
The loft project has ~30 integration test binaries plus a custom
testing framework; picking the wrong binary or the wrong macro means
either a slow CI cycle or a test that doesn't actually validate the
change.

For runtime debugging conventions (LOFT_LOG presets, dump files), see
the parent project's [TESTING.md](../../../doc/claude/TESTING.md).  This
skill covers the *authoring* side: where a test belongs, which macro
to use, and how to keep the suite fast.

---

## Test-binary layout

Each `tests/*.rs` file becomes one integration-test binary.  Pick the
binary that matches the kind of behaviour you're verifying.

| Binary | Purpose | Typical macro / harness |
|---|---|---|
| `tests/wrap.rs` | Loft script suites in `tests/scripts/*.loft` and `tests/docs/*.loft` driven through the **interpreter**.  Verifies parse → scope-check → bytecode → execute end-to-end against `// @EXPECT_*` annotations in the script. | Reads scripts from disk; no Rust-level macro. |
| `tests/native.rs` | Same scripts as wrap.rs but driven through `--native` (rustc compilation).  Catches codegen vs interp divergence. | Disk-driven; reuses `find_loft_rlib` + `compile_native_job` helpers. |
| `tests/issues.rs` | The 540-test regression register — every fix lands here.  Most expression / control-flow / parser fixes get pinned by a small named test. | `code!(...)` |
| `tests/expressions.rs` | Language feature tests grouped by topic (T1 tuples, T1.10 cells, …). | `code!(...)` and `expr!(...)` |
| `tests/parse_errors.rs` | Negative tests — the loft source must produce a specific diagnostic. | `code!(...).error(...).warning(...)` |
| `tests/threading.rs` | `run_parallel_*` direct API tests.  Touch this when changing `src/parallel.rs` or `src/codegen_runtime.rs::n_parallel_*`. | Direct Rust API calls. |
| `tests/threading_chars.rs` | `par(...)` over `vector<character>` and `vector<text>` canaries; 4 currently `#[ignore]`d on T1.8a. | Direct Rust API. |
| `tests/tuple_matrix.rs` | Plan-14 cross-mode matrix — every cell runs interp + `--native` and asserts byte-identical stdout.  **Heavy by default — every cell is `#[ignore]`d.** | `cross_mode!(name, body)` |
| `tests/codegen_emitter.rs` | Native-codegen emitter registry tests.  P200 / P202 / P203 / P204 / P205 regressions live here. | Direct registry/dispatch tests. |
| `tests/exit_codes.rs` | Whole-binary exit-code tests — invokes the compiled `loft` binary as a subprocess via `env!("CARGO_BIN_EXE_loft")`. | `std::process::Command` |
| `tests/error_messages.rs` | Diagnostic *rendering* tests (not just text content) — caret placement, source-line context, summary. | Subprocess invocation. |
| `tests/leak.rs` | Store-allocation leak detection. | `code!(...)` with `LOFT_LOG=alloc_free` style asserts. |
| `tests/format.rs` | `{x:fmt}` interpolation tests. | `code!(...)` |
| `tests/imports.rs` | `use foo` / `use foo::*` tests with fixtures in `tests/lib/`. | `code!(...)` and disk fixtures. |
| `tests/slots.rs` / `slot_v2_baseline.rs` | Slot-allocator invariants.  Touch when changing `src/scopes.rs` or slot codegen. | Programmatic IR walks. |
| `tests/graphics_gold.rs` | Pixel-comparison golden tests for the graphics library. | Subprocess invocation. |
| `tests/html_wasm.rs` / `wasm_entry.rs` | WASM build verification. | Subprocess. |

**When unsure where a test belongs**, look for an existing test that
shares the *kind* of failure you'd reproduce (parse error → parse_errors,
runtime mismatch → issues, native-vs-interp → tuple_matrix, codegen
template → codegen_emitter).  Match the shape, don't invent a new file.

**Common module:** `tests/common/mod.rs` is `mod common;` from each
binary.  It exposes `cached_default()` (cached default-stdlib parse —
saves ~2s per test) and `cross_mode::run_cross_mode()` (the cross-mode
harness).  Every helper there is `#[allow(dead_code)]` because not
every binary uses every helper.

---

## The `code!` macro — primary unit-test API

Lives in `tests/testing.rs`.  Used by issues / expressions / parse_errors /
format / leak / imports / structs / vectors / strings / etc.

```rust
mod testing;
extern crate loft;
use loft::data::Value;

#[test]
fn my_test() {
    code!("fn test() { x = 3 + 4; }")        // loft source
        .expr("x")                            // expression to evaluate
        .result(Value::Int(7));               // expected value
}
```

Chain methods:

| Method | Effect |
|---|---|
| `.expr(s)` | After running the loft source, evaluate this expression and capture its result for `.result()` |
| `.result(v)` | Assert the most recent `.expr()` produced this `Value` (`Value::Int`, `Value::Text`, `Value::Boolean`, `Value::Null`, …) |
| `.typed(t)` | Assert the result type matches |
| `.error(msg)` | Expect this exact diagnostic (substring or full).  Format: `"<text> at <test_name>:<line>:<col>"`.  Multiple `.error(...)` calls assert multiple diagnostics. |
| `.warning(msg)` | Same as `.error` but for warnings.  Warnings don't suppress execution. |
| `.fatal(msg)` | Expect a fatal panic with this message. |

The loft source must declare `fn test() { ... }` — the framework
runs `test` after parsing.  Helper fns at file scope are fine.  **No
nested fns.**

`expr!` is shorthand for "wrap a single expression in `fn test() { … }`":

```rust
expr!("3 + 4").result(Value::Int(7));
```

Use `code!` when you need a multi-statement test or fixture
declarations; `expr!` for one-liners.

---

## The `cross_mode!` macro — interp ↔ native equivalence

Lives in `tests/common/cross_mode.rs`.  Used **only** by
`tests/tuple_matrix.rs` today.  Plan-14 owns the cross-mode contract:

```rust
mod common;

cross_mode!(my_cell, r#"
    fn test() {
        t = (3, 7);
        print("{t.0},{t.1}\n");
        assert(t.0 == 3 && t.1 == 7, "my_cell");
    }
"#);
```

Mechanics:
1. Writes the body to `/tmp/loft_xmode_<test_name>.loft`.
2. Runs `loft --interpret` and `loft --native` as subprocesses
   (uses `env!("CARGO_BIN_EXE_loft")`).
3. Captures both stdouts, normalises CRLF→LF and trailing
   whitespace, asserts:
   - both modes succeed (exit 0),
   - **both stdouts are byte-identical**.

**Body contract:**
- The body must declare `fn test() { … }` — that is the entry point.
- Helper fns must live alongside `fn test`, not nested inside it.
- The harness appends `fn main() { test(); }`, so don't include your own.

**Heavy by default.**  Every `cross_mode!`-generated test is
`#[ignore = "tuple_matrix — run with --test tuple_matrix -- --ignored"]`.
Default `cargo test` skips them.  To run the matrix:

```bash
# All cells:
cargo test --release --test tuple_matrix -- --ignored

# A single cell:
cargo test --release --test tuple_matrix -- --ignored e1_d1_int_int_local

# Skip known-broken cells (P207, T1.8a):
cargo test --release --test tuple_matrix -- --ignored \
    --skip e1_d1_char_int_local \
    --skip e1_d2_return_int_int \
    --skip e2_d2_return_text_text
```

Each `--native` run invokes `rustc`; cells take 1–10 s each on a warm
target/.  That's why they're not on the default path.

---

## Pure loft tests — `tests/scripts/*.loft` and `tests/docs/*.loft`

A `.loft` test file is **three tests for the price of one**.  The same
file is picked up by:

- `tests/wrap.rs::dir` (and `::loft_suite`) — runs it through the
  **interpreter** end-to-end.
- `tests/native.rs::tests` — generates Rust source, compiles with
  `rustc`, runs the **native** binary.
- `tests/wrap.rs::wasm_dir` — compiles via `--native-wasm`, optionally
  runs with `wasmtime` (skipped silently if `wasm32-wasip2` or
  `wasmtime` is unavailable).

### What a `.loft` test asserts with

**`assert(condition, message)` is the whole vocabulary.** There is no `assert_eq`, no
`assert_true`, and no `Test::` namespace — those are habits from other languages and cost a
compile error (`Unknown function assert_eq_int`) before they cost anything else.

```loft
assert(seen == 3, "the generator arrives three times: {seen}");
```

Interpolate the value you GOT into the message. The condition carries what you wanted, so a
failure that prints only "assertion failed" tells the next reader nothing they cannot already
see in the source, while `got 2` tells them how far off it was. (A general `assert_eq` that
reports both sides is loft-lang/loft#1147.)

The message is a normal interpolated text, so a struct field, a length or a whole expression
can go in it.

**That's why pure loft tests have a bigger testing scope than Rust
integration tests.**  A `code!(...)` test in `tests/issues.rs`
exercises one execution path (the in-process interpreter via
`State::execute`).  A `.loft` script automatically exercises all three
backends — interpreter, `--native`, and WASM — and any divergence
between them surfaces as a backend-specific failure.  When you fix a
codegen bug, dropping a single `.loft` reproducer into `tests/scripts/`
locks all three backends in one stroke; the same coverage in Rust
would be three separate tests.

This is also why **the cross-mode Rust harness exists at all**: plan-14
needs precise per-cell control of which loft snippet runs in which
mode and a stdout-equivalence assertion.  For broader regression
coverage where you trust the assertions in the script body itself, a
`.loft` file under `tests/scripts/` is the lighter-weight option.

### Where to put a new `.loft` test

| Location | Purpose | Doc fields required? |
|---|---|---|
| `tests/scripts/<NN>-<topic>.loft` | Regression script for a fix or a feature corner case.  Numbered prefix sorts the run order; new files get the next free number. | No |
| `tests/docs/<NN>-<topic>.loft` | Topic-level documentation script.  Output is part of the public language reference (`gendoc` writes HTML from these). | **Yes** — `@NAME` + `@TITLE` |
| `tests/lib/<name>.loft` | Library fixture for tests using `// @ARGS: --lib tests/lib`. | No |

### Test-driver annotations

All annotations are line comments of the form `// @KEY[: VALUE]`, placed
either in the file header (above the first declaration) or just above a
specific `fn`.

| Annotation | Scope | Effect |
|---|---|---|
| `// @NAME: <short>` | Header (docs only) | Short name for the HTML doc index.  Required in `tests/docs/*.loft`. |
| `// @TITLE: <text>` | Header (docs only) | Full title for the rendered doc page.  Required in `tests/docs/*.loft`. |
| `// @ARGS: --lib <dir>` | Header | Extra CLI args passed to both wrap.rs and native.rs runners.  Only `--lib <dir>` is recognised at the test layer; other flags are ignored.  Use to point a script at a fixture directory (e.g. `// @ARGS: --lib tests/lib`). |
| `// @EXPECT_ERROR: <substring>` | Anywhere | The script is expected to fail parse / scope-check / runtime with a diagnostic containing this substring.  Multiple `@EXPECT_ERROR:` lines accumulate.  Native runs **skip** files with `@EXPECT_ERROR` (the negative test only runs against the interpreter). |
| `// @EXPECT_WARNING: <substring>` | Anywhere | Like `@EXPECT_ERROR` but for warnings — execution proceeds. |
| `// @EXPECT_FAIL` | File-level (header) **or** fn-level (the comment block immediately above a `fn`) | Tolerate a panic.  File-level: parse / scope-check / runtime failures are accepted anywhere.  Fn-level: only the named fn's panic is tolerated; sibling fns still must pass.  Add a colon-trailing reason when known: `// @EXPECT_FAIL: native function not loaded`.  Native runs **skip** files with `@EXPECT_FAIL` entirely. |
| `// #warn <text>` | Anywhere | Older-style expected warning.  Still supported; `@EXPECT_WARNING:` is preferred for new tests. |

**Diagnostic-format rule:** the `<substring>` for `@EXPECT_ERROR` and
`@EXPECT_WARNING` is a substring match against the rendered diagnostic.
Don't include the trailing ` at file:line:col` location — that's
appended by the renderer and would break when line numbers shift.  Just
the message text.

### File-level skip arrays

When a script can't run in one backend (rare; usually a known feature
gap), name it in the relevant skip array instead of marking the script
itself with `@EXPECT_FAIL`:

| Constant | File | When to add |
|---|---|---|
| `SUITE_SKIP` (in `wrap.rs`) | Currently empty.  Used for "interpreter can't run this" — extremely rare since the interpreter is the reference backend. |
| `WASM_SKIP` (in `wrap.rs`) | Add when WASM build can't accept the script (e.g. threading model differences).  Each entry includes a `// todo!()` comment explaining why. |
| `NATIVE_SKIP` (in `native.rs`) | Currently empty for `tests/docs/`.  Same idea: native-specific feature gap. |
| `SCRIPTS_NATIVE_SKIP` (in `native.rs`) | Same as `NATIVE_SKIP` but for `tests/scripts/`. |

Prefer `@EXPECT_FAIL` for "this is broken right now" (single source of
truth in the script) and the skip arrays for "this backend
fundamentally can't run this kind of script" (orthogonal to the bug
status).

### When a `.loft` test is preferred over a Rust unit test

- The bug reproduces in a sequence of statements that already mirrors
  loft idioms (file I/O, struct construction, vector ops).
- You want all three backends to assert the same behaviour without
  writing the assertion three times.
- The fix is in codegen / runtime, where backend divergence is the
  actual hazard.

### When a Rust unit test is preferred

- The behaviour is a pure parse-error (no runtime to verify) — use
  `code!(...).error(...)` in `tests/parse_errors.rs`, faster than
  spinning up wrap+native+wasm.
- You need precise control over a `Value` comparison or type — use
  `code!(...).expr(...).result(Value::Int(...))` in `tests/issues.rs`.
- The bug is in the testing harness itself.
- Cross-mode equivalence with byte-identical stdout matters (use
  `cross_mode!`).

---

## `#[ignore = "<reason>"]` conventions

Every `#[ignore]` attribute MUST carry a reason.  Bare `#[ignore]` is
opaque and breaks the audit trail.  **The reason MUST name the
trigger to resume** so a future `cargo test -- --ignored` run +
`grep` audit can locate every parked test and tell what should
re-activate it.

Reason categories:

| Reason format | Meaning | Example |
|---|---|---|
| `P###` / `#NNN` | Open bug — now a **GitHub Issue** (legacy `P###` ids survive as references; PROBLEMS.md is the closed archive).  Un-ignore in the commit that closes it (`Fixes #NNN`). | `#[ignore = "P207 — native char-tuple-elem eq codegen bug"]` |
| `<feature-tag> — <plan-ref>` | Waiting on a feature or plan phase that hasn't shipped.  Un-ignore in a one-line follow-up commit when the feature lands. | `#[ignore = "T1.8a — plan-06 phase 9a"]` |
| `tuple_matrix — run with …` | Heavy-by-default test.  Auto-applied by `cross_mode!`.  Don't write by hand. | (macro-applied) |
| `<plan>-<phase>` | Pending implementation in a multi-phase plan.  Same un-ignore rules as a bug, but tracked via the plan rather than an Issue. | `#[ignore = "plan-14 phase 03"]` |
| `<plan> (<sub>) — un-ignore when <trigger>` | User-facing lock-in test: the test demonstrates a today-broken behaviour, marked ignored so CI stays green; auto-flips to PASS when the trigger fires. | `#[ignore = "plan-17 (A) caveat — implicit generic-tuple type inference; un-ignore when the parser propagates substituted return types to receiving variables (DEFERRED.md / USER_FACING.md)"]` |

**The trigger phrase is mandatory.** Acceptable forms include
`un-ignore when <X>`, `triggers when <X>`, or `<plan>-phase <N>`.
The convention is greppable: `cargo test -- --ignored` + reading
the reason should tell a future contributor what to do.

When un-ignoring, the commit message names the reason being retired and
the new test status (`P207 closes; cell flips from #[ignore] to PASS`).

### Lock-in tests for user-facing deferred items

When deferring an item that affects user code (anything that
belongs in `doc/claude/USER_FACING.md`), **write the lock-in test
in the same commit as the deferral**.  The test exercises the
today-broken shape, asserts the post-fix behaviour, and is
`#[ignore]`d with a trigger.  When the fix lands, the test goes
green automatically — preventing accidental release without the
fix.

Example (plan-17 phase 01 follow-up):

```rust
#[test]
#[ignore = "plan-17 (A) caveat — implicit generic-tuple type inference; un-ignore when the parser propagates substituted return types to receiving variables (DEFERRED.md / USER_FACING.md)"]
fn plan17_a_implicit_generic_tuple_type_inference() {
    code!(
        "fn min_max<T: Ordered>(a: T, b: T) -> (T, T) {
    if a < b { (a, b) } else { (b, a) }
}
fn run() -> integer {
    t = min_max(7, 3);                         // <- no annotation
    t.0 * 10 + t.1
}"
    )
    .expr("run()")
    .result(Value::Int(37));
}
```

The two-file index — `doc/claude/plans/DEFERRED.md` (every parked
item) and `doc/claude/USER_FACING.md` (user-visible subset) — is
the single source of truth.  A lock-in test references the
relevant file in its ignore reason so a future contributor can
trace the audit trail in one grep.

---

## Filing a bug you find while testing

Authoritative policy: `CLAUDE.md` § Bug-filing policy + `doc/claude/ISSUE_TRACKING.md`.
**Open bugs live as GitHub Issues now, not PROBLEMS.md rows** (PROBLEMS.md is the
closed/historical archive; the legacy `P###` ids survive only as references).

1. **Default is to FIX, not file.** A bug you surface while testing is the cheapest
   you'll ever fix — code paths warm, repro at hand: fix it + pin a regression.  File
   only when you are NOT fixing it now (it blocks the task, or it's M+/needs design).
2. **When you file → `gh issue create`** (the `bug_report` template): a minimal
   reproducer (expected vs observed per backend), `sev:` + `area:` labels, and a
   **verified** `wa:*` workaround.  Not a PROBLEMS.md row.
3. **Do NOT file** for: clippy / formatter complaints (fix in-branch, note the commit
   — memory `feedback_fix_old_clippy.md`); or a bug you fix in the SAME change (the
   fix + regression test ARE the record — close it with `Fixes #NNN`, don't also file).
4. **Pin every fix with a regression test.** Name it `p<NNN>_<short>` for a legacy
   P-id or `tests/scripts/NN-<slug>.loft` for a GitHub issue; reference it in the
   commit (`Fixes #NNN`).

---

## Targeted regression — which suites to run

**Just run `./scripts/find_problems.sh`.** Its default is the CURATED set —
everything except a short named list of slow-and-few binaries — which is 3733 of
3833 tests in ~70s instead of ~370s. You no longer have to guess which suites
your change could break, and guessing is what the old table here asked for: it
named 18 binaries of 177, and two (`format`, `graphics_gold`) had not existed for
some time.

Guessing does not work, and there is a worked example. An over-broad change to
the parser's `null()` was caught by `binary_io_matrix` — a binary no
parser-touches-these-suites map would ever have picked. Curating by INCLUSION has
to predict which suite will catch a bug, which nobody can do in advance; curating
by EXCLUSION leaves a miss set that is small, named and reviewable.

| You want | Run |
|---|---|
| the normal check before a commit | `./scripts/find_problems.sh` (~70s) |
| a tight loop on one area | `./scripts/find_problems.sh --subject <name>` (seconds) |
| everything, incl. the slow-and-few | `./scripts/find_problems.sh --full` (~370s) |
| to see subjects + what is excluded | `./scripts/find_problems.sh --list-subjects` |

Subjects are `parser scopes codegen runtime store wasm packages lsp sql docs
host`, defined as binary-name PATTERNS in `scripts/test_subjects.sh` so a new
binary joins the subject its name already matches. They are a convenience for
tight loops, not the safety mechanism — the default is subtractive, so a gap in
a subject costs seconds, never coverage.

What the default skips is eight binaries that are slow AND have very few tests
(they hold 57% of the suite's work for 4.5% of its binaries). CI's
`Test (ubuntu-latest)` job runs the suite unsharded as a required check, so none
of them can be skipped on the way to main.

**Always run after the targeted set:**
- `cargo fmt --all -- --check`
- `cargo clippy --release --all-targets -- -D warnings`
- `cargo clippy --release --all-targets --no-default-features -- -D warnings`

The two clippy variants together are the local CI gate; the
`--no-default-features` variant catches lint debt in conditionally
compiled paths and is the one most often skipped.

---

## When the targeted set isn't enough

For multi-subsystem changes (compiler refactors, ABI changes, big
plans crossing parser+codegen+runtime), use the background full-run:

```bash
./scripts/find_problems.sh --bg     # detached cargo test --release --no-fail-fast
./scripts/find_problems.sh --peek   # mid-run stats
./scripts/find_problems.sh --wait   # block until done
```

`/tmp/loft_problems.txt` gets a structured summary (FAILED list,
stdout blocks, SIGSEGV context, wrap-suite `--nocapture` re-run if
a crash masks a `.loft` filename).  See
[TESTING.md § Preferred shape](../../../doc/claude/TESTING.md) for the
full rationale.

---

## Naming conventions

- `p<NNN>_<short_describe>` — regression test for P<NNN>.  Lives in
  the binary that exercises the relevant code path (most often
  `issues.rs` or `parse_errors.rs`).
- `<feature>_<aspect>_<expected>` — feature tests.  E.g.
  `tuple_match_binding`, `tuple_compound_assign_rejected`.
- `e<elem>_d<dest>_<sub>` — plan-14 matrix cells.  Don't reuse this
  prefix outside `tests/tuple_matrix.rs`.

The `should_panic` attribute is rare — most negative behaviour goes
through `.error()` / `.warning()` on `code!`.  Reserve `should_panic`
for runtime panics that have no diagnostic-printing path.

---

## Pre-flight checklist for a new test

- [ ] Picked the right binary (matches the failing-shape).
- [ ] Used `code!` for unit tests, `cross_mode!` for runtime + cross-backend, `expr!` for one-liner expression results.
- [ ] If `#[ignore]`, the reason follows the conventions table above.
- [ ] If pinning a P-id fix, the test name starts with `p<NNN>_`.
- [ ] If un-ignoring, the commit message names the reason being retired.
- [ ] Ran the targeted-suite list, not the full suite, unless the change is multi-subsystem.
- [ ] `cargo fmt --all -- --check` and both clippy variants are green.
- [ ] No nested `fn` definitions in any loft body string.
- [ ] No `->` arm separators in any `match` (use `=>`).
- [ ] No `cross_mode!` body shorter than `fn test() { … }` (the harness appends `fn main`, nothing else).
- [ ] If introducing a new `tests/*.rs` binary, added a row to the test-binary table above.
- [ ] If adding a `.loft` test, picked the right location: `tests/scripts/` (regression) vs `tests/docs/` (also drives HTML).
- [ ] `tests/docs/*.loft` files have both `@NAME:` and `@TITLE:` header comments.
- [ ] `@EXPECT_ERROR:` / `@EXPECT_WARNING:` substrings do NOT include the `at file:line:col` tail.
- [ ] `@EXPECT_FAIL` placement is correct: file-level only when the comment is in the header above the first declaration; fn-level only when the comment is the line(s) immediately above the target `fn`.

---

## Parser tracing (`LOFT_TRACE`)

Compile-time debugging vantages with near-zero overhead when
disabled.  Distinct from `LOFT_LOG` (runtime/bytecode) and
log-config (loft-program-level).  See `src/trace.rs` for the
implementation; this section documents *when to use it*.

### Enabling

```bash
# Single category:
LOFT_TRACE=call cargo run --release --bin loft -- --interpret /tmp/foo.loft

# Multiple categories, comma-separated:
LOFT_TRACE=call,field,generic cargo run ...

# Everything:
LOFT_TRACE=all cargo run ...

# During tests (use --nocapture to see stderr):
LOFT_TRACE=generic cargo test --release --test issues plan17 -- --nocapture
```

When `LOFT_TRACE` is unset (default), trace calls compile to one
bool load + one branch; the branch predictor learns "always-false"
fast and the overhead is below measurement.  Format-string args
are evaluated only when the branch fires.

### Currently registered categories

| Category | Site | Use when debugging |
|---|---|---|
| `call` | `Parser::call` after `find_fn` | "Which def did the parser resolve, and was it skipped as Generic?"  Used heavily in plan-17 (A). |
| `field` | `Parser::field` after attr lookup | "Did method dispatch find the attribute, what's the receiver type?"  Used in plan-17 (B). |
| `generic` | `predict_generic_return_type` + `try_generic_instantiation` | "What did first-pass predict?  What did second-pass instantiate?"  Used in plan-17 (A). |
| `match` | `expect_match_arm_arrow` | "What arrow did the parser see at the arm boundary?"  Used in P206 + plan-18. |

### Adding a new category — selective rule

The `LOFT_TRACE` infrastructure is designed for **recurring**
diagnostic vantages — not one-off probes.  When debugging surfaces
a useful eprintln, ask:

- **One-off** (specific to this bug, unlikely to recur): keep the
  `eprintln!` local, remove before commit.  No trace point.
- **Recurring** (likely to be revisited for similar bugs in the
  same subsystem): convert to a `loft_trace!(category, …)` call.
  Permanent; future sessions enable the category and observe.

Categories live where the recurrence justifies them.  Today's
four categories cover parser-time debugging because that's where
the recent bug-fix density was.  Future categories MAY cover:

- `database` — store allocation, free, leak tracking.
- `data` / `types` — type resolution, narrowing, deps.
- `codegen` — generation/* template substitution + emit.

Add categories only when a real recurring use case appears, not
pre-emptively.  `src/trace.rs` documents the steps:

1. Add a `pub <name>: bool` field to `TraceCategories`.
2. Add the initialiser line.
3. Add a macro arm in `loft_trace!` (mirror existing pattern).
4. Update the table above.

### When NOT to use `LOFT_TRACE`

- Runtime/bytecode debugging — use `LOFT_LOG` (TESTING.md).
- Loft-program-level logging — use `log_info` / `log_warn` /
  `log_error` from inside loft programs (STDLIB.md § Logging).
- One-time diagnostic probes — temporary `eprintln!` is fine
  for a single session; remove before commit.

---

## Cross-references

- [TESTING.md](../../../doc/claude/TESTING.md) — runtime debugging knobs:
  `LogConfig`, `LOFT_LOG`, dump file format, `LOFT_DUMP_DEPTH`.
- [ISSUE_TRACKING.md](../../../doc/claude/ISSUE_TRACKING.md) — where bugs
  live (open → GitHub Issues, closed → PROBLEMS.md archive) + the item
  lifecycle.  Run `gh issue list` before filing a new one.
- [DEVELOPMENT.md](../../../doc/claude/DEVELOPMENT.md) — branch policy,
  commit ordering, push gate.
- [loft-write skill](../loft-write/SKILL.md) — for the loft-source
  side of test bodies (types, syntax, error→fix table).
- `tests/testing.rs` — `code!` + `expr!` macro source.
- `tests/common/cross_mode.rs` — `cross_mode!` harness source.
- `tests/native.rs` — donor of `find_loft_rlib`,
  `compile_native_job`, `run_native_job` helpers.
