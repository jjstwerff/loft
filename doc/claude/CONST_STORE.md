
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Constant Store Design

## Context

File-scope vector constants (`QUAD = [1, 2, 3];`) crash when referenced inside
functions (P127). The IR represents vector literals as a program (`Value::Block`
with temporary `Var(0)`/`Var(1)`) that collides with the caller's variables when
inlined. Var-remapping approaches fail due to two-pass counter divergence.

This document designs a **constant store** -- a single reserved Store that holds
all pre-built constant data, populated during compilation and locked at runtime.
The design also migrates long string constants from the ad-hoc `text_code` buffer
into this unified constant store.

---

## Contents
- [Store budget](#store-budget)
- [Reserved constant: CONST_STORE](#reserved-constant-const_store)
- [What lives in the constant store](#what-lives-in-the-constant-store)
- [Opcode access to constant data](#opcode-access-to-constant-data)
- [Reference site codegen](#reference-site-codegen)
- [Implementation steps](#implementation-steps)
- [Memory-mapped constant store](#memory-mapped-constant-store)
- [WASM fast startup](#wasm-fast-startup)
- [Lifetime and safety](#lifetime-and-safety)

---

## Store budget

### Per-store overhead

| Component | Cost |
|-----------|------|
| `Store` struct (Rust) | ~112 bytes (ptr, HashSet, flags, counters) |
| Heap allocation minimum | 800 bytes (`Store::new(100)` = 100 words x 8B) |
| Header + PRIMARY record | 16 bytes |
| **Minimum per store** | **~928 bytes** |

A `[1, 2, 3]` constant holds 12 bytes of data in a 928-byte store -- 80x overhead.

### Store pool limit

`DbRef.store_nr` is `u16` -- hard cap of 65,535 stores. `u16::MAX` is the null
sentinel. The runtime warns at 30 active stores. P122 proved game loops can
exhaust the pool in seconds. Constants are never freed -- each one permanently
consumes a slot.

### Shared store wins

| Metric | Per-constant stores | Shared store |
|--------|-------------------|--------------|
| Slots consumed | N | **1** |
| 10 small constants | 9,280 bytes | ~1,048 bytes |
| 100 constants | 92,800 bytes | ~4,000 bytes |

---

## Reserved constant: `CONST_STORE`

```rust
// src/database/mod.rs
/// Store index reserved for compile-time constant data.
/// Always allocated during State::new(), locked before execution.
pub const CONST_STORE: u16 = 1;
```

### Allocation order

| Index | Purpose | Allocated in |
|-------|---------|-------------|
| 0 | Stack store (evaluation stack) | `State::new()` |
| 1 | **Constant store** (read-only data) | `State::new()` |
| 2+ | Runtime stores (structs, vectors) | `OpDatabase` at runtime |

`State::new()` allocates store 0 (stack) then immediately allocates store 1
(constants). Both exist before `byte_code()` runs. Store 1 starts empty and
is populated during compilation.

---

## What lives in the constant store

### Vector constants (P127 fix)

File-scope vector constants like `QUAD = [1, 2, 3]` are built as vector
records in the constant store during `byte_code()`. Each constant's `DbRef`
points into store 1 at a unique record position.

### Long string constants (replaces `text_code`)

Currently, string literals >= 256 bytes are stored in a separate `text_code:
Arc<Vec<u8>>` buffer and referenced by `OpConstLongText(start, size)`. The
runtime pushes a raw `Str` pointer into this buffer.

With the constant store, long strings use `Store::set_str()` which copies
bytes into store 1. At runtime, `OpConstStoreText` pushes a `Str` pointing
into the constant store's memory. This eliminates the `text_code` field.

Short strings (< 256 bytes) remain embedded in the bytecode stream via
`OpConstText` -- the bytecode buffer is already `Arc<Vec<u8>>` and the
overhead is lower than a store record for small strings.

### Future: struct constants, lookup tables

The design generalises to any constant data expressible in a Store.
Constant structs, pre-built sorted collections, or static lookup tables
could be added later using the same mechanism.

---

## Opcode access to constant data

### New opcode: `OpConstRef`

Pushes a pre-built constant's DbRef onto the evaluation stack.

```
Bytecode:  [OpConstRef] [d_nr: u32]
Stack:     ... -> ... DbRef
```

The opcode reads a definition number from the bytecode, looks up the
constant's `DbRef` from a compile-time table, and pushes it.

```rust
// fill.rs
fn const_ref(s: &mut State) {
    let d_nr = *s.code::<u32>();
    let db_ref = s.const_refs[d_nr as usize];
    s.put_stack(db_ref);
}
```

`const_refs: Vec<DbRef>` on State is indexed by definition number. Most
entries are zeroed (non-constant defs); only constant definitions have
valid DbRefs. Populated during `byte_code()`.

### Modified opcode: `OpConstLongText` -> `OpConstStoreText`

Replaces the current `text_code`-based long text opcode. Reads a store
record position from the bytecode, constructs a `Str` pointing into the
constant store.

```
Bytecode:  [OpConstStoreText] [rec: u32] [pos: u32]
Stack:     ... -> ... Str
```

```rust
// fill.rs
fn const_store_text(s: &mut State) {
    let rec = *s.code::<u32>();
    let pos = *s.code::<u32>();
    let store = &s.database.allocations[CONST_STORE as usize];
    let ptr = unsafe { store.ptr.offset((rec as isize) * 8 + (pos as isize)) };
    let len = store.get_int(rec, pos.wrapping_sub(4)) as u32;
    s.put_stack(Str { ptr, len });
}
```

### Unchanged: `OpConstText` (short strings)

Short strings (< 256 bytes) stay embedded in the bytecode. The constant
store overhead (8-byte alignment, record headers) makes it less efficient
than the 1-byte-prefix inline format for small strings.

---

## Reference site codegen

When a vector constant with a pre-built `const_ref` is referenced:

```
Parser emits:                    Bytecode generated:

__cv = null                      OpConvRefFromNull      (12B DbRef on stack)
OpDatabase(__cv, vec_tp)         OpDatabase             (allocate fresh store)
OpConstRef(d_nr)                 OpConstRef d_nr        (push constant DbRef)
OpCopyRecord(const, __cv, tp)   OpCopyRecord           (deep-copy into fresh store)
return __cv                      ... (fresh store on stack)
```

The `__cv` variable is created via `create_unique` on **both** parser passes
(one call, consistent counter). No Var collision possible -- the constant's
internal `Var(0)`/`Var(1)` are never inlined.

---

## Implementation steps

### Step 1: Reserve `CONST_STORE` constant and allocate in `State::new()`

**Files:** `src/database/mod.rs`, `src/state/mod.rs`

- Add `pub const CONST_STORE: u16 = 1;`
- In `State::new()`, after allocating store 0 (stack), allocate store 1:
  `db.database(100)` -- starts at 100 words, grows as needed
- Add `const_refs: Vec<DbRef>` field to State, initialized empty

### Step 2: Add `const_ref` field to Definition

**File:** `src/data.rs`

- Add `pub const_ref: Option<crate::keys::DbRef>` to the `Definition` struct
- Initialize to `None` in all constructors

### Step 3: Add `OpConstRef` opcode

**Files:** `src/fill.rs`, `src/state/codegen.rs`

- Add `const_ref` function to `fill.rs`
- Add entry to `OPERATORS` array, bump array size from 252 to 253
- Register the opcode name in the parser's operator table
- In `codegen.rs`, handle the new opcode in `generate()`

### Step 4: Build vector constants during `byte_code()`

**File:** `src/compile.rs`

- After the bytecode generation loop, iterate all `DefType::Constant` definitions
- For vector constants: extract literal values from the Block IR
- Build the vector in store 1 using `Stores::record_new()` / `Store::set_int()` /
  `Stores::record_finish()`
- Store the resulting `DbRef` in `definition.const_ref`
- Populate `state.const_refs` vector
- Lock store 1: `state.database.allocations[CONST_STORE].locked = true`

### Step 5: Emit `OpConstRef` + `OpCopyRecord` at reference sites

**File:** `src/parser/objects.rs`

- When `DefType::Constant` with `const_ref.is_some()` is referenced:
  emit `__cv` variable + `OpDatabase` + `OpConstRef` + `OpCopyRecord`
- Remove the failed Var-remapping code (`max_var_in_value`, `offset_vars_in_value`)
- Fallback: if `const_ref` is `None` (non-literal vector), keep current
  Block-inlining behavior

### Step 6: Migrate long strings to constant store

**Files:** `src/state/codegen.rs`, `src/fill.rs`, `src/state/mod.rs`

- In `gen_text()`, for strings >= 256 bytes: instead of appending to `text_code`,
  call `Store::set_str()` on the constant store
- Emit `OpConstStoreText(rec, pos)` instead of `OpConstLongText(start, size)`
- Add `const_store_text` to `OPERATORS` (array size -> 254)
- Remove `text_code: Arc<Vec<u8>>` from State (deferred -- can keep for
  backward compatibility initially)

### Step 7: Un-ignore tests and verify

**Files:** `tests/issues.rs`, `doc/claude/PROBLEMS.md`

- Both P127 tests are no longer `#[ignore]`d (un-ignored when the const store landed)
- Verify: `cargo test -p loft --test issues p127` passes in debug mode
- Verify: `cargo test -p loft --test issues` full suite passes
- Verify: `cargo test --release` full suite passes
- Verify: `cargo clippy` clean

### Step 8: Documentation

**Files:** `doc/claude/CONST_STORE.md`, `CLAUDE.md`

- Save this design as `doc/claude/CONST_STORE.md`
- Add entry to CLAUDE.md documentation index
- Update PROBLEMS.md to mark P127 fixed

---

## Memory-mapped constant store

### Motivation

The constant store is read-only and its contents are deterministic -- the same
source program always produces the same constant data. On systems that support
memory-mapped files, the constant store can be written to disk alongside the
program (e.g., `program.loft` -> `program.loftc` or `.loft/constants.store`)
and mapped on subsequent runs, skipping reconstruction entirely.

This is a stepping stone toward a full bytecode cache (`.loftc`), which the
CLAUDE.md documentation index already references as a deferred design.

### Platform support

| Platform | mmap available | Approach |
|----------|---------------|----------|
| Native (Linux/macOS/Windows) | Yes (`mmap` feature, `mmap-storage` crate) | File-backed `Store::open()` |
| WASM (browser) | No | Heap-backed `Store::new()` -- build every run |
| WASM (WASI) | Possible (future) | Could use virtual FS |

The `mmap` feature is already default-enabled for native builds and absent
from WASM builds (`Cargo.toml:27`). The constant store uses the same
feature gate.

### File format

The constant store file is a raw Store image -- the same byte layout that
`Store::open()` already reads. It starts with the `SIGNATURE` (`0x53746f31`
= "Sto1") header, followed by records exactly as they appear in memory.
No versioning beyond the signature; if the store format changes, the cache
is invalidated.

### Cache invalidation

The constant store cache is valid only if:
1. The source file(s) haven't changed since the cache was written
2. The loft interpreter version matches (store layout may change between versions)

**Strategy:** store a hash (SHA-256 of source + version string) in the first
record of the constant store. On load, compute the hash and compare. If
mismatched, discard the cache and rebuild.

Alternatively, use file modification timestamps (simpler, less robust) or
always rebuild and let the OS page cache handle repeated runs (simplest,
defers mmap to the bytecode cache milestone).

### Lifecycle with mmap

```
First run:
  State::new()
    -> allocate CONST_STORE as heap Store
  byte_code()
    -> populate CONST_STORE with vector/string constants
    -> lock store
  execute()
    -> opcodes read from locked CONST_STORE
  exit
    -> write CONST_STORE to disk if mmap feature enabled

Subsequent runs (cache hit):
  State::new()
    -> CONST_STORE = Store::open("program.loftc")  (mmap, zero-copy)
    -> verify hash
    -> lock store
  byte_code()
    -> skip constant building (const_ref DbRefs read from stored definitions)
  execute()
    -> opcodes read from mmap'd CONST_STORE -- pages faulted in on demand
```

### Implementation notes

- `Store::open()` already handles mmap-backed stores with `MmapStorage`
- The mmap'd store is `locked = true` and has `file: Some(MmapStorage)` --
  `Drop` skips deallocation (line 103-106 in store.rs)
- `Str` pointers into the constant store remain valid because the mmap
  mapping persists for the program lifetime
- Parallel workers clone stores for thread safety, but the constant store
  is read-only -- it can be shared without cloning (add exception to
  `clone_for_worker`)

---

## WASM fast startup

In WASM there is no mmap, but there's a more impactful optimization path.

### Current WASM startup cost

```
compile_and_run() -- called on every page load / playground run
  1. Populate VirtFS with include_str! source text        ~0 ms (static data)
  2. Parse 5 stdlib files x 2 passes                      ~50-100 ms
  3. Parse graphics lib files x 2 passes                   ~20-50 ms
  4. Parse user file x 2 passes                            ~5-20 ms
  5. Scope analysis                                        ~5 ms
  6. Bytecode generation                                   ~10 ms
  7. Execute                                               variable
```

Steps 2-3 are the bottleneck: the standard library is re-parsed from source
text on every invocation. The constant store helps step 7 but doesn't touch
the real cost.

### Fastest possible startup: pre-compiled WASM image

The ultimate optimization is to include the compiled standard library
(Data definitions + bytecode + constant store) as static data in the WASM
binary, not the source text. The startup would be:

```
compile_and_run()
  1. Deserialize pre-built Data + bytecode + constants     ~1-5 ms
  2. Parse user file only (2 passes)                       ~5-20 ms
  3. Scope analysis (user code only)                       ~1 ms
  4. Bytecode generation (user code only)                  ~2 ms
  5. Execute                                               variable
```

This requires serializing `Data`, `State.bytecode`, and the constant store
into a binary format that can be `include_bytes!` into the WASM binary.

### The constant store enables this path

With constants in a Store (a flat byte buffer), serialization is trivial --
the Store's `ptr` buffer is already the serialized form. Deserializing is
just pointing at the static bytes. Without the constant store, vector
constants live as IR trees in the Data structure, which would need a complex
serializer for the recursive `Value` enum.

### Implementation levels

| Level | What | Startup cost | Effort |
|-------|------|-------------|--------|
| 0 (current) | Re-parse everything from source | ~100-200 ms | -- |
| 1 (constant store) | Constants pre-built, still parse stdlib | ~90-180 ms | Low |
| 2 (pre-compiled stdlib) | Data + bytecode + constants as static binary | ~10-30 ms | Medium |
| 3 (incremental) | Cache user compilation in IndexedDB | ~5-20 ms repeat | High |

Level 1 is the P127 fix. Level 2 is the big win -- it requires the constant
store as a prerequisite because the Store byte buffer is the natural
serialization format for constant data.

### Static byte inclusion (level 2)

```rust
// Generated at build time by a build.rs script or offline tool
const STDLIB_DATA: &[u8] = include_bytes!("../generated/stdlib.bin");
const STDLIB_CONST_STORE: &[u8] = include_bytes!("../generated/constants.store");
const STDLIB_BYTECODE: &[u8] = include_bytes!("../generated/bytecode.bin");
```

The constant store bytes can be used directly as a Store buffer by setting
`Store.ptr` to the static data address (zero-copy, like mmap). The `borrowed`
flag (A14.1) prevents `Drop` from deallocating static memory.

---

## Lifetime and safety

- Store 1 is **never freed** -- persists for the program's lifetime
- **Locked** after construction (`store.locked = true`) -- writes panic in
  debug, are no-ops in release
- `OpCopyRecord` deep-copies into fresh runtime stores -- mutations to copies
  don't affect the constant
- No `OpFreeRef` for store 1 -- it has no runtime refcount
- Parallel workers can safely read from the locked store (read-only = thread-safe)
- The constant store does not participate in the debug-mode "Database N not
  correctly freed" exit check -- it is expected to remain allocated
- When mmap-backed, the OS manages page lifetime; the store outlives the
  program if other processes map the same file (harmless -- read-only data)

---

## Phasing and status

1. **Phase A** (P127 fix): **Done.** Heap-backed constant store, vector
   constants pre-built in `byte_code()`, `OpConstRef` opcode, long strings
   in CONST_STORE, `text_code` buffer removed.

2. **Phase D** (bytecode cache): **Done.** `.loftc` file format caches
   bytecode + stores + const_refs + function positions. SHA-256 cache key
   from source content + version. `byte_code_with_cache()` skips the
   `def_code()` loop on cache hit. `src/cache.rs` module.

3. **Phase B** (mmap): **Deferred.** Cache files are 5-10 KB — mmap
   overhead (syscall + page tables) exceeds memcpy savings at this size.
   Becomes worthwhile when Phase C embeds a large stdlib cache.

4. **Phase C** (WASM pre-compiled stdlib): **Deferred.** The bottleneck is
   re-parsing the stdlib from source (~100ms). Skipping this requires
   serializing the `Data` struct (definitions, types, attributes) — not
   just bytecode. The `Data` struct has 130+ public members across
   recursive enums (`Value`, `Type`), requiring either serde or hand-written
   binary serialization. Estimated effort: Medium-High. The `include_bytes!`
   approach from the design works once serialization exists.
