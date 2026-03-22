// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Roadmap

Items in expected implementation order, grouped by milestone.
Full descriptions and Fix paths: [PLANNING.md](PLANNING.md).

**Effort key:** Small · Medium · Med–High · High · Very High

---

---

## 0.8.3 — Language syntax extensions

| ID    | Title                                                        | Effort    | Depends on      | Source                      |
|-------|--------------------------------------------------------------|-----------|-----------------|-----------------------------|
| S5    | Fix optional `& text` parameter subtract-with-overflow panic (Issue 89) | Small |      | PROBLEMS.md #89             |
| S7    | Add diagnostic error for `string` type name — should be `text` (Issue 82) | Trivial |    | PROBLEMS.md #82             |
| S8    | Compile-time error when hash-value struct has field named `key` (Issue 83) | Small |     | PROBLEMS.md #83             |
| P3    | Vector aggregates (sum, min_of, any, all, count_if)          | Low–Med   | P1              | Stdlib audit 2026-03-15     |
| L2    | Nested patterns in field positions                           | Medium    |                 | MATCH.md L2                 |
| L3    | **`FileResult` enum** — mutating fs ops return enum + `.ok()` *(3 ph)* | Small |        | User request 2026-03-19     |
| L3.1  | ↳ `FileResult` enum + `io_result` Rust helper               | Small     |                 | 02_images.loft, database/io.rs |
| L3.2  | ↳ Op signatures + all Rust impls (fill.rs, io.rs)           | Small     | L3.1            | fill.rs, state/io.rs, database/io.rs |
| L3.3  | ↳ `ok()` method + public API wrappers + test migration       | Small     | L3.2            | 02_images.loft, tests/      |
| A10   | **Field iteration — `for f in s#fields`** *(5 ph)*           | Medium    |                 | Design eval 2026-03-18      |
| A10.0 | ↳ Remove `fields` from `KEYWORDS` (revert L3 code change)   | Small     |                 | lexer.rs                    |
| A10.1 | ↳ `Field` + `FieldValue` types in `default/01_code.loft`    | Small     | A10.0           | 01_code.loft                |
| A10.2 | ↳ `ident#fields` in `parse_for` → `Value::FieldsOf`         | Small     | A10.1           | collections.rs, data.rs     |
| A10.3 | ↳ Loop unrolling in `parse_for` for `Type::FieldsOf`         | Medium    | A10.2           | collections.rs, typedef.rs  |
| A10.4 | ↳ Error messages, docs, test coverage                        | Small     | A10.3           | LOFT.md, STDLIB.md, tests/  |

---

## 0.8.4 — HTTP client and JSON

| ID    | Title                                                        | Effort    | Depends on      | Source                      |
|-------|--------------------------------------------------------------|-----------|-----------------|-----------------------------|
| H1    | `#json` annotation — parser + `to_json` synthesis           | Small     |                 | WEB_SERVICES.md             |
| H2    | JSON primitive extraction stdlib (`src/database/json.rs`)    | Small     | H1              | WEB_SERVICES.md             |
| H3    | `from_json` codegen — scalar struct fields                   | Medium    | H1, H2          | WEB_SERVICES.md             |
| H4    | HTTP client stdlib + `HttpResponse` (`ureq`)                 | Medium    | H2              | WEB_SERVICES.md             |
| H5    | Nested/array/enum `from_json` + integration tests            | Med–High  | H3, H4          | WEB_SERVICES.md             |

---

## 0.9.0 — Standalone executable

| ID    | Title                                                        | Effort    | Depends on      | Source                      |
|-------|--------------------------------------------------------------|-----------|-----------------|-----------------------------|
| L1    | Error recovery after token failures                          | Medium    |                 | DEVELOPERS.md Step 5        |
| A1    | **Parallel workers: extra args + text/ref returns** *(2 ph)* | High      |                 | THREADING deferred          |
| A1.1  | ↳ Extra context arguments (compile-time wrapper synthesis)   | Medium    |                 | collections.rs, parallel.rs |
| A1.2  | ↳ Text/reference return types (merge worker-local stores)    | Medium    | A1.1            | parallel.rs, store.rs       |
| A2    | **Logger: hot-reload, run-mode helpers, release + debug flags** *(4 ph)* | Medium |       | LOGGER.md § Remaining Work  |
| A2.1  | ↳ Wire hot-reload (`check_reload` in n_log_* bodies)         | Small     |                 | native.rs                   |
| A2.2  | ↳ `is_production()` + `is_debug()` + `RunMode` enum          | Small     |                 | native.rs, 01_code.loft     |
| A2.3  | ↳ `--release` flag + `debug_assert()` elision                | Small–Med | A2.2            | control.rs, main.rs         |
| A2.4  | ↳ `--debug` per-type safety logging (overflow, OOB, null)    | Medium    | A2.2            | fill.rs, native.rs          |
| P2    | **REPL / interactive mode** *(4 phases)*                     | High      | L1              | Prototype goal              |
| P2.1  | ↳ Input completeness detection (`is_complete`)               | Small     |                 | new repl.rs                 |
| P2.2  | ↳ Single-statement execution in persistent `State`           | Medium    | P2.1            | main.rs, repl.rs            |
| P2.3  | ↳ Automatic value output for non-void results                | Small     | P2.2            | repl.rs                     |
| P2.4  | ↳ Error recovery — session continues after diagnostics       | Medium    | P2.2, L1        | repl.rs, parser.rs          |

---

## 1.0.0 — IDE + stability contract

| ID    | Title                                                        | Effort    | Depends on      | Source                      |
|-------|--------------------------------------------------------------|-----------|-----------------|-----------------------------|
| R1    | Workspace split (prerequisite for W1)                        | Small     |                 | Extraction plan             |
| W1    | WASM foundation (Rust feature + wasm-bridge.js)              | Medium    | R1              | WEB_IDE.md M1               |
| W2    | Editor shell (CodeMirror 6 + Loft grammar)                   | Medium    | W1              | WEB_IDE.md M2               |
| W4    | Multi-file projects (IndexedDB)                              | Medium    | W2              | WEB_IDE.md M4               |
| W3    | Symbol navigation (go-to-def, find-usages)                   | Medium    | W1, W2          | WEB_IDE.md M3               |
| W5    | Docs & examples browser                                      | Small–Med | W2              | WEB_IDE.md M5               |
| W6    | Export/import ZIP + PWA offline                              | Small–Med | W4              | WEB_IDE.md M6               |

_W2 and W4 can be developed in parallel after W1; W3 and W5 can follow independently._

---

## 1.1+ — Backlog

| ID    | Title                                                        | Effort    | Depends on      | Source                      |
|-------|--------------------------------------------------------------|-----------|-----------------|-----------------------------|
| S6    | Fix `for` loop in recursive function — per-function variable scoping (Issue 84) | High | | PROBLEMS.md #84 |
| A12   | **Lazy work-variable init** (blocked: Issues 68–70)          | Medium    | A12.1–A12.3     | PLANNING.md A12             |
| A12.1 | ↳ Fix `first_set_in` Block/Loop descent (Issue 68)           | Small     |                 | PROBLEMS.md #68             |
| A12.2 | ↳ Text slot reuse: require exact size match (Issue 69)       | Small–Med | A12.1           | PROBLEMS.md #69             |
| A12.3 | ↳ Revert `Type::Text` override in `generate_set` (Issue 70)  | Trivial   | A12.2           | PROBLEMS.md #70             |
| A5    | **Closure capture for lambdas** *(5 phases)*                 | Very High | P1              | Depends on P1               |
| A5.1  | ↳ Capture analysis — identify free variables                 | Small     | P1              | scopes.rs, expressions.rs   |
| A5.2  | ↳ Closure record layout — synthesise anon struct type        | Small     | A5.1            | data.rs, typedef.rs         |
| A5.3  | ↳ Capture at call site — alloc record, copy captured vars    | Medium    | A5.2            | codegen.rs                  |
| A5.4  | ↳ Closure body reads — redirect to closure record arg        | Medium    | A5.3            | codegen.rs, fill.rs         |
| A5.5  | ↳ Lifetime + cleanup — `OpFreeRef` at end of enclosing scope | Small     | A5.4            | scopes.rs                   |
| T1    | **Tuple types** — multi-value returns, stack-allocated compound values *(7 ph)* | Very High | | TUPLES.md |
| T1.1  | ↳ Type system — `Type::Tuple`, element offsets, `element_size` helpers | Medium | | data.rs, typedef.rs |
| T1.2  | ↳ Parser — type notation, literal syntax, destructuring assignment | Medium | T1.1 | parser/           |
| T1.3  | ↳ Scope analysis — tuple variable intervals, text/ref element lifetimes | Small | T1.2 | scopes.rs |
| T1.4  | ↳ Bytecode codegen — slot allocation, element read/write     | Medium    | T1.3            | state/codegen.rs            |
| T1.5  | ↳ SC-4: Reference-tuple parameters with owned elements       | Small     | T1.4            | compiler                    |
| T1.6  | ↳ SC-8: Tuple-aware mutation guard                           | Small     | T1.4            | scopes.rs                   |
| T1.7  | ↳ SC-7: `not null` annotation for tuple integer elements     | Small     | T1.4            | typedef.rs                  |
| TR1   | **Stack trace introspection** (`stack_trace()`, `StackFrame`, `ArgValue`) *(4 ph)* | Medium | | STACKTRACE.md |
| TR1.1 | ↳ Shadow call-frame vector (push/pop per fn call)            | Small     |                 | state/mod.rs                |
| TR1.2 | ↳ Type declarations — `ArgValue` enum, `StackFrame` struct (`default/04_stacktrace.loft`) | Small | TR1.1 | 04_stacktrace.loft |
| TR1.3 | ↳ Materialisation — `stack_trace()` native builds `vector<StackFrame>` | Medium | TR1.2 | state/mod.rs, fill.rs |
| TR1.4 | ↳ Call-site line numbers — track source position in call frame | Small   | TR1.3           | state/codegen.rs            |
| CO1   | **Coroutines** (`yield`, `iterator<T>`, `yield from`) *(6 ph)* | Very High | TR1           | COROUTINE.md                |
| CO1.1 | ↳ `iterator<T>` type + `CoroutineStatus` enum (`default/05_coroutine.loft`) | Small | TR1.2 | typedef.rs |
| CO1.2 | ↳ `OpCoroutineCreate` + `OpCoroutineNext` — frame construction and advance | High | CO1.1 | state/mod.rs, data.rs |
| CO1.3 | ↳ `OpYield` — serialise live stack to heap frame, return to caller | High | CO1.2 | state/mod.rs |
| CO1.4 | ↳ `yield from` — sub-generator delegation                   | Medium    | CO1.3           | state/mod.rs                |
| CO1.5 | ↳ `for item in generator` integration — iterator protocol    | Small     | CO1.3           | parser/collections.rs       |
| CO1.6 | ↳ `next()` / `exhausted()` stdlib functions                 | Small     | CO1.2           | native.rs, 05_coroutine.loft |
| N2    | Native codegen: `CallRef` function-pointer dispatch (Issue 77) | Medium  |                 | PROBLEMS.md #77             |
| N3    | Native codegen: resolve `external` crate reference (Issue 79) | Low     |                 | PROBLEMS.md #79             |
| N4    | Native codegen: fix LIFO store-free order in generated frees (Issue 80) | Medium |        | PROBLEMS.md #80             |
| N5    | Native codegen: `file_from_bytes` for `DbRef` vector types (Issue 86) | Medium |          | PROBLEMS.md #86             |
| N6    | Native codegen: text method in format interpolation — emit `&str` (Issue 87) | Small |    | PROBLEMS.md #87             |
| N7    | Native codegen: `directory()` scratch buffer argument (Issue 88) | Small  |                | PROBLEMS.md #88             |
| N9    | Native codegen: exhaustive IR pattern matching — remove `panic!` catch-alls (Issue 61) | Medium | N2–N7 | PROBLEMS.md #61 |
| A13   | **Complete two-zone slot assignment** — Steps 8 + 10 *(3 ph)* | Medium  |                 | SLOTS.md Steps 8, 10        |
| A13.1 | ↳ Step 8: `Set(v, Block)` ordering fix in `place_large_and_recurse` | Medium |            | SLOTS.md Step 8             |
| A13.2 | ↳ Step 10: Audit `build_scope_parents` for missing IR variants | Medium  | A13.1           | SLOTS.md Step 10            |
| A13.3 | ↳ `scan_inner`: add `Value::Iter` arm (latent false-positive scope gap) | Small |          | SLOTS.md § Open Issues      |
| A4    | **Spatial index operations** *(4 phases)*                    | High      |                 | PROBLEMS #22                |
| A4.1  | ↳ Insert + exact lookup; remove pre-gate for these ops       | Medium    |                 | database.rs, fill.rs        |
| A4.2  | ↳ Bounding-box range query `spacial[x1..x2, y1..y2]`        | Medium    | A4.1            | database.rs, collections.rs |
| A4.3  | ↳ Removal (`spacial[key] = null`, remove in iterator)        | Small     | A4.1            | database.rs                 |
| A4.4  | ↳ Full iteration; remove remaining pre-gate                  | Small     | A4.2,3          | database.rs, io.rs          |
| A7    | **Native extension libraries (`cdylib` + `#native`)** *(3 ph)* | High   |                 | EXTERNAL_LIBS.md Ph2        |
| A7.1  | ↳ `#native` annotation + symbol registration                 | Medium    |                 | parser.rs, compiler, state  |
| A7.2  | ↳ `cdylib` loader (`libloading`, optional feature)           | Medium    | A7.1            | state.rs, Cargo.toml        |
| A7.3  | ↳ Package layout + `loft-plugin-api` crate                   | Medium    | A7.2            | new workspace member        |

---

## Deferred

| ID    | Title                                                        | Effort    | Notes                                               |
|-------|--------------------------------------------------------------|-----------|-----------------------------------------------------|
| P4    | Bytecode cache (`.loftc`)                                    | Medium    | Superseded by Tier N native codegen                 |
| A7.4  | External Libs Phase 3 — package registry, `loft install`, SHA-256 | Medium | 2.x; ecosystem must exist first (EXTERNAL_LIBS.md Ph3) |

---

## See also
- [PLANNING.md](PLANNING.md) — Full descriptions, fix paths, and effort justifications for every item
- [DEVELOPMENT.md](DEVELOPMENT.md) — Branch naming, commit sequence, and CI workflow for implementing an item
- [RELEASE.md](RELEASE.md) — Gate criteria each milestone must satisfy before tagging
