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

## Native Function Registration

### The Plugin API Crate

A new workspace member `plugin-api/` (or published to crates.io as `loft-plugin-api`)
defines the stable C-ABI boundary:

```rust
// plugin-api/src/lib.rs

#[repr(C)]
pub struct LoftPluginCtx {
    // Opaque. Plugins must not use this field directly.
    _state: *mut (),
    // Register one native function. `name` must match naming convention
    // (n_<fn> for global, t_<N><Type>_<method> for methods).
    // `func` signature: fn(*mut Stores, *mut DbRef) — opaque raw pointers.
    pub register_fn: unsafe extern "C" fn(
        ctx: *mut LoftPluginCtx,
        name: *const std::ffi::c_char,
        func: unsafe extern "C" fn(*mut (), *mut ()),
    ),
    // Internal: pointer to the staging Vec. Plugins must not use this.
    _staged: *mut (),
}
```

`Stores` and `DbRef` are opaque to the plugin API. The trampoline (see below) casts
the raw pointers back to real types inside the interpreter's address space.

### The Registration Trampoline (`src/extensions.rs`)

```rust
// pseudocode
pub fn load_all(state: &mut State, paths: Vec<String>) {
    for path in paths { load_one(state, &path); }
}

fn load_one(state: &mut State, path: &str) {
    let lib = libloading::Library::new(path).expect("dlopen failed");
    let register: Symbol<unsafe extern "C" fn(*mut LoftPluginCtx)> =
        unsafe { lib.get(b"loft_register_v1\0") }.expect("symbol not found");

    let mut staged: Vec<(String, Call)> = Vec::new();
    let mut ctx = LoftPluginCtx { ..., _staged: &mut staged as *mut _ as *mut () };
    unsafe { register(&mut ctx) };

    for (name, call) in staged {
        state.static_fn(&name, call);
    }
    // Keep `lib` alive for the interpreter's lifetime (store in State or a global).
}

unsafe extern "C" fn trampoline_register(
    ctx: *mut LoftPluginCtx,
    name: *const c_char,
    func: unsafe extern "C" fn(*mut (), *mut ()),
) {
    let staged = unsafe { &mut *((*ctx)._staged as *mut Vec<(String, Call)>) };
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    let call: Call = unsafe { std::mem::transmute(func) };
    staged.push((name, call));
}
```

`libloading` (MIT/Apache-2.0) handles OS-level `dlopen`/`LoadLibrary` abstraction and
is added as an optional dependency under a `native-extensions` Cargo feature flag.

### Plugin Implementation Strategy

**Option A (v1 recommendation) — link against the `loft` crate directly:**

The plugin crate depends on `loft = "1.0"` (which exposes a `[lib]` target) and writes
functions using the real `Call` signature — identical to functions in `src/native.rs`.
Zero extra wrapper infrastructure needed.

**Option B (future) — thin wrapper accessors in `loft-plugin-api`:**

Expose `loft_get_i32`, `loft_put_i32`, etc. as `extern "C"` function pointers inside
`LoftPluginCtx`, to support non-Rust plugin authors. Deferred to Phase 3.

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

### Plugin Crate

```toml
# Cargo.toml
[package]
name = "loft-opengl"

[lib]
crate-type = ["cdylib"]

[dependencies]
loft = "1.0"
gl   = "0.14"
```

```rust
// src/lib.rs
use loft::database::Stores;
use loft::keys::DbRef;

fn n_gl_create_window(stores: &mut Stores, stack: &mut DbRef) {
    let title  = *stores.get::<loft::keys::Str>(stack);
    let height = *stores.get::<i32>(stack);
    let width  = *stores.get::<i32>(stack);
    // ... platform-specific window creation ...
    stores.put(stack, handle as i32);
}

fn n_gl_clear(stores: &mut Stores, stack: &mut DbRef) {
    let color = *stores.get::<i32>(stack);
    unsafe { gl::ClearColor(/* ... */); gl::Clear(gl::COLOR_BUFFER_BIT); }
}

fn n_gl_present(stores: &mut Stores, stack: &mut DbRef) { /* ... */ }
fn n_gl_destroy_window(stores: &mut Stores, stack: &mut DbRef) { /* ... */ }

#[no_mangle]
pub extern "C" fn loft_register_v1(ctx: *mut loft_plugin_api::LoftPluginCtx) {
    unsafe {
        let r = (*ctx).register_fn;
        r(ctx, b"n_gl_create_window\0".as_ptr() as _, n_gl_create_window as _);
        r(ctx, b"n_gl_clear\0".as_ptr() as _, n_gl_clear as _);
        r(ctx, b"n_gl_present\0".as_ptr() as _, n_gl_present as _);
        r(ctx, b"n_gl_destroy_window\0".as_ptr() as _, n_gl_destroy_window as _);
    }
}
```

The author builds with `cargo build --release` and places the resulting shared library in
the `native/` directory of their package.

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

## Backwards Compatibility and ABI Stability

### Stable Surface (from 1.0)

| Element | Notes |
|---------|-------|
| `Call = fn(&mut Stores, &mut DbRef)` in `src/database/mod.rs` | Plugin function signature |
| `state.static_fn(name, call)` in `src/state/mod.rs` | Only registration path |
| `stores.get::<T>(stack)` / `stores.put(stack, val)` for primitive types | Stack accessors |
| `loft_register_v1` entry-point signature | C ABI entry point |
| `LoftPluginCtx` struct layout | `repr(C)`, append-only |

Everything else in `src/` is internal and may change between minor versions. Plugin
authors must not link against `State`, `Data`, or `Parser`.

### Version Encoding in Symbol Names

The exported symbol name encodes the major version:

```
loft_register_v1    (for interpreter 1.x)
loft_register_v2    (for interpreter 2.x, if breaking ABI)
```

A v1 plugin loaded into a v2 interpreter fails at symbol resolution with a clear error
rather than a silent ABI mismatch.

### Minor-Version Compatibility

`LoftPluginCtx` is `repr(C)` and append-only. New fields may be added at the end in
minor versions. Older plugins compiled against an earlier minor version remain binary-
compatible because they never access fields beyond what they knew about.

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

### Phase 2 — Native Extension Loading (target: 1.1)

**Goal:** A Rust crate compiled to a shared library can register new native functions
with the interpreter via `use mylib;` in a loft script.

**Changes required:**

1. `Cargo.toml` — add `libloading` under optional `native-extensions` feature.
2. `src/extensions.rs` (new) — `load_all()`, `load_one()`, trampoline.
3. `src/manifest.rs` — extend to parse `native = "..."` field; resolve platform path.
4. `src/parser/mod.rs` — add `pending_native_libs: Vec<String>` to `Parser`; populate
   it in the `use` processing loop when a manifest contains `native`.
5. `src/main.rs` — call `extensions::load_all(state, p.pending_native_libs)` between
   `compile::byte_code()` and `state.execute_argv()`.
6. `src/compile.rs` — handle `#native "name"` annotation: at bytecode generation look
   up name in `state.library_names`; emit fatal diagnostic if not found.
7. `plugin-api/` (new workspace member) — publish `loft-plugin-api` with
   `LoftPluginCtx` definition.

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
