# Test Coverage Gaps

Last updated 2026-04-02.  Previous measurement (2026-03-20) with `cargo llvm-cov`:
Overall: **70.8% line / 75.9% function**.

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

---

## Files with 0% coverage

| File | Reason | Action |
|---|---|---|
| `src/documentation.rs` | HTML doc generation | Covered by `gendoc` binary; no Rust unit tests |
| `src/gendoc.rs` | HTML doc binary | Same as above |
| `src/main.rs` | CLI entry point | No integration tests invoke the binary directly |

---

## Files with critically low coverage (< 50%)

| File | Line cover | Function cover | Key gaps |
|---|---|---|---|
| `src/native.rs` | ~30% | ~35% | Many stdlib functions only exercised via native compilation, not interpreter coverage |
| `src/database/allocation.rs` | 43.0% | 68.2% | Store growth paths, boundary conditions — see §Database below |
| `src/logger.rs` | ~40% | ~55% | Production-mode paths, crash_tail logging |

---

## Remaining gap areas

### 1. Database / store boundary conditions (`src/database/allocation.rs`, 43.0%)

**Uncovered paths** — these are Rust-internal tests (`tests/issues.rs` or `tests/limits.rs`):
- Allocation just below `MAX_STORE_WORDS` — verify no panic
- Two consecutive large allocations that together exceed `MAX_STORE_WORDS` — verify correct panic message
- `vector<T>` with a very large number of elements (10,000+) — verify growth, iteration, and removal all work
- `sorted<T>` with 10,000+ elements — exercises radix tree deep traversal

**Store search paths** (`src/database/search.rs`, 49.7%):
- Range queries on `sorted<T[k1, k2]>` with multi-key bounds
- Range queries returning zero results
- Range queries where lower bound > upper bound
- `index<T[id]>` range iteration

---

### 2. Parser stress / error recovery (`src/parser/`)

Not feasible as `.loft` script tests (require Rust harness for cascade error checking):

- **Deep nesting**: `((((((((((0))))))))))` to ~100 levels — stack overflow or useful error?
- **Long format strings**: 1,000+ character string with 100+ interpolations
- **Cascading errors**: single typo causing 10+ downstream parse errors — verify messages are actionable

---

### 3. Logger (`src/logger.rs`)

Partly addressed by `tests/logger_severity.rs` (Rust-internal) and `53-logging.loft` (script).

Still missing (Rust-internal only):
- Production mode: `log_fatal` in production mode — verify `had_fatal` is set instead of panicking
- `LOFT_LOG=crash_tail:N` env var — verify last N lines are flushed on panic

---

### 4. Reference / DbRef operations (`src/keys.rs`, 74.3%)

Rust-internal tests only:
- Null `DbRef` dereference — what error is produced?
- `store_nr` out of range (index into a non-existent store)
- Record offset beyond the allocated record size

---

### 5. Collection mutation patterns

**Script-testable but blocked by parser bugs:**
- `s.field = [vector_literal]` — parser panic (index out of bounds); tracked in PROBLEMS.md
- **Sorted remove during iteration**: `OpRemove` inside `for r in sorted { ... }` loop — untested

**Rust-internal only:**
- Hash remove with non-existent key — verify no panic, returns null
- `vector<ref(T)>` — append, iterate, remove references

---

### 6. Features tested only in `tests/*.rs` (not reproducible as `.loft` scripts)

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

## Known native codegen limitations

| Script | Issue | Status |
|---|---|---|
| `56-closures.loft` | Was blocked by two bugs: `default_native_value` for `Type::Function` emitted `0_u32` instead of tuple; `format_text` didn't wrap `Value::CallRef` text returns with `&*` | **Fixed** — both bugs resolved, all 56 scripts pass native |

---

## Priority order

1. **Database store boundaries** — `limits.rs` or `issues.rs`; important for correctness under real workloads.
2. **Parser stress / error recovery** — new `parser_stress.rs`; medium effort, high value for robustness.
3. **Logger production mode** — low risk; add to existing `logger_severity.rs`.
4. **DbRef edge cases** — add to `data_structures.rs`; low effort.
5. **Sorted remove during iteration** — blocked until sort-iterate-remove is properly supported.
6. **Struct field vector literal reassignment** — blocked by parser bug.

---

## See also
- [TESTING.md](TESTING.md) — How to write tests, use `LOFT_LOG` presets, and add script/doc test files
- [FAILURES.md](FAILURES.md) — Historical failure analysis that informed the coverage measurement baseline
- [PROBLEMS.md](PROBLEMS.md) — Open bugs that block or reduce coverage in specific areas
- [PLANNING.md](PLANNING.md) — Enhancement backlog; coverage gaps often align with planned work
