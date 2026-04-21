# `integer size(N)` — proper honor of the size annotation

## Problem

Post-2c, `integer` widened from 4 bytes to 8 bytes.  The stdlib
defines narrow aliases:

```loft
pub type u8  = integer limit(0, 255)   size(1);
pub type i8  = integer limit(-128, 127) size(1);
pub type u16 = integer limit(0, 65535) size(2);
pub type i16 = integer limit(-32768, 32767) size(2);
pub type i32 = integer                  size(4);
pub type u32 = integer limit(0, 4294967294);   // no size → heuristic
```

The **limit()** clauses for u8/i8/u16/i16 drive the existing width
heuristic in `data::Type::size()` — which returns 1 (byte) or 2
(short) based on `max - min`.  Those already route to `Parts::Byte`
and `Parts::Short` in `typedef::fill_database`.

**But `i32` has no limit** — it inherits integer's full range and
relies entirely on `size(4)`.  The parser at
`src/parser/definitions.rs:434-438` reads the `size(N)` keyword and
**discards the number**.  Post-2c, `i32` therefore silently becomes
an 8-byte integer.

Downstream breakage:
- `sizeof(i32)` returns 8 instead of 4 (the script test scripts
  currently work around this by accepting 8).
- `File.ref: i32` becomes 8B, inflating the File struct layout and
  forcing round-5 offset updates across `src/state/io.rs`,
  `src/codegen_runtime.rs`, and `src/database/io.rs` (`path` 24→32,
  `ref` 28→8, `format` 32→36, `current` 8→16, `next` 16→24).
- File I/O test scripts with `f#read(4) as i32` fail with
  "range end index 8 out of range for slice of length 4" because
  `read_data` for db\_tp=integer expects 8 bytes.
- Generic `T` bounded by `Ordered` instantiated with `i32` computes
  element stride via `type_element_size` and similar fallbacks which
  must match the explicit 4-byte width.

## Goals

1. Honor `size(N)` where `N ∈ {1, 2, 4, 8}` for any type alias of
   `integer` (or of another integer-aliased type).
2. Preserve the existing narrow-integer behaviour (u8/u16/i8/i16 via
   `Parts::Byte`/`Parts::Short`).
3. Add a **4-byte integer field** representation so `i32` is 4 B at
   rest (struct field, vector element, file byte).
4. Keep variables/stack slots 8 B (i64) regardless of field width —
   consistent with u8/u16 today.  Narrow values widen on load, narrow
   on store.
5. `sizeof(i32) == 4`, `sizeof(integer) == 8`.
6. File-I/O round-trip: `f += x as i32` writes 4 bytes and
   `f#read(4) as i32` reads 4 bytes consistently, for both raw
   variable storage and struct-field serialisation.

## Non-goals

- **Not** adding `size(16)` or arbitrary widths.  Only the four
  cardinal IEEE/integer widths the stdlib already recognises.
- **Not** removing `size(N)` parsing.  Always allowed, ignored when
  inconsistent with `limit()`.
- **Not** breaking existing pre-2c script assertions the user has
  already updated.

## Design

### 1. Capture the annotation at parse time

`src/parser/definitions.rs` — extend the `size` keyword handler to
pass the integer to the type definition:

```rust
let mut forced_size: Option<u8> = None;
if self.lexer.has_keyword("size") {
    self.lexer.token("(");
    if let Some(n) = self.lexer.has_integer() {
        if matches!(n, 1 | 2 | 4 | 8) {
            forced_size = Some(n as u8);
        } else {
            diagnostic!(Error, "size(N) must be 1, 2, 4, or 8");
        }
    }
    self.lexer.token(")");
}
if let Some(sz) = forced_size {
    self.data.definitions[d_nr as usize].forced_size = Some(sz);
}
```

Add `forced_size: Option<u8>` to `data::Definition`.

### 2. Extend `Type::Integer` with size info

Add an optional explicit size to the Type variant:

```rust
pub enum Type {
    …
    Integer(i32, u32, bool),             // (min, max, not_null) — status quo
    …
}
```

Options:
- **Option A** — add a fourth field: `Integer(i32, u32, bool, Option<u8>)`.
  Requires touching every `Type::Integer(…)` match across the codebase
  (~50 sites).
- **Option B** — store the forced size on the `Definition` struct only,
  and thread it through the resolution path.  Fewer code sites touched.
- **Option C (chosen)** — store the forced size on the *Definition*
  AND on a companion `type_elm → size_override` map accessed by
  `Type::size()` when present.  `Type::Integer` stays the same.

Option C: `data::Data` gains:

```rust
pub size_overrides: HashMap<u32 /* def_nr */, u8>,
```

Filled when parsing `pub type X = integer size(N)`.

### 3. Make `Type::size(nullable)` consult the override

In `src/data.rs:391` — when a type's resolution returned
`Type::Integer(min, max, …)`, the caller sometimes has access to the
def_nr (the type alias the user wrote) and sometimes not.

For field allocation we do have the def_nr.  Rework
`typedef::fill_database`:

```rust
Type::Integer(minimum, _, not_null) => {
    let field_nullable = nullable && !not_null;
    // Prefer the alias's forced_size, then fall back to the
    // limit-based heuristic.
    let s = data.forced_size(t_nr)
        .unwrap_or_else(|| a_type.size(field_nullable));
    match s {
        1 => database.byte(minimum, field_nullable),
        2 => database.short(minimum, field_nullable),
        4 => database.int(minimum, field_nullable),
        _ => database.name("integer"),   // 8 B
    }
}
```

Here `t_nr = data.type_elm(&a_type)` gives the alias's def_nr.

Similarly for `sizeof(T)` in `parser::control::parse_size` — use
`forced_size(def_nr)` first.

### 4. New `Parts::Int(min, nullable)` and `database.int()`

```rust
// src/database/mod.rs
pub enum Parts {
    …
    Byte(i32, bool),
    Short(i32, bool),
    Int(i32, bool),              // NEW: 4-byte integer field
    …
}
```

```rust
// src/database/types.rs
pub fn int(&mut self, min: i32, nullable: bool) -> u16 {
    let name = format!("int<{min},{nullable}>");
    if let Some(nr) = self.names.get(&name) { *nr }
    else {
        let num = self.types.len() as u16;
        self.types.push(Type::new(&name, Parts::Int(min, nullable), 4));
        self.names.insert(name, num);
        num
    }
}
```

Key choice: **use raw i32 storage with i32::MIN as null sentinel**,
not the +1 encoding used by `Parts::Byte`/`Short`.  At 4 bytes we have
enough room to reserve a single value; the shift-by-one encoding that
byte/short use to free the 0 byte isn't needed.

### 5. Store accessors

Reuse existing raw accessors:
- `Store::get_i32_raw(rec, fld) -> i32` — already exists.
- `Store::set_i32_raw(rec, fld, val)` — already exists.

Add narrow-integer-aware wrappers in `Store`:

```rust
pub fn get_int4(&self, rec: u32, fld: u32, min: i32) -> i64 {
    let raw = self.get_i32_raw(rec, fld);
    if raw == i32::MIN { i64::MIN } else { i64::from(raw) }
}

pub fn set_int4(&mut self, rec: u32, fld: u32, min: i32, val: i64) -> bool {
    let v = if val == i64::MIN { i32::MIN } else { val as i32 };
    self.set_i32_raw(rec, fld, v);
    true
}
```

`min` is accepted for API symmetry with byte/short but isn't used in
the encoding.

### 6. New opcodes

Declare in `default/01_code.loft`:

```loft
fn OpGetInt4(v1: reference, fld: const u16, min: const integer) -> integer;
#rust"{{let db = @v1; stores.store(&db).get_int4(db.rec, db.pos + u32::from(@fld), @min as i32)}}"

fn OpSetInt4(v1: reference, fld: const u16, min: const integer, val: integer);
#rust"{{let db = @v1; stores.store_mut(&db).set_int4(db.rec, db.pos + u32::from(@fld), @min as i32, @val);}}"
```

Codegen emission in `src/parser/mod.rs` — extend the set_op switch:

```rust
Type::Integer(min, _, _) => {
    let m = Value::Int(min);
    let s = …;   // as in fill_database
    match s {
        1 => self.cl("OpSetByte",  &[ref_code, pos_val, m, val_code]),
        2 => self.cl("OpSetShort", &[ref_code, pos_val, m, val_code]),
        4 => self.cl("OpSetInt4",  &[ref_code, pos_val, m, val_code]),
        _ => self.cl("OpSetInt",   &[ref_code, pos_val, val_code]),
    }
}
```

Do the same for get.

### 7. Read/write serialisation (file I/O)

`src/database/io.rs`:

- `binary_size(tp)`: add `Parts::Int(_, _) => 4`.
- `read_data` / `write_data` Parts::Int: 4 bytes endian-swapped,
  handling null as i32::MIN.
- `dispatch_read_data`: add Parts::Int to the narrow-int special-case
  — after zeroing the 8-byte variable slot, call `set_int` with the
  decoded i64 (already the pattern for Byte/Short).
- `assemble_write_data`: same pattern — read the raw i64 from the
  variable slot, serialise as 4-byte `(v as i32).to_le_bytes()`.

### 8. `type_element_size` / `element_store_size`

`src/parser/mod.rs:type_element_size` and
`src/parser/collections.rs:element_store_size`:

When the type's def_nr has a `forced_size`, return it.  Otherwise the
existing integer=8 fallback stands.

### 9. Variable storage

**No change.**  Variables remain 8-byte slots on the stack (OpPutInt,
OpVarInt).  Narrowing happens only at the field boundary
(`struct.field = x` → OpSetInt4 narrows; `x = struct.field` →
OpGetInt4 widens).

This matches the existing u8/u16 pattern exactly.

### 10. File I/O: unwind the round-5 File offset churn

With i32 honoring size(4), `File.ref: i32` goes back to 4 bytes in
the struct.  The calculated layout becomes:

- size (long) 0
- current (long) 8
- next (long) 16
- path (text) 24
- ref (i32) 28
- format (enum) 32

**Identical to pre-2c.**  All the `file.pos + 8 → + 16`, `+ 16 → + 24`,
`+ 24 → + 32`, `+ 28 → + 8`, `+ 32 → + 36` offset updates (~86 sites
across `state/io.rs`, `codegen_runtime.rs`, `database/io.rs`, and the
parser's `file_op` hidden fields) can be reverted.

**Plus** the round-5 change of `ref` from `set_i32_raw`/`get_i32_raw`
with `i32::MIN` sentinel back to the 4-byte path.

This also undoes the `get_dir` `vector_append(&vector, 37, …)` size
(back to 33) and the test-script size assertions (`sizeof(Pos) = 16`
→ 8, `sizeof(i32) = 8` → 4, `inf_pos as integer` saturation bound
back to i32::MAX).

## Implementation order

1. **Capture** `forced_size` on Definition; parser updated.
   (src/parser/definitions.rs, src/data.rs)

2. **Thread** `forced_size` into `Type::size()` resolution used by
   `fill_database`, `parse_size`, `type_element_size`,
   `element_store_size`.
   (src/typedef.rs, src/parser/control.rs, src/parser/mod.rs,
   src/parser/collections.rs)

3. **Add `Parts::Int`** and `database.int()`.
   (src/database/mod.rs, src/database/types.rs — including format,
   allocation, matching, binary_size)

4. **Add store accessors** `get_int4` / `set_int4`.
   (src/store.rs)

5. **Add opcodes** `OpGetInt4` / `OpSetInt4` in stdlib; regen
   `src/fill.rs`; grow OPERATORS array.
   (default/01_code.loft, src/fill.rs)

6. **Update codegen** to emit the new ops for size==4 integer fields.
   (src/parser/mod.rs set_op/get_op switches)

7. **Update file I/O** (binary_size, read_data, write_data,
   assemble_write_data, dispatch_read_data) for `Parts::Int`.
   (src/database/io.rs, src/state/io.rs)

8. **Revert round-5 File offset churn** once `File.ref: i32` is back
   to 4 B — the pre-2c layout comes back unchanged.
   (src/state/io.rs, src/codegen_runtime.rs, src/database/io.rs,
   src/parser/objects.rs file_op)

9. **Revert test-script assertions**:
   - `tests/scripts/06-structs.loft` — sizeof(integer) stays 8,
     sizeof(i32) returns to 4, sizeof(Pos) returns to 8.
   - `tests/scripts/02-floats.loft` — infinity-as-integer saturates
     at i64::MAX (keep — integer IS 8B; the update was correct).
   - `tests/docs/13-file.loft` — size asserts revert to 16/28 for
     `u8+u8+u16+i32+long` (4-byte i32 now honored).

10. **Test** the cluster of stale-as-i32 scripts: binary, binary\_ops,
    files (already pass with 32 bytes), file\_debug, dir, last,
    loft\_suite, parser\_debug, stress.

## Risks

- **Type::Integer with forced_size but without a resolvable def_nr**:
  when a literal like `127 as i32` gets typed as Integer with the i32
  alias, we need the cast to preserve the size-4 intent.  Casts
  already set the target type's name; the def_nr is reachable via
  `data.type_def_nr(&type_expr)`.  Verify in `parse_type`.

- **Generic instantiation caching**: generic T stubs that reference
  an integer field bake a placeholder size.  The round-5 substitution
  fix (`type_element_size(concrete, data)` when sizes differ) already
  handles the update; just need `type_element_size` to consult
  `forced_size`.

- **Native codegen (`src/generation/*`)**: emits Rust code for fields.
  Must emit i32 reads/writes (not i64) for Parts::Int fields.  Already
  distinguishes narrow ints — extend the same switch.

- **DbRef null-sentinel encoding** in key comparison (`src/keys.rs`):
  `get_int4` returns i64::MIN for null, so key compare just needs to
  handle that sentinel in the narrow path.

## Acceptance

- `cargo test --release` — net failure count drops from ~18 to ~8
  (narrowing to just the Class C native cascade, Class D parallel
  edge, and any newly surfaced bug).
- `sizeof(i32) == 4`, `sizeof(integer) == 8` (script test).
- `f += x as i32; f#read(4) as i32 == x` round-trips.
- `File` struct layout reverts to the pre-2c offsets; round-5 offset
  tables in state/io.rs disappear from the diff.
