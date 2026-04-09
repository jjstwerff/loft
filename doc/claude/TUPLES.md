
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Tuple Design

> **Status: completed in 0.8.3.** T1.1–T1.7 implemented; tuple-returning
> functions (T1.4) and LHS destructuring remain deferred.

Tuples are anonymous, fixed-arity, stack-allocated compound value types for
returning multiple values without defining a named struct.

---

## Syntax

```loft
// Type notation — two or more element types
(integer, text)
(float, float, boolean)

// Function return
fn min_max(v: vector<integer>) -> (integer, integer) {
    (min_val, max_val)
}

// Literal
t = (1, "hello")

// Element access (zero-based integer literal)
a = foo()       // a: (integer, text)
b = a.0         // integer
c = a.1         // text

// Element assignment
a.0 = 5
a.0 += 3

// LHS deconstruction (deferred)
(lo, hi) = min_max(values)
```

---

## Memory layout

Each tuple is a contiguous stack region. Elements are laid out in declaration
order, each naturally aligned. Total size = sum of `element_size(T_i)`.

Text elements (`text`, `text not null`) use full `String` (24 bytes) when owned
by the tuple. Argument-passed text uses `Str` (16 bytes) per the standard
calling convention.

---

## Known limitations

| ID | Issue | Resolution |
|---|---|---|
| SC-1 | Text element use-after-free on return | Caller-allocated slots |
| SC-2 | Text double-free on tuple copy | Deep-copy text elements |
| SC-5 | LIFO store violation on scope exit | Reverse element free order |
| SC-7 | `not null` inaccessible for tuple integers | `integer not null` annotation |

---

## Non-goals

Named tuple fields, single-element tuples, tuples in struct fields, tuple
iteration, whole-tuple formatting, variadic tuples — all compile errors.
Use named structs or element-by-element access instead.

---

## Deferred work

- **T1.4** — Tuple-returning functions (caller-allocated return slots)
- **LHS destructuring** — `(a, b) = expr` syntax
- **Tuple patterns in match** — destructure before `match` for now
- **`&tuple` with owned elements** — per-element DbRef expansion

---

## See also

- [LOFT.md](LOFT.md) — language reference
- [INTERMEDIATE.md](INTERMEDIATE.md) — `Type`/`Value` enums, stack layout
- [SLOTS.md](SLOTS.md) — slot assignment for owned elements

---

# Tuple Destructuring in `match` (T1.9)

Design for `match` expressions whose subject is a `Type::Tuple`.

## Contents
- [Current State and Dependencies](#current-state-and-dependencies)
- [Goals](#goals)
- [Syntax](#syntax)
- [Element Pattern Forms](#element-pattern-forms)
- [Exhaustiveness](#exhaustiveness)
- [IR Lowering](#ir-lowering)
- [Implementation Plan](#implementation-plan)
- [Edge Cases](#edge-cases)
- [Test Plan](#test-plan)

---

## Current State and Dependencies

`Type::Tuple` is fully implemented (T1.1–T1.7, 0.8.3). The following works today:

```loft
t: (integer, text) = (42, "hello")
(a, b) = t              // LHS destructuring — works
x = t.0                 // element access — works
t.1 = "world"           // element assignment — works
```

`parse_match` dispatches on the subject type. `Type::Tuple` falls into the catch-all
and emits "match requires an enum, struct, or scalar type" — not yet handled.

**Dependency on T1.8:** Tuple-returning functions (`-> (A, B)`) are deferred as T1.8.
Tuple match on a function call result (`match foo() { ... }`) requires T1.8a first.
Tuple match on a local variable, parameter, or literal is independent and can land now.

---

## Goals

- Allow `match` to destructure and dispatch on tuple subjects.
- Support all element pattern forms that exist for scalar match: wildcards, binding
  variables, literals, ranges, or-patterns, and `null`.
- Support nested tuple patterns for tuple-valued elements.
- Exhaustiveness: a compile error when no arm is total (catches all cases).
- Guards work the same way as for enum and struct match.

---

## Syntax

```
tuple-match     ::= 'match' tuple-expr '{' tuple-arm+ '}'

tuple-arm       ::= tuple-pattern [ guard ] '->' expression

tuple-pattern   ::= '_'                                  // total wildcard
                  | '(' elem-pattern { ',' elem-pattern } ')'

elem-pattern    ::= '_'                                  // element wildcard
                  | identifier                           // binding variable
                  | literal                              // exact match
                  | literal '..' literal                 // range (exclusive)
                  | literal '..=' literal                // range (inclusive)
                  | elem-pattern '|' elem-pattern        // or-pattern
                  | '(' elem-pattern { ',' elem-pattern } ')'  // nested tuple
                  | 'null'                               // null match

guard           ::= 'if' expression
```

Arms are separated by newlines; `;` is also accepted as a separator.

### Examples

```loft
t: (integer, text) = (42, "hello")

// Basic binding and wildcard
match t {
    (0, msg)  -> println("zero: {msg}")
    (n, "")   -> println("empty text at {n}")
    (n, msg)  -> println("{n}: {msg}")
}

// Range on first element
match t {
    (0..10, _) -> println("single digit")
    (10..100, name) -> println("two digits: {name}")
    _ -> println("large or negative")
}

// Or-pattern in element position
match t {
    (1 | 2 | 3, _) -> println("one, two, or three")
    (0, _) | (_, "") -> println("zero or empty")  // ERROR — arm-level | not supported
    _ -> println("other")
}

// Nested tuple
coords: ((float, float), boolean) = ((1.0, 2.0), true)
match coords {
    ((0.0, 0.0), _)    -> println("origin")
    ((x, y), true)     -> println("active: {x},{y}")
    ((x, y), false)    -> println("inactive: {x},{y}")
}

// Guard
match t {
    (n, msg) if n > 100 -> println("large: {msg}")
    (n, msg)            -> println("normal: {n} {msg}")
}
```

### What is NOT supported (T1.9 scope)

- **Or-patterns at arm level**: `(1, _) | (2, _) -> ...` — only `|` inside an element
  position is supported (reuses existing scalar or-pattern). A full arm-level or-pattern
  requires restructuring the arm-building loop (deferred to T1.10 if needed).
- **Rest patterns**: `(first, ..)` — tuple arity is fixed and known at compile time;
  no `..` rest needed (unlike vector/slice match).
- **Match on tuple-returning function calls**: `match foo() { ... }` — requires T1.8a
  (tuple function return convention). The subject must be a tuple variable or parameter.

---

## Element Pattern Forms

### Wildcard `_`

No condition generated. No binding. The arm does not become total from this element
alone, but a `_` at every position (or a top-level `_` arm) makes the arm total.

### Binding variable `name`

No condition generated. A new variable `name` of the element's type is declared and
bound to `tuple_var.i` at the start of the arm body.

```loft
(n, msg) ->          // n bound to t.0, msg bound to t.1
```

Binding a name that already exists in scope re-uses that variable if the type matches;
otherwise creates a new one (same rule as LHS destructuring).

A `_` identifier is treated as a wildcard, not a binding (consistent with the rest of
the language).

### Literal `42`, `"hello"`, `true`

Generates an equality condition: `tuple_var.i == literal`.

The literal type must be compatible with the element type (same rules as scalar match).

### Range `lo..hi` / `lo..=hi`

Generates a range condition: `tuple_var.i >= lo && tuple_var.i < hi` (exclusive) or
`tuple_var.i >= lo && tuple_var.i <= hi` (inclusive).

Reuses `parse_match_pattern` which already handles ranges for scalar match.

### Or-pattern `p1 | p2 | p3`

Generates an or-condition: `cond(p1) || cond(p2) || cond(p3)`.

Reuses the existing or-pattern infrastructure in `parse_match_pattern`.

### `null`

Generates a null check. Only valid for nullable element types.

### Nested tuple `(p1, p2)`

When the element type is itself a `Type::Tuple`, the pattern may be a nested
`(...)`. Recursive call to the element-pattern parser for each sub-element.

Generates a conjunction of sub-element conditions, ANDed into the outer arm condition.

---

## Exhaustiveness

Tuple values cannot be enumerated (unlike enum variants), so exhaustiveness is checked
via a simpler rule:

> **A tuple match is exhaustive if at least one arm is *total*.**

An arm is *total* when:
- It is a bare `_` wildcard, OR
- Every element position is either `_` or a plain binding variable (no condition).

A guarded arm (`if cond`) is never total — the guard may fail at runtime.

If no arm is total, a **compile-time error** is emitted:
```
error: tuple match is not exhaustive — add a wildcard arm `_` or an all-binding
       arm `(a, b, ...)` to cover all cases
```

This matches the exhaustiveness model for scalar match and vector/slice match.

---

## IR Lowering

### Subject storage

The subject tuple is stored in a temp variable to avoid re-evaluation and to give
`TupleGet` a stable `var_nr`:

```rust
let v = self.create_unique("match_subj", subject_type);  // u16 var_nr
self.vars.defined(v);
// preamble: Set(v, subject_expr)
```

### Per-arm IR shape

Each arm lowers to a block:

```
[
  // 1. Check arm condition (conjunction of element conditions)
  If(cond,
    Block([
      // 2. Bind variables: Set(name_var, TupleGet(v, i)) for each binding
      Set(n_var, TupleGet(v, 0)),
      Set(msg_var, TupleGet(v, 1)),
      // 3. Optional guard as inner if
      If(guard_cond, body, <next_arm>)
    ]),
    <next_arm>
  )
]
```

When no element conditions exist (total arm), the outer `If` is omitted and the body
runs unconditionally (with only bindings + guard if present).

### Example lowering

```loft
match t {
    (0, msg) -> println("zero: {msg}")
    (n, _)   -> println("{n}")
}
```

Lowers to:

```
tmp_v = t                          // Set(v, subject)
If(
  OpEqInt(TupleGet(v, 0), 0),      // t.0 == 0
  Block([
    Set(msg_var, TupleGet(v, 1)),  // bind msg
    println("zero: {msg_var}")
  ]),
  Block([
    Set(n_var, TupleGet(v, 0)),    // bind n (wildcard on t.1 — no binding)
    println("{n_var}")
  ])
)
```

### Nested tuple lowering

```loft
match coords {                    // coords: ((float, float), boolean)
    ((0.0, 0.0), _) -> "origin"
    ((x, y), active) -> "{x},{y},{active}"
}
```

Lowers to:

```
tmp_v = coords
If(
  OpAndBool(
    OpEqFloat(TupleGet(TupleGet(v, 0), 0), 0.0),   // coords.0.0 == 0.0
    OpEqFloat(TupleGet(TupleGet(v, 0), 1), 0.0)    // coords.0.1 == 0.0
  ),
  "origin",
  Block([
    Set(x_var,      TupleGet(TupleGet(v, 0), 0)),  // bind x
    Set(y_var,      TupleGet(TupleGet(v, 0), 1)),  // bind y
    Set(active_var, TupleGet(v, 1)),               // bind active
    "{x_var},{y_var},{active_var}"
  ])
)
```

**Note on nested TupleGet:** `TupleGet(TupleGet(v, 0), 0)` reads element 0 of the inner
tuple. The codegen for `TupleGet` already handles this by computing the byte offset of
the element within the outer tuple's stack slot. A nested `TupleGet(inner, i)` where
`inner` is another `TupleGet` chains the offset correctly.

---

## Implementation Plan

All changes are in `src/parser/control.rs`.

### Step T1.9-1 — Dispatch in `parse_match`

In the subject-type match inside `parse_match`:

```rust
Type::Tuple(_) => {
    return self.parse_tuple_match(subject, &subject_type, code);
}
```

This goes in the existing dispatch block alongside the vector and scalar dispatches
(lines ~324–337).

### Step T1.9-2 — `parse_tuple_match` (new function)

```rust
fn parse_tuple_match(
    &mut self,
    subject: Value,
    subject_type: &Type,
    code: &mut Value,
) -> Type {
    let Type::Tuple(elem_types) = subject_type.clone() else { unreachable!() };
    let arity = elem_types.len();

    // Store subject in temp — gives TupleGet a stable var_nr.
    let v = self.create_unique("match_subj", subject_type);
    self.vars.defined(v);

    self.lexer.token("{");
    let mut result_type = Type::Void;
    struct TupleArm { cond: Option<Value>, bindings: Vec<Value>, guard: Option<Value>, body: Value }
    let mut arms: Vec<TupleArm> = Vec::new();
    let mut has_wildcard = false;
    let match_pos = self.lexer.pos().clone();

    loop {
        if self.lexer.peek_token("}") { break; }

        if self.lexer.has_token("_") {
            // Total wildcard arm
            has_wildcard = true;
            let guard = self.parse_optional_guard();
            self.lexer.token("->");
            let mut body = Value::Null;
            let bt = self.expression(&mut body);
            result_type = self.merge_types(result_type, bt);
            arms.push(TupleArm { cond: None, bindings: vec![], guard, body });
        } else {
            // Tuple pattern arm
            self.lexer.token("(");
            let mut arm_cond: Option<Value> = None;
            let mut arm_bindings: Vec<Value> = Vec::new();

            for i in 0..arity {
                if i > 0 { self.lexer.token(","); }
                self.parse_tuple_elem_pattern(
                    v, i as u16, &elem_types[i].clone(),
                    &mut arm_cond, &mut arm_bindings,
                );
            }
            self.lexer.token(")");

            // A pattern with no conditions is total.
            if arm_cond.is_none() {
                has_wildcard = true;
            }

            let guard = self.parse_optional_guard();
            // Guarded arm — even if all-binding, not total.
            if guard.is_some() && arm_cond.is_none() {
                has_wildcard = false;  // guard may fail
            }

            self.lexer.token("->");
            let mut body = Value::Null;
            let bt = self.expression(&mut body);
            result_type = self.merge_types(result_type, bt);
            arms.push(TupleArm { cond: arm_cond, bindings: arm_bindings, guard, body });
        }

        self.lexer.has_token(";");
    }

    self.lexer.token("}");

    if !has_wildcard && !self.first_pass {
        diagnostic_at!(
            self.lexer, match_pos, Level::Error,
            "tuple match is not exhaustive — add a wildcard arm `_` or \
             an all-binding arm `({})` to cover all cases",
            elem_types.iter().map(|_| "_").collect::<Vec<_>>().join(", ")
        );
    }

    // Build if-chain from last arm to first.
    let mut chain = Value::Null;
    for arm in arms.into_iter().rev() {
        let arm_body = if arm.bindings.is_empty() && arm.guard.is_none() {
            arm.body
        } else {
            let mut stmts = arm.bindings;
            let body = if let Some(guard) = arm.guard {
                // guard failure falls through to chain (the following arms)
                v_if(guard, arm.body, chain.clone())
            } else {
                arm.body
            };
            stmts.push(body);
            v_block(stmts, result_type.clone(), "tuple arm")
        };

        chain = if let Some(cond) = arm.cond {
            v_if(cond, arm_body, chain)
        } else {
            arm_body
        };
    }

    let preamble = Value::Set(v, Box::new(subject));
    *code = v_block(vec![preamble, chain], result_type.clone(), "tuple_match");
    result_type
}
```

### Step T1.9-3 — `parse_tuple_elem_pattern` (new function)

```rust
fn parse_tuple_elem_pattern(
    &mut self,
    tuple_var: u16,
    idx: u16,
    elem_type: &Type,
    arm_cond: &mut Option<Value>,
    arm_bindings: &mut Vec<Value>,
) {
    let elem_val = Value::TupleGet(tuple_var, idx);

    if self.lexer.has_token("_") {
        // Wildcard — no condition, no binding.
        return;
    }

    // Nested tuple pattern: (p1, p2, ...) when elem_type is Type::Tuple.
    if self.lexer.peek_token("(")
        && matches!(elem_type, Type::Tuple(_))
    {
        if let Type::Tuple(inner_types) = elem_type.clone() {
            // Create a temp var for the inner tuple element so we can TupleGet from it.
            let inner_v = self.create_unique("match_inner", elem_type);
            self.vars.defined(inner_v);
            arm_bindings.push(Value::Set(inner_v, Box::new(elem_val)));
            self.lexer.token("(");
            for (i, inner_type) in inner_types.iter().enumerate() {
                if i > 0 { self.lexer.token(","); }
                self.parse_tuple_elem_pattern(
                    inner_v, i as u16, inner_type, arm_cond, arm_bindings,
                );
            }
            self.lexer.token(")");
            return;
        }
    }

    // Check if the next token is a plain identifier (binding variable).
    // An identifier is a binding if it is lower_case and not a keyword or literal.
    if let Some(name) = self.try_parse_binding_identifier(elem_type) {
        // Binding — no condition; add Set statement.
        let var_nr = self.vars.add_variable(&name, elem_type, &mut self.lexer);
        self.vars.defined(var_nr);
        arm_bindings.push(Value::Set(var_nr, Box::new(elem_val)));
        return;
    }

    // Scalar pattern: literal, range, or-pattern, null.
    // Store elem_val in a temp var for parse_match_pattern (which needs a var_nr).
    let tmp = self.create_unique("elem_tmp", elem_type);
    self.vars.defined(tmp);
    arm_bindings.push(Value::Set(tmp, Box::new(elem_val)));

    let (pat_cond, _) = self.parse_match_pattern(elem_type, tmp);
    *arm_cond = Some(match arm_cond.take() {
        None => pat_cond,
        Some(c) => self.cl("OpAndBool", &[c, pat_cond]),
    });
}
```

**`try_parse_binding_identifier`**: peeks at the next token. Returns `Some(name)` if it
is a lower-case identifier that is not a keyword and does not look like the start of a
literal or operator. Returns `None` if the next token is a literal, `null`, `(`, or a
keyword. This distinguishes `(n, msg) ->` (bindings) from `(0, "foo") ->` (literals).

Heuristic: if `lexer.peek()` is `LexItem::Identifier(name)` → binding. If it is
`LexItem::Integer`, `LexItem::Text`, `LexItem::Boolean` → literal (use scalar pattern).

### Step T1.9-4 — `parse_optional_guard` helper (extract or inline)

Guards are already parsed in `parse_match` with `self.lexer.has_token("if")`.  Either
inline the same pattern in `parse_tuple_match` or extract a small helper.

---

## Edge Cases

| Case | Behaviour |
|---|---|
| Subject is not a `Type::Tuple` | Existing dispatch; no change |
| Arm arity ≠ subject tuple arity | Compile error "expected N elements in pattern, found M" |
| Binding name already used in the same arm | Compile error "duplicate binding in tuple pattern" |
| Nested tuple element accessed in guard | Works — bindings are emitted before guard evaluation |
| All-`_` arm (`(_, _, ...)`) | Total; satisfies exhaustiveness |
| Guarded all-binding arm | Not total — guard may fail; still need an unguarded arm |
| `null` subject | TupleGet on a null variable — same as accessing a null tuple today (debug assert); document as UB until T1.8b adds null-safety |
| Text element bound in arm | Works; lifetime same as LHS destructuring binding |
| Tuple element that is itself a struct enum | Scalar `match` on that element is not in scope of T1.9 — use a separate outer `match` |
| Match on `match` result (chained) | Works — outer match dispatches to `parse_tuple_match` normally |

---

## Test Plan

New test file `tests/tuple_match.rs` or additions to `tests/match.rs`:

| Test | Coverage |
|---|---|
| `tuple_match_binding` | `(n, msg) ->` — all bindings, exhaustive |
| `tuple_match_literal` | `(0, "x") ->` — exact literals on both elements |
| `tuple_match_wildcard` | `(0, _)` and `_` arm — exhaustive via wildcard |
| `tuple_match_range` | `(1..10, _) ->` — range on first element |
| `tuple_match_or_elem` | `(1 \| 2 \| 3, _) ->` — or-pattern in element position |
| `tuple_match_guard` | `(n, msg) if n > 0 ->` — guard, with fallthrough arm |
| `tuple_match_nested` | `((x, y), true) ->` — nested tuple pattern |
| `tuple_match_three` | `(a, b, c) ->` — three-element tuple |
| `tuple_match_as_expr` | `val = match t { ... }` — match produces a value |
| `tuple_match_not_exhaustive` | No total arm → compile error |
| `tuple_match_guarded_not_total` | Guarded all-binding arm → still requires unguarded arm |
| `tuple_match_arity_mismatch` | `(a, b, c)` on `(integer, text)` → compile error |
| `tuple_match_null_elem` | `(null, _)` on nullable element → works |
| `tuple_match_in_function` | Tuple match as function return value |

### Reference loft script

```loft
// tests/docs/28-tuples.loft — new section

fn classify(t: (integer, text)) -> text {
    match t {
        (0, _)            -> "zero"
        (1..10, "")       -> "small-empty"
        (1..10, s)        -> "small: {s}"
        (n, msg) if n < 0 -> "negative: {n} {msg}"
        (n, msg)          -> "{n}: {msg}"
    }
}

fn main() {
    assert(classify((0, "x"))   == "zero",          "zero case");
    assert(classify((5, ""))    == "small-empty",   "small-empty");
    assert(classify((5, "hi"))  == "small: hi",     "small binding");
    assert(classify((-3, "no")) == "negative: -3 no", "guard");
    assert(classify((99, "ok")) == "99: ok",        "fallback");

    // Nested tuple match
    coords: ((float, float), boolean) = ((0.0, 0.0), true)
    result = match coords {
        ((0.0, 0.0), _) -> "origin"
        ((x, y), true)  -> "active {x},{y}"
        ((x, y), false) -> "inactive {x},{y}"
    }
    assert(result == "origin", "nested tuple: {result}");
}
```

---

## See also
- [TUPLES.md](TUPLES.md) — Full tuple design; T1.8a/b for function-return convention
- [MATCH.md](MATCH.md) — Existing match design; L2 nested field patterns
- [PLANNING.md](PLANNING.md) — T1 backlog
- `src/parser/control.rs` — `parse_match`, `parse_scalar_match`, `parse_vector_match`
- `src/data.rs` — `Type::Tuple`, `Value::TupleGet`, `Value::TuplePut`
