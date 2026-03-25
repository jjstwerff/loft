// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Roadmap

Items in expected implementation order, grouped by milestone.
Full descriptions and fix paths: [PLANNING.md](PLANNING.md).

**Effort key:** Small · Medium · Med–High · High · Very High

**Maintenance rule:** When an item is completed, remove it from this file entirely.
Do not keep completed items — the ROADMAP tracks only what remains to be done.
Completed work belongs in CHANGELOG.md (user-facing) and git history (implementation).
Issue IDs (S9, A1.2, etc.) must not appear in source code comments either; use
plain English describing the purpose of the code.

---

## 0.8.3 — Language syntax extensions

| ID     | Title                                                   | Effort    | Depends on  | Source                  |
|--------|---------------------------------------------------------|-----------|-------------|-------------------------|
| A10    | Field iteration (`for f in s#fields`)                   | Medium    |             | Design eval 2026-03-18  |
| A10.0  | ↳ Remove `fields` from KEYWORDS                         | Small     | ✓ done      | lexer.rs                |
| A10.1  | ↳ `StructField` + `FieldValue` types                    | Small     | ✓ done      | user code (not stdlib)  |
| A10.2  | ↳ `ident#fields` detection in `iter_op`                 | Small     | ✓ done      | collections.rs          |
| A10.3  | ↳ Loop unrolling in `parse_field_iteration`             | Medium    | S14, S15    | collections.rs          |
| A10.4  | ↳ Error messages, docs, tests                           | Small     | A10.3       | LOFT.md, tests/         |
| A5     | Closure capture for lambdas                             | Very High |             | PLANNING.md A5          |
| A5.1   | ↳ Capture analysis (identify free variables)            | Small     |             | scopes.rs               |
| A5.2   | ↳ Closure record layout                                 | Small     | A5.1        | data.rs, typedef.rs     |
| A5.3   | ↳ Capture at call site                                  | Medium    | A5.2        | codegen.rs              |
| A5.4   | ↳ Closure body reads via closure record                 | Medium    | A5.3        | codegen.rs, fill.rs     |
| A5.5   | ↳ Lifetime + cleanup (`OpFreeRef`)                      | Small     | A5.4        | scopes.rs               |
| T1     | Tuple types                                             | Very High |             | TUPLES.md               |
| T1.1   | ↳ Type system (`Type::Tuple`, offsets)                  | Medium    |             | data.rs, typedef.rs     |
| T1.2   | ↳ Parser (notation, literals, destructuring)            | Medium    | T1.1        | parser/                 |
| T1.3   | ↳ Scope analysis (intervals, lifetimes)                 | Small     | T1.2        | scopes.rs               |
| T1.4   | ↳ Bytecode codegen (slot alloc, read/write)             | Medium    | T1.3        | state/codegen.rs        |
| T1.5   | ↳ Reference-tuple parameters                            | Small     | T1.4        | compiler                |
| T1.6   | ↳ Tuple-aware mutation guard                            | Small     | T1.4        | scopes.rs               |
| T1.7   | ↳ `not null` for tuple integer elements                 | Small     | T1.4        | typedef.rs              |
| TR1    | Stack trace introspection                               | Medium    |             | STACKTRACE.md           |
| TR1.1  | ↳ Shadow call-frame vector                              | Small     |             | state/mod.rs            |
| TR1.2  | ↳ `ArgValue` + `StackFrame` type declarations           | Small     | TR1.1       | 04_stacktrace.loft      |
| TR1.3  | ↳ `stack_trace()` materialisation                       | Medium    | TR1.2       | state/mod.rs, fill.rs   |
| TR1.4  | ↳ Call-site line numbers in frames                      | Small     | TR1.3       | state/codegen.rs        |
| CO1    | Coroutines (`yield`, `iterator<T>`, `yield from`)       | Very High | TR1         | COROUTINE.md            |
| CO1.1  | ↳ `iterator<T>` type + `CoroutineStatus`                | Small     | TR1.2       | typedef.rs              |
| CO1.2  | ↳ `OpCoroutineCreate` + `OpCoroutineNext`               | High      | CO1.1       | state/mod.rs, data.rs   |
| CO1.3  | ↳ `OpYield` (serialise stack to heap)                   | High      | CO1.2       | state/mod.rs            |
| CO1.4  | ↳ `yield from` delegation                               | Medium    | CO1.3       | state/mod.rs            |
| CO1.5  | ↳ `for item in generator` integration                   | Small     | CO1.3       | parser/collections.rs   |
| CO1.6  | ↳ `next()` / `exhausted()` stdlib                       | Small     | CO1.2       | native.rs               |
| S14    | Struct-enum stdlib field positions (#80)                 | Medium    |             | CAVEATS.md C9           |
| S15    | Struct-enum same-name variant field offsets (#81)        | Medium    |             | CAVEATS.md C10          |
| L8     | Warn on format specifier / type mismatch                | Small     |             | CAVEATS.md C14          |

---

## 0.8.4 — HTTP client

JSON serialisation (`{value:j}`) and deserialisation (`Type.parse(text)`, `vector<T>.parse()`)
are already implemented.  No `#json` annotation needed — see [WEB_SERVICES.md](WEB_SERVICES.md).

| ID     | Title                                                   | Effort    | Depends on  | Source                  |
|--------|---------------------------------------------------------|-----------|-------------|-------------------------|
| H4     | HTTP client stdlib + `HttpResponse` (ureq)              | Medium    |             | WEB_SERVICES.md         |
| H4.1   | ↳ `HttpResponse` struct + `ok()` method                 | Small     |             | default/04_web.loft     |
| H4.2   | ↳ `http_get`, `http_post`, `http_put`, `http_delete`    | Medium    | H4.1        | native_http.rs          |
| H4.3   | ↳ Header support (`http_get_h`, `http_post_h`)          | Small     | H4.2        | native_http.rs          |
| H4.4   | ↳ Documentation + integration tests                     | Small     | H4.2        | tests/docs/             |

---

## 0.9.0 — Standalone executable

| ID     | Title                                                   | Effort    | Depends on  | Source                  |
|--------|---------------------------------------------------------|-----------|-------------|-------------------------|
| L1     | Error recovery after token failures                     | Medium    |             | DEVELOPERS.md Step 5    |
| A2     | Logger: hot-reload, run-mode, release + debug flags     | Medium    |             | LOGGER.md               |
| A2.1   | ↳ Wire hot-reload in log functions                      | Small     |             | native.rs               |
| A2.2   | ↳ `is_production()` + `is_debug()` + `RunMode`          | Small     |             | native.rs, 01_code.loft |
| A2.3   | ↳ `--release` flag + `debug_assert()` elision           | Small–Med | A2.2        | control.rs, main.rs     |
| A2.4   | ↳ `--debug` per-type safety logging                     | Medium    | A2.2        | fill.rs, native.rs      |
| P2     | REPL / interactive mode                                 | High      | L1          | Prototype goal          |
| P2.1   | ↳ Input completeness detection                          | Small     |             | new repl.rs             |
| P2.2   | ↳ Single-statement execution in persistent state        | Medium    | P2.1        | main.rs, repl.rs        |
| P2.3   | ↳ Automatic value output for non-void results           | Small     | P2.2        | repl.rs                 |
| P2.4   | ↳ Error recovery in session                             | Medium    | P2.2, L1    | repl.rs, parser.rs      |
| A7     | Native extension libraries (`cdylib` + `#native`)       | High      |             | EXTERNAL_LIBS.md Ph2    |
| A7.1   | ↳ `#native` annotation + symbol registration            | Medium    |             | parser.rs, compiler     |
| A7.2   | ↳ `cdylib` loader (`libloading`)                        | Medium    | A7.1        | state.rs, Cargo.toml    |
| A7.3   | ↳ Package layout + `loft-plugin-api` crate              | Medium    | A7.2        | new workspace member    |

---

## 1.0.0 — IDE + stability contract

| ID     | Title                                                   | Effort    | Depends on  | Source                  |
|--------|---------------------------------------------------------|-----------|-------------|-------------------------|
| R1     | Workspace split                                         | Small     |             | Extraction plan         |
| W1     | WASM foundation                                         | Medium    | R1          | WEB_IDE.md M1           |
| W2     | Editor shell (CodeMirror 6 + Loft grammar)              | Medium    | W1          | WEB_IDE.md M2           |
| W3     | Symbol navigation (go-to-def, find-usages)              | Medium    | W1, W2      | WEB_IDE.md M3           |
| W4     | Multi-file projects (IndexedDB)                         | Medium    | W2          | WEB_IDE.md M4           |
| W5     | Docs & examples browser                                 | Small–Med | W2          | WEB_IDE.md M5           |
| W6     | Export/import ZIP + PWA offline                          | Small–Med | W4          | WEB_IDE.md M6           |

_W2 and W4 can be developed in parallel after W1; W3 and W5 can follow independently._

---

## 1.1+ — Backlog

| ID     | Title                                                   | Effort    | Depends on  | Source                  |
|--------|---------------------------------------------------------|-----------|-------------|-------------------------|
| O1     | Superinstruction peephole rewriting                     | Medium    |             | compile.rs              |
| O2     | Stack raw pointer cache (eliminate store-indirection)   | High      |             | PERFORMANCE.md P2       |
| O4     | Native: direct-emit local collections                   | High      |             | PERFORMANCE.md N1       |
| O5     | Native: omit `stores` from pure functions               | High      | O4          | PERFORMANCE.md N2       |
| O7     | WASM: pre-allocate string buffers in format path        | Medium    | W1          | PERFORMANCE.md W1       |
| A4     | Spatial index operations                                | High      |             | PROBLEMS.md #22         |
| A4.1   | ↳ Insert + exact lookup                                 | Medium    |             | database.rs, fill.rs    |
| A4.2   | ↳ Bounding-box range query                              | Medium    | A4.1        | database.rs             |
| A4.3   | ↳ Removal                                               | Small     | A4.1        | database.rs             |
| A4.4   | ↳ Full iteration                                        | Small     | A4.2, A4.3  | database.rs, io.rs      |
| A12    | Lazy work-variable initialization                       | Medium    |             | PLANNING.md A12         |
| S16    | Native codegen: enum method dispatch                    | Small–Med |             | CAVEATS.md C2           |
| S17    | Slot: text below TOS in nested scopes                   | Medium    |             | CAVEATS.md C4           |
| S18    | Slot: sequential file blocks conflict                   | Medium    |             | CAVEATS.md C5           |
| L7     | Non-zero exit code on parse/runtime errors              | Small     |             | CAVEATS.md C6           |

---

## Deferred indefinitely

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
