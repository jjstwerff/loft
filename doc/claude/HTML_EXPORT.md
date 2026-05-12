<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# HTML export pipeline — `loft --html`

Reference for the shipped single-file HTML export pipeline.
`loft --html game.loft` produces a self-contained `.html` file
that runs a fully compiled loft program in the browser at
native speed.

**Non-goal:** embedding the interpreter.  The program compiles
to optimised WASM via the native codegen pipeline; the HTML
file is just a base64-embedded WASM blob + inline JS bridge.

## Pipeline architecture

```
loft --html game.loft
  │
  ├─ Parse + compile (existing)
  ├─ Generate Rust source (existing output_native_reachable)
  ├─ rustc --target wasm32-unknown-unknown --crate-type cdylib
  ├─ wasm-opt -O1 (optional)
  └─ Assemble HTML (base64 WASM + inline JS bridge)
```

**Size:** ~200–400 KB raw WASM, ~70–130 KB gzipped (typical).

The CLI flag is parsed in `src/main.rs` (alongside `--native-emit`).
Output defaults to `game.html` next to the source; override with
`--html out.html`.

## Codegen — cdylib entry point

In normal `--native` mode, the generated Rust has `fn main()`.
For `--html`, codegen instead emits an exported entry point:

```rust
use std::cell::RefCell;

thread_local! {
    static STORES: RefCell<Option<Stores>> = RefCell::new(None);
}

#[no_mangle]
pub extern "C" fn loft_start() -> i32 {
    let mut stores = Stores::new();
    init(&mut stores);
    n_main(&mut stores);
    0  // 0 = finished, 1 = yielded (frame loop)
}
```

The `cdylib` crate type produces a `.wasm` module rather than a
binary.  The exported `loft_start` is what the JS bridge calls.

## JS bridge — host imports

The HTML loader passes a JS `imports` object into the WASM
instance.  The shape:

```javascript
const imports = {
  env: {
    // println goes through this host import
    loft_println: (ptr, len) => { /* read from memory.buffer, decode UTF-8, append */ },

    // GL functions are imports — see "GL bridge" below
    gl_clear: (r, g, b, a) => { /* WebGL2 call */ },
    gl_draw: (vao, count) => { /* WebGL2 call */ },
    gl_swap_buffers: () => { /* request next animation frame */ },
    gl_poll_events: () => { /* return 0 if user closed tab, 1 otherwise */ },
    // … one entry per GL function the program uses

    // Frame yield — see "Frame yield contract" below
    loft_yield: () => { /* save state, request animation frame, exit loft_start */ },
  }
};

WebAssembly.instantiate(wasmBytes, imports).then(r => {
  instance = r.instance;
  instance.exports.loft_start();
});
```

Strings cross the boundary as `(ptr, len)` pairs into the WASM
linear memory.  The JS side reads via
`new Uint8Array(instance.exports.memory.buffer)` and decodes with
`TextDecoder('utf-8')`.

## GL bridge — WebGL2 imports

Every GL function the loft program uses becomes an import.
Codegen scans the reachable native call graph and emits one
import per `gl_*` function found.  The JS bridge implements
each as a thin wrapper over the WebGL2 context obtained from
the `<canvas>`:

```javascript
const gl = canvas.getContext('webgl2', { antialias: true });

// Each gl_* loft function maps to one or more WebGL2 calls
const imports = {
  env: {
    gl_clear: (r, g, b, a) => {
      gl.clearColor(r/255, g/255, b/255, a/255);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    },
    gl_draw: (vao, count) => {
      gl.bindVertexArray(vaoTable[vao]);
      gl.drawArrays(gl.TRIANGLES, 0, count);
    },
    // VAO / shader / buffer / texture handles are integer ids stored
    // in JS-side tables, indexed by the values loft passes around.
    // …
  }
};
```

VAOs, shaders, buffers, and textures are GL handles in JS
tables; loft programs pass around the integer ids returned by
`gl_create_*` functions.

## Frame yield contract — browser game loop

The interpreter / native code runs synchronously.  In the
browser, that would block the main thread for an entire frame
sequence — only the last frame would render, no input would
process.

The pipeline solves this by **yielding at `gl_swap_buffers`**:

1. The loft `gl_swap_buffers()` call is mapped to an import
   `loft_yield`.
2. `loft_yield` saves the loft program's state into the
   `STORES` thread-local, requests `requestAnimationFrame`,
   then returns control to the JS bridge.
3. JS receives control; on the next frame, it resumes loft
   from where it left off.

User loft code is **identical** on native and browser:

```loft
for _ in 0..3600 {
  if !gl_poll_events() { break }
  gl_clear(bg);
  gl_draw(vao, count);
  gl_swap_buffers();   // ← yields on browser; blocks 16ms on native
}
```

No callback pattern, no API change, no user-visible
generators.  This is the design that "Step 7: Frame yield"
shipped with.

The native target uses `wasm-opt --asyncify` if available, OR
a manual state-machine yield implemented in codegen
(measure-and-decide; the current implementation favours
asyncify for binary size).

## HTML assembly — what ships in the output

The output `.html` is a single self-contained file:

```html
<!DOCTYPE html>
<html><head>
  <meta charset="utf-8">
  <title>Loft Program</title>
  <style>/* canvas full-bleed; pre console for non-GL programs */</style>
</head><body>
  <canvas id="c" tabindex="0" style="display:none"></canvas>
  <pre id="out"></pre>
  <script>
    const wasmB64 = "<base64-encoded WASM>";
    const wasmBytes = Uint8Array.from(atob(wasmB64), c => c.charCodeAt(0));
    /* host imports as described above */
    /* WebAssembly.instantiate + loft_start() */
  </script>
</body></html>
```

The `<canvas>` is hidden by default; GL functions show it on
first call.  The `<pre id="out">` collects `println` output.

## wasm-opt integration

If `wasm-opt` is on `PATH`, the pipeline runs `-O1` post-
compile to shrink the binary.  `-Oz` was tried but was too
slow for the iterative-build use case (CLI feedback latency
matters more than a few KB).  See CHANGELOG_TECHNICAL.md
"loft --html switched to wasm-opt -O1" for the rationale.

If `wasm-opt` is not on `PATH`, the pipeline skips the
optimisation step with a warning.

## Customisation hooks

Limited surface today; expand as need arises:

- `--html out.html` — explicit output path
- `LOFT_HTML_TITLE=…` (env var) — override the `<title>`
- WASM imports use the `env` namespace — replace any import
  via a custom JS bridge if you wrap the output HTML

## Build prerequisites

For users running `loft --html`:

- Rust toolchain with `wasm32-unknown-unknown` target
  installed (`rustup target add wasm32-unknown-unknown`).
- `libloft.rlib` + transitive deps for that target
  (installed by `make install` to
  `/usr/local/share/loft/wasm32-unknown-unknown/`).
- `wasm-opt` on `PATH` (optional but recommended for size).
- A modern browser with WebGL2 + WebAssembly + ES module
  support (any 2020+ browser).

## Where to find the code

| Concern | Location |
|---|---|
| `--html` CLI flag parsing | `src/main.rs` |
| Codegen entry-point switch (cdylib vs main) | `src/generation/mod.rs` (output_native_reachable, with `wasm_cdylib: bool`) |
| WASM-import emission for GL / println / yield | `src/generation/` (ops dispatch + native registry) |
| HTML assembly + base64 embedding | `src/main.rs` (HTML template + `wasm-opt` invocation) |
| End-to-end test | `tests/html_wasm.rs` (runs `--html` against fixture loft programs, verifies the output HTML loads + executes via headless chromium) |
| Makefile targets | `make game` (brick-buster.html), `make wasm-html-test` (E2E gate) |

## Subsequent W1.x work (not part of W1.1)

W1.18 added WASM Worker Thread infrastructure for parallel
`par(...)` in the browser.  W1.19 / W1.20 added the random /
time host bridges.  See [WASM.md](WASM.md) for the broader
WASM runtime + CHANGELOG_TECHNICAL.md for the W1.x history.

The HTML export pipeline itself (W1.1) was complete in 0.8.4;
later W1.x evolutions enhance the runtime but don't change
the export model documented above.

## See also

- [WASM.md](WASM.md) — WASM runtime architecture (VFS, host
  bridges, threading, frame yield mechanics — the layer this
  pipeline produces output for)
- [`lib_plans/future/02-graphics/README.md`](lib_plans/future/02-graphics/README.md) — graphics library design
  (the GL functions imported by the HTML bridge)
- [`lib_plans/future/05-game-infra/README.md`](lib_plans/future/05-game-infra/README.md) — game infrastructure
  (sprites, tilemap, etc., consumed by HTML-exported games)
- `plans/finished/31-html-export/README.md` — closure record
  (commits, evidence, historical 10-step build sequence
  preserved as archaeology)
