// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Match Expression Design (T1-4)

> **Status: implemented** — 17/17 tests pass; shipped 2026-03-16.
> Tracking item: T1-4 (removed from PLANNING.md on completion).

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
- [Implementation — Phase 1: Plain Enum](#implementation--phase-1-plain-enum)
- [Implementation — Phase 2: Struct Enum Field Binding](#implementation--phase-2-struct-enum-field-binding)
- [Implementation — Phase 3: Exhaustiveness and Diagnostics](#implementation--phase-3-exhaustiveness-and-diagnostics)
- [Implementation — Phase 4: Match as Expression](#implementation--phase-4-match-as-expression)
- [Implementation — Phase 5: Hook into expression()](#implementation--phase-5-hook-into-expression)
- [Files Changed](#files-changed)
- [Edge Cases](#edge-cases)
- [Test Plan](#test-plan)
- [Non-Goals](#non-goals)

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

## Implementation — Phase 1: Plain Enum

**File: `src/parser/control.rs`**

Add `parse_match` after `parse_if`.  Restrict to plain enums first
(`is_struct == false`).

### 1a. Extract the discriminant expression

Given `subject_code: Value` of type `Type::Enum(e_nr, false, _)`:

```rust
// A plain enum stack value is already an `enumerate`.
// OpConvIntFromEnum converts it to i32 for OpEqInt.
let disc = self.cl("OpConvIntFromEnum", &[subject_code.clone()]);
```

### 1b. Parse one arm

```rust
// Consume the variant name
let pattern_name = self.lexer.has_identifier()  // or has_token("_")
// Look up the EnumValue definition
let variant_def_nr = self.data.def_nr(&pattern_name);
// Retrieve discriminant number
let discriminant: i32 = if let Value::Enum(nr, _)
    = self.data.def(variant_def_nr).attributes[0].value  // first (only) attribute
{
    i32::from(nr)
} else { 0 };
// Consume =>
self.lexer.token("=>");
// Parse the arm body as an expression
let mut arm_code = Value::Null;
let arm_type = self.expression(&mut arm_code);
```

### 1c. Build the if-chain recursively

```rust
// Start with Value::Null (the "no arm matched" base case).
// Build in reverse order so the innermost If is the last arm.
let mut chain = Value::Null;
for (disc_nr, arm_code) in arms.iter().rev() {
    let cmp = self.cl("OpEqInt", &[disc_expr.clone(), Value::Int(*disc_nr)]);
    chain = v_if(cmp, arm_code.clone(), chain);
}
```

### 1d. Full function signature

```rust
pub(crate) fn parse_match(&mut self, code: &mut Value) -> Type {
    // 1. Parse subject
    let mut subject = Value::Null;
    let subject_type = self.expression(&mut subject);
    let (e_nr, is_struct) = match &subject_type {
        Type::Enum(nr, s, _) => (*nr, *s),
        _ => {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error,
                    "match requires an enum type, got {}",
                    subject_type.name(&self.data));
            }
            return Type::Null;
        }
    };

    self.lexer.token("{");
    let mut arms: Vec<(Option<i32>, Value, Type)> = Vec::new(); // (disc, code, type)
    let mut covered: HashSet<u32> = HashSet::new();
    let mut has_wildcard = false;

    // 2. Parse arms until "}"
    loop {
        if self.lexer.peek_token("}") { break; }
        if self.lexer.has_token("_") {
            has_wildcard = true;
            self.lexer.token("=>");
            let mut arm_code = Value::Null;
            let arm_type = self.expression(&mut arm_code);
            arms.push((None, arm_code, arm_type));
            break; // wildcard must be last
        }
        let Some(pattern_name) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "expect variant name or '_'");
            break;
        };
        // ... look up variant, consume =>, parse body (see phases below)
        let (disc, arm_code, arm_type) = self.parse_plain_arm(e_nr, &pattern_name, &mut covered);
        arms.push((Some(disc), arm_code, arm_type));
    }
    self.lexer.token("}");

    // 3. Exhaustiveness (Phase 3)
    // 4. Type unification (Phase 4)
    // 5. Build if-chain
    let result_type = /* unified arm type */;
    *code = self.build_match_chain(&arms, &subject_code, is_struct, e_nr);
    result_type
}
```

---

## Implementation — Phase 2: Struct Enum Field Binding

**File: `src/parser/control.rs`**

When `is_struct == true` and the arm has a `{` after the variant name,
introduce scoped variables for each named field.

### 2a. Discriminant for struct enum

```rust
// The "enum" discriminant is at attribute index 0, field position 0
// in the reference.  OpGetEnum reads it; OpConvIntFromEnum converts.
let disc_val = self.cl("OpConvIntFromEnum",
    &[self.cl("OpGetEnum", &[subject_code.clone(), Value::Int(0)])]);
```

### 2b. Parse the optional field-binding list

```rust
// After the variant name, optionally: `{ field1, field2, ... }`
let mut bindings: Vec<String> = Vec::new();  // field names requested by the user
if self.lexer.has_token("{") {
    loop {
        let Some(fname) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "expect field name in match binding");
            break;
        };
        bindings.push(fname);
        if !self.lexer.has_token(",") { break; }
    }
    self.lexer.token("}");
}
```

### 2c. Resolve field bindings to variant attributes

Given `variant_def_nr` (the `EnumValue` definition), fields start at
`attributes[1]` (attribute 0 is the discriminant).

```rust
// Build: field_name -> (attr_index, Type)
let mut field_map: Vec<(String, usize, Type)> = Vec::new();
let variant_def = self.data.def(variant_def_nr);
for attr_idx in 1..variant_def.attributes.len() {
    let attr = &variant_def.attributes[attr_idx];
    field_map.push((attr.name.clone(), attr_idx, attr.typedef.clone()));
}
```

Validate each name in `bindings` exists in `field_map`; emit an error
for unknown field names, and an error for duplicates.

### 2d. Introduce arm-scoped variables

For each bound field, create a variable in the current scope and emit
a `Set` to read the field from the subject reference at the start of
the arm body.  Use `get_field` to emit the correct opcode for the
field's type.

```rust
let mut arm_stmts: Vec<Value> = Vec::new();
let mut arm_vars: Vec<(String, u16)> = Vec::new();  // name -> var_nr (for cleanup)

for bound_name in &bindings {
    let Some((_, attr_idx, field_type)) = field_map.iter()
        .find(|(n, _, _)| n == bound_name)
    else {
        diagnostic!(self.lexer, Level::Error,
            "variant {} has no field '{}'",
            pattern_name, bound_name);
        continue;
    };
    // Create (or reuse on second pass) a local variable.
    let v_nr = self.create_var(bound_name, field_type);
    self.vars.defined(v_nr);
    arm_vars.push((bound_name.clone(), v_nr));
    // Emit: bound_name = get_field(variant_def_nr, attr_idx, subject_code)
    let field_read = self.get_field(
        variant_def_nr,
        *attr_idx,
        subject_code.clone(),
    );
    arm_stmts.push(v_set(v_nr, field_read));
}
```

### 2e. Parse the arm body and wrap with field-read prefix

```rust
self.lexer.token("=>");
let mut body = Value::Null;
let body_type = self.expression(&mut body);

// Prepend field-read stmts to the arm body via a Block.
arm_stmts.push(body);
let arm_block = v_block(arm_stmts, body_type.clone(), "match_arm");
```

**Note on variable scoping**: `add_variable` in `variables.rs` reuses
an existing name on the second parse pass (returns the same `u16`).
This is safe because (1) on the first pass, the variable is created
with the field's type, and (2) field-bound variables from different
arms of the same match have different names, so they do not collide.

For variables whose names clash with an outer-scope variable, the
existing shadowing behaviour applies (INCONSISTENCIES #10 / T2-10).

---

## Implementation — Phase 3: Exhaustiveness and Diagnostics

**File: `src/parser/definitions.rs`** — extract helper

```rust
/// Returns the names of variants not present in `covered_defs`.
pub(crate) fn missing_variants(
    &self,
    e_nr: u32,
    covered_defs: &HashSet<u32>,
) -> Vec<String> {
    self.data.definitions.iter().enumerate()
        .filter(|(_, d)| d.def_type == DefType::EnumValue && d.parent == e_nr)
        .filter(|(v_nr, _)| !covered_defs.contains(&(*v_nr as u32)))
        .map(|(_, d)| d.name.clone())
        .collect()
}
```

The existing `warn_missing_enum_variants` calls this and emits a
**Warning**.  `parse_match` calls it and emits an **Error** when no
wildcard is present.

**File: `src/parser/control.rs`** — in `parse_match`, after the arm loop:

```rust
// Exhaustiveness check (only on second pass to avoid double errors).
if !self.first_pass && !has_wildcard {
    let missing = self.missing_variants(e_nr, &covered);
    if !missing.is_empty() {
        diagnostic!(
            self.lexer,
            Level::Error,
            "match on {} is not exhaustive — missing: {}",
            self.data.def(e_nr).name,
            missing.join(", ")
        );
    }
}

// Duplicate-arm warning.
// `covered` is a HashSet<u32>; duplicates are detected separately
// with a Vec that allows repetition.
```

Duplicate detection: maintain a `Vec<u32>` (ordered arm variant defs)
alongside the `HashSet`.  After parsing each arm, if `covered` already
contains the def_nr, emit a warning before inserting.

```rust
if covered.contains(&variant_def_nr) {
    diagnostic!(
        self.lexer,
        Level::Warning,
        "unreachable arm: {} already matched",
        pattern_name
    );
} else {
    covered.insert(variant_def_nr);
}
```

---

## Implementation — Phase 4: Match as Expression

All arm types must unify.  After the arm loop:

```rust
// Collect all arm types (skip void arms — they were used as statements).
let mut result_type = Type::Void;
for (_, _, arm_type) in &arms {
    if matches!(arm_type, Type::Void) {
        continue; // statement arm — fine
    }
    if matches!(result_type, Type::Void) {
        result_type = arm_type.clone();
    } else {
        let unified = merge_dependencies(&result_type, arm_type);
        if !self.first_pass && unified == Type::Void {
            // merge_dependencies returns the first type when types differ;
            // detect an actual mismatch by comparing the two types.
            if !result_type.compatible(arm_type) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot unify match arm types: {} and {}",
                    result_type.name(&self.data),
                    arm_type.name(&self.data)
                );
            }
        }
        result_type = unified;
    }
}
```

`compatible` already exists on `Type` (`src/data.rs:285`); it returns
true for types that are assignment-compatible.

---

## Implementation — Phase 5: Hook into `expression()`

**File: `src/parser/expressions.rs`**

In the `expression` function, add the `match` branch alongside `if`:

```rust
} else if self.lexer.has_token("if") {
    self.parse_if(val)
} else if self.lexer.has_token("match") {   // ← add this
    self.parse_match(val)
} else if self.lexer.has_token("fn") {
```

`match` is valid in any expression position: right-hand side of
assignment, function argument, last expression in a block, etc.

---

## Files Changed

| File | Change | ~Lines |
|---|---|---|
| `src/lexer.rs` | Add `"match"` to `KEYWORDS` (**done in commit 2a**) | 1 |
| `src/parser/definitions.rs` | Extract `missing_variants` helper | ~15 |
| `src/parser/control.rs` | Add `parse_match` | ~140 |
| `src/parser/expressions.rs` | Hook `has_token("match")` in `expression()` | 3 |
| `tests/match.rs` | Integration tests | ~355 |

No changes needed in `src/fill.rs`, `src/state/codegen.rs`, or
`src/data.rs` — the if-chain lowering reuses all existing infrastructure.

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

---

## Test Plan

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

## Non-Goals

- **Guard clauses**: `Circle { r } if r > 1.0 => ...` — use an `if`
  inside the arm body.
- **Or-patterns**: `North | South => ...` — not planned for T1-4.
- **Binding by mutable reference**: field bindings are read-only copies.
- **Tuple/struct destructuring**: patterns on non-enum types are out of scope.

---

## See also

- [PLANNING.md](PLANNING.md) — T1-4 backlog entry
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — #6 (plain enum methods)
- [DEVELOPMENT.md](DEVELOPMENT.md) — Commit ordering and CI validation
