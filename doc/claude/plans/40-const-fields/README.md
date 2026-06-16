<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN40 — `const` struct fields (write-once-at-construction)

Extend the existing `const` keyword from locals + parameters to
**struct fields**, giving loft a "frozen after construction" field
modifier.  Closes the locals-vs-fields asymmetry surfaced in
[INCONSISTENCIES.md § 33](../../INCONSISTENCIES.md#33-const-applies-to-locals-and-parameters-but-not-fields).

## Status

**Not started.**  Single-feature design; no phasing.  Effort: M
(parser + type-checker, no runtime change, no schema change).

## Goal

```loft
struct Token {
  const id:    integer not null,    // set once at construction
  const issued_at: long not null,
  payload:     text                 // mutable, as today
}

t = Token { id: 42, issued_at: 1_000_000l, payload: "" };
t.payload = "hello";        // OK
t.id = 99;                  // ERROR: cannot reassign const field 'id'
```

The constraint is **purely static** — the runtime layout is
unchanged, the access ops (`OpGetInt`, `OpSetInt`, …) are unchanged.
The check is rejection of `OpSet*(field=const_field, …)` outside
the constructor's `Insert` IR.

## Why now

After [@P246](../../PROBLEMS.md) closed (file-scope `const`
accepted) and the UPPER_CASE-non-const warning landed, loft's
immutability story is uniform at every scope **except** struct
fields.  Locals get `const x = 5;`; parameters get
`fn f(const x: T)`; constants get `const PI = 3.14;`.  Fields
have no equivalent — every field is implicitly mutable.  The
first real consumer of the gap is [TTT v5's `Cell`
struct](../39-tic-tac-toe/README.md): `c_color`, `c_height`, and
`c_age` should ALL be const after construction (the tick loop
rebuilds the entire cell via `chunk.ck_cells[idx] = Cell{…}`,
not via in-place field writes).  Marking them const would let the
parser CATCH the wrong pattern (`cell.c_color = 0;` is the only
allowed mutation today and is silently legal everywhere).

## Design

### Syntax

`const` slots in alongside the existing field modifiers
(`not null`, `limit(...)`, `default(...)` / `= expr`,
`virtual(...)`).  Order is `const` first, before the type:

```loft
struct Foo {
  const x: integer not null,           // const + not null
  const y: integer = 0,                // const + default
  const w: integer not null = 1,       // const + both
  z: integer                            // mutable, no change
}
```

`const virtual(expr)` is **rejected** at parse time —
`virtual` already implies "computed, no storage, no write" which
const subsumes; combining the two would be a documentation
hazard with no benefit.

### Constructor enforcement

Every `const` field MUST be initialised in the struct literal
unless it has a `default` clause.  This matches the existing
rule for `not null` fields (with default).  Diagnostic when
missing:

```
struct Foo { const x: integer not null }
fn main() { f = Foo {}; }
```

```
error: field 'x' is `const` and has no default — every const
       field must be initialised in the struct literal
```

### Post-construction enforcement

Any `OpSet*(field=const_field, …)` outside the parse path that
emits the constructor's `Insert` IR is a parse error:

```
fn touch(t: Token) { t.id = 99; }
```

```
error: cannot reassign const field 'id' of struct 'Token' —
       const fields are write-once-at-construction
```

The check fires regardless of the access route:

- direct: `t.id = 99`
- nested: `box.t.id = 99`
- via `&` parameter: `fn touch(t: &Token) { t.id = 99; }` rejected
- through a closure capture: `|t| t.id = 99` rejected

### What `const` does NOT cover

- **Mutation through `Reference<T>`**: `Reference<T>` already
  bypasses normal field-write checks (it points into the heap
  directly).  P33 does NOT extend the const check to
  `Reference<T>` writes — that would require runtime tracking
  and undermine the "purely static, zero-cost" property.
  Document this as an explicit limitation.
- **Vector / map / hash element mutation through a const field**:
  `const items: vector<integer>` makes the FIELD const (you
  can't reassign `s.items = […]`) but the elements
  (`s.items[0] = 5`) are still mutable.  Same rule as Rust's
  `let v = vec![1,2,3]; v[0] = 5;` — outer binding immutable,
  contents not.
- **Struct-enum variants**: a `const` variant field follows the
  same rule as a regular const field.  Variant matching with
  field bindings (`if shape is Circle { radius }`) creates a
  read-only local; that's already const-by-construction and
  needs no annotation.

### Interaction with the UPPER_CASE warning

Field names are conventionally `lower_case` (or `prefix_field`
like `c_color`).  The UPPER_CASE-non-const warning on locals
does NOT extend to fields — `const` on a field signals
immutability already, and field naming follows a different
convention.  No new warning here.

## Implementation plan

| ID  | Step | File(s) | Effort |
|-----|------|---------|--------|
| 33.1 | Parser: accept `const` as a field modifier alongside `not null` / `limit(...)` / `= expr` | `src/parser/definitions.rs::parse_struct` (or wherever fields are parsed) | S |
| 33.2 | `data::Attribute` carries a `const_field: bool` flag | `src/data.rs` | S |
| 33.3 | Constructor check: every `const` field without a default must appear in the struct literal | `src/parser/objects.rs` (struct-literal parse) | S |
| 33.4 | Field-write check: `OpSet*` emission rejects writes to const fields outside `Insert` IR | `src/parser/expressions.rs` / `src/parser/fields.rs` (assignment LHS) | M |
| 33.5 | Reject `const virtual(...)` at parse time | wherever modifier combination is validated | XS |
| 33.6 | Doc updates: LOFT.md § Field modifiers; SKILL.md field-modifier table | `doc/claude/LOFT.md`, `.claude/skills/loft-write/SKILL.md` | S |
| 33.7 | Test suite: `tests/issues.rs::p33_const_field_*` — accept valid construction, reject reassignment / missing init / `const virtual` | `tests/issues.rs` | M |
| 33.8 | (Optional) Apply `const` to TTT v5's `lib/world/src/world.loft::Cell` (c_color, c_height, c_age) and verify the tick loop keeps working — the first real consumer that proves the rule earns its keep | `lib/world/src/world.loft` | S |

**Total effort:** M (one focused session; no new opcodes, no
schema changes, no runtime code touched).

**Dependencies:** none.  Builds on @P246's `const` story.

## Open questions (not blocking)

- Should `const` fields auto-be `not null`?  Semi-Rust answer:
  yes, because a const field that can be null forever can never
  be "later set to a real value" — if the construction sets it
  to null, it stays null.  Counter: explicit `not null` is loft's
  style, don't bake it in.  **Default: keep them orthogonal**;
  `const x: integer` (nullable, written once, can stay null)
  is legal and meaningful.
- `pub const field`?  Currently struct fields don't have
  visibility modifiers (everything in a `pub` struct is
  reachable).  Out of scope.
- Const on enum variant fields?  In scope — works the same way
  via the regular field-parse path.

## Out of scope (deferred)

- Reference<T> mutation enforcement — would require runtime
  tracking, defeats the zero-cost property.
- Const trait fields — interfaces don't carry field declarations
  today; this would couple to a separate language feature.

## Test strategy

Three positive shapes, three negative shapes, plus the TTT v5
real-world consumer:

```loft
// p33_const_field_constructed_and_read
struct Token { const id: integer not null }
t = Token { id: 42 };
assert(t.id == 42, "read after construction");

// p33_const_field_with_default
struct Cfg { const port: integer not null = 8080 }
c1 = Cfg {};
c2 = Cfg { port: 9000 };
assert(c1.port == 8080 && c2.port == 9000, "default + override");

// p33_const_field_in_nested_position
struct Inner { const tag: integer not null }
struct Outer { i: Inner }
o = Outer { i: Inner { tag: 7 } };
assert(o.i.tag == 7, "nested read");
```

Negative (`@EXPECT_ERROR`):

```loft
// p33_const_field_reassign_rejected
struct Token { const id: integer not null }
fn main() {
  t = Token { id: 1 };
  t.id = 99;          // ERROR: cannot reassign const field 'id'
}

// p33_const_field_missing_init_rejected
struct Foo { const x: integer not null }
fn main() {
  f = Foo {};         // ERROR: field 'x' is `const` and has no default
}

// p33_const_virtual_rejected
struct Bad { const v: integer virtual($.x * 2), x: integer }
//          ERROR: `const virtual(...)` — virtual already implies …
```

## See also

- [INCONSISTENCIES.md § 33](../../INCONSISTENCIES.md) — gap motivation
- [PROBLEMS.md @P246](../../PROBLEMS.md) — file-scope `const` (the @P246 closure that motivates the symmetric rule)
- [LOFT.md § Field modifiers](../../LOFT.md) — current modifier list (to be extended)
- `.claude/skills/loft-write/SKILL.md § Field modifiers` — user-facing reference (to be extended)
- [lib/world/src/world.loft](../../../../lib/world/src/world.loft) — first real consumer (Cell c_color/c_height/c_age)
