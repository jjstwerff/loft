---
render_with_liquid: false
---
# Test Coverage Gaps

Last updated 2026-04-02.  Measurement with `cargo llvm-cov --summary-only`:
Overall: **71.3% line / 74.9% function** (previous: 70.8% / 75.9%).

---

## Resolved since last measurement

The following gaps from the previous version are now covered by `tests/scripts/*.loft`:

| Gap | Resolution |
|---|---|
| Math domain boundaries (`native.rs`) | `02-floats.loft` lines 79–94: log(0), log(-1), sqrt(-1), asin(2), acos(-2), pow(0,0), pow(-1,0.5) |
| Text-to-number parsing | `01-integers.loft` lines 143–150: "abc", "123abc", "", " 42 " as integer all return null |
| Float comparison epsilon | `02-floats.loft` lines 105–124: boundary tests for float (1e-9) and single (1e-6) tolerances |
| Null sentinel edge cases (`ops.rs`) | `01-integers.loft` lines 112–136: bitwise ops on null, negative shift, shift of null; `02-floats.loft` lines 137–141: NaN comparisons |
| Type conversion edge cases | `02-floats.loft` lines 126–144: NaN/Infinity as integer/long, truncation toward zero; `54-auto-convert.loft`: mixed-type widening, i64 precision |
| Logger functions | `53-logging.loft`: all four log levels, format interpolation, control flow |
| `codegen_runtime.rs` (0% coverage) | `native_scripts()` in `native.rs` now compiles all 56 scripts through the native backend |
| `parser/builtins.rs` (par parsing) | `22-threading.loft` lines 77–180: 14 par() variants (forms 1/2, return types, context args) |
| Closures / lambda capture | `56-closures.loft`: integer/text capture, timing, factory functions, HOFs |
| JSON parsing / serialization | `57-json.loft`: serialize (:j), .parse(), cast (as), nested structs, vectors, #errors |
| Constraints (L6) | `58-constraints.loft`: field assert, cross-field, vector elements, constraint + parse |
| Store locks | `59-locks.loft`: const params/locals, #lock attribute, get_store_lock() |
| char+char / text index ops | `60-char-text-ops.loft`: char concatenation, text index addition, void-body lambda |
| stack_trace() | `55-stack-trace.loft`: nested calls, function name verification |
| Auto type conversion | `54-auto-convert.loft`: int*long, int+float, long+float, comparisons, i64 precision loss |
| Non-ASCII character predicates | `03-text.loft`: is_lowercase/is_uppercase/is_alphabetic on accented characters, to_uppercase/to_lowercase on non-ASCII text |
| Yield inside par() rejected | `36-parse-errors.loft`: `@EXPECT_ERROR` for yield in par body |
| `native.rs` critically low (~30%) | Now 78.1% line — 56 scripts compile through native backend |

---

## Files with 0% coverage

| File | Reason | Action |
|---|---|---|
| `src/documentation.rs` (1238 lines) | HTML doc generation | Covered by `gendoc` binary; no Rust unit tests |
| `src/gendoc.rs` (1495 lines, 1.9%) | HTML doc binary | Same as above |
| `src/main.rs` (622 lines, 20.1%) | CLI entry point | Exit-code tests cover parse-error path; most CLI flags untested |
| `src/radix_tree.rs` (311 lines) | Planned feature, `#[allow(dead_code)]` | Iterator `.next()` is a stub (`None`); entire module unused — blocked on feature completion |
| `src/test_runner.rs` (1226 lines) | Test harness infrastructure | Only used when running tests; not itself tested |

---

## Files with critically low coverage (< 50%)

| File | Line cover | Function cover | Key gaps |
|---|---|---|---|
| `src/native_utils.rs` | 12.3% | 9.1% | WASM/installed-layout paths, rlib rebuild triggers, project root detection |
| `src/database/allocation.rs` | 38.6% | 71.0% | Store growth paths, boundary conditions — see §Database below |
| `src/logger.rs` | 39.3% | 52.0% | Production-mode paths, config reload, rotation, rate limiting — see §Logger below |
| `src/extensions.rs` | 45.5% | 33.3% | Plugin dedup, library load/symbol failures, WASM feature gate |
| `src/variables/validate.rs` | 45.6% | 85.7% | Scope cycle detection, sibling conflicts, diagnostic formatting |
| `src/database/search.rs` | 46.5% | 57.9% | Multi-key range queries, zero-result ranges, index iteration — see §Database below |

---

## Per-file coverage snapshot

Files above 50% with notable gaps or recent changes:

| File | Line % | Func % | Notes |
|---|---|---|---|
| `src/parser/builtins.rs` | 56.0 | 78.6 | `par()` parsing covered; remaining: error recovery paths |
| `src/database/mod.rs` | 60.6 | 80.0 | Database init/schema paths |
| `src/keys.rs` | 63.5 | 55.2 | Float/enum key comparisons, offset arithmetic, NaN — see §DbRef |
| `src/database/io.rs` | 66.2 | 73.7 | File I/O record ops |
| `src/state/debug.rs` | 67.6 | 70.4 | Dump/trace helpers |
| `src/vector.rs` | 67.8 | 64.0 | `reverse_vector()` 0 hits, 8-byte sort paths — see §Vector |
| `src/log_config.rs` | 72.9 | 88.0 | |
| `src/ops.rs` | 73.8 | 81.9 | |
| `src/lexer.rs` | 74.8 | 89.9 | |
| `src/parser/control.rs` | 74.9 | 90.5 | |
| `src/state/codegen.rs` | 76.3 | 93.8 | |
| `src/native.rs` | 78.1 | 85.9 | Major improvement from ~30%; 56 scripts pass native |
| `src/fill.rs` | 86.5 | 84.4 | 233 opcodes; remaining: rarely-used ops |

---

## Remaining gap areas

### 1. Database / store boundary conditions (`src/database/allocation.rs`, 38.6%)

**Uncovered paths** — Rust-internal tests (`tests/issues.rs` or `tests/limits.rs`):
- Allocation just below `MAX_STORE_WORDS` — verify no panic
- Two consecutive large allocations that together exceed `MAX_STORE_WORDS` — verify correct panic message
- `vector<T>` with a very large number of elements (10,000+) — verify growth, iteration, and removal all work
- `sorted<T>` with 10,000+ elements — exercises radix tree deep traversal

**Store search paths** (`src/database/search.rs`, 46.5%):
- Range queries on `sorted<T[k1, k2]>` with multi-key bounds
- Range queries returning zero results
- Range queries where lower bound > upper bound
- `index<T[id]>` range iteration
- Hash/index remove paths

---

### 2. Vector operations (`src/vector.rs`, 67.8%)

**Script-testable:**
- `reverse_vector()` — 0 hits; needs a `.loft` script that calls vector reverse
- `sort_vector()` 8-byte element paths (long/float sort) — 0 hits
- `sort_vector()` 1-byte and 2-byte element paths — 0 hits
- `vector_step()` forward single-step — related to iteration but different semantics

---

### 3. Parser stress / error recovery (`src/parser/`)

Not feasible as `.loft` script tests (require Rust harness for cascade error checking):

- **Deep nesting**: `((((((((((0))))))))))` to ~100 levels — stack overflow or useful error?
- **Long format strings**: 1,000+ character string with 100+ interpolations
- **Cascading errors**: single typo causing 10+ downstream parse errors — verify messages are actionable

---

### 4. Logger (`src/logger.rs`, 39.3%)

Partly addressed by `tests/logger_severity.rs` and `53-logging.loft`.

Still missing (Rust-internal only):
- Production mode: `log_fatal` in production mode — verify `had_fatal` is set instead of panicking
- `LOFT_LOG=crash_tail:N` env var — verify last N lines are flushed on panic
- Config hot-reload path — rate limiting between `check_reload()` calls
- File rotation: daily boundary detection, `max_files` cleanup
- Per-file severity overrides (path prefix matching)
- Rate-limit window reset notices

---

### 5. Native utilities (`src/native_utils.rs`, 12.3%)

Rust-internal only:
- `loft_lib_dir_for()` — WASM target lookups, installed-layout fallbacks
- `ensure_rlib_fresh()` — cargo rebuild branch
- `newest_mtime_in()` — directory walk error handling
- `project_dir()` — platform detection branches

---

### 6. Reference / DbRef operations (`src/keys.rs`, 63.5%)

Rust-internal tests only:
- Null `DbRef` dereference — what error is produced?
- `store_nr` out of range (index into a non-existent store)
- Record offset beyond the allocated record size
- Float/enum key comparisons — NaN/Infinity edge cases
- `DbRef::plus()` / `DbRef::min()` offset arithmetic

---

### 7. Slot validation (`src/variables/validate.rs`, 45.6%)

Rust-internal only:
- Scope cycle detection (assertion path)
- Sibling-scope conflict skipping
- Work-variable aliasing with `skip_free` flag (S34 optimization)
- Diagnostic formatting paths

---

### 8. Extensions / plugin loading (`src/extensions.rs`, 45.5%)

Rust-internal only:
- Plugin deduplication (`loaded.contains()` branch)
- Library open/symbol lookup failures
- WASM feature gate codepath

---

### 9. Collection mutation patterns

**Script-testable but blocked by parser bugs:**
- `s.field = [vector_literal]` — parser panic (index out of bounds); tracked in PROBLEMS.md
- **Sorted remove during iteration**: `OpRemove` inside `for r in sorted { ... }` loop — untested

**Rust-internal only:**
- Hash remove with non-existent key — verify no panic, returns null
- `vector<ref(T)>` — append, iterate, remove references

---

## Resolved test infrastructure issues

| Issue | Resolution |
|---|---|
| `tests/wrap.rs` only called `main()` — 20 scripts with `fn test_*()` were never executed by `cargo test` | `run_test` now discovers all zero-param user functions; `loft_suite` no longer filters on `fn main(`. Scripts 56–60 and 10–40 (test-style) now run during `cargo test` and contribute to `cargo llvm-cov`. |
| `06-structs.loft` pre-existing failures (`test_colours`, `test_vector_argument`) | Annotated `@EXPECT_FAIL`; `wrap.rs` now respects `@EXPECT_FAIL`, `@EXPECT_ERROR`, `@EXPECT_WARNING` annotations |
| `31-vectors.loft` undeclared parse errors | Added `@EXPECT_ERROR` annotations for slice-to-vector type errors |
| `36-parse-errors.loft` missing `PECycB` error annotation | Added second `@EXPECT_ERROR: 'PECycB' contains itself` |

## Known test issues

| Test | Issue | Status |
|---|---|---|
| `exit_codes::warning_only_program_exits_zero` | Fails under `cargo llvm-cov` instrumentation (exits 101 instead of 0); passes under normal `cargo test` | Environment-specific; does not affect real coverage |

---

## Known native codegen limitations

| Script | Issue | Status |
|---|---|---|
| `56-closures.loft` | Was blocked by two bugs: `default_native_value` for `Type::Function` emitted `0_u32` instead of tuple; `format_text` didn't wrap `Value::CallRef` text returns with `&*` | **Fixed** — both bugs resolved, all 56 scripts pass native |

---

## Features tested only in `tests/*.rs` (not reproducible as `.loft` scripts)

These are inherently Rust-API tests. They don't need `.loft` script equivalents:

| Feature | Rust test file | Why not scriptable |
|---|---|---|
| Parallel worker API (`run_parallel_*`) | `threading.rs` | Directly calls Rust parallel infrastructure |
| Data structures API (Stores/tree/hash) | `data_structures.rs` | Raw Rust store operations |
| Logger severity routing | `logger_severity.rs` | Directly calls `Logger::log()` |
| Production mode (`had_fatal`) | `issues.rs` | Tests Rust `State` flag |
| Code generation correctness (N-series) | `issues.rs` | Checks generated `.rs` file content |
| Module system (`use lib::*`) | `imports.rs` | `run_test` doesn't configure `lib_dirs` |
| Code formatter roundtrips | `format.rs` | Tests Rust formatter API |
| Native compilation pipeline | `native.rs` | Tests `rustc` compilation of generated code |
| Native library loading | `native_loader.rs` | Tests cdylib loading |
| WASM compilation | `wasm_entry.rs` | Tests wasm target |

---

## Priority order

1. **Vector reverse/sort** — `.loft` script test; low effort, closes `reverse_vector()` 0% gap.
2. **Database store boundaries** — `limits.rs` or `issues.rs`; important for correctness under real workloads.
3. **Database range queries** — `.loft` scripts with multi-key sorted collections; moderate effort.
4. **Parser stress / error recovery** — new `parser_stress.rs`; medium effort, high value for robustness.
5. **Logger production mode + rotation** — add to existing `logger_severity.rs`; low risk.
6. **DbRef edge cases** — add to `data_structures.rs`; low effort.
7. **Slot validation paths** — add synthetic IR tests to `tests/slots.rs`.
8. **Native utils** — Rust unit tests with filesystem mocking; moderate effort.
9. **Sorted remove during iteration** — blocked until sort-iterate-remove is properly supported.
10. **Struct field vector literal reassignment** — blocked by parser bug.

---

## See also
- [TESTING.md](TESTING.md) — How to write tests, use `LOFT_LOG` presets, and add script/doc test files
- [PROBLEMS.md](PROBLEMS.md) — Open bugs that block or reduce coverage in specific areas
- [PLANNING.md](PLANNING.md) — Enhancement backlog; coverage gaps often align with planned work
