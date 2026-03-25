// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Roadmap

Items in expected implementation order, grouped by milestone.
Full descriptions and fix paths: [PLANNING.md](PLANNING.md).

**Effort:** S = Small · M = Medium · MH = Med–High · H = High · VH = Very High

**Design:** ✓ = detailed design in place · ~ = partial/outline · — = needs design

**Maintenance rule:** When an item is completed, remove it from this file entirely.
Do not keep completed items — the ROADMAP tracks only what remains to be done.
Completed work belongs in CHANGELOG.md (user-facing) and git history (implementation).

---

## 0.8.3 — Language syntax extensions

| ID     | Title                                                   | Effort    | Depends on  | Source                  |
|--------|---------------------------------------------------------|-----------|-------------|-------------------------|
| P3     | Vector aggregates (sum, min_of, any, all, count_if)     | Low–Med   | P1          | Stdlib audit 2026-03-15 |
| P5     | First-parameter generic functions (`fn name<T>(...)`)   | Medium    |             | Design 2026-03-25       |
| P5.1   | ↳ `<T>` syntax + `DefType::Generic` template storage    | Small     |             | definitions.rs, data.rs |
| P5.2   | ↳ Call-site instantiation + name mangling               | Medium    | P5.1        | control.rs              |
| P5.3   | ↳ Validation errors for disallowed uses of T            | Small     | P5.1        | parser/ (second pass)   |
| P5.4   | ↳ Tests + LOFT.md § Generic functions                   | Small     | P5.2, P5.3  | tests/docs/, LOFT.md    |
| T2     | `size(t)` — character count for text                    | Small     |             | User request 2026-03-24 |
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
| L6     | Prevent double evaluation of `expr ?? default`          | Small     |             | operators.rs line 330   |
| L7     | `init(expr)` stored field initialiser with `$` reference | Small–Med |             | Design 2026-03-25       |

---

## 0.8.4 — HTTP client

JSON serialisation (`{value:j}`) and deserialisation (`Type.parse(text)`, `vector<T>.parse()`)
are already implemented.  No `#json` annotation needed — see [WEB_SERVICES.md](WEB_SERVICES.md).

| ID     | Title                                          | E  | Design | Depends on | Source              |
|--------|-------------------------------------------------|----|--------|------------|---------------------|
| H4     | HTTP client stdlib + `HttpResponse` (ureq)     | M  | ✓      |            | WEB_SERVICES.md     |
| H4.1   | ↳ `HttpResponse` struct + `ok()` method         | S  | ✓      |            | default/04_web.loft |
| H4.2   | ↳ `http_get`, `http_post`, `http_put`, `http_delete` | M | ✓ | H4.1      | native_http.rs      |
| H4.3   | ↳ Header support (`http_get_h`, `http_post_h`) | S  | ✓      | H4.2       | native_http.rs      |
| H4.4   | ↳ Documentation + integration tests            | S  | ✓      | H4.2       | tests/docs/         |

---

## 0.9.0 — Standalone executable

| ID     | Title                                          | E  | Design | Depends on | Source              |
|--------|-------------------------------------------------|----|--------|------------|---------------------|
| L1     | Error recovery after token failures            | M  | ~      |            | DEVELOPERS.md       |
| A2     | Logger: hot-reload, run-mode, release + debug  | M  | ✓      |            | LOGGER.md           |
| A2.1   | ↳ Wire hot-reload in log functions              | S  | ✓      |            | native.rs           |
| A2.2   | ↳ `is_production()` + `is_debug()` + `RunMode` | S  | ✓      |            | 01_code.loft        |
| A2.3   | ↳ `--release` flag + `debug_assert()` elision  | MH | ✓      | A2.2       | control.rs, main.rs |
| A2.4   | ↳ `--debug` per-type safety logging            | M  | ✓      | A2.2       | fill.rs, native.rs  |
| P2     | REPL / interactive mode                        | H  | ✓      | L1         | PLANNING.md P2      |
| P2.1   | ↳ Input completeness detection                  | S  | ✓      |            | new repl.rs         |
| P2.2   | ↳ Single-statement execution                    | M  | ✓      | P2.1       | main.rs, repl.rs    |
| P2.3   | ↳ Automatic value output                        | S  | ✓      | P2.2       | repl.rs             |
| P2.4   | ↳ Error recovery in session                    | M  | ✓      | P2.2, L1   | repl.rs, parser.rs  |
| A7     | Native extension libraries (`cdylib`)          | H  | ✓      |            | EXTERNAL_LIBS.md    |
| A7.1   | ↳ `#native` annotation + symbol registration   | M  | ✓      |            | parser.rs           |
| A7.2   | ↳ `cdylib` loader (`libloading`)               | M  | ✓      | A7.1       | state.rs            |
| A7.3   | ↳ Package layout + `loft-plugin-api` crate     | M  | ✓      | A7.2       | new workspace       |

---

## 1.0.0 — IDE + stability contract

| ID     | Title                                          | E  | Design | Depends on | Source              |
|--------|-------------------------------------------------|----|--------|------------|---------------------|
| R1     | Workspace split                                | S  | ~      |            | Extraction plan     |
| W1     | WASM foundation                                | M  | ✓      | R1         | WEB_IDE.md M1       |
| W2     | Editor shell (CodeMirror 6 + Loft grammar)     | M  | ✓      | W1         | WEB_IDE.md M2       |
| W3     | Symbol navigation (go-to-def, find-usages)     | M  | ✓      | W1, W2     | WEB_IDE.md M3       |
| W4     | Multi-file projects (IndexedDB)                | M  | ✓      | W2         | WEB_IDE.md M4       |
| W5     | Docs & examples browser                        | MH | ✓      | W2         | WEB_IDE.md M5       |
| W6     | Export/import ZIP + PWA offline                 | MH | ✓      | W4         | WEB_IDE.md M6       |

_W2 and W4 can be developed in parallel after W1; W3 and W5 can follow independently._

---

## 1.1+ — Backlog

| ID     | Title                                          | E  | Design | Depends on | Source              |
|--------|-------------------------------------------------|----|--------|------------|---------------------|
| L7     | Non-zero exit code on parse/runtime errors     | S  | —      |            | CAVEATS.md C6       |
| S17    | Slot: text below TOS in nested scopes          | M  | —      |            | CAVEATS.md C4       |
| S18    | Slot: sequential file blocks conflict          | M  | —      |            | CAVEATS.md C5       |
| A12    | Lazy work-variable initialization              | M  | ~      |            | PLANNING.md A12     |
| S16    | Native codegen: enum method dispatch           | MH | Done   |            | resolved — C2 removed |
| O1     | Superinstruction peephole rewriting            | M  | ~      |            | compile.rs          |
| O2     | Stack raw pointer cache                        | H  | ~      |            | PERFORMANCE.md P2   |
| A4     | Spatial index operations                       | H  | ~      |            | PROBLEMS.md #22     |
| A4.1   | ↳ Insert + exact lookup                         | M  | ~      |            | database.rs         |
| A4.2   | ↳ Bounding-box range query                      | M  | ~      | A4.1       | database.rs         |
| A4.3   | ↳ Removal                                       | S  | ~      | A4.1       | database.rs         |
| A4.4   | ↳ Full iteration                                | S  | ~      | A4.2, A4.3 | database.rs         |
| O4     | Native: direct-emit local collections          | H  | ~      |            | PERFORMANCE.md N1   |
| O5     | Native: omit `stores` from pure functions      | H  | ~      | O4         | PERFORMANCE.md N2   |
| O7     | WASM: pre-allocate format string buffers       | M  | —      | W1         | PERFORMANCE.md W1   |

---

## Deferred indefinitely

| ID     | Title                                          | E  | Notes                                    |
|--------|-------------------------------------------------|----|------------------------------------------|
| P4     | Bytecode cache (`.loftc`)                      | M  | Superseded by native codegen             |
| A7.4   | External libs: package registry + `loft install`| M | 2.x; ecosystem must exist first          |

---

## See also

- [PLANNING.md](PLANNING.md) — Full descriptions, fix paths, and effort justifications for every item
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark data and designs for O1–O7
- [DEVELOPMENT.md](DEVELOPMENT.md) — Branch naming, commit sequence, and CI workflow
- [RELEASE.md](RELEASE.md) — Gate criteria each milestone must satisfy before tagging
