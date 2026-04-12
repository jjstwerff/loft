
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# W1.1 — Single-file HTML Export

## Goal

`loft --html game.loft` produces a self-contained `.html` file that runs a
fully compiled loft program in the browser at native speed.

**Non-goal:** embedding the interpreter.  The game compiles to optimized WASM
via the existing native codegen pipeline.

---

## Architecture overview

```
loft --html game.loft
  │
  ├─ Parse + compile (existing)
  ├─ Generate Rust source (existing output_native_reachable)
  ├─ rustc --target wasm32-unknown-unknown --crate-type cdylib
  ├─ wasm-opt -Oz (optional)
  └─ Assemble HTML (base64 WASM + inline JS bridge)
```

**Size target:** ~200–400 KB raw WASM, ~70–130 KB gzipped.

---

## Step-by-step implementation

Each step is independently testable.  Commit after each green step.

---

### Step 1: Build libloft.rlib for wasm32-unknown-unknown

**What:** Add a second WASM target to `make install` so the loft runtime
library is available for browser compilation.

**File:** `Makefile`

**Change:** After the existing `wasm32-wasip2` build line, add:

```makefile
cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random
```

And install the rlib + deps:

```makefile
sudo install -d /usr/local/share/loft/wasm32-unknown-unknown/deps
sudo install -m 644 target/wasm32-unknown-unknown/release/libloft.rlib \
    /usr/local/share/loft/wasm32-unknown-unknown/
sudo cp target/wasm32-unknown-unknown/release/deps/*.rlib \
    /usr/local/share/loft/wasm32-unknown-unknown/deps/
```

**Test:** `make install` succeeds and
`/usr/local/share/loft/wasm32-unknown-unknown/libloft.rlib` exists.

**Verify:** `ls -la /usr/local/share/loft/wasm32-unknown-unknown/libloft.rlib`

**Risk:** The `loft` crate may not compile for `wasm32-unknown-unknown`
because some modules use `std::fs`, `std::time::Instant`, or other
OS-specific APIs.  The existing `#[cfg(feature = "wasm")]` gates handle
some of this, but `--no-default-features --features random` may hit new
compilation errors.  Fix with `#[cfg]` gates on the affected code.

---

### Step 2: Compile a trivial program to browser WASM

**What:** Manually test that the generated Rust code compiles for
`wasm32-unknown-unknown`.

**No code changes** — this is a manual verification step.

**Test sequence:**
```bash
# 1. Generate Rust from a simple loft program
echo 'fn main() { println("hello"); }' > /tmp/step2.loft
cargo run --bin loft -- --native-emit /tmp/step2.rs /tmp/step2.loft

# 2. Try compiling for wasm32-unknown-unknown
rustc --edition=2024 --target wasm32-unknown-unknown \
  --crate-type cdylib -O \
  --extern loft=/usr/local/share/loft/wasm32-unknown-unknown/libloft.rlib \
  -L dependency=/usr/local/share/loft/wasm32-unknown-unknown/deps \
  -o /tmp/step2.wasm /tmp/step2.rs
```

**Expected:** Compilation fails because `fn main()` is not valid for
`cdylib`.  The error confirms we can link against the rlib — the entry
point is the next step.

**If it fails on libloft compilation issues:** Go back to step 1 and add
`#[cfg]` gates.

---

### Step 3: cdylib entry point codegen

**What:** When generating for `--html`, emit `#[no_mangle] pub extern "C"`
exported functions instead of `fn main()`.

**Files:** `src/generation/mod.rs`

**Change:** The current `output_native_reachable` generates:

```rust
fn main() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_main(&mut stores);
}
```

Add a parameter `wasm_cdylib: bool`.  When true, generate:

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
    // Return 0 = finished, 1 = yielded (frame loop)
    0
}
```

Frame yield (for games with `gl_swap_buffers`) is deferred to step 7.
This step only handles non-interactive programs.

**Test:**
```bash
echo 'fn main() { println("hello"); }' > /tmp/step3.loft
cargo run --bin loft -- --native-emit /tmp/step3.rs /tmp/step3.loft
# Manually verify /tmp/step3.rs has loft_start() instead of main()
# (This requires passing a flag — add --html to --native-emit for now)

rustc --edition=2024 --target wasm32-unknown-unknown \
  --crate-type cdylib -O \
  --extern loft=... -L dependency=... \
  -o /tmp/step3.wasm /tmp/step3.rs

ls -la /tmp/step3.wasm   # should exist
```

**Verify:** The `.wasm` file is produced.  Check size — should be much
smaller than 1.4 MB.

---

### Step 4: Minimal HTML loader (no GL)

**What:** Write a self-contained HTML file that loads and runs a compiled
WASM module for a console-only program.

**Files:** `src/main.rs` — add the `--html` flag and HTML assembly logic.

**Changes:**

1. Parse `--html [out.html]` flag (same pattern as `--native-emit`).

2. After generating and compiling the WASM (steps 2-3), read the `.wasm`
   file, base64-encode it, and write an HTML file:

```html
<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Loft Program</title></head>
<body>
<pre id="out"></pre>
<script>
const wasmB64 = "BASE64_DATA";
const wasmBytes = Uint8Array.from(atob(wasmB64), c => c.charCodeAt(0));

// Minimal host: capture println output
const output = document.getElementById('out');
const decoder = new TextDecoder();

const imports = {
  env: {
    // println writes to the <pre> element
    loft_println: (ptr, len) => {
      const mem = new Uint8Array(instance.exports.memory.buffer);
      output.textContent += decoder.decode(mem.subarray(ptr, ptr + len)) + '\n';
    }
  }
};

let instance;
WebAssembly.instantiate(wasmBytes, imports).then(r => {
  instance = r.instance;
  instance.exports.loft_start();
});
</script>
</body></html>
```

**Note:** `println` in the generated code calls `codegen_runtime::n_println`
which uses Rust's `println!` macro.  For `wasm32-unknown-unknown`, `println!`
goes nowhere (no stdout).  The codegen needs to route `n_println` through
a WASM import instead.  This is addressed in step 5.

**Test:**
```bash
echo 'fn main() { println("hello from wasm"); }' > /tmp/step4.loft
cargo run --bin loft -- --html /tmp/step4.html /tmp/step4.loft
# Open /tmp/step4.html in a browser
# Verify "hello from wasm" appears
```

**If `println` doesn't work:** That's expected — step 5 fixes it.
The test for this step is just that the HTML file is produced and the
WASM loads without errors in the browser console.

---

### Step 5: Route println through WASM import

**What:** Make text output work in the browser.  The generated code's
`n_println` needs to call a host-provided function instead of Rust's
`println!` macro.

**Files:** `src/codegen_runtime.rs` or `src/generation/mod.rs`

**Approach A — codegen-time:** When generating for `--html`, emit
calls to an imported `loft_print(ptr, len)` function instead of
`println!`.  This requires the code generator to know the target.

**Approach B — runtime `#[cfg]`:** In `codegen_runtime.rs`, gate
`n_println` on `#[cfg(not(target_arch = "wasm32"))]` and provide a
WASM variant that calls an imported function:

```rust
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "loft_io")]
extern "C" {
    fn loft_host_print(ptr: *const u8, len: usize);
}

#[cfg(target_arch = "wasm32")]
pub fn n_println(stores: &mut Stores, msg: &str) {
    loft_host_print(msg.as_ptr(), msg.len());
}
```

Approach B is cleaner — it's in the runtime library, no codegen changes.

**Test:** Same as step 4 but now "hello from wasm" actually appears in
the browser.

---

### Step 6: GL functions as WASM imports

**What:** Make graphics programs compile for the browser.  GL functions
must be imported from JavaScript, not linked from the native crate.

**Files:** `src/generation/mod.rs`, `src/codegen_runtime.rs`

**Current generated code:**
```rust
extern crate loft_graphics_native;
// ... calls like:
loft_graphics_native::loft_gl_clear(color);
```

**For wasm32-unknown-unknown, generate instead:**
```rust
#[link(wasm_import_module = "loft_gl")]
extern "C" {
    fn loft_gl_create_window(w: u32, h: u32, title_ptr: *const u8,
                             title_len: usize) -> i32;
    fn loft_gl_poll_events() -> i32;
    fn loft_gl_swap_buffers();
    fn loft_gl_clear(color: u32);
    fn loft_gl_create_shader(v_ptr: *const u8, v_len: usize,
                             f_ptr: *const u8, f_len: usize) -> u32;
    fn loft_gl_use_shader(program: u32);
    fn loft_gl_draw(vao: u32, n_vertices: u32);
    // ... all GL functions from lib/graphics/native/src/lib.rs
}
```

**Implementation:** In `output_native_preamble()`, when a `wasm_browser`
flag is set:
- Skip `extern crate loft_graphics_native;`
- Instead emit the `#[link(wasm_import_module)]` block with all GL
  function signatures
- The signatures come from `data.native_packages` / `data.native_symbols`
  or can be hardcoded for the graphics package

**JS side:** Restructure `doc/loft-gl.js` to export a
`buildLoftGLImports(canvas, memory)` function that returns the import
object.  The bridge needs a `memory` reference to read string arguments
(`*const u8, usize` pairs) from WASM linear memory.

Add a helper:
```javascript
function readString(memory, ptr, len) {
  return new TextDecoder().decode(
    new Uint8Array(memory.buffer, ptr, len)
  );
}
```

**Test:**
```bash
# Use an existing simple GL example
cargo run --bin loft -- --html /tmp/triangle.html \
    --lib lib/ lib/graphics/examples/02-hello-triangle.loft
# Open in browser — should show a colored triangle
```

---

### Step 7: Frame yield for game loops

**What:** Games call `gl_swap_buffers()` each frame, then continue the
game loop.  In the browser, execution must yield back to the JS event
loop for `requestAnimationFrame`.

**Current interpreter approach:** `gl_swap_buffers` sets
`state.database.frame_yield = true`, the interpreter loop exits,
JS calls `resume_frame()` on next rAF.

**Native code approach:** The generated game loop is a Rust `loop { }`.
It cannot yield mid-loop.  Two options:

**Option A — Split main into start/frame:**
The code generator detects the game loop pattern (`loop { ... gl_swap_buffers(); ... }`)
and splits it into `loft_start()` (init before loop) and `loft_frame()`
(one loop iteration).  Complex — requires loop analysis.

**Option B — Cooperative yield via import:**
`gl_swap_buffers` is a WASM import.  The JS implementation sets a flag.
After calling `gl_swap_buffers`, the generated code checks the flag and
returns from `loft_frame()`.  The game loop becomes:

```rust
// Generated game loop
loop {
    // ... game logic ...
    loft_gl_swap_buffers();
    if loft_gl_should_yield() { return 1; } // yield to JS
    if !loft_gl_poll_events() { break; }
}
```

Where `loft_gl_should_yield()` is a WASM import that always returns true
in the browser (every frame yields) and always returns false natively.

The `loft_frame()` export resumes the loop by re-entering after the
yield point.  This requires the game state (stores + local variables) to
persist between calls — they live in global statics.

**This is the hardest step.** It requires:
1. Global state persistence across calls
2. A resume mechanism (re-enter the loop body)
3. Codegen changes to insert yield checks after `gl_swap_buffers`

**Alternative simpler approach:** Use `Asyncify` or `wasm-opt --asyncify`
to automatically transform the WASM so that any imported function can
suspend and resume execution.  This is a well-tested tool (used by
Emscripten).  The JS side uses:

```javascript
const instance = await Asyncify.instantiate(wasmBytes, imports);
instance.exports.loft_start();  // runs until gl_swap_buffers suspends
// On each rAF, Asyncify automatically resumes
```

**Recommendation:** Try `--asyncify` first.  If the size overhead is
acceptable (~10-20%), it avoids all codegen complexity.

**Test:** Run 01-hello-window.loft (window + clear + poll events loop)
in the browser via `--html`.  The window should appear and respond to
close.

---

### Step 8: HTML assembly with GL bridge

**What:** Put it all together — the `--html` flag produces one file.

**Files:** `src/main.rs`

**Implementation:**

```rust
fn emit_html(wasm_path: &str, out_path: &str, title: &str) -> std::io::Result<()> {
    let wasm_bytes = std::fs::read(wasm_path)?;
    let wasm_b64 = base64::encode(&wasm_bytes);
    let gl_js = include_str!("../doc/loft-gl-wasm.js");  // restructured bridge
    let template = format!(r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{title}</title>
<style>body{{margin:0;background:#000;display:flex;justify-content:center;
align-items:center;height:100vh}}canvas{{display:block}}</style>
</head><body>
<canvas id="c" tabindex="0"></canvas>
<script>
{gl_js}

const wasmB64 = "{wasm_b64}";
const wasmBytes = Uint8Array.from(atob(wasmB64), c => c.charCodeAt(0));
const canvas = document.getElementById('c');
const mem = {{ buffer: null }};
const imports = buildLoftImports(canvas, mem);
WebAssembly.instantiate(wasmBytes, imports).then(({{instance}}) => {{
  mem.buffer = instance.exports.memory.buffer;
  if (instance.exports.loft_start()) {{
    (function frame() {{
      if (instance.exports.loft_frame()) requestAnimationFrame(frame);
    }})();
  }}
}});
</script></body></html>"#);
    std::fs::write(out_path, template)
}
```

**Test:**
```bash
cargo run --bin loft -- --html /tmp/brick-buster.html \
    --lib lib/ lib/graphics/examples/25-brick-buster.loft
ls -la /tmp/brick-buster.html  # check size
# Open in browser — Brick Buster should run
```

---

### Step 9: wasm-opt integration

**What:** Optionally run `wasm-opt -Oz` to shrink the binary.

**Files:** `src/main.rs`

**Change:** After rustc produces the `.wasm`, try to run:
```bash
wasm-opt -Oz --strip-debug --strip-producers -o optimized.wasm raw.wasm
```

If `wasm-opt` is not found, use the unoptimized binary and print:
```
note: install wasm-opt (binaryen) for smaller output
```

**Test:** Compare sizes with and without `wasm-opt`.

---

### Step 10: End-to-end test

**What:** Automated test that `--html` produces a valid file.

**Files:** `tests/exit_codes.rs`

**Test:**
```rust
#[test]
fn html_export_produces_file() {
    let dir = std::env::temp_dir();
    let src = dir.join("html_export_test.loft");
    let out = dir.join("html_export_test.html");
    std::fs::write(&src, "fn main() { println(\"ok\"); }").unwrap();
    let result = Command::new(loft_bin())
        .arg("--html").arg(&out).arg(&src)
        .current_dir(workspace_root())
        .output().unwrap();
    let _ = std::fs::remove_file(&src);
    assert!(result.status.success(), "expected --html to succeed");
    let html = std::fs::read_to_string(&out).unwrap();
    let _ = std::fs::remove_file(&out);
    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("loft_start"));
    // WASM binary is embedded as base64
    assert!(html.len() > 1000, "HTML too small: {} bytes", html.len());
}
```

---

## Open questions

1. **wasm32-unknown-unknown allocator:** `cdylib` for this target may need
   `#[global_allocator]`.  The default (`dlmalloc` via `std`) should work.
   Verify in step 2.

2. **String passing in WASM imports:** GL functions receive `(*const u8, usize)`.
   The JS bridge reads these from `instance.exports.memory.buffer`.
   Verify that Rust's string layout in WASM linear memory is contiguous.

3. **Asyncify vs manual yield:** Step 7 proposes `wasm-opt --asyncify`.
   Measure the size overhead.  If >30%, implement manual yield.

4. **Non-GL console programs:** Step 4 handles these with a `<pre>` element.
   May need `loft_host_print` import (step 5).

---

## See also

- [WASM.md](WASM.md) — Interpreter WASM architecture
- [OPENGL.md](OPENGL.md) — Graphics library design
- [GAME_INFRA.md](GAME_INFRA.md) § W1.1 — Original sketch
- [OPENGL_IMPL.md](OPENGL_IMPL.md) — Implementation checklist
