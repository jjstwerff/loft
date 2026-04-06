
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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
- [REGISTRY.md](REGISTRY.md) — Package registry design
