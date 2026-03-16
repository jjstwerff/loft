# Loft Language Design Inconsistencies

This document catalogues asymmetries, surprising behaviours, and design tensions in the
loft language. It focuses on things that *work* but feel inconsistent — bugs and crashes
belong in [PROBLEMS.md](PROBLEMS.md) instead.

Each entry notes **Severity**: High = silent wrong behaviour; Medium = surprising but safe;
Low = cosmetic or minor. Where a path to resolution is obvious it is included.

---

## Contents
- [~~1. `const` Has Two Different Meanings~~ **FIXED**](#1-const-has-two-different-meanings--fixed-2026-03-14)
- [2. Vector Has a Much Richer API Than Sorted / Index / Hash](#2-vector-has-a-much-richer-api-than-sorted--index--hash)
- [3. Loop Attribute `#index` Has Different Semantics on Text vs. Vector](#3-loop-attribute-index-has-different-semantics-on-text-vs-vector)
- [6. Plain Enums Cannot Have Methods; Struct-Enum Variants Can](#6-plain-enums-cannot-have-methods-struct-enum-variants-can)
- [8. Method vs. Free Function Is an Arbitrary Standard-Library Choice](#8-method-vs-free-function-is-an-arbitrary-standard-library-choice)
- [9. Text/Character Split: Indexing and Slicing Return Different Types](#9-textcharacter-split-indexing-and-slicing-return-different-types)
- [10. Null Sentinel Values Vary Invisibly by Type](#10-null-sentinel-values-vary-invisibly-by-type)
- [11. Reverse Iteration Works on Ranges and Vectors but Panics on Sorted/Index](#11-reverse-iteration-works-on-ranges-and-vectors-but-panics-on-sortedindex)
- [12. Index Range-Query Second-Key Semantics Depend on Sort Direction](#12-index-range-query-second-key-semantics-depend-on-sort-direction)
- [~~13. Library Definitions Always Require `libname::` Prefix~~ **FIXED**](#13-library-definitions-always-require-libname-prefix--fixed-2026-03-16)
- [14. Format Strings: Nested Literals Fail (Zero-Pad Fixed)](#14-format-strings-nested-literals-fail-zero-pad-fixed)
- [~~15. `fn <name>` Function References Only Work in `par(...)` Context~~ **FIXED**](#15-fn-name-function-references-only-work-in-par-context--fixed-2026-03-15)
- [~~16. `Format` Enum Mixes File Mode With Absence~~ **FIXED**](#16-format-enum-mixes-file-mode-with-absence--fixed-2026-03-14)
- [17. Implicit Type Coercion Rules Are Not Uniform](#17-implicit-type-coercion-rules-are-not-uniform)
- [18. `#break` Reuses the `#attribute` Syntax for a Control-Flow Statement](#18-break-reuses-the-attribute-syntax-for-a-control-flow-statement)
- [23. `sizeof(u8)` / `sizeof(u16)` Return the Stack Size, Not the Byte-Packed Size](#23-sizeofu8--sizeofou16-return-the-stack-size-not-the-byte-packed-size)
- [24. ~~For-loop Mutation Guard Only Catches Direct Variable Append~~ **FIXED**](#24-for-loop-mutation-guard-only-catches-direct-variable-append--fixed-2026-03-14)
- [Summary by Severity](#summary-by-severity)

---

## ~~1. `const` Has Two Different Meanings~~ **FIXED 2026-03-14**

Local `const` variables now receive compile-time enforcement via `Variable.const_param`,
the same flag used for `const` parameters.  Any write (direct reassignment, `+=` on
text/vector, struct-constructor assignment) is a compile-time error in all build modes.
The debug-build runtime store-lock (`n_set_store_lock`) is no longer emitted.

```loft
fn read(v: const Counter) -> integer { v.value }   // compile-time: static proof
const c = Counter { value: 42 };                   // compile-time: any write = error
c = Counter { value: 9 }  // ERROR: Cannot modify const variable 'c'
```

Tests: `const_local_*` in `tests/immutability.rs` (15 tests total).

---

## 2. Vector Has a Much Richer API Than Sorted / Index / Hash

**Severity: Medium** — partially resolved 2026-03-14 (loop attributes documented/implemented)

All four collection types use the same `+=` and `for` syntax.  The iteration toolkit is:

| Feature | `vector` | `sorted` | `index` | `hash` |
|---|---|---|---|---|
| `#first`, `#count` in loop | ✓ | ✓ | ✓ | N/A (cannot iterate) |
| `#index` in loop | ✓ (0-based) | ✓ (0-based array pos) | ✗ compile error | N/A |
| `e#remove` in filtered loop | ✓ | ✓ | ✓ | use `h[key] = null` |
| `rev()` reverse iteration | ✓ (via range) | ✓ | ✓ | N/A |
| Slicing `[a..b]` | ✓ | ✗ | ✗ | ✗ |
| Comprehension `[for x in ...]` | ✓ | ✗ | ✗ | ✗ |
| Filtered `for x in c if cond` | ✓ | ✓ | ✓ | N/A |

Remaining API gaps (slicing, comprehension) are structural and not planned.

---

## 3. Loop Attribute `#index` Has Different Semantics on Text vs. Vector

**Severity: Medium** (also [PROBLEMS.md](PROBLEMS.md) #23)

```loft
txt = "12😊🙃45"
for c in txt {
    c#index   // byte offset AFTER the iterator has advanced (post-advance)
    c#next    // byte offset after this character — the "next" position
}

for v in vec {
    v#index   // 0-based element position (counts whole elements)
    // no v#next
}
```

Both are called `#index` but one is a byte offset and the other is an element count.
The text form has an additional `#next` helper that vectors don't need. A programmer
expecting "position of the current item" will get a byte offset on text, not a character
count.

---

## ~~4. `&` Ref-Parameter Works Differently for Primitives vs. Collections~~ **FIXED**

~~**Severity: High**~~ **Fixed 2026-03-13** ([PROBLEMS.md](PROBLEMS.md) #4).

`assign_refvar_vector` was added to `parser.rs` (analogous to `assign_refvar_text`),
called from `parse_assign` after the text path. It intercepts `v += expr` inside a
`&vector<T>` parameter and emits `OpVarRef + OpGetStackRef(0) + OpAppendVector` so the
caller's vector is extended in place.

```loft
fn append(v: &vector<integer>, x: integer) {
    v += [x];   // now works: caller sees the new element
}
```

Tests: `ref_param_append_bug` in `tests/issues.rs`.

---

## 5. ~~Empty Vector Field Cannot Be Appended To~~ **FIXED**

~~**Severity: High**~~ **Fixed 2026-03-12** ([PROBLEMS.md](PROBLEMS.md) #5).

`b.items += [1]` and `b.items += 1` now work on empty fields.  The fix routes
scalar and bracket-wrapped element appends through `new_record` with
`is_field = true` when `var_nr == u16::MAX`.

Tests: `vec_field_append_scalar`, `vec_field_append_bracket_scalar_works`,
`vec_field_append_bracket_multi_works` in `tests/issues.rs`.

---

## 6. Plain Enums Cannot Have Methods; Struct-Enum Variants Can

**Severity: Medium**

```loft
enum Direction { North, East, South, West }    // plain enum (integer under the hood)
enum Shape { Circle { r: float }, Rect { ... } }  // struct enum

fn area(self: Circle) -> float { ... }   // OK: polymorphic dispatch on variant type
fn area(self: Rect)   -> float { ... }   // OK

fn label(self: Direction) -> text { ... }  // ERROR: plain enum variant is not a struct type
```

Struct-enum variants are distinct struct types and support method dispatch. Plain enum
values are named integers and cannot be method receivers. There is no way to attach
behaviour directly to a plain enum variant — you must write a free function that switches
on the value. This surprises users who expect uniform OOP-like dispatch across all enums.

**Resolution path:** T1-4 (`match` expression) resolves this. Once `match` exists, a free
function on a plain enum is simply a `match` body — no per-variant method syntax needed.
T1-5 (plain enum methods) is superseded by T1-4 and will be skipped.

---

## ~~7. Polymorphic Text Methods on Struct-Enum Crash at Runtime~~ **FIXED 2026-03-13**

**Was: High** (also [PROBLEMS.md](PROBLEMS.md) #3 — now fixed)

Three bugs were fixed: (1) `enum_fn` now forwards `RefVar(Text)` buffer arguments to
each variant call in the dispatcher IR; (2) `generate_call` suppresses `OpGetStackText`
when forwarding a `RefVar` arg to a `RefVar` param; (3) `format_stack_float` corrected
from `pos - 12` to `pos - 16` (off-by-4 that caused SIGSEGV).
Test: `polymorphic_text_method_on_enum` in `tests/issues.rs`.

---

## 8. Method vs. Free Function Is an Arbitrary Standard-Library Choice

**Severity: Low**

There is no language-level rule about what becomes a method vs. a free function; it
depends entirely on whether the standard library defines the first parameter as `self`.

```loft
length(v)           // free function — NOT v.length()
length(text)        // same free function name for both text and vector

text.starts_with(s) // method — defined as fn starts_with(self: text, ...)
text.find(s)        // method
abs(n)              // free function — NOT n.abs()
pow(b, e)           // free function
```

A user cannot predict whether an operation is a method or a free function without looking
it up. Some text operations are methods (`starts_with`, `find`, `trim`) while the most
basic one (`length`) is a free function. The language allows both forms equally; the
inconsistency is in the standard-library naming choices.

---

## 9. Text/Character Split: Indexing and Slicing Return Different Types

**Severity: Low**

```loft
txt = "hello"
txt[0]      // character — a single Unicode scalar value
txt[0..1]   // text — a one-character string

vec = [1, 2, 3]
vec[0]      // integer
vec[0..1]   // vector<integer> — consistent: element type vs. collection type
```

`txt[i]` and `txt[i..i+1]` are different types (`character` vs. `text`), making string
manipulation awkward: building a text from characters requires `"{c}"` interpolation, not
direct concatenation with `+`. The vector pattern (element vs. slice of same collection
type) would be cleaner.

---

## 10. Null Sentinel Values Vary Invisibly by Type

**Severity: Medium**

Every nullable type uses a different in-band sentinel to represent null:

| Type | Null sentinel |
|---|---|
| `integer` | `i32::MIN` (-2 147 483 648) |
| `long` | `i64::MIN` |
| `float` / `single` | `NaN` |
| `text` | opaque `STRING_NULL` pointer |
| `reference` | `DbRef { rec: 0 }` |
| `enum (plain)` | byte value `255` |
| `boolean` | not documented |
| `character` | not documented |

Practical consequence: a legitimate integer value of `i32::MIN` is indistinguishable from
null. Division by zero returns `i32::MIN`, but so does `(-2147483648 / 1)`. Arithmetic
that reaches the sentinel exactly appears null.

For floats, `NaN != NaN` in IEEE 754, so `f == null` requires a special null check; the
language papers over this with `!f` but the underlying sentinel leaks when floats are
compared directly.

**Resolution path:** Document the sentinel for every type. Clarify whether `i32::MIN` can
arise from non-null arithmetic, and if so, provide a way to distinguish it.

---

## 11. ~~Reverse Iteration Works on Ranges and Vectors but Panics on Sorted/Index~~ **FIXED 2026-03-14**

**Severity: Medium** (was; [PROBLEMS.md](PROBLEMS.md) #17 — now FIXED)

```loft
for e in pows[rev(0..=3)] { }   // WORKS: reverse a range slice of a vector
for r in rev(db.sorted) { }     // NOW WORKS (fixed 2026-03-14)
```

`rev(sorted_col)` and `rev(index_col)` now produce reverse-key-order iteration.
The fix added `Parser::reverse_iterator` flag, `vector::vector_step_rev()`, and
updated `step()` in `state.rs` to call it when bit 64 is set in `on`.

---

## 12. Index Range-Query Second-Key Semantics Depend on Sort Direction

**Severity: Medium**

```loft
struct Elm { nr: integer, key: text, value: integer }
struct Db { map: index<Elm[nr, -key]> }   // key is DESCENDING

for v in db.map[83..92, "Two"] { }
// Means: nr ∈ [83, 92) AND key from "Two" going DOWNWARD
// because the key field is declared descending
```

The second position in a range query is not a range — it is a boundary in the sort
direction of that field. If the field is ascending, `"Two"` means "from Two upward"; if
descending it means "from Two downward". The sort direction is declared at the struct
definition, which may be far from the query. This makes range queries hard to reason about
without constantly checking the index declaration.

---

## 13. ~~Library Definitions Always Require `libname::` Prefix~~ **FIXED 2026-03-16**

**Severity: Low** (was; [PROBLEMS.md](PROBLEMS.md) #13 — now FIXED by T1-2)

`use mylib::*` now imports all names from `mylib` into the current scope, and
`use mylib::Point, add` imports specific names. Local definitions shadow imported
names silently (local wins). Importing a name that does not exist in the library
produces a compile-time error. Three tests in `tests/imports.rs`.

---

## 14. Format Strings: Nested Literals — **FIXED 2026-03-14**

**Severity: Low** (was; [PROBLEMS.md](PROBLEMS.md) #9 — now FIXED)

```loft
// Nested string literal inside a format expression now works (fixed 2026-03-14)
assert("{\"hello\":3}" == "hello", ...)   // OK
hw = "hello"; assert("{hw:3}" == "hello") // also OK

// Zero-padding negative integers places sign first (fixed 2026-03-13)
assert("{-1:03}" == "-01")   // correct; was "0-1" before the fix
```

`format_int`/`format_long` in `native.rs` detect `token == b'0'` + non-empty sign and emit the sign before the zero-padded digit string.

Nested string literal fix: `src/lexer.rs` `in_format_expr` flag + `string_nested()` method. See [PROBLEMS.md](PROBLEMS.md) #9 for full details.

---

## ~~15. `fn <name>` Function References Only Work in `par(...)` Context~~ **FIXED 2026-03-15**

`fn <name>` references are now fully callable in any expression context:

```loft
fn square(n: integer) -> integer { n * n }
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }

fn test() {
    f = fn square;
    result = f(9);                   // direct call through fn-ref variable: 81
    result2 = apply(fn square, 7);   // pass fn-ref as parameter: 49
}
```

**Implementation:**
- `Value::CallRef(v_nr, args)` — new IR node for dynamic dispatch through a fn-ref variable.
- `OpCallRef(fn_var, arg_size)` — new bytecode op in `02_images.loft` (opcode 252) / `fill.rs`.
- `State::fn_call_ref` reads `d_nr` from the stack slot, looks up `fn_positions[d_nr]`, calls `fn_call`.
- `parse_call` detects calls where the callee name is a local `Type::Function` variable and emits `CallRef` instead of `Call`.
- `Data::find_fn` now guards against `type_def_nr == u32::MAX` (hit when the first argument has `Type::Function` type) to avoid a panic looking up an invalid definition.

Limitations still present: fn-refs cannot be stored in struct fields or vector elements (no stack layout for owned Function slots); text/reference return types not supported in `parallel_for` workers (store merging not yet implemented).

---

## ~~16. `Format` Enum Mixes File Mode With Absence~~ **FIXED 2026-03-14**

`f#exists` is now a first-class boolean attribute on `File` variables.  It returns
`true` when the file or directory exists (i.e. `f#format != Format.NotExists`).

```loft
f = file("path");
if !f#exists { println("not found"); return; }
if f#format == TextFile { ... }     // now only called when file exists
```

Implementation: `file_op()` in `parser.rs` — new `"exists"` keyword branch emits
`OpGetEnum(var, 32) != Format.NotExists`.  `Format.NotExists` is kept for backward
compatibility via `f#format`.
Tests: `file_exists_true`, `file_exists_false` in `tests/issues.rs`.

---

## 17. Implicit Type Coercion Rules Are Not Uniform

**Severity: Low**

| Conversion | Form required |
|---|---|
| Any type → boolean (`if v`, `!v`, `assert`) | Implicit |
| Integer ↔ float in arithmetic | Implicit widening |
| Float → integer (truncate) | Explicit: `f as integer` |
| Text → integer/float | Explicit: `"5" as integer` |
| Integer → text | Implicit inside `"{v}"` only |
| Plain-enum name → enum | Explicit: `"West" as Direction` |
| Struct-enum variant → parent enum | Implicit on assignment |

There is no single rule. Boolean coercion is always implicit; most numeric conversions
require `as`; format-string interpolation converts silently. A user cannot predict from
first principles whether a given conversion is automatic or requires an explicit cast.

---

## 18. `#break` Reuses the `#attribute` Syntax for a Control-Flow Statement

**Severity: Low**

```loft
for x in 1..5 {
    for y in 1..5 {
        if cond { x#break; }   // breaks out of the x loop
    }
}
```

The `#` suffix notation is used for two completely different purposes:
- **Loop metadata** — `x#first`, `x#count`, `x#index`, `x#next`, `x#remove`: read or
  write properties that are expressions or assignment targets.
- **Loop control** — `x#break`: a statement that transfers control and has no value.

Making `x#attr` sometimes an expression and sometimes a jump instruction complicates the
mental model for the `#` notation.

---

## 19. ~~`for c in enum_vector` Loops Forever~~ **FIXED**

~~**Severity: High**~~ **Fixed 2026-03-13** (`src/fill.rs::get_enum`, [PROBLEMS.md](PROBLEMS.md) #30): out-of-bounds vector reads returned `i32::MIN`, which truncated to `0u8` — a valid enum value — so `conv_bool_from_enum` never saw the null sentinel (`255`). Fix maps `i32::MIN → 255u8`. Tests: `tests/scripts/08-enums.loft`.

---

## ~~20. Enum Comparison Operators Partially Undocumented~~ **FIXED 2026-03-13**

Added a note under "Enum types" in [`LOFT.md`](LOFT.md): all six comparison operators (`==`, `!=`, `<`, `<=`, `>`, `>=`) work on plain enums in declaration order.

---

## 21. Bitwise `^`, `|`, `&` Treat Zero as a Null Operand — **FIXED**

~~**Severity: High**~~ **Fixed 2026-03-13** (`src/native.rs`): removed `&& v2 != 0` guard from XOR/AND/OR bitwise operators. Division, remainder, and shift operators retain their guard. Tests: `tests/scripts/01-integers.loft`.

---

## 22. Missing Polymorphic Method for a Struct-Enum Variant — **FIXED**

~~**Severity: High**~~ **Fixed 2026-03-13.** `enum_fn` now emits a `Warning` for each unimplemented variant. Silence it with an empty-body stub (`fn area(self: Rect) -> float { }`), which returns null at runtime without panicking and suppresses the unused-parameter warning.

**Remaining gap fixed (2026-03-13):** Direct call to an unimplemented variant now emits `"Unknown field Rect.area"` without cascading parse errors or panics. The `field` function in `parser.rs` now consumes the trailing `(…)` after an unknown-field diagnostic so the rest of the statement parses cleanly. Tests: `direct_call_to_stub` and `direct_call_unimplemented_variant` in `tests/parse_errors.rs`.

Tests: `tests/parse_errors.rs::missing_variant_impl`, `tests/scripts/08-enums.loft` (Blob/Full/Hollow).

---

## 24. ~~For-loop Mutation Guard Only Catches Direct Variable Append~~ **FIXED 2026-03-14**

~~**Severity: Low**~~

**Fixed 2026-03-14.** The guard now covers both cases:

```loft
for e in v         { v += [x]; }        // ERROR — direct var (was already caught)
for e in db.items  { db.items += [x]; } // ERROR — field access (now caught too)
```

**How the fix works:**
- `Iterator` struct (`variables.rs`) already had a `value: Box<Value>` field; previously it
  stored a temp-copy variable for vectors, hiding the original expression.
- Added `Function::set_coll_value(orig: Value)` to restore the original collection expression
  after the vector temp-copy substitution in `parse_for`.
- Added `Function::is_iterated_value(val: &Value) -> bool` — walks active iterators and
  compares the LHS `Value` using `PartialEq`.
- In `parse_assign`, the `+=` guard now checks two cases: (1) `Value::Var` LHS against
  `is_iterated_var`, (2) non-`Var` LHS against `is_iterated_value`.

**Tests:** `for_loop_mutation_guard_simple_var`, `for_loop_mutation_guard_field_access`,
`for_loop_mutation_guard_different_field_ok` in `tests/issues.rs`.

---

## 23. `sizeof(u8)` / `sizeof(u16)` Return the Stack Size, Not the Byte-Packed Size

**Severity: Low**

Range-constrained integer types (`u8`, `u16`, `i32`) pack to 1, 2, and 4 bytes respectively
when stored as struct fields. However, using them as an argument to `sizeof()` returns the
**stack slot size** (always 4, the underlying `integer`), not the field byte size:

```loft
sizeof(u8)     // 4  — surprising: "should be 1"
sizeof(u16)    // 4  — surprising: "should be 2"
sizeof(integer) // 4  — consistent

struct Tiny { a: u8, b: u8 }
sizeof(Tiny)   // 2  — correct: fields are packed
```

So `sizeof(u8)` and `sizeof(Tiny.a)` give different answers for the same field type. A
programmer reasoning about struct layout will find `sizeof(u8) != sizeof(u8 in a struct)`.

The distinction is:
- `sizeof(TYPE_KEYWORD)` — stack slot size used for local variables (always the base type
  size: `integer` = 4).
- `sizeof(STRUCT)` — sum of packed field sizes as stored in the database/vector.

**Resolution path:** Either document the distinction clearly in the language reference, or
make `sizeof(u8)` return 1 to match struct packing. The second option would require `sizeof`
to consult the type's declared limit range rather than the underlying storage class.

---

## Summary by Severity

### High (silent wrong behaviour)
| # | Issue |
|---|---|
| ~~4~~ | ~~`&` ref-param: vector `+=` silently discards new elements~~ **FIXED** |
| ~~5~~ | ~~Empty vector field: `b.items += [x]` silently no-ops~~ **FIXED** |
| ~~7~~ | ~~Polymorphic text methods on struct-enum variants cause runtime crash~~ **FIXED** |
| ~~19~~ | ~~`for c in enum_vector` loops infinitely~~ **FIXED** |
| ~~21~~ | ~~`a ^ 0`, `a \| 0`, `a & 0` all return null instead of the correct value~~ **FIXED** |
| ~~22~~ | ~~Missing polymorphic method: direct call panics compiler; polymorphic call returns garbage~~ **FIXED** |

### Medium (surprising but safe)
| # | Issue |
|---|---|
| 1 | `const` on params = compile-time check; `const` on locals = debug-only runtime lock |
| 2 | `#first`/`#index`/`#remove` availability on sorted/index/hash not documented |
| 3 | `#index` is byte-offset on text, element-position on vector |
| 6 | Plain enums cannot have methods; struct-enum variants can |
| 10 | Null sentinels vary by type; `i32::MIN` is ambiguous with legitimate arithmetic |
| ~~11~~ | ~~`rev()` works on vector; panics on sorted/index~~ **FIXED** |
| 12 | Index range-query second-key boundary depends on undeclared sort direction |
| 14 | Nested string literals in format expressions fail; `{-1:03}` → `"0-1"` not `"-01"` |
| 15 | `fn <name>` function references only consumable by `par(...)` |

### Low (cosmetic or minor)
| # | Issue |
|---|---|
| 8 | Method vs. free function assignment is arbitrary in the standard library |
| 9 | `txt[i]` is `character`; `txt[i..i+1]` is `text` — different types |
| ~~13~~ | ~~No wildcard import; `libname::` prefix always required~~ **FIXED** |
| 16 | `Format` enum includes `NotExists` (absence mixed with file mode) |
| 17 | Type coercion rules are not uniform (implicit / explicit / format-only) |
| 18 | `x#break` is a jump statement, reusing the `#attribute` expression syntax |
| 20 | Enum `>`, `<=`, `>=` operators work but are not documented |
| 23 | `sizeof(u8)` returns 4 (stack size), not 1 (field byte size) |
| ~~24~~ | ~~For-loop mutation guard only catches direct variable append; field access bypasses it~~ **FIXED** |

---

## See also
- [PROBLEMS.md](PROBLEMS.md) — Known bugs, limitations, workarounds, and fix plans
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
