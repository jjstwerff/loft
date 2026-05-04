---
name: loft-test
description: Reference for writing Rust integration tests for the loft interpreter and native backend. Apply whenever adding, editing, or reviewing tests/*.rs. Covers test-binary layout, the `code!` and `cross_mode!` macros, ignore conventions, P-id rules, and the targeted-suite map for subsystem changes.
user-invocable: false
---

# Loft Testing Reference

Always consult this before adding or modifying tests under `tests/`.
The loft project has ~30 integration test binaries plus a custom
testing framework; picking the wrong binary or the wrong macro means
either a slow CI cycle or a test that doesn't actually validate the
change.

For runtime debugging conventions (LOFT_LOG presets, dump files), see
the parent project's [TESTING.md](../../doc/claude/TESTING.md).  This
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

## `#[ignore = "<reason>"]` conventions

Every `#[ignore]` attribute MUST carry a reason.  Bare `#[ignore]` is
opaque and breaks the audit trail.  Reason categories:

| Reason format | Meaning | Example |
|---|---|---|
| `P###` | Open bug tracked in PROBLEMS.md.  Un-ignore in the same commit that closes the P-id. | `#[ignore = "P207 — native char-tuple-elem eq codegen bug"]` |
| `<feature-tag> — <plan-ref>` | Waiting on a feature or plan phase that hasn't shipped.  Un-ignore in a one-line follow-up commit when the feature lands. | `#[ignore = "T1.8a — plan-06 phase 9a"]` |
| `tuple_matrix — run with …` | Heavy-by-default test.  Auto-applied by `cross_mode!`.  Don't write by hand. | (macro-applied) |
| `<plan>-<phase>` | Pending implementation in a multi-phase plan.  Same un-ignore rules as P### but tracked via the plan rather than PROBLEMS.md. | `#[ignore = "plan-14 phase 03"]` |

When un-ignoring, the commit message names the reason being retired and
the new test status (`P207 closes; cell flips from #[ignore] to PASS`).

---

## P-id filing rules

Authoritative rules (see also memory `feedback_fix_old_clippy.md`):

1. **DO file** P-ids in `doc/claude/PROBLEMS.md` for genuine open bugs
   you discover while doing other work — runtime crashes, wrong
   results, codegen mismatches, parser hangs.  Filing prevents the
   issue from getting lost when you context-switch.
2. **Do NOT file** P-ids for:
   - Clippy lints / formatter complaints (those are routine
     maintenance — fix in the same branch and describe in the commit
     message, not in PROBLEMS.md).
   - Bugs that are fixed in the same commit — the commit message and
     regression test carry the audit trail; a born-resolved P-id is
     noise.
3. **Pin every fix with a regression test** and reference the test in
   the closing P-id text.  The convention from the existing 200-series
   entries:  
   `Pinned by `p<NNN>_<short_name>` regression tests in
   `tests/<binary>.rs`.`
4. P-ids are global; the next free number is one past the last entry
   in PROBLEMS.md (P207 as of 2026-05-04).

---

## Targeted regression — which suites to run

Don't default to `cargo test --release --no-fail-fast` (≈7 minutes).
Most changes only touch a few subsystems.  Map your edit to the
relevant suites:

| You touched | Run these (in order; ~3-5 minutes total) |
|---|---|
| `src/parser/*` | `parse_errors`, `issues`, `expressions`, `format` |
| `src/scopes.rs` / slot allocator | `slots`, `slot_v2_baseline`, `frame_vars`, `issues`, `leak` |
| `src/state/*` (interpreter) | `wrap`, `issues`, `expressions`, `threading` |
| `src/generation/*` (native codegen) | `native`, `native_loader`, `native_ext`, `codegen_emitter`, `issues` |
| `src/parallel.rs` / `src/codegen_runtime.rs` (parallel) | `threading`, `threading_chars`, `parallel_rebase` |
| `src/wasm.rs` / WASM bridges | `html_wasm`, `wasm_entry` |
| `default/*.loft` (stdlib) | `wrap`, `issues`, `expressions`, plus the topic-specific suite |
| `lib/graphics/` | `graphics_gold`, plus `wrap` (graphics tests live in `tests/docs/`) |
| `tests/common/cross_mode.rs` or `tests/tuple_matrix.rs` | `tuple_matrix` (with `--ignored` + skip pattern) |

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
[TESTING.md § Preferred shape](../../doc/claude/TESTING.md) for the
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

---

## Cross-references

- [TESTING.md](../../doc/claude/TESTING.md) — runtime debugging knobs:
  `LogConfig`, `LOFT_LOG`, dump file format, `LOFT_DUMP_DEPTH`.
- [PROBLEMS.md](../../doc/claude/PROBLEMS.md) — open P-ids; check
  before filing a new one.
- [DEVELOPMENT.md](../../doc/claude/DEVELOPMENT.md) — branch policy,
  commit ordering, push gate.
- [loft-write skill](../loft-write/SKILL.md) — for the loft-source
  side of test bodies (types, syntax, error→fix table).
- `tests/testing.rs` — `code!` + `expr!` macro source.
- `tests/common/cross_mode.rs` — `cross_mode!` harness source.
- `tests/native.rs` — donor of `find_loft_rlib`,
  `compile_native_job`, `run_native_job` helpers.
