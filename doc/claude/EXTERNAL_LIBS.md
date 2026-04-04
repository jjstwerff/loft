---
render_with_liquid: false
---
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# External Library Support

This document describes the design for separately-packaged Loft libraries, including
libraries that ship compiled Rust code for features that cannot be expressed in Loft
itself (e.g. OpenGL, audio, database drivers).

---

## Contents
- [Overview](#overview)
- [Package Format](#package-format)
- [Two Flavours of Library](#two-flavours-of-library)
- [Discovery and Loading](#discovery-and-loading)
- [Native Function Registration](#native-function-registration)
- [OpenGL — Concrete Example](#opengl--concrete-example)
- [Build Tooling](#build-tooling)
- [Backwards Compatibility and ABI Stability](#backwards-compatibility-and-abi-stability)
- [Security and Sandboxing](#security-and-sandboxing)
- [Phased Rollout](#phased-rollout)
- [Code Touchpoints Summary](#code-touchpoints-summary)

---

## Overview

The interpreter currently supports two categories of built-in functionality:

- **Loft-implemented** — pure `.loft` files in `default/`, loaded by `parse_dir()` in
  `src/main.rs` before the user script.
- **Native (Rust)** — declared with `#rust "..."` annotations and backed by Rust
  functions registered in the `FUNCTIONS` table in `src/native.rs`, installed via
  `native::init(state)` inside `compile::byte_code()`.

Neither mechanism supports code arriving at runtime from an external directory. This
document designs a two-phase extension of the existing `use` / `lib_path()` system
to support separately-packaged libraries, including ones that ship compiled shared
libraries.

Two existing extension points shape the design:
- `lib_path()` in `src/parser/mod.rs` — already implements a multi-step search chain
  ending with `LOFT_LIB`. New package layouts add two candidates there.
- `state.static_fn(name, call)` in `src/state/mod.rs` — the only official registration
  path for new `Call = fn(&mut Stores, &mut DbRef)` functions. All plugin registration
  ultimately calls this.

---

## Package Format

### Directory Layout

A library named `opengl` lives in a directory whose name is the library identifier:

```
opengl/
  loft.toml            # mandatory for native; optional for pure-loft
  src/
    opengl.loft        # public API surface (types, fn signatures, bodies)
    internal.loft      # optional additional loft files
  native/
    libloft_opengl.so      # Linux shared library (optional)
    libloft_opengl.dylib   # macOS shared library (optional)
    loft_opengl.dll        # Windows DLL (optional)
```

The interpreter resolves the entry `.loft` file at `<lib-dir>/<name>/src/<name>.loft`,
falling back to `<lib-dir>/<name>.loft` for single-file pure-loft libraries (preserving
the current `lib_path()` behaviour). The `native/` directory is only present when Rust
code is included.

### Manifest: `loft.toml`

```toml
[package]
name    = "opengl"
version = "0.2.1"
loft    = ">=1.0"          # minimum interpreter version required

[library]
# Path to the entry .loft file, relative to the package root.
# Default: "src/<name>.loft"
entry = "src/opengl.loft"

# Shared library name stem (without lib prefix or extension).
# Omit entirely for pure-loft packages.
native = "loft_opengl"
```

The manifest is optional for single-file pure-loft libraries. It is mandatory for native
extension packages.

### Versioning

Version strings follow semver. The `loft` field is checked at load time against
`env!("CARGO_PKG_VERSION")`. A major version mismatch, or a requested minor/patch
greater than the interpreter's, is a fatal load-time error. This check runs inside the
new `lib_path_manifest()` helper in `src/parser/mod.rs`, before any parsing or dynamic
loading occurs.

---

## Two Flavours of Library

### Pure-Loft Libraries

A pure-loft library is identical in structure to the current `default/` directory but
packaged separately. It consists exclusively of `.loft` files.

No changes to the runtime are required. The interpreter reads the entry `.loft` file via
the existing `parse_dir()` / `parse()` flow. The only change is to `lib_path()`: when it
finds a directory named `<id>`, it reads `loft.toml` (if present), resolves the `entry`
field, and returns that path.

**Authoring model:** write ordinary loft code, ship a directory. Users write `use mylib;`.

### Native Extension Libraries

A native extension library ships a shared library alongside its `.loft` API files. The
shared library exposes a single C-ABI entry point:

```c
// Exported symbol: loft_register_v1
void loft_register_v1(LoftPluginCtx *ctx);
```

where `LoftPluginCtx` is defined in the companion crate `loft-plugin-api`. The library
author links against `loft-plugin-api`, calls the provided `register_fn` callback for each
native function to expose, and builds a `cdylib`.

The interpreter locates and `dlopen`s the shared library **after** `byte_code()` (so
that function names declared in `.loft` API files are already known), then calls
`loft_register_v1`, which populates a staging function table. After the call returns the
interpreter iterates that table and calls `state.static_fn(name, fn_ptr)` for each
entry — the same path used by `native::init()` today.

---

## Discovery and Loading

### Search Chain (as shipped)

`lib_path()` in `src/parser/mod.rs` tries the following candidates in order:
1. `lib/<id>.loft` relative to CWD
2. `<id>.loft` relative to CWD
3. `<cur_dir>/lib/<id>.loft`
4. `<base_dir>/lib/<id>.loft` (when inside `tests/`)
5. `<cur_script_stem>/<id>.loft`
6–7. Each directory in `parser.lib_dirs` (`--lib` / `--project` flags), single-file
7c. **[pure-loft layout, shipped 2026-03-16]** `<dir>/<id>/src/<id>.loft` for each `<dir>` in `lib_dirs` — packaged layout
7b. Each directory in `LOFT_LIB` (env-var, cross-platform split), single-file
7d. **[pure-loft layout, shipped 2026-03-16]** `<dir>/<id>/src/<id>.loft` for each `<dir>` in `LOFT_LIB`
8–9. `<cur_dir>/<id>.loft` / `<base_dir>/<id>.loft`

The helper `lib_path_manifest(dir, id) -> Option<String>` checks that `<dir>/<id>` is a
directory, reads `<dir>/<id>/loft.toml` when present, validates the version requirement,
and returns the resolved entry path (or `None` on mismatch or missing file).

### Load-Time Sequencing

No change to `use` syntax. The existing `use <identifier>;` grammar is sufficient.

What changes: `parse_file()` in `src/parser/mod.rs` processes `use` statements.
Phase 2 will extend `lib_path_manifest()` to also return the parsed manifest when
a `native = "..."` field is present; the interpreter will then resolve the
platform-correct shared library path and append it to a new `pending_native_libs:
Vec<String>` on `Parser`. The actual `dlopen` step happens in `main.rs` after
`byte_code()`, not during parsing.

Extended startup sequence in `main.rs`:

```
parse_dir(default)                           // existing
parse(user_script)                           // existing — populates pending_native_libs
scopes::check(data)                          // existing
State::new(database)                         // existing
compile::byte_code(state, data)              // existing — native::init() runs here
extensions::load_all(state, pending_libs)    // NEW — dlopen + register
state.execute_argv("main", ...)              // existing
```

---

## Native Function Registration — Dual-Mode Design

Native functions run in two modes depending on how the loft program is executed.

### The two modes

| Mode | How it runs | Native function path |
|------|-------------|---------------------|
| **Interpreter** (`loft run`, `loft test`) | Bytecode interpreter | cdylib loaded via `dlopen`; C-ABI boundary |
| **Native codegen** (`loft --native`) | Generated Rust compiled by `rustc` | Direct call via `--extern` rlib linking |

### Why two modes

A Rust `cdylib` embeds its own copy of all dependencies.  Rust does not
guarantee struct layout (`repr(Rust)`) across compilation units.  Passing
`&mut Stores` from the interpreter to a cdylib causes segfaults because the
two copies of `Stores` have different field offsets.

The `--native` path avoids this: the generated Rust source and the package's
native crate are compiled together by `rustc`, sharing the same `loft` rlib.
No dynamic loading, no layout mismatch, zero-cost direct calls.

### The C-ABI split: logic in the cdylib, store access in the interpreter

Package native crates (`cdylib`) export pure C-ABI functions that operate on
**primitives and raw byte buffers only** — never on `Stores` or `DbRef`.
The interpreter provides a thin glue function (in `src/native.rs`) that:

1. Pops arguments from the loft stack using `stores.get::<T>()`
2. Calls the cdylib's C-ABI function with primitive values
3. Writes the result back into the store using `stores.put()` / `store.claim()`

```
┌──────────────────────┐     C-ABI boundary     ┌──────────────────────┐
│     Interpreter      │ ←──────────────────────→│   Package cdylib     │
│                      │                         │                      │
│  n_load_png():       │                         │  loft_decode_png():  │
│    path = pop(stack) │                         │    decode PNG file   │
│    call cdylib ──────│── path_ptr, path_len ──→│    return pixels     │
│    ←─────────────────│── w, h, pixel_ptr ──────│                      │
│    write to store    │                         │                      │
│    push(stack, true) │                         │  (depends on: png)   │
│                      │                         │  (no loft dep)       │
│  (depends on: loft)  │                         │                      │
│  (no png dep needed) │                         │                      │
└──────────────────────┘                         └──────────────────────┘
```

**Key property**: the `png` crate is only a dependency of the cdylib, not of
the interpreter.  The interpreter loads the cdylib on demand when the imaging
package is used.

### Passing bulk data efficiently via raw pointers

For functions that process large buffers (textures, audio, network payloads),
the interpreter can pass **raw pointers into store memory** across the C-ABI
boundary.  The store's data is contiguous — `DbRef` resolves to a byte offset
in a flat memory region.

**Reading store data from a cdylib** (e.g. OpenGL texture upload):

```
Interpreter:                    cdylib:
  resolve DbRef → *const u8      receive (ptr, len)
  pass (ptr, len) via C-ABI      glTexImage2D(ptr, len, ...)
```

**Writing into store memory from a cdylib** (e.g. PNG decode, network receive):

```
Interpreter:                    cdylib:
  store.claim(size)               receive (ptr, len)
  resolve → *mut u8               write decoded data into ptr
  pass (ptr, len) via C-ABI       return bytes_written
  update store metadata
```

The pointer is valid for the duration of the C-ABI call — the interpreter
holds the store and does not relocate memory during the call.  This gives
native code **zero-copy access** to store buffers for both reads and writes.

**No `Stores` type crosses the boundary.**  The interpreter resolves the
`DbRef` to a raw pointer before calling the cdylib, and updates store metadata
(vector length, field values) after the call returns.

### Example: PNG decode (imaging package)

**cdylib** (`lib/imaging/native/src/lib.rs`) — depends on `png` only:

```rust
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_decode_png(
    path_ptr: *const u8, path_len: usize,
    out_width: *mut u32, out_height: *mut u32,
    out_pixels: *mut *mut u8, out_pixels_len: *mut usize,
) -> bool {
    // Decode using the png crate, return raw RGB bytes
}
```

**Interpreter glue** (`src/native.rs`) — no `png` dependency:

```rust
fn n_load_png(stores: &mut Stores, stack: &mut DbRef) {
    let path = stores.get::<Str>(stack);
    // Call cdylib's decode function (resolved via dlopen)
    let (w, h, pixels) = call_decode_png(path.str());
    // Write pixel data into the store
    write_image_to_store(stores, image_ref, w, h, &pixels);
    stores.put(stack, true);
}
```

### Example: OpenGL texture upload (future)

**cdylib** (`lib/opengl/native/src/lib.rs`) — depends on `gl` only:

```rust
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_gl_upload_texture(
    pixel_ptr: *const u8,    // raw pointer into store memory
    pixel_len: usize,
    width: u32,
    height: u32,
) -> u32 {
    // Upload directly from the store buffer — zero copy
    gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGB as i32,
        width as i32, height as i32, 0,
        gl::RGB, gl::UNSIGNED_BYTE, pixel_ptr as *const _);
    texture_id
}
```

**Interpreter glue**:

```rust
fn n_gl_upload_texture(stores: &mut Stores, stack: &mut DbRef) {
    let image_ref = stores.get::<DbRef>(stack);
    // Resolve DbRef to raw pointer — zero copy
    let (ptr, len) = stores.buffer_ptr(image_ref);
    let tex_id = call_gl_upload_texture(ptr, len, width, height);
    stores.put(stack, tex_id as i32);
}
```

### How packages declare native functions

```loft
fn load_png(path: text, image: Image) -> boolean;
#native "n_load_png"
```

The `#native` annotation tells the compiler:
- **Interpreter**: register a panic-stub at bytecode time; the real function
  is loaded via `native::init()` from the `FUNCTIONS` table in `src/native.rs`.
- **`--native`**: emit a direct Rust call to the symbol in the loft rlib.

### Naming convention

- `n_<fn>` for global functions (e.g. `n_load_png`)
- `t_<N><Type>_<method>` for methods (e.g. `t_5Image_width`, where 5 = len("Image"))

### `--native` codegen path

When compiling with `--native`, the generated Rust source calls native functions
directly.  The `loft` crate provides built-in glue functions as `pub fn` exports.
The `rustc` invocation includes `--extern loft=<path>/libloft.rlib`.

For packages with their own native crate, the build pipeline additionally passes
`--extern <pkg>=<path>/lib<pkg>.rlib` so the generated code can call the
package's functions directly — bypassing the cdylib C-ABI boundary entirely.
In `--native` mode, the PNG decode function is called as a normal Rust function
with shared types, not through `dlopen`.

### Adding a new native function to a package

1. **cdylib**: implement the pure logic as `extern "C"` functions in
   `native/src/lib.rs`.  Only depend on external crates (png, gl, ureq) — not loft.
2. **Interpreter glue**: add a thin wrapper in `src/native.rs` that pops stack
   args, calls the cdylib function pointer, and writes results into stores.
3. **extensions.rs**: add the C-ABI function pointer type and symbol resolution.
4. **Package .loft**: declare with `#native "n_my_func"`.
5. Works in both interpreter and `--native` modes automatically.

---

## Testing Native Packages

### Running package tests

From the package directory:

```sh
cd lib/imaging
loft test                    # runs all tests in tests/
loft test 14-image           # runs a single test file
```

`loft test` reads `loft.toml`, adds `src/` to the import path, resolves
dependencies via the parent directory, and discovers test files in `tests/`.

For packages with `native = "stem"` in `loft.toml`, the interpreter
registers the package's own native library path before parsing test files.
Dependency native libraries are discovered automatically when the parser
processes `use` statements.

### Testing from the project root

```sh
make test-packages           # discovers lib/*/tests/*.loft and runs loft test
make ci                      # includes test-packages after cargo test
```

`make test-packages` iterates over `lib/*/tests/*.loft`, entering each
package directory and running `loft test` so that `loft.toml` is found
and dependencies are resolved correctly.

---

## OpenGL — Concrete Example

### User Code

```loft
use opengl;

fn main() {
    window = gl_create_window(800, 600, "Hello OpenGL");
    gl_clear(0x002244ff);
    gl_present(window);
    gl_destroy_window(window);
}
```

No prefix qualification is needed (consistent with how `use mylib;` works today).

### Package on Disk

```
~/.loft/libs/opengl/
  loft.toml
  src/
    opengl.loft
  native/
    libloft_opengl.so      # Linux
    libloft_opengl.dylib   # macOS
    loft_opengl.dll        # Windows
```

`loft.toml`:

```toml
[package]
name    = "opengl"
version = "0.1.0"
loft    = ">=1.0"

[library]
entry  = "src/opengl.loft"
native = "loft_opengl"
```

### `opengl.loft` — API Surface

```loft
// Opaque window handle
type Window size(4);

// Declare functions backed by native Rust implementations.
// The `#native` annotation names the symbol registered by loft_register_v1.
pub fn gl_create_window(width: integer, height: integer, title: text) -> Window;
#native "n_gl_create_window"

pub fn gl_clear(color: integer);
#native "n_gl_clear"

pub fn gl_present(window: Window);
#native "n_gl_present"

pub fn gl_destroy_window(window: Window);
#native "n_gl_destroy_window"
```

`#native "name"` is a new annotation parallel to the existing `#rust "..."` inline-code
annotation. At bytecode generation the compiler looks up `name` in `state.library_names`.
If not found, a fatal diagnostic is emitted: _"native function 'name' was not registered;
is the extension loaded?"_

### Native Functions

OpenGL functions don't access stores — they call the `gl` crate directly.
These are independent functions that will use the host callback interface
(once implemented).  For now, they are registered as interpreter built-ins:

```rust
// src/native_gl.rs (in the loft interpreter, behind "opengl" feature)
fn n_gl_create_window(stores: &mut Stores, stack: &mut DbRef) {
    let title  = *stores.get::<Str>(stack);
    let height = *stores.get::<i32>(stack);
    let width  = *stores.get::<i32>(stack);
    // ... platform-specific window creation ...
    stores.put(stack, handle as i32);
}
```

In `--native` mode, the generated code calls these directly from the loft rlib.

When the host callback interface (LoftHost) is implemented, these functions
will move to a `lib/opengl/native/` cdylib that only pops/pushes stack values
through C-ABI callbacks and calls `gl::*` functions independently.

---

## Build Tooling

### Pure-Loft Authors

No build step needed. Ship a directory with `.loft` files and an optional `loft.toml`.

### Native Extension Authors

Standard Cargo workflow:

```sh
cargo build --release --target x86_64-unknown-linux-gnu
cargo build --release --target x86_64-apple-darwin
cargo build --release --target x86_64-pc-windows-gnu
```

The interpreter resolves the platform filename at runtime:
- Linux: `lib<name>.so`
- macOS: `lib<name>.dylib`
- Windows: `<name>.dll`

### Distribution

Ship as a directory tree:

```
opengl-0.1.0/
  loft.toml
  src/
  native/
  README.md
  LICENSE
```

Users place the directory anywhere in `LOFT_LIB` or in their project's `lib/` directory.
No package registry is defined at this stage — deferred to Phase 3.

---

## Compatibility

### Stable API Surface

| Element | Notes |
|---------|-------|
| `fn(&mut Stores, &mut DbRef)` | The `Call` type for store-coupled functions |
| `stores.get::<T>(stack)` / `stores.put(stack, val)` | Stack accessors for primitives |
| `DbRef`, `Str` in `keys.rs` | Heap pointer and string view types |
| `#native "n_func"` annotation | Declares a native-backed function in `.loft` |

**Store-coupled functions** live in `src/native.rs` and are part of the
interpreter.  They use `Stores`/`DbRef` directly — safe because they run in
the same compilation unit.

**Independent functions** (future) use a `repr(C)` callback table (`LoftHost`)
that only passes C-ABI primitives across the cdylib boundary.  No Rust struct
layouts are shared.

**`--native` mode** links everything as rlibs in one `rustc` invocation — all
types are shared, all calls are direct, zero overhead.

---

## Security and Sandboxing

### Native Extensions Are Fully Trusted

Loading via `dlopen` gives the plugin full process access. There is no sandbox. This is
the correct tradeoff for a developer-tooling scripting language and mirrors the model of
Python ctypes, Ruby FFI, and Node.js native addons.

When a native extension is loaded, the interpreter prints to stderr:

```
loft: loading native extension 'opengl' from /home/user/.loft/libs/opengl/native/libloft_opengl.so
```

This is suppressible with `--quiet`.

### File-System Sandboxing Is Not Inherited

The `--project` flag sandboxes loft script file I/O. This does NOT apply to native
extension code. Extension authors are responsible for their own I/O behaviour.

### Future: `--no-native` Flag

A `--no-native` flag would disable all native extension loading for use in CI or
untrusted-script environments. Implementation: if the flag is set and
`p.pending_native_libs` is non-empty, exit with a fatal diagnostic before
`load_native_extensions()` is called. Design this flag into `main.rs` from Phase 2,
even if not exposed until Phase 3.

### Future: Hash Verification

A `loft.toml` field `native_sha256` could record the expected SHA-256 of each
platform's shared library for verification before loading. Deferred until a package
registry exists to make hash distribution meaningful.

---

## Phased Rollout

### Phase 1 — Pure-Loft Package Layout ✓ shipped (2026-03-16)

**Goal:** A developer can ship a multi-file loft library in a directory with `loft.toml`,
place it in `LOFT_LIB`, and use it with `use mylib;`.

**Shipped changes:**

1. `src/parser/mod.rs` — `lib_path()` extended with steps 7c/7d for the
   `<dir>/<id>/src/<id>.loft` layout, in both `lib_dirs` and `LOFT_LIB`.
2. `src/manifest.rs` (new) — minimal `loft.toml` line-scanner.  Checks the
   `loft = ">=X.Y"` version requirement; emits a fatal diagnostic on mismatch.
   Reads the optional `[library] entry` override.
3. `tests/package_layout.rs` + `tests/lib/testpkg/` — two integration tests
   (layout discovery and version-mismatch rejection).

No changes to `State`, `Stores`, `native.rs`, or `compile.rs`.

---

### Phase 2 — Native Extension Loading ✓ shipped

**Goal:** A Rust crate compiled to a shared library can register new native functions
with the interpreter via `use mylib;` in a loft script.

**Shipped changes:**

1. `Cargo.toml` — `libloading` under `native-extensions` feature (now default).
2. `src/extensions.rs` — `load_all()`, `load_one()` with pure-Rust `loft_register`,
   `auto_build_native()` for on-demand compilation.
3. `src/manifest.rs` — parses `native = "..."`, `[native]`, `[native.functions]`,
   `[native.wasm]` sections.
4. `src/parser/mod.rs` — `pending_native_libs` populated with auto-build fallback.
5. `src/main.rs` — calls `extensions::load_all()` between `byte_code()` and execution.
6. `src/test_runner.rs` — calls `extensions::load_all()` per test function.
7. `src/compile.rs` — `#native "name"` registers panic-stubs replaced at load time.

---

### Phase 3 — Registry and Install (target: 0.8.4)

Full design in [REGISTRY.md](REGISTRY.md).

#### Phase 3a — Registry lookup and download

A plain text registry file (`~/.loft/registry.txt` or `LOFT_REGISTRY`) maps
package names and versions to `.zip` download URLs:

```
# name  version  url
graphics 0.1.0  https://example.com/graphics-0.1.0.zip
graphics 0.2.0  https://example.com/graphics-0.2.0.zip
opengl   0.1.0  https://example.com/opengl-0.1.0.zip
```

New CLI behaviour:
- `loft install graphics` — find latest version in registry, download zip, install
- `loft install graphics@0.1.0` — install specific version
- `loft install .` / `loft install /path` — local install unchanged (Phase 1)

New source file: `src/registry.rs` with `read_registry()`, `find_package()`, and
`download_and_extract()` (feature-gated under `registry`).

New Cargo deps (optional, `registry` feature, on by default):
- `ureq = "2"` — blocking HTTP download
- `zip  = "2"` — zip extraction

#### Phase 3b — Registry management commands (future)

- `loft registry list` / `search` / `fetch <url>` / `add`

#### Deferred

- SHA-256 hash verification of downloaded zips.
- `--no-native` flag for sandboxed environments.
- Thin `loft-plugin-api` accessor wrappers (Option B) for non-Rust plugin authors.
- Cross-platform pre-built binary packages distributed by library authors.
- Documentation generation from external package `.loft` API files.

---

## Code Touchpoints Summary

| File | Change | Phase |
|------|--------|-------|
| `src/parser/mod.rs` | Extend `lib_path()` for directory layout; add `lib_path_manifest()`; add `pending_native_libs` to `Parser`; populate in `use` loop | 1 + 2 |
| `src/manifest.rs` | New: minimal `loft.toml` reader, version check | 1 |
| `src/main.rs` | Add `extensions::load_all()` call between `byte_code()` and `execute_argv()` | 2 |
| `src/extensions.rs` | New: `load_all()`, `load_one()`, trampoline | 2 |
| `src/compile.rs` | Handle `#native "name"` annotation in `byte_code()` | 2 |
| `Cargo.toml` | Add `libloading` under `native-extensions` feature | 2 |
| `plugin-api/` | New workspace member: `LoftPluginCtx`, `loft-plugin-api` crate | 2 |

---

## See also
- [PLANNING.md](PLANNING.md) — Version milestones and priority-ordered backlog
- [COMPILER.md](COMPILER.md) — `lib_path()` and `parse_file()` internals
- [INTERNALS.md](INTERNALS.md) — `State::static_fn()` and `native::init()` details
- [../DEVELOPERS.md](../DEVELOPERS.md) — Feature proposal process and quality gates
