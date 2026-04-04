---
render_with_liquid: false
---
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
| `src/radix_tree.rs` | Radix tree for `spacial<T>` (partially implemented) |

---

## Store — Raw Heap Allocator (`src/store.rs`)

### Memory Layout

A `Store` is a single contiguous heap allocation addressed in **8-byte words** (not bytes). The raw pointer is `*mut u8` but all offsets are expressed as word indices (`u32`).

```
word 0: size header  (i32, positive = claimed, negative = free, magnitude = record size in words)
word 1..n: record data
```

Records are variable-length. The size word at offset 0 of each record encodes both the length and the claimed/free state:

- **Positive**: record is live; size = word count of record body (not counting the header word itself).
- **Negative**: record is free; `-size` = word count of the free block body.

Free blocks form an implicit free list. `claim(size)` walks the store from the start, finds the first free block large enough, and either takes it exactly (if the remainder is too small to split) or splits it and leaves a smaller free block.

### Store struct

```rust
pub struct Store {
    data: *mut u8,    // raw backing allocation
    size: u32,        // current capacity in words
}
```

The store grows by doubling when a claim cannot be satisfied from free space.

### Key Functions

| Function | Description |
|---|---|
| `claim(size: u32) -> u32` | Allocate `size` words; returns `rec` (word offset of the header) |
| `delete(rec: u32)` | Free a record; marks header negative; may merge adjacent free blocks |
| `resize(rec: u32, size: u32) -> u32` | Grow or shrink a record in place if possible; otherwise relocates |
| `validate()` | Debug check: walks all records verifying headers are consistent |
| `get_int(rec, pos) -> i32` | Read a 4-byte integer at word `rec`, byte offset `pos*4` |
| `set_int(rec, pos, val)` | Write a 4-byte integer |
| `get_long(rec, pos) -> i64` | Read an 8-byte integer (two consecutive words) |
| `set_long(rec, pos, val)` | Write an 8-byte integer |
| `get_str(rec, pos) -> &str` | Read a string reference (pointer + length stored inline) |
| `set_str(rec, pos, val)` | Write a string reference |
| `lock()` | Mark the store as read-only |
| `unlock()` | Remove the read-only mark |
| `is_locked() -> bool` | Return whether the store is currently locked |

Typed accessors (`get_int`, `get_long`, `get_str`, etc.) apply a byte-level offset within the record. String storage encodes the pointer and length as two 4-byte words.

### Store Locking

A `Store` carries a `locked: bool` flag that marks the store as read-only at runtime.

```rust
pub struct Store {
    // ...
    pub locked: bool,
}
```

When `locked` is `true`, any attempt to write to the store is treated as an error:

- **Debug builds** (`#[cfg(debug_assertions)]`): `addr_mut` panics immediately with `"Write to locked store at rec={rec} fld={fld}"`. `claim()` and `delete()` also `debug_assert!` against a locked store.
- **Release builds**: writes are silently discarded. `addr_mut` returns a pointer to a thread-local 256-byte dummy buffer so the caller never dereferences a null pointer. `claim()` returns 0; `delete()` is a no-op.

This design gives a hard fail-fast contract in development while keeping production overhead to a single branch.

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
| `Spacial(u16, Vec<u16>)` | Spatial index (future) |

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

The `Spacial(u16, Vec<u16>)` variant of `Parts` is the schema-level marker for a spatial index collection. Its planned backing structure is a **radix tree** implemented in `src/radix_tree.rs`.

The radix tree is partially implemented (inserts and finds work; iteration and removal are stubs). See `doc/claude/INTERNALS.md` for the full API and record layout. The `Spacial` type is reserved in the schema today but not yet wired to the radix tree operations in the interpreter. See [INTERNALS.md](INTERNALS.md) for the full API and record layout.

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
                            ├── Spacial         → src/radix_tree.rs (partial)
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
