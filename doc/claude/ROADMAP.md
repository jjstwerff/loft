// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Roadmap

Items in expected implementation order, grouped by milestone.
Full descriptions and fix paths: [PLANNING.md](PLANNING.md).

**Effort:** XS = Tiny · S = Small · M = Medium · MH = Med–High · H = High · VH = Very High

**Design:** ✓ = detailed design in place · ~ = partial/outline · — = needs design

**Maintenance rule:** When an item is completed, remove it from this file entirely.
Do not keep completed items — the ROADMAP tracks only what remains to be done.
Completed work belongs in CHANGELOG.md (user-facing) and git history (implementation).

---

## 0.8.3 — Language completeness + parallel safety

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| W1.18     | WASM: `par()` via Node.js Worker Threads                  | H  | ✓      |              | WASM.md § W1.18               |
| W1.18-1   | ↳ `#[cfg(wasm+threading)]` branch in `parallel.rs`       | S  | ✓      |              | src/parallel.rs               |
| W1.18-2   | ↳ `worker_entry` exported via `#[wasm_bindgen]`           | S  | ✓      | W1.18-1      | src/lib.rs                    |
| W1.18-3   | ↳ `worker.mjs` — park/wake loop + `worker_entry`          | S  | ✓      | W1.18-2      | tests/wasm/worker.mjs         |
| W1.18-4   | ↳ `parallel.mjs` — `LoftThreadPool` spawn/terminate       | S  | ✓      | W1.18-3      | tests/wasm/parallel.mjs       |
| W1.18-5   | ↳ `harness.mjs` — `initThreaded()` + `@threaded` routing  | S  | ✓      | W1.18-4      | tests/wasm/harness.mjs        |
| W1.18-6   | ↳ Remove `19-threading.loft` from `WASM_SKIP`             | S  | ✓      | W1.18-5      | tests/wrap.rs                 |
| A5.6      | Closure: cross-scope capture (16-byte fn-ref + chained call) | H  | ✓      | A5.6a–h ✓   | PLANNING.md § A5.6            |
| A5.6-1    | ↳ Widen `Type::Function` to 16 bytes + `OpVarFnRef`       | S  | ✓      |              | variables/mod.rs, codegen.rs  |
| A5.6-2    | ↳ `OpStoreClosure` — embed closure DbRef in fn-ref slot   | S  | ✓      | A5.6-1       | fill.rs, vectors.rs           |
| A5.6-3    | ↳ `fn_call_ref` reads closure from bytes 4..16            | S  | ✓      | A5.6-2       | state/mod.rs, control.rs      |
| A5.6-4    | ↳ `parse_part`: chained `expr(args)` on `Type::Function`  | S  | ✓      | A5.6-3       | operators.rs                  |
| A5.6-5    | ↳ Un-ignore `closure_capture_text` test                   | XS | ✓      | A5.6-4       | tests/expressions.rs          |
| A5.6h     | ↳ Fix fn-ref if-else SIGSEGV (`fn_ref_conditional_call`)  | S  | ✓      | A5.6-2       | codegen.rs, emit.rs, mod.rs   |
| CO1.7     | Coroutines: yield from inside for-loops                   | M  | ✓      | CO1.1–CO1.6  | PLANNING.md § CO1.7           |
| CO1.8     | ↳ Multi-text parameters + nested-block safety             | S  | ✓      | CO1.3d       | PLANNING.md § CO1.8           |
| CO1.9     | ↳ Store iteration generation guard in release builds      | S  | ✓      | CO1.6        | PLANNING.md § CO1.9           |
| T1.9      | Tuple destructuring in `match`                            | S  | ✓      |              | TUPLE_MATCH.md                |
| T1.9-1    | ↳ `Type::Tuple` dispatch in `parse_match`                 | XS | ✓      |              | control.rs                    |
| T1.9-2    | ↳ `parse_tuple_match` — arm loop, if-chain                | S  | ✓      |              | control.rs                    |
| T1.9-3    | ↳ `parse_tuple_elem_pattern` — wildcard/binding/literal/range/nested | S | ✓ | | control.rs            |
| T1.9-4    | ↳ Tests + doc additions (`28-tuples.loft`)                | S  | ✓      |              | tests/docs/                   |
| T1.10     | Tuple homogeneous-type coverage (text/store/struct/vector)| S  | ✓      | T1.8a, T1.8b | PLANNING.md § T1.10           |
| T1.11     | Tuple type constraints (struct fields + compound assign)  | XS | ✓      | T1.1, T1.2   | PLANNING.md § T1.11           |
| A8        | Slicing & comprehension on `sorted` / `index`             | M  | ✓      |              | SORTED_SLICE.md               |
| A8.1      | ↳ Open-ended bounds (`col[lo..]`, `col[..hi]`, `col[..]`) | S  | ✓      |              | fields.rs, codegen_runtime.rs |
| A8.2      | ↳ Range slicing on `sorted` (`sorted[lo..hi]`)            | XS | ✓      | A8.1         | fields.rs                     |
| A8.3      | ↳ Partial-key match iterator (`col[k1]` on multi-key)     | M  | ✓      |              | fields.rs                     |
| A8.4      | ↳ Comprehensions on key ranges                            | S  | ✓      | A8.1         | tests/docs/                   |
| A8.5      | ↳ Reverse range iteration (`rev(col[lo..hi])`)            | S  | ✓      | A8.1         | fields.rs, objects.rs         |
| A8.6      | ↳ `match` on collection results (tests + docs)            | S  | ✓      |              | tests/docs/                   |
| A14       | `par_light`: lightweight parallel loop                    | MH | ✓      |              | LIGHT_PAR.md                  |
| A14.1     | ↳ `Store::borrow_locked_for_light_worker` + sentinel Drop | S  | ✓      |              | LIGHT_PAR.md § L1             |
| A14.2     | ↳ `WorkerPool` struct                                     | S  | ✓      | A14.1        | LIGHT_PAR.md § L2             |
| A14.3     | ↳ `Stores::clone_for_light_worker`                        | S  | ✓      | A14.1, A14.2 | LIGHT_PAR.md § L3             |
| A14.4     | ↳ `run_parallel_light`                                    | S  | ✓      | A14.3        | LIGHT_PAR.md § L4             |
| A14.5     | ↳ Compiler call-graph analysis + `M` computation          | M  | ✓      |              | LIGHT_PAR.md § L5             |
| A14.6     | ↳ Parser: `par_light(...)` clause                         | S  | ✓      | A14.4, A14.5 | LIGHT_PAR.md § L6             |
| A14.7     | ↳ Performance benchmark                                   | S  | ✓      | A14.6        | LIGHT_PAR.md § L7             |
| I1        | Interfaces: add `interface` keyword to lexer              | XS | ✓      |              | src/lexer.rs                  |
| I2        | Interfaces: `DefType::Interface` + `Definition.bounds: Vec<u32>` | S | ✓ | I1        | src/data.rs                   |
| I3        | Interfaces: parse interface declarations (first pass)     | M  | ✓      | I2           | src/parser/definitions.rs     |
| I3.1      | ↳ `op <> (...)` sugar in interface bodies → `OpCamelCase` | XS | ✓      | I3           | src/parser/definitions.rs     |
| I4        | Interfaces: `<T: A + B>` bound syntax + conflict detection | S  | ✓      | I2           | src/parser/definitions.rs     |
| I5        | Interfaces: type resolution + `Self` placeholder          | S  | ✓      | I3           | src/typedef.rs                |
| I5.1      | ↳ Phase-1 factory-method restriction diagnostic           | XS | ✓      | I5           | src/typedef.rs                |
| I6        | Interfaces: satisfaction checking at instantiation        | M  | ✓      | I4, I5       | src/parser/definitions.rs     |
| I7        | Interfaces: allow bounded method calls on `T`             | S  | ✓      | I6           | src/parser/control.rs         |
| I8.1      | Interfaces: same-type binary operators (`T op T`)         | S  | ✓      | I6           | src/parser/operators.rs       |
| I8.2      | ↳ Result-type propagation from interface signature        | S  | ✓      | I8.1         | src/parser/operators.rs       |
| I8.3      | ↳ Mixed-type binary operators (`T op concrete`)           | S  | ✓      | I8.2         | src/parser/operators.rs       |
| I8.4      | ↳ Unary operators (`OpNeg`, etc.)                         | XS | ✓      | I8.1         | src/parser/operators.rs       |
| I9        | Interfaces: stdlib (`Ordered`, `Equatable`, `Addable`, `Numeric`, `Scalable`, `Printable`) | M | ✓ | I7, I8.2, I8.3, I8.4 | default/01_code.loft |
| I9.1      | ↳ Convert `sum_of`, `min_of`, `max_of` to bounded-generic loft | S | ✓ | I9          | default/01_code.loft          |
| I9.2      | ↳ `sum_of(v, identity)` caller-supplied-identity overload | XS | ✓      | I9           | default/01_code.loft          |
| I10       | Interfaces: "does not satisfy" diagnostics                | S  | ✓      | I6           | src/diagnostics.rs            |
| I11       | Interfaces: gendoc stub/guard for `DefType::Interface`    | XS | ✓      | I2           | src/documentation.rs          |
| S16b      | Range queries on `sorted<EnumVariant[field]>` resolve key against parent enum → "Unknown in expression type ParentEnum" | S | ✓ | | src/parser/fields.rs, index_type |

---

## 0.8.4 — HTTP client

JSON serialisation (`{value:j}`) and deserialisation (`Type.parse(text)`, `vector<T>.parse()`)
are already implemented.  No `#json` annotation needed — see [WEB_SERVICES.md](WEB_SERVICES.md).

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| H4        | HTTP client stdlib + `HttpResponse` (ureq)                | M  | ✓      |              | WEB_SERVICES.md               |
| H4.1      | ↳ `HttpResponse` struct + `ok()` method                   | S  | ✓      |              | default/04_web.loft           |
| H4.2      | ↳ `http_get`, `http_post`, `http_put`, `http_delete`      | M  | ✓      | H4.1         | native_http.rs                |
| H4.3      | ↳ Header support (`http_get_h`, `http_post_h`)            | S  | ✓      | H4.2         | native_http.rs                |
| H4.4      | ↳ Documentation + integration tests                       | S  | ✓      | H4.2         | tests/docs/                   |

---

## 0.9.0 — Standalone executable

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| L1        | Error recovery after token failures                       | M  | ✓      |              | PLANNING.md § L1              |
| A2        | Logger: hot-reload, run-mode, release + debug             | M  | ✓      |              | LOGGER.md                     |
| A2.1      | ↳ Wire hot-reload in log functions                        | S  | ✓      |              | native.rs                     |
| A2.2      | ↳ `is_production()` + `is_debug()` + `RunMode`            | S  | ✓      |              | 01_code.loft                  |
| A2.3      | ↳ `--release` flag + `debug_assert()` elision             | MH | ✓      | A2.2         | control.rs, main.rs           |
| A2.4      | ↳ `--debug` per-type safety logging                       | M  | ✓      | A2.2         | fill.rs, native.rs            |
| P2        | REPL / interactive mode                                   | H  | ✓      | L1           | PLANNING.md § P2              |
| P2.1      | ↳ Input completeness detection                            | S  | ✓      |              | new repl.rs                   |
| P2.2      | ↳ Single-statement execution                              | M  | ✓      | P2.1         | main.rs, repl.rs              |
| P2.3      | ↳ Automatic value output                                  | S  | ✓      | P2.2         | repl.rs                       |
| P2.4      | ↳ Error recovery in session                               | M  | ✓      | P2.2, L1     | repl.rs, parser.rs            |

---

## 1.0.0 — IDE + stability contract

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| W2        | Editor shell (CodeMirror 6 + Loft grammar)                | M  | ✓      | W1           | WEB_IDE.md M2                 |
| W3        | Symbol navigation (go-to-def, find-usages)                | M  | ✓      | W1, W2       | WEB_IDE.md M3                 |
| W4        | Multi-file projects (IndexedDB)                           | M  | ✓      | W2           | WEB_IDE.md M4                 |
| W5        | Docs & examples browser                                   | MH | ✓      | W2           | WEB_IDE.md M5                 |
| W6        | Export/import ZIP + PWA offline                           | MH | ✓      | W4           | WEB_IDE.md M6                 |

_W2 and W4 can be developed in parallel after W1; W3 and W5 can follow independently._

---

## 1.1+ — Backlog

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| W1.14     | WASM Tier 2: Web Worker pool; `par()` parallelism         | VH | ✓      | W1.13, W4    | WASM.md — Threading           |
| I12       | Interfaces: factory methods (`fn zero() -> Self`) — phase 2 | S | ✓    | I5.1         | INTERFACES.md § Q4/Q6         |
| I8.5      | Interfaces: left-side concrete operand (`concrete op T`)  | S  | ~      | I8.3         | INTERFACES.md § Phase 1 gaps  |
| A12       | Lazy work-variable initialization                         | M  | ✓      |              | PLANNING.md § A12             |
| O2        | Stack raw pointer cache                                   | H  | ✓      |              | PLANNING.md § O2              |
| A4        | Spatial index operations                                  | H  | ✓      |              | PLANNING.md § A4              |
| A4.1      | ↳ Insert + exact lookup                                   | M  | ✓      |              | PLANNING.md § A4 Phase 1      |
| A4.2      | ↳ Bounding-box range query                                | M  | ✓      | A4.1         | PLANNING.md § A4 Phase 2      |
| A4.3      | ↳ Removal                                                 | S  | ✓      | A4.1         | PLANNING.md § A4 Phase 3      |
| A4.4      | ↳ Full iteration                                          | S  | ✓      | A4.2, A4.3   | PLANNING.md § A4 Phase 4      |
| O4        | Native: direct-emit local collections                     | H  | ✓      |              | PLANNING.md § O4              |
| O5        | Native: omit `stores` from pure functions                 | H  | ✓      | O4           | PLANNING.md § O5              |

---

## Deferred indefinitely

| ID    | Title                                                     | E  | Notes                                                              |
|-------|-----------------------------------------------------------|----|-------------------------------------------------------------------|
| O1    | Superinstruction peephole rewriting                       | M  | Blocked: opcode table full (254/256 used); requires opcode-space redesign |
| P4    | Bytecode cache (`.loftc`)                                 | M  | Superseded by native codegen                                       |
| A7.4  | External libs: package registry + `loft install`          | M  | 2.x; ecosystem must exist first                                    |

---

## See also

- [PLANNING.md](PLANNING.md) — Full descriptions, fix paths, and effort justifications for every item
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark data and designs for O1–O7
- [DEVELOPMENT.md](DEVELOPMENT.md) — Branch naming, commit sequence, and CI workflow
- [RELEASE.md](RELEASE.md) — Gate criteria each milestone must satisfy before tagging
