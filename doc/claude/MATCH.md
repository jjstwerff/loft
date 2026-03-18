// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Match Expression Design (T1-4)

> **Status: implemented** — 17/17 tests pass; shipped 2026-03-16.
> Tracking item: T1-4 (removed from PLANNING.md on completion).

A pattern-matching expression for dispatching on enum values with
compiler-checked exhaustiveness.

---

## Contents

**Implemented (T1-4)**
- [Goals](#goals)
- [Syntax](#syntax)
- [Semantics](#semantics)
- [Relationship to Polymorphic Dispatch](#relationship-to-polymorphic-dispatch)
- [Type System Internals](#type-system-internals)
- [IR Lowering](#ir-lowering)
- [Edge Cases](#edge-cases)
- [Test Coverage](#test-coverage)
- [Non-Goals (T1-4)](#non-goals-t1-4)

**Planned Extensions**
- [T1-14 Scalar patterns](#t1-14--scalar-patterns)
- [T1-15 Or-patterns (`|`)](#t1-15--or-patterns-)
- [T1-16 Guard clauses (`if`)](#t1-16--guard-clauses-if)
- [T1-17 Range patterns](#t1-17--range-patterns)
- [T1-18 Plain struct destructuring](#t1-18--plain-struct-destructuring)
- [L2 Nested patterns in field positions](#t1-19--nested-patterns-in-field-positions)
- [T1-20 Remaining patterns (null, binding)](#t1-20--remaining-patterns-null-binding)
- [T1-21 Slice and vector patterns](#t1-21--slice-and-vector-patterns)

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

## Planned Extensions

The sections below describe future additions to the `match` expression,
each corresponding to a backlog ticket.  All are **planned** — not yet
implemented.

- [T1-14 Scalar patterns](#t1-14--scalar-patterns)
- [T1-15 Or-patterns (`|`)](#t1-15--or-patterns-)
- [T1-16 Guard clauses (`if`)](#t1-16--guard-clauses-if)
- [T1-17 Range patterns](#t1-17--range-patterns)
- [T1-18 Plain struct destructuring](#t1-18--plain-struct-destructuring)
- [L2 Nested patterns in field positions](#t1-19--nested-patterns-in-field-positions)
- [T1-20 Remaining patterns (null, binding)](#t1-20--remaining-patterns-null-binding)
- [T1-21 Slice and vector patterns](#t1-21--slice-and-vector-patterns)

---

### T1-14  Scalar patterns

> **Status: planned**

Match on integer, long, float, single, text, boolean, and character
literal values.

#### Syntax

```
scalar-pattern ::= integer-literal
                 | long-literal
                 | float-literal
                 | 'true' | 'false'
                 | text-literal
                 | character-literal
```

```loft
// integer
grade = match score {
    100     => "perfect"
    90..=99 => "A"        // (range — T1-17)
    _       => "below A"
}

// text
msg = match command {
    "quit"   => "bye"
    "help"   => show_help()
    _        => "unknown"
}

// boolean
match flag {
    true  => enable()
    false => disable()
}
```

#### Semantics

**Integer, long, float, single, character** — equality test against a
literal value.  Exhaustiveness is not checkable (infinite domain);
a wildcard `_` is required or the result is nullable.

**Text** — lexicographic equality via `OpEqText`.  Same exhaustiveness
rule as integer.

**Boolean** — two variants (`true`, `false`).  When both are covered the
match is exhaustive (no wildcard required).  A duplicate arm (`true`
twice) produces a warning.

**Float / single** — warn at compile time that floating-point equality is
unreliable (NaN is never equal to itself).  The match still compiles.

`null` patterns are covered separately in T1-20.

#### IR Lowering

Subject is stored in a temp var (re-evaluation avoidance, same as plain
enums):

```
// match score { 100 => a, _ => b }
Block([
  Set(tmp, score_expr),
  If(OpEqInt(Var(tmp), 100), a, b)
])
```

Comparison ops by type:

| Type | Op |
|---|---|
| `integer` | `OpEqInt` |
| `long` | `OpEqLong` |
| `float` | `OpEqFloat` |
| `single` | `OpEqSingle` |
| `text` | `OpEqText` |
| `boolean` (true) | `Var(tmp)` directly |
| `boolean` (false) | `OpNot(Var(tmp))` |
| `character` | `OpEqInt` (character is a byte) |

#### Implementation Notes

**File: `src/parser/control.rs`**

`parse_match` currently branches on `Type::Enum` / `Type::Reference`.
Extend the subject-type dispatch to scalar types:

```rust
match &subject_type {
    Type::Enum(nr, s, _)          => { /* existing */ }
    Type::Reference(d_nr, _)
        if data.def_type(*d_nr) == DefType::EnumValue => { /* existing */ }
    Type::Integer(_, _)
    | Type::Long
    | Type::Float
    | Type::Single
    | Type::Character             => parse_scalar_match(subject_type),
    Type::Text(_)                 => parse_text_match(),
    Type::Boolean                 => parse_boolean_match(),
    _ => { diagnostic!(..., "match requires an enum or scalar type"); }
}
```

Within the arm loop, pattern parsing dispatches on subject type:
- Integer/long/float/single/character: call `self.lexer.has_integer()` /
  `has_float()` / etc.
- Text: call `self.lexer.has_string()`.
- Boolean: call `self.lexer.has_token("true")` / `has_token("false")`.

Exhaustiveness tracking for boolean: maintain a `bool has_true`, `bool
has_false` set alongside `has_wildcard`.

#### Edge Cases

| Case | Behaviour |
|---|---|
| Integer match, no wildcard | Result is nullable (no error); consider a warning |
| Float match | Compile warning: floating-point equality may mismatch NaN |
| Boolean, both arms | Exhaustive — wildcard arm after is "unreachable arm" warning |
| Literal type mismatch (e.g. float literal in integer match) | Compile error |
| Duplicate scalar arm | Warning: unreachable arm |

#### Test Plan

| Test | Coverage |
|---|---|
| `scalar_integer_exact` | `match n { 0 => ..., 1 => ..., _ => ... }` |
| `scalar_text_dispatch` | `match s { "a" => ..., "b" => ..., _ => ... }` |
| `scalar_bool_exhaustive` | `match b { true => ..., false => ... }` — no wildcard |
| `scalar_bool_wildcard` | `match b { true => ..., _ => ... }` |
| `scalar_float_warn` | Float literal arm → compile warning |
| `scalar_duplicate_arm` | Two identical integer arms → warning |
| `scalar_missing_wildcard` | Integer match, no wildcard → nullable result |

---

### T1-15  Or-patterns (`|`)

> **Status: planned**
> **Depends on:** T1-14 (needed for scalar or-patterns)

Multiple patterns for one arm, separated by `|`.  The arm fires when
any alternative matches.

#### Syntax

```
pattern ::= single-pattern { '|' single-pattern }
```

```loft
match direction {
    North | South => "vertical"
    East  | West  => "horizontal"
}

match code {
    200 | 201 | 204 => "success"
    400 | 404       => "client error"
    _               => "other"
}

match c {
    'a' | 'e' | 'i' | 'o' | 'u' => "vowel"
    _                            => "consonant"
}
```

#### Semantics

Patterns are tested left to right.  The first alternative that matches
fires the arm body.  All alternatives must be of the same pattern kind
(all enum variants, all integer literals, all text literals, etc.).

**Exhaustiveness**: for enum subjects, each variant in a disjunction
is added to the coverage set individually.

**Guards** (T1-16) apply to the whole disjunction — a guard after
`|`-patterns must pass for any of them to fire.

**Or-patterns in field positions** (L2): allowed for sub-patterns,
e.g. `{ status: Paid | Refunded }`.

#### IR Lowering

The combined condition is a right-fold of `||`:

```
// match d { North | South => a, _ => b }
If(
  call_op("||", cmp_North, cmp_South),
  a,
  b
)
```

which compiles to:

```
If(OpOrBool(OpEqInt(d, 1), OpEqInt(d, 3)),   // 1=North, 3=South
   a,
   b)
```

For three or more alternatives: `cmp_A || (cmp_B || cmp_C)` — right-fold.

#### Implementation Notes

**Structural refactor of `parse_match`** — the `arms` vector must store
a pre-built condition `Value` rather than a raw discriminant `i32`:

```rust
// Before T1-15:
arms: Vec<(Option<i32>, Value, Type)>   // (disc_nr | wildcard, body, type)

// After T1-15:
arms: Vec<(Option<Value>, Value, Type)> // (cond | wildcard, body, type)
```

In the arm pattern loop:

```rust
let mut cond: Value = build_pattern_cond(first_pattern);
while self.lexer.has_token("|") {
    let next_cond = build_pattern_cond(self.parse_next_pattern());
    cond = self.call_op_value("or", cond, next_cond);
}
```

`build_pattern_cond` is a helper that returns a `Value` boolean
expression for a single pattern against the subject temp var.

For enum or-patterns, each variant's `def_nr` is added to `covered`.

#### Edge Cases

| Case | Behaviour |
|---|---|
| Mixed pattern kinds (`North \| 42`) | Compile error: type mismatch in or-pattern |
| All enum variants via `\|` | Exhaustive — same as listing them individually |
| Duplicate in disjunction (`North \| North`) | Warning: redundant alternative |
| Wildcard in or-pattern (`North \| _`) | Warning: `_` makes earlier alternatives unreachable; reduce to `_` |
| Or-pattern with guard (T1-16) | Guard applies to the whole disjunction |

#### Test Plan

| Test | Coverage |
|---|---|
| `or_enum_two` | `North \| South => "v"` |
| `or_enum_exhaustive` | `N \| S` and `E \| W` — all 4 covered |
| `or_scalar_int` | `1 \| 2 \| 3 => "small"` |
| `or_scalar_text` | `"yes" \| "y" \| "1" => true` |
| `or_mixed_error` | `North \| 42` → compile error |
| `or_duplicate` | `North \| North` → warning |
| `or_wildcard` | `North \| _` → warning |

---

### T1-16  Guard clauses (`if`)

> **Status: implemented** — 7 tests pass; shipped 2026-03-17.

An optional boolean condition after the pattern.  If the guard fails,
the arm does not fire and matching continues with the next arm.

#### Syntax

```
arm ::= pattern [ 'if' expression ] '=>' expression
```

```loft
match shape {
    Circle { radius } if radius > 0.0 => PI * radius * radius
    Circle { radius }                 => 0.0   // zero or negative radius
    Rect { width, height }            => width * height
    Square { side }                   => side * side
}

match score {
    n if n >= 90 => "A"
    n if n >= 80 => "B"
    _            => "C"
}
```

#### Semantics

A guard is a boolean expression evaluated after the pattern matches and
any field bindings are set up.  Field-bound variables are in scope for
the guard expression.

**Exhaustiveness**: an arm with a guard is **not counted** in the
coverage set, because the guard may fail.  Exhaustiveness is only
satisfied by unconditional arms or a wildcard.

**Fall-through**: when a guard fails, matching continues from the next
arm — the guard-fail path jumps to the same `chain_rest` as if the
pattern had not matched.

#### IR Lowering

Guard creates a nested `If` inside the pattern branch:

```
// Circle { r } if r > 0 => body_a, Circle { r } => body_b, _ => body_c
// Built right-to-left; chain_rest is the chain built so far.

If(circle_cmp,
   If(guard_r_gt_0,
      Block([r_set, body_a]),
      chain_rest_after_this_arm),   // ← fall-through on guard fail
   chain_rest_after_this_arm)       // ← pattern did not match
```

Both `false` branches of the outer `If` point to the same logical chain.
Since `Value` is a tree (not a DAG) this requires **cloning** `chain_rest`.
Cloning is acceptable because chains are small for typical match widths.

```rust
let guarded_arm = v_if(guard_cond, arm_body, chain_rest.clone());
let arm_node    = v_if(pattern_cond, guarded_arm, chain_rest);
chain = arm_node;
```

#### Implementation Notes

After parsing a pattern (and any field bindings), check for `if`:

```rust
let guard_opt = if self.lexer.has_token("if") {
    let mut guard_code = Value::Null;
    let guard_type = self.expression(&mut guard_code);
    if !self.first_pass && guard_type != Type::Boolean {
        diagnostic!(..., "guard must be boolean, got {}", ...);
    }
    Some(guard_code)
} else {
    None
};
```

In the chain-building loop (right-to-left), guarded arms clone the
rest-chain:

```rust
let node = match guard_opt {
    None        => v_if(cond, arm_body, chain.clone()),
    Some(guard) => v_if(cond, v_if(guard, arm_body, chain.clone()), chain.clone()),
};
chain = node;
```

Exhaustiveness: only insert into `covered` when `guard_opt.is_none()`.

#### Edge Cases

| Case | Behaviour |
|---|---|
| Guard on wildcard `_ if cond` | Allowed; exhaustiveness not satisfied (warn if no unconditional wildcard follows) |
| Guard type is not boolean | Compile error |
| Two guarded arms for same variant, no unconditional fallback | Not exhaustive — error if no wildcard |
| Guard references outer-scope variable | Allowed (normal expression rules) |
| Guard references field not bound in pattern | Compile error (variable not defined) |

#### Test Plan

| Test | Coverage |
|---|---|
| `guard_basic` | Single guarded arm with unconditional fallback |
| `guard_multiple` | Multiple guarded arms for same variant |
| `guard_with_binding` | Field binding used inside guard |
| `guard_non_bool` | Guard returns integer → compile error |
| `guard_wildcard` | `_ if cond` — exhaustiveness warning |
| `guard_exhaustive` | Guarded + unconditional same variant — no error |

---

### T1-17  Range patterns

> **Status: planned**
> **Depends on:** T1-14 (scalar pattern infrastructure)

Match a value that falls within a closed or half-open range.

#### Syntax

```
range-pattern ::= scalar-literal '..' scalar-literal   // exclusive end
               |  scalar-literal '..' '=' scalar-literal  // inclusive end
```

(Mirrors the for-loop range syntax.  `..=` is parsed as `..` then `=`
since `..=` is not a single token.)

```loft
match n {
    0      => "zero"
    1..=9  => "single digit"
    10..100 => "two digits"
    _      => "large"
}

match temp {
    ..=0.0  => "freezing"    // open start: -∞ to 0.0 inclusive
    0.0..20.0 => "cool"
    20.0..=37.0 => "warm"
    _       => "hot"
}

match initial {
    'a'..'n' => "first half"
    'n'..'z' => "second half (approx)"
    _        => "non-alpha"
}
```

#### Semantics

`lo..hi` matches when `lo <= subject < hi`.
`lo..=hi` matches when `lo <= subject <= hi`.

An open-start range `..=hi` (no left bound) matches from the type's
minimum (or negative infinity for floats).  An open-end range `lo..` is
not supported in patterns (use a guard instead).

Range patterns apply to: `integer`, `long`, `float`, `single`, `character`.
Text ranges use lexicographic order.

Ranges do **not** count toward exhaustiveness (the domain is not
enumerated).  A wildcard is required or the result is nullable.

Ranges may appear as alternatives in or-patterns (T1-15):
`1..=5 | 10..=15 => "valid"`.

#### IR Lowering

For `lo..=hi` with integer subject `Var(tmp)`:

```
OpAndBool(
  OpLeInt(Value::Int(lo), Var(tmp)),   // lo <= subject
  OpLeInt(Var(tmp), Value::Int(hi))    // subject <= hi
)
```

For `lo..hi` (exclusive):

```
OpAndBool(
  OpLeInt(Value::Int(lo), Var(tmp)),
  OpLtInt(Var(tmp), Value::Int(hi))
)
```

For open-start `..=hi`: omit the lower bound check (always true).

Op table:

| Type | `<=` | `<` |
|---|---|---|
| `integer` | `OpLeInt` | `OpLtInt` |
| `long` | `OpLeLong` | `OpLtLong` |
| `float` | `OpLeFloat` | `OpLtFloat` |
| `single` | `OpLeSingle` | `OpLtSingle` |
| `text` | `OpLeText` | `OpLtText` |
| `character` | `OpLeInt` | `OpLtInt` |

#### Implementation Notes

In the scalar pattern parser, after reading a literal:

```rust
let lo_val = parse_scalar_literal();
if self.lexer.has_token("..") {
    let inclusive = self.lexer.has_token("=");
    let hi_val = parse_scalar_literal();
    let lo_cmp = self.cl("OpLeXxx", &[lo_val, subject_var.clone()]);
    let hi_op  = if inclusive { "OpLeXxx" } else { "OpLtXxx" };
    let hi_cmp = self.cl(hi_op,  &[subject_var.clone(), hi_val]);
    cond = self.cl("OpAndBool", &[lo_cmp, hi_cmp]);
} else {
    cond = self.cl("OpEqXxx", &[subject_var.clone(), lo_val]);
}
```

`OpAndBool` and its argument ordering must be confirmed from
`default/01_code.loft` — reuse the existing `&&` operator path via
`call_op("and", ...)` if `OpAndBool` is not directly named.

For open-start: detect `..` / `..=` immediately at the start of a
pattern arm (before any literal) and parse the right bound only.

#### Edge Cases

| Case | Behaviour |
|---|---|
| `lo > hi` (inverted range) | Warning: empty range, arm is unreachable |
| Single-value range `5..=5` | Equivalent to exact match; no error, but a hint may help |
| Float range boundary (NaN) | NaN comparisons always false; range containing NaN never matches |
| `..=hi` open start | Valid; matches all values ≤ hi |
| `lo..` open end | Not supported in patterns — compile error; suggest guard |
| Range in or-pattern | Allowed: `1..=5 \| 10..=15 => body` |
| Range type mismatch (float literal in integer match) | Compile error |

#### Test Plan

| Test | Coverage |
|---|---|
| `range_int_inclusive` | `1..=10 =>` basic inclusive range |
| `range_int_exclusive` | `1..10 =>` basic exclusive range |
| `range_open_start` | `..=0 =>` matches negatives and zero |
| `range_chained` | Multiple disjoint ranges covering a domain |
| `range_float` | Float range with compile warning |
| `range_inverted` | `10..=1 =>` → warning: empty range |
| `range_or` | `1..=5 \| 10..=15 =>` or-pattern with ranges |
| `range_char` | `'a'..'z' =>` character range |

---

### T1-18  Plain struct destructuring

> **Status: planned**

Match a struct value and optionally bind its fields.

#### Syntax

```
struct-pattern ::= TypeName '{' field-binding-list '}'
                 | TypeName
field-binding-list ::= field-binding { ',' field-binding }
field-binding      ::= Identifier                        // bind field to same-name var
                     | Identifier ':' pattern             // (L2: sub-pattern)
```

```loft
struct Point { x: float, y: float }
struct Circle { center: Point, radius: float }

match shape {
    Circle { center, radius } if radius > 0.0 => area(radius)
    Circle { center, radius }                 => 0.0
}

// Field rename not planned — use binding sub-patterns (L2) instead.
```

#### Semantics

A plain struct has exactly one "variant" (the struct itself), so there
is no discriminant comparison.  A `TypeName { ... }` arm always matches
(modulo guard).  `_` as a wildcard arm is redundant but harmless.

**Type check**: if the named type does not match the subject's type,
emit a compile error.  The name may also be omitted (`{ x, y }`) as an
anonymous struct pattern; the subject's type provides the field set.

**Field binding**: same mechanism as struct-enum field binding — field
variables are created with the field's type and initialised via
`get_field`.

**Exhaustiveness**: satisfied by any arm that lacks a guard, since there
is only one possible shape.  A wildcard arm after an unconditional struct
pattern produces an "unreachable arm" warning.

#### IR Lowering

No discriminant comparison needed — the arm body is wrapped directly:

```
// match p { Point { x, y } => x + y }
Block([
  Set(x, get_field(Point_nr, 0, subject)),
  Set(y, get_field(Point_nr, 1, subject)),
  OpAddFloat(Var(x), Var(y))
])
```

With a guard (T1-16):

```
If(guard_cond,
   Block([x_set, y_set, body]),
   chain_rest)
```

#### Implementation Notes

**Subject type dispatch** in `parse_match`:

```rust
Type::Reference(d_nr, _)
    if self.data.def_type(*d_nr) == DefType::Struct => {
        // plain struct match
        (u32::MAX, false /*is_struct_enum*/, true, *d_nr /*struct_nr*/)
    }
```

**Arm parsing**:
- `has_identifier()` → expect type name equal to `data.def(struct_nr).name`;
  emit error if different.
- Optionally `has_token("{")` → parse field bindings (same code path as
  struct-enum field binding in T1-4).
- No discriminant lookup.

The pattern condition `cond` is `Value::Boolean(true)` for a
no-guard unconditional arm; the arm body is emitted directly without an
`If` wrapper.

**Subject temp var**: for struct subjects (DbRef) the LIFO constraint
applies — do **not** create a temp var copy.  Use the subject reference
directly (same rule as struct-enum in T1-4).

#### Edge Cases

| Case | Behaviour |
|---|---|
| Wrong type name in pattern | Compile error |
| Anonymous `{ x, y }` (no type name) | Valid; inferred from subject type |
| Unknown field in binding | Compile error |
| Duplicate field in binding | Compile error |
| Wildcard after unconditional struct arm | Warning: unreachable arm |
| Multiple unconditional struct arms | Warning: second arm is unreachable |

#### Test Plan

| Test | Coverage |
|---|---|
| `struct_basic_bind` | `Point { x, y } => x + y` |
| `struct_named_type` | Type name verified against subject type |
| `struct_wrong_name` | Wrong type name → compile error |
| `struct_partial_bind` | Bind only some fields; others ignored |
| `struct_with_guard` | `Point { x, y } if x > 0 => ...` |
| `struct_wildcard_unreachable` | Wildcard after full struct arm → warning |
| `struct_anonymous` | `{ x, y } =>` anonymous binding |

---

### L2  Nested patterns in field positions

> **Status: planned**
> **Depends on:** T1-14 (scalar), T1-18 (struct), T1-4 (struct-enum)

A field in a struct or struct-enum pattern may itself carry a
sub-pattern instead of (or in addition to) a binding variable.

#### Syntax

```
field-binding  ::= Identifier                     // bind to variable
                 | Identifier ':' pattern          // sub-pattern (no binding)
                 | Identifier ':' 'as' Identifier  // sub-pattern bind (T1-20)
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
- Scalar literal / range (requires T1-14, T1-17)
- Or-pattern (requires T1-15)
- Nested struct (requires T1-18)
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
// guard comes on top as per T1-16
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
| Sub-pattern binds a field that is itself a binding name | Field-level binding via sub-pattern ok; T1-20 `as` binding for capturing |
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

---

### T1-20  Remaining patterns (null, binding)

> **Status: planned**
> **Depends on:** T1-14 (scalar infrastructure)

Catch-all for pattern forms that do not fit the categories above.

#### Null patterns

Match a null value explicitly:

```loft
match maybe_value {
    null => "nothing here"
    _    => "has a value"
}
```

`null` is already a KEYWORD so `has_token("null")` works.  The condition
is an equality check against the type's null sentinel:

```rust
cond = call_op("==", subject_var.clone(), Value::Null, ...)
// lowers to OpEqInt(subject, null_sentinel) for integer, etc.
```

A `null` arm does **not** contribute to type-based exhaustiveness (null
is not a variant; it is the absence of a value).  For a nullable type,
both `null` and at least one non-null arm (or wildcard) are needed to
achieve full coverage.

#### Wildcard binding

A bare identifier that is not a recognised enum variant binds the
matched subject to a new variable:

```loft
match n {
    0 => "zero"
    x => "nonzero: {x}"   // x bound to the subject value
}
```

For enum subjects, an unrecognised identifier is a compile error (not a
binding).  For scalar subjects, any identifier that is not a keyword is
treated as a wildcard binding.

```rust
// In scalar arm parsing:
if let Some(name) = self.lexer.has_identifier() {
    let v_nr = self.create_var(&name, &subject_type);
    arm_stmts.push(v_set(v_nr, subject_var.clone()));
    // No condition — always matches (wildcard with binding).
    has_wildcard = true;
}
```

The wildcard binding must be the last arm (same rule as `_`).

#### Binding patterns (`@`)

Bind the matched value AND test a sub-pattern:

```loft
match n {
    x @ 1..=10 => x * 2       // x bound to n; n must be in range
    x @ 0      => 0
    _          => -1
}
```

Syntax: `name '@' pattern`.  Lowers to:

```
If(pattern_cond,
   Block([Set(x, subject_var), body]),
   chain_rest)
```

The binding happens inside the true branch (not before the test).

`@` is not currently in TOKENS — it must be added.

#### Character patterns

Characters are integers (bytes) in loft.  Character literal patterns
use the same `OpEqInt` path as integer patterns (T1-14).

```loft
match c {
    'a' => "lower a"
    'A' => "upper A"
    _   => "other"
}
```

#### Implementation Notes

**`null` pattern**: detect `has_token("null")` in the arm loop before
`has_identifier()`.  The null arm should be allowed for any nullable
type; emit a warning for non-nullable subjects.

**Wildcard binding**: after exhausting literal/keyword checks in the
scalar arm parser, `has_identifier()` returns the binding name.
Distinguish from enum variant by checking `data.def_nr(&name)` — if
`u32::MAX` (not found), it is a binding; otherwise it is a variant.

**`@` binding**: add `"@"` to TOKENS.  In arm parsing, after reading the
binding name, check `has_token("@")` and if present parse the
sub-pattern normally.

#### Edge Cases

| Case | Behaviour |
|---|---|
| `null` arm on non-nullable subject | Warning: unreachable arm |
| Wildcard binding after `_` | Warning: unreachable arm |
| `@` binding with guard | `name @ pattern if guard` — all in scope |
| Binding name conflicts with outer variable | Shadowing (T2-10 warning when T2-10 is implemented) |
| `null` arm for enum subject | Valid — matches enum null (discriminant 0) |

#### Test Plan

| Test | Coverage |
|---|---|
| `null_pattern` | `null => ...` arm matches null subject |
| `null_non_nullable` | `null` arm on `not null` subject → warning |
| `wildcard_binding` | `x => "got {x}"` binds subject |
| `at_binding_range` | `x @ 1..=10 => x * 2` |
| `at_binding_enum` | `x @ North \| South => ...` |
| `char_pattern` | `'a' => ...` character literal |

---

### T1-21  Slice and vector patterns

> **Status: planned**
> **Depends on:** T1-14, T1-15, T1-16

Match vectors and text strings by their structure (length and element
positions).

#### Syntax

```
slice-pattern     ::= '[' slice-elements ']'
slice-elements    ::= ε
                    | slice-elem { ',' slice-elem }
slice-elem        ::= '..'                      // rest (no binding)
                    | Identifier '..'           // rest with binding (creates sub-vector)
                    | pattern                   // element pattern
```

```loft
// vector
match items {
    []                  => "empty"
    [x]                 => "single: {x}"
    [first, ..]         => "starts with {first}"
    [.., last]          => "ends with {last}"
    [a, b]              => "exactly two: {a} and {b}"
    [first, rest.., last] => "first={first}, last={last}"
}

// text (character-level)
match s {
    []         => "empty"
    [c, ..]    => "starts with '{c}'"
    [.., c]    => "ends with '{c}'"
}
```

#### Semantics

A `[` token at the start of an arm pattern introduces a slice pattern
(not a field-binding list).  The disambiguator is the subject type:
`Type::Vector` or `Type::Text` → slice pattern; otherwise `[` is a
syntax error in pattern position.

**Length tests**:
- `[]` → `OpLengthVector(v) == 0`
- `[x]` → `OpLengthVector(v) == 1`
- `[a, b]` → `OpLengthVector(v) == 2`
- `[first, ..]` → `OpLengthVector(v) >= 1`
- `[a, b, ..]` → `OpLengthVector(v) >= 2`
- `[.., last]` → `OpLengthVector(v) >= 1`
- `[a, .., z]` → `OpLengthVector(v) >= 2`

**Element bindings**: `OpGetVector(v, elem_size, index)` for vectors;
`OpTextCharacter(s, index)` for text characters.  Negative index
counting from the end: `OpGetVector(v, sz, OpMinInt(len, delta))`.

**Rest binding** (`rest..`): creates a sub-vector slice.  Initial
implementation may defer rest binding and support only `..` (skip).

**Element patterns**: any scalar pattern from T1-14 is valid for an
element position.  Or-patterns (T1-15) are allowed.

**Exhaustiveness**: not tracked — slice length is unbounded.  A wildcard
is required (or the result is nullable).

**Text slices** use character-level indexing.  `OpLengthText` for length;
`OpTextCharacter` for character access.  Only the `[]` (empty text) and
`[c, ..]` (first character) forms are particularly useful; full text
slice patterns may have limited practical value.

#### IR Lowering

```
// [first, ..] => body   (vector with at least one element)
// len = OpLengthVector(Var(subj))
// cond = OpLtInt(0, len)   (i.e. len > 0)
// first_val = OpGetVector(Var(subj), elm_size, 0)

If(cond,
   Block([
     Set(len_var, OpLengthVector(Var(subj))),
     Set(first_var, OpGetVector(Var(subj), elm_size, 0)),
     body
   ]),
   chain_rest)
```

```
// [.., last] => body   (at least one element; last at index len-1)

If(OpLtInt(0, len_var),
   Block([
     Set(len_var,  OpLengthVector(Var(subj))),
     Set(last_var, OpGetVector(Var(subj), elm_size,
                     OpMinInt(Var(len_var), Value::Int(1)))),
     body
   ]),
   chain_rest)
```

(Note: `OpMinInt(len, 1)` computes `len - 1`; the exact op name must be
verified — it may be `OpMinInt` for subtraction or a dedicated
`OpDecInt`.)

For exact-length arms, the length comparison uses `OpEqInt`:

```
If(OpEqInt(OpLengthVector(Var(subj)), Value::Int(N)), ...)
```

For text, substitute `OpLengthText` and `OpTextCharacter`.

#### Implementation Notes

**Entry point**: at the start of arm parsing, after checking for `_`,
check `has_token("[")`:

```rust
if self.lexer.has_token("[") {
    self.parse_slice_pattern(subject_var, subject_type, &mut cond, &mut arm_stmts)
}
```

`parse_slice_pattern` emits:
1. `len_var = OpLengthVector(subject_var)` (once, shared across elements).
2. Length condition depending on fixed vs. `..` positions.
3. `Set(elem_var, OpGetVector(..., index))` for each bound element.
4. Optional element sub-conditions (from element patterns in T1-14/T1-15).

**Rest binding** (`rest..`): captures elements from position `i` to
`len - j` as a new vector.  Requires either:
- A `OpGetTextSub`-style slice opcode for vectors (does not currently
  exist — needs a new `OpSliceVector`), or
- An O(n) copy loop.

For the initial implementation, `..` (no bind) is supported; `rest..` is
a compile error with message "rest binding in slice patterns not yet
supported".

**Element count validation**: more than one `..` in a pattern is a
compile error.

#### Edge Cases

| Case | Behaviour |
|---|---|
| Two `..` in pattern | Compile error |
| `[..]` alone (match any length) | Equivalent to `_`; warn as redundant |
| Element pattern type mismatch | Compile error |
| `rest..` binding | Compile error (initial — not yet implemented) |
| Zero-element vector with `[..]` | Matches (any length) |
| Text slice pattern | Supported for `[]` and `[c, ..]` forms |
| Nested slice inside slice | Not supported (patterns are flat) |

#### Test Plan

| Test | Coverage |
|---|---|
| `slice_empty` | `[] =>` matches empty vector |
| `slice_one` | `[x] =>` matches exactly one element |
| `slice_two` | `[a, b] =>` matches exactly two |
| `slice_first` | `[first, ..] =>` binds first element |
| `slice_last` | `[.., last] =>` binds last element |
| `slice_first_last` | `[a, .., z] =>` binds both ends |
| `slice_text_empty` | `[] =>` on text string |
| `slice_text_first_char` | `[c, ..] =>` on text |
| `slice_elem_pattern` | `[0, ..] =>` element with scalar sub-pattern |
| `slice_two_rest` | Two `..` → compile error |

---

## See also

- [PLANNING.md](PLANNING.md) — T1-14 through T1-21 backlog entries
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — #6 (plain enum methods)
- [DEVELOPMENT.md](DEVELOPMENT.md) — Commit ordering and CI validation
