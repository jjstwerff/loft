// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Roadmap

Items in expected implementation order, grouped by milestone.
Full descriptions and fix paths: [PLANNING.md](PLANNING.md).

**Effort key:** Small · Medium · Med–High · High · Very High

---

## 0.8.2 — Native stability, slot correctness, and interpreter performance

**Native test parity achieved (2026-03-23):** all 305 `.loft` tests pass in both
interpreter and native mode.  Issues #77 (fn-ref dispatch) and #80 (LIFO store-free)
are fixed.  `loft --tests --native` with binary caching, stale-rlib auto-rebuild,
and `file.loft::fn` filtering is implemented.  CI now fails on any native regression.

| ID     | Title                                                   | Effort    | Depends on  | Source             |
|--------|---------------------------------------------------------|-----------|-------------|--------------------|
| S5     | Fix `& text` parameter subtract-with-overflow panic     | Small     |             | PROBLEMS.md #89    |
| S9     | Fix `character + character` codegen panic                | Small     |             | PROBLEMS.md #90    |
| O1     | Superinstruction merging (peephole, opcodes 240–245)    | Medium    |             | PERFORMANCE.md P1  |
| O6     | `_nn` variants: drop `long` sentinel from local arith   | Low       |             | PERFORMANCE.md N3  |
| A1     | Parallel workers: extra args + value-struct + text/ref  | Med–High  |             | THREADING.md       |
| A1.1   | ↳ Extra args + value-struct returns                     | Medium    |             | parallel.rs        |
| A1.2   | ↳ Text/reference returns (dedicated result store)       | Medium    | A1.1        | parallel.rs        |
| A12    | Lazy work-variable initialization                       | Medium    | A12.1–A12.3 | PLANNING.md A12    |
| A12.1  | ↳ Fix `first_set_in` Block/Loop descent                 | Small     |             | PROBLEMS.md #68    |
| A12.2  | ↳ Text slot reuse: require exact size match             | Small–Med | A12.1       | PROBLEMS.md #69    |
| A12.3  | ↳ Revert `Type::Text` override in `generate_set`        | Trivial   | A12.2       | PROBLEMS.md #70    |
| A13    | Complete two-zone slot assignment                       | Medium    |             | SLOTS.md           |
| A13.1  | ↳ Fix `Set(v, Block)` ordering in slot placer           | Medium    |             | SLOTS.md Step 8    |
| A13.2  | ↳ Audit `build_scope_parents` for missing IR variants   | Medium    | A13.1       | SLOTS.md Step 10   |
| A13.3  | ↳ Add `Value::Iter` arm to `scan_inner`                 | Small     |             | SLOTS.md           |

---

## 0.8.3 — Language syntax extensions

| ID     | Title                                                   | Effort    | Depends on  | Source                  |
|--------|---------------------------------------------------------|-----------|-------------|-------------------------|
| P3     | Vector aggregates (sum, min_of, any, all, count_if)     | Low–Med   | P1          | Stdlib audit 2026-03-15 |
| L2     | Nested patterns in field positions                      | Medium    |             | MATCH.md L2             |
| L3     | `FileResult` enum for mutating fs operations            | Small     |             | User request 2026-03-19 |
| L3.1   | ↳ `FileResult` enum + `io_result` helper                | Small     |             | database/io.rs          |
| L3.2   | ↳ Op signatures + all Rust impls                        | Small     | L3.1        | fill.rs, state/io.rs    |
| L3.3   | ↳ `ok()` method + public API + test migration           | Small     | L3.2        | 02_images.loft, tests/  |
| A10    | Field iteration (`for f in s#fields`)                   | Medium    |             | Design eval 2026-03-18  |
| A10.0  | ↳ Remove `fields` from KEYWORDS                         | Small     |             | lexer.rs                |
| A10.1  | ↳ `Field` + `FieldValue` types in stdlib                | Small     | A10.0       | 01_code.loft            |
| A10.2  | ↳ `ident#fields` → `Value::FieldsOf` in parser          | Small     | A10.1       | collections.rs, data.rs |
| A10.3  | ↳ Loop unrolling for `Type::FieldsOf`                   | Medium    | A10.2       | collections.rs          |
| A10.4  | ↳ Error messages, docs, tests                           | Small     | A10.3       | LOFT.md, tests/         |

---

## 0.8.4 — HTTP client and JSON

| ID     | Title                                                   | Effort    | Depends on  | Source          |
|--------|---------------------------------------------------------|-----------|-------------|-----------------|
| H1     | `#json` annotation + `to_json` synthesis                | Small     |             | WEB_SERVICES.md |
| H2     | JSON primitive extraction stdlib                        | Small     | H1          | WEB_SERVICES.md |
| H3     | `from_json` codegen — scalar struct fields              | Medium    | H1, H2      | WEB_SERVICES.md |
| H4     | HTTP client stdlib + `HttpResponse` (ureq)              | Medium    | H2          | WEB_SERVICES.md |
| H5     | Nested/array/enum `from_json` + integration tests       | Med–High  | H3, H4      | WEB_SERVICES.md |

---

## 0.9.0 — Standalone executable

| ID     | Title                                                   | Effort    | Depends on  | Source                |
|--------|---------------------------------------------------------|-----------|-------------|-----------------------|
| L1     | Error recovery after token failures                     | Medium    |             | DEVELOPERS.md Step 5  |
| A2     | Logger: hot-reload, run-mode, release + debug flags     | Medium    |             | LOGGER.md             |
| A2.1   | ↳ Wire hot-reload in log functions                      | Small     |             | native.rs             |
| A2.2   | ↳ `is_production()` + `is_debug()` + `RunMode`          | Small     |             | native.rs, 01_code.loft |
| A2.3   | ↳ `--release` flag + `debug_assert()` elision           | Small–Med | A2.2        | control.rs, main.rs   |
| A2.4   | ↳ `--debug` per-type safety logging                     | Medium    | A2.2        | fill.rs, native.rs    |
| P2     | REPL / interactive mode                                 | High      | L1          | Prototype goal        |
| P2.1   | ↳ Input completeness detection                          | Small     |             | new repl.rs           |
| P2.2   | ↳ Single-statement execution in persistent state        | Medium    | P2.1        | main.rs, repl.rs      |
| P2.3   | ↳ Automatic value output for non-void results           | Small     | P2.2        | repl.rs               |
| P2.4   | ↳ Error recovery in session                             | Medium    | P2.2, L1    | repl.rs, parser.rs    |

---

## 1.0.0 — IDE + stability contract

| ID     | Title                                                   | Effort    | Depends on  | Source        |
|--------|---------------------------------------------------------|-----------|-------------|---------------|
| R1     | Workspace split                                         | Small     |             | Extraction plan |
| W1     | WASM foundation                                         | Medium    | R1          | WEB_IDE.md M1 |
| W2     | Editor shell (CodeMirror 6 + Loft grammar)              | Medium    | W1          | WEB_IDE.md M2 |
| W4     | Multi-file projects (IndexedDB)                         | Medium    | W2          | WEB_IDE.md M4 |
| W3     | Symbol navigation (go-to-def, find-usages)              | Medium    | W1, W2      | WEB_IDE.md M3 |
| W5     | Docs & examples browser                                 | Small–Med | W2          | WEB_IDE.md M5 |
| W6     | Export/import ZIP + PWA offline                         | Small–Med | W4          | WEB_IDE.md M6 |

_W2 and W4 can be developed in parallel after W1; W3 and W5 can follow independently._

---

## 1.1+ — Backlog

| ID     | Title                                                   | Effort    | Depends on  | Source               |
|--------|---------------------------------------------------------|-----------|-------------|----------------------|
| A5     | Closure capture for lambdas                             | Very High | P1          | PLANNING.md A5       |
| A5.1   | ↳ Capture analysis (identify free variables)            | Small     | P1          | scopes.rs            |
| A5.2   | ↳ Closure record layout                                 | Small     | A5.1        | data.rs, typedef.rs  |
| A5.3   | ↳ Capture at call site                                  | Medium    | A5.2        | codegen.rs           |
| A5.4   | ↳ Closure body reads via closure record                 | Medium    | A5.3        | codegen.rs, fill.rs  |
| A5.5   | ↳ Lifetime + cleanup (`OpFreeRef`)                      | Small     | A5.4        | scopes.rs            |
| T1     | Tuple types                                             | Very High |             | TUPLES.md            |
| T1.1   | ↳ Type system (`Type::Tuple`, offsets)                  | Medium    |             | data.rs, typedef.rs  |
| T1.2   | ↳ Parser (notation, literals, destructuring)            | Medium    | T1.1        | parser/              |
| T1.3   | ↳ Scope analysis (intervals, lifetimes)                 | Small     | T1.2        | scopes.rs            |
| T1.4   | ↳ Bytecode codegen (slot alloc, read/write)             | Medium    | T1.3        | state/codegen.rs     |
| T1.5   | ↳ Reference-tuple parameters                            | Small     | T1.4        | compiler             |
| T1.6   | ↳ Tuple-aware mutation guard                            | Small     | T1.4        | scopes.rs            |
| T1.7   | ↳ `not null` for tuple integer elements                 | Small     | T1.4        | typedef.rs           |
| TR1    | Stack trace introspection                               | Medium    |             | STACKTRACE.md        |
| TR1.1  | ↳ Shadow call-frame vector                              | Small     |             | state/mod.rs         |
| TR1.2  | ↳ `ArgValue` + `StackFrame` type declarations           | Small     | TR1.1       | 04_stacktrace.loft   |
| TR1.3  | ↳ `stack_trace()` materialisation                       | Medium    | TR1.2       | state/mod.rs, fill.rs |
| TR1.4  | ↳ Call-site line numbers in frames                      | Small     | TR1.3       | state/codegen.rs     |
| CO1    | Coroutines (`yield`, `iterator<T>`, `yield from`)       | Very High | TR1         | COROUTINE.md         |
| CO1.1  | ↳ `iterator<T>` type + `CoroutineStatus`                | Small     | TR1.2       | typedef.rs           |
| CO1.2  | ↳ `OpCoroutineCreate` + `OpCoroutineNext`               | High      | CO1.1       | state/mod.rs, data.rs |
| CO1.3  | ↳ `OpYield` (serialise stack to heap)                   | High      | CO1.2       | state/mod.rs         |
| CO1.4  | ↳ `yield from` delegation                               | Medium    | CO1.3       | state/mod.rs         |
| CO1.5  | ↳ `for item in generator` integration                   | Small     | CO1.3       | parser/collections.rs |
| CO1.6  | ↳ `next()` / `exhausted()` stdlib                       | Small     | CO1.2       | native.rs            |
| O2     | Stack raw pointer cache (eliminate store-indirection)   | High      |             | PERFORMANCE.md P2    |
| O4     | Native: direct-emit local collections                   | High      |             | PERFORMANCE.md N1    |
| O5     | Native: omit `stores` from pure functions               | High      | O4          | PERFORMANCE.md N2    |
| O7     | wasm: pre-allocate string buffers in format path        | Medium    | W1          | PERFORMANCE.md W1    |
| A4     | Spatial index operations                                | High      |             | PROBLEMS.md #22      |
| A4.1   | ↳ Insert + exact lookup                                 | Medium    |             | database.rs, fill.rs |
| A4.2   | ↳ Bounding-box range query                              | Medium    | A4.1        | database.rs          |
| A4.3   | ↳ Removal                                               | Small     | A4.1        | database.rs          |
| A4.4   | ↳ Full iteration                                        | Small     | A4.2, A4.3  | database.rs, io.rs   |
| A7     | Native extension libraries (`cdylib` + `#native`)       | High      |             | EXTERNAL_LIBS.md Ph2 |
| A7.1   | ↳ `#native` annotation + symbol registration            | Medium    |             | parser.rs, compiler  |
| A7.2   | ↳ `cdylib` loader (`libloading`)                        | Medium    | A7.1        | state.rs, Cargo.toml |
| A7.3   | ↳ Package layout + `loft-plugin-api` crate              | Medium    | A7.2        | new workspace member |

---

## Deferred

| ID     | Title                                                   | Effort    | Notes                                    |
|--------|---------------------------------------------------------|-----------|------------------------------------------|
| P4     | Bytecode cache (`.loftc`)                               | Medium    | Superseded by native codegen             |
| A7.4   | External libs: package registry + `loft install`        | Medium    | 2.x; ecosystem must exist first          |

---

## See also
- [PLANNING.md](PLANNING.md) — Full descriptions, fix paths, and effort justifications for every item
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark data and designs for O1–O7
- [DEVELOPMENT.md](DEVELOPMENT.md) — Branch naming, commit sequence, and CI workflow
- [RELEASE.md](RELEASE.md) — Gate criteria each milestone must satisfy before tagging
