
# Library Package Format

Design for a unified packaging format that supports pure-loft libraries,
native Rust extensions, and WASM targets — with OpenGL as the driving
use case.

---

## Contents
- [Goals](#goals)
- [Current state](#current-state)
- [Package layout](#package-layout)
- [Manifest: `loft.toml`](#manifest-lofttoml)
- [Package dependencies](#package-dependencies)
- [Function binding model](#function-binding-model)
- [Package test suite](#package-test-suite)
- [Build pipeline](#build-pipeline)
- [Target matrix](#target-matrix)
- [OpenGL case study](#opengl-case-study)
- [Security model](#security-model)
- [Implementation phases](#implementation-phases)

---

## Goals

1. A single package can contain loft source, native Rust code, and
   pre-compiled WASM — consumers don't choose; the runtime picks the
   right variant for the target.
2. `use graphics;` in a loft program works identically whether running
   via the interpreter, `--native`, or `--native-wasm`.
3. OpenGL/WebGL bindings ship as a package, not as built-in stdlib.
4. Package authors write Rust once; the build system produces native
   and WASM artifacts from the same source.
5. No C ABI.  All native code is Rust linking against `libloft.rlib`.

---

## Current state

| Layer | Status |
|---|---|
| Pure-loft packages (`use lib;`) | **Shipped** — directory layout, version check, `lib/` search |
| `loft.toml` manifest | **Shipped** — entry, version, native stem fields |
| `#native "symbol"` annotation | **Parsed** — bytecode dispatch NOT connected |
| `extensions.rs` cdylib loader | **Designed** — feature-gated, not integrated |
| WASM virtual filesystem | **JS side done** — Rust stubs return early |
| Native codegen (`--native`) | **Working** — generates Rust, compiles with rustc |
| WASM codegen (`--native-wasm`) | **Working** — targets wasm32-wasip2 |

The gap: no package can currently ship native code that works across
interpreter, native, and WASM targets from a single source.

---

## Package layout

```
graphics/
├── loft.toml                 # manifest
├── src/
│   ├── graphics.loft         # public loft API (types, wrappers)
│   ├── draw.loft             # loft-implemented rasterizer
│   └── math.loft             # loft-implemented matrix ops
├── tests/
│   ├── draw.loft             # test_* functions for draw module
│   ├── math.loft             # test_* functions for math module
│   └── integration.loft      # cross-module integration tests
├── native/
│   ├── Cargo.toml            # Rust crate for native functions
│   ├── src/
│   │   └── lib.rs            # implements #[loft_fn] functions
│   └── build.rs              # optional build script
└── prebuilt/                 # optional: pre-compiled artifacts
    ├── x86_64-linux/
    │   └── libgraphics.rlib
    ├── aarch64-macos/
    │   └── libgraphics.rlib
    └── wasm32-wasip2/
        └── libgraphics.wasm
```

**Rules:**
- `src/` is mandatory — every package has at least one `.loft` file
- `native/` is optional — only if the package has Rust-implemented functions
- `prebuilt/` is optional — avoids requiring Rust toolchain on consumer machine
- The primary `.loft` file (`src/graphics.loft`) declares the public API
  including `#native` function signatures

---

## Manifest: `loft.toml`

```toml
[package]
name = "graphics"
version = "0.1.0"
loft = ">=0.9"

[library]
entry = "src/graphics.loft"

[dependencies]
# Other loft packages this package needs.
# Keys are package names; values are version requirements.
math = ">=0.1"              # from registry or ~/.loft/lib/
utils = { path = "../utils" }  # local path (development)

[native]
# Rust crate in native/ — compiled to rlib (interpreter/native) or
# wasm (WASM target) at install time or first use.
crate = "native"

# Functions implemented in Rust.  Keys are loft function names;
# values are Rust symbol paths.  The loft compiler verifies signatures
# match between the .loft declaration and the Rust implementation.
[native.functions]
save_png = "graphics_native::save_png"
load_font = "graphics_native::load_font"
glyph_metrics = "graphics_native::glyph_metrics"
gl_create_window = "graphics_native::gl::create_window"
gl_swap_buffers = "graphics_native::gl::swap_buffers"

[native.wasm]
# WASM-specific overrides: some functions have different implementations
# in WASM (WebGL instead of OpenGL, Canvas2D instead of pixel buffer).
gl_create_window = "graphics_native::webgl::create_canvas"
gl_swap_buffers = "graphics_native::webgl::flush_canvas"

[native.dependencies]
# Additional crate dependencies the native code needs.
# These are added to the native/Cargo.toml [dependencies] section.
glutin = "0.32"
fontdue = "0.9"
png = "0.17"
```

---

## Package dependencies

### Declaring dependencies

A package declares its dependencies in `loft.toml`:

```toml
[dependencies]
math = ">=0.2"                    # version requirement
utils = { path = "../utils" }     # local path (for development)
json = { version = ">=1.0" }      # explicit version field
```

In loft source, the dependency is imported with `use`:

```loft
// src/graphics.loft
use math;       // imports math package — types, functions available
use utils;      // imports utils package

pub fn transform(canvas: Canvas, mat: math.Mat4) -> Canvas {
  // math.Mat4 is a type from the math package
  // ...
}
```

### Resolution order

When the compiler encounters `use math;` it searches:

1. **Local `src/`** — sibling files in the same package
2. **`[dependencies]` paths** — `path = "..."` entries from `loft.toml`
3. **Package lib directories** — `~/.loft/lib/math/`, project `lib/math/`
4. **`--lib` CLI flag** — explicit search directories
5. **`LOFT_LIB` environment variable**

The first match wins.  If the dependency has its own `loft.toml`, its
version is checked against the requirement.

### Transitive dependencies

If `graphics` depends on `math`, and `math` depends on `utils`, then
building `graphics` also loads `utils`.  The compiler resolves
transitively:

```
graphics/loft.toml  →  [dependencies] math = ">=0.2"
math/loft.toml      →  [dependencies] utils = ">=0.1"
```

Resolution:
1. Parse `graphics/src/graphics.loft` → encounters `use math;`
2. Find `math/` package → read `math/loft.toml` → version check
3. Parse `math/src/math.loft` → encounters `use utils;`
4. Find `utils/` package → read `utils/loft.toml` → version check
5. Parse `utils/src/utils.loft`
6. All types and functions from `utils` and `math` are now available

### Diamond dependencies

When two packages depend on the same package:

```
graphics → math >=0.2
graphics → physics → math >=0.1
```

Loft loads `math` **once** at the highest compatible version.  Since
`>=0.2` satisfies `>=0.1`, version `0.2` is used.

If requirements conflict (e.g., `math =0.2` vs `math =0.3`), the
compiler emits:

```
Error: conflicting dependency versions for 'math':
  graphics requires =0.2
  physics requires =0.3
```

### Version syntax

| Pattern | Meaning |
|---|---|
| `">=0.2"` | Any version 0.2.0 or higher |
| `">=0.2.1"` | Any version 0.2.1 or higher |
| `"=0.2.0"` | Exactly 0.2.0 |
| `{ path = "../math" }` | Local directory (no version check) |
| `{ version = ">=1.0" }` | Same as string form, explicit syntax |

No caret (`^`) or tilde (`~`) ranges — only `>=` and `=`.  This keeps
the resolver simple and predictable.

### Cycle detection

Circular dependencies are rejected:

```
Error: circular dependency: graphics → math → graphics
```

The resolver tracks the dependency chain and panics on cycles before
any source is parsed.

### Native dependency propagation

When package A depends on package B, and both have `[native]` sections,
the build system must link both rlibs:

```
graphics/native/ depends on math/native/  (Rust crate dependency)
```

This is expressed in `graphics/native/Cargo.toml`:

```toml
[dependencies]
math_native = { path = "../../math/native" }
```

The loft build system passes both `--extern` flags to rustc:
```bash
rustc --extern math_native=.../libmath_native.rlib \
      --extern graphics_native=.../libgraphics_native.rlib \
      generated_program.rs
```

### Lock file

After resolving all dependencies, `loft install` writes `loft.lock`:

```toml
# Auto-generated — do not edit
[[package]]
name = "math"
version = "0.2.3"
source = "~/.loft/lib/math"

[[package]]
name = "utils"
version = "0.1.0"
source = "~/.loft/lib/utils"
```

Subsequent builds use `loft.lock` for reproducibility.  `loft update`
re-resolves and rewrites the lock file.

---

## Function binding model

### Declaration in `.loft`

```loft
// src/graphics.loft

pub struct Canvas {
  width: integer not null,
  height: integer not null,
  data: vector<integer>     // RGBA pixel buffer
}

// Pure loft: implemented in draw.loft
pub fn clear(self: Canvas, color: integer) {
  for px_i in 0..self.width * self.height {
    self.data[px_i] = color;
  }
}

// Native: implemented in Rust, declared with #native
pub fn save_png(self: const Canvas, path: text);
#native "save_png"

pub fn load_font(path: text) -> integer;
#native "load_font"
```

### Implementation in Rust

```rust
// native/src/lib.rs

use loft::codegen_runtime::{Stores, DbRef};

/// Save the Canvas pixel buffer as a PNG file.
///
/// Signature must match the loft declaration:
///   fn save_png(self: const Canvas, path: text)
///
/// Arguments are passed as store references; the runtime marshals
/// loft types to Rust types via the Stores API.
#[loft_fn]
pub fn save_png(stores: &Stores, canvas: &DbRef, path: &str) {
    let width = stores.get_int(canvas, "width");
    let height = stores.get_int(canvas, "height");
    let data_ref = stores.get_field(canvas, "data");
    // ... encode PNG using the `png` crate
}

#[loft_fn]
pub fn load_font(stores: &mut Stores, path: &str) -> i32 {
    // ... load font via fontdue, return font ID
}
```

### Three execution paths

| Path | How native functions run |
|---|---|
| **Interpreter** | `#native` triggers `extensions::load_one()` → dlopen rlib → call registered function pointer |
| **`--native`** | Generated Rust calls the function directly (linked at compile time via `--extern graphics_native=...`) |
| **`--native-wasm`** | Generated Rust calls the WASM variant (linked from `prebuilt/wasm32-wasip2/` or compiled in-situ) |

### Signature verification

At `byte_code()` time, the compiler checks:
1. Every `#native "symbol"` function in `.loft` has a matching entry in
   `loft.toml [native.functions]`
2. The Rust function's parameter count matches the loft declaration
3. Return type is compatible (integer↔i32, text↔&str, reference↔&DbRef)

Mismatch → compile-time error with clear message:
```
Error: native function 'save_png' expects 2 parameters (Canvas, text)
       but Rust symbol 'graphics_native::save_png' has 3 parameters
```

---

## Package test suite

### Directory convention

Tests live in `tests/` alongside `src/`.  Each `.loft` file in `tests/`
is a test module.  Functions named `fn test_*()` with zero parameters
are discovered and run — same convention as `loft --tests` on the main
project.

```
graphics/
├── src/
│   ├── graphics.loft
│   ├── draw.loft
│   └── math.loft
├── tests/
│   ├── draw.loft           # tests for draw module
│   ├── math.loft           # tests for math module
│   ├── integration.loft    # cross-module tests
│   └── fixtures/           # test data (PNG files, configs)
│       └── reference.png
```

### Running tests

```bash
# Run all tests in a package
loft --tests graphics/tests

# Run a single test file
loft --tests graphics/tests/draw.loft

# Run a single test function
loft --tests graphics/tests/draw.loft::test_clear_canvas

# Run tests via all backends
loft --tests graphics/tests                      # interpreter
loft --tests --native graphics/tests             # native
loft --tests --native-wasm graphics/tests        # WASM (if wasmtime available)
```

The `--tests` runner:
1. Adds `src/` to the import search path so `use graphics;` works
2. Loads the default library and the package's `.loft` files
3. Discovers all `fn test_*()` functions in each test file
4. Runs each in isolation with a fresh State (same as `loft --tests` today)
5. Reports per-function pass/fail with `@EXPECT_FAIL`/`@EXPECT_ERROR` support

### Manifest test configuration

```toml
[test]
# Extra directories added to lib search path during tests.
# Useful when the package depends on other local packages.
lib = ["../other-package/src"]

# Files or patterns to skip (broken tests, platform-specific).
skip = ["tests/webgl.loft"]

# Timeout per test function (seconds). Default: 30.
timeout = 10
```

### Test file structure

Test files import the package and use `assert`:

```loft
// tests/draw.loft
use graphics;

fn test_clear_canvas() {
  canvas = Canvas { width: 10, height: 10 };
  canvas.clear(0xFF000000);
  assert(canvas.data[0] == 0xFF000000, "clear sets all pixels");
  assert(canvas.data[99] == 0xFF000000, "clear sets last pixel");
}

fn test_draw_rect() {
  canvas = Canvas { width: 100, height: 100 };
  canvas.clear(0xFF000000);
  draw_rect(canvas, 10, 10, 20, 20, 0xFFFF0000);
  assert(canvas.data[10 * 100 + 10] == 0xFFFF0000, "top-left corner");
  assert(canvas.data[5 * 100 + 5] == 0xFF000000, "outside rect unchanged");
}
```

### Annotations

Same annotation system as the main project's `tests/scripts/`:

| Annotation | Effect |
|---|---|
| `// @EXPECT_FAIL: <text>` | Tolerate runtime panic matching `<text>` |
| `// @EXPECT_ERROR: <text>` | Expect compile-time error matching `<text>` |
| `// @EXPECT_WARNING: <text>` | Expect compiler warning matching `<text>` |
| `// @IGNORE` | Skip this function or file |
| `// @ARGS: --production` | Run with extra CLI flags |

### Fixtures and test data

Test data lives in `tests/fixtures/`.  The test runner sets the working
directory to the package root so relative paths work directly.

```
tests/
├── draw.loft
├── math.loft
└── fixtures/
    ├── reference.png         # binary: expected render output
    ├── terrain.txt           # text: map configuration
    ├── vertices.bin          # binary: pre-computed mesh data
    └── expected/
        ├── clear_black.txt   # text: expected pixel dump
        └── rect_red.txt      # text: expected pixel dump
```

#### Text fixtures

Read with `file(...).lines()` or `file(...).content()`:

```loft
fn test_terrain_loading() {
  lines = file("tests/fixtures/terrain.txt").lines();
  assert(lines.len() > 0, "terrain file has content");
  assert(lines[0] == "width=100", "first line is width");
}

fn test_pixel_dump_matches() {
  canvas = Canvas { width: 4, height: 4 };
  canvas.clear(0xFF000000);
  expected = file("tests/fixtures/expected/clear_black.txt").content();
  actual = canvas_to_text(canvas);
  assert(actual == expected, "pixel dump matches reference");
}
```

#### Binary fixtures

Read with `f#format = LittleEndian` and `f#read(n) as T`:

```loft
fn test_load_binary_mesh() {
  f = file("tests/fixtures/vertices.bin");
  f#format = LittleEndian;
  count = f#read(4) as i32;
  assert(count == 36, "expected 36 vertices");
  for vtx_i in 0..count {
    vx = f#read(4) as single;
    vy = f#read(4) as single;
    vz = f#read(4) as single;
    // verify data is in expected range
    assert(vx >= -1.0f, "x in range");
    assert(vx <= 1.0f, "x in range");
  }
}
```

#### Generating reference data

Test functions can write reference data on first run, then compare
on subsequent runs.  Use a helper pattern:

```loft
fn update_or_compare(path: text, actual: text) {
  ref = file(path);
  if ref#format == NotExists {
    // First run: write the reference
    {uc_f = file(path); uc_f += actual; uc_f += "\n";}
  } else {
    expected = ref.content();
    assert(actual + "\n" == expected, "output differs from {path}");
  }
}

fn test_render_output() {
  canvas = Canvas { width: 10, height: 10 };
  draw_rect(canvas, 2, 2, 6, 6, 0xFFFF0000);
  dump = canvas_to_hex(canvas);
  update_or_compare("tests/fixtures/expected/small_rect.txt", dump);
}
```

On first run, `small_rect.txt` is created.  On subsequent runs, the
output is compared against it.  To update references after an intentional
change, delete the fixture files and re-run.

#### Binary output comparison

For binary formats (PNG), compare file size and a sample of bytes
rather than exact equality (compression may vary):

```loft
fn test_png_round_trip() {
  canvas = Canvas { width: 8, height: 8 };
  canvas.clear(0xFFFF0000);
  save_png(canvas, "tests/fixtures/actual.png");

  actual = file("tests/fixtures/actual.png");
  assert(actual#format != NotExists, "PNG written");
  assert(actual#size > 50l, "PNG has header + data");
  assert(actual#size < 500l, "PNG is reasonable size for 8x8");

  // Read back and verify pixels
  loaded = Image { };
  loaded.png("tests/fixtures/actual.png");
  assert(loaded.width == 8, "width preserved");
  assert(loaded.height == 8, "height preserved");
  assert(loaded.data[0].r == 255, "red channel preserved");

  delete("tests/fixtures/actual.png");
}
```

#### Fixture data in `loft.toml`

Declare fixture directories so `loft install` includes them and
`loft test` knows where to find them:

```toml
[test]
fixtures = "tests/fixtures"
```

When `loft install` copies a package, `tests/` and `tests/fixtures/`
are included so consumers can run the package's test suite to verify
their installation works.
```

### `loft test` shorthand

Inside a package directory, `loft test` is equivalent to
`loft --tests tests/` with the package's `src/` on the lib path:

```bash
cd graphics/
loft test              # runs all tests
loft test draw         # runs tests/draw.loft
loft test draw::test_clear_canvas  # single function
```

This is implemented as:
1. Detect `loft.toml` in the current directory
2. Read `[library] entry` to find `src/`
3. Add `src/` to `--lib` search path
4. Invoke `--tests tests/` (or the specified target)

### CI integration

Package authors add a test step to their CI:

```yaml
- name: Test loft package
  run: |
    loft test
    loft test --native       # optional: also test native backend
```

The `loft test` command exits 0 on all-pass, 1 on any failure — standard
CI-compatible exit codes.  `@EXPECT_FAIL` tests count as passes (the
failure is expected).  Only unexpected failures cause exit 1.

### Test discovery rules

| Pattern | Discovered? | Notes |
|---|---|---|
| `fn test_foo()` | Yes | Zero-param, name starts with `test_` or is `main` |
| `fn helper(x: integer)` | No | Has parameters |
| `fn main()` | Yes | Always an entry point |
| `fn count() -> iterator<integer>` | No | Generator (returns iterator) |
| `fn _internal()` | No | Starts with `_` (private helper) |

Functions starting with `_` are skipped by convention — they're private
helpers called by test functions but not entry points themselves.

---

## Build pipeline

### Consumer's view

```bash
# Install a package (downloads or builds native code)
loft install graphics

# Use it — works on all targets
loft my_program.loft           # interpreter: dlopen libgraphics
loft --native my_program.loft  # native: link graphics_native.rlib
loft --native-wasm out.wasm my_program.loft  # wasm: link wasm variant
```

### What `loft install` does

```
1. Locate package (local path, or future: registry)
2. Read loft.toml
3. If [native] section exists:
   a. Check prebuilt/ for current target
   b. If missing or stale: cargo build native/ for current target
   c. Copy rlib to ~/.loft/lib/<package>/<target>/
4. Copy src/*.loft to ~/.loft/lib/<package>/src/
5. Register in ~/.loft/lib/<package>/loft.toml
```

### What `loft my_program.loft` does (enhanced)

```
1. parse_dir("default/")
2. For each `use <pkg>`:
   a. Find <pkg>/loft.toml in lib search path
   b. Parse src/<entry>.loft
   c. If [native] exists:
      - Interpreter: queue rlib for dlopen after byte_code()
      - Native: add --extern <pkg>_native=<rlib> to rustc
      - WASM: add --extern <pkg>_native=<wasm_rlib> to rustc
3. byte_code() — connects #native symbols to loaded functions
4. execute()
```

---

## Target matrix

| Feature | Interpreter | `--native` | `--native-wasm` |
|---|---|---|---|
| Pure loft code | ✓ bytecode | ✓ compiled Rust | ✓ compiled WASM |
| `#rust` inline | ✓ fill.rs dispatch | ✓ emitted inline | ✓ emitted inline |
| `#native` external | ✓ dlopen rlib | ✓ linked rlib | ✓ linked wasm rlib |
| File I/O | ✓ OS calls | ✓ OS calls | ✓ VirtFS bridge |
| OpenGL | ✓ glutin/gl | ✓ glutin/gl | ✗ WebGL (different API) |
| Threading | ✓ rayon | ✓ rayon | ✗ sequential |

---

## OpenGL case study

### Why OpenGL drives the package design

OpenGL is the first real-world use case that requires:
- **Native code** (GL context creation, shader compilation, buffer management)
- **Platform-specific variants** (OpenGL on desktop, WebGL in browser)
- **Large loft-side logic** (rasterizer, matrix math, scene graph)
- **Binary dependencies** (glutin, fontdue, png crate)

If the package format handles OpenGL cleanly, it handles everything.

### Package structure

```
graphics/
├── loft.toml
├── src/
│   ├── graphics.loft       # re-exports: pub use draw; pub use text;
│   ├── draw.loft            # Canvas, Rgba, Draw — pure loft rasterizer
│   ├── primitives.loft      # rect, ellipse, line, bezier — pure loft
│   ├── text.loft            # Font, TextStyle, draw_text — pure loft
│   ├── math.loft            # Mat4, Vec3, matrix ops — pure loft
│   ├── mesh.loft            # Vertex, Triangle, Mesh — pure loft
│   ├── scene.loft           # Transform, Camera, Light — pure loft
│   └── gl.loft              # OpenGL/WebGL API — #native bindings
├── native/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs           # re-exports
│       ├── png_io.rs        # save_png, load_png
│       ├── font.rs          # load_font, glyph_metrics, rasterize_glyph
│       ├── gl.rs            # create_window, swap_buffers, create_shader, ...
│       └── webgl.rs         # WASM variants of gl.rs functions
```

### `gl.loft` — the binding layer

```loft
// Types
pub struct Window { id: integer not null }
pub struct Shader { id: integer not null }
pub struct Buffer { id: integer not null }

// Window management
pub fn create_window(title: text, width: integer, height: integer) -> Window;
#native "gl_create_window"

pub fn swap_buffers(self: Window);
#native "gl_swap_buffers"

pub fn should_close(self: Window) -> boolean;
#native "gl_should_close"

pub fn poll_events(self: Window);
#native "gl_poll_events"

// Shader operations
pub fn create_shader(vertex_src: text, fragment_src: text) -> Shader;
#native "gl_create_shader"

pub fn use_shader(self: Shader);
#native "gl_use_shader"

// Buffer operations
pub fn create_buffer(data: vector<single>) -> Buffer;
#native "gl_create_buffer"

pub fn draw_triangles(self: Buffer, count: integer);
#native "gl_draw_triangles"
```

### User program

```loft
use graphics;

fn main() {
  // 2D software rendering — works everywhere (pure loft)
  canvas = Canvas { width: 800, height: 600 };
  canvas.clear(0xFF000000);     // black
  draw_rect(canvas, 100, 100, 200, 150, 0xFFFF0000);  // red rectangle
  save_png(canvas, "output.png");

  // 3D hardware rendering — requires native GL package
  win = create_window("My App", 800, 600);
  shader = create_shader(VERTEX_SRC, FRAGMENT_SRC);
  buf = create_buffer([0.0f, 0.5f, 0.0f, -0.5f, -0.5f, 0.0f, 0.5f, -0.5f, 0.0f]);
  while !win.should_close() {
    win.poll_events();
    shader.use_shader();
    buf.draw_triangles(3);
    win.swap_buffers();
  }
}
```

### WASM variant

On `--native-wasm`, `loft.toml [native.wasm]` overrides:
- `gl_create_window` → `webgl::create_canvas` (creates `<canvas>` element)
- `gl_swap_buffers` → `webgl::flush_canvas` (requestAnimationFrame)
- `gl_create_shader` → `webgl::create_shader` (WebGL2 shader API)

The loft code is identical.  Only the native implementation changes.

### What stays in pure loft

| Component | Why loft, not Rust |
|---|---|
| 2D rasterizer (scanline fill, Bezier) | Performance contract — proves the interpreter is fast enough |
| Matrix math (Mat4, Vec3 ops) | Simple arithmetic — no benefit from native |
| Scene graph (transforms, camera) | Pure data manipulation |
| GLB binary writer | Byte-level file I/O — loft's `File` API handles it |
| Mesh generation | Vertex computation — pure math |

### What must be native

| Component | Why Rust, not loft |
|---|---|
| PNG encode/decode | Depends on `png` crate (zlib compression) |
| Font rasterization | Depends on `fontdue` crate (TrueType parsing) |
| GL context + window | Depends on `glutin`/`winit` (OS window management) |
| GL API calls | OpenGL is a C API; Rust FFI is the natural bridge |
| WebGL API calls | Browser DOM access via `web-sys` in WASM |

---

## Security model

### Interpreter mode

Native packages load shared libraries via `dlopen`.  A loaded library has
full process access — it can read files, open sockets, allocate memory.

**Mitigation:**
- `--no-native` flag: refuse to load any `#native` functions.  The program
  runs only pure-loft code; native calls produce a runtime error.
- Package signatures (Phase 3): SHA-256 hash in a lock file; refuse to
  load if the hash doesn't match.
- Origin tracking: `loft.toml` records the source URL; the runtime warns
  when loading a native package from an unknown origin.

### WASM mode

WASM is sandboxed by the runtime (wasmtime, browser).  Native functions
compiled to WASM can only access capabilities granted by the host:
- File I/O: only through the VirtFS bridge
- Network: only if the host provides a WASI socket capability
- GPU: only through WebGL (browser) or headless EGL (wasmtime)

No additional sandboxing needed — WASM's capability model is sufficient.

### Native mode (`--native`)

The generated Rust binary links the native package's rlib statically.
The binary has full OS access.  Same security as any compiled program.
No sandboxing — the user chose to compile and run native code.

---

## Implementation phases

| Phase | Scope | Effort | Depends on |
|---|---|---|---|
| **P1** | Connect `#native` to interpreter dispatch | Medium | `extensions.rs` completion |
| **P2** | `loft install` for local packages | Medium | P1 |
| **P3** | Native codegen `--extern` for `#native` packages | Medium | P1 |
| **P4** | WASM codegen with native package wasm rlib | Medium | P3 |
| **P5** | OpenGL package: 2D canvas + PNG | Medium | P1 |
| **P6** | OpenGL package: font rendering | Small | P5 |
| **P7** | OpenGL package: GL window + shader | High | P5 + glutin |
| **P8** | WebGL variant + WASM integration | High | P4 + P7 |

P1 is the foundation — without interpreter dispatch of `#native` symbols,
nothing else works.  P5-P6 can proceed in parallel with P3-P4 since the
2D canvas and font rendering don't need GL.

---

## See also
- [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) — Current Phase 1 library loading
- [OPENGL.md](OPENGL.md) — OpenGL rendering design
- [OPENGL_IMPL.md](OPENGL_IMPL.md) — Step-by-step OpenGL implementation
- [WASM.md](WASM.md) — WASM architecture overview
- [WASM_FS_STEPS.md](WASM_FS_STEPS.md) — Virtual filesystem bridge steps
