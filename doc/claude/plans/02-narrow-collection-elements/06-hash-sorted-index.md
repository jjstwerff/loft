<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 6 — Extend to Hash / Sorted / Index

**Status:** blocked by Phase 5.

**Goal:** the same narrow-element treatment works for `hash`,
`sorted`, and `index` collections when they're parameterised over an
integer alias — which in practice means narrow keys (for
`hash<Struct[u32_key]>`) and, potentially, narrow primitive-valued
hashes (uncommon).

---

## What's different vs Vector

Vector is the simple case: elements are values of type `T`.  The
other collections are more structured:

- **`hash<Struct[key1, key2]>`** — stores references to `Struct`
  records; keys are fields on those records.  Narrowing matters
  only if a key field is a narrow integer alias.  That's handled
  by the existing struct-field path (unchanged since pre-Phase-0).
- **`sorted<Struct[key]>`, `index<Struct[key]>`** — same story,
  references to structs.  The Vector-arm narrow-content rule
  doesn't directly apply because the elements aren't primitives.

So Phase 6 is SMALLER than it looks:

1. **Primitive-content hash/sorted/index** — does loft even allow
   `hash<i32>` (hash of primitive)?  Probably not directly — these
   collections typically require named key fields.  Confirm by
   reading the parser.  If it's disallowed, Phase 6 has nothing to
   do and the README row can be struck out.

2. **Primitive-content Spacial** — skipped entirely; Spacial is a
   C7/P22 diagnostic-only stub.

3. **Vector-of-Struct with narrow primitive fields** — already
   works pre-Phase-0 via the struct-field narrowing path.  No new
   work.

Likely outcome: Phase 6 is a doc update + regression test that
explicitly confirms `hash<Struct[i32_key]>` (with a narrow key
field) still narrows correctly, closing the loop.

---

## Audit

Start by checking what collection-with-primitive-content forms the
parser accepts:

```bash
rg 'hash<|sorted<|index<' tests/scripts/ lib/ | head -20
```

Typical forms seen in the codebase:
```
hash<Row[id]>        — references to Row records, keyed by `id`
sorted<Order[date]>  — references to Order, keyed by `date`
index<Node[parent]>  — non-unique index into Node by `parent`
```

Primitive-content forms (`hash<integer>`, `sorted<i32>`) are likely
parse errors.  Confirm via a one-off test:

```loft
struct H { h: hash<i32> }  // Does this parse?
```

If it parses, Phase 6 needs to cover it — follow the Phase 2 /
Phase 3 / Phase 4 pattern for the Hash/Sorted/Index arms in
`typedef.rs::fill_database`.

If it doesn't parse, Phase 6 closes with a documentation note:
"Hash / Sorted / Index require struct-referenced element types;
primitive-content narrowing is not applicable."

---

## If primitive-content collections exist

Apply the Phase 2 pattern to Hash/Sorted/Index arms.  For example,
Hash arm at `src/typedef.rs:363` (approximately):

```rust
Type::Hash(c_nr, key_fields, _) => {
    // ... existing setup ...
    let c_tp = /* same narrow-content lookup as Vector arm */;
    let tp = database.hash(c_tp, &key_fields);
    // ...
    tp
}
```

Then Phase 3 (read path) and Phase 4 (write path) extensions for
these collection kinds.

---

## Regression tests (if applicable)

```rust
// tests/issues.rs

#[test]
fn p184_hash_narrow_key_field() {
    // Control: hash of structs with a narrow u32 key field.
    // Already works via struct-field narrowing; this confirms
    // the Phase 0-5 work didn't break it.
    code!(r#"
        struct Row { id: u32 not null, name: text }
        struct Db { rows: hash<Row[id]> }
        fn test() {
            db = Db { rows: [] };
            db.rows += [Row { id: 42, name: "hi" }];
            assert(db.rows[42].name == "hi");
        }
    "#).result(Value::Null);
}

#[test]
#[ignore = "Depends on whether hash<i32> parses — gate by audit outcome"]
fn p184_hash_primitive_narrow() {
    code!(r#"
        fn test() {
            h: hash<i32> = [];  // TBD syntax
            // ...
        }
    "#).result(Value::Null);
}
```

---

## Closing the initiative

After Phase 6 completes:

1. Update `doc/claude/PROBLEMS.md` § P184 detail entry: strike
   the heading, add "**Fixed**" status block with date, fix
   summary, and test reference.
2. Update `doc/claude/PROBLEMS.md` § P184 row in the quick-ref
   table: cross it out, update the workaround column to point at
   the merge commit.
3. Update `doc/claude/CAVEATS.md` § C54 "Cdylib FFI wrapper claim
   was obsolete" bullet: the situation has changed, so either
   revise or remove.
4. Move `doc/claude/plans/02-narrow-collection-elements/` to
   `doc/claude/plans/finished/02-narrow-collection-elements/`.
5. Update `doc/claude/plans/README.md` — move row from Current
   to Finished.
6. Remove the `glb_write_indices` workaround in
   `lib/graphics/src/glb.loft` (done in Phase 4 already, but
   re-verify after Phase 6 ships).

---

## Acceptance

- [ ] Primitive-content Hash/Sorted/Index forms: audit complete,
      either implemented or documented as not-applicable.
- [ ] Regression tests as appropriate.
- [ ] Initiative closeout checklist (above) complete.
- [ ] Full test suite green: `make ci` + `make test-packages`.
