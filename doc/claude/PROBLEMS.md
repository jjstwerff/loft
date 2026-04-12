
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Problems in Loft

Known bugs, unimplemented features, and limitations in the loft
language and interpreter.  Each entry records the symptom, workaround, and
recommended fix path.

Completed fixes are removed — history lives in git and `CHANGELOG.md`.

## Contents
- [Open Issues — Quick Reference](#open-issues--quick-reference)
- [Unimplemented Features](#unimplemented-features)
- [Interpreter Robustness](#interpreter-robustness)
- [Web Services Design Constraints](#web-services-design-constraints)
- [Graphics / WebGL](#graphics--webgl)

---

## Open Issues — Quick Reference

| # | Issue | Severity | Workaround |
|---|-------|----------|------------|
| 22 | Spatial index (`spacial<T>`) operations not implemented | Low | Compile-time error; use `sorted<T>` or `index<T>` |
| 54 | `json_items` returns opaque `vector<text>` | Low | Accepted limitation; `JsonValue` enum deferred |
| 55 | Thread-local `http_status()` not parallel-safe | Medium | Design constraint — use `HttpResponse` struct |
| ~~86~~ | Lambda capture | — | **Fully resolved** — real closures shipped in 0.8.3; the original codegen self-reference error is no longer reachable. Regression guards: `tests/issues.rs::p1_1_lambda_void_body` (runtime `count == 42`), plus `capture_detected` / `no_capture_no_error` / `local_not_captured` in `tests/parse_errors.rs` |
| 90 | `fn_call` HashMap lookup per call | Low | Negligible overhead; encode line in `OpCall` if measured |
| 91 | `init(expr)` parameter form missing | Low | Pass default explicitly at call site |
| 135 | Sprite atlas row indexing swap | Low | Cosmetic — affects 2×2 atlas layout |
| 137 | `loft --html` Brick Buster runtime `unreachable` panic | Medium | Native mode works; browser build broken after instantiate |
| 138 | `--native` rustc E0460: rand_core version mismatch | Low | Mitigated — `make play` builds `--lib --bin` together and the `--native` driver prints an actionable hint on E0460 |

---

## Unimplemented Features

### 22. Spatial index operations are not implemented

`spacial<T>` in any field or variable type emits a compile-time error:

```
spacial<T> is not yet implemented; use sorted<T> or index<T> for ordered lookups
```

Both first-pass and second-pass parsers reject it, so no program reaches
the runtime "Not implemented" panics.  Test:
`spacial_not_implemented` in `tests/parse_errors.rs`.

**Remaining work:** implement insert, lookup, copy, remove, iteration in
`database.rs` and `fill.rs`.  Iteration first, then remove, then copy.
The spacial index structure (radix tree or R-tree) is already allocated
in the schema; iteration traversal is the main missing piece.

---

## Interpreter Robustness

### ~~86~~. Lambda capture — FULLY RESOLVED (closures shipped)

With real closure capture in 0.8.3, the original codegen error
`[generate_set] ... Var(1) self-reference — storage not yet allocated`
is no longer reachable.  The parser-level mitigation
(*"lambda captures variable X — closure capture is not yet supported"*)
is also gone since the feature is implemented.

The original reproducer now runs correctly end-to-end:

```loft
fn test() {
    count = 0;
    f = fn(x: integer) { count += x; };
    f(10); f(32);
    assert(count == 42);   // passes
}
```

**Regression guards:**
- `tests/issues.rs::p1_1_lambda_void_body` — runtime behaviour (`count == 42`)
- `tests/parse_errors.rs::capture_detected` — parse succeeds, no diagnostic
- `tests/parse_errors.rs::no_capture_no_error` — no false capture positives
- `tests/parse_errors.rs::local_not_captured` — lambda-local vars don't trigger capture

No open action.  Kept here as a marker for CHANGELOG readers; remove on
the next 0.9.0 maintenance sweep.

---

### 90. `fn_call` HashMap lookup for source line on every call

**Symptom:** Every loft function call performs
`self.line_numbers.get(&self.code_pos)` inside `fn_call`.  Before
stack-trace support, the line lookup only ran during the rare
`stack_trace()` snapshot.

**Root cause:** The source line is not encoded in `OpCall` operands; it
lives in `line_numbers: HashMap<u32, u32>` keyed by bytecode position
and must be looked up at runtime.

**Workaround:** None needed — O(1) amortised lookup, small against the
`Vec::push` + dispatch already in `fn_call`.

**Fix path (if measured significant):** encode the source line as an
extra `u32` operand of `OpCall` in codegen; the `call` handler reads it
and passes it to `fn_call`, eliminating the runtime lookup at the cost
of 4 bytes per `OpCall`.

---

### 91. `init(expr)` parameter form missing

**Symptom:** `init(expr)` on function parameters (dynamic defaults
computed from earlier parameters) is not implemented.

**Scope:** The core struct-field `init(expr)` works correctly —
evaluated once at creation, `$` references resolved, writable after
construction.  Circular detection on struct fields is also in place
(`tests/scripts/72-parse-error-caveats.loft`).  Only the parameter-form
extension is missing.

**Workaround:** Compute the default at the call site and pass it
explicitly.

**Fix path:** in `parse_arguments`, accept `init(expr)` alongside
`= expr`; store the expression in `Attribute.value`; at the call site,
emit the expression when no argument is supplied.

---

## Web Services Design Constraints

### 54. `json_items` returns opaque `vector<text>` — no compile-time element type

**Severity:** Low — accepted design limitation

`json_items(body)` parses a JSON array and returns element bodies as
`vector<text>`.  The compiler cannot verify that the caller's parse
function receives a valid JSON object body rather than an arbitrary
string; a runtime parse error produces a partial zero-value struct
rather than a diagnostic.

**Workaround:** validate the HTTP response status before parsing
(`if resp.ok()`).

**Fix path (deferred):** a `JsonValue` enum (Object / Array / String /
Number / Boolean / Null) gives compile-time structure but at high design
cost.  Target: 1.1+.

See also: [WEB_SERVICES.md](WEB_SERVICES.md).

---

### 55. Thread-local `http_status()` is not parallel-safe

**Severity:** Medium — design trap; do not introduce this API.

An `http_status()` returning the most-recent HTTP status as a
thread-local integer (C `errno` pattern) is incorrect under loft's
parallel execution model.  A `parallel_for` worker calling `http_get`
would corrupt the thread-local of the calling thread.

**Resolution:** return an `HttpResponse` struct from all HTTP functions.
Status is a field on the returned value, not global state.  See
WEB_SERVICES.md Approach B.  This is a design constraint to observe,
not a bug to fix.

---

## Graphics / WebGL

### 135. Sprite atlas row indexing swap

**Severity:** Low — cosmetic.

**Symptom:** in a 2×2 sprite atlas, sprites 1 and 3 appear at
swapped canvas positions when drawn via `draw_sprite`.  The smoke
test (`tests/scripts/snap_smoke.sh`) pixel-samples the affected
corners and confirms the mis-placement is reproducible.

**Root cause:** interaction between `gl_upload_canvas`'s Y-flip
(row reversal during upload, `lib.rs:837`), `draw_sprite`'s
V-coordinate computation (`graphics.loft:773-776`), and the
orthographic projection in `create_painter_2d` (`-2/H`, which also
flips Y).  Two of the three flips cancel; the third lands in an
unexpected quadrant, so row indexing into the atlas is off by one
row.

**Workaround:** arrange sprites in a single row (N×1 atlas) until
the flip sequence is normalised.

**Fix path:** decide a single canonical Y direction (screen-origin
top-left) and remove the compensating flip from one of the three
sites — most naturally the upload, since it's the one introduced
last.  Test: extend `snap_smoke.sh` to assert all four corners of
a 2×2 atlas are placed correctly.

---

### 137. `loft --html` Brick Buster: runtime `unreachable` panic

**Severity:** Medium — breaks the deployed `brick-buster.html` on
GitHub Pages; the wasm instantiates but panics as soon as `loft_start`
runs.

**Symptom:** the browser reports

```
Uncaught (in promise) RuntimeError: unreachable executed
    at wasm-function[234]:…
    at wasm-function[229]:…
    …
    at wasm-function[258]:…
```

Reproducible in Node with stub imports: `loft_start()` throws
`unreachable` on the first call, regardless of whether asyncify is
enabled (tested with `wasm-opt -O1 --asyncify` and with no asyncify
pass at all).

**Narrowed down:**

- Not an instantiation failure — all 25 host imports (`loft_gl.*`,
  `loft_io.*`) are present and the wasm compiles.  Pull request #168
  fixed the earlier instantiation-time bug by switching `-Oz` to
  `-O1`; this new failure is at *runtime*, not at instantiate.
- Not a generated-Rust `todo!()` — `grep -c 'todo!'` on the emitted
  `/tmp/loft_html.rs` returns 0.  Every `#native` function has a real
  extern declaration + call.
- Not an asyncify artefact — reproduces with `wasm-opt -O1
  --strip-debug --strip-producers` (no `--asyncify`).
- The panic originates in generated bytecode dispatch, not in a
  host-call — the call stack has no import frames.

**Workaround:** native mode (`make play`) runs the game correctly;
only the browser build is broken.

**Fix path:**

1. Capture the pre-wasm-opt `/tmp/loft_html.wasm` and instantiate it
   directly in Node to confirm the panic is in the rustc output, not
   a wasm-opt transformation.
2. Bisect which `#native` function's return path is unsafe: stub
   each import individually with a `throw new Error(name)` sentinel
   and see which one is hit last before the unreachable — that
   narrows the loft function whose emitted Rust body diverges.
3. Inspect the emitted Rust for that function in
   `src/generation/dispatch.rs::output_native_direct_call` — likely
   a type-marshalling mismatch between the loft signature and the
   generated `extern "C"` prototype (e.g. a `text` param that
   should pass `ptr, len` but was emitted as a single `i32`).
4. Add a browser-path assertion to `make game` that instantiates
   the built wasm in Node and runs `loft_start` against `loft-gl-wasm.js`
   stubs, failing CI if it panics.

**Tracking:** discovered 2026-04-12 while verifying the
`make play` target.  Native path works; browser path wedged.

---

### 138. `--native` rustc E0460: `rand_core` version mismatch

**Severity:** Medium — blocks `loft --native <script>` and `make play`
on a checkout where `cargo build --release --bin loft` has run without
`--lib`.

**Symptom:** `rustc` fails compiling the generated `/tmp/loft_native.rs`
with

```
error[E0460]: found possibly newer version of crate `rand_core` which `loft` depends on
  --> /tmp/loft_native.rs:16:1
   |
16 | extern crate loft;
   | ^^^^^^^^^^^^^^^^^^
   = note: the following crate versions were found:
           crate `rand_core`: …/librand_core-<hashA>.rmeta
           crate `rand_core`: …/librand_core-<hashB>.rmeta
           crate `rand_core`: …/librand_core-<hashC>.rmeta
           crate `loft`: …/libloft.rlib
```

The E0460 cascades: every subsequent `use loft::codegen_runtime::*;`
fails to resolve, producing 700+ "cannot find function `OpNewRecord`"
/ `cr_call_push` / `OpFreeRef` / `n_set_store_lock` etc. E0425 errors.
The generated source itself is fine — rustc can't load the `loft` crate.

**Root cause:** cargo's incremental-build state has `libloft.rlib`
referencing an older `rand_core` rmeta hash than what's currently in
`target/release/deps/`.  This happens when `--bin loft` rebuilds but
`--lib` is left stale.

**Workaround (already shipped):** `make play` step 1 now runs
`cargo build --release -q --lib --bin loft` so the rlib is always
current.  A manual `cargo clean && cargo build --release` is the
fallback when a user's tree has other stale artefacts.

**Mitigation (shipped, `src/main.rs`):** the `--native` driver now
captures rustc's stderr and, on E0460 with "rand_core" or
"possibly newer version of crate", prints an actionable hint —

```
loft: native compilation failed because the cached `libloft.rlib`
references a different dependency version than the one now in
`target/release/deps/`.

Fix:  cargo build --release --lib --bin loft
Or:   cargo clean && cargo build --release
```

This replaces the previous 700-error cascade with a single recovery
instruction.  Test: introduce a stale rlib (`cargo build --bin loft`
after modifying a dependency version) and run
`loft --native <any-file>` — the hint should appear.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [CAVEATS.md](CAVEATS.md) — Verifiable edge cases with reproducers
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements
