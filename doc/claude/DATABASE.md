
# Database and Storage Layer

## Overview

The runtime data layer is split across multiple source files that together implement a typed, heap-allocated, store-based memory model:

## Contents
- [Overview](#overview)
- [Store — Raw Heap Allocator (`src/store.rs`)](#store--raw-heap-allocator-srcstorers)
- [Stores — Type Schema + Multi-Store Manager (`src/database/`)](#stores--type-schema--multi-store-manager-srcdatabase)
- [DbRef, Key, Content — Universal Pointer and Key Types (`src/keys.rs`)](#dbref-key-content--universal-pointer-and-key-types-srckeysrs)
- [Vector Operations (`src/vector.rs`)](#vector-operations-srcvectorrs)
- [Red-Black Tree (`src/tree.rs`)](#red-black-tree-srctreers)
- [Open-Addressing Hash Table (`src/hash.rs`)](#open-addressing-hash-table-srchashrs)
- [Spatial Index (`src/radix_tree.rs`)](#spatial-index-srcradix_treers)
- [How the Layers Fit Together](#how-the-layers-fit-together)

---

| File | Role |
|---|---|
| `src/store.rs` | Raw word-addressed heap allocator (`Store`) |
| `src/database/mod.rs` | `Stores` constructor, basic get/put, parse-key helpers |
| `src/database/types.rs` | Type-building: `structure`, `field`, `finish`, `sorted`, `hash`, sizes |
| `src/database/allocation.rs` | Store claim/free, `copy_claims*`, `clone_for_worker` |
| `src/database/search.rs` | Find/iterate: `find`, `find_vector`, `find_index`, `next`, `remove` |
| `src/database/structures.rs` | Record construction, field get/set, `vector_add`, struct parsing |
| `src/database/io.rs` | File I/O: `read_data`, `write_data`, `get_file`, `get_dir`, `get_png` |
| `src/database/format.rs` | Display/formatting: `show`, `dump`, `rec`, `path` |
| `src/keys.rs` | Universal store pointer (`DbRef`), key descriptors, compare/hash |
| `src/vector.rs` | Dynamic arrays: by-value (Vector), by-reference (Array/Ordered) |
| `src/tree.rs` | Left-leaning red-black tree for `sorted<T>` / `index<T>` |
| `src/hash.rs` | Open-addressing hash table for `hash<T>` / `index<T>` by hash |
| `src/radix_tree.rs` | Store-backed binary PATRICIA/radix tree over an abstract bit-key oracle (backs `spatial<T>`) |
| `src/radix_db.rs` | DB↔tree bridge: Morton/Z-order key interleaving + range/proximity primitives for `spatial<T>` |
| `src/spatial.rs` | Morton-coded near/within/nearest geometry algorithms used by `src/radix_db.rs` |

---

## Store — Raw Heap Allocator (`src/store.rs`)

See `src/store.rs` module docs for memory layout, signed size headers, and
free-block allocation.  See `src/keys.rs` module docs for `DbRef`, `Str`,
`Key`, and `Content` types.

### Durable stores (`Store::open_durable`) — @PLN43

A durable store is a normal mmap-backed `Store` plus a 40-byte `.dmeta`
sidecar file alongside the main store file.  The sidecar holds a signature
(`"DStoreV1"`), tier id, CRC32 over the main file, and a `last_clean_ns`
timestamp.  On clean drop the sidecar is rewritten atomically (`write tmp
→ fsync → rename`); on `kill -9` the sidecar stays stale, and the next
open detects corruption and invokes the consumer's rebuild callback.

The main store file is bit-for-bit identical to a non-durable store —
durability is a metadata layer, not a payload-layout change.  Existing
record/claim/resize code paths are untouched.

Three tiers are planned; phase 01 (the first PR slice on the
`store-durable-phase1` branch) ships **Tier 1 — `IntegrityOnly`** only:

| Tier | Mode | Hot-path cost | Loss bound | Consumer |
|---|---|---|---|---|
| 1 | `IntegrityOnly` | None (only msync on clean drop) | Everything since last clean drop | `personal/training` port (initial), `@PLN42` indexer (when phase 08 lands) |
| 2 | `SnapshotEvery(interval)` (planned, phase 02) | One msync per interval | One interval | TTT v5 multiplayer (`plans/future/32-…`) |
| 3 | `WAL` (planned, phase 03) | fsync per record, amortised by group-commit window | Zero for committed writes | @PLN6 audience demo |

API surface:

```rust
use loft::store::{Store, DurabilityMode};

let store = Store::open_durable(
    path,
    DurabilityMode::IntegrityOnly {
        on_corruption: Box::new(|p| rebuild_from_source(p)),
    },
)?;
```

**Fresh-file semantics.**  When the main file doesn't exist yet,
`on_corruption` fires with `TailMarkerMissing` and is expected to
"create empty + populate from authoritative sources" — not "repair
existing file."  After the callback returns successfully, `open_durable`
captures a fresh sidecar and retries once.

**Drop-on-panic is by design.**  A panic between open and clean drop
skips the sidecar write → next open detects corruption → callback fires.
This is what makes Tier 1 cheap.  Do not use Tier 1 for data that cannot
be re-derived from authoritative sources; use Tier 2 or Tier 3 instead.

Full design + implementation history:
[`doc/claude/plans/43-loft-store-durable/`](plans/43-loft-store-durable/README.md).

### Working-set store loader + layout sidecar (`.dschema`) — @PLN97 arc G

`store_persist_bind(collection, path)` binds a keyed collection to a durable
store and writes a `<path>.dschema` **layout-identity sidecar** beside it
(`src/schema_sidecar.rs`: `LayoutIdentity` = the `layout_algo_hash` + per-type
layout dump). The **working-set loaders** — `store_load_key` / `store_load_keys`
/ `store_load_key_text` (hash point lookups) and `store_load_range` (sorted
range) — materialise only the entries a query touches, reading just the pages
those touch from a **local file or an `http(s)://` Range server**
(`src/paged_reader.rs`), then relocate each matched entry's heap graph into a
sound local store (`store_verify` proves the copy; every copyable field shape is
handled, `vector<text>`/`vector<vector>` safely refused).

Before any schema-derived read the loader checks the `.dschema`: a store whose
recorded layout differs from the loading program's collection type is REFUSED
(the **layout-identity gate**) rather than misread as foreign bytes; an absent
sidecar (a legacy store) falls back to the `store_verify` backstop. Set
`LOFT_LOADER_STATS=1` to observe `bytes_fetched` vs file size.

These loaders work **on every target, the browser included** (loft#678). Only the
byte source differs, and it is the sole thing that does: a native build issues
`Range` GETs over `ureq`, while `--html` issues them through the asyncify
`fetch()` host import that `store_load_url_trusted` already uses
(`net::fetch_range`, behind `PageProvider`). Everything above that seam — the
paged reader, the traversal, the relocating copy — is one code path, so a browser
load reads the same pages a native one does: `tests/paged_browser.rs` runs a real
`--html` bundle against a 3.8 MB store and pins the cost at a bounded handful of
64 KiB pages (~7% of the image), the same fraction the native path reports.
The build-time availability is the `paged_store` cfg (`build.rs`): the
`remote-store` feature, or the browser target, which cannot use `ureq` at all.

**Every paged refusal is reported on stderr** (`store loader: refusing <path> — …;
loaded NOTHING (a refusal, not an absent key)`). These loaders signal failure as
`false` / `0`, which is exactly what an ABSENT KEY looks like, so a silent refusal
reads as missing data and hides an unsupported shape. The refusal reasons are: the
layout gate above; an unopenable source; a store with no recorded type; an entry
with a field the working-set copy cannot relocate (`vector<text>` /
`vector<vector>` — see `store_load_vectext_refuse.loft`); and **a collection
declared as a struct FIELD**, whose bound store records the *wrapper struct* as its
type so no hash/sorted root is found ([#632](https://github.com/loft-lang/loft/issues/632)
— declare it as an annotated local `h: hash<T[k]> = []` for paged loads, or read it
whole with `store_load`, which carries the field form fine). Pinned by
`store_load_field_refusal.loft`.

Full design:
[`plans/97-layout-contract/REMOTE_STORE_LOADER.md`](plans/97-layout-contract/REMOTE_STORE_LOADER.md);
the layout contract itself: [`formal/layout.md`](formal/layout.md).

---

## Stores — Type Schema + Multi-Store Manager (`src/database/`)

### Stores struct

```rust
pub struct Stores {
    pub types: Vec<Type>,           // all registered types
    names: HashMap<String, u16>,    // type name → index
    allocations: Vec<Store>,        // one Store per allocation context
    pub max: u16,                   // number of registered types
}
```

`Stores` owns the complete type schema and all live stores. The `types` vector is append-only at runtime; type indices (`u16`) are stable.

### Fixed Base Type IDs

The following type indices are permanently fixed:

| ID | Type |
|---|---|
| 0 | `integer` (32-bit signed) |
| 1 | `long` (64-bit signed) |
| 2 | `single` (32-bit float) |
| 3 | `float` (64-bit float) |
| 4 | `boolean` |
| 5 | `text` (string) |
| 6 | `character` |

Types 0–6 are registered at construction time and never relocated.

### Type struct

```rust
pub struct Type {
    pub name: String,
    pub parts: Parts,
    pub keys: Vec<Key>,      // key fields for sorted/hash/index
    pub size: u32,           // byte size of one record
    pub align: u32,          // alignment requirement
    pub linked: bool,        // has back-reference (tree backward links)
    pub complex: bool,       // contains non-trivial types (strings, refs)
}
```

### Parts enum

`Parts` describes the runtime layout and category of a type:

| Variant | Description |
|---|---|
| `Base` | Primitive (integer, long, float, boolean, text, character) |
| `Struct(Vec<Field>)` | Named fields with offsets |
| `Enum(Vec<(u16, String)>)` | Discriminated union; entries are (discriminant, name) |
| `EnumValue(u8, Vec<Field>)` | One variant of an enum (discriminant + fields) |
| `Byte(i32, bool)` | Byte-sized integer; `bool` = signed |
| `Short(i32, bool)` | 16-bit integer; `bool` = signed |
| `Vector(u16)` | Dynamic by-value array of element type `u16` |
| `Array(u16)` | Dynamic by-reference array of element type `u16` |
| `Sorted(u16, Vec<(u16,bool)>)` | Red-black tree ordered by key fields; `bool` = ascending |
| `Ordered(u16, Vec<(u16,bool)>)` | Ordered array (binary search) by key fields |
| `Hash(u16, Vec<u16>)` | Open-addressing hash table; field indices as hash keys |
| `Index(u16, Vec<(u16,bool)>, u16)` | Combo: sorted tree + hash table for a single collection |
| `Radix(u16, Vec<u16>)` | Spatial index for `spatial<T[x,y]>` / `spatial<T[x,y,z]>` — Morton/Z-order radix tree, 1–3 coordinate axes (renamed from `Spatial`) |

### Field struct

```rust
pub struct Field {
    pub name: String,
    pub type_nr: u16,    // index into Stores::types
    pub offset: u32,     // byte offset within the record
}
```

### Stores API

| Method | Description |
|---|---|
| `new() -> Stores` | Create empty stores; registers base types 0–6 |
| `structure(name) -> u16` | Register a new struct type; returns its index |
| `field(type_nr, name, field_type, offset)` | Add a field to an existing struct type |
| `enumerate(name) -> u16` | Register a new enum type |
| `value(enum_nr, discriminant, name)` | Add a variant to an enum type |
| `finish()` | Seal schema registration (calculates sizes, alignment) |
| `allocate() -> u16` | Create a new `Store`; returns its index |
| `store(nr) -> &Store` | Borrow store by index |
| `mut_store(nr) -> &mut Store` | Mutably borrow store by index |
| `byte(min: i32, nullable: bool) -> u16` | Register or get a byte integer type; name = `"byte"` for (0,false) or `"byte<min,nullable>"` |
| `short(min: i32, nullable: bool) -> u16` | Register or get a 16-bit integer type; name = `"short<min,nullable>"` |
| `database(size: u32) -> DbRef` | Allocate a new top-level store slot; `size=u32::MAX` means no record claim |
| `free(db: &DbRef)` | Release a top-level store slot (LIFO order required) |
| `null() -> DbRef` | Allocate an empty store slot (calls `database(u32::MAX)`) |
| `read_data(r, tp, little_endian, data)` | Serialize a stored value to raw bytes (for writing to binary file) |
| `write_data(r, tp, little_endian, data)` | Deserialize raw bytes into a stored value (from reading a binary file) |
| `lock_store(r: &DbRef)` | Lock the store that owns `r` (no-op for null refs) |
| `unlock_store(r: &DbRef)` | Unlock the store that owns `r` |
| `is_store_locked(r: &DbRef) -> bool` | Return whether the store that owns `r` is locked |

### Constant store (`CONST_STORE`)

Store index `1` is reserved for compile-time constant data:

```rust
// src/database/mod.rs
pub const CONST_STORE: u16 = 1;
```

Allocated by `State::new()` immediately after the stack store (index 0)
and before any runtime store.  Populated during `byte_code()` and
**locked** before `execute()` runs.

| Index | Purpose | Allocated in |
|---|---|---|
| 0 | Stack store (evaluation stack, record in store 1000 historical alias) | `State::new()` |
| 1 | **Constant store** (read-only data) | `State::new()` |
| 2+ | Runtime stores (structs, vectors) | `OpDatabase` at runtime |

**What lives in `CONST_STORE`** (@PLN82 Phase A, 2026):

- **Vector constants** — file-scope `QUAD = [1, 2, 3];` is built as
  a vector record in `CONST_STORE` during `byte_code()`.  Each
  constant's `DbRef` is recorded in `Definition.const_ref` and
  cached in `State.const_refs[d_nr]`.  Closes P127 (Var-collision
  on inlined vector-literal IR).
- **Long string constants** (>= 256 bytes) — `Store::set_str()`
  copies bytes into `CONST_STORE`; `OpConstStoreText` reads the
  `Str` pointer at runtime.  Replaced the ad-hoc `text_code:
  Arc<Vec<u8>>` buffer that previously lived on `State`.
- Short strings (< 256 bytes) stay embedded inline in the bytecode
  via `OpConstText` — record-header overhead exceeds the inline
  format's 1-byte-prefix cost at small sizes.

**Reference-site codegen** for vector constants:

```text
__cv = null
OpDatabase(__cv, vec_tp)             # allocate fresh runtime store
OpConstRef(d_nr)                     # push the constant's DbRef
OpCopyRecord(const, __cv, tp)        # deep-copy into __cv's store
return __cv                          # caller owns __cv (mutable)
```

Each reference site allocates a fresh runtime store and deep-copies
the constant record in.  Mutations to the copy never affect the
original; the copy participates in normal `OpFreeRef` lifetime.

**Lifetime + safety**:

- `CONST_STORE` is **never freed** — persists for the program's lifetime.
- **Locked** after construction (`store.locked = true`) — writes panic
  in debug, are no-ops in release.
- No `OpFreeRef` for `CONST_STORE` — it has no runtime refcount.
- Parallel workers may read the locked store directly without cloning
  (read-only = thread-safe).
- Excluded from the debug-mode "Database N not correctly freed" exit
  check — expected to remain allocated.

See [INTERMEDIATE.md § Bytecode State](INTERMEDIATE.md#bytecode-state--srcstate)
for `State.const_refs`'s role in `OpConstRef` dispatch.

For deferred follow-ups (mmap-backed cache file; WASM
pre-compiled stdlib including `CONST_STORE` as static bytes via
`include_bytes!`) see
[`plans/82-const-store/`](plans/82-const-store) §
Memory-mapped + WASM fast startup.

### Store Locking via `Stores`

`Stores` exposes three methods that wrap the per-`Store` lock flag:

```rust
pub fn lock_store(&mut self, r: &DbRef)       // enable write-protection
pub fn unlock_store(&mut self, r: &DbRef)     // remove write-protection
pub fn is_store_locked(&self, r: &DbRef) -> bool
```

All three methods silently ignore null refs (`r.rec == 0`) and out-of-range store indices so they are safe to call unconditionally from generated code.

These methods are surfaced to loft code via two native functions registered in `src/native.rs`:

| Native function | Loft declaration (`default/01_code.loft`) |
|---|---|
| `n_get_store_lock` | `fn get_store_lock(r: reference) -> boolean` |
| `n_set_store_lock` | `fn set_store_lock(r: reference, locked: boolean)` |

The `reference` parameter type accepts any concrete `Reference` type at call sites thanks to the type-compatibility check in the parser.

### `d#lock` Syntax

Loft code interacts with store locks through the `#lock` pseudo-field syntax:

```loft
c#lock        // read: boolean — true if the store is locked
c#lock = true // write: lock the store
```

**Parser routing** (`src/parser/collections.rs` and `src/parser/expressions.rs`):
- `iter_op` detects the `lock` keyword and emits `n_get_store_lock(c)` for reads.
- `towards_set` converts a `n_get_store_lock` call into `n_set_store_lock` for the left-hand side of an assignment.
- `parse_assign` validates the assignment: only a literal `true` or `false` is accepted (not an expression); assigning `false` to a `const` variable or argument is a compile-time error.

**Constraints enforced by the compiler**:
1. `d#lock` is only valid on `Reference` or `Vector` typed variables; any other type is a diagnostic error.
2. The right-hand side must be a constant boolean (`true` or `false`).
3. `d#lock = false` on a `const` variable is a compile-time error.

### `const` Variables and Arguments

The `const` keyword can be applied to local variable declarations and function arguments:

```loft
const d = Counter { value: 42 }   // local const variable
fn read_value(self: const Counter) // const argument
```

**Semantics**:
- The compiler marks the variable with `const_param`, preventing reassignment via `OpSet` in generated bytecode.
- In **debug builds only** (`#[cfg(debug_assertions)]`): the store is automatically locked immediately after initialisation (local `const`) or at the start of the function body (const arguments). This turns any accidental write into a runtime panic.
- In **release builds**: the lock is _not_ set automatically; only explicit `d#lock = true` in loft code locks the store.
- Reading `c#lock` on a const variable emits a runtime `n_get_store_lock` call. In a debug build this always returns `true` because the store was auto-locked; in release it returns whatever the current flag is.

**Implementation locations**:
- Auto-lock for local `const`: `expression()` in `src/parser/expressions.rs` — after the initialising assignment is compiled, inserts a `n_set_store_lock` call under `#[cfg(debug_assertions)]`.
- Auto-lock for const arguments: `parse_code()` in `src/parser/expressions.rs` — inserts lock calls at the start of the function body for every argument that is both an argument and const.

### Binary File I/O: `read_data` and `write_data`

`read_data` reads from a `DbRef` into a `Vec<u8>` (for writing to a binary file). `write_data` reads from a `&[u8]` into a `DbRef` (for reading from a binary file).

**Critical design constraint**: temp variables used for file I/O (created by `write_to_file` / `read_from_file` in `parser.rs`) are **always stored as full i32 on the stack** (`Context::Variable` always allocates 4 bytes for all integer types). This means `read_data`/`write_data` for `Parts::Byte` and `Parts::Short` must use `get_int`/`set_int`, NOT `get_byte`/`get_short`.

The reason: `get_short(rec, pos, min)` reads the null-sentinel-encoded storage (`stored_u16 = value − min + 1`) and returns the actual value. But a temp var's slot holds a raw i32 (no encoding offset). Using `get_short` on an i32 temp var returns `raw_u16 − 1`, which is off by one.

| Part type | `read_data` (store → bytes) | `write_data` (bytes → store) |
|---|---|---|
| `Base(0)` / `Base(6)` (integer/char) | `get_int` → 4 bytes | `set_int` from 4 bytes |
| `Base(1)` (long) | `get_long` → 8 bytes | `set_long` from 8 bytes |
| `Base(2)` (single) | `get_single` → 4 bytes | `set_single` from 4 bytes |
| `Base(3)` (float) | `get_float` → 8 bytes | `set_float` from 8 bytes |
| `Base(4)` (boolean) | `get_byte(_, _, 0) as u8` → 1 byte | `set_byte(_, _, 0, data[0])` |
| `Base(5)` (text) | `get_str` → UTF-8 bytes | `set_str` from UTF-8 bytes |
| `Parts::Byte(_, _)` | `get_int` → truncate to u8 → 1 byte | `set_int(i32::from(data[0]))` |
| `Parts::Short(_, _)` | `get_int` → truncate to i16 → 2 bytes | `set_int(i32::from(i16::from_le/be_bytes))` |
| `Parts::Struct(fields)` | recurse for each field | recurse for each field |
| `Parts::Enum(_)` | `get_byte` → 1 byte | `set_int(i32::from(data[0]))` |
| `Parts::Vector(elem_tp)` | iterate elements, recurse per element | `vector_append` + `write_data` per element + `vector_finish` |

**Note**: `Parts::Byte`/`Parts::Short` in `read_data`/`write_data` are designed for temp variable contexts (i32 layout). Using these with actual 1/2-byte struct fields would produce incorrect results. Struct serialization via `Parts::Struct` recursion is not yet fully tested.

---

## DbRef, Key, Content — Universal Pointer and Key Types (`src/keys.rs`)

### DbRef

```rust
pub struct DbRef {
    pub store_nr: u16,   // which Store in Stores::allocations
    pub rec: u32,        // word offset of the record within the store
    pub pos: u32,        // byte offset within the record (field position)
}
```

`DbRef` is the universal runtime pointer. It encodes a complete address: which store, which record, and which field offset within that record. A null reference is `store_nr == 0 && rec == 0`.

### Key

```rust
pub struct Key {
    pub type_nr: i8,    // positive = ascending, negative = descending; magnitude = type code
    pub position: u16,  // byte offset of this field within the record
}
```

Type codes for `Key::type_nr`:

| Code | Type |
|---|---|
| 1 | `integer` (32-bit) |
| 2 | `long` (64-bit) |
| 3 | `single` (32-bit float) |
| 4 | `float` (64-bit float) |
| 6 | `text` (string reference) |
| other | byte-sized field |

Negative `type_nr` means descending order for that key field.

### Content

```rust
pub enum Content {
    Long(i64),
    Float(f64),
    Single(f32),
    Str(Str),
}
```

Used as the return type of `get_key` when extracting a key value from a record for comparison or hashing.

### Str

```rust
pub struct Str {
    pub ptr: *const u8,
    pub len: u32,
}
```

Zero-copy string reference into store memory. Lifetime is tied to the store; no heap allocation.

### Key Functions

| Function | Description |
|---|---|
| `compare(store, rec, other, keys) -> Ordering` | Compare two records by a list of `Key` fields |
| `key_compare(store, rec, key_vals, keys) -> Ordering` | Compare a record against extracted `Content` values |
| `hash(store, rec, keys) -> u64` | Hash a record by its key fields |
| `key_hash(key_vals, keys) -> u64` | Hash a list of `Content` values using the same algorithm |
| `get_key(store, rec, key) -> Content` | Extract one key field value from a record |
| `store(db_ref) -> &Store` | Resolve a `DbRef` to a `&Store` (shared borrow) |
| `mut_store(db_ref) -> &mut Store` | Resolve a `DbRef` to a `&mut Store` |

---

## Vector Operations (`src/vector.rs`)

Three distinct collection layouts share the vector source file.

### By-Value Vector (`Vector` / `Parts::Vector`)

Elements are stored inline within the vector record:

```
word 0: claimed size (in words, same as Store header)
word 1: length (element count)
word 2+: element data (size bytes per element, packed)
```

Initial capacity claim: `(11 * element_size + 15) / 8` words — room for approximately 11 elements before the first resize.

| Function | Description |
|---|---|
| `vector_add(store, rec, size) -> u32` | Append one element slot; returns byte offset of new element |
| `vector_remove(store, rec, pos, size)` | Remove element at byte position `pos`; shifts remaining elements |
| `vector_next(store, rec, pos, size) -> u32` | Advance byte position by `size`; returns next byte offset |
| `vector_step(store, rec, index, size) -> u32` | Advance to next element index (forward) |
| `vector_step_rev(store, rec, index, size) -> u32` | Advance to previous element index (reverse) |
| `vector_length(store, rec) -> u32` | Return element count |

### Narrow vector elements

Vectors of narrow integer aliases (`vector<u8>` / `vector<u16>` /
`vector<i8>` / `vector<i16>` / `vector<i32>` / `vector<u32>`)
honour the alias's `forced_size` so that, e.g., `vector<i32>`
stores 4 bytes per element rather than 8.

The encoding for vector elements differs from struct fields:

- **Struct field** `Parts::Short` encodes `raw = val - min + 1`,
  reserving raw 0 as the null sentinel.
- **Vector element** `Parts::ShortRaw` (added 2026-04-22 alongside
  the rest of @PLAN02) encodes `raw = val - min` directly.

The divergence is required because `vector_add` raw-byte-copies
element bytes from source to destination — the +1 offset of
`Parts::Short` would cause read/write mismatch.  `Parts::Byte`
and `Parts::Int` are direct-encoded already and need no separate
"raw" variant; only the 2-byte case needed `Parts::ShortRaw`.
The 8-byte fallback (`Parts::Long`) is also direct.

Public surface (in `src/data.rs`):

| API | Returns | Use |
|---|---|---|
| `IntegerSpec::vector_narrow_width()` | `Option<u8>` (1 / 2 / 4, or `None` for the 8-byte fallback) | "Should this vector element narrow?" |
| `Data::narrow_vector_content(content)` | content type with `forced_size` applied | Wrap a content type before calling `database.vector(...)` |

**Compiler-contributor gotcha**: `typedef.rs::fill_database`
walks ONLY struct definitions.  Local-variable / parameter /
return-type vector registration happens at every
`database.vector(c_tp)` call site in `src/parser/`.  Both paths
must call `narrow_vector_content()` on their content type before
registering, or narrowing only takes effect for struct fields.
See [INTERMEDIATE.md § Integer Storage Size](INTERMEDIATE.md#integer-storage-size)
for the per-variant table and the rule selection.

### Sorted By-Value Vector (`Parts::Ordered`)

Same record layout as Vector. Elements are kept in sorted order via binary search insertion.

| Function | Description |
|---|---|
| `sorted_find(store, rec, size, keys, vals) -> (u32, bool)` | Binary search; returns (byte_offset, found) |
| `sorted_add(store, rec, size, keys) -> u32` | Append then insertion-sort to correct position; returns offset |
| `sorted_finish(store, rec, size, keys)` | Insertion-sort the last added element into correct position |

### By-Reference Array (`Array` / `Parts::Array` / `Parts::Sorted`)

Stores 4-byte record references (offsets into a separate store) rather than inline data. Used for `sorted<T>` where `T` is a struct stored elsewhere.

| Function | Description |
|---|---|
| `ordered_find(store, rec, ref_store, keys, vals) -> (u32, bool)` | Binary search over references; dereferences into `ref_store` for comparison |
| `array_add(store, rec, ref_rec) -> u32` | Append a reference; returns slot offset |
| `array_remove(store, rec, pos)` | Remove reference at slot `pos`; shifts remaining |

---

## Red-Black Tree (`src/tree.rs`)

Used for `sorted<T>` and `index<T>` collections that need O(log n) insert/delete/find with O(1) iteration via backward links.

### Node Layout

Each node is a record in a `Store`. The tree-management fields are stored at a fixed offset (`fields`) within the record, after any user data fields:

```
offset fields+0: LEFT  (i32) — positive = left child rec, negative = backward link to parent
offset fields+4: RIGHT (i32) — positive = right child rec, negative = backward link to parent
offset fields+8: FLAG  (i32) — 1 = red, 0 = black
```

User data fields occupy bytes 0 .. `fields-1`.

### Backward Links

Negative values in LEFT/RIGHT are backward links to the parent node (stored as the negated rec value). This enables O(1) `next` and `previous` without a stack or parent pointer field:

- From any node, follow backward links up until you come from a left child → that ancestor is `next`.
- `previous` is symmetric (came from a right child).
- This is the key structural invariant: the tree simultaneously encodes the parent relationship for traversal without extra memory.

### Limits

```rust
const RB_MAX_DEPTH: usize = 30;
```

Maximum tree depth of 30 is sufficient for up to ~2^15 nodes in a balanced red-black tree.

### Key Functions

| Function | Description |
|---|---|
| `find(store, root, keys, vals) -> (u32, bool)` | Search; returns (rec, found) |
| `add(store, root, rec, keys) -> u32` | Insert `rec`; rebalances; returns new root |
| `remove(store, root, rec, keys) -> u32` | Delete `rec`; rebalances; returns new root |
| `first(store, root) -> u32` | Leftmost node (minimum key) |
| `last(store, root) -> u32` | Rightmost node (maximum key) |
| `next(store, rec) -> u32` | In-order successor via backward links; 0 if none |
| `previous(store, rec) -> u32` | In-order predecessor via backward links; 0 if none |
| `validate(store, root, keys)` | Debug: verify RB invariants and backward-link consistency |

### Rebalancing

Standard left-leaning red-black tree rotations and color-flips. `add` performs a top-down split on the way down then a bottom-up fixup on the way back up. `remove` uses the standard delete-and-recolor approach, delegating to a helper for the six deletion cases.

---

## Open-Addressing Hash Table (`src/hash.rs`)

Used for `hash<T>` and the hash component of `index<T>`.

### Record Layout

The hash table is stored as a single record in a `Store`:

```
word 0: room    (u32) — number of slots / 2 + 1  (actual slot count = room * 2 - 2 approximately)
word 1: length  (u32) — number of live elements
word 2+: slots  (4 bytes each) — each slot is a rec value (0 = empty, non-zero = occupied)
```

The slot count derived from `room` grows as a power of two.

### Probing and Load Factor

Collision resolution is **linear probing**: on collision, advance slot index by 1 (wrapping). The load factor threshold is:

```rust
length * 14 / 16 >= room
```

When this condition is met after an insertion, the table is rehashed into a new record with doubled capacity.

### Hash Function

`hash` from `src/keys.rs` is used to compute a 64-bit hash from the record's key fields. The slot index is `hash % slot_count`.

### Key Functions

| Function | Description |
|---|---|
| `add(store, hash_rec, ref_store, elem_rec, keys) -> u32` | Insert element; triggers rehash if over load factor; returns (possibly new) hash_rec |
| `find(store, hash_rec, ref_store, keys, vals) -> u32` | Lookup by key values; returns rec or 0 |
| `remove(store, hash_rec, ref_store, elem_rec, keys) -> u32` | Delete element; returns (possibly compacted) hash_rec |
| `validate(store, hash_rec, ref_store, keys)` | Debug: verify all slots are reachable from their hash position |

### Deletion

Deletion uses **backward shift**: after zeroing the removed slot, scan forward and shift back any element whose probe distance to the now-vacant slot is shorter than its probe distance to its current slot. This maintains the invariant that every element is reachable from its home slot by linear probing without encountering an empty slot.

The probe distance formula used:
```rust
d = (slot - ideal + elms) % elms
```
An element at `idx` with ideal slot `ideal` moves to `hole` when `d_hole < d_idx`. The slot containing the element to remove is found by scanning from `hash(rec) % elms` forward until a slot equals `rec.rec`.

**Null-rec guard**: `remove()` returns immediately if `rec.rec == 0` (element not found). Callers can safely call remove with a lookup result without checking first.

### `database::remove()` for Index

`database::remove()` routes to `tree::remove()` for `Parts::Index`. The `fields` argument passed to `tree::remove` must be the **byte offset** of the tree node pointers within the record (= `8 + struct_field[left_field_index].position`), not the raw field index. This is computed via `self.fields(db)` (same helper used by `tree::add`).

---

## Spatial Index (`src/radix_tree.rs`)

`spatial<T[x,y]>` / `spatial<T[x,y,z]>` (@PLN48) is a fully implemented keyed
collection on both backends (interpreter + `--native`). The `Radix(u16,
Vec<u16>)` variant of `Parts` is the schema-level marker — content type nr
plus the coordinate key field indices; the runtime `Type::Radix(content,
coord_fields, deps)` (`src/data.rs`) mirrors it. This was renamed from
`Spatial` to `Radix` (storage-honest — the language keyword stays `spatial`).

The backing structure is a **store-backed binary PATRICIA/radix tree**
(`src/radix_tree.rs`) over an abstract bit-key oracle. `src/radix_db.rs` is
the DB↔tree bridge: it interleaves the coordinate axes into a **Morton /
Z-order** key and implements `add`/`find`/`remove`/`count`/`records`/`range`.
`src/spatial.rs` holds the underlying near/within/nearest geometry algorithms
`radix_db.rs` builds on.

**Dimensionality: 1 to 3 coordinate axes** (`MAX_AXES = 3` in
`src/radix_db.rs`). The parser rejects a `spatial<T[a,b,c,d]>` with more than
3 axes with a diagnostic (*"spatial<T[…] > supports at most 3 coordinate
axes, got N"*); a bare `spatial<T>` with no key fields is also rejected
(*"needs coordinate key fields"*). See `tests/parse_errors.rs::spatial_needs_coordinate_keys`
and `::spatial_rejects_more_than_three_axes`.

Supported operations, all working on both backends:
- **Construct**: `xs: spatial<Mob[x, y]> = [];`, including as a struct field.
- **Append**: `xs += [Mob{x: 1, y: 2}];`.
- **Iterate**: `for m in xs { … }` — yields records in the tree's natural
  Morton/Z-order (no sort, unlike `hash`).
- **Length**: `xs.len()` — O(1), reads the tree's cached length word.
- **Range slices** — the language surface for proximity queries (no
  `.near`/`.within`/`.nearest` methods; spatial reuses ordinary slicing):
  `xs[(x,y)..]` (open outward walk, caller `break`s), `xs[(x,y)..:n]` (capped
  at `n`), and `xs[(x1,y1)..(x2,y2)]` (bounding-box). All three are the raw
  Morton-code interval — a bounding box is a *superset* of the geometric box
  (Z-order threads through codes outside it), so the caller filters or breaks
  as needed, same as any other keyed range slice. Slices carry up to 3 axes.

See [INTERNALS.md](INTERNALS.md) for the full radix-tree API and record
layout, and [plans/48-spacial-index/README.md](plans/48-spacial-index/README.md)
for the design history.

---

## How the Layers Fit Together

```
loft runtime value
    └── DbRef { store_nr, rec, pos }
            │
            ├── Stores::allocations[store_nr]   (Store — raw allocator)
            │       └── record at word offset rec
            │               └── field at byte offset pos
            │
            └── Stores::types[type_nr]          (Type — schema)
                    └── Parts::Sorted / Hash / Vector / Struct / ...
                            │
                            ├── Vector layout   → src/vector.rs
                            ├── Sorted/Index    → src/tree.rs  (+ src/vector.rs for Ordered)
                            ├── Hash            → src/hash.rs
                            ├── Radix           → src/radix_tree.rs + src/radix_db.rs
                            └── Key comparison  → src/keys.rs
```

- A `sorted<MyStruct>` is a red-black tree in one `Store`; the node records also contain the user data fields (the tree fields are appended after the user fields at offset `fields`).
- A `hash<MyStruct>` is a hash-table record in one `Store` pointing to element records in another (or the same) `Store`.
- An `index<MyStruct>` combines both: the same element records are simultaneously in a red-black tree (for range queries and ordered iteration) and a hash table (for O(1) lookup by key).
- A `vector<T>` is a single record with inline elements; a `sorted<T>` by value uses the same layout but maintains sort order via insertion sort on add.
- All cross-record pointers are `u32` rec offsets within the same `Store`; cross-store references use the full `DbRef`.

---

## See also
- [INTERMEDIATE.md](INTERMEDIATE.md) — Value/Type enums in detail; 233 bytecode operators; State layout
- [INTERNALS.md](INTERNALS.md) — calc.rs, stack.rs, create.rs, native.rs, ops.rs, parallel.rs, radix_tree.rs
- [DESIGN.md](DESIGN.md) — Algorithm catalog with complexity analysis for hash, index, sorted, store
