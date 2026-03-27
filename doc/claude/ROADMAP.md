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

## 0.8.3 — WASM runtime + native extensions *(Rust steps done; JS steps deferred)*

W1.1–W1.9 (Rust), A7.1–A7.3, and W1.10 (VirtFS JS) completed in 0.8.3.
W1.11–W1.13 require Node.js + wasm-pack to run — deferred to a later milestone.

| ID        | Title                                                | E  | Design | Depends on   | Source                     |
|-----------|------------------------------------------------------|----|--------|--------------|----------------------------|
| W1.11     | ↳ Host factory `createHost` + bridge tests           | M  | ✓      | W1.9, W1.10  | tests/wasm/host.mjs        |
| W1.12     | ↳ LayeredFS + base-tree builder (`build-base-fs.js`) | M  | ✓      | W1.10        | tests/wasm/layered-fs.mjs  |
| W1.13     | ↳ Full loft test suite via WASM (`suite.mjs`)        | M  | ✓      | W1.11        | tests/wasm/suite.mjs       |

---

## 0.8.4 — HTTP client

JSON serialisation (`{value:j}`) and deserialisation (`Type.parse(text)`, `vector<T>.parse()`)
are already implemented.  No `#json` annotation needed — see [WEB_SERVICES.md](WEB_SERVICES.md).

| ID        | Title                                                | E  | Design | Depends on   | Source                     |
|-----------|------------------------------------------------------|----|--------|--------------|----------------------------|
| H4        | HTTP client stdlib + `HttpResponse` (ureq)           | M  | ✓      |              | WEB_SERVICES.md            |
| H4.1      | ↳ `HttpResponse` struct + `ok()` method              | S  | ✓      |              | default/04_web.loft        |
| H4.2      | ↳ `http_get`, `http_post`, `http_put`, `http_delete` | M  | ✓      | H4.1         | native_http.rs             |
| H4.3      | ↳ Header support (`http_get_h`, `http_post_h`)       | S  | ✓      | H4.2         | native_http.rs             |
| H4.4      | ↳ Documentation + integration tests                  | S  | ✓      | H4.2         | tests/docs/                |

---

## 0.9.0 — Standalone executable

| ID        | Title                                                | E  | Design | Depends on   | Source                     |
|-----------|------------------------------------------------------|----|--------|--------------|----------------------------|
| L1        | Error recovery after token failures                  | M  | ~      |              | DEVELOPERS.md              |
| A2        | Logger: hot-reload, run-mode, release + debug        | M  | ✓      |              | LOGGER.md                  |
| A2.1      | ↳ Wire hot-reload in log functions                   | S  | ✓      |              | native.rs                  |
| A2.2      | ↳ `is_production()` + `is_debug()` + `RunMode`       | S  | ✓      |              | 01_code.loft               |
| A2.3      | ↳ `--release` flag + `debug_assert()` elision        | MH | ✓      | A2.2         | control.rs, main.rs        |
| A2.4      | ↳ `--debug` per-type safety logging                  | M  | ✓      | A2.2         | fill.rs, native.rs         |
| P2        | REPL / interactive mode                              | H  | ✓      | L1           | PLANNING.md P2             |
| P2.1      | ↳ Input completeness detection                       | S  | ✓      |              | new repl.rs                |
| P2.2      | ↳ Single-statement execution                         | M  | ✓      | P2.1         | main.rs, repl.rs           |
| P2.3      | ↳ Automatic value output                             | S  | ✓      | P2.2         | repl.rs                    |
| P2.4      | ↳ Error recovery in session                          | M  | ✓      | P2.2, L1     | repl.rs, parser.rs         |
| S19       | Fix #85: struct-enum locals not freed (debug)        | S  | ✓      |              | CAVEATS.md C16             |
| S20       | Fix #91: init(expr) circular dep detection           | S  | ✓      |              | CAVEATS.md C18             |
| S21       | Fix #92: stack_trace() empty in par workers          | S  | ✓      |              | CAVEATS.md C17             |

---

## 1.0.0 — IDE + stability contract

| ID        | Title                                                | E  | Design | Depends on   | Source                     |
|-----------|------------------------------------------------------|----|--------|--------------|----------------------------|
| W2        | Editor shell (CodeMirror 6 + Loft grammar)           | M  | ✓      | W1           | WEB_IDE.md M2              |
| W3        | Symbol navigation (go-to-def, find-usages)           | M  | ✓      | W1, W2       | WEB_IDE.md M3              |
| W4        | Multi-file projects (IndexedDB)                      | M  | ✓      | W2           | WEB_IDE.md M4              |
| W5        | Docs & examples browser                              | MH | ✓      | W2           | WEB_IDE.md M5              |
| W6        | Export/import ZIP + PWA offline                      | MH | ✓      | W4           | WEB_IDE.md M6              |

_W2 and W4 can be developed in parallel after W1; W3 and W5 can follow independently._

---

## 1.1+ — Backlog

| ID        | Title                                                | E  | Design | Depends on   | Source                     |
|-----------|------------------------------------------------------|----|--------|--------------|----------------------------|
| W1.14     | WASM Tier 2: Web Worker pool; `par()` parallelism    | VH | ✓      | W1.13, W4    | WASM.md — Threading        |
| S17       | Slot: text below TOS in nested scopes                | M  | —      |              | CAVEATS.md C4              |
| S18       | Slot: sequential file blocks conflict                | M  | —      |              | CAVEATS.md C5              |
| A12       | Lazy work-variable initialization                    | M  | ~      |              | PLANNING.md A12            |
| O1        | Superinstruction peephole rewriting                  | M  | ~      |              | compile.rs                 |
| O2        | Stack raw pointer cache                              | H  | ~      |              | PERFORMANCE.md P2          |
| A4        | Spatial index operations                             | H  | ~      |              | PROBLEMS.md #22            |
| A4.1      | ↳ Insert + exact lookup                              | M  | ~      |              | database.rs                |
| A4.2      | ↳ Bounding-box range query                           | M  | ~      | A4.1         | database.rs                |
| A4.3      | ↳ Removal                                            | S  | ~      | A4.1         | database.rs                |
| A4.4      | ↳ Full iteration                                     | S  | ~      | A4.2, A4.3   | database.rs                |
| O4        | Native: direct-emit local collections                | H  | ~      |              | PERFORMANCE.md N1          |
| O5        | Native: omit `stores` from pure functions            | H  | ~      | O4           | PERFORMANCE.md N2          |
| T1.8      | Tuple: function return + text elements               | M  | ✓      | T1.1–7       | CAVEATS.md C20             |
| CO1.4-fix | `yield from` slot-assignment regression fix          | M  | ✓      | CO1.4        | CAVEATS.md C21             |
| A5.6      | Closure: mutable + text capture                      | M  | ✓      | A5.1–5       | CAVEATS.md C1              |
| N8        | Native codegen: tuples, coroutines, generics         | H  | ✓      | T1, CO1      | CAVEATS.md C19             |
| L9        | Format specifier mismatch → compile error            | S  | ✓      | L8           | CAVEATS.md C14             |
| L10       | `while` loop syntax sugar                            | S  | ✓      |              | CAVEATS.md C11             |

---

## Deferred indefinitely

| ID    | Title                                                | E  | Notes                            |
|-------|------------------------------------------------------|----|----------------------------------|
| P4    | Bytecode cache (`.loftc`)                            | M  | Superseded by native codegen     |
| A7.4  | External libs: package registry + `loft install`     | M  | 2.x; ecosystem must exist first  |

---

## See also

- [PLANNING.md](PLANNING.md) — Full descriptions, fix paths, and effort justifications for every item
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark data and designs for O1–O7
- [DEVELOPMENT.md](DEVELOPMENT.md) — Branch naming, commit sequence, and CI workflow
- [RELEASE.md](RELEASE.md) — Gate criteria each milestone must satisfy before tagging
