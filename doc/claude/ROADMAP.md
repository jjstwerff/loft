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

## 0.8.4 — HTTP client + OpenGL library

All 0.8.4 features are implemented as **loft libraries** that work across all three
backends (interpreter, `--native` codegen, WASM).  The design follows the existing
cross-backend pattern established by the standard library:

**Architecture:**
- **Pure-loft types and logic** in `.loft` files — parsed identically by all backends.
- **Native ops** (Rust functions) only for operations loft cannot express: HTTP sockets,
  PNG I/O, GPU context creation, font rasterization.
- **Platform bridge** via the existing `#[cfg(feature)]` pattern in `src/wasm.rs`:
  interpreter/native use Rust crates directly; WASM calls JavaScript host functions.
- **Library packaging** via `loft.toml` manifests (EXTERNAL_LIBS.md Phase 1) so users
  `import "graphics"` or `import "web"` without knowing which backend runs underneath.

| Layer | HTTP (H4) | Graphics (GL0–GL4) | Desktop GL (GL5) | WebGL (GL6) |
|-------|-----------|-------------------|-------------------|-------------|
| **Loft types** | `HttpResponse` in `default/04_web.loft` | `Rgba`, `Canvas`, `Mesh`, `Scene` in `lib/graphics/*.loft` | reuses GL4 types | reuses GL4 types |
| **Loft logic** | JSON via `{v:j}` / `Type.parse()` | blend, line, Bezier, fill, GLB writer — all in loft | — | — |
| **Native ops** | `ureq` HTTP calls | `save_png` (existing), `fontdue` glyph raster | `glutin` window + `glow` GL calls | — |
| **WASM bridge** | `fetch()` via `host_http_get` etc. | `save_png` via host bridge | N/A (no desktop GL) | `<canvas>` WebGL2 context via JS |
| **Cargo feature** | `http` (new) | `png` (existing) + `fontdue` (new) | `opengl` (new, optional) | `wasm` (existing) |

**Interpreter:** loads `.loft` files via `parse_dir`; native ops registered in
`src/native.rs`; extension crates loaded via `load_one` if `native-extensions` enabled.

**Native codegen:** same `.loft` files parsed; `#rust` annotations emit inline Rust;
native ops compiled directly into the generated binary via `codegen_runtime`.

**WASM:** `.loft` files embedded as `include_str!` in `src/wasm.rs`; native ops bridged
to JavaScript host functions; GL5 is N/A (no desktop GL in browser), GL6 uses WebGL2
via `web-sys` bindings.

JSON serialisation (`{value:j}`) and deserialisation (`Type.parse(text)`, `vector<T>.parse()`)
are already implemented.  No `#json` annotation needed — see [WEB_SERVICES.md](WEB_SERVICES.md).

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| H4        | HTTP client stdlib + `HttpResponse` (ureq)                | M  | ✓      |              | WEB_SERVICES.md               |
| H4.1      | ↳ `HttpResponse` struct + `ok()` method                   | S  | ✓      |              | default/04_web.loft           |
| H4.2      | ↳ `http_get` / `http_post` / `http_put` / `http_delete`   | M  | ✓      | H4.1         | native_http.rs + wasm bridge  |
| H4.3      | ↳ Header support (`http_get_h`, `http_post_h`)            | S  | ✓      | H4.2         | native_http.rs + wasm bridge  |
| H4.4      | ↳ Documentation + integration tests                       | S  | ✓      | H4.2         | tests/docs/                   |
| GL0       | Graphics library scaffolding (files, stubs, fontdue dep)  | S  | ✓      |              | OPENGL_IMPL.md Phase 0       |
| GL1       | Canvas: Rgba, Canvas struct, pixel ops, blend, save_png   | M  | ✓      | GL0          | OPENGL_IMPL.md Phase 1       |
| GL2       | Drawing primitives: line, rect, circle, Bezier, fill      | MH | ✓      | GL1          | OPENGL_IMPL.md Phase 2       |
| GL3       | Text rendering: fontdue glyph raster, draw_text           | M  | ✓      | GL1          | OPENGL_IMPL.md Phase 3       |
| GL4       | GLB export: mesh, scene, material → binary glTF file      | H  | ✓      | GL1          | OPENGL_IMPL.md Phase 4       |
| GL5       | OpenGL desktop: window, shader, render loop (optional)    | H  | ✓      | GL4          | OPENGL_IMPL.md Phase 5       |
| GL6       | WebGL browser: canvas WebGL2 context via web-sys           | H  | ✓      | GL4          | OPENGL_IMPL.md Phase 6       |
| TS        | Test server: embedded Rust HTTP server for H4 integration | M  | ✓      |              | PLANNING.md § TS              |
| TS.1      | ↳ `loft serve app.loft` — tiny HTTP server binary         | S  | ✓      |              | src/serve.rs                  |
| TS.2      | ↳ Route dispatch: loft `fn handle(r: Request) -> Response`| M  | ✓      | TS.1         | src/serve.rs                  |
| TS.3      | ↳ H4 integration tests against local test server          | S  | ✓      | TS.2, H4.2   | tests/web/                    |

**Test server design (TS):**

A minimal Rust HTTP server (`src/serve.rs`, new binary `loft-serve`) that loads a
loft script and calls a user-defined `fn handle(request: Request) -> Response` for
each incoming HTTP request.  Purpose: provide a local test target for H4 integration
tests without depending on external services.

```
loft serve app.loft --port 8080
```

**How it works:**
1. `src/serve.rs` uses `tiny_http` (small, no async, no tokio) to accept connections.
2. For each request, it constructs a loft `Request` struct (method, path, headers, body)
   and calls `fn handle(r: Request) -> Response` in the loaded loft script.
3. The loft function returns a `Response` struct (status, content_type, body).
4. The Rust server writes the HTTP response back to the client.

**Loft types** (in `default/04_web.loft`):
```loft
struct Request {
    method: text
    path: text
    body: text
}

struct Response {
    status: integer init(200)
    content_type: text init("text/plain")
    body: text
}
```

**Example loft script** (`examples/hello_server.loft`):
```loft
import "web"

fn handle(r: Request) -> Response {
    if r.path == "/hello" {
        Response { body: "Hello, {r.method}!" }
    } else if r.path == "/json" {
        data = MyData { name: "test", value: 42 };
        Response { content_type: "application/json", body: "{data:j}" }
    } else {
        Response { status: 404, body: "not found" }
    }
}
```

**H4 integration test** (`tests/web/`):
```rust
// 1. Start loft-serve with tests/web/test_server.loft on a random port
// 2. Use http_get/http_post from the loft H4 client to hit localhost
// 3. Assert response status, body, content-type
// 4. Shut down server
```

This tests the full round-trip: loft HTTP client → Rust TCP → loft request handler
→ Rust TCP → loft HTTP response parsing.  No external dependencies, fully offline,
deterministic.

**Cargo feature:** `serve` (new, optional) — gates `tiny_http` dependency.
Not included in default features or WASM builds.

---

## 0.9.0 — Standalone executable + developer warnings

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| L1        | Error recovery after token failures                       | M  | ✓      |              | PLANNING.md § L1              |
| A2        | Logger: hot-reload, run-mode, release + debug             | M  | ✓      |              | LOGGER.md                     |
| A2.1      | ↳ Wire hot-reload in log functions                        | S  | ✓      |              | native.rs                     |
| A2.2      | ↳ `is_production()` + `is_debug()` + `RunMode`            | S  | ✓      |              | 01_code.loft                  |
| A2.3      | ↳ `--release` flag + `debug_assert()` elision             | MH | ✓      | A2.2         | control.rs, main.rs           |
| A2.4      | ↳ `--debug` per-type safety logging                       | M  | ✓      | A2.2         | fill.rs, native.rs            |
| C52       | Stdlib name clash: warning + `std::` prefix               | M  | ✓      |              | PLANNING.md § C52             |
| W-warn    | Developer warnings (Clippy-inspired)                      | M  | —      |              | see below                     |
| P2        | REPL / interactive mode                                   | H  | ✓      | L1           | PLANNING.md § P2              |
| P2.1      | ↳ Input completeness detection                            | S  | ✓      |              | new repl.rs                   |
| P2.2      | ↳ Single-statement execution                              | M  | ✓      | P2.1         | main.rs, repl.rs              |
| P2.3      | ↳ Automatic value output                                  | S  | ✓      | P2.2         | repl.rs                       |
| P2.4      | ↳ Error recovery in session                               | M  | ✓      | P2.2, L1     | repl.rs, parser.rs            |

### W-warn — Developer warnings

Loft currently emits 6 warnings.  The following additional warnings would
catch common mistakes earlier, inspired by Rust's compiler and Clippy:

**Existing warnings (0.8.3):**
- Dead assignment — variable overwritten before read
- Variable never read — assigned but never used
- Parameter never read — function parameter unused
- Unreachable code — code after return/break/continue
- Format specifier mismatch — `:x` on text, `:05` on boolean
- Unnecessary const — `const` parameter never modified

**Proposed new warnings (0.9.0):**

| Warning | Rust/Clippy equivalent | Example |
|---------|----------------------|---------|
| Comparison always true/false | `clippy::absurd_extreme_comparisons` | `x >= 0` when x is `integer not null` |
| Unnecessary parentheses | `clippy::unnecessary_parens` | `if (x > 0) { ... }` |
| Empty loop/if body | `clippy::empty_loop` | `for x in v { }` |
| Single-element vector literal | `clippy::single_element_loop` | `for x in [42] { ... }` |
| Shadowed variable in same scope | `clippy::shadow_unrelated` | `x = 1; x = "hello"` (type change) |
| Unused import | `unused_imports` | `use lib;` but no `lib::` references |
| Identical if/else branches | `clippy::if_same_then_else` | `if c { x } else { x }` |
| Division by literal zero | compile-time div-by-zero | `x / 0` |
| Infinite loop without break | `clippy::empty_loop` | `while true { }` without break |
| Stdlib name shadow | C52 | `fn len(t: text)` shadows stdlib |

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
| W1.18-6   | Remove `19-threading.loft` from `WASM_SKIP`               | S  | ✓      | W1.18-5      | tests/wrap.rs                 |
| W1.14     | WASM Tier 2: Web Worker pool; `par()` parallelism         | VH | ✓      | W1.18        | WASM.md — Threading           |
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
