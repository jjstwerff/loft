
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
- the External Libraries section below — Current Phase 1 library loading
- [OPENGL.md](OPENGL.md) — OpenGL rendering design
- [OPENGL_IMPL.md](OPENGL_IMPL.md) — Step-by-step OpenGL implementation
- [WASM.md](WASM.md) — WASM architecture overview
- [WASM.md](WASM.md) — Virtual filesystem bridge steps

---


# Package Registry

Design for a file-based package registry that maps library names and versions to
download URLs.  This is Phase 3 of the external library support described in
the External Libraries section below.

---

## Contents
- [Goals](#goals)
- [Registry File Format](#registry-file-format)
- [Registry File Locations](#registry-file-locations)
- [CLI Interface](#cli-interface)
- [Install Flow](#install-flow)
- [Registry Sync](#registry-sync)
- [Installed Package Check](#installed-package-check)
- [Version Resolution](#version-resolution)
- [Zip Package Layout](#zip-package-layout)
- [Security Considerations](#security-considerations)
- [Implementation](#implementation)
- [Phased Rollout](#phased-rollout)
- [Code Touchpoints](#code-touchpoints)

---

## Goals

1. A developer can run `loft install graphics` and get the library without
   manually downloading or placing files.
2. Different versions of the same library each have their own URL — there is
   no "latest pointer" file that needs to be updated server-side.
3. The registry is a plain text file — it can be hosted on any static file
   server, checked into a git repository, or maintained by hand.
4. The format is human-readable and editable without tooling.
5. No central authority is required.  Users can point to any registry file.

---

## Registry File Format

A registry file is a UTF-8 text file.  Each non-blank, non-comment line
declares one package version:

```
# source: https://raw.githubusercontent.com/jjstwerff/loft-registry/main/registry.txt
# Loft package registry
# Format: <name> <version> <url> [status]
#
# Lines starting with # are comments.  Blank lines are ignored.
# Entries are matched top-to-bottom; the first match wins
# when searching for an exact version.  For "latest", all
# active entries are compared by semver and the highest wins.

graphics 0.1.0 https://example.com/packages/graphics-0.1.0.zip yanked:CVE-2026-001
graphics 0.2.0 https://example.com/packages/graphics-0.2.0.zip
opengl   0.1.0 https://example.com/packages/opengl-0.1.0.zip   deprecated:use-graphics
math     1.0.0 https://example.com/packages/math-1.0.0.zip
math     1.1.0 https://example.com/packages/math-1.1.0.zip
```

### Fields

| Field | Description |
|-------|-------------|
| `name` | Package identifier — must match `[a-z][a-z0-9_]*` |
| `version` | Semver string `MAJOR.MINOR.PATCH` |
| `url` | HTTPS URL to a `.zip` file containing the package |
| `status` | Optional governance field — see below |

### Status field

| Value | Meaning |
|-------|---------|
| *(absent)* | Active — installable without warning |
| `deprecated:<slug>` | Installable but warns; skipped for "latest" if any active version exists |
| `yanked:<slug>` | Not installable; always skipped for "latest"; existing installs unaffected |

The `<slug>` is a short human-readable reason (e.g. `CVE-2026-001`, `outdated`,
`malicious`).  It appears verbatim in diagnostics.

### The `source:` directive

The first `# source: <url>` comment line in the file records where the file
itself was downloaded from.  `loft registry sync` reads this URL to know where
to fetch updates.

```
# source: https://raw.githubusercontent.com/jjstwerff/loft-registry/main/registry.txt
```

The URL points to the personal repository initially.  If the registry migrates
to a GitHub organisation (e.g. `loft-lang/registry`), the `source:` line in
the file is updated and users get the new URL automatically on their next sync —
no interpreter release is needed.

Rules:
- The `source:` line must be the first non-blank line of the file.
- Only one `source:` line is recognised; subsequent ones are plain comments.
- If absent, `loft registry sync` falls back to the `LOFT_REGISTRY_URL`
  environment variable, then the compiled-in default URL.
- The `source:` line is preserved verbatim when `sync` rewrites the file.
- Teams hosting a private registry change only this one line — all other
  registry mechanics (sync, check, install) work identically.

### Constraints

- Fields are separated by one or more ASCII spaces or tabs.
- Trailing whitespace on a line is ignored.
- The URL must start with `https://` or `http://`.
- A name may appear multiple times with different versions.
- Duplicate `(name, version)` pairs: first entry wins (top-to-bottom).
- Yanked entries are never removed — they stay in the file as a permanent
  auditable record with their `yanked:` status.

---

## Registry File Locations

The interpreter searches for a registry file in this order:

1. **`LOFT_REGISTRY` environment variable** — must be an absolute path to a
   local file.  Set this to use a team-internal or project-specific registry.
2. **`~/.loft/registry.txt`** — the user's personal registry, installed by
   the user or by a future `loft registry fetch` command.

If no registry file is found and the user runs `loft install <name>` (not a
local path), the command exits with a clear diagnostic:

```
loft install: no registry file found.
  Create ~/.loft/registry.txt or set LOFT_REGISTRY to a registry file path.
```

### Multiple Registries (future)

A future `loft registry` subcommand could merge multiple sources.  For Phase 3
a single file is sufficient.

---

## CLI Interface

### Installing from registry

```sh
loft install graphics            # install latest version from registry
loft install graphics@0.1.0      # install specific version
```

### Installing from local path (unchanged, Phase 1)

```sh
loft install .                   # install package in current directory
loft install /path/to/mypkg      # install from absolute path
loft install ../sibling          # install from relative path
```

The heuristic for distinguishing registry lookups from local paths:
- Argument starts with `/`, `./`, or `../` → local path.
- Argument contains a path separator (`/`) → local path.
- Otherwise → registry lookup, with optional `@version` suffix.

### Registry subcommands

```sh
loft registry sync              # download latest registry.txt from source URL
loft registry check             # compare installed packages against registry
loft registry list              # show all packages in registry
loft registry list --installed  # show only installed packages
```

### Updated help text

```
  install [target]              install a package to ~/.loft/lib/ for global use
                                install .        — install package in current dir
                                install /p       — install package at /p
                                install name     — download latest from registry
                                install name@v   — download specific version

  registry <subcommand>         manage the local package registry
                                sync             — pull latest registry from source URL
                                check            — report updates, deprecations, yanks
                                list             — browse all packages in registry
                                list --installed — show only installed packages
```

---

## Install Flow

For a registry install (`loft install graphics`):

```
1. Parse "graphics" → name="graphics", version=None
2. Find registry file (LOFT_REGISTRY or ~/.loft/registry.txt)
3. Read and parse registry file
4. find_package(entries, "graphics", None) → pick highest semver entry
5. Download zip from entry.url to a temporary file
6. Extract zip to a temporary directory
7. Locate the package root inside the extracted tree
   (directory containing loft.toml, or the root itself)
8. Call install_package(pkg_root) — existing Phase 1 logic
9. Clean up temporary directory
10. Print: "installed graphics 0.2.0 → ~/.loft/lib/graphics/"
```

For a versioned install (`loft install graphics@0.1.0`):

Steps 1–10 same, except step 4 uses `find_package(entries, "graphics", Some("0.1.0"))`
and step 6 is a hard error if the version is not found.

---

## Registry Sync

`loft registry sync` downloads the authoritative registry file from GitHub (or
a custom source URL) and replaces the local `~/.loft/registry.txt`.

### Sync flow

```
1. Determine source URL:
   a. Read LOFT_REGISTRY_URL env var — if set, use it.
   b. Read local ~/.loft/registry.txt for a "# source: <url>" first line.
   c. Fall back to compiled-in default:
      https://raw.githubusercontent.com/jjstwerff/loft-registry/main/registry.txt

2. Download the URL via HTTPS to a temporary file.

3. Validate the downloaded content:
   - Must be valid UTF-8.
   - Must contain at least one non-comment, non-blank line.
   - Basic format check: each data line must have three whitespace-separated fields.

4. If the download succeeds:
   - Replace ~/.loft/registry.txt with the downloaded content.
   - Print: "registry synced: 14 packages, 28 versions  (2026-04-04)"

5. If the download fails:
   - Leave the existing ~/.loft/registry.txt unchanged.
   - Print error to stderr and exit 1:
     "loft registry sync: download failed: <reason>"
     "  local registry is unchanged."
```

### First-time sync (no local registry)

If `~/.loft/registry.txt` does not exist, `loft registry sync` downloads from
`LOFT_REGISTRY_URL` or the compiled-in default (which tracks wherever the
official registry lives — personal repo or org) and creates the file.  A user
running `loft install` for the first time is directed to run sync first:

```
loft install: no registry file found.
  Run 'loft registry sync' to download the package registry.
  Or set LOFT_REGISTRY to a local registry file path.
```

### Staleness tracking

The file modification time of `~/.loft/registry.txt` is used as the sync
timestamp.  No separate metadata file is needed.

If the local registry is older than **7 days** when `loft registry check` is
run, a warning is printed before the check results:

```
warning: registry was last synced 9 days ago.
  Run 'loft registry sync' to get the latest security information.
```

This warning does not affect the exit code.

### Custom and private registries

Teams can host their own registry file anywhere and point to it:

```sh
export LOFT_REGISTRY=/path/to/company-registry.txt
loft registry sync   # syncs from the source: URL inside that file
```

Or permanently by placing a registry file with a custom `# source:` URL at
`~/.loft/registry.txt`.  The official registry and a custom registry can be
used together only if they are manually merged — a single local file is the
intended model.

---

## Installed Package Check

`loft registry check` scans `~/.loft/lib/` for installed packages, reads each
`loft.toml` for the installed name and version, and compares against the local
registry file.

### Check flow

```
1. Scan ~/.loft/lib/*/loft.toml — collect (name, version) for each installed pkg.
2. Read local registry file.
3. Warn if registry is older than 7 days (does not affect exit code).
4. For each installed package, classify:
   - yanked   — installed version has yanked:<slug> in registry
   - deprecated — installed version has deprecated:<slug> in registry
   - outdated — installed version is active but a higher active version exists
   - current  — installed version is the highest active version
   - unknown  — name not found in registry at all
5. Collect count of registry packages not installed (new packages available).
6. Print report (see below).
7. Exit 0 if no installed packages are yanked; exit 1 if any are yanked.
```

### Output format

```
$ loft registry check
registry: 14 packages, 28 versions  (synced 2 days ago)

installed packages (4):
  graphics  0.1.0  YANKED      CVE-2026-001 — run: loft install graphics
  opengl    0.1.0  deprecated  use-graphics — run: loft install opengl
  math      1.0.0  outdated    → 1.1.0      — run: loft install math
  utils     0.3.0  current

new packages in registry not installed: 10
  run 'loft registry list' to browse

1 security issue — yanked packages must be updated.
```

When all packages are current:

```
$ loft registry check
registry: 14 packages, 28 versions  (synced 2 days ago)

installed packages (4):
  graphics  0.2.0  current
  math      1.1.0  current
  utils     0.3.0  current
  geo       0.5.0  current

all installed packages are up to date.
```

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | No yanked packages installed (updates/deprecations may exist — informational only) |
| 1 | At least one installed package is yanked — action required |

Exit code 1 is intentionally reserved for security-level issues so that CI
pipelines can use `loft registry check` as a gate without triggering on every
available update.

### `loft registry list`

Lists all packages in the registry with their available versions and installed
status:

```
$ loft registry list
name       versions                    installed   status
---------  --------------------------  ----------  --------
geo        0.4.0  0.5.0               0.5.0
graphics   0.1.0  0.2.0               0.1.0       YANKED (0.1.0)
math       1.0.0  1.1.0               1.1.0
opengl     0.1.0                      0.1.0       deprecated
utils      0.3.0  0.4.0  0.5.0        0.3.0       outdated
web        0.1.0                      —
```

`loft registry list --installed` shows only rows where installed is not `—`.

---

## Version Resolution

### Latest version

When no version is specified, all entries whose `name` matches are collected
and compared using semver ordering.  The entry with the highest version is
selected.

Semver comparison: `(major, minor, patch)` tuples compared lexicographically.
This reuses the `version_ge` logic already in `src/manifest.rs`.

### Exact version match

When a version is given (`@0.1.0`), the registry is searched top-to-bottom
for the first entry with matching `(name, version)`.  If not found, the
install fails with:

```
loft install: package 'graphics@0.1.0' not found in registry.
  Available versions: 0.2.0
```

### Already installed

Before downloading, the installer checks `~/.loft/lib/<name>/loft.toml`.
If the installed version matches the selected registry entry, it prints:

```
loft install: graphics 0.2.0 is already installed.
```

and exits without downloading.  Use `--force` to reinstall anyway (future).

---

## Zip Package Layout

The downloaded `.zip` file must contain the package as a directory:

```
graphics-0.2.0/          ← top-level directory (name optional)
  loft.toml
  src/
    graphics.loft
    math.loft
  tests/
    canvas.loft
```

The installer finds the package root by searching for `loft.toml` inside the
extracted tree (depth-first, stopping at the first match).  This tolerates
both flat layout (`loft.toml` at zip root) and the conventional
`name-version/loft.toml` layout produced by GitHub release archives.

If no `loft.toml` is found but a `src/` directory is present at the zip root,
the zip root is treated as the package root (permissive fallback for pure-loft
packages that skip the manifest).

If neither condition is met, the install fails:

```
loft install: could not find package root in downloaded zip.
  Expected loft.toml or src/ directory inside the archive.
```

---

## Security Considerations

### HTTPS only

The installer enforces that URLs start with `https://`.  Plain `http://` URLs
are rejected with a warning unless overridden by a future `--allow-http` flag.

### No signature verification (Phase 3)

Phase 3 does not verify package signatures or checksums.  A future
`loft.toml` field `sha256 = "..."` could hold the expected hash of the
downloaded zip, verified before extraction.  Deferred until the registry
ecosystem is established enough that hash distribution is meaningful.

### Native code trust

Downloaded packages that include native shared libraries (Phase 2 feature)
are fully trusted once installed — `dlopen` gives the plugin full process
access, identical to any other native extension.  The registry is a
distribution mechanism, not a trust boundary.

### Registry file trust

The registry file is a plain text file from the local filesystem.  It does
not execute any code.  A compromised registry file can point to a malicious
zip, but the user controls which registry file is used.

---

## Implementation

### New: `src/registry.rs`

```rust
pub struct RegistryEntry {
    pub name:    String,
    pub version: String,
    pub url:     String,
    /// None = active; Some("yanked:CVE-2026-001") or Some("deprecated:reason")
    pub status:  Option<String>,
}

impl RegistryEntry {
    pub fn is_yanked(&self)     -> bool { self.status.as_deref().unwrap_or("").starts_with("yanked") }
    pub fn is_deprecated(&self) -> bool { self.status.as_deref().unwrap_or("").starts_with("deprecated") }
    pub fn is_active(&self)     -> bool { self.status.is_none() }
    pub fn status_slug(&self)   -> &str { /* part after ':' */ }
}

/// Parse a registry file.  Returns all entries including yanked/deprecated.
/// Also returns the source URL extracted from the "# source: <url>" header.
pub fn read_registry(path: &str) -> (Vec<RegistryEntry>, Option<String>);

/// Find the registry file path (LOFT_REGISTRY env var → ~/.loft/registry.txt).
pub fn registry_path() -> Option<std::path::PathBuf>;

/// Find the source URL: LOFT_REGISTRY_URL env var → source: header in file → compiled-in default.
pub fn source_url(file_source: Option<&str>) -> String;

/// Find the best matching entry for install.
/// version=None → highest semver active entry; version=Some → exact match (any status).
pub fn find_package<'a>(
    entries: &'a [RegistryEntry],
    name:    &str,
    version: Option<&str>,
) -> Option<&'a RegistryEntry>;

/// Scan ~/.loft/lib/ (or given dir) for installed packages.
/// Returns (name, version) for each directory containing a readable loft.toml.
pub fn installed_packages(lib_dir: &std::path::Path) -> Vec<(String, String)>;

pub enum PackageStatus<'a> {
    Yanked     { entry: &'a RegistryEntry },
    Deprecated { entry: &'a RegistryEntry, latest: Option<&'a RegistryEntry> },
    Outdated   { installed: &'a str, latest: &'a RegistryEntry },
    Current,
    Unknown,   // name not in registry
}

/// Compare an installed (name, version) pair against the registry.
pub fn classify<'a>(
    entries: &'a [RegistryEntry],
    name:    &str,
    version: &str,
) -> PackageStatus<'a>;

/// Download the zip at entry.url to a temp file, extract, return package root.
#[cfg(feature = "registry")]
pub fn download_and_extract(
    entry:    &RegistryEntry,
    tmp_base: &std::path::Path,
) -> Result<std::path::PathBuf, String>;

/// Download url into dst_path.  Returns Err with a human-readable message on failure.
#[cfg(feature = "registry")]
pub fn download_file(url: &str, dst: &std::path::Path) -> Result<(), String>;
```

### Cargo.toml additions

```toml
[features]
registry = ["dep:ureq", "dep:zip"]

[dependencies]
ureq = { version = "2", optional = true }
zip  = { version = "2", optional = true }
```

The `registry` feature is included in the `default` feature set so that
`cargo build` produces a `loft` binary with install-from-registry support.
It is excluded from the `wasm` feature set (no network access from WASM).

### `src/main.rs` changes

**`install` subcommand:**
1. After reading the argument, determine whether it is a local path or a
   registry reference (heuristic described above).
2. For registry references: parse the optional `@version` suffix, call
   `registry::registry_path()`, `registry::read_registry()`,
   `registry::find_package()`, then `registry::download_and_extract()`.
3. Pass the extracted package root to the existing `install_package()`.
4. Remove the temporary directory after install completes (or on error).

**`registry` subcommand:**
- `registry sync` — call `registry::source_url()`, `registry::download_file()`,
  validate content, write to `registry_path()`.
- `registry check` — call `registry::installed_packages()`, `registry::read_registry()`,
  `registry::classify()` for each; print report; exit 1 if any yanked.
- `registry list [--installed]` — read registry, scan installed, print table.

### `src/lib.rs` addition

```rust
pub mod registry;
```

### Error handling

All errors during download or extraction are printed to stderr and exit with
code 1 — same pattern as the rest of `main()`.

---

## Phased Rollout

### Phase 3a — Registry lookup and download (0.8.4, Sprint 9)

- `src/registry.rs` — parse registry file, find entry, download + extract zip
- `Cargo.toml` — add `ureq` and `zip` under `registry` feature
- `src/main.rs` — extend `install` subcommand to handle registry names
- `src/lib.rs` — expose `registry` module
- Tests: unit tests in `registry.rs`; integration test `tests/registry.rs`
- Docs: this file; update `EXTERNAL_LIBS.md` Phase 3 section

### Phase 3b — Registry sync and check (0.8.4, Sprint 9)

- `loft registry sync` — download latest registry from `source:` URL
- `loft registry check` — compare installed packages against registry; exit 1 on yanks
- `loft registry list [--installed]` — browse registry with installed status column
- `status` field parsing in `read_registry()` (yanked/deprecated)
- `installed_packages()` scanner, `classify()` function
- Staleness warning when registry is older than 7 days

### Phase 3c — Registry management (future)

- `loft registry search <term>` — filter registry entries by name prefix
- `loft registry add <name> <version> <url>` — append an entry to local file
- Deferred until Phase 3b is in use and the UX is understood.

### Phase 3c — SHA-256 verification (future)

- Optional `loft.toml` field: `zip_sha256 = "abc123..."`
- Or a parallel `.sha256` file next to the `.zip` in the registry
- Verified before extraction
- Deferred until registry ecosystem is established.

---

## Code Touchpoints

| File | Change | Phase |
|------|--------|-------|
| `src/registry.rs` | New: `read_registry`, `find_package`, `download_and_extract` | 3a |
| `src/lib.rs` | Expose `registry` module | 3a |
| `src/main.rs` | Extend `install` for registry names | 3a |
| `Cargo.toml` | Add `registry` feature, `ureq`, `zip` deps | 3a |
| `tests/registry.rs` | Integration tests for 3a | 3a |
| `src/registry.rs` | Add `status` field, `classify`, `installed_packages`, `download_file`, `source_url` | 3b |
| `src/main.rs` | Add `registry sync`, `registry check`, `registry list` subcommands | 3b |
| `tests/registry.rs` | Extend with sync/check/list tests | 3b |
| `doc/claude/REGISTRY.md` | This file | 3a+3b |
| `doc/claude/EXTERNAL_LIBS.md` | Update Phase 3 section | 3a |

---

## See also

- the Registry Governance section below — submission process, review checklist, yank/deprecation procedures
- the External Libraries section below — full external library design including Phases 1 and 2
- [PACKAGES.md](PACKAGES.md) — unified package format (interpreter + native + WASM)
- [PLANNING.md](PLANNING.md) — priority backlog

---


# Registry Governance

Procedures for adding third-party libraries to the central Loft package registry
and for responding when problems are discovered in listed packages.

The registry is a plain text file (`registry.txt`) maintained in a GitHub
repository.  It starts as a personal repository (`jjstwerff/loft-registry`)
and can migrate to a shared GitHub organisation (`loft-lang/registry` or
similar) when the community grows to the point where one person cannot handle
the review load alone.  Both hosting models are described here.  The file
format is described in the Registry section below.  This document governs who
may add entries and what happens when an entry must be restricted or removed.

---

## Contents
- [Principles](#principles)
- [Shared Registry Hosting](#shared-registry-hosting)
- [Registry Format — Extended Fields](#registry-format--extended-fields)
- [Submission Requirements](#submission-requirements)
- [Review Checklist](#review-checklist)
- [Approval Workflow](#approval-workflow)
- [Native Package Track](#native-package-track)
- [Problem Reporting](#problem-reporting)
- [Severity Classification](#severity-classification)
- [Response Procedures](#response-procedures)
- [Yanking and Deprecation](#yanking-and-deprecation)
- [Author Appeals](#author-appeals)
- [Registry Maintainer Responsibilities](#registry-maintainer-responsibilities)

---

## Principles

1. **Source-visible** — every registered package must have a publicly readable
   source repository.  Binary-only packages are not accepted.
2. **Fast to restrict** — yanking a package is a one-line edit to `registry.txt`
   and takes effect immediately for new installs.  Security response must not
   be slowed by process.
3. **Proportionate** — minor bugs do not trigger yanks.  The response matches
   the severity.
4. **Stable URLs** — a registered URL for a specific version must never change.
   If the file moves, a new version entry is added.  Old entries are not edited.
5. **Scalable authority** — the process starts with one person and scales to a
   small team without changing the rules.  Any single Maintainer may approve a
   submission or act on a security report; consensus is not required for routine
   work.  Policy changes require team discussion.  See
   [Shared Registry Hosting](#shared-registry-hosting).

---

## Shared Registry Hosting

### Solo model (starting point)

The registry begins as a personal repository owned by the project author
(`jjstwerff/loft-registry`).  One person handles all submissions, yanks, and
deprecations.  The compiled-in `source:` URL in the interpreter points here.

This model works for a small package ecosystem.  When the submission queue
regularly takes more than one person can process within the response windows,
it is time to migrate to the team model.

### Team model — GitHub organisation

Create a GitHub organisation (e.g. `loft-lang`) and transfer the repository
to `loft-lang/registry`.  Update the compiled-in `source:` URL in
`src/registry.rs` and the official registry file header at the same time.
Users who run `loft registry sync` will pick up the new URL on their next sync;
no interpreter release is required.

#### Roles

| Role | Count | Permissions |
|------|-------|-------------|
| **Admin** | 1–2 | Add/remove Maintainers; change branch protection; modify this governance document |
| **Maintainer** | 2–5 | Approve submissions; yank/deprecate entries; merge PRs to `registry.txt` |
| **Reviewer** | optional | Review pull requests and issues; no merge permission |

**Reviewer** is an informal role — anyone with a GitHub account can comment on
submission issues.  The label is used in issue assignment to acknowledge people
who contribute reviews without holding Maintainer rights.

#### How decisions are made

- **Routine submissions** — any single Maintainer may approve after the review
  period.  No consensus or second approval is required.  First available
  Maintainer picks up the issue.
- **P0 yanks** — any single Maintainer may yank immediately without consulting
  others.  They notify the rest of the team via a comment on the yank commit or
  a GitHub team mention as soon as they act.
- **Rejections** — any single Maintainer may reject.  The author may re-open
  the issue and request that a different Maintainer review if they believe the
  rejection was incorrect.
- **Policy changes** (to this document) — a pull request, open for at least
  7 days, visible to all Maintainers.  No objection from any Maintainer within
  that window constitutes approval.  Objections must be resolved before merging.
- **Team membership** — Admin only.  A new member is added when nominated by
  any Maintainer and no existing Maintainer objects within 7 days.

#### Load balancing

Issues are self-assigned: any Maintainer picks up an unassigned submission.
If a submission sits unassigned for 4 days, GitHub's stale-issue bot pings
the team.  Maintainers are encouraged to claim issues they have domain knowledge
in (e.g. graphics Maintainer reviews graphics packages).

A rotating on-call schedule for P0/P1 security reports is optional but
recommended when the team reaches 3 or more members: one Maintainer per week is
designated as the primary responder for that week's urgent reports.

#### Joining the team

A person is eligible when they have:

1. Contributed at least **3 substantive reviews** on submission or problem
   issues in the registry repository (comments that check requirements, test
   the package, or identify concerns — not just "+1").
2. Been nominated by any existing Maintainer in a GitHub issue titled
   `Team nomination: <handle>`.
3. Received no objection from existing Maintainers within 7 days.

An Admin then adds the person to the GitHub team.  No vote is taken; silence
is consent.

#### Leaving the team

- **Voluntary** — open an issue or message an Admin.  Access is removed promptly.
- **Inactive** — a Maintainer with no review activity for **6 months** receives
  a 30-day notice issue.  If no activity follows, their Maintainer access is
  downgraded to Reviewer by an Admin.  They can rejoin the Maintainer role by
  resuming activity and requesting re-elevation from any Admin.

#### Branch protection settings (recommended)

```
Branch: main
  Require pull request before merging: ON
  Required approvals: 1
  Dismiss stale reviews: ON
  Allow specified actors to push directly: Maintainers (for P0 emergency yanks)
```

Allowing Maintainers to bypass the PR requirement exists solely for P0 yanks
where speed matters more than process.  Every direct push must include a
comment on the registry issue explaining the urgency.

#### Conflict resolution

If two Maintainers disagree on a submission decision:

1. Either may request a second Maintainer review by posting `@loft-lang/maintainers please review`.
2. If a third Maintainer agrees with one side, that side prevails.
3. If the team is evenly split and cannot resolve within 14 days, the submission
   is held and the author is notified.  The team writes up the specific concern
   in the issue so the author can address it directly.

For severity disputes on problem reports, the higher severity always wins
initially: it is safer to over-restrict and loosen later than the reverse.

---

## Registry Format — Extended Fields

The base format (`name version url`) is extended with an optional fourth field
to record governance status:

```
# name  version  url  [status[:detail]]
graphics  0.2.0  https://example.com/graphics-0.2.0.zip
graphics  0.1.0  https://example.com/graphics-0.1.0.zip  yanked:CVE-2026-001
opengl    0.1.0  https://example.com/opengl-0.1.0.zip    deprecated:use-graphics
math      1.0.0  https://example.com/math-1.0.0.zip      yanked:malicious
```

### Status values

| Status | Meaning |
|--------|---------|
| *(absent)* | Active — installable without warning |
| `deprecated:<reason>` | Installable but warns; excluded from "latest" selection |
| `yanked:<reason>` | Not installable; excluded from "latest"; existing installs unaffected |

The `reason` field is a short slug used in diagnostics.  It may reference a
CVE identifier, a GitHub issue number, or a brief human-readable label.

### Installer behaviour

| User action | Active | Deprecated | Yanked |
|-------------|--------|------------|--------|
| `install name` (latest) | installs | skipped — next active version is used | skipped |
| `install name@version` (exact) | installs | installs + warning | fails with reason |
| Existing install | works | works | works (no change to local files) |

When a deprecated version is the only available version:

```
warning: graphics 0.1.0 is deprecated (use-graphics).
  No other version is available.  Installing deprecated version.
```

---

## Submission Requirements

A library is eligible for submission if all of the following are true:

### Required for all packages

- **Public source repository** — hosted on GitHub, GitLab, Codeberg, or similar.
  The URL must be provided in the submission issue.
- **Open-source licence** — any OSI-approved licence is accepted.  The licence
  must appear in the repository root (`LICENSE`, `LICENSE.md`, or `COPYING`).
- **`loft.toml` with `name` and `version`** — both fields must be present and
  match the proposed registry entry.
- **Reproducible tests** — `loft --tests <pkg>/tests/` must pass cleanly on the
  submitter's platform.  Test output must be included in the submission.
- **Stable download URL** — the `.zip` URL must remain permanently accessible.
  GitHub release assets, tagged archives, or static file hosting are all
  acceptable.  Direct repository archive URLs (e.g. `github.com/.../archive/`)
  are *not* acceptable because their content can change silently.
- **No name collision** — the package name must not duplicate an existing
  registry entry (including deprecated entries).  If the intent is to supersede
  a deprecated package, contact the maintainer before submitting.

### Additional requirements for native packages

Native packages ship compiled shared libraries and execute arbitrary code inside
the interpreter process.  They require extra scrutiny:

- **Rust source only** — native extensions must be written in Rust.  Pre-compiled
  blobs with no corresponding source are rejected.
- **No `unsafe` outside the plugin boundary** — `unsafe` is permitted only in
  the `loft_register_v1` entry point and in direct FFI calls to platform APIs.
  All other Rust code must be safe.
- **Dependency audit** — the submission must list all crate dependencies and
  their versions.  Dependencies with known CVEs at submission time are a
  blocking issue.
- **Explicit capability declaration** — the submission must state clearly what
  system resources the native code accesses (network, filesystem, GPU, audio,
  etc.).  This is informational, not restrictive, but must be accurate.

---

## Review Checklist

The maintainer works through this checklist before approving:

### Pure-loft packages

- [ ] Source repository is public and readable
- [ ] Licence file is present and OSI-approved
- [ ] `loft.toml` fields `name` and `version` match the submission
- [ ] Download URL is stable (not a mutable archive URL)
- [ ] `loft --tests` passes (submitter-provided output reviewed)
- [ ] No name collision with existing registry entries
- [ ] Package description in the issue makes the purpose clear
- [ ] Package does not re-implement a core stdlib function
      (acceptable if it extends or specialises it)

### Native packages (all of the above, plus)

- [ ] Rust source is public and the entry point matches `loft_register_v1`
- [ ] `unsafe` is confined to the registration entry point and FFI calls
- [ ] Cargo.toml dependencies list reviewed; no known-vulnerable versions
- [ ] Capability declaration matches what the code actually does
- [ ] At least one reviewer other than the submitter has read the Rust source
      (the maintainer counts; community review is welcome but not required)

---

## Approval Workflow

### Step 1 — Open a submission issue

The package author opens a GitHub issue in the registry repository
(`jjstwerff/loft-registry` or `loft-lang/registry` if the team model is active)
using the **Package Submission** template.  Required fields:

- Package name and version
- Download URL (the exact `.zip` URL)
- Source repository URL
- Licence identifier (e.g. `MIT`, `Apache-2.0`, `LGPL-3.0-or-later`)
- Brief description (1–3 sentences)
- Test output paste or link to a CI run
- For native packages: capability declaration and dependency list

### Step 2 — Community review period

The issue remains open for **7 calendar days** before the maintainer makes a
decision.  Community members may:

- Report concerns (security, name confusion, licence issues)
- Confirm they tested the package successfully
- Suggest improvements to the submission

The 7-day period may be waived by any Maintainer for:
- A patch to an already-approved package (same name, new version)
- A dependency of an already-approved package

In the team model, any available Maintainer self-assigns the issue within
4 days of it being opened.  If no one self-assigns, GitHub's stale bot pings
the team.

### Step 3 — Maintainer decision

After the review period any Maintainer may act:

- **Approves** — adds the entry to `registry.txt` via a pull request, closes
  the issue with a link to the commit.
- **Requests changes** — lists specific blockers in the issue.  The author
  addresses them and re-requests review.  The same or a different Maintainer
  may handle the follow-up.  A new 7-day period does not restart unless the
  Maintainer judges the concerns were substantial.
- **Rejects** — closes the issue with a written reason.  Rejection reasons
  include: name collision, licence incompatibility, fails to build or test,
  native package fails the safety checklist, or the package duplicates
  existing stdlib functionality without adding value.  The author may ask a
  different Maintainer to re-review if they believe the rejection was wrong.

### Step 4 — Ongoing versions

Once a package is approved, the author may add new versions by opening a
**New Version** issue (lighter template: URL + test output only).  The 7-day
period applies unless waived.  The maintainer verifies the `loft.toml` version
field increments monotonically and the URL is stable, then appends the new line.

---

## Native Package Track

Native packages (those with `#native` annotations and compiled shared libraries)
follow the same workflow but with a **14-day** review period and a mandatory
Rust source review.  The checklist item "at least one reviewer other than the
submitter has read the Rust source" must be satisfied before any Maintainer
approves.

**Solo model** — if no community reviewer steps forward in 14 days, the single
maintainer performs the source review alone.  This is acceptable for small
packages but uncomfortable for large or complex ones; such packages may be held.

**Team model** — the approving Maintainer must not be the sole reviewer of the
Rust source.  A second Maintainer or a community Reviewer must have commented
confirming they read the native code.  This cross-review requirement is the
primary reason native packages exist as a separate track: with a team, it is
always satisfiable without holding packages indefinitely.

---

## Problem Reporting

Anyone — user, security researcher, or package author — may report a problem by
opening a GitHub issue in the registry repository with the **Problem Report**
label.

Required information:

- Package name and affected versions
- Description of the problem
- Reproduction steps or proof of concept (for security issues: report privately
  first — see below)
- Suggested severity (the maintainer makes the final call)

### Security vulnerabilities — private disclosure

For security issues (malicious code, data exfiltration, privilege escalation,
or any issue where publishing reproduction steps could cause immediate harm),
report privately:

- Use GitHub's **private security advisory** feature on the registry repository
  (works for both the solo and team models — all Maintainers see it).
- Email any individual Maintainer whose address is on their GitHub profile if
  the advisory feature is not available.

Any single Maintainer who receives a credible private report will yank the
affected versions within **24 hours**, before any public disclosure, and notify
the rest of the team immediately after acting.  In the team model, the on-call
Maintainer (if a rotation is in place) is the primary recipient.

---

## Severity Classification

| Severity | Examples | Target response |
|----------|----------|-----------------|
| **P0 — Critical** | Malicious code, data exfiltration, remote code execution, supply-chain attack | Yank within 24 h; no discussion required |
| **P1 — High** | Data loss, crash in common use path, security issue without active exploit | Deprecate within 48 h; yank if no fix in 14 days |
| **P2 — Medium** | Incorrect output, API incompatibility with a published version, failed tests | Notify author; deprecate if no fix in 30 days |
| **P3 — Low** | Documentation error, minor edge-case bug, cosmetic issue | Notify author; no forced action |

Severity is assigned by the maintainer after reviewing the report.  The reporter's
suggested severity is taken as input, not as binding.

---

## Response Procedures

### P0 — Critical

1. **Any single Maintainer** yanks all affected versions immediately — a
   direct push to `registry.txt` is allowed under branch protection for exactly
   this case.  No approval from other Maintainers is needed; speed is paramount.
2. The acting Maintainer posts a team notification (GitHub team mention or email)
   within 1 hour of the yank explaining what was done and why.
3. A public issue is opened describing the problem at a high level (no exploit
   details if not yet public).
4. If the author is reachable and acting in good faith, they are given
   opportunity to release a fixed version before the public issue is opened.
   This window is at most **24 hours**.
5. If the package was malicious or the author is unresponsive, the package is
   permanently removed from the registry (all versions yanked with
   `yanked:malicious` or `yanked:removed`).
6. The public issue references the yank commit and summarises the nature of the
   problem.

### P1 — High

1. Maintainer marks affected versions `deprecated:<issue-number>` within 48 h.
2. Maintainer notifies the package author via the GitHub issue and, if possible,
   via the source repository's issue tracker.
3. Author has **14 days** to release a patched version.
4. If a fix is released and passes the review checklist, the patch version is
   added to the registry and the deprecation reason updated to point to it.
5. If no fix appears in 14 days, the affected versions are yanked.

### P2 — Medium

1. A GitHub issue is opened in the registry repository referencing the problem.
2. The package author is tagged and has **30 days** to respond.
3. If a fix is released within 30 days, the new version is added normally and
   the issue is closed.
4. If no response or fix within 30 days, the affected versions are deprecated.
5. If 60 days pass with no fix, the affected versions are yanked.

### P3 — Low

1. The issue is opened and the author is notified.
2. No forced action.  The issue remains open until the author fixes it or
   closes it as "won't fix".
3. The maintainer may add a deprecation comment in the issue if the bug causes
   significant confusion, but registry entries are not changed.

---

## Yanking and Deprecation

### What yanking does

- The status field for the entry in `registry.txt` changes to `yanked:<reason>`.
- `loft install name` (latest) skips yanked entries.
- `loft install name@version` for a yanked version fails with the reason:
  ```
  error: graphics 0.1.0 has been yanked (CVE-2026-001).
    Install a different version or check the project repository for a fix.
  ```
- Existing local installations are **not removed**.  Yanking affects new installs only.
- A yanked entry is never removed from `registry.txt` entirely — the line
  remains so that users who already have that version can understand why it is
  flagged.

### What deprecation does

- The status field changes to `deprecated:<reason>`.
- `loft install name` (latest) skips deprecated entries and selects the next
  active version.  If no active version exists, the deprecated one is installed
  with a warning.
- `loft install name@version` installs the deprecated version with a warning:
  ```
  warning: graphics 0.1.0 is deprecated (outdated).
    Consider upgrading to graphics 0.2.0.
  ```
- Existing installations are unaffected.

### Permanent removal

In cases of confirmed malicious packages, the entry status is set to
`yanked:removed` and a note is added to the registry changelog.  The URL field
is replaced with a placeholder (`-`) so no download is possible even if a user
edits the status field manually.

---

## Author Appeals

If a package author believes a yank or deprecation was applied incorrectly:

1. Open a GitHub issue in the registry repository titled
   `Appeal: <package> <version>`.
2. Explain why the action was incorrect and provide evidence (fixed code,
   misattributed CVE, etc.).
3. **Solo model** — the maintainer reviews within **7 days**, taking the
   reporter's argument at face value since there is no second opinion available.
4. **Team model** — the appeal is reviewed by a Maintainer who was *not*
   involved in the original decision.  This separation is one of the concrete
   benefits of the team model: appeals are not judged by the person being
   challenged.  Resolution within **7 days**.
5. If the appeal is upheld, the status is removed or changed and a new version
   is added if appropriate.
6. P0 yanks (malicious code) are not subject to appeal.

---

## User-Side Verification

Users can check their installed packages against the latest registry at any time
using two commands (see the Registry section above):

```sh
loft registry sync     # pull latest registry.txt from GitHub
loft registry check    # compare installed packages against registry
```

`loft registry check` exits with code 1 if any installed package is yanked,
making it usable as a CI gate:

```sh
# In a CI pipeline — fails if any yanked package is installed
loft registry sync && loft registry check
```

Typical output when a yank is relevant to the user:

```
  utils  0.3.0  YANKED  CVE-2026-001 — run: loft install utils
```

The staleness warning (registry older than 7 days) reminds users to sync
regularly without being an error.

### How yanks reach users

1. Maintainer edits `registry.txt` — adds `yanked:<reason>` to the affected line.
2. The change is committed and pushed to `jjstwerff/loft-registry` on GitHub.
3. Any user who runs `loft registry sync` gets the updated file immediately.
4. `loft registry check` then surfaces the yank in the terminal and in CI.

No action is required from package authors or the loft interpreter itself to
propagate the yank — the registry file is the single source of truth.

---

## Registry Maintainer Responsibilities

These apply to every Maintainer regardless of model.

### Response times (shared commitment)

| Action | Target |
|--------|--------|
| Self-assign an open submission | 4 days |
| Complete submission review after review period | 14 days |
| P0 yank after credible private report | 24 hours |
| P1 deprecation decision | 48 hours |
| Appeal review | 7 days |

Response times are per-team, not per-individual — if the assigned Maintainer
cannot meet a deadline, any other Maintainer may step in.  In the solo model
these are personal commitments; in the team model they are collective ones.

### Record-keeping (all Maintainers)

- `registry.txt` is kept in a git repository with a public commit history.
  Every addition, yank, and deprecation is a traceable commit with the acting
  Maintainer's identity visible in `git log`.
- `REGISTRY_CHANGELOG.md` in the same repository summarises all yanks and
  deprecations in human-readable form, updated with every status change.
- Entries are never removed from `registry.txt` — only the `status` field is
  added.  The file is a permanent auditable record.

### Additional responsibilities in the team model

- **On-call rotation** — when the team has 3 or more Maintainers, maintain a
  weekly on-call schedule for P0/P1 responses.  The schedule is published in
  the repository's `MAINTAINERS.md`.
- **Monthly async review** — post a brief summary to the repository's GitHub
  Discussions each month: open submissions, recent yanks/deprecations, team
  membership changes.  This keeps all Maintainers informed even if they were
  not the ones acting.
- **MAINTAINERS.md** — keep a `MAINTAINERS.md` file in the registry repository
  listing current Maintainers, their GitHub handles, and (if applicable) which
  week they are on call.  Update it when membership changes.

### Stepping down as primary owner (solo → team migration)

When the solo maintainer decides to migrate to the team model:

1. Create the GitHub organisation and transfer the repository.
2. Invite 2–4 people who have already been reviewing submissions as community
   members; they become the first Maintainers.
3. Update the `source:` URL in the registry file and the compiled-in default
   in `src/registry.rs` in a coordinated interpreter patch release.
4. Publish a `REGISTRY_CHANGELOG.md` entry and a GitHub release note explaining
   the transition.

The original owner retains an Admin role in the organisation indefinitely,
but may reduce their Maintainer workload to match the team capacity.

---

## See also

- the Registry section below — file format, install flow, version resolution, implementation
- the External Libraries section below — package format, Phase 1–3 rollout
- [PACKAGES.md](PACKAGES.md) — package layout and native extension design

---


# External Library Support

Separately-packaged loft libraries, including libraries that ship compiled Rust
code for features that cannot be expressed in loft itself (e.g. HTTP, PNG
decoding, random number generation, OpenGL).

---

## Contents
- [Package Format](#package-format)
- [Two Flavours of Library](#two-flavours-of-library)
- [Discovery and Loading](#discovery-and-loading)
- [C-ABI Boundary Design](#c-abi-boundary-design)
- [Auto-Marshalling Dispatch](#auto-marshalling-dispatch)
- [loft-ffi Helper Crate](#loft-ffi-helper-crate)
- [Store Allocation from Native Code](#store-allocation-from-native-code)
- [Code Generation: loft generate](#code-generation-loft-generate)
- [Testing Native Packages](#testing-native-packages)
- [Security](#security)
- [Shipped Libraries](#shipped-libraries)

---

## Package Format

### Directory Layout

A library named `random` lives in a directory whose name is the library identifier:

```
random/
  loft.toml              # mandatory for native; optional for pure-loft
  src/
    random.loft          # public API surface
  native/
    Cargo.toml           # Rust crate producing a cdylib + rlib
    src/
      lib.rs             # C-ABI implementations
      generated.rs       # auto-generated stubs (from loft generate)
  tests/
    15-random.loft       # package tests
  docs/
    21-random.loft       # documentation examples
```

### Manifest: `loft.toml`

```toml
[package]
name    = "random"
version = "0.1.0"
loft    = ">=0.8"          # minimum interpreter version required

[library]
entry  = "src/random.loft"   # path to the entry .loft file
native = "loft_random"       # Cargo crate name stem (omit for pure-loft)
```

The `loft` version field is checked at load time against the interpreter version.
A major version mismatch is a fatal load-time error.

### Native Crate Convention

The `native/Cargo.toml` produces both a `cdylib` (for interpreter mode) and an
`rlib` (for `--native` codegen):

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
loft-ffi = { path = "../../../loft-ffi" }   # optional but recommended
# external crates only — never depend on the loft interpreter
```

---

## Two Flavours of Library

### Pure-Loft Libraries

Consists exclusively of `.loft` files. No build step needed.
Users write `use mylib;` and the interpreter resolves the entry file.

Examples: `lib/crypto`, `lib/game_protocol`, `lib/shapes`, `lib/arguments`.

### Native Extension Libraries

Ships a compiled shared library alongside `.loft` API files. The `.loft` files
declare function signatures with `#native` annotations; the shared library
provides the C-ABI implementations.

Examples: `lib/random`, `lib/server`, `lib/web`, `lib/imaging`, `lib/graphics`.

---

## Discovery and Loading

### Search Chain

`lib_path()` in `src/parser/mod.rs` tries candidates in order:
1. `lib/<id>.loft` and `<id>.loft` relative to CWD
2. Each directory in `parser.lib_dirs` (`--lib` / `--project` flags)
3. Packaged layout: `<dir>/<id>/src/<id>.loft` for each search directory
4. Each directory in `LOFT_LIB` environment variable
5. Fallback: `<cur_dir>/<id>.loft` / `<base_dir>/<id>.loft`

When a directory `<id>/` is found, `lib_path_manifest()` reads `loft.toml`,
validates the version requirement, and resolves the entry path.

### Load-Time Sequencing

```
parse_dir(default)                           # load standard library
parse(user_script)                           # populates pending_native_libs
scopes::check(data)                          # scope analysis
State::new(database)                         # create runtime
compile::byte_code(state, data)              # bytecode gen; native::init() runs
extensions::load_all(state, pending_libs)    # dlopen cdylibs + auto-marshal
state.execute_argv("main", ...)              # run
```

### Auto-Build

If a cdylib is not found but `native/Cargo.toml` exists, the interpreter
runs `cargo build --release` automatically via `auto_build_native()`.

---

## C-ABI Boundary Design

### The Split: Logic in cdylib, Store Access in Interpreter

Package native crates export pure C-ABI functions that operate on **primitives,
raw byte buffers, and `LoftStore`/`LoftRef` handles** — never on `Stores` or
`DbRef` directly.

```
┌──────────────────────┐     C-ABI boundary     ┌──────────────────────┐
│     Interpreter      │ ←──────────────────────→│   Package cdylib     │
│                      │                         │                      │
│  auto-marshal:       │                         │  n_rand(lo, hi):     │
│    pop args from     │── i32, i32 ────────────→│    PCG64 generate    │
│    loft stack        │                         │    return i32        │
│    ←─────────────────│── i32 ──────────────────│                      │
│    push result       │                         │  (depends on:        │
│                      │                         │   rand_core, rand_pcg│
│  (depends on: loft)  │                         │   NOT loft)          │
└──────────────────────┘                         └──────────────────────┘
```

**Key properties:**
- External crate dependencies (png, ureq, rand) live only in the cdylib
- The interpreter loads the cdylib on demand when the package is `use`d
- No Rust struct layouts are shared across the boundary
- `LoftStore` provides controlled store access via callback function pointers

### Dual-Mode Execution

| Mode | How it runs | Native function path |
|------|-------------|---------------------|
| **Interpreter** (`loft run`) | Bytecode + auto-marshal | cdylib via `dlopen`; C-ABI boundary |
| **Native codegen** (`loft --native`) | Generated Rust | Direct call via `--extern` rlib linking |

In `--native` mode, everything compiles as rlibs in one `rustc` invocation —
types are shared, calls are direct, zero overhead.

### Declaring Native Functions

```loft
pub fn rand(lo: integer, hi: integer) -> integer;
#native "n_rand"
```

The `#native` annotation tells the compiler to register a panic-stub at
bytecode time. The real function is loaded from the cdylib via
`extensions::wire_native_fns()`.

**Naming convention:**
- `n_<fn>` for global functions (e.g. `n_rand`, `n_tcp_listen`)
- `t_<N><Type>_<method>` for methods (N = char count of type name)

---

## Auto-Marshalling Dispatch

The auto-marshaller in `src/extensions.rs` bridges loft stack values to C-ABI
calls without per-function glue code.

### How It Works

1. **`compute_sig()`** reads the `#native` definition's types and produces a
   compact `NativeSig { params: Vec<ArgT>, ret: Option<ArgT> }`.

2. **`wire_native_fns()`** iterates all `#native` definitions, resolves symbols
   via dlsym, and replaces panic-stubs with the generic `native_auto_dispatch`.

3. **`native_auto_dispatch()`** pops arguments from the loft stack in reverse
   order, builds typed `ArgVal` values, and calls `dispatch_call()`.

4. **`dispatch_call()`** pattern-matches on the signature and calls the native
   function pointer with the correct C-ABI cast.

### Type Mapping

| Loft type | ArgT | C-ABI type |
|-----------|------|-----------|
| `integer` / `character` | `I32` | `i32` |
| `long` | `I64` | `i64` |
| `float` | `F64` | `f64` |
| `single` | `F32` | `f32` |
| `boolean` | `Bool` | `bool` |
| `text` | `Text` | `*const u8, usize` |
| struct / vector / collection | `Ref` | `LoftRef` (with `LoftStore` prepended) |

When any parameter or return type is `Ref`, a `LoftStore` handle is prepended
as the first C-ABI argument, giving the native function access to store memory.

For functions returning `Ref` with no `Ref` parameters (e.g. `rand_indices`),
the dispatcher allocates a fresh store for the result automatically.

### Thread-Local State

During a native call, a thread-local `CURRENT_STORES` holds a raw pointer
to the interpreter's `Stores`. This enables the `LoftStore` allocation
callbacks to reach back into the interpreter for `claim()` and `resize()`
operations.

---

## loft-ffi Helper Crate

The `loft-ffi` crate (`/loft-ffi/`) provides safe building blocks for native
extension authors. No dependencies.

### Core Types

**`LoftRef`** — Opaque reference to a store object (struct, vector, collection):
```rust
#[repr(C)]
pub struct LoftRef {
    pub store_nr: u16,
    pub rec: u32,
    pub pos: u32,
}
```

**`LoftStore`** — Direct memory access to a store buffer, with allocation callbacks:
```rust
#[repr(C)]
pub struct LoftStore {
    pub ptr: *mut u8,                    // base pointer (may move on alloc)
    pub size: u32,                       // capacity in 8-byte words
    pub ctx: LoftStoreCtx,              // opaque context for callbacks
    pub claim_fn: ...,                   // allocate words → rec
    pub reload_fn: ...,                  // refresh ptr/size after alloc
    pub resize_fn: ...,                  // resize record → new rec
}
```

**`LoftStr`** — `#[repr(C)]` text return type (borrowed pointer, valid until
next `ret()` call on the same thread).

### Text Helpers

```rust
// Convert C-ABI text parameter to &str
let name = unsafe { loft_ffi::text(name_ptr, name_len) };

// Return a String as LoftStr (stored in thread-local buffer)
loft_ffi::ret(format!("Hello, {name}!"))

// Return a borrowed &str without copying
loft_ffi::ret_ref(some_str)
```

### Field Access

`LoftStore` provides direct read/write methods for store memory:
- `get_int()` / `set_int()` — `i32` fields
- `get_long()` / `set_long()` — `i64` fields
- `get_float()` / `set_float()` — `f64` fields
- `get_byte()` / `set_byte()` — `u8` fields (boolean, simple enum)
- `get_text()` — read text field as `(*const u8, usize)`
- `get_ref()` — read sub-reference field as `LoftRef`

All take `(rec, pos, offset)` and compute byte address as `rec * 8 + pos + offset`.

### Null Sentinels

```rust
pub const NULL_INT: i32 = i32::MIN;
pub const NULL_LONG: i64 = i64::MIN;
```

---

## Store Allocation from Native Code

Native extensions can allocate records and build vectors directly in the store
via `LoftStore` methods. Each mutating operation automatically reloads the
store pointer, since allocation may trigger reallocation.

### Low-Level Allocation

```rust
// Allocate raw words (auto-reloads ptr)
let rec = unsafe { store.claim(words) };

// Resize a record (may relocate; auto-reloads ptr)
let new_rec = unsafe { store.resize(rec, new_words) };

// Manually refresh ptr/size
unsafe { store.reload() };
```

### Record Allocation

```rust
// Allocate a struct record (store_nr derived from the LoftStore handle)
let r = unsafe { store.alloc_record(words) };
// r.rec = record number, r.pos = 8 (data start)
```

### Vector Operations

```rust
// Create an empty vector with pre-allocated capacity
let mut v = unsafe { store.alloc_vector(elem_size, capacity) };

// Append elements (handles resize automatically)
unsafe { store.vector_push_int(&mut v, 42) };
unsafe { store.vector_push_long(&mut v, 123i64) };
unsafe { store.vector_push_float(&mut v, 3.14) };

// Read current length
let len = unsafe { store.vector_len(&v) };
```

The `vector_push_*` methods update `v.rec` in place if the vector record
moves during resize. The minimum allocation is 11 elements (matching the
interpreter's convention). The `store_nr` is derived automatically from
the `LoftStore` handle.

### Callback Architecture

The allocation callbacks bridge native code back into the interpreter:

```
Native extension                    Interpreter (via thread-local)
─────────────────                   ──────────────────────────────
store.claim(words)
  → claim_fn(ctx, words)    ──→    Store::claim(words) → rec
  → reload_fn(ctx, &ptr, &size) →  read store.base_ptr(), capacity
  ← updated ptr, size
  ← rec
```

`LoftStoreCtx` encodes the `store_nr`; the thread-local `CURRENT_STORES`
holds a pointer to the interpreter's `Stores` for the duration of the call.

### Safety Guarantees

The callback infrastructure provides two safety mechanisms:

1. **Panic containment**: All three callbacks (`ffi_claim`, `ffi_resize`,
   `ffi_reload`) wrap their bodies in `std::panic::catch_unwind` to prevent
   panics from propagating across the C-ABI boundary. On panic, `claim`
   returns 0, `resize` returns the original record unchanged, and `reload`
   is a no-op.

2. **RAII cleanup**: `dispatch_call` uses a guard struct whose `Drop` impl
   clears `CURRENT_STORES`, ensuring the thread-local is reset even if the
   native function or a callback panics.

---

## Code Generation: `loft generate`

The `loft generate` command reads a package's `.loft` declarations and produces
a `native/src/generated.rs` file with correct C-ABI signatures and `todo!()`
bodies.

### Usage

```sh
cd lib/random
loft generate .          # writes native/src/generated.rs
```

### What It Generates

For each `#native` declaration:

1. **C-ABI function signature** with proper type marshalling:
   - Scalars pass directly (`i32`, `i64`, `f64`, `f32`, `bool`)
   - `text` becomes `(name_ptr: *const u8, name_len: usize)` with a
     `let name = unsafe { loft_ffi::text(...) }` body line
   - Struct/vector/collection becomes `LoftRef`, with `LoftStore` prepended
   - Simple enums become `u8`

2. **Return type handling:**
   - Scalars return directly
   - `text` returns `LoftStr` with `loft_ffi::ret(result)` pattern
   - Struct/vector returns `LoftRef`

3. **Field offset modules** for struct types referenced as parameters:
   ```rust
   pub mod image_fields {
       pub const NAME: u16 = 0;   // text (record ref)
       pub const WIDTH: u16 = 4;  // integer
       pub const HEIGHT: u16 = 8; // integer
       pub const DATA: u16 = 12;  // vector ref
   }
   ```

4. **`todo!()` bodies** for the developer to fill in.

### Example Output

For `fn rand_indices(n: integer) -> vector<integer>; #native "n_rand_indices"`:

```rust
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_rand_indices(
    store: loft_ffi::LoftStore,
    n: i32,
) -> loft_ffi::LoftRef /* vector<integer> */ {
    let result: loft_ffi::LoftRef = todo!("implement n_rand_indices(n)");
    result
}
```

---

## Testing Native Packages

### From the package directory

```sh
cd lib/random
loft test                    # runs all tests in tests/
loft test 15-random          # runs a single test file
```

`loft test` reads `loft.toml`, adds `src/` to the import path, resolves
dependencies, and discovers test files in `tests/`. Native libraries are
registered before parsing test files.

### From the project root

```sh
make test-packages           # discovers lib/*/tests/*.loft and runs loft test
make ci                      # includes test-packages after cargo test
```

---

## Security

### Native Extensions Are Fully Trusted

Loading via `dlopen` gives the plugin full process access. No sandbox. This
mirrors Python ctypes, Ruby FFI, and Node.js native addons.

### File-System Sandboxing Is Not Inherited

The `--project` flag sandboxes loft script file I/O. This does NOT apply to
native extension code.

---

## Shipped Libraries

| Library | Type | Native deps | Functions |
|---------|------|------------|-----------|
| `random` | Native | `rand_core`, `rand_pcg` | `rand`, `rand_seed`, `rand_indices` |
| `server` | Native | std::net only | TCP listen/accept, HTTP parse, WebSocket |
| `web` | Native | `ureq` | HTTP client (`http_do`) |
| `imaging` | Native | `png` | PNG decode (`load_png`) |
| `graphics` | Native | `glutin`, `gl` | OpenGL window/rendering |
| `crypto` | Pure loft | — | SHA-256, HMAC, base64 |
| `game_protocol` | Pure loft | — | Multiplayer messaging |
| `shapes` | Pure loft | — | Shape helpers |
| `arguments` | Pure loft | — | CLI argument parsing |

---

## Key Source Files

| File | Role |
|------|------|
| `src/extensions.rs` | cdylib loader, auto-marshalling dispatcher, allocation callbacks |
| `src/native.rs` | Built-in function registry (`FUNCTIONS` table, `init()`) |
| `src/manifest.rs` | `loft.toml` reader and version checker |
| `src/main.rs` | `generate_native_stubs()` for `loft generate` |
| `loft-ffi/src/lib.rs` | `LoftRef`, `LoftStore`, `LoftStr`, allocation helpers |

---

## See Also
- [COMPILER.md](COMPILER.md) — `lib_path()` and `parse_file()` internals
- [INTERNALS.md](INTERNALS.md) — `State::static_fn()` and `native::init()` details
- the Registry section below — Package registry design

---

separate GitHub repositories for independent publishing and development.

---

## Current state

All libraries live under `lib/` in the main `loft` repository:

| Library | Type | Native crate | Dependencies |
|---|---|---|---|
| `arguments` | pure-loft | — | — |
| `crypto` | pure-loft | — | — |
| `game_protocol` | pure-loft | — | — |
| `shapes` | pure-loft | — | `graphics` |
| `random` | native | `loft-random` | — |
| `web` | native | `loft-web` | — |
| `imaging` | native | `loft-imaging` | — |
| `server` | native | `loft-server` | `web` |
| `graphics` | native | `loft-graphics-native` | — |

Standalone `.loft` files not yet packaged: `code.loft`, `docs.loft`,
`lexer.loft`, `parser.loft`, `logger.loft`, `wall.loft`, `testlib.loft`.

---

## Target: three repositories

### 1. `loft-graphics` — dedicated repo

Large, complex, platform-specific. Has its own Rust dependencies (glutin,
gl, winit, fontdue), 22 tutorial examples, and will grow into a full
graphics engine.

**Contents:**

```
loft-graphics/
  graphics/          # OpenGL bindings, canvas, color, font rendering
  shapes/            # Shape generation (depends on graphics)
  engine/            # (future) Scene graph, game loop, asset pipeline
```

**Rationale:** GPU/headless-GL CI requirements, high iteration rate during
engine development, different contributor profile (graphics programmers).

### 2. `loft-server` — dedicated repo

Complex networking stack with security-sensitive dependencies. Will grow
with TLS, ACME, auth, RBAC, and game-server features.

**Contents:**

```
loft-server/
  server/            # TCP, HTTP, WebSocket
  web/               # HTTP client (ureq) — server depends on this
  game_protocol/     # Multiplayer messaging protocol
```

**Rationale:** Security updates on networking crates need independent
release cadence. Integration testing requires network access. `web` is
bundled here because `server` depends on it and they share the HTTP domain.

### 3. `loft-libs` — monorepo for everything small

All remaining libraries: small Rust-crate wrappers and pure-loft utilities.
Easy to manage together, similar structure, low complexity.

**Contents (initial):**

```
loft-libs/
  random/            # RNG (rand_pcg wrapper)
  crypto/            # SHA-256, HMAC, base64
  imaging/           # PNG encode/decode (png crate wrapper)
  arguments/         # CLI argument parsing
```

**Future additions** (as they get packaged):

```
  json/              # JSON parse/serialize
  regex/             # Regular expressions
  csv/               # CSV reading/writing
  logger/            # Structured logging
  ...
```

---

## What stays in `loft`

- `default/*.loft` — standard library, tightly coupled to interpreter version
- `loft-ffi/` — the FFI helper crate, used by all native libraries
- `tests/lib/` — test packages for the library loading mechanism itself
- Standalone `.loft` files (`lexer.loft`, `parser.loft`, etc.) — these are
  tools for the language itself, not user-facing libraries

---

## Migration steps

### Phase 1: Prepare (before any move)

- [ ] **P1.1** Ensure all library tests pass: `make test`
- [ ] **P1.2** Tag the current state: `git tag pre-lib-split`
- [ ] **P1.3** Create the three GitHub repositories:
      `loft-graphics`, `loft-server`, `loft-libs`
- [ ] **P1.4** Design a shared CI workflow template for library repos
      (build native crates, run `loft` test discovery on `tests/`)
- [ ] **P1.5** Decide on `loft-ffi` distribution: publish to crates.io
      or use git dependency. Native libraries in external repos need to
      reference it somehow

### Phase 2: Extract `loft-graphics`

- [ ] **P2.1** Create `loft-graphics` repo with README, LICENSE, CI
- [ ] **P2.2** Copy `lib/graphics/` and `lib/shapes/` preserving directory
      structure. Update `shapes/loft.toml` dependency path
- [ ] **P2.3** Copy or symlink `loft-ffi` (or point Cargo.toml at crates.io /
      git dep)
- [ ] **P2.4** Verify: all graphics and shapes tests pass standalone
- [ ] **P2.5** Set up release CI: on tag push, build zips per library,
      attach to GitHub Release
- [ ] **P2.6** Remove `lib/graphics/` and `lib/shapes/` from main repo

### Phase 3: Extract `loft-server`

- [ ] **P3.1** Create `loft-server` repo with README, LICENSE, CI
- [ ] **P3.2** Copy `lib/server/`, `lib/web/`, `lib/game_protocol/`
- [ ] **P3.3** Update `server/loft.toml` dependency on `web` to use local
      path within the new repo
- [ ] **P3.4** Verify: all server, web, and game_protocol tests pass
- [ ] **P3.5** Set up release CI
- [ ] **P3.6** Remove from main repo

### Phase 4: Extract `loft-libs`

- [ ] **P4.1** Create `loft-libs` repo with README, LICENSE, CI
- [ ] **P4.2** Copy `lib/random/`, `lib/crypto/`, `lib/imaging/`,
      `lib/arguments/`
- [ ] **P4.3** Verify: all tests pass
- [ ] **P4.4** Set up release CI (single release, one zip per library)
- [ ] **P4.5** Remove from main repo

### Phase 5: Clean up main repo

- [ ] **P5.1** Remove empty `lib/` directory (or keep only for `loft-ffi`)
- [ ] **P5.2** Update documentation: CLAUDE.md, EXTERNAL_LIBS.md, PACKAGES.md
      to reference the new repos
- [ ] **P5.3** Update `LOFT_LIB` / `--lib` documentation to explain how
      users point at externally cloned libraries
- [ ] **P5.4** Consider a `loft install` command or script that clones/
      downloads libraries from their repos

---

## Release workflow (per library repo)

Each repo publishes releases with one zip per library:

```yaml
# .github/workflows/release.yml
on:
  push:
    tags: ['v*']
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Package libraries
        run: |
          for dir in */; do
            [ -f "$dir/loft.toml" ] || continue
            name="${dir%/}"
            zip -r "${name}-${GITHUB_REF_NAME}.zip" "$dir"
          done
      - uses: softprops/action-gh-release@v2
        with:
          files: "*.zip"
```

Download URLs follow the pattern:
```
https://github.com/<org>/<repo>/releases/download/v1.0.0/<library>-v1.0.0.zip
```

These URLs map directly into the loft package registry format described in
REGISTRY.md.

---

## Open questions

1. **`loft-ffi` distribution** — crates.io publish vs git dependency?
   Publishing to crates.io is cleaner for external library authors but
   adds a release step. Git dependency is simpler for now.

2. **Shared versioning vs per-library versioning** — within each repo,
   do all libraries share a version (simpler) or version independently
   (more flexible)? Recommend starting with shared versions.

3. **CI loft binary** — library repos need a `loft` binary to run tests.
   Options: download from GitHub Releases, build from source as CI step,
   or use a pre-built Docker image.

4. **Transitive dependencies across repos** — `shapes` depends on
   `graphics` (same repo, fine). If a future `loft-libs` library needs
   `server`, that's a cross-repo dependency. The registry / `loft install`
   needs to handle this.
