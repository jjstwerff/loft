
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Interfaces — Design and Implementation Plan

> **Status: implemented (I1–I8).  Stdlib interfaces (I9) shipped.**
> Known issue: P136 — for-loop + interface method on struct causes use-after-free.
> Native codegen: text-returning interface methods have a Str wrapping issue.

Structural interfaces for loft: implicit satisfaction, static dispatch only.
Primarily motivated by enabling bounded generic functions (`<T: Ordered>`).

---

## Contents

- [Motivation](#motivation)
- [Design principles](#design-principles)
- [Syntax](#syntax)
- [Semantics](#semantics)
- [Operator interfaces](#operator-interfaces)
- [Arithmetic in generic bodies](#arithmetic-in-generic-bodies)
- [What is out of scope](#what-is-out-of-scope)
- [Comparison to Go interfaces](#comparison-to-go-interfaces)
- [Standard library interfaces](#standard-library-interfaces)
- [Implementation steps](#implementation-steps)
  - [I1 — Lexer: add `interface` keyword](#i1--lexer-add-interface-keyword)
  - [I2 — Data: add `DefType::Interface` and `Definition.bound`](#i2--data-add-deftypeinterface-and-definitionbound)
  - [I3 — Parser first pass: parse interface declarations](#i3--parser-first-pass-parse-interface-declarations)
  - [I4 — Parser first pass: parse `<T: Bound>` syntax](#i4--parser-first-pass-parse-t-bound-syntax)
  - [I5 — Type resolution: validate interface bodies](#i5--type-resolution-validate-interface-bodies)
  - [I6 — Satisfaction checking at instantiation](#i6--satisfaction-checking-at-instantiation)
  - [I7 — Allow bounded method calls on T](#i7--allow-bounded-method-calls-on-t)
  - [I8 — Operator interfaces](#i8--operator-interfaces)
  - [I9 — Standard library interfaces](#i9--standard-library-interfaces)
  - [I10 — Diagnostics](#i10--diagnostics)
- [Open questions](#open-questions)

---

## Motivation

Loft's current single-`<T>` generics are opaque: no arithmetic, method calls,
field access, or comparisons are allowed on a generic `T`. This forces generic
algorithms to be either reimplemented per type or written as native Rust functions.

The most painful gap is bounded generics — functions like `max_of`, `min_of`,
and user-defined sort comparators that need `T` to be comparable. All of these
currently live in native Rust or are duplicated per concrete type in the stdlib.

A second gap is generic consumers: a function that accepts "any comparable
collection element" has no way to express that today.

Interfaces fix this by adding **compile-time constraints** on `T`. No runtime
overhead is introduced — the compiler creates a specialised copy per concrete
type (as it already does for generics), and the constraint is verified at
the call site.

---

## Design principles

1. **Implicit satisfaction (structural)** — a type satisfies an interface by
   having the required methods. No `impl Interface for Type` declaration is
   needed. This matches loft's existing dispatch model (writing
   `fn area(self: Circle)` automatically participates in the `Shape` dispatch
   wrapper without any explicit declaration).

2. **Static dispatch only** — interfaces are constraints on generic type
   parameters, not first-class values. `x: Ordered` as a variable type is
   a compile error. There are no vtables, no heap-allocated interface values.

3. **`Self` in interface bodies** — within an interface declaration, `Self`
   is a placeholder for the concrete type that will satisfy the interface.
   At instantiation, every `Self` is substituted with the actual concrete type.

4. **Multiple bounds with `+`** — `<T: A + B + C>` is supported. Bounds are
   `+`-separated after the `:`. The data model stores them as `Vec<u32>` from
   the start; satisfaction is checked for each bound independently.

5. **Methods only** — interface method signatures use `self: Self` as the
   first parameter, matching loft's existing method convention. Operator
   interfaces use the `OpCamelCase` naming the stdlib already uses internally.

---

## Syntax

### Interface declaration

```loft
interface Ordered {
    fn less_than(self: Self, other: Self) -> boolean
}

interface Printable {
    fn to_text(self: Self) -> text
}
```

`interface` is a new top-level keyword. Each method is a bare signature
(no body). `Self` is the only type variable allowed inside the interface body.

### Bounded generic function

```loft
fn max_of<T: Ordered>(v: vector<T>) -> T {
    result = v[0];
    for item in v {
        if result.less_than(item) { result = item; }
    }
    result
}
```

The bound is written as `<T: InterfaceName>`, or with multiple bounds as
`<T: A + B>`. Inside the function body, any method declared in any of the
listed interfaces may be called on values of type `T`. All other restrictions
on `T` remain (no field access, no arithmetic unless the bound includes the
relevant operator interface).

```loft
fn find_max_and_log<T: Ordered + Printable>(v: vector<T>) {
    best = max_of(v);
    log_info(best.to_text());
}
```

### Satisfying an interface

No declaration is required. Any type that has all the required methods
automatically satisfies the interface:

```loft
struct Priority { value: integer }
fn less_than(self: Priority, other: Priority) -> boolean {
    self.value < other.value
}

// Priority now satisfies Ordered — no explicit declaration.
max_of([Priority{value: 3}, Priority{value: 1}, Priority{value: 7}])
```

If a method is missing, the compiler reports an error at the call site:

```
error: Priority does not satisfy interface Ordered
  missing: fn less_than(self: Priority, other: Priority) -> boolean
```

---

## Semantics

### Satisfaction check

A concrete type `C` satisfies interface `I` if, for every method signature
`fn m(self: Self, p1: T1, ...) -> R` declared in `I`, there exists a
function `m` visible at the call site whose first parameter type is `C`,
whose remaining parameters match `T1, ...` (with `Self` replaced by `C`),
and whose return type matches `R` (with `Self` replaced by `C`).

The check is performed when a bounded generic function is instantiated —
i.e. when `max_of(v)` is first encountered with a concrete `T`. The check
happens once per concrete type per function; subsequent calls with the same
`T` skip the check.

### Dispatch inside the generic body

When the compiler specialises a bounded generic function for a concrete `T`,
method calls `x.m(...)` where `m` is declared in the bound interface are
resolved to the concrete function `m` for type `T`. This is the same
process as ordinary method resolution — no new dispatch mechanism is needed.
The specialised copy of the function body is compiled with concrete types
substituted throughout, exactly as the existing generic specialisation does.

### Visibility

Satisfaction is checked using functions that are in scope at the **call site**,
not at the interface definition site. A type defined in library A can satisfy
an interface defined in library B as long as both are visible to the caller.

---

## Operator interfaces

Loft operators dispatch via the `OpCamelCase` naming scheme. An interface
can declare operator requirements using the same names:

```loft
interface Addable {
    fn OpAdd(self: Self, other: Self) -> Self
}

interface Ordered {
    fn OpLt(self: Self, other: Self) -> boolean
    fn OpGt(self: Self, other: Self) -> boolean
}
```

Inside a generic body bounded by `Addable`, `x + y` is allowed and resolves
to the `OpAdd` implementation for the concrete type. This requires a small
change to the existing generic body type-checking: when an operator is applied
to `T`, check if the operator name is declared in the bound interface before
emitting the "operator requires concrete type" error.

A built-in operator interface is **satisfied automatically** if the concrete
type already has the relevant operator defined — no `fn OpAdd(self: Priority)`
stub is required if `+` already works on `Priority`.

**Note:** `fn less_than` (method form) and `fn OpLt` (operator form) are
two separate ways to express the same capability. Prefer the method form for
readability in user-defined interfaces; the operator form for stdlib interfaces
that hook into loft's operator dispatch.

---

## Arithmetic in generic bodies

This section details the four cases that arise when `T` participates in
arithmetic, and how each is handled by the interface design.

### Case 1 — Same-type binary operators: `T op T -> T` or `T op T -> boolean`

The common case: `total + item`, `a < b`, `x == y`. Both operands are `T`;
the result is either `T` or a concrete type (`boolean`).

```loft
interface Addable {
    fn OpAdd(self: Self, other: Self) -> Self
}
fn sum_of<T: Addable>(v: vector<T>) -> T {
    result = v[0];
    for item in v[1..] { result = result + item; }
    result
}
```

Inside the generic body, the type of `result + item` is determined by the
interface method's return type (`Self` → `T`). Step I8 reads the declared
return type from the interface method signature and uses it as the expression
type in the IR, rather than emitting the "operator requires concrete type"
error.

### Case 2 — Mixed-type binary operators: `T op concrete -> T`

Sometimes the second operand is a fixed concrete type, not another `T`:
`distance * 2.0`, `count + 1`. This is expressible by declaring the concrete
type explicitly in the interface method signature:

```loft
interface Scalable {
    fn OpMul(self: Self, factor: float) -> Self
}
fn scale_all<T: Scalable>(v: vector<T>, factor: float) -> vector<T> {
    [item * factor for item in v]
}
```

The satisfaction check (I6) matches the second parameter as a concrete type,
not `Self`. At the operator dispatch site (I8), when `x * factor` is
encountered with `x: T` and `factor: float`, the interface's `OpMul(Self, float)`
signature matches and the call is allowed.

Concrete types on the **left** side (`concrete op T -> T`) are not supported
in phase 1 — operator dispatch always starts from the `self` position.

### Case 3 — Operators with non-Self return type: `T op T -> concrete`

Some operators produce a widened or different type: average computation needs
`T / T -> float`, a hash function needs `T -> integer`. These are declared
with a concrete return type in the interface:

```loft
interface Hashable {
    fn hash(self: Self) -> integer
}
interface Averageable {
    fn OpAdd(self: Self, other: Self) -> Self
    fn OpDiv(self: Self, divisor: integer) -> float
}
fn average<T: Averageable>(v: vector<T>) -> float {
    total = v[0];
    for item in v[1..] { total = total + item; }
    total / len(v)
}
```

In the generic body, `total / len(v)` has type `float` (the declared return
type of `OpDiv`), not `T`. Step I8 must propagate the return type from the
interface signature to the IR expression type. `Self` in the return position
is replaced with `T`; any concrete type is used as-is.

### Case 4 — Zero / identity element

A recurring problem in generic arithmetic is initialisation: `total = 0`
does not type-check when `total: T`. Loft's null-propagation model provides
a natural solution: **null is the universal zero sentinel for arithmetic**.

```loft
fn sum_of<T: Addable>(v: vector<T>) -> T {
    result = v[0];              // null if vector is empty
    for item in v[1..] { result = result + item; }
    result                      // null for empty vector
}
```

If the vector is empty, `v[0]` returns the null sentinel for `T`
(`i32::MIN` for integer, `NaN` for float, etc.), and the loop body never
executes. The caller receives null, which is consistent with loft's standard
pattern for "no value" results.

For cases where null-as-zero is not acceptable, declare a `zero()` factory
method in the interface (see open question Q4 and Q6 below).

### Compound assignment

`total += item` desugars to `total = total + item` in loft's parser, so it
is handled by `OpAdd` automatically. No separate treatment needed.

### Unary operators

Declared with `self` only, no second parameter:

```loft
interface Negatable {
    fn OpNeg(self: Self) -> Self
}
fn negate_all<T: Negatable>(v: vector<T>) -> vector<T> {
    [-item for item in v]
}
```

I8 handles unary operators identically to binary ones: map the operator token
to its `OpCamelCase` name (`-` unary → `"OpNeg"`), check the bound, allow if
declared.

---

## What is out of scope

- **Dynamic dispatch / interface values** — `x: Ordered = my_priority` is not
  supported. Interfaces are constraint annotations, not types.
- **Composite interfaces / interface inheritance** — `interface A extends B` is
  not supported; declare the methods directly in the interface that needs them.
- **Associated types** — no `type Item` inside an interface.
- **Default method implementations** — no bodies inside interface declarations.
- **Interface inheritance** — no `interface A extends B`.
- **Implementing an interface for a type you didn't define** — satisfaction is
  structural; if the method exists with the right signature, it counts. There is
  no orphan rule.

All of the above can be added later. The implementation steps below are
designed to avoid closing off these extensions prematurely.

---

## Comparison to Go interfaces

| Property | Go interfaces | loft interfaces (this design) |
|---|---|---|
| Satisfaction | Implicit / structural | Implicit / structural (same) |
| Dynamic dispatch | Yes — interface values carry a vtable | No — bounds only, no vtables |
| Interface as a type | `var x io.Reader = ...` | Not allowed |
| Generic bounds | `[T interface{ M() }]` (Go 1.18+) | `<T: Interface>` (same concept) |
| Operator requirements | Not natively expressible | Via `OpCamelCase` method names |
| Multiple bounds | `[T A ∩ B]` | `<T: A + B>` — supported |
| Default methods | No | No |

The dispatch model (no vtables, static specialisation) aligns with Go 1.18+
generic constraints rather than classic Go interface values.

---

## Standard library interfaces

Phase 1 defines these interfaces in `default/01_code.loft`:

```loft
// Comparison — for sorting and min/max
interface Ordered {
    fn OpLt(self: Self, other: Self) -> boolean
    fn OpGt(self: Self, other: Self) -> boolean
}

// Equality — for set membership and index lookup
interface Equatable {
    fn OpEq(self: Self, other: Self) -> boolean
    fn OpNe(self: Self, other: Self) -> boolean
}

// Addition and subtraction — for sum_of and generic accumulators
interface Addable {
    fn OpAdd(self: Self, other: Self) -> Self
    fn OpMin(self: Self, other: Self) -> Self
}

// Full scalar arithmetic — all four operators, same-type
interface Numeric {
    fn OpAdd(self: Self, other: Self) -> Self
    fn OpMin(self: Self, other: Self) -> Self
    fn OpMul(self: Self, other: Self) -> Self
    fn OpDiv(self: Self, other: Self) -> Self
    fn OpNeg(self: Self) -> Self
    fn OpLt(self: Self, other: Self) -> boolean
    fn OpGt(self: Self, other: Self) -> boolean
}

// Scaling by a float factor — for normalisation and interpolation
interface Scalable {
    fn OpMul(self: Self, factor: float) -> Self
}

// Text conversion — for generic print/log helpers
interface Printable {
    fn to_text(self: Self) -> text
}
```

Built-in types (`integer`, `long`, `float`) automatically satisfy `Ordered`,
`Equatable`, `Addable`, and `Numeric` via their existing operator definitions.
`text` satisfies `Ordered` and `Equatable`. No extra declarations are needed.

**Stdlib functions converted from native to bounded-generic loft** (depends on I8):

| Function | Bound | Notes |
|---|---|---|
| `sum_of<T: Addable>` | `Addable` | first-element init; null for empty vector |
| `min_of<T: Ordered>` | `Ordered` | first-element init; null for empty vector |
| `max_of<T: Ordered>` | `Ordered` | first-element init; null for empty vector |

---

## Implementation steps

Each step is independently compilable and testable. Steps I1–I6 are the core;
I7–I10 add usability and standard library support.

---

### I1 — Lexer: add `interface` keyword

**File:** `src/lexer.rs`

Add `"interface"` to the `KEYWORDS` static slice. After this step,
`interface` is tokenised as `Token("interface")` instead of
`Identifier("interface")`, making it available as a reserved keyword for
the parser.

**Test:** parsing a file that uses `interface` as an identifier should produce
a keyword-conflict error (same as `struct`, `fn`, etc.).

---

### I2 — Data: add `DefType::Interface` and `Definition.bound`

**Files:** `src/data.rs`

**2a.** Add a new variant to `DefType`:

```rust
pub enum DefType {
    // ... existing variants ...
    /// An interface declaration: a named set of required method signatures.
    /// Child definitions (via parent links) are the required method stubs.
    Interface,
}
```

**2b.** Add a `bound` field to `Definition`:

```rust
pub struct Definition {
    // ... existing fields ...
    /// For Generic functions: the def_nrs of all required interfaces (empty = no bounds).
    pub bounds: Vec<u32>,
}
```

Initialise `bounds` to `vec![]` in `Definition`'s constructor. Using a `Vec`
from the start means multiple bounds (`<T: A + B>`) requires no data model
change later — only the parser needs extending.

**Conflict detection:** if two bounds in `bounds` declare a method with the
same name but different signatures, emit an error at the `fn` declaration site:
`"interfaces A and B both declare method foo with conflicting signatures"`.
This is checked once when the bounds are resolved in the second pass.

**Test:** `Definition` constructs with `bounds = vec![]` without affecting
existing behaviour. A generic function with two bounds stores two entries.

---

### I3 — Parser first pass: parse interface declarations

**File:** `src/parser/definitions.rs`

Add a `parse_interface(&mut self) -> bool` method, called from
`parse_file`'s top-level loop alongside `parse_struct`, `parse_enum`, etc.

```
interface Ident { fn_signature* }

fn_signature = "fn" Ident "(" param_list ")" [ "->" type ] ";"
               // no body — ends with ";" or "}"
```

First pass actions:
1. Consume `interface`.
2. Read the interface name (must be `CamelCase`; emit error otherwise).
3. Call `data.add_def(name, pos, DefType::Interface)` to register it.
4. Parse each method signature. For each:
   - Call `data.add_def(method_name, pos, DefType::Function)` with
     `parent = interface_def_nr`.
   - Store parameter types and return type in the `Definition.attributes`
     or `Definition.returned` fields (same layout as a regular function stub).
   - `Self` in parameter/return types is stored as `Type::Unknown(interface_def_nr)`
     as a placeholder; it is resolved to the concrete type at instantiation.
5. Skip the body (no second-pass IR generation for interfaces).

**Test:** a file with a valid interface declaration parses without error.
An interface with a duplicate name emits the existing "already defined" diagnostic.

---

### I4 — Parser first pass: parse `<T: Bound>` syntax

**File:** `src/parser/definitions.rs`, inside `parse_function`.

The existing generic parsing detects `<T>` at the function name:

```rust
// Current code (simplified):
if lexer.has_token("<") {
    type_var_name = lexer.identifier();
    lexer.token(">");
    is_generic = true;
}
```

Extend this to optionally read `: A + B + ...` after the type variable:

```rust
if lexer.has_token("<") {
    type_var_name = lexer.identifier();
    let mut bound_names: Vec<String> = vec![];
    if lexer.has_token(":") {
        bound_names.push(lexer.identifier());       // first bound
        while lexer.has_token("+") {
            bound_names.push(lexer.identifier());   // additional bounds
        }
    }
    lexer.token(">");
    is_generic = true;
    // ...
    if !bound_names.is_empty() {
        self.pending_bounds = bound_names;   // resolved in second pass
    }
}
```

In the second pass, resolve each name in `pending_bounds` via
`data.def_nr(&name)` and push the result into `definition.bounds`. If any
name does not resolve to a `DefType::Interface`, emit "unknown interface".
After all bounds are resolved, run conflict detection (see I2).

**Test:** `fn foo<T: Ordered>(...) { ... }` stores one bound.
`fn foo<T: Ordered + Printable>(...) { ... }` stores two bounds.
`fn foo<T>(...) { ... }` stores zero bounds. Unknown interface name errors.

---

### I5 — Type resolution: validate interface bodies

**File:** `src/typedef.rs`, inside `actual_types` or a new `check_interfaces`.

After type resolution, iterate over all `DefType::Interface` definitions.
For each required method (child definitions with the interface as parent):

- Resolve all `Type::Unknown(interface_def_nr)` (the `Self` placeholder) to
  a sentinel that the satisfaction checker in I6 will substitute.
- Validate that all other types in the signature are known and concrete.
- Emit errors for unresolved types in interface bodies.

No bytecode is generated for interface definitions themselves.

**Test:** an interface with an unknown type in a method signature emits a
clear "unknown type" error. An interface with all valid types passes silently.

---

### I6 — Satisfaction checking at instantiation

**File:** `src/parser/definitions.rs`, inside the generic specialisation logic.

Currently, when a generic function `fn foo<T>(...)` is called with a concrete
`T = Point`, the compiler looks for or creates a specialised copy named
`foo_Point`. Extend this to also run a satisfaction check:

```rust
fn check_satisfaction(
    data: &Data,
    concrete_type: u32,   // def_nr of the concrete struct/enum
    bound: u32,           // def_nr of one required interface
    call_pos: &Position,
    diagnostics: &mut Diagnostics,
) {
    // Collect required method signatures from the interface's children.
    for child in data.children_of(bound) {
        let concrete_fn = data.find_method(child.name, concrete_type);
        if concrete_fn == u32::MAX {
            diagnostics.error(
                call_pos,
                &format!(
                    "{} does not satisfy interface {}: missing fn {}",
                    data.def(concrete_type).name,
                    data.def(bound).name,
                    child.name,
                )
            );
        } else {
            // Check return type and param types match (with Self → concrete_type).
        }
    }
}

// Call once per bound, per (concrete_type, generic_fn) pair:
for &bound in &definition.bounds {
    check_satisfaction(data, concrete_type, bound, call_pos, diagnostics);
}
```

Cache results per `(concrete_type, generic_fn)` pair to avoid re-checking on
every call. The cache key covers all bounds together; if any bound fails the
whole instantiation fails.

**Test:** calling `max_of([Priority{...}])` where `Priority` has `less_than`
compiles cleanly. Calling `max_of([Thing{...}])` where `Thing` lacks
`less_than` emits the "does not satisfy" error.

---

### I7 — Allow bounded method calls on T

**File:** `src/parser/control.rs` or `src/parser/objects.rs` — wherever
method calls on generic `T` currently emit the
`"generic type T: method call requires a concrete type"` error.

When `x.method(args)` is encountered and `x` is of generic type `T`:

1. Collect `definition.bounds` for the enclosing generic function.
2. Search each bound's children for a method named `method`. Stop at the
   first match.
3. If found in any bound: allow the call. The method resolves to the concrete
   implementation when the specialised copy is compiled.
4. If not found in any bound: emit the existing "method call requires a
   concrete type" error, listing all bounds that were searched.

**Test:** inside `fn find_max_and_log<T: Ordered + Printable>`, both
`result < item` and `item.to_text()` compile. A method not in either bound
still errors.

---

### I8 — Operator interfaces

**File:** `src/parser/operators.rs` — wherever operators on generic `T`
currently emit the `"operator '+' requires a concrete type"` error.

When an operator expression is encountered with a `T`-typed operand, the
procedure is:

1. Map the operator token to its `OpCamelCase` name
   (e.g. `+` → `"OpAdd"`, unary `-` → `"OpNeg"`).
2. Search **each bound** in `definition.bounds` for a child method named
   `"OpAdd"` (or the relevant name). Stop at the first match.
3. If not found in any bound: emit the existing "operator requires concrete type" error.
4. If found: validate the operand types against the interface signature:
   - For binary operators: the right-hand operand must match the declared
     second-parameter type (either `Self` → `T`, or a concrete type like
     `float`). Emit a type-mismatch error if they differ.
   - For unary operators: no second operand to check.
5. Determine the result type from the interface method's declared return type:
   - `Self` in return position → `T` (the generic type variable).
   - Any concrete type (e.g. `boolean`, `float`) → that concrete type.
   Emit the operator as a `Call(op_nr, args)` IR node with this result type.

This result-type propagation is the key addition over the boolean allow/deny
check: the generic body's type checker needs to know whether `x / y` produces
`T` or `float` in order to type-check subsequent expressions.

This requires the operator-name mapping (operator token → `OpCamelCase`)
which already exists in `src/parser/operators.rs`. It only needs to be
made accessible at the check site.

**Covered cases:**
- `T + T -> T` — same-type binary, Self return
- `T < T -> boolean` — same-type binary, concrete return
- `T * float -> T` — mixed-type binary, Self return
- `T / T -> float` — same-type binary, concrete return
- `-T -> T` — unary, Self return
- `T += T` — desugars to `T = T + T` before this stage; handled by OpAdd

**Test:** inside `fn sum_of<T: Addable>`, `total = total + item` compiles
and the result has type `T`. Inside `fn average<T: Averageable>`,
`total / len(v)` compiles and the result has type `float`. Inside
`fn id<T>` (no bound), `total = total + item` still errors.

---

### I9 — Standard library interfaces

**File:** `default/01_code.loft`

Add interface declarations at the top of the file, before the operator
definitions they describe:

```loft
pub interface Ordered {
    fn OpLt(self: Self, other: Self) -> boolean
    fn OpGt(self: Self, other: Self) -> boolean
}

pub interface Equatable {
    fn OpEq(self: Self, other: Self) -> boolean
    fn OpNe(self: Self, other: Self) -> boolean
}

pub interface Addable {
    fn OpAdd(self: Self, other: Self) -> Self
}

pub interface Printable {
    fn to_text(self: Self) -> text
}
```

Convert the currently-native `sum_of`, `min_of`, `max_of`, `any_of`, `all_of`
from native Rust implementations to bounded generic loft functions where
feasible. Those that require operator access (`sum_of`, `min_of`, `max_of`)
depend on I8 landing first.

**Test:** existing tests for these stdlib functions pass unchanged.
A new test shows a user-defined type satisfying `Ordered` and being passed
to `max_of`.

---

### I10 — Diagnostics

**Files:** `src/diagnostics.rs`, satisfaction check in I6.

Polish the error messages from the satisfaction check:

```
error[I01]: type `Priority` does not satisfy interface `Ordered`
  --> example.loft:14:5
   |
14 |     max_of(priorities)
   |     ^^^^^^ `Ordered` required by this bound on `T`
   |
   = missing: fn OpLt(self: Priority, other: Priority) -> boolean
   = missing: fn OpGt(self: Priority, other: Priority) -> boolean
   = help: add `fn OpGt(self: Priority, other: Priority) -> boolean { ... }`
```

Also add a diagnostic for using an interface name as a type
(`x: Ordered = ...`) with a clear "interfaces cannot be used as types" message.

**Test:** a deliberately unsatisfied call produces the formatted multi-line
error. Using an interface as a variable type produces the specific message.

---

## Open questions

**Q1: Multiple bounds** — resolved. `<T: A + B>` is supported from the start.
`Definition.bounds` is `Vec<u32>` from I2 onward; the parser (I4) reads
`+`-separated names in a loop; satisfaction (I6) and lookup (I7, I8) iterate
over all bounds. The incremental cost over a single-bound design is ~40 lines.

**Q2: Operator method naming in interfaces** — requiring users to write
`fn OpLt(self: Self, other: Self) -> boolean` is consistent with internals
but surprising to users expecting `<` syntax. Consider allowing
`op < (self: Self, other: Self) -> boolean` as syntactic sugar in interface
bodies that desugars to `fn OpLt`. This is a purely cosmetic change that
can be added without altering the data model.

*Mitigation:* Add `op <op> (self: Self, ...) -> T` sugar in `parse_interface`
(`src/parser/definitions.rs`) that maps the operator token to its `OpCamelCase`
name and stores it as an ordinary method stub. Zero data model impact; the
desugaring happens before any downstream step sees the signature.

**Q3: Interface visibility / `pub`** — should interfaces follow the same
`pub` / non-`pub` visibility rules as functions? Recommended: yes, using the
existing `pub_visible` field on `Definition`.

*Mitigation:* Reuse `pub_visible` on `Definition` unchanged. `parse_interface`
checks for a leading `pub` token and sets the flag exactly as `parse_function`
does. No new field or mechanism required.

**Q4: `Self` in return position** — `fn create(x: integer) -> Self` (a
factory method with no `self` parameter) is probably not useful at this stage
and complicates the `Self` substitution. Restrict `Self` to appear only when
`self: Self` is the first parameter in phase 1.

*Mitigation (phase 1):* In the I5 validation pass, emit
`"factory methods (Self in return without self parameter) are not yet supported"`
if `Self` appears in the return type but no `self: Self` first parameter is
present. This makes the restriction explicit rather than silently producing
wrong code. The caller-supplied-identity overload
(`fn sum_of<T: Addable>(v: vector<T>, identity: T) -> T`) is the recommended
workaround for the empty-collection case (see Q6).

*Mitigation (phase 2):* Track a separate `Self` substitution for parameterless
factory methods keyed by the call-site's concrete type. Requires no data-model
change; only extends the substitution logic in I6.

**Q5: Interfaces in the doc generator** — `gendoc` (`src/documentation.rs`)
will need a rendering path for `DefType::Interface`. Deferring to after the
feature lands; add a stub that omits interfaces from HTML output until then.

*Mitigation:* Add a guard in the `documentation.rs` rendering loop that
silently skips `DefType::Interface` definitions (the same pattern used for
any unhandled variant). This prevents a panic on the first `cargo run --bin
gendoc` run after I2 lands. A proper interface section (name, signatures,
known implementing types) can be added as a follow-up without touching any
other step.

**Q6: Zero/identity element for generic arithmetic** — the first-element
initialisation pattern (`result = v[0]; for item in v[1..]`) is loft-idiomatic
and returns null for empty collections, which is consistent with null
propagation elsewhere. However, some algorithms need an explicit zero:
an empty-safe `sum_of` that returns 0 (not null) for an empty vector.
Two paths exist:

- **Relax Q4** and allow factory methods without `self`: `fn zero() -> Self`.
  Then `Addable` gains `fn zero() -> Self`, and `sum_of` calls `T.zero()`
  for its initial value. Requires extending `Self` substitution to cover
  parameterless functions.
- **Caller-supplied identity**: add an overload
  `fn sum_of<T: Addable>(v: vector<T>, identity: T) -> T`
  where the caller passes the zero value. No language change needed.

*Mitigation:* Ship the caller-supplied-identity overload in phase 1 alongside
I9. Add it next to the first-element form in `default/01_code.loft`. This
covers the empty-safe use case with no language change. Revisit the factory
method form (`fn zero() -> Self`) in phase 2 after Q4 is relaxed.

---

## Phase 1 gaps

### Left-side concrete operand (`concrete op T -> T`)

`2.0 * my_t_value` is not supported in phase 1. Operator dispatch always
starts from the `self` position, so the left operand must be of type `T`.

*Mitigation (phase 1):* Document as a known limitation. Most cases can be
rewritten using commutativity: `my_t_value * 2.0`. Where commutativity does
not hold, the user defines a helper method instead of relying on operator
syntax.

*Mitigation (phase 2):* After the primary `T.OpMul(concrete)` lookup
succeeds, allow declaring `fn OpMul(factor: float, self: Self) -> Self` in the
interface with `factor` as the first parameter — but this requires either
commutativity to be declared explicitly in the interface, or a second-pass
fallback lookup. Add a design note before implementing to avoid ambiguity with
existing overload resolution.
