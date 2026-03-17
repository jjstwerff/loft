# Testing Framework

## Contents
- [Overview](#overview)
- [Entry Points](#entry-points)
- [The Testing Framework (`tests/testing.rs`)](#the-testing-framework-teststestingrs)
- [Generated Test Files (`tests/generated/`)](#generated-test-files-testsgenerated)
- [Additional Output Files](#additional-output-files)
- [LogConfig — Debug Logging Framework](#logconfig--debug-logging-framework)
- [`tests/wrap.rs` — shared runner for docs and scripts tests](#testswraprs--shared-runner-for-docs-and-scripts-tests)
- [`tests/docs/` — end-to-end loft files (user documentation)](#testsdocs--end-to-end-loft-files)
- [File Layout Summary](#file-layout-summary)
- [Running the Tests](#running-the-tests)
- [Validating Generated Code — the `generated/` Workspace](#validating-generated-code--the-generated-workspace)
- [Key Constraints](#key-constraints)
- [`tests/scripts/` — standalone loft test suite](#testsscripts--standalone-loft-test-suite)
- [Debugging failures in `tests/scripts/`](#debugging-failures-in-testsscripts)

---

## Overview

The loft test suite has two distinct layers:

1. **Interpreter tests** (`tests/*.rs`) — Rust integration tests that parse and run loft code through the full compiler pipeline, validating results, errors, and warnings at the interpreter level.
2. **Generated Rust tests** (`tests/generated/*.rs`) — self-contained Rust files emitted by the interpreter tests (debug builds only) that replay the same logic through the compiled code generator, validating the generated Rust output.

Both layers share a common structure: the interpreter tests drive everything, and the generated tests are a by-product of running them.

---

## Entry Points

### `tests/*.rs` — interpreter test files

Each file is a Cargo integration test (auto-discovered because it lives directly in `tests/`). The test files are:

| File | Contents |
|---|---|
| `expressions.rs` | Type-check tests, labeled loops, mutual recursion, null returns, character appends (simple arithmetic/loop tests live in `tests/scripts/`) |
| `enums.rs` | Complex enum definitions, polymorphism via parent enum, JSON formatting, nested types |
| `strings.rs` | Complex string operations: UTF-8 indexing, reference params, rfind, parsing loops |
| `objects.rs` | Struct creation, `:#` pretty-print format, field references, text independence, mutable reference params |
| `vectors.rs` | Complex vector/sorted/index/hash operations; remove-by-key; for-comprehension; large growth |
| `sizes.rs` | `sizeof` expressions and struct layout (complex struct/collection byte sizes) |
| `data_structures.rs` | Combined data structure behaviour |
| `parse_errors.rs` | Tests that expect specific parse/type errors (all diagnostic — must stay in `.rs`) |
| `immutability.rs` | Immutability diagnostics (`ref never modified`, `const mutated`) |
| `slot_assign.rs` | Stack-slot assignment correctness (no overlapping slots) |
| `log_config.rs` | Unit tests for the `LogConfig` debug-logging framework |
| `threading.rs` | Rust-level parallel API tests (`run_parallel_int`, `WorkerProgram`, `clone_for_worker`) |
| `issues.rs` | Minimal reproducers for known open/fixed issues (see [PROBLEMS.md](PROBLEMS.md)) |
| `expressions_auto_convert.rs` | Auto-conversion edge cases (hand-written) |
| `wrap.rs` | Runs `.loft` files from `tests/docs/`; generates HTML docs |
| `testing.rs` | The framework itself; not a runnable test target |

Each file includes `mod testing;` which pulls in `tests/testing.rs` as a module.

---

## The Testing Framework (`tests/testing.rs`)

### Macros

```rust
code!("loft source code")   // parse and run a block of loft code
expr!("loft expression")    // shorthand: wraps the expression in a test() fn
```

Both macros call into `testing_code` / `testing_expr`, which construct a `Test` struct and capture the Rust function name via `stdext::function_name!()`. The function name is parsed to extract:

- **`self.name`** — the short function name (e.g. `define_enum`)
- **`self.file`** — the containing module name (e.g. `enums`)

These two strings determine where the generated test file is written.

### The `Test` struct

```rust
pub struct Test {
    name: String,         // short test name
    file: String,         // module / file name
    expr: String,         // loft expression to evaluate
    code: String,         // loft code block (may be empty)
    warnings: Vec<String>,
    errors: Vec<String>,
    fatal: Vec<String>,
    sizes: HashMap<String, u32>,
    result: Value,        // expected interpreter result
    tp: Type,             // expected type (when needed)
}
```

### Builder methods

Tests are configured with a fluent builder API before the `Test` is dropped:

| Method | Purpose |
|---|---|
| `.result(Value::...)` | Assert the `test()` function returns this value |
| `.tp(Type::...)` | Override the inferred result type (needed for booleans, enums) |
| `.expr("...")` | Set the loft expression (shorthand for a `test()` routine) |
| `.error("...")` | Expect a specific parse/type error (repeatable) |
| `.fatal("...")` | Expect a fatal parse error |
| `.warning("...")` | Expect a specific warning (repeatable) |

### Execution model — `Drop`

**All test logic runs inside `impl Drop for Test`.** There is no explicit `.run()` call; the test executes automatically when the `Test` value goes out of scope at the end of the `#[test]` function.

The `drop` implementation:

1. Constructs a `Parser` and loads the default library from `default/`.
2. Appends a synthesised `test()` function (see below) when `.expr()` or `.result()` was set.
3. Parses the combined loft source via `p.parse_str(...)`.
4. Validates struct sizes against any `.sizes` entries.
5. Runs `scopes::check` (scope/type analysis).
6. **Debug builds only:** calls `generate_code` (writes `tests/generated/`).
7. Calls `assert_diagnostics` — panics if the actual warnings/errors do not exactly match the expected set.
8. If parsing succeeded: runs `byte_code` + `state.execute("test", ...)`.
9. **Debug builds only:** logs bytecode and execution trace to `tests/dumps/<file>_<name>.txt`.

### Synthesised `test()` function

When `.expr("...")` and `.result(...)` are both set, the framework generates a loft snippet:

```loft
pub fn test() {
    test_value = { <expr> };
    assert(
        test_value == <result>,
        "Test failed {test_value} != <result>"
    );
}
```

When `.result()` is `Value::Null` with a non-unknown type (i.e. testing that the expression returns null), it generates:

```loft
pub fn test() {
    <expr>;
}
```

---

## Generated Test Files (`tests/generated/`)

Generated files are written only in **debug builds** (`#[cfg(debug_assertions)]`). They are produced inside `Test::generate_code`, called from `Drop::drop`.

### `tests/generated/default.rs`

Written on every test execution (overwritten each time). Contains the compiled Rust representation of the default library only — everything up to `start` (the definition count before the test's own loft code was parsed). This file has no `#[test]` function; it serves as a reference snapshot of the default-library schema.

### `tests/generated/<file>_<name>.rs`

Written only when a test has a non-null `.result` or a non-unknown `.tp` (i.e., tests that validate output). The file name is `<file>_<name>.rs` where `<file>` is the Rust module name and `<name>` is the test function name.

For example, the test:
```rust
// in tests/enums.rs
#[test]
fn define_enum() {
    code!("enum Code { ... }")
        .expr("...")
        .result(Value::str("..."));
}
```
produces `tests/generated/enums_define_enum.rs`.

### Structure of a generated file

```rust
#![allow(unused_imports)]
#![allow(unused_parens)]
#![allow(unused_variables)]
#![allow(unreachable_code)]
#![allow(unused_mut)]
#![allow(clippy::unnecessary_to_owned)]
#![allow(clippy::double_parens)]

extern crate loft;
use loft::database::Stores;
use loft::keys::{DbRef, Str, Key, Content};
use loft::external;
use loft::external::*;
use loft::vector;

fn init(db: &mut Stores) {
    // Registers all types from the default library + the test's own types.
    // Enumerations via db.enumerate / db.value.
    // Structs via db.structure / db.field.
    // Ends with db.finish().
    ...
}

fn n_test(stores: &mut Stores) { ... }  // generated Rust translation of the test's loft code

// Additional generated functions for each loft function defined in the test.

#[test]
fn code_<name>() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
```

The `init` function reconstructs the full type schema — both default-library types and any types added by the test — so each generated file is a fully self-contained Rust integration test.

---

## Additional Output Files

### `tests/dumps/<file>_<name>.txt` (debug builds only)

Written by `Test::output_code`. The content is controlled by a `LogConfig` value
selected at test time (see [LogConfig — Debug Logging Framework](#logconfig--debug-logging-framework) below).

Default content (preset `full`):

- The raw loft source code for the test.
- All type definitions introduced by the test (types beyond those in the default library).
- IR (intermediate representation) for each non-default function.
- Bytecode disassembly with slot annotations (`var=name[slot]:type`).
- The execution trace with variable-name annotations on stack-access steps.

Set the `LOFT_LOG` environment variable before running tests to select a different preset.

These files are useful for debugging compiler output and are not committed.

---

## LogConfig — Debug Logging Framework

`src/log_config.rs` provides structured control over what appears in the
`tests/dumps/*.txt` files and in the interpreter's execution trace.

### Selecting a preset at test time

Set the `LOFT_LOG` environment variable before `cargo test`:

```bash
LOFT_LOG=minimal   cargo test --test expressions expr_add   # execution only
LOFT_LOG=static    cargo test --test objects                 # IR + bytecode, no execution
LOFT_LOG=ref_debug cargo test --test objects reference       # snapshots on Ref ops
LOFT_LOG=bridging  cargo test --test expressions             # bridging invariant warnings
LOFT_LOG=crash_tail:20 cargo test --test vectors             # last 20 execution lines
LOFT_LOG=fn:helper cargo test --test expressions             # one function only
LOFT_LOG=variables cargo test --test slot_assign             # variable table per function
```

| `LOFT_LOG` value | Preset | Description |
|---|---|---|
| `full` *(default)* | `LogConfig::full()` | IR + bytecode + execution, slot annotations |
| `static` | `LogConfig::static_only()` | IR + bytecode; no execution trace |
| `minimal` | `LogConfig::minimal()` | Execution for `test` only; no IR/bytecode |
| `ref_debug` | `LogConfig::ref_debug()` | Full + stack snapshots on Ref/CreateStack ops |
| `bridging` | `LogConfig::bridging()` | Execution + bridging-invariant check |
| `crash_tail` or `crash_tail:N` | `LogConfig::crash_tail(N)` | Last N execution lines; flushed on panic |
| `fn:<name>` | `LogConfig::function(name)` | Only the named function |
| `variables` | `LogConfig::variables()` | IR + bytecode + variable table per function (no execution) |
| `all_fns` | `LogConfig::all_fns()` | Bytecode of **all** functions including `default/` built-ins; large but essential for diagnosing crashes whose opcode address falls inside a built-in |

The `variables` preset appends a table after each function's bytecode showing every variable's
name, short type, scope number, stack-slot range `[start, end)`, and live interval `[first_def, last_use]`.
Arguments are marked with `arg`.  Variables that have no slot yet (`stack_pos == u16::MAX`) or
that were never defined still appear so the full picture is visible.  Example:

```
variables for myfile:fn n_find_max(nodes:vector<ref(Node)>) -> integer
  #    arg  name                 type           scope  slot         live
  ----------------------------------------------------------------------
  0    arg  nodes                vec<ref(382)>  0      [0, 12)      -
  1         best                 int            1      [16, 20)     [6, 32]
  2         _vector_1            vec<ref(382)>  2      [20, 32)     [8, 15]
  3         n#index              int            2      [32, 36)     [10, 17]
  4         n                    ref(382)       3      [36, 48)     [19, 28]
```

### `LogConfig` struct

```rust
pub struct LogConfig {
    /// Which phases to include in the output.
    pub phases: LogPhase,           // { ir: bool, bytecode: bool, execution: bool }

    /// Only log IR/bytecode/execution for functions whose name contains one
    /// of these strings.  None = all functions.
    pub show_functions: Option<Vec<String>>,

    /// Only include execution steps whose opcode name (without Op prefix)
    /// contains one of these strings.  None = all opcodes.
    pub trace_opcodes: Option<Vec<String>>,

    /// Keep only the last N lines of the execution trace.  On panic the
    /// buffer is flushed before re-raising.  None = unlimited.
    pub trace_tail: Option<usize>,

    /// Append var=name[slot]:type to bytecode and =varname to execution steps.
    pub annotate_slots: bool,

    /// Capture a stack snapshot after every opcode whose name contains one
    /// of these strings.  None = never snapshot.
    pub snapshot_opcodes: Option<Vec<String>>,

    /// Number of bytes to print per snapshot.
    pub snapshot_window: usize,

    /// Warn when runtime stack_pos deviates from compile-time expected value.
    pub check_bridging: bool,

    /// Print the variable table (name, type, scope, slot, live interval) after
    /// each function's bytecode.  Enabled by the `variables` preset.
    pub show_variables: bool,

    /// Include functions from the `default/` built-in library in the bytecode
    /// dump.  Enabled by `LOFT_LOG=all_fns`; essential for diagnosing crashes
    /// whose opcode address falls inside a built-in.
    pub show_all_functions: bool,
}
```

### Building a custom config

```rust
use loft::log_config::{LogConfig, LogPhase};

let config = LogConfig {
    phases: LogPhase::execution_only(),
    trace_opcodes: Some(vec!["Call".to_string(), "Return".to_string()]),
    annotate_slots: true,
    ..LogConfig::full()
};
```

### Key implementation files

| File | Role |
|---|---|
| `src/log_config.rs` | `LogConfig`, `LogPhase`, `TailBuffer` definitions and presets |
| `src/compile.rs` | `show_code(writer, state, data, config)` — static IR + bytecode output |
| `src/state/debug.rs` | `execute_log(log, name, config, data)` — execution trace with all filters |
| `src/state/debug.rs` | `dump_code(f, d_nr, data, annotate_slots)` — per-function bytecode dump |
| `tests/testing.rs` | Creates config via `LogConfig::from_env()`, passes to `show_code` + `execute_log` |
| `tests/wrap.rs` | Same: `LogConfig::from_env()` for docs/scripts file tests |
| `tests/log_config.rs` | Unit tests covering all filters, presets, and pipeline integration |

### Notes for Claude

- `src/main.rs` re-declares `mod log_config;` because it re-includes all source modules
  directly rather than importing from the library crate.
- The bridging check (`check_bridging: true`) will always report a violation on the
  FIRST instruction of the root test function because `execute_log` places the sentinel
  return address at runtime position 4–7 while compile-time tracking starts at 0.
  This is a known harmless offset, not a real bug.
- `crash_tail` mode wraps the execution loop in `catch_unwind(AssertUnwindSafe(...))`;
  if a panic occurs the tail buffer is flushed to the log file before re-raising.

---

## `tests/wrap.rs` — shared runner for docs and scripts tests

`run_test(path, debug)` is the core of every test in `tests/wrap.rs`:

1. Creates a `Parser`, loads the default library, parses the given `.loft` file.
2. **Fails immediately** if `p.diagnostics` is non-empty — any warning, error, or info
   message causes the test to return `Err(InvalidData)`.  This means `for _ in x` loop
   variables that are never read will fail a test via the "Variable never read" warning.
3. Runs `scopes::check`, `byte_code`, then `state.execute("main", ...)`.
4. In debug builds, writes a bytecode dump to `tests/dumps/<filename>.txt` first.
   If `debug = true`, also writes an execution trace using `execute_log`.

`LOFT_LOG` is respected: `LogConfig::from_env()` is called in `run_test` exactly as in `testing.rs`.

Named test entrypoints in `tests/wrap.rs`:

| Test name | What it runs | Notes |
|---|---|---|
| `dir` | All `tests/docs/*.loft` files + HTML doc regeneration | Skips files listed in `SUITE_SKIP` |
| `loft_suite` | All `tests/scripts/*.loft` files | — |
| `integers` … `stress` | One `tests/scripts/` file each (16 tests) | See `script_test!` table below |
| `last` | `tests/docs/16-parser.loft` | — |
| `threading` | `tests/docs/19-threading.loft` | — |
| `logging` | `tests/docs/20-logging.loft` | — |
| `file_debug` | `tests/docs/13-file.loft` with execution trace | — |
| `parser_debug` | `tests/docs/16-parser.loft` with execution trace | `#[ignore]` — run with `cargo test -- parser_debug --ignored` |

Individual script tests (generated by `script_test!` macro):

| Test name | Script file |
|---|---|
| `integers` | `01-integers.loft` |
| `floats` | `02-floats.loft` |
| `text` | `03-text.loft` |
| `booleans` | `04-booleans.loft` |
| `control_flow` | `05-control-flow.loft` |
| `functions` | `06-functions.loft` |
| `structs` | `07-structs.loft` |
| `enums` | `08-enums.loft` |
| `vectors` | `09-vectors.loft` |
| `collections` | `10-collections.loft` |
| `files` | `11-files.loft` |
| `binary` | `12-binary.loft` |
| `binary_ops` | `13-binary-ops.loft` |
| `formatting` | `14-formatting.loft` |
| `script_threading` | `15-threading.loft` (named `script_threading` to avoid clash with `threading`) |
| `stress` | `16-stress.loft` |
| `map_filter_reduce` | `17-map-filter-reduce.loft` |
| `random` | `18-random.loft` |
| `min_max_clamp` | `19-min-max-clamp.loft` |
| `math_functions` | `20-math-functions.loft` |

Run any single script with `cargo test --test wrap <name>`, e.g.:
```bash
cargo test --test wrap files
cargo test --test wrap collections
```

### WRAP_LOCK — serialisation guard

All `#[test]` functions in `wrap.rs` acquire a process-wide `static Mutex<()>` (`WRAP_LOCK`)
before calling `run_test`. This prevents two tests from executing the same script concurrently
when Cargo runs the test binary with multiple threads (the default). Without this guard,
for example, `loft_suite` and `files` would both execute `11-files.loft` at the same time,
causing filesystem races.

The lock is poisoning-tolerant (`unwrap_or_else(|e| e.into_inner())`): a panicking test
releases the lock and the next test can proceed.

### SUITE_SKIP — skipping known-broken docs files

The `SUITE_SKIP` const in `tests/wrap.rs` lists `tests/docs/` files that are currently broken and
should not block the `dir` test. The `dir` test skips any file whose name is in this list
and prints a note explaining why:

```rust
const SUITE_SKIP: &[&str] = &[
    // (all previously skipped files have been fixed — see CHANGELOG.md)
];
```

`last` runs `16-parser.loft` without a trace; `parser_debug` runs it with a full execution
trace and is marked `#[ignore]` because the trace takes ~100 s. To add a new entry: append
the filename and a comment with the issue number. Remove it once the underlying issue is fixed.

### LOFT_DUMP — controlling debug output in docs/scripts tests

In debug builds, `run_test` (called by `dir`, `loft_suite`, `threading`, etc.) normally
writes a bytecode dump to `tests/dumps/<filename>.txt`. Set `LOFT_DUMP=1` in the environment
to enable this write for non-debug (`debug=false`) test runs:

```bash
LOFT_DUMP=1 cargo test --test wrap dir   # writes bytecode dumps for every docs file
```

Without `LOFT_DUMP=1`, the dump is suppressed for the normal `dir`/`loft_suite` tests
(only written when `debug=true`, i.e. for `file_debug` and `parser_debug`). This avoids
writing ~20 large files during a routine `cargo test` run.


---

## `tests/docs/` — end-to-end loft files

**Purpose: user documentation.** Each file produces one HTML page via `@NAME`/`@TITLE` headers and `//`-comment prose. They are also valid runnable loft programs, so `dir` both regenerates HTML docs and validates the language features shown in each page.

Not connected to the `Test` builder API. The `last` test runs only the final file for fast iteration.

Current docs files (23 files, `00`–`22`):

| File | Topic |
|---|---|
| `00-general.loft` | General language features |
| `01-keywords.loft` | Keyword coverage |
| `02-text.loft` | Text operations |
| `03-integer.loft` | Integer arithmetic |
| `04-boolean.loft` | Boolean logic |
| `05-float.loft` | Floating-point |
| `06-function.loft` | Functions, defaults, recursion |
| `07-vector.loft` | Vectors |
| `08-struct.loft` | Structs |
| `09-enum.loft` | Enums |
| `10-sorted.loft` | Sorted collections |
| `11-index.loft` | B-tree index |
| `12-hash.loft` | Hash collections |
| `13-file.loft` | File I/O |
| `14-image.loft` | PNG images |
| `15-lexer.loft` | Lexer/parser library use |
| `16-parser.loft` | Parser library use |
| `17-libraries.loft` | Library imports and extension methods |
| `18-locks.loft` | Store locking and `const` parameters |
| `19-threading.loft` | Parallel execution (`par(b=worker, threads)` for-loop clause) |
| `20-logging.loft` | Runtime logging (`log_info`, `log_warn`, `log_error`, `log_fatal`) |
| `21-random.loft` | Random numbers (`rand`, `rand_seed`, `rand_indices`) |
| `22-time.loft` | Time functions (`now`, `ticks`) |

---

## File Layout Summary

```
tests/
  testing.rs              # Framework: Test struct, macros, Drop impl, generate_code
  expressions.rs          # Interpreter tests: type-check, labeled loops, null returns
  enums.rs                # Interpreter tests: complex enums, polymorphism, JSON
  strings.rs              # Interpreter tests: complex string ops, reference params
  objects.rs              # Interpreter tests: structs, :#format, mutable references
  vectors.rs              # Interpreter tests: complex vector / sorted / hash
  sizes.rs                # Interpreter tests: struct sizes / sizeof (complex layout)
  data_structures.rs      # Interpreter tests: combined data structures
  parse_errors.rs         # Interpreter tests: expected parser errors (diagnostic)
  immutability.rs         # Interpreter tests: immutability diagnostics
  threading.rs            # Interpreter tests: Rust-level parallel API
  expressions_auto_convert.rs  # Hand-written generated-style test (pre-generator)
  issues.rs               # Regression tests for known issues (see [PROBLEMS.md](PROBLEMS.md))
  wrap.rs                 # Runner for docs/ and scripts/; also generates HTML docs
  docs/
    00-general.loft ... 21-random.loft     # User documentation loft programs (22 files)
    wordlist.txt                           # Edge-case string keys for 21-stress.loft
  generated/
    default.rs            # Default-library schema snapshot (no #[test])
    <file>_<name>.rs      # One file per result-bearing interpreter test
  dumps/
    <file>_<name>.txt     # Bytecode + trace dumps (debug, not committed)
  scripts/
    01-integers.loft ...  # Feature test loft programs (no HTML generation)
    wordlist.txt          # Edge-case string keys for 16-stress.loft
```

---

## Running the Tests

```bash
# Run all interpreter tests (generates tests/generated/ as a side effect):
cargo test

# Run a specific interpreter test file:
cargo test --test enums

# Run a specific test function:
cargo test --test enums define_enum

# Run only docs/scripts tests (wrap.rs):
cargo test --test wrap

# Full test cycle including generated tests (see Makefile):
make test
```

`make test` runs the `clippy` target first (which runs `cargo clippy`, `rustfmt`, and `cargo run --bin gendoc` to regenerate HTML docs), then:

1. Deletes all files in `tests/generated/` and `tests/result/`.
2. Runs `cargo test -- --nocapture --test-threads=1`, appending output to `result.txt`.

---

## Validating Generated Code — the `generated/` Workspace

Two directories:
- `tests/generated/` — ephemeral output from interpreter tests (158+ files, cleared by `make test`)
- `generated/tests/` — committed reviewed subset; standalone Cargo workspace with `loft = { path = ".." }`

| Target | Purpose |
|---|---|
| `make generate` | `meld tests/generated/ generated/tests/` — review and copy approved files into the committed corpus |
| `make gtest` | `cargo clippy --tests`, `rustfmt`, `cargo test` inside `generated/` — lint, format-check, and run all promoted tests |
| `make meld` | Compare `tests/generated/text.rs` and `fill.rs` against `src/` counterparts; open meld if they differ |

```
cargo test (debug)
  └─► tests/generated/*.rs   (158+ files, ephemeral)
        │
        ▼  make generate  (meld review)
        │
        ▼  generated/tests/*.rs  (committed, reviewed subset)
              │
              ▼  make gtest
                   clippy → rustfmt → cargo test  (inside generated/ workspace)
```

---

## Key Constraints

- **Generated tests are debug-only.** `generate_code` and `output_code` are guarded by `#[cfg(debug_assertions)]`. Release builds (`cargo test --release`) skip file generation entirely.
- **`default.rs` has no `#[test]` function** and is excluded from the second-pass Cargo registration.
- **`expressions_auto_convert.rs`** exists as a hand-written `tests/` file from before the generator existed; the corresponding generated file is skipped to avoid a Cargo name collision.
- **Test execution order within a file** is non-deterministic (Cargo runs tests in parallel by default). `make test` passes `--test-threads=1` to force sequential execution and capture output deterministically into `result.txt`.

---

## `tests/scripts/` — standalone loft test suite

**Purpose: the primary, long-term comprehensive test suite for the loft language.**
Every language feature and standard-library function should eventually have coverage here.
Each file is a self-contained loft program with a `fn main()` that asserts correct behaviour.
No HTML generation, no `@NAME`/`@TITLE` headers. Can be run directly through the `loft` binary
or via `cargo test --test wrap loft_suite`.

### Design intent and growth policy

`tests/scripts/` is the canonical place for new tests. When adding a feature, fixing a bug, or
covering an untested language behaviour, the default choice is to extend an existing script or
add a new one — not to add a Rust `.rs` test.

**Add to `tests/scripts/` when:**
- Testing language semantics: operators, control flow, type coercion, collections, formatting, etc.
- Testing standard-library functions.
- Covering an edge case in correct (non-error) code.
- Writing a regression test for a runtime bug fix.

**Add to `tests/*.rs` only when the scenario cannot be expressed as a loft script:**
- The test expects a compile-time error or warning (all `parse_errors.rs`, `immutability.rs`,
  `format_strings.rs` diagnostics).
- The test calls Rust APIs directly (`data_structures.rs`, `threading.rs`, `log_config.rs`,
  `expressions_auto_convert.rs`).
- The test exercises compiler internals that only surface via the Rust test framework
  (`slot_assign.rs`).
- The test is a minimal reproducer for a tracked issue (`issues.rs`), where the tightest
  possible reproduction matters more than script-style readability.

**When a `.rs` test and a script test cover the same behaviour**, the `.rs` test should be removed
— the script is the authoritative version. Both the round-1 and round-2 deduplication passes in
March 2026 removed `.rs` tests on this basis; further passes are welcome whenever a script gains
new coverage that duplicates an existing `.rs` test.

In `cargo test` mode, `run_test` writes a bytecode dump to `tests/dumps/` in debug builds.
No generated Rust code is produced.

```
tests/scripts/
  01-integers.loft    arithmetic, bitwise, null, type conversions
  02-floats.loft      float/single arithmetic, math functions, null (NaN)
  03-text.loft        concatenation, len, indexing, slicing, UTF-8 iteration, search, join
  04-booleans.loft    logical ops, short-circuit, null truthiness
  05-control-flow.loft  if/else, for loops, ranges, break, named break, loop metadata
  06-functions.loft   default args, reference params, early return, recursion
  07-structs.loft     constructors, methods, virtual fields, JSON/format, vectors of structs
  08-enums.loft       plain enums, struct-enum variants, polymorphic dispatch
  09-vectors.loft     literals, append, slice, iteration, removal, #index/#first/#count
  10-collections.loft sorted, index, hash — lookup, ordered iteration, range queries
  11-files.loft       text file I/O: lines(), move/delete, path safety, file().files() listing
                      Note: lines() test placed last to avoid Issue 29 slot overlap
  12-binary.loft      binary file I/O: typed reads/writes (u8/u16/i32/long/single/float/text/integer-vector), endianness, mixed
  13-binary-ops.loft  binary file operations: seek+overwrite, set_size (truncate/extend), incomplete read, size field
  14-formatting.loft  format specifiers: integers, floats, booleans, text, long, single, vectors
  15-threading.loft   parallel_for, fn references, worker functions
  16-stress.loft      build-and-free cycles for all 4 collection types; reads wordlist.txt
  17-map-filter-reduce.loft  map, filter, reduce higher-order functions
  18-random.loft      rand, rand_seed, rand_indices — range, reproducibility, permutation
  19-min-max-clamp.loft  min, max, clamp for integer, long, single, float; null propagation
  20-math-functions.loft  exp, ln, log2, log10 for single and float; null propagation
  wordlist.txt        edge-case string keys: fruits, short strings, alphabet, digits, spaces, duplicates
```

Run with:

```bash
cargo test --test wrap loft_suite   # run all tests/scripts/ files via the test framework
make loft-test                      # build loft (release) then run every file
./target/release/loft tests/scripts/07-structs.loft   # run one file
```

The `cargo test` path uses `run_test` from `tests/wrap.rs`, which:
- Fails on any compiler diagnostic (including warnings such as "Variable never read")
- Writes a bytecode dump to `tests/dumps/<filename>.txt` in debug builds
- Respects `LOFT_LOG` for the bytecode dump

Each file has a `fn main()` that calls `assert(condition, message)` for every case.
A failing assert panics and prints the message, naming the failed test.

### Known language quirks affecting test authoring

The following behaviours differ from what one might naively expect:

| Behaviour | Correct approach |
|---|---|
| `for _ in text_var` → "Variable never read" warning → test fails | Use a named variable, or restructure to avoid iterating text just for a count |
| ~~`for _ in enum_vector` → infinite loop~~ **FIXED** | `for x in v` now terminates correctly for `vector<PlainEnum>` |
| `empty = []` → "Indexing a non vector" compile error | Use a typed one-element vector then remove it: `t = [99]; for v in t { v#remove; }` |
| `"Purple" as Direction` returns `0`, not null sentinel `255` | Check format string: `"{bad}" == "null"` rather than `!bad` |
| `#index` in `for i in 10..14` returns the loop variable value (10–13), not 0-based count | Use `#count` for 0-based counting; `#index == loop_var` for integer ranges |
| Default struct integer fields are `0`, not null | Assert `== 0`, not `== null` |
| Same variable name in multiple sequential `{ }` blocks: `validate_slots` exempts same-name+same-slot pairs (Issue 28, fixed) | Both same-name and different-name sequential blocks now work |
| Two *differently-named* reference/vector/text variables in a long function that share a slot and have overlapping `first_def`/`last_use` intervals trigger a false `validate_slots` panic (Issue 29, unfixed) | Order the code so the second variable is introduced after the last use of the first; see `11-files.loft` (`lines()` test placed last) |
| `to_uppercase` / `to_lowercase` / `replace` return `Str` (16 bytes), not `String` (24 bytes) | Use `stores.scratch` pattern (see [INTERNALS.md](INTERNALS.md)) |
| ~~`for r in sorted if cond { r#remove; }` with large N gives silently wrong results~~ **FIXED 2026-03-14** (PROBLEMS #33 — no actual bug; test confirmed passing) | — |
| ~~`for r in index_var { r#remove; }` with large N panics "Unknown record"~~ **FIXED 2026-03-14** (PROBLEMS #35 — `fill_iter` loop_db_tp and `state::remove` both fixed) | — |
| ~~`for i in 0..N { idx[i, name] = null; }` leaves 1 record behind (large N)~~ **FIXED 2026-03-14** (PROBLEMS #34 — `tree::remove` now always updates root pointer even when last element removed) | — |
| `long` is a reserved type keyword — `long = "..."` fails with "Not implemented operation = for type null" | Use a different variable name (e.g. `alphabet`, `longstr`) |

---

## Debugging failures in `tests/scripts/` {#debugging-failures-in-testsscripts}

### Strategy overview

When `make loft-test` reports a failure, work from the outside in:

1. **Run the failing file directly** — the panic message names the exact assert.
2. **Narrow to the failing assert** — comment out asserts below the first failure to isolate it.
3. **Print intermediate values** — add `print("{var}")` before the assert to see the actual value.
4. **Run via the Rust test framework** — convert the minimal case to `expr!(...)` in a `tests/*.rs`
   file; this enables `LOFT_LOG` debug output without modifying the source.
5. **Use the debug binary** — `cargo build --bin loft` produces a binary with extra runtime
   checks; segfaults often produce clearer output or trigger a Rust panic instead.

### Failure types and fixes

#### Assert fires with wrong value

```
panicked at src/fill.rs:1772:5: my assert message
```

The message is whatever string was passed as the second argument to `assert()`.
Add `print("{actual}")` directly before the failing assert to see the actual value.
Common causes:
- Off-by-one in an expected range or loop count — trace manually.
- Floating-point rounding — use `round()` before comparing or widen the tolerance.
- Format output differs from expected — print both sides and compare byte-by-byte.

#### Segfault (no output)

```
Segmentation fault (core dumped)
```

The interpreter hit an unguarded memory access.  Run the debug binary for a Rust panic
instead of a silent crash:

```bash
cargo build --bin loft          # debug build, slower but safer
./target/debug/loft tests/scripts/08-enums.loft
RUST_BACKTRACE=1 ./target/debug/loft tests/scripts/08-enums.loft
```

Common causes:
- Calling a feature that is not yet implemented (e.g. `enum_value as integer`,
  unimplemented stdlib method) — the interpreter falls through to an unreachable branch.
- Passing a wrong type where the runtime expects a specific layout (e.g. a struct-enum
  variant used as a plain enum).
- Remove the suspect line; if the segfault disappears, the line triggers the bug.

#### Parse error — "Dual definition of"

```
Dual definition of <name> at file.loft:line:col
```

A name is defined twice in the same scope.  Common triggers:

- **Nested format string with escaped quotes**: `"outer {\"inner\"}"` — previously the
  lexer treated `\"` as ending the outer string. This was fixed in 2026-03-14 via the
  `in_format_expr` flag in `src/lexer.rs`; `\"` inside `{...}` now works correctly.
- **Two struct definitions with the same field name in the same file**: when two structs
  both have a field called e.g. `key`, the parser may confuse their field numbers during
  type resolution.  Rename one field or split the structs into separate test files.
- **Re-declaring a function with identical parameter types**: loft allows overloading by
  type; identical signatures are an error.

#### Parse error — "Undefined type"

A type name appears before its `struct`/`enum` definition.  Move the definition above its
first use, or above any function that references it.

#### Wrong result from index range query

If a range query like `db.map[83..92, "Two"]` returns unexpected elements, the most likely
cause is a **field-offset conflict**: two structs defined in the same file share a field
name at different positions.  For example:

```loft
struct A { key: text }           // key is field 0
struct B { nr: integer, key: text }  // key is field 1
```

When both `sorted<A[-key]>` and `index<B[nr,-key]>` exist in the same file, the compiler
may resolve `key` to the wrong field number for one of the lookups.

Fix: use distinct field names, or place conflicting struct definitions in separate test files.

#### Compile error — "Cannot add elements to '...' while it is being iterated"

```
Error: Cannot add elements to 'v' while it is being iterated — use a separate collection or add after the loop
Error: Cannot add elements to a collection while it is being iterated — use a separate collection or add after the loop
```

This is a deliberate compile-time guard. Appending to a collection during iteration is
unsafe: vectors re-read their length on every step (so new elements are visited, risking
an infinite loop), and sorted/index insertions corrupt stored iterator positions.

**Fix options:**
- Collect additions in a separate variable and append after the loop: `extra = []; ... for e in v { ... extra += [x]; } v += extra;`
- Remove elements during iteration with `e#remove` in a filtered loop — this is the one safe in-loop mutation.

**Scope:** The guard covers both direct variable mutations (`v += x`) and field-access
mutations (`db.items += x`) as of 2026-03-14.

#### Wrong iteration order in sorted/index

Verify the sort direction: `-field` means **descending**, `field` means **ascending**.
A mismatch between the declared direction and the expected order is the most common mistake.
Trace the expected element sequence manually before writing the assert.

---

## See also
- [PROBLEMS.md](PROBLEMS.md) — Known bugs, limitations, workarounds, and fix plans
- [QUICK_START.md](QUICK_START.md) — Compact orientation: execution path, key data structures, conventions
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy (LOFT_LOG presets, scope bugs, slot conflicts), working with Claude
