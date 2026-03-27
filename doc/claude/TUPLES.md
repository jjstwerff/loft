// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Tuple Design

> **Status: completed in 0.8.3.** T1.1–T1.7 implemented; tuple-returning functions (T1.4) and LHS destructuring remain deferred.

Tuples are anonymous, fixed-arity, stack-allocated compound value types. They exist
as a lightweight mechanism for returning multiple values from a function and for
temporarily storing or passing those values, without the heap overhead and declaration
ceremony of a named struct.

---

## Contents

- [Goals](#goals)
- [Syntax](#syntax)
- [Semantics](#semantics)
- [Memory Layout](#memory-layout)
- [Null Behaviour](#null-behaviour)
- [Copy Semantics](#copy-semantics)
- [Calling Convention for Owned Elements](#calling-convention-for-owned-elements)
- [Grammar Changes](#grammar-changes)
- [IR Representation](#ir-representation)
- [Implementation Phases](#implementation-phases)
  - [Core phases 1–4](#core-phases-14)
  - [Mitigation phases 5–7](#mitigation-phases-57)
- [Known Limitations](#known-limitations)
- [Non-Goals](#non-goals)

---

## Goals

- Allow functions to return multiple values without defining a dedicated struct.
- Allow callers to either destructure the return immediately or store the result in a
  variable for later element access or copying.
- Keep tuples as pure stack-value types — no heap allocation for the tuple container.
- Fit naturally into the existing type system, parser, and codegen with minimal
  special-casing.

---

## Syntax

### Type notation

A tuple type is a parenthesised, comma-separated list of **two or more** element types:

```loft
(integer, text)
(float, float, boolean)
(integer, vector<Item>)
(integer, (float, boolean))     // nested tuple
```

`(T)` with a single type is a parenthesised type expression, not a tuple.
Single-element tuples are not supported.

### Function return type

```loft
fn min_max(v: vector<integer>) -> (integer, integer) {
    (min_val, max_val)
}
```

### Tuple literal

A tuple literal is a parenthesised, comma-separated list of **two or more** expressions.
A trailing comma is allowed:

```loft
t = (1, "hello")
t = (x + 1, name, 3.14,)
```

### Explicit type annotation

Variables may be annotated with a tuple type in the same way as any other type:

```loft
a: (integer, text) = foo()
b: (float, float, boolean)      // declared without value; must be assigned before use
```

### Element access

Tuple elements are accessed with dot notation followed by a **non-negative decimal
integer literal** (zero-based):

```loft
a = foo()           // a: (integer, text)
b = a.0             // b: integer  — first element
c = a.1             // c: text     — second element
```

The index must be a decimal literal known at compile time. Hex/octal/binary literals
and out-of-range indices are compile errors.

For nested tuples accesses chain:

```loft
t = (1, (3.14, true))
x = t.1.0           // 3.14 — first element of the inner tuple
```

Individual tuple elements may be used inside format strings:

```loft
println("{t.0}: {t.1}")     // valid — elements are normal typed expressions
println("{t}")              // ERROR — whole-tuple formatting not supported (see Non-Goals)
```

### Element assignment

Individual elements of a tuple **variable** may be assigned or compound-assigned:

```loft
a.0 = 5
a.0 += 3
a.1 = "new value"
```

Assignment to a temporary is a compile error:

```loft
foo().0 = 5         // ERROR: cannot assign to a temporary tuple element
```

### LHS deconstruction

When the right-hand side evaluates to a tuple, the left-hand side may be a
parenthesised list of variable names:

```loft
(lo, hi) = min_max(values)
(x, y, z) = compute_coords()
```

Rules:
- All names are plain `lower_case` identifiers with no type annotation; types are
  inferred from the tuple.
- Arity must match exactly; `+=` and other compound operators are not supported.
- Repeating the same name in one deconstruction is a compile error.
- If a name already exists in scope with the matching type it is re-assigned;
  otherwise a new variable is declared.

### Function parameters

A tuple type may be used as a parameter type (pass by value):

```loft
fn print_pair(t: (integer, text)) {
    println("{t.0}: {t.1}")
}

print_pair((42, "hello"))   // tuple literal
print_pair(my_pair)         // variable
```

To mutate the caller's tuple variable, use `&`:

```loft
fn swap(pair: &(integer, integer)) {
    tmp = pair.0
    pair.0 = pair.1
    pair.1 = tmp
}

swap(&coords)
```

`&(T1, T2)` supports all element types including `text`, `reference`, and `vector`
via the per-element DbRef protocol (see [Phase 5](#phase-5--sc-4-reference-tuple-parameters-with-owned-elements)).
Nested tuples inside a `&tuple` parameter are recursively flattened.

The standard `&` enforcement applies: a parameter that is never mutated is a
compile error.

### Tuples are not iterable

Tuples may not be used as the target of a `for` loop. `for x in t` where `t` is a
tuple type is always a compile error. Iterate over individual collection elements
(`for x in t.0`) instead.

---

## Semantics

### Allowed element types

Any type that a local variable may hold is a valid tuple element:

| Element type | Notes |
|---|---|
| `boolean`, `integer`, `long`, `float`, `single`, `character` | |
| `integer not null` | suppresses null-check elision; see [Phase 7](#phase-7--sc-7-not-null-annotation-for-tuple-integer-elements) |
| `text` | deep-copied on assignment; see [Copy Semantics](#copy-semantics) |
| plain `enum` | |
| struct-`enum` / polymorphic enum reference | 12-byte DbRef |
| `reference<T>` | DbRef copied; same heap object shared |
| `vector<T>`, `hash<T[...]>`, `sorted<T[...]>`, `index<T[...]>` | container DbRef copied |
| `fn(T) -> R` | 4-byte d_nr |
| `iterator<T, I>` | iterator state |
| `(T1, T2, ...)` — nested tuple | laid out inline; all rules apply recursively |

### Restriction: no tuples in struct fields

```loft
struct Pair {
    data: (integer, text)   // ERROR: tuple types are not allowed in struct fields
}
```

Structs are stored as packed heap records. Tuple fields would require per-element
owned-type tracking inside the record layout. Use a named struct with explicit
fields instead.

### Restriction: no `&T` inside a tuple

```loft
t: (&integer, text)             // ERROR: &T is not allowed as a tuple element
fn foo() -> (&text, integer)    // ERROR
```

A `&T` element would be a stack pointer into the caller's frame. Since tuples are
primarily used as return values, permitting reference elements would produce dangling
pointers. This restriction applies everywhere, not only in return types.

`&(T1, T2)` as a parameter is **not** a reference inside the tuple; it is a
reference to a tuple variable and is valid.

---

## Memory Layout

A tuple lives entirely on the stack. Each element occupies its natural stack-slot
size, laid out contiguously in declaration order.

### Element sizes

| Type | Stack size |
|---|---|
| `boolean` | 1 byte |
| `integer` (unconstrained) | 4 bytes |
| `integer limit(0, 255)` / `u8` | 1 byte |
| `integer limit(0, 65535)` / `u16` | 2 bytes |
| `long` | 8 bytes |
| `float` | 8 bytes |
| `single` | 4 bytes |
| `character` | 4 bytes |
| plain `enum` | 1 byte |
| `text` | 24 bytes (Rust `String`) |
| `reference<T>`, `vector<T>`, collection, struct-`enum` | 12 bytes (DbRef) |
| `fn(T) -> R` | 4 bytes (d_nr as `i32`) |

### Total size and offsets

```
sizeof((T0, T1, T2)) = size(T0) + size(T1) + size(T2)
offset(N)            = sum of sizes of elements 0 .. N-1
```

Both total size and per-element offsets are compile-time constants. The existing
`sizeof` operator extends naturally: `sizeof((integer, text))` returns 28.

### Variable layout example

```
a: (integer, text, float)      // 4 + 24 + 8 = 36 bytes total
   a.0 at stack_pos + 0        // 4 bytes
   a.1 at stack_pos + 4        // 24 bytes
   a.2 at stack_pos + 28       // 8 bytes
```

---

## Null Behaviour

There is no null tuple. A variable of tuple type always holds all of its elements.
The elements follow their own null rules:

- `text` element: can be null (internal null pointer sentinel).
- `integer` element: can be null (sentinel `i32::MIN`), unless annotated `not null`.
- `reference<T>` element: can be null (record 0).

Accessing a tuple variable before it is assigned triggers the same uninitialized-
variable error as any other local variable.

---

## Copy Semantics

`b = a` where `a: (T1, T2, ...)` copies each element according to its type:

- **Primitives** (`integer`, `long`, `float`, `single`, `boolean`, `character`,
  plain `enum`, `fn`): copied by value; the two variables are independent.
- **`text` elements**: **deep-copied** — a fresh `String` buffer is allocated and
  the content is duplicated. This is the only safe policy without move semantics
  and matches loft's text-assignment behaviour everywhere else.
- **`reference<T>`, `vector<T>`, collection elements**: the 12-byte DbRef is
  copied; both variables point to the same heap object.

### Re-assignment frees previous owned elements

When an already-initialised tuple variable is re-assigned — whether by a new tuple
literal, a function return, or `b = a` — its existing `text` elements are freed
(`OpFreeText`) before the new values are written, exactly as for plain text variable
re-assignment. DbRef elements (reference, vector) are freed via `OpFreeRef` first
if they own their allocation.

```loft
a = make_pair()     // a.1 owns a String buffer
a = make_pair()     // old a.1 buffer freed, new one written
```

```loft
a = pair()
b = a               // b.1 is a deep copy — independent buffer
b.1 += " world"
assert(a.1 == "hello")  // a.1 unaffected

b.0 += [4]          // if element 0 is vector<X>: b.0 and a.0 share the DbRef;
                    // both see the appended element
```

---

## Calling Convention for Owned Elements

Tuple returns always use **caller-allocated slots**, whether or not any element is
an owned type. This is consistent with how plain `text` returns already work in
loft and avoids a special case in codegen.

1. **Caller** allocates the full tuple slot (including `String` and DbRef fields)
   on its own stack before the call.
2. **Caller** passes a `DbRef` pointing to the pre-allocated region via
   `OpCreateStack`.
3. **Callee** writes each element at its offset using existing stack-ops
   (`OpAppendStackText` for text, ref set ops for DbRef elements, scalar writes for
   primitives).
4. On return, all elements already live in the caller's frame — no transfer needed.

**Why naive "push left to right" fails for text:** if the callee pushed a `String`
onto its own stack, `OpFreeStack` would free the backing buffer before the caller
could use it — a use-after-free. Caller-allocated slots avoid this entirely.

### Scope exit order for owned elements

When a tuple variable goes out of scope, elements must be freed in **reverse index
order** (N-1 down to 0). This maintains the LIFO invariant of `Stores::free()`:
if element 0 is `reference<A>` (store K) and element 1 is `reference<B>` (store
K+1), freeing K before K+1 would decrement `max` while K+1 is still live —
corrupting the allocator. Reverse order is always safe.

For nested tuples the requirement applies recursively: the innermost-last element
is freed before outer elements.

---

## Grammar Changes

All changes are additive.

### Type grammar (extended)

```
type ::= ...
       | '(' type ',' type { ',' type } [ ',' ] ')'   // tuple type (≥ 2 elements)
       | integer_type 'not' 'null'                     // non-nullable integer (Phase 7)
```

### Expression grammar (extended)

```
single ::= ...
         | '(' expr ',' expr { ',' expr } [ ',' ] ')'  // tuple literal (≥ 2 elements)

operators ::= single { '.' ( ident [ '(' args ')' ] | Integer )
                     | '[' index ']'
                     | '#' ident }
            { op operators }
```

The `'.' Integer` arm handles `a.0`, `a.1`, etc. The integer token must be a
non-negative decimal literal; hex/octal/binary and out-of-range indices are compile
errors.

### Assignment grammar (extended)

```
assignment ::= '(' ident { ',' ident } ')' '=' operators    // LHS destructuring
             | operators [ ( '=' | '+=' | '-=' | '*=' | '/=' | '%=' ) operators ]
```

The destructuring form is only valid at the statement level and only with `=`.

---

## IR Representation

### New `Type` variant

```rust
Tuple(Vec<Type>),
// Invariants (enforced at type-check time):
//   - len >= 2
//   - no element is Type::RefVar(_)
//   - Type::Tuple may not appear in any struct field position
```

Supporting changes in `data.rs`:
- `size_of`: sum of element sizes, recursing into nested `Tuple`.
- `depend()`: union of `depend()` of all elements, recursing into nested `Tuple`.
- `depending(on)`: prepend `on` to every element's dep list, recursing.
- `Display`: render as `(T0, T1, ...)`.

### `Type::Integer` — `not null` flag (Phase 7)

```rust
Integer(i32, u32, bool),   // (min, max, not_null)
```

The flag is `false` for all existing `Integer` uses; no bytecode or runtime changes.
Its sole effect is in the type checker: null-check expressions on a `not null`
element emit a compile warning and the null branch is elided from codegen.

### New `Value` variant

```rust
Tuple(Vec<Value>),
// Constructs a tuple using the caller-allocated-slot convention.
// The callee writes each element into the caller's pre-allocated region.
```

### Element read

Tuple element `N` of variable `v` is read by applying `generate_var` to the element
type at `stack_pos + offset(N)`. Existing opcodes (`OpVarInt`, `OpVarText`,
`OpVarRef`, etc.) are reused; the encoded distance accounts for the intra-tuple
byte offset.

### Element write

Symmetric to element read: `OpPutInt`, `OpPutText`, `OpSetRef`, etc. at
`stack_pos + offset(N)`. For text assignment, `OpFreeText` is emitted on the
existing content before the new string is constructed.

### Block result type for tuple-returning functions

A function returning `(T1, T2)` sets its root `Block.result = Type::Tuple([T1, T2])`.
Codegen reads this to allocate the caller-side slot and emit the `OpCreateStack`
call before the `call` opcode, mirroring the existing text-return path.

### LHS deconstruction lowering

`(a, b) = foo()` desugars to:

```
__tmp = foo()       // caller-allocated; owned elements land directly in __tmp
a = __tmp.0         // text: deep-copy; primitives: scalar copy; DbRef: handle copy
b = __tmp.1
// __tmp freed (in reverse element order); a and b have independent text buffers
```

### `&(T1, T2)` parameter — per-element DbRef protocol (Phase 5)

Implemented as N consecutive DbRefs in the callee's parameter frame (12 bytes each,
total N×12 bytes). The caller emits one `OpCreateStack` per element, each targeting
the element's individual stack position: `tuple_var.stack_pos + offset(K)`.

Inside the callee, element K is accessed via its DbRef using the standard
`generate_var` pattern for `RefVar(TK)` — no new opcodes required:

| Element type | Read | Write |
|---|---|---|
| `Integer` | `OpVarRef(dist_K)` → `OpGetInt(0)` | `OpVarRef(dist_K)` → `OpSetInt(0)` |
| `Text` | `OpVarRef(dist_K)` → `OpGetStackText` | `OpVarRef(dist_K)` → `OpAppendStackText` |
| `Vector` | `OpVarRef(dist_K)` → `OpGetStackRef(0)` | `OpVarRef(dist_K)` → `OpAppendVector` |
| `Reference` | `OpVarRef(dist_K)` → `OpGetRef(0)` | `OpVarRef(dist_K)` → `OpSetRef(0)` |

`dist_K = stack_pos_at_instruction - (param_base + K * 12)`

Because each `OpCreateStack(pos)` produces a DbRef with
`pos = stack_cur.pos + absolute_element_position`, the callee's `string_ref_mut`
and `GetStackText` land at the exact `String` address on the caller's stack with
no offset arithmetic.

Nested tuples are recursively flattened: an inner tuple element `(U, V)` contributes
two DbRefs to the list. Leaf elements are always primitive/owned types.

---

## Implementation Phases

### Core phases 1–4

#### Phase 1 — Type system (`src/data.rs`, `src/typedef.rs`)

1. Add `Type::Tuple(Vec<Type>)` to `data.rs`.
2. Implement `size_of`, `depend()`, `depending()`, and `Display` for `Tuple`;
   both `depend()` and `depending()` recurse into nested `Tuple` elements.
3. Expose three free functions in `data.rs` for reuse by closure records (A5)
   and any future compound-type feature:
   - `element_size(t: &Type) -> usize` — stack width of a single element type.
   - `element_offsets(types: &[Type]) -> Vec<usize>` — byte offset of each element.
   - `owned_elements(types: &[Type]) -> Vec<(usize, &Type)>` — `(offset, type)` pairs
     for elements that require `OpFreeText` or `OpFreeRef` on scope exit.
4. Extend `parse_type` to recognise `'(' type ',' ...` as a tuple type.
5. In `typedef.rs`, reject `Type::Tuple` in any struct field position.
6. In `typedef.rs`, reject `Type::RefVar` in any tuple element position.
7. In `typedef.rs`, reject owned-element types (`text`, `reference`, `vector`,
   collection) inside `Type::RefVar(Tuple(...))` — enforced until Phase 5 lifts
   the restriction.

#### Tests — Phase 1

| Test | What it verifies |
|---|---|
| `error_tuple_in_struct` | compile error when a struct field type is a tuple |
| `error_refvar_in_tuple` | compile error for `(&integer, text)` as element type |
| `element_offsets_helper` | `element_offsets(&[Integer, Text, Float])` returns `[0, 4, 28]` |
| `owned_elements_helper` | `owned_elements(&[Integer, Text, Reference<T>])` returns two entries (text at 4, ref at 28) |

---

#### Phase 2 — Parser (`src/parser/`)

1. **Tuple literal** (`expressions.rs`): in the `'('` branch of `parse_single`,
   after parsing the first expression, check for a comma; if present, collect
   remaining expressions and return `Value::Tuple`.
2. **Element access** (`expressions.rs`): in the postfix loop of `parse_operators`,
   after `.`, accept an `Integer` token as a tuple index. Resolve the element type
   and byte offset; emit the appropriate read expression.
3. **Element assignment** (`expressions.rs`): extend the LHS path to recognise
   `var.N` as assignable when `var` has a `Tuple` type. Emit `OpFreeText` on the
   old text value before writing the new one.
4. **LHS deconstruction** (`expressions.rs`): detect
   `'(' lower_ident { ',' lower_ident } ')' '='` and emit the temporary sequence.
5. **Tuple return type / parameter / annotation**: no extra parsing; handled by the
   extended `parse_type`.
6. **`&(T1, T2)` parameter**: parse `&` followed by a tuple type as
   `Type::RefVar(Tuple(...))`. Owned-element variants are rejected by Phase 1
   item 6 until Phase 5.
7. **Not-iterable guard**: in `parse_for`, check whether the iterable expression
   has a `Tuple` type and emit a compile error if so.

#### Tests — Phase 2

| Test | What it verifies |
|---|---|
| `element_assignment` | `a.0 = 5; a.0 += 3; assert(a.0 == 8)` — read/write and compound-assign |
| `tuple_literal_argument` | `print_pair((42, "hello"))` — inline literal as function argument |
| `three_element_destructure` | `(a, b, c) = foo3()` — parser handles three-name LHS |
| `error_index_out_of_range` | compile error for `t.5` on a 3-element tuple |
| `error_assign_to_temp` | compile error for `foo().0 = 5` |
| `error_wrong_arity_destructure` | compile error when name count ≠ tuple arity |
| `error_tuple_not_iterable` | compile error for `for x in t` where `t` is a tuple |

---

#### Phase 3 — Scope analysis (`src/scopes.rs`)

For a tuple variable leaving scope, emit lifecycle ops in **reverse element index
order** (N-1 down to 0):

- Emit `OpFreeText` for every `text` element at `stack_pos + offset(N)`.
- Emit `OpFreeRef` for every `reference`, `vector`, or collection element.
- Recurse into nested `Tuple` elements before processing the outer element.
- Suppress all free ops for elements of a tuple that is being returned.

**Reverse-order rationale:** If element 0 is `reference<A>` (store K) and element
1 is `reference<B>` (store K+1), `Stores::free()` requires K+1 to be released
before K. Forward iteration frees K first — violating LIFO.

**Recursion rationale:** A `text` element inside a `(text, boolean)` inner tuple
at outer element 1 sits at `stack_pos + offset(1) + inner_offset(0)`. A flat scan
misses it; recursion through `Type::Tuple` catches it at any nesting depth.

#### Tests — Phase 3

| Test | What it verifies |
|---|---|
| `tuple_with_two_refs` | `(reference<A>, reference<B>)` freed in reverse element order; no LIFO violation |
| `nested_tuple_text` | `(integer, (text, boolean))` — text inside nested tuple freed exactly once |

---

#### Phase 4 — Bytecode codegen (`src/state/codegen.rs`)

1. **`Value::Tuple`**: emit caller-allocated-slot convention for all tuple returns.
   The caller pre-allocates the full tuple region; the callee receives a `DbRef`
   and writes each element at its offset using existing stack-ops.
2. **Block result type**: when compiling a function whose return type is `Tuple`,
   set `Block.result = Type::Tuple(...)` and allocate the caller-side slot before
   the `call` opcode, mirroring the text-return path.
3. **Element read**: `generate_var` for the element type at distance
   `stack_pos_at_instruction - (tuple_var_stack_pos + offset(N))`.
4. **Element write (primitive)**: `OpPutInt` / `OpPutBool` / etc.
5. **Element write (text)**: `OpFreeText` on existing content, then text
   construction sequence.
6. **Tuple copy (`b = a`)**: element-by-element —
   - Primitives: scalar copy opcodes.
   - Text: `OpFreeText` on `b.N`, then `OpText` + `OpAppendText` to deep-copy.
   - DbRef elements: 12-byte copy, with `OpFreeRef` on the old value if owned.
7. **`&(T1, T2)` stub**: emit a single `OpCreateStack` at the tuple start
   (primitive-only tuples only; owned-element `&tuple` is rejected by Phase 1
   item 6 until Phase 5).

#### Tests — Phase 4

| Test | What it verifies |
|---|---|
| `return_and_destructure` | `(a, b) = foo()` where `foo` returns `(integer, text)` — end-to-end |
| `store_and_access` | `t = foo(); assert(t.0 == 1); assert(t.1 == "x")` |
| `copy_primitive_independent` | `u = t; u.0 = 99; assert(t.0 == 1)` — integer copies are independent |
| `copy_text_independent` | `u = t; u.1 += " world"; assert(t.1 == "hello")` — text is deep-copied |
| `copy_vector_shared` | `u = t; u.0 += [4]` — vector DbRef is shared between copies |
| `reassign_frees_text` | `a = foo(); a = foo()` — old text buffer freed on second assignment |
| `element_text_reassign` | `a.1 = "new"; assert(a.1 == "new")` — old buffer freed, new buffer valid |
| `tuple_param_by_value` | pass `(integer, float)` as argument; callee reads both elements |
| `tuple_param_by_ref_prim` | `&(integer, integer)` mutation visible to caller |
| `tuple_with_text_scope` | text element freed at scope exit — no leak, no double-free |
| `tuple_with_vector` | vector DbRef shared; append through copy visible in original |
| `nested_tuple` | `(integer, (float, boolean))` — `t.1.0` and `t.1.1` access and assignment |
| `nested_destructure` | `(a, b) = foo()` where `b` is itself a tuple |
| `fn_element` | `fn(integer) -> integer` element is callable |
| `text_deconstruct_independence` | `(s, n) = make_text_int(); s += " x"` — `s` has its own buffer |
| `sizeof_tuple` | `sizeof((integer, text)) == 28` |

---

### Mitigation phases 5–7

#### Phase 5 — SC-4: Reference-tuple parameters with owned elements

**Problem:** A single `OpCreateStack` at the tuple start gives the callee a DbRef
to the tuple's beginning. Reading or writing a `text` element at an interior offset
(e.g. `DbRef.pos + 4`) requires an offset-aware string helper that does not exist.
Without it, stack-text ops access the wrong memory location.

**Solution: per-element DbRef expansion**

`&(T1, T2, ..., TN)` is lowered at codegen to **N consecutive DbRefs** (12 bytes
each, total N×12 bytes). The caller emits one `OpCreateStack` per element at the
element's individual stack position:

```
// Caller preparing &(integer, text, vector<X>):
OpCreateStack(tuple_sp + offset(0))   // → DbRef_0 → integer slot
OpCreateStack(tuple_sp + offset(1))   // → DbRef_1 → text String slot
OpCreateStack(tuple_sp + offset(2))   // → DbRef_2 → vector DbRef slot
```

The callee accesses each element via its DbRef using the standard `RefVar(TK)` read/
write sequences (see the table in [IR Representation](#ir-representation)). No new
opcodes are required.

**`&` mutation enforcement:** any write to any element's DbRef satisfies the "must
mutate" requirement.

**Nested tuples:** an inner tuple `(U, V)` at position K is recursively flattened,
contributing two DbRefs to the list. Accessing `pair.K` where K is a nested tuple
returns a synthetic view over the corresponding DbRef sub-range. Assigning a nested
tuple as a whole copies elements from the sub-range.

**Changes required:**

1. Remove the owned-element rejection added in Phase 1 item 6.
2. Replace the single-`OpCreateStack` stub in Phase 4 item 7 with the N-DbRef
   expansion.
3. Extend `generate_var` to recognise `RefVar(Tuple(...))` and route to element-by-
   element DbRef addressing.

#### Tests — Phase 5

| Test | What it verifies |
|---|---|
| `tuple_ref_text` | `&(integer, text)` — callee appends to text element; caller observes change |
| `tuple_ref_vector` | `&(integer, vector<X>)` — callee appends to vector; caller observes change |
| `tuple_ref_nested` | `&(integer, (float, boolean))` — callee writes inner-tuple element through ref |
| `tuple_ref_no_new_opcodes` | bytecode dump confirms only existing `RefVar` opcode sequences are used |

---

#### Phase 6 — SC-8: Tuple-aware mutation guard

**Problem:** The for-loop mutation guard tracks the iterated collection by variable
index (`u16`). For `for x in t.0 { t.0 += [x] }`, the collection is element 0 of
variable `t`. A guard tracking only `t_var_nr` fires on any mutation of `t` (too
broad) or misses mutations to `t.0` (too narrow).

**Solution: element-path tracking**

Extend the iterator context in `variables/`:

```rust
struct IterContext {
    collection_var: u16,
    element_path: Vec<usize>,  // [] = whole variable; [0] = .0; [1, 2] = .1.2
}
```

`for x in expr` resolves `expr` to a `(var_nr, element_path)` pair:

- `for x in items` → `(items_nr, [])`
- `for x in t.0` → `(t_nr, [0])`
- `for x in t.1.2` → `(t_nr, [1, 2])`

The guard fires when the mutated path is a **prefix-match or exact-match** of the
iterated path. Mutating `t.0` or `t.0.field` while iterating `t.0` is forbidden.
Mutating `t.1` while iterating `t.0` is allowed.

**Changes required:**

1. Add `element_path: Vec<usize>` to the iterator-tracking record in `variables/`.
2. Extend `parse_for` to record the element path from tuple-element access chains.
3. Extend the mutation-check site to compare `(var_nr, element_path)` pairs using
   the prefix-match rule on `+=` and `#remove`.

#### Tests — Phase 6

| Test | What it verifies |
|---|---|
| `error_mutate_iterated_tuple_element` | compile error for `for x in t.0 { t.0 += [x] }` (exact match) |
| `error_mutate_iterated_sub_element` | compile error for `for x in t.0 { t.0.field = v }` (prefix match) |
| `allow_mutate_different_tuple_element` | `for x in t.0 { t.1 += [x] }` is allowed |
| `allow_mutate_outer_variable` | `for x in t.0 { other += [x] }` is allowed |

---

#### Phase 7 — SC-7: `not null` annotation for tuple integer elements

**Problem:** Integer elements always carry the `i32::MIN` null sentinel. There is
no way to declare an element non-nullable, so the compiler emits null-check paths
even when the programmer guarantees non-null values, and `i32::MIN` is unavailable
as a data value.

**Solution: `not null` as a compile-time annotation**

```loft
(integer not null, text)
(integer limit(0, 100) not null, float)
```

This is a **type-checker-only** change. The runtime representation is unchanged
(4-byte i32; `i32::MIN` remains the internal sentinel). Effects:

1. **Null-check elision:** `!t.0` and `t.0 == null` produce a compile warning and
   the null branch is removed from codegen.
2. **Assignment checking:** assigning `null` to a `not null` element is a compile
   error.
3. **Literal checking:** passing `null` for a `not null` element in a tuple literal
   or function call is a compile error.

**Caveat:** arithmetic that produces exactly `i32::MIN` at runtime is still treated
as null by any remaining null-aware op. This matches the existing contract for
`not null` struct fields. For the full 32-bit range without null-sentinel ambiguity,
use `long`.

**Type representation:**

```rust
Integer(i32, u32, bool),   // (min, max, not_null)
```

All existing `Integer(min, max)` constructions become `Integer(min, max, false)`.
The compiler flags every unhandled match arm, making the migration exhaustive.

**Changes required:**

1. Change `Type::Integer(i32, u32)` to `Type::Integer(i32, u32, bool)` in `data.rs`;
   update all match arms.
2. Extend `parse_type` to accept `not null` after an integer type and set the flag.
3. In the type checker, warn on null comparisons and error on null assignment to
   `not null` elements.
4. In codegen, elide `OpConvBoolFromInt` null-check paths for `not null` integers.

#### Tests — Phase 7

| Test | What it verifies |
|---|---|
| `not_null_element_no_null_check` | bytecode for `!t.0` on `integer not null` contains no null-check branch |
| `not_null_element_assignment` | literal non-null integer assigned to `not null` element compiles and runs |
| `error_null_assign_to_not_null` | compile error when `null` assigned to a `not null` element |
| `not_null_long_range` | documents that `long` is needed when `i32::MIN` must be a valid data value |

---

## Known Limitations

All safety concerns identified during design have been addressed in the phases above.
The table below summarises their resolution for reference.

| ID | Issue | Resolution |
|---|---|---|
| SC-1 | Text element use-after-free on return | Caller-allocated slots (Phase 4) |
| SC-2 | Text double-free on tuple copy | Deep-copy text elements (Phase 4) |
| SC-3 | LHS deconstruction use-after-free | Follows from SC-2 deep-copy (Phase 4) |
| SC-4 | `&tuple` with owned elements — no access mechanism | Per-element DbRef expansion (Phase 5) |
| SC-5 | LIFO store violation on scope exit | Reverse element free order (Phase 3) |
| SC-6 | Nested tuple lifecycle missed by flat scan | Recursive scope analysis (Phase 3) |
| SC-7 | `not null` inaccessible for tuple integers | `integer not null` annotation (Phase 7) |
| SC-8 | Mutation guard unaware of tuple element identity | Element-path tracking (Phase 6) |

One residual constraint: `long` must be used instead of `integer not null` when
`i32::MIN` is genuinely needed as a runtime data value, since the annotation is
a compile-time contract only and does not change the runtime null sentinel.

---

## Non-Goals

- **Named tuple fields** — use a named struct instead.
- **Single-element tuples** — use the type directly.
- **Tuples in struct fields** — explicitly disallowed; use a named struct.
- **Tuple iteration** — `for x in t` is a compile error; iterate element collections.
- **Tuple patterns in `match` arms** — destructure at the call site before `match`.
- **Tuple equality / comparison** — compare elements individually.
- **Whole-tuple format strings** — `"{t}"` is not supported; format elements individually.
- **Variadic or runtime-arity tuples** — all arities are compile-time constants.

---

## See also

- [LOFT.md](LOFT.md) — language reference (types, structs, functions)
- [COMPILER.md](COMPILER.md) — parser, IR, type system, two-pass design
- [INTERMEDIATE.md](INTERMEDIATE.md) — `Type` and `Value` enum details; bytecode operators;
  `text_positions` invariant; `string_ref_mut` vs `GetStackText`; `generate_var` table
- [SLOTS.md](SLOTS.md) — stack slot assignment (relevant for owned tuple elements)
- [MATCH.md](MATCH.md) — pattern matching design (non-goal: tuple patterns in match)
- [PLANNING.md § A5](PLANNING.md) — closure capture; reuses `element_size`, `element_offsets`,
  and `owned_elements` from Phase 1 of this design for closure record layout and lifetime
