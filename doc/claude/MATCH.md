---
render_with_liquid: false
---
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Match Expression Design (T1-4)

> **Status: fully implemented** — shipped progressively 2026-03-16 through 2026-03-18.
> Core (T1-4), guards (T1-16), scalars (T1-14), or-patterns (T1-15), ranges (T1-17),
> struct destructuring (T1-18), null/binding (T1-20), slice/vector patterns (T1-21).
> Only planned extension remaining: **L2 Nested patterns in field positions** (see PLANNING.md).

A pattern-matching expression for dispatching on enum values with
compiler-checked exhaustiveness.

---

## Contents

- [Goals](#goals)
- [Syntax](#syntax)
- [Semantics](#semantics)
- [Relationship to Polymorphic Dispatch](#relationship-to-polymorphic-dispatch)
- [Type System Internals](#type-system-internals)
- [IR Lowering](#ir-lowering)
- [Edge Cases](#edge-cases)
- [Test Coverage](#test-coverage)
- [Non-Goals (T1-4)](#non-goals-t1-4)
- [Planned Extension: L2 Nested patterns in field positions](#l2--nested-patterns-in-field-positions)

---

## Goals

- Give plain enums a dispatch mechanism (currently impossible without
  an if/else chain — INCONSISTENCIES #6).
- Let struct-enum code bind variant fields directly in the arm body,
  eliminating the need to declare a method for every per-variant
  expression.
- Produce a value so `match` can appear on the right-hand side of an
  assignment or as a function argument.
- Error at compile time when not all variants are covered and no
  wildcard is present.

---

## Syntax

```
match-expr      ::= 'match' expression '{' arm+ '}'
arm             ::= pattern '=>' expression
pattern         ::= variant-pattern | '_'
variant-pattern ::= Identifier [ '{' field-list '}' ]
field-list      ::= Identifier { ',' Identifier }
```

The `match` keyword is reserved.  Arms are separated by newlines; no
trailing comma or semicolon is required between arms.  `=>` is already
in `TOKENS` so `has_token("=>")` works today.

---

## Semantics

### Plain enum arms

Dispatch on value equality for each named variant.

```loft
enum Direction { North, East, South, West }

label = match direction {
    North => "N"
    East  => "E"
    South => "S"
    West  => "W"
}
```

### Struct enum arms with field binding

The optional `{ field, ... }` clause introduces scoped read-only
variables bound to the named fields of that variant.

```loft
enum Shape {
    Circle { radius: float },
    Rect   { width: float, height: float },
    Square { side:  float },
}

area = match shape {
    Circle { radius }        => PI * radius * radius
    Rect   { width, height } => width * height
    Square { side }          => side * side
}
```

A variant arm without field bindings is valid: `Circle => true`.
Bound field variables are immutable within the arm body.

### Wildcard arm

`_` matches any variant not covered by a preceding arm.  It must be
the last arm; a wildcard followed by another arm is an error.

### Exhaustiveness

No wildcard present → every variant must appear exactly once.
A missing variant is a **compile-time error**.
A duplicate variant arm is a compile-time **warning** ("unreachable arm").

### Result type

All arm bodies must produce compatible types (same unification rules as
`if`/`else` — `merge_dependencies`).  A type mismatch is an error.
When used as a statement the result is dropped and arms may be void.

---

## Relationship to Polymorphic Dispatch

The existing `enum_fn` / `create_enum_dispatch_fn` mechanism
synthesises a dispatcher at parse time for method calls like
`shape.area()`.  `match` and polymorphic dispatch coexist:

| Mechanism | Trigger | Use case |
|---|---|---|
| Polymorphic dispatch | `shape.method()` | Reusable, named per-variant behaviour |
| `match` expression | `match shape { ... }` | One-off dispatch with optional field binding |

T1-4 also resolves INCONSISTENCIES #6 — plain-enum methods can now be
written as free functions:

```loft
fn label(d: Direction) -> text {
    match d {
        North => "N"
        East  => "E"
        South => "S"
        West  => "W"
    }
}
```

---

## Type System Internals

### `Type::Enum(d_nr, is_struct, deps)`

| Field | Meaning |
|---|---|
| `d_nr` | Definition number of the enum type |
| `is_struct` | `false` = plain enum (stack byte); `true` = struct enum (Reference) |
| `deps` | Lifetime dependencies |

### Finding all variants of an enum

```rust
data.definitions.iter().enumerate()
    .filter(|(_, d)| d.def_type == DefType::EnumValue && d.parent == e_nr)
```

### Plain enum discriminant

`data.def(variant_def_nr).attributes[n].value` is `Value::Enum(n+1, u16::MAX)`.
Discriminants start at 1; 0 is null.

### Struct enum layout

`EnumValue` `variant_def_nr` has:
- `attributes[0]`: the `"enum"` discriminant field; value = `Value::Enum(n+1, u16::MAX)`
- `attributes[1..]`: the user-declared struct fields in declaration order

`get_field(variant_def_nr, attr_idx, subject_code)` reads a field from
the variant struct reference — `attr_idx == 0` reads the discriminant,
`attr_idx == 1` reads the first user field, etc.

---

## IR Lowering

`match` lowers to a chain of `Value::If` nodes.  No new IR nodes or
bytecode opcodes are needed — `fill.rs` and `state/codegen.rs` are
unchanged.

### Plain enum arm

```
match direction { North => a, East => b, _ => c }
```

compiles to (discriminants start at 1):

```
If(OpEqInt(OpConvIntFromEnum(Var(direction)), 1),   // North
   Block([a]),
   If(OpEqInt(OpConvIntFromEnum(Var(direction)), 2), // East
      Block([b]),
      Block([c])))
```

### Struct enum arm

```
match shape { Circle { radius } => f(radius), _ => 0.0 }
```

compiles to (discriminant of Circle = 1, radius is attributes[1]):

```
If(OpEqInt(OpConvIntFromEnum(OpGetEnum(Var(shape), 0)), 1),
   Block([
     Set(radius_var, get_field(Circle_def_nr, 1, Var(shape))),
     f(Var(radius_var))
   ]),
   Block([0.0]))
```

---

## Edge Cases

| Case | Behaviour |
|---|---|
| Subject is not an enum type | Compile-time error |
| Arm names a variant from a different enum | Compile-time error |
| Missing variant, no wildcard | Compile-time **error** |
| Wildcard only | Valid; matches everything |
| Wildcard not last | Compile-time error ("unreachable arm after wildcard") |
| Duplicate arm for the same variant | Compile-time warning ("unreachable arm") |
| Field name in binding does not exist on variant | Compile-time error |
| Duplicate field in binding `{ x, x }` | Compile-time error |
| Arm body type mismatch | Compile-time error |
| `match` as last expression in a block | Returns the matched arm's value |
| `match` as a statement | Arms may be void; result is dropped |
| Nested `match` inside an arm body | Works via recursive `expression()` call |
| `null` subject | Plain enum: discriminant is 0; no arm matches (falls through to wildcard or null result). Struct enum: `OpGetEnum` on a null reference returns the null sentinel. |
| Guarded arm exhaustiveness | A guarded arm (`Variant if cond => ...`) does **not** count as covering that variant for exhaustiveness. The guard may fail at runtime, so a wildcard `_` or unguarded arm is still required. This matches Rust's behaviour. (INCONSISTENCIES #26) |

---

## Test Coverage

17 tests in `tests/match.rs` (all passing).

| Test | Coverage |
|---|---|
| `plain_all_arms` | All variants covered; each arm returns the expected value |
| `plain_wildcard` | Partial coverage + `_`; wildcard catches remainder |
| `plain_wildcard_first` | Wildcard catches the first variant |
| `plain_missing_arm` | Missing variant without wildcard → compile error |
| `plain_as_statement` | Match used as void statement |
| `plain_as_integer_value` | Match produces an integer used in arithmetic |
| `plain_in_function` | Match used as function return value |
| `struct_no_binding` | `Circle => true, _ => false` |
| `struct_single_field` | `Circle { radius } => radius * radius` |
| `struct_multi_field` | `Rect { width, height } => width * height` |
| `struct_all_variants` | All three Shape variants with exhaustiveness |
| `struct_missing_arm` | Missing struct-enum variant without wildcard → compile error |
| `match_nested` | `match` arm body contains another `match` |
| `match_in_call` | `f(match d { ... })` — match as a function argument |
| `match_non_enum` | `match 42 { ... }` → compile error |
| `match_type_mismatch` | Arms returning incompatible types → compile error |
| `match_duplicate_arm` | Same variant twice → warning |

---

## Non-Goals (T1-4)

- **Binding by mutable reference**: field bindings are read-only copies.
- **Tuple destructuring**: loft has no tuple types.

---

## L2  Nested patterns in field positions

> **Status: planned**
> **Depends on:** scalar patterns, struct destructuring, and struct-enum patterns (all implemented)

A field in a struct or struct-enum pattern may itself carry a
sub-pattern instead of (or in addition to) a binding variable.

#### Syntax

```
field-binding  ::= Identifier                     // bind to variable
                 | Identifier ':' pattern          // sub-pattern (no binding)
                 | Identifier ':' Identifier  // sub-pattern bind
```

```loft
enum Status { Pending, Paid, Refunded }
struct Order { id: integer, status: Status, amount: float }

match order {
    Order { status: Paid,     amount } if amount > 0.0 => charge(amount)
    Order { status: Refunded, amount }                 => refund(amount)
    Order { status: Pending }                          => queue()
}

// nested struct enum inside struct
match event {
    Event { source: Http { method: "GET" } } => handle_get()
    Event { source: Http { method: "POST" } } => handle_post()
    _ => ignore()
}
```

#### Semantics

A field with a `:` sub-pattern generates an **additional condition** for
the arm rather than a binding variable.  The arm fires only when both
the primary condition (discriminant) and all field sub-conditions are
satisfied.

Sub-pattern kinds allowed in field positions:
- Enum variant name (e.g. `Paid`) — equality on discriminant
- Enum variant name with bindings (`Paid { amount }`) — discriminant + field binds
- Scalar literal / range
- Or-pattern
- Nested struct
- `_` — always true (equivalent to no sub-pattern)

**Exhaustiveness**: a field sub-pattern reduces the arm's coverage.  An
arm `Order { status: Paid }` does NOT cover `status: Pending` or
`status: Refunded`.  Exhaustiveness for the top-level subject is
satisfied only by an arm with no field constraints (or a wildcard).

#### IR Lowering

Field sub-patterns fold into the arm condition with `&&`:

```
// Order { status: Paid, amount } if amount > 0.0 => body
// Assume status field is at offset s_off, discriminant Paid = 2

cond = OpAndBool(
  OpEqInt(OpGetEnum(subject, s_off), 2),    // status == Paid
  outer_discriminant_cond                    // Order variant (if struct-enum)
)
// guard comes on top (see guard clause semantics)
```

For a deeply nested struct:

```
// { source: Http { method: "GET" } }
// outer cond: source variant == Http
// inner cond: method == "GET"

outer_cond = OpEqInt(disc_of_source, Http_disc)
inner_cond = OpEqText(get_field(Http_nr, method_attr, get_field(Event_nr, source_attr, subject)), "GET")
cond = OpAndBool(outer_cond, inner_cond)
```

The field-read chain uses `get_field` recursively; each level returns a
reference suitable as the subject for the next level.

#### Implementation Notes

Extend the field-binding parser (currently used by T1-4 struct-enum):

```rust
let bound_name = self.lexer.has_identifier();
if self.lexer.has_token(":") {
    // Sub-pattern: parse without creating a binding variable.
    let field_subject = get_field(variant_nr, attr_idx, subject.clone());
    let sub_cond = self.parse_sub_pattern(field_subject, field_type);
    extra_conds.push(sub_cond);
} else {
    // Plain binding: existing code path.
    let v_nr = self.create_var(&bound_name, &field_type);
    arm_stmts.push(v_set(v_nr, get_field(variant_nr, attr_idx, subject.clone())));
}
```

`parse_sub_pattern(subject_val, subject_type)` is a recursive entry
point into the pattern machinery, returning a boolean `Value` condition.
This function reuses the same dispatch as the top-level `parse_match`
pattern parser but returns a condition expression rather than building
an arm.

#### Edge Cases

| Case | Behaviour |
|---|---|
| Sub-pattern type mismatch | Compile error |
| Sub-pattern binds a field that is itself a binding name | Field-level binding via sub-pattern ok |
| Deeply nested (3+ levels) | Works via recursive `parse_sub_pattern` |
| Or-pattern in field position (`status: Paid \| Refunded`) | Allowed; `OpOrBool` of two discriminant checks |
| Wildcard sub-pattern (`status: _`) | No condition generated; treated same as not specifying the field |

#### Test Plan

| Test | Coverage |
|---|---|
| `nested_enum_field` | `{ status: Paid } =>` enum sub-pattern in struct field |
| `nested_scalar_field` | `{ code: 200 } =>` scalar sub-pattern |
| `nested_or_field` | `{ status: Paid \| Refunded } =>` or in field |
| `nested_two_fields` | `{ a: 1, b: 2 } =>` multiple field sub-patterns |
| `nested_struct_in_struct` | Two-level struct nesting |
| `nested_struct_enum_in_struct` | Struct containing a struct-enum field |
| `nested_exhaustiveness` | Field sub-patterns do not contribute to top-level coverage |

## See also

- [PLANNING.md](PLANNING.md) — L2 backlog entry with effort and target milestone
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — #26 (guarded arms and exhaustiveness)
- [LOFT.md](LOFT.md) — Match expression syntax reference
- [DEVELOPMENT.md](DEVELOPMENT.md) — Commit ordering and CI validation
