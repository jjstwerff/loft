// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Roadmap

Items in expected implementation order, grouped by milestone.
Full descriptions and Fix paths: [PLANNING.md](PLANNING.md).

**Effort key:** Small · Medium · Med–High · High · Very High

---

## 0.8.2 — Stability, efficiency, and native codegen

| ID    | Title                                                        | Effort    | Depends on      | Source                      |
|-------|--------------------------------------------------------------|-----------|-----------------|-----------------------------|
| L4    | Fix empty `[]` literal as mutable vector argument            | Medium    |                 | PROBLEMS.md #44             |
| L5    | Fix `v += extra` via `&vector` ref-param (panic / silent nop) | Medium  |                 | PROBLEMS.md #56             |
| A13   | Float and Long dead-slot reuse in `assign_slots`             | Very Small |                | PLANNING.md A13             |
| A14   | `skip_free` flag — replace `clean_work_refs` type mutation   | Small     |                 | PLANNING.md A14             |
| A15   | Exhaustive `inline_ref_set_in` + fallback assertion          | Very Small |                | PLANNING.md A15             |
| A8    | Destination-passing for text-returning natives               | Med–High  |                 | String arch review          |
| N9    | Repair fill.rs auto-generation (N20b–N20d remaining)         | Medium    |                 | NATIVE.md N20               |
| N1    | `--native` CLI flag                                          | Medium    | N6, N9          | NATIVE.md                   |

---

## 0.8.3 — Language syntax extensions

| ID    | Title                                                        | Effort    | Depends on      | Source                      |
|-------|--------------------------------------------------------------|-----------|-----------------|-----------------------------|
| P1    | **Lambda expressions** *(3 phases)*                          | Med–High  |                 | Prototype goal              |
| P1.1  | ↳ Parser — `fn(params) -> type block` primary expression     | Small     |                 | expressions.rs              |
| P1.2  | ↳ Compilation — synthesise anon def, emit def-nr             | Medium    | P1.1            | codegen.rs, compile.rs      |
| P1.3  | ↳ Integration — map/filter/reduce with inline lambdas        | Small     | P1.2            | tests only                  |
| P3    | Vector aggregates (sum, min_of, any, all, count_if)          | Low–Med   | P1              | Stdlib audit 2026-03-15     |
| L2    | Nested patterns in field positions                           | Medium    |                 | MATCH.md L2                 |
| L3    | **`FileResult` enum** — mutating fs ops return enum + `.ok()` *(3 ph)* | Small |        | User request 2026-03-19     |
| L3.1  | ↳ `FileResult` enum + `io_result` Rust helper               | Small     |                 | 02_images.loft, database/io.rs |
| L3.2  | ↳ Op signatures + all Rust impls (fill.rs, io.rs)           | Small     | L3.1            | fill.rs, state/io.rs, database/io.rs |
| L3.3  | ↳ `ok()` method + public API wrappers + test migration       | Small     | L3.2            | 02_images.loft, tests/      |
| A12   | Lazy work-variable initialization (accurate intervals)        | Small–Med |                 | PLANNING.md A12             |
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
| H2    | JSON primitive extraction stdlib (`serde_json`)              | Medium    | H1              | WEB_SERVICES.md             |
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
| A5    | **Closure capture for lambdas** *(5 phases)*                 | Very High | P1              | Depends on P1               |
| A5.1  | ↳ Capture analysis — identify free variables                 | Small     | P1              | scopes.rs, expressions.rs   |
| A5.2  | ↳ Closure record layout — synthesise anon struct type        | Small     | A5.1            | data.rs, typedef.rs         |
| A5.3  | ↳ Capture at call site — alloc record, copy captured vars    | Medium    | A5.2            | codegen.rs                  |
| A5.4  | ↳ Closure body reads — redirect to closure record arg        | Medium    | A5.3            | codegen.rs, fill.rs         |
| A5.5  | ↳ Lifetime + cleanup — `OpFreeRef` at end of enclosing scope | Small     | A5.4            | scopes.rs                   |
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
