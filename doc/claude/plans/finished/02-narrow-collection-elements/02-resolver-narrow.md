<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 2 — Resolver emits narrow collection types

**Status:** blocked by Phase 1.

**Goal:** `src/typedef.rs::fill_database`'s Vector arm (and later
Hash / Sorted / Index arms) emits a narrow-element database type
whenever the content `Type::Integer` carries a forced_size.  The
database type registry ends up with entries like
`vector<int<min,null>>` alongside `vector<integer>`.

This is the first phase where user-visible behaviour changes: after
this lands, `vector<i32>` fields really do store 4-byte elements.
Reads and writes are still broken until Phase 3 / Phase 4.  So this
phase MUST ship together with at least Phase 3 — don't tag a
release in between.

---

## The change

Current code at `src/typedef.rs:325-341` (Vector arm):

```rust
Type::Vector(c_type, _) => {
    let c_nr = data.type_elm(&c_type);
    if c_nr == u32::MAX {
        continue;  // P156: unresolved content
    }
    let mut c_tp = data.def(c_nr).known_type;
    if c_tp == u16::MAX {
        fill_database(data, database, c_nr);
        c_tp = data.def(c_nr).known_type;
    }
    let tp = database.vector(c_tp);
    data.check_vector(c_nr, tp, &data.def(d_nr).position.clone());
    tp
}
```

New shape:

```rust
Type::Vector(c_type, _) => {
    let c_nr = data.type_elm(&c_type);
    if c_nr == u32::MAX {
        continue;
    }
    // P184: honour the forced_size carried on the content Type::Integer
    // (set in Phase 1 by `parse_type` from the user-typed alias's
    // size(N) annotation).  `database.byte/short/int` return narrow
    // DB types with Parts::Byte/Short/Int and the matching byte width.
    let c_tp = if let Type::Integer(minimum, _, not_null, Some(forced)) = &*c_type {
        match forced.get() {
            1 => database.byte(*minimum, !not_null),
            2 => database.short(*minimum, !not_null),
            4 => database.int(*minimum, !not_null),
            _ => {
                // 8 → fall through to the normal path (plain integer);
                // any other value is a parse-time error (caught earlier
                // in parser/definitions.rs:442 via the `only 1/2/4/8`
                // check at parser/mod.rs:1582).
                let mut c_tp = data.def(c_nr).known_type;
                if c_tp == u16::MAX {
                    fill_database(data, database, c_nr);
                    c_tp = data.def(c_nr).known_type;
                }
                c_tp
            }
        }
    } else {
        let mut c_tp = data.def(c_nr).known_type;
        if c_tp == u16::MAX {
            fill_database(data, database, c_nr);
            c_tp = data.def(c_nr).known_type;
        }
        c_tp
    };
    let tp = database.vector(c_tp);
    data.check_vector(c_nr, tp, &data.def(d_nr).position.clone());
    tp
}
```

The `database.byte/short/int` helpers already exist
(`src/database/types.rs:577/604/621`) and are used by the
Integer-arm struct-field path today.

---

## Consequences — new database types materialise

Before this phase, `database.vector(0)` (content = plain integer
at known_type 0) is the only integer-vector type in the registry.

After this phase, additional entries appear:

| Source                | Content db_tp                                | Vector name             | Stride |
|-----------------------|----------------------------------------------|-------------------------|-------:|
| `vector<integer>`     | `0` (the default integer slot)               | `vector<integer>`       |      8 |
| `vector<i32>`         | `database.int(i32::MIN+1, false)`            | `vector<int<-2147483647,false>>` |  4 |
| `vector<i16>`         | `database.short(i16::MIN+1, false)`          | `vector<short<-32767,false>>`    |  2 |
| `vector<u8>`          | `database.byte(0, false)`                    | `vector<byte<0,false>>`          |  1 |
| `vector<i32 not null>`| `database.int(i32::MIN+1, true)`             | `vector<int<-2147483647,true>>`  |  4 |

The content-type naming reuses the `name` field in `Parts::Int`
(see `database::types::int()`).  Vector naming is automatic via
`database.vector(c_tp)`'s `format!("vector<{}>", ...)`.

---

## Check for type-nr instability

`database.vector(c_tp)` caches by name — each narrow variant gets a
fresh u16 type-nr.  Audit whether any code assumes "vector-of-X has
a fixed type-nr":
- `src/data.rs` — search for `known_type == 7` (the default
  vector's slot per `typedef.rs:29`).  Hits suggest someone caches
  a well-known vector type-nr; those callers need to use the
  field's actual db_tp, not the canonical one.
- `src/database/types.rs:698` — vector-type check in `copy_claims`.

None of these should fire, but double-check.

---

## Temporary dual-path validation

Keep the existing `alias_d_nr`-based path in the Integer arm
(struct fields of narrow type) AND add the new Type-carried path in
the Vector arm.  Write an integration test:

```rust
// tests/data_structures.rs::p184_narrow_vector_element_db_tp
#[test]
fn p184_narrow_vector_element_db_tp() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str("struct Box { v: vector<i32>, w: vector<integer> }", "t", false);
    scopes::check(&mut p.data);
    let box_def = p.data.def_nr("Box");
    // v: vector<i32> → content type Parts::Int with size 4
    let v_db_tp = p.data.def(box_def).attributes[0].db_tp;
    // Read Parts::Vector(content_tp), then content_tp's size.
    let content_v = parts_vector_content(&p.database, v_db_tp);
    assert_eq!(p.database.size(content_v), 4, "vector<i32> element size");
    // w: vector<integer> → content type the default integer (size 8)
    let w_db_tp = p.data.def(box_def).attributes[1].db_tp;
    let content_w = parts_vector_content(&p.database, w_db_tp);
    assert_eq!(p.database.size(content_w), 8, "vector<integer> element size");
}
```

(`parts_vector_content` is a small test helper that reads
`Parts::Vector(content)` from the database's type table.)

When this test passes, the resolver is correctly emitting narrow
types.  The rest of the runtime (read / append / iterate) remains
broken until the later phases.

---

## Risks

- **Existing `vector<integer>` callers**.  Plain-integer vectors
  must keep their 8-byte stride.  The new code path only fires when
  the content Type::Integer has `Some(forced)` — which Phase 1
  guaranteed only fires for narrow aliases.  But VERIFY via the
  `vector<integer>` control assertion above.
- **Nested vectors**.  `vector<vector<i32>>` — the outer vector's
  content Type is `Type::Vector(Box<Type::Integer(..., Some(4))>,
  ...)`, not `Type::Integer`.  The `if let Type::Integer(...) = &*c_type`
  guard skips it → outer vector falls through to the normal path →
  inner vector resolves recursively via `fill_database(c_nr)` which
  hits this same arm for the inner content → narrow inner type
  cached in the registry.  Phase 2's narrowing should naturally
  propagate to nested cases.  Add a test for `vector<vector<i32>>`
  to confirm.
- **Global vector constants**.  `pub CONST: vector<i32> = [1, 2, 3];`
  at file scope — the CONST_STORE populator at
  `src/compile.rs::build_const_vectors` writes each element via
  `OpSet*`.  Audit whether those emission sites honour the narrow
  element size.  If they don't, file-scope narrow vectors won't
  populate correctly.

---

## Acceptance

- [ ] `cargo test --release --test data_structures` includes
      the new `p184_narrow_vector_element_db_tp` test and passes.
- [ ] `cargo test --release` overall green.
- [ ] Inspect a debug dump of a parsed `vector<i32>` field — the
      db_tp's name is `vector<int<min,null>>`, not `vector<integer>`.
- [ ] Plain `vector<integer>` fields keep db_tp name `vector<integer>`
      and stride 8.
- [ ] The `glb.loft` workaround stays in place; the user-level
      reproducer from PROBLEMS.md § P184 is still broken (reads +
      writes haven't been fixed yet).  This is expected and documented
      in the phase file — do NOT tag a release between Phase 2 and
      Phase 3.

---

## Rollback

Revert this commit only.  The Type field from Phase 1 stays, just
unread.  No data corruption risk because no narrow types have been
written to any persistent store (loft has no on-disk schema to
migrate).
