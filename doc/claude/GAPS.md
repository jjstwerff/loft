# Test Coverage Gaps

Coverage measured 2026-03-20 with `cargo llvm-cov` (passing tests only, 23 known failures excluded — see [FAILURES.md](FAILURES.md)).
Overall: **70.8% line / 75.9% function**.

---

## Files with 0% coverage

These files are entirely untouched by the current test suite.

| File | Reason | Action |
|---|---|---|
| `src/radix_tree.rs` | Used by `sorted`/`index`/`hash` — those tests all fail (slot-conflict bug) | Fix Bug 1 from FAILURES.md first; coverage will follow automatically |
| `src/parser/builtins.rs` | Parallel worker helpers (`par(...)`) — no test exercises the parallel parser path | Add `par(...)` parse tests to `tests/threading.rs` or a new `tests/parallel_parse.rs` |
| `src/codegen_runtime.rs` | Native code generation path (`cargo run --bin loft -- --generate`) | Needs a `tests/codegen_runtime.rs` exercising the generation pipeline |
| `src/documentation.rs` | HTML doc generation | Covered by `gendoc` binary; no Rust unit tests |
| `src/gendoc.rs` | HTML doc binary | Same as above |
| `src/main.rs` | CLI entry point | No integration tests invoke the binary directly |

---

## Files with critically low coverage (< 50%)

| File | Line cover | Function cover | Key gaps |
|---|---|---|---|
| `src/native.rs` | 25.8% | 31.6% | 39 of 57 stdlib functions never called — see §Native below |
| `src/database/allocation.rs` | 43.0% | 68.2% | Store growth paths, boundary conditions — see §Database below |
| `src/logger.rs` | 34.7% | 54.2% | Rate-limiting, production-mode paths, log levels other than `info` |

---

## Detailed gap areas

### 1. Native stdlib functions (`src/native.rs`, 25.8%)

The native registry has 57 functions; only 18 are exercised. Uncovered groups:

**Math domain boundaries** — add to `tests/scripts/02-floats.loft` or a new `tests/native_math.rs`:
- `log(0)` → `-Infinity`; `log(-1)` → `NaN`
- `sqrt(-1)` → `NaN`
- `asin(2)`, `acos(-2)` → `NaN` (out of domain)
- Trig on very large angles (precision loss)
- `pow(0, 0)` → `1`; `pow(-1, 0.5)` → `NaN`

**`single` (f32) arithmetic** — no test uses `single` type at all:
- `add_single`, `mul_single`, `div_single`, `rem_single`
- NaN / Infinity propagation for f32 (different precision than f64)
- Comparison epsilon: `0.000001` for single vs `0.000000001` for float

**Text-to-number parsing**:
- `"abc" as integer` — what is returned?
- `"123abc" as integer` — partial parse result?
- `"" as integer` — empty string conversion
- `" 42 " as integer` — leading/trailing whitespace

**Character / codepoint functions** — covered in `strings.rs` basic paths only:
- `is_numeric`, `is_alpha`, `is_upper`, `is_lower` on boundary codepoints (U+0000, U+007F, U+0080, U+10FFFF)
- `to_upper` / `to_lower` on non-ASCII characters

---

### 2. Null sentinel edge cases (`src/ops.rs`, 65.8%)

Null sentinels: `i32::MIN` (integer null), `i64::MIN` (long null), `f64::NAN` (float null).

**Unverified behaviours** — add to `tests/scripts/01-integers.loft` and `tests/scripts/02-floats.loft`:

| Expression | Expected | Tested? |
|---|---|---|
| `i32::MIN & 5` | null | No |
| `i32::MIN \| 5` | null | No |
| `i32::MIN ^ 5` | null | No |
| `5 << 32` | undefined / null | No |
| `5 << -1` | undefined / null | No |
| `(-5) >> 1` | negative (sign-extended) | No |
| `nan < 0.0` | false | No |
| `nan >= nan` | false | No |
| `0.0 / 0.0` | NaN | No |
| `1.0 / 0.0` | Infinity | No |
| `inf + inf` | Infinity | No |
| `inf - inf` | NaN | No |
| `inf * 0.0` | NaN | No |

**Shift validation** — `src/ops.rs` has `assert!((0..32).contains(&v2))` in debug only; release builds are unchecked:
- Shift by 32 or more (should produce null or error, not silent UB)
- Shift with null operand (`i32::MIN << 1`)

---

### 3. Type conversion edge cases (`src/ops.rs` cast functions)

Add to `tests/expressions_auto_convert.rs`:

| Conversion | Expected | Tested? |
|---|---|---|
| `NaN as integer` | `i32::MIN` (null) | No |
| `Infinity as integer` | implementation-defined | No |
| `-Infinity as long` | implementation-defined | No |
| `i64::MAX as float` | precision loss | No |
| `i32::MIN as float` | -2147483648.0 (exact) | No |
| `i32::MIN as long` | `i64::MIN` (still null) | No |
| `3.9 as integer` | `3` (truncation) | Yes (partial) |
| `(-3.9) as integer` | `-3` (truncation toward zero) | No |

---

### 4. Float comparison epsilon (`src/fill.rs`)

The equality operators use a tolerance: `0.000001` for `single`, `0.000000001` for `float`.
No test verifies these thresholds. Add to `tests/scripts/02-floats.loft`:

```loft
// exactly at epsilon boundary
assert(1.0 + 0.0000000005 == 1.0);   // within float epsilon → equal
assert(1.0 + 0.0000000015 != 1.0);   // outside float epsilon → not equal
assert(1.0s + 0.0000005s == 1.0s);   // within single epsilon → equal
assert(1.0s + 0.0000015s != 1.0s);   // outside single epsilon → not equal
```

---

### 5. Database / store boundary conditions (`src/database/allocation.rs`, 43.0%)

**Uncovered paths** — add to `tests/issues.rs` or a new `tests/limits.rs`:
- Allocation just below `MAX_STORE_WORDS` — verify no panic
- Two consecutive large allocations that together exceed `MAX_STORE_WORDS` — verify correct panic message
- `vector<T>` with a very large number of elements (10,000+) — verify growth, iteration, and removal all work
- `sorted<T>` with 10,000+ elements — exercises radix tree deep traversal (also covers `src/radix_tree.rs`)

**Store search paths** (`src/database/search.rs`, 49.7%):
- Range queries on `sorted<T[k1, k2]>` with multi-key bounds
- Range queries returning zero results
- Range queries where lower bound > upper bound
- `index<T[id]>` range iteration (currently untested per coverage data)

---

### 6. Parser stress / error recovery (`src/parser/`, mixed coverage)

Add a new `tests/parser_stress.rs`:

- **Deep nesting**: `((((((((((0))))))))))` to ~100 levels — does the parser stack-overflow or return a useful error?
- **Long format strings**: 1,000+ character string with 100+ interpolations
- **Malformed format specifiers**: `"{x:"` (no closing `}`), `"{:}"` (empty variable name)
- **Cascading errors**: single typo that causes 10+ downstream parse errors — verify error messages are actionable and don't repeat the same location
- **`use` after declaration**: `use` statement appearing after a `fn` definition — should produce a clear error
- **Duplicate field names across structs in the same file**: the Quick Start notes this causes "Unknown field" errors in collections, but there is no explicit test

---

### 7. Reference / DbRef operations (`src/keys.rs`, 74.3%)

Add to `tests/issues.rs` or `tests/data_structures.rs`:

- Null `DbRef` dereference — what error is produced?
- `store_nr` out of range (index into a non-existent store)
- Record offset beyond the allocated record size
- `DbRef` comparison: two refs to the same record are equal; two refs to different records in different stores

---

### 8. Collection mutation patterns (`src/vector.rs`, `src/hash.rs`)

Currently missing from `tests/vectors.rs`:

- **Hash remove**: `h["key"] = null` removes entry — basic test exists, but null-key and non-existent-key cases are untested
- **Sorted remove during iteration** (Bug 1 tests, once fixed): verify that `OpRemove` inside a `for r in sorted { ... }` loop does not corrupt the iterator
- **Struct field reassignment**: `s.v = [item1, item2]` where `v` is a `vector<T>` field — tests only cover scalar field mutation
- **Vector of references**: `vector<ref(T)>` — append, iterate, remove

---

### 9. Logger (`src/logger.rs`, 34.7%)

Add to `tests/log_config.rs`:

- `log_warn` and `log_error` paths (only `log_info` is exercised)
- Rate-limiting: same message logged >N times within window — verify suppression
- Production mode: `log_fatal` in production mode — verify behaviour differs from debug
- `LOFT_LOG=crash_tail:N` env var — verify last N lines are flushed on panic

---

## Coverage by file (full table)

| File | Lines | Missed | Cover | Functions | Missed |
|---|---|---|---|---|---|
| `calc.rs` | 56 | 5 | 91.1% | 1 | 0 |
| `codegen_runtime.rs` | 237 | 237 | **0.0%** | 17 | 17 |
| `compile.rs` | 48 | 0 | 100.0% | 2 | 0 |
| `create.rs` | 122 | 4 | 96.7% | 4 | 0 |
| `data.rs` | 990 | 203 | 79.5% | 85 | 16 |
| `database/allocation.rs` | 467 | 266 | **43.0%** | 22 | 7 |
| `database/format.rs` | 450 | 108 | 76.0% | 23 | 9 |
| `database/io.rs` | 230 | 108 | 53.0% | 13 | 3 |
| `database/mod.rs` | 209 | 88 | 57.9% | 17 | 4 |
| `database/search.rs` | 382 | 176 | **49.7%** | 19 | 8 |
| `database/structures.rs` | 614 | 111 | 81.9% | 26 | 2 |
| `database/types.rs` | 682 | 80 | 88.3% | 41 | 4 |
| `diagnostics.rs` | 41 | 6 | 85.4% | 10 | 2 |
| `documentation.rs` | 610 | 610 | **0.0%** | 41 | 41 |
| `fill.rs` | 1323 | 313 | 76.3% | 239 | 54 |
| `formatter.rs` | 550 | 115 | 79.1% | 22 | 0 |
| `gendoc.rs` | 785 | 785 | **0.0%** | 55 | 55 |
| `generation.rs` | 1206 | 152 | 87.4% | 61 | 3 |
| `hash.rs` | 162 | 25 | 84.6% | 7 | 0 |
| `keys.rs` | 167 | 43 | 74.3% | 21 | 5 |
| `lexer.rs` | 777 | 116 | 85.1% | 64 | 4 |
| `log_config.rs` | 221 | 34 | 84.6% | 24 | 2 |
| `logger.rs` | 383 | 250 | **34.7%** | 24 | 11 |
| `main.rs` | 201 | 201 | **0.0%** | 12 | 12 |
| `manifest.rs` | 72 | 2 | 97.2% | 12 | 0 |
| `native.rs` | 555 | 412 | **25.8%** | 57 | 39 |
| `ops.rs` | 424 | 137 | 67.7% | 65 | 12 |
| `parallel.rs` | 114 | 1 | 99.1% | 5 | 0 |
| `parser/builtins.rs` | 224 | 224 | **0.0%** | 6 | 6 |
| `parser/collections.rs` | 1126 | 387 | 65.6% | 22 | 2 |
| `parser/control.rs` | 1379 | 254 | 81.6% | 29 | 1 |
| `parser/definitions.rs` | 894 | 214 | 76.1% | 26 | 0 |
| `parser/expressions.rs` | 2896 | 441 | 84.8% | 85 | 3 |
| `parser/mod.rs` | 1089 | 205 | 81.2% | 45 | 3 |
| `platform.rs` | 12 | 0 | 100.0% | 4 | 0 |
| `png_store.rs` | 28 | 0 | 100.0% | 2 | 0 |
| `radix_tree.rs` | 171 | 171 | **0.0%** | 18 | 18 |
| `scopes.rs` | 403 | 15 | 96.3% | 19 | 0 |
| `stack.rs` | 74 | 7 | 90.5% | 9 | 0 |
| `state/codegen.rs` | 912 | 227 | 75.1% | 33 | 3 |
| `state/debug.rs` | 724 | 212 | 70.7% | 27 | 9 |
| `state/io.rs` | 639 | 166 | 74.0% | 31 | 6 |
| `state/mod.rs` | 309 | 11 | 96.4% | 31 | 1 |
| `state/text.rs` | 330 | 103 | 68.8% | 34 | 9 |
| `store.rs` | 796 | 223 | 72.0% | 77 | 16 |
| `tree.rs` | 529 | 20 | 96.2% | 34 | 1 |
| `typedef.rs` | 227 | 15 | 93.4% | 8 | 0 |
| `variables.rs` | 1177 | 215 | 81.7% | 111 | 11 |
| `vector.rs` | 464 | 53 | 88.6% | 25 | 2 |
| **TOTAL** | **26481** | **7751** | **70.7%** | **1665** | **401** |

---

## Priority order

1. **Fix FAILURES.md Bug 1** (slot-conflict) — unlocks `radix_tree.rs` and 14 script tests automatically; no new tests needed.
2. **`native.rs` math domain boundaries** — high value, easy to write, only requires `02-floats.loft` additions.
3. **Null sentinel / shift edge cases** — `ops.rs` is well-understood; 10–15 targeted assertions in the script tests.
4. **Type conversion edge cases** — `expressions_auto_convert.rs` additions; no new infrastructure.
5. **Database store boundaries** — `limits.rs` or `issues.rs`; important for correctness under real workloads.
6. **Float comparison epsilon** — 4 targeted assertions; very low effort.
7. **`single` (f32) arithmetic** — requires adding `single` type tests; medium effort.
8. **Parser stress / error recovery** — new `parser_stress.rs`; medium effort, high value for robustness.
9. **Logger paths** — low risk; add to existing `log_config.rs`.
10. **`codegen_runtime.rs`** — large effort (requires generation pipeline harness); defer until generation is more stable.

---

## See also
- [TESTING.md](TESTING.md) — How to write tests, use `LOFT_LOG` presets, and add script/doc test files
- [FAILURES.md](FAILURES.md) — Historical failure analysis that informed the coverage measurement baseline
- [PROBLEMS.md](PROBLEMS.md) — Open bugs that block or reduce coverage in specific areas
- [PLANNING.md](PLANNING.md) — Enhancement backlog; coverage gaps often align with planned work
