// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Roadmap

Items in expected implementation order, grouped by milestone.
Full descriptions and Fix paths: [PLANNING.md](PLANNING.md).

**Effort key:** Small · Medium · Med–High · High · Very High

---

## 0.9.0 — Standalone executable

| ID    | Title                                                        | Effort    | Depends on      | Source                      |
|-------|--------------------------------------------------------------|-----------|-----------------|-----------------------------|
| L1    | Error recovery after token failures                          | Medium    |                 | DEVELOPERS.md Step 5        |
| P1    | **Lambda expressions** *(3 phases)*                          | Med–High  |                 | Prototype goal              |
| P1.1  | ↳ Parser — `fn(params) -> type block` primary expression     | Small     |                 | expressions.rs              |
| P1.2  | ↳ Compilation — synthesise anon def, emit def-nr             | Medium    | P1.1            | codegen.rs, compile.rs      |
| P1.3  | ↳ Integration — map/filter/reduce with inline lambdas        | Small     | P1.2            | tests only                  |
| P3    | Vector aggregates (sum, min_of, any, all, count_if)          | Low–Med   | P1              | Stdlib audit 2026-03-15     |
| L2    | Nested patterns in field positions                           | Medium    | T1-14,T1-18     | MATCH.md L2                 |
| A9    | Vector slice becomes independent copy on mutation            | Medium    |                 | TODO in vector.rs           |
| A6    | **Stack slot `assign_slots` pre-pass** *(3 phases)*          | High      |                 | ASSIGNMENT.md Steps 3+4     |
| A6.1  | ↳ Standalone `assign_slots()` — not wired in                 | Medium    |                 | variables.rs                |
| A6.2  | ↳ Shadow mode — assert agrees with `claim()`; log mismatches | Medium    | A6.1            | scopes.rs                   |
| A6.3  | ↳ Replace `claim()` — `assign_slots` becomes sole mechanism  | Small     | A6.2            | codegen.rs                  |
| A8    | Destination-passing for text-returning natives               | Med–High  |                 | String arch review          |
| A3    | Optional Cargo features                                      | Medium    |                 | OPTIONAL_FEATURES.md        |
| N2    | Fix `output_init` intermediate type registration             | Medium    |                 | NATIVE.md N10a              |
| N3    | Fix `output_set` DbRef deep copy                             | Small     |                 | NATIVE.md N10b              |
| N4    | Fix `OpFormatDatabase` for struct-enum variants              | Small     |                 | NATIVE.md N10c              |
| N5    | Fix null DbRef in vector operations                          | Small     |                 | NATIVE.md N10d              |
| N7    | Add `OpFormatFloat`/`OpFormatStackLong` handlers             | Small     |                 | NATIVE.md N10e-3            |
| N8    | Fix empty pre-eval and prefix issues                         | Small     |                 | NATIVE.md N10e-5            |
| N6    | **Implement `OpIterate`/`OpStep` in codegen_runtime** *(3 ph)* | High   |                 | NATIVE.md N10e-2            |
| N6.1  | ↳ Vector iteration — index-based loop with `_iter` counter   | Medium    |                 | codegen_runtime.rs          |
| N6.2  | ↳ `sorted` + `index` iteration via existing helpers          | Medium    | N6.1            | codegen_runtime.rs          |
| N6.3  | ↳ Reverse iteration + range sub-expressions                  | Medium    | N6.2            | generation.rs               |
| N9    | Repair fill.rs auto-generation                               | Medium    |                 | NATIVE.md N20               |
| N1    | `--native` CLI flag                                          | Medium    | N2–N8           | NATIVE.md                   |
| A1    | **Parallel workers: extra args + text/ref returns** *(2 ph)* | High      |                 | THREADING deferred          |
| A1.1  | ↳ Extra context arguments (compile-time wrapper synthesis)   | Medium    |                 | collections.rs, parallel.rs |
| A1.2  | ↳ Text/reference return types (merge worker-local stores)    | Medium    | A1.1            | parallel.rs, store.rs       |

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
| A2    | **Logger: production mode, source injection, hot-reload** *(3 ph)* | Med–High |          | LOGGER.md                   |
| A2.1  | ↳ Structured panic handler → JSON log entry                  | Small     |                 | logger.rs, state/mod.rs     |
| A2.2  | ↳ Source-location injection at compile time                  | Medium    | A2.1            | control.rs, codegen.rs      |
| A2.3  | ↳ Hot-reload log-level config (inotify/kqueue)               | Medium    | A2.1            | logger.rs                   |
| P2    | **REPL / interactive mode** *(4 phases)*                     | High      |                 | Prototype goal              |
| P2.1  | ↳ Input completeness detection (`is_complete`)               | Small     |                 | new repl.rs                 |
| P2.2  | ↳ Single-statement execution in persistent `State`           | Medium    | P2.1            | main.rs, repl.rs            |
| P2.3  | ↳ Automatic value output for non-void results                | Small     | P2.2            | repl.rs                     |
| P2.4  | ↳ Error recovery — session continues after diagnostics       | Medium    | P2.2            | repl.rs, parser.rs          |
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
