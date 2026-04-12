
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
| 22 | `spacial<T>` keyword reserved but unimplemented | Low | **0.9.0:** remove the keyword until A4 ships (treat as unknown type) |
| 54 | `json_items` returns opaque `vector<text>` | Medium | **0.9.0:** ship `JsonValue` enum (promoted from 1.1+) — typeless API contradicts loft's type-system promise |
| 91 | `init(expr)` parameter defaults cannot reference earlier args | Medium | **0.9.0:** inject earlier args into `self.vars` during `parse_arguments` |
| 135 | Canvas Y-flip three-way compensation off-by-ones 2×N atlases | Medium | **0.8.5:** normalise to screen-top-left `(0,0)` — remove upload-side row reversal |
| 137 | `loft --html` Brick Buster runtime `unreachable` panic | High | **0.8.5 blocker:** phase-C bisection of `#native` functions |

---

## Unimplemented Features

### 22. `spacial<T>` keyword reserved but unimplemented — 0.9.0: remove

`spacial<T>` emits a compile-time error today:

```
spacial<T> is not yet implemented; use sorted<T> or index<T> for ordered lookups
```

**Decision:** a keyword that always errors claims namespace and
misleads users.  Remove `spacial` as a reserved keyword in 0.9.0 —
treat `spacial<T>` as a plain unknown-type error alongside any other
misspelled identifier.  Re-add when A4 actually ships the radix/R-tree
backing (1.1+).

**Fix path:** delete the `"spacial"` match arm in
`src/parser/definitions.rs`, remove `Type::Spacial` from `src/data.rs`,
and drop the surrounding schema hooks in `src/database/types.rs`.
The existing schema allocation is dead code.  Update
`tests/parse_errors.rs::spacial_not_implemented` to expect the generic
"unknown type" diagnostic instead of the current bespoke message.

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

### 91. `init(expr)` / default-from-earlier-parameter — 0.9.0

**Symptom:** `fn make_rect(w: integer, h: integer = w)` fails with
*"Unknown variable 'w'"*; the default expression cannot reference
earlier parameters of the same function.

**Decision:** implement for 0.9.0.  This is an idiomatic default
most scripting languages support; failing with an "unknown variable"
message is a stumbling-stone for new users.

**Fix path:** in `parse_arguments`, inject each parsed argument into
`self.vars` (via `add_variable` + `become_argument` + `defined`)
before parsing the next default expression.  A first attempt hit
two-pass-parser state issues; the re-attempt needs to cleanly
separate "scratch binding for default resolution" from the real
argument slots the caller adds afterwards.

---

## Web Services

### 54. `json_items` returns opaque `vector<text>` — 0.9.0

**Symptom:** `json_items(body)` returns `vector<text>` where each
element is either a JSON object body or garbage.  The caller writes
`MyStruct.parse(body)` and gets a partial zero-value struct on malformed
input — no type checking, no diagnostic.

**Decision:** ship the `JsonValue` enum in 0.9.0 (promoted from 1.1+).
Typeless "vector<text>" APIs contradict loft's own pitch as a
statically-typed language.

**Fix path:** `JsonValue { Object(hash<JsonPair[key]>), Array(vector<JsonValue>),
String(text), Number(float), Boolean(boolean), Null }` with
`JsonValue.parse(text) -> JsonValue` and `value.as_struct<T>() -> T`.
See [WEB_SERVICES.md](WEB_SERVICES.md).

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
