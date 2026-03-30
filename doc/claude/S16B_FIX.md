# S16b Fix: Range Queries on `sorted<EnumVariant[field]>`

## Problem

Range queries `col[lo..hi]` on `sorted<EnumVariant[field]>` fail with:

```
Unknown in expression type Shape
loop variable 'c' has type null but was previously used as Circle
```

Full iteration (`for c in col { ... }`) and point lookup (`col[key]`) work correctly;
only the range-query form is broken.

Tracked as roadmap item S16b.
Reproducer: `tests/scripts/25-sorted-enum-variant-range.loft`.

---

## Root cause

The bug lives in `index_type()` (`src/parser/fields.rs`).

For a collection `sorted<Circle[score]>`, the element definition number `circle_def_nr`
is the `DefType::EnumValue` definition of the `Circle` variant.  During the first parser
pass, `Circle` was registered with:

```rust
// src/parser/definitions.rs
self.data.set_returned(v_nr, Type::Enum(d_nr, true, Vec::new()));
//                                       ^^^^
//                                       parent enum (Shape), not variant (Circle)
```

`index_type()` blindly returns `.returned` for any `Sorted / Index / Hash / Spacial`
collection:

```rust
// BEFORE (buggy)
} else if let Type::Sorted(d_nr, _, _) | ... = t {
    self.data.def(*d_nr).returned.clone()   // → Type::Enum(shape_d_nr, true, [])
}
```

This is correct for plain structs (`.returned = Type::Reference(d, [])`) but wrong for
struct-enum variants (`.returned = Type::Enum(parent, true, [])`).

### Propagation path for range queries

```
for c in db.circles[15..25] { ... }
 │
 ├─ parse_in_range
 │   └─ expression("db.circles[15..25]")
 │       └─ parse_index(Type::Sorted(circle_def_nr, ...))
 │           ├─ elm_type = index_type(Sorted)
 │           │             = data.def(circle_def_nr).returned
 │           │             = Type::Enum(shape_def_nr, true, [])   ← WRONG
 │           └─ parse_key() builds Value::Iter(...)   [key range iterator]
 │
 │   in_type = Type::Enum(shape_def_nr, true, [])   ← WRONG
 │
 └─ parse_for_iter_setup(in_type = Enum(shape, true, []))
     └─ var_tp = for_type(Enum(shape, true, []))
                 → hits error case → "Unknown loop type Shape"
                 → returns Type::Null
     └─ for_var created with Type::Null
     └─ c.score → field access on Null → "Unknown in expression type Shape"
```

### Why full iteration and point lookup work

**Full iteration** (`for c in db.circles`) does not call `parse_index()`.  The loop
type comes from `for_type(Type::Sorted(circle_def_nr, ...))` which maps directly to
`Type::Reference(circle_def_nr, [])`.

**Point lookup** (`db.circles[10].radius`) also calls `index_type()` and gets the wrong
`Type::Enum(shape, true, [])`, but the subsequent field access `.radius` uses
`find_poly_enum_field(shape_def_nr, "radius")` which searches all variants of `Shape`
and finds `radius` in `Circle` at the correct byte offset.  Range queries fail before
field access can happen because `for_type()` rejects `Type::Enum(_, true, _)`.

---

## Fix

### Primary fix — `index_type()` (`src/parser/fields.rs`)

```rust
// AFTER
} else if let Type::Sorted(d_nr, _, _)
| Type::Hash(d_nr, _, _)
| Type::Index(d_nr, _, _)
| Type::Spacial(d_nr, _, _) = t
{
    let ret = self.data.def(*d_nr).returned.clone();
    // S16b: struct-enum variants have .returned = Type::Enum(parent, true, []).
    // For collection element access we need Type::Reference(variant_def_nr, [])
    // so that field access and range-query for-loops resolve fields against the
    // variant struct (not the parent enum), and for_type() can map the element type.
    if matches!(ret, Type::Enum(_, true, _)) {
        Type::Reference(*d_nr, Vec::new())
    } else {
        ret
    }
}
```

After this fix, `index_type(Sorted(circle_def_nr, ...))` returns
`Type::Reference(circle_def_nr, [])` — the same element type that full iteration
produces via `for_type(Sorted(circle_def_nr, ...))`.

The `parse_index()` caller already applies dependency propagation:
```rust
let mut elm_type = self.index_type(&t);
for on in t.depend() {
    elm_type = elm_type.depending(on);
}
```
so any dependencies on the `Sorted` type are correctly applied to the element type
without any change to this loop.

### Secondary fix — `v_block` annotation in `parse_key()` (`src/parser/fields.rs`)

The range-query branch of `parse_key()` builds a `Value::Iter` whose `next` block was
annotated with `typedef.clone()` = `Type::Sorted(...)` (the collection type).  After
the primary fix the block annotation is no longer load-bearing, but it is cosmetically
wrong and confuses IR dumps.  Fixed to derive the element type:

```rust
let elem_type = match typedef {
    Type::Sorted(el, _, dep) | Type::Index(el, _, dep) => {
        Type::Reference(*el, dep.clone())
    }
    _ => typedef.clone(),
};
*code = Value::Iter(
    u16::MAX,
    Box::new(start),
    Box::new(v_block(vec![self.cl("OpStep", &ls)], elem_type, "Iterate keys")),
    Box::new(Value::Null),
);
```

---

## Tests

| Script | What it covers | Status after fix |
|--------|---------------|-----------------|
| `23-field-overlap-structs.loft` | Plain structs sharing field name `val` at different offsets; sorted lookup, full iteration, range query, index range query | ✓ (was already passing) |
| `24-field-overlap-enum-struct.loft` | Struct-enum variants sharing field `score`; plain-struct/variant sharing `key`; basic lookup and full iteration | ✓ (was already passing) |
| `25-sorted-enum-variant-range.loft` | Range query `col[lo..hi]` on `sorted<Circle[score]>` — the S16b reproducer | ✓ (fixed by this change) |

All existing tests continue to pass (40/40).

---

## Why the fix is safe

1. **Plain structs unaffected** — their `.returned = Type::Reference(d, [])` does not
   match `Type::Enum(_, true, _)`, so the new branch is never taken.
2. **Enum without fields unaffected** — simple enum values have `is_variant = false`
   (`Type::Enum(_, false, _)`); the guard checks `true` explicitly.
3. **Point lookup still works** — `db.circles[10].radius` previously used
   `find_poly_enum_field(shape, "radius")` (searching all variants).  After the fix it
   uses direct attribute lookup `data.attr(circle_v_nr, "radius")`.  Both produce the
   same byte offset; the result is identical.
4. **No OpGetRecord / OpIterate / OpStep changes** — the fix is purely in the type
   layer; bytecode generation is unchanged.
