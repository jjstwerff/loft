
# Loft Language Reference

Loft is a statically-typed, imperative scripting language with null safety and built-in parallel execution.
Source files use the `.loft` extension. The language compiles to an internal bytecode representation
and can emit Rust code for host integration.

**Quick reference with common patterns and gotchas:** see the loft-write skill (`.claude/skills/loft-write/SKILL.md`).

---

## Contents
- [Naming Conventions (enforced by the parser)](#naming-conventions-enforced-by-the-parser)
- [Types](#types)
- [Declarations](#declarations)
- [Operators](#operators)
- [Literals](#literals)
- [String formatting](#string-formatting)
- [Control flow](#control-flow)
- [Variables](#variables)
- [Vectors](#vectors)
- [Key-based collections (hash / index / sorted)](#key-based-collections-hash--index--sorted)
- [Structs and record initialization](#structs-and-record-initialization)
- [Methods and function calls](#methods-and-function-calls)
- [Assertions](#assertions)
- [Sizeof](#sizeof)
- [Polymorphism / dynamic dispatch](#polymorphism--dynamic-dispatch)
- [File structure](#file-structure)
- [External function annotations (`#rust`, `#iterator`)](#external-function-annotations-rust-iterator)
- [Operator definitions (internal)](#operator-definitions-internal)
- [Shebang](#shebang)
- [Summary of grammar (informal)](#summary-of-grammar-informal)
- [Best Practices](#best-practices)
- [Design decisions and constraints](#design-decisions-and-constraints)

---

## Naming Conventions (enforced by the parser)

| Construct              | Convention         | Examples               |
|------------------------|--------------------|------------------------|
| Functions, variables   | `lower_case`       | `my_fn`, `count`       |
| Types, structs, enums  | `CamelCase`        | `Terrain`, `Format`    |
| Enum values            | `CamelCase`        | `Text`, `FileName`     |
| Constants              | `UPPER_CASE`       | `PI`, `MAX_SIZE`       |
| Operator definitions   | `OpXxx` prefix     | `OpAdd`, `OpEqInt`     |

---

## Types

### Primitive types

| Type        | Description                                      |
|-------------|--------------------------------------------------|
| `boolean`   | `true` / `false`                                 |
| `integer`   | 32-bit signed integer (range can be constrained) |
| `long`      | 64-bit signed integer; literals end with `l`     |
| `float`     | 64-bit floating-point; literals contain a `.`    |
| `single`    | 32-bit float; literals end with `f`              |
| `character` | A single Unicode character                       |
| `text`      | A UTF-8 string; `len()` counts bytes             |

Any variable or field can hold a `null` (absent) value unless declared `not null`.

#### Null representation

Loft uses in-band sentinel values to represent `null`. Each type has a dedicated sentinel:

| Type | Null sentinel | Notes |
|------|---------------|-------|
| `boolean` | `false` | `!b` is true for both `null` and `false` |
| `integer` | `i32::MIN` (-2 147 483 648) | See warning below |
| `long` | `i64::MIN` | Same risk as integer at the `i64` boundary |
| `float` | `NaN` | IEEE 754: `NaN != NaN`, but `!f` correctly detects null |
| `single` | `NaN` (32-bit) | Same as `float` |
| `character` | `'\0'` (NUL) | The null character is not a valid loft character value |
| `text` | internal null pointer | Opaque; `!t` detects it; `len(t)` returns null |
| `reference` | record 0 | Opaque; `!r` detects it |
| plain `enum` | byte `255` | Limits plain enums to 255 variants |

**Integer sentinel warning:** Arithmetic that produces exactly `i32::MIN` (e.g.
`-2147483647 - 1`) becomes indistinguishable from `null`. Division by zero also
returns `null` (`i32::MIN`). If a program needs the full 32-bit signed range, use
`long` instead. For struct fields, `not null` reclaims the sentinel value for
storage, allowing the full range.

**`!value` asymmetry — read carefully:** the unary `!` operator reads as "is null
or default?" but the answer differs by type because the null sentinel is in-band.
For `boolean`, `false` *is* the null sentinel — `!b` is true for **both** `null`
and `false`, and the two cases are indistinguishable.  For `integer`, `0` is a
valid non-null value — `!n` fires **only** for `i32::MIN`, not for `0`.  Code
ported from a boolean guard to an integer guard (or vice versa) silently changes
meaning:

```loft
flag: boolean = false;
if !flag { /* runs */ }     // catches both null and false

count: integer = 0;
if !count { /* skipped */ } // catches only null; zero passes through
```

The idiomatic "zero or null" check on an integer is `count == 0 or !count`,
or simply `count == 0` if the sentinel and zero should be treated the same.

Integer ranges can be constrained with `limit`:
```
integer limit(-128, 127)   // fits in a byte
integer limit(0, 65535)    // fits in a short
```

The default library also defines convenient width-specific aliases:
```
u8    // integer limit(0, 255)
i8    // integer limit(-128, 127)
u16   // integer limit(0, 65535)
i16   // integer limit(-32768, 32767)
i32   // integer (explicit 32-bit)
```

### Composite types

| Type syntax                        | Description                                           |
|------------------------------------|-------------------------------------------------------|
| `vector<T>`                        | Dynamic array of `T`                                  |
| `hash<T[field1, field2]>`          | Hash-indexed collection of `T` on the given fields    |
| `index<T[field1, -field2]>`        | B-tree index (ascending/descending)                   |
| `sorted<T[field]>`                 | Sorted vector on the given fields                     |
| `reference<T>`                     | Reference (pointer) to a stored `T` record            |
| `iterator<T, I>`                   | Iterator yielding `T` using internal state `I`        |
| `fn(T1, T2) -> R`                  | First-class function type                             |

The key fields are declared **inside** the angle brackets with the element type.
A `-` prefix on a field name means descending order:
```
sorted<Elm[-key]>           // single key, descending
index<Elm[nr, -key]>        // two keys: nr ascending, key descending
hash<Count[c, t]>           // compound hash key
```

**Gotcha — iteration direction is declared on the struct, not on the query.**
A `-` prefix on a key field in `sorted<T[-key]>` or `index<T[-key]>` flips
the iteration direction of *every* query against that collection — plain
`for v in db.map`, range queries, and partial-key lookups all walk
descending instead of ascending.  Reading the query site alone never
reveals the direction: the `-` lives in the struct declaration, possibly
hundreds of lines away.  When reviewing a query, cross-check the index
declaration before reasoning about what "starts at X" means.
Regression guards in `tests/issues.rs` (`inc12_sorted_ascending_iterates_forward`,
`inc12_sorted_descending_iterates_backward`) lock the two directions on
otherwise-identical structs.

### Enum types

Simple enums (value types):
```
enum Format {
    Text,
    Number,
    FileName
}
```

Simple enum values support all six comparison operators (`==`, `!=`, `<`, `<=`, `>`, `>=`).
The ordering follows declaration order (`Text < Number < FileName`).

Polymorphic enums (each variant has its own fields, stored as a record):
```
enum Shape {
    Circle { radius: float },
    Rectangle { width: float, height: float }
}
```

### Struct types

```
struct Argument {
    short: text,
    long: text,
    mandatory: boolean,
    description: text
}
```

Fields are declared as `name: type` with optional modifiers **after** the type:
- `limit(min, max)` — constrain an integer field to a range
- `not null` — disallow the null value (enables full integer range in storage)
- `= expr` — stored default value, applied when field is omitted in constructor
- `assert(expr)` / `assert(expr, message)` — runtime constraint checked on every write
- `computed(expr)` — calculated on every access, **not stored** in the record

In default/computed expressions, `$` refers to the record:
```
struct Object {
    name_length: integer = len($.name),   // stored default: computed at construction
    name: text
}

struct Circle {
    radius: float,
    area: float computed(3.14159 * $.radius * $.radius)   // recomputes on access
}
```

Example with all modifiers:
```
struct Point {
    r: integer limit(0, 255) not null,
    g: integer limit(0, 255) not null,
    b: integer limit(0, 255) not null
}
```

---

## Declarations

### Functions

```
fn function_name(param: type, other: type = default_value) -> return_type {
    // body
}
```

- `pub` prefix makes a definition publicly visible (applies to functions, structs, and enums).
- Parameters with a `&` prefix are passed by mutable reference (in-out for any type).
  - **Enforced**: a `&` parameter that is never mutated (directly or transitively through a called function) is a **compile error**. Drop the `&` if the parameter is read-only.
- Parameters with `const` prevent mutation of that parameter inside the function body.
  - `const` is a compile-time check: any assignment to a `const` parameter is an **error**.
  - `& const T` is syntactically valid but unusual — it means "pass by reference, but don't write to it" (which is redundant; prefer plain `const T` passed by value for primitives).
  - `Attribute.constant/mutable` on function definitions are NOT set for `const` user-defined-function parameters (that would break bytecode generation). The check lives purely in `Variable.const_param`.
- Default parameter values are supported.
- Functions without a `->` clause return `void`.
- A function body ending in an expression (without `;`) returns that value.

External (Rust-implemented) functions are declared without a body, followed by `#rust "..."`:
```
pub fn starts_with(self: text, value: text) -> boolean;
#rust "@self.starts_with(@value)"
```

### Constants

```
PI = 3.14159265358979;
```

Constants must be `UPPER_CASE` and are defined at file scope.

### Types and type aliases

```
type MyInt = integer;
type Coord = integer limit(-32768, 32767);
type Handler = fn(Request) -> Response;
type Pair = (integer, text);
```

Type aliases are purely compile-time substitutions — `Handler` and
`fn(Request) -> Response` are the same type.  Aliases for `fn(...)` types
and tuple types are supported (C55).

In library/default files, `size(n)` specifies the storage size in bytes:
```
pub type u8 = integer limit(0, 255) size(1);
```

### Library imports

```
use arguments;
```

Searches for `arguments.loft` in `lib/`, the current directory, directories from the
`LOFT_LIB` environment variable, and relative to the current script.
`use` declarations must appear at the top of the file, before any other declarations.

---

## Operators

Listed by precedence (lowest to highest):

| Precedence | Operators                              | Notes                   |
|------------|----------------------------------------|-------------------------|
| 0 (lowest) | `??`, `?? return`                      | null-coalescing / early return (C56) |
| 1          | `\|\|`, `or`                           | logical OR              |
| 2          | `&&`, `and`                            | logical AND             |
| 3          | `==`, `!=`, `<`, `<=`, `>`, `>=`       | comparison              |
| 4          | `\|`                                   | bitwise OR              |
| 5          | `^`                                    | bitwise EOR             |
| 6          | `&`                                    | bitwise AND             |
| 7          | `<<`, `>>`                             | bit shift               |
| 8          | `-`, `+`                               | addition/subtraction    |
| 9          | `*`, `/`, `%`                          | multiplication/division |
| 10         | `as` (type cast/conversion)            |                         |

Unary operators: `!` (logical not), `-` (negation).

Assignment operators: `=`, `+=`, `-=`, `*=`, `/=`, `%=`.

### The `??` operator (null-coalescing)

`lhs ?? rhs` evaluates to `lhs` if it is not null, otherwise evaluates to `rhs`:

```loft
name = record.optional_field ?? "unknown"
count = map_lookup ?? 0
first = a ?? b ?? c    // chains: first non-null of a, b, c
```

The operator is left-associative and chains: `a ?? b ?? c` is `(a ?? b) ?? c`.
If `lhs` has a statically-known `null` type (the bare `null` literal), `??` returns `rhs` directly.

**Note:** For complex LHS expressions (function calls, field chains), the compiler automatically
materialises the result into a temporary variable so the expression is evaluated exactly once.
Simple variable reads skip the temporary since they have no side effects.

### The `as` operator

Used for explicit type casts and conversions:
```
10l as integer      // long to integer
"json-text" as Program   // deserialize text as a struct
```

### Parsing (JSON / loft text → struct)

`Type.parse(text)` parses JSON or loft-native text into a struct record.
`vector<T>.parse(text)` parses a JSON array into an iterable vector.
Parse errors are accessible via `record#errors`.

```
user = User.parse(`{{"id":42,"name":"Alice"}}`);
scores = vector<Score>.parse(`[{{"value":10}},{{"value":20}}]`);
for e in user#errors { log_warn(e); }
```

---

## Literals

| Kind             | Syntax examples                     |
|------------------|-------------------------------------|
| Integer          | `42`, `0xff`, `0b1010`, `0o17`      |
| Long             | `10l`, `42l`                        |
| Float            | `3.14`, `1.0`                       |
| Single           | `1.0f`, `0.5f`                      |
| Character        | `'a'`, `'😊'`                       |
| Boolean          | `true`, `false`                     |
| Null             | `null`                              |
| String           | `"hello world"`                     |
| Function ref     | `fn double_score`                   |
| Lambda (long)    | `fn(x: integer) -> integer { x * 2 }` |
| Lambda (short)   | `\|x\| { x * 2 }`                    |

A **function reference** (`fn <name>`) produces a `Type::Function` value whose runtime representation is the definition number of the named function.  The compiler resolves the name at **compile time** and errors if it does not exist or is not a function.  The value is 4 bytes (same as `integer`).

**Calling a fn-ref variable:** a variable or parameter of type `fn(T) -> R` can be called directly:

```loft
f = fn double_score           // type: fn(const Score) -> integer
x = f(some_score)             // calls double_score via f
```

**`fn(T) -> R` as a parameter type:**

```loft
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
result = apply(fn double_it, 5)
```

**Lambda expressions** produce an inline anonymous function at the expression level.
Two syntactic forms are available:

```loft
// Long form — all types explicit; always valid
fn(x: integer) -> integer { x * 2 }
fn(x: integer, y: integer) -> integer { x + y }

// Short form — types inferred from call-site context
|x| { x * 2 }
|x, y| { x + y }
|| { 0 }                        // zero parameters: uses the || token

// Short form with explicit annotations (when no context is available)
transform: fn(integer) -> integer = |x: integer| -> integer { x * 2 }
```

Short-form parameter types are inferred from the expected `fn(T1, T2) -> R` type at the
call site.  If inference is impossible (no context, no annotation), the compiler errors:
*"cannot infer type for lambda parameter 'x'; add an explicit type annotation"*.

Its primary use is with the higher-order functions `map`, `filter`, and `reduce`, as well as the `par(...)` for-loop clause:

```loft
// Named fn-ref
fn double(x: integer) -> integer { x * 2 }
fn is_pos(x: integer) -> boolean { x > 0 }
fn add(a: integer, b: integer) -> integer { a + b }

doubled  = map(nums, fn double);        // [2, 4, 6, ...]
positive = filter(nums, fn is_pos);     // only positive elements
total    = reduce(nums, 0, fn add);     // sum

// Equivalent using lambdas (short form, types inferred)
doubled  = map(nums, |x| { x * 2 });
positive = filter(nums, |x| { x > 0 });
total    = reduce(nums, 0, |a, b| { a + b });

for a in items par(b=double(a), 4) { results += [b] }
```

**`map(v, fn f) -> vector<U>`** — applies `f` to every element; returns a new vector of the return type of `f`.

**`filter(v, fn pred) -> vector<T>`** — returns a new vector with only elements for which `pred` returns `true`.

**`reduce(v, init, fn f) -> U`** — left-folds: starts from `init`, applies `f(acc, elm)` for each element in order.

### Closures

A lambda that references variables from the enclosing scope is a **closure**.
The captured values are **copied into the closure record at definition time**
(value semantics, like Rust `move` closures).

```loft
greeting = "Hello"
greet = fn(name: text) -> text { "{greeting}, {name}!" }
greeting = "Bye"       // does NOT affect the closure
greet("world")         // "Hello, world!" — captured at definition time
```

**Cross-scope closures** — a function can return a closure to its caller.
The captured values travel with the lambda:

```loft
fn make_adder(n: integer) -> fn(integer) -> integer {
    fn(x: integer) -> integer { n + x }
}
add5 = make_adder(5)
add5(10)               // 15
```

**Capture rules:**
- Integers, floats, booleans: copied by value.
- Text: deep-copied (independent of original after capture).
- Struct references: the DbRef is copied (both point to the same store record while both are alive).
- Mutation inside the closure does not affect the outer variable (and vice versa).

**Limitations:**
- Capturing closures in `vector<fn(...)>` is supported only for non-capturing lambdas or when all elements are the same closure type.
- `spacial<T>` collections cannot store closures.

See [THREADING.md](THREADING.md) § fn Expression for how function references are used with `par(...)`.

---

## String literals

Loft has two string literal syntaxes. Both support `{expr}` interpolation.

### Double-quoted strings (`"..."`)

Single-line. Supports `\n`, `\t`, `\\`, `\"` escapes.

```
"hello {name}"           // interpolation
"line1\nline2"           // escape sequences
"literal {{braces}}"     // escape { } by doubling
```

### Backtick strings (`` `...` ``)

**Multi-line.** Bare `"` is literal inside backtick strings (no escaping needed).
Auto-strips common leading indentation based on the closing backtick's column.
First and last lines are trimmed if they contain only whitespace.

```
shader = `
  #version 330 core
  layout (location = 0) in vec3 aPos;
  void main() {
      gl_Position = vec4(aPos, 1.0);
  }
`;

msg = `Hello, {name}!
  You have {count} messages.`;
```

Use backtick strings for GLSL shaders, multi-line templates, or text containing `"`.

## String formatting

Both `"..."` and `` `...` `` strings support format specifiers using `{...}`:

```
"Value: {x}"             // embed variable
"Hex: {n:#x}"            // hexadecimal with 0x prefix
"Oct: {n:o}"             // octal
"Bin: {n:b}"             // binary
"Padded: {n:+4}"         // width 4, always show sign
"Zero-padded: {n:03}"    // width 3, zero-padded
"Float: {f:4.2}"         // width 4, 2 decimal places
"Left: {s:<5}"           // left-aligned width 5
"Right: {s:>5}"          // right-aligned
"Center: {s:^7}"         // center-aligned
"{x:j}"                  // JSON output
"{x:#}"                  // pretty-printed multi-line output
```

Escape `{` and `}` as `{{` and `}}`.

For-expressions can be used inside strings to produce formatted lists:
```
"values: {for x in 1..7 {x*2}:02}"   // produces [02,04,06,08,10,12]
```

---

## Control flow

### If / else if / else

```
if condition {
    // ...
} else if other {
    // ...
} else {
    // ...
}
```

`if` can be used as an expression when both branches produce a value:
```
result = if x > 0 { x } else { -x }
```

### For loops

```
for item in collection {
    // item is each element
}
```

Ranges:
```
for i in 1..10 { }            // 1 to 9 (exclusive end)
for i in 1..=10 { }           // 1 to 10 (inclusive end)
for i in 0..2147483647 { }    // near-unbounded (break as needed)
```

Text iteration yields characters:
```
for c in some_text { }    // c: character
```

Filtered iteration:
```
for item in collection if item.active { }
```

Reverse iteration:
```
for i in rev(1..10) { }        // integer range in reverse (9, 8, 7, …, 1)
for x in rev(sorted_col) { }   // sorted / index collection in reverse key order
```

Inside a loop, the iteration variable supports several attributes using `#`:

| Attribute    | Meaning                                                                              |
|--------------|--------------------------------------------------------------------------------------|
| `v#index`    | For **text** loops: byte offset of the **start** of the current character.           |
|              | For **vector** and **sorted** loops: 0-based position of the current element.        |
|              | Not supported on **index** loops (compile error — use `#count` instead).             |
| `v#next`     | For **text** loops only: byte offset immediately **after** the current character.    |
| `v#count`    | Number of iterations completed so far (works on all collection types).               |
| `v#first`    | `true` for the first element only (works on all collection types).                   |
| `v#remove`   | Remove the current element (filtered loops only; see below).                         |

**Collection type support matrix:**

| Attribute | `vector` | `sorted` | `index` | `hash` |
|-----------|----------|----------|---------|--------|
| `#first`  | ✓        | ✓        | ✓       | N/A — cannot iterate directly |
| `#count`  | ✓        | ✓        | ✓       | N/A |
| `#index`  | ✓ (0-based) | ✓ (0-based array position) | ✗ compile error | N/A |
| `#remove` | ✓ (filtered) | ✓ (filtered) | ✓ (filtered) | use `h[key] = null` |

**Gotcha — `#index` does not mean the same thing on text and vector.** On a text
loop `c#index` is a **byte offset** into the underlying UTF-8 (so it advances by
2–4 per non-ASCII character); on a vector or sorted loop `v#index` is a 0-based
**element position**.  Code that relies on `#index` being a counter — say
`if c#index == 5 { … }` — works on ASCII, then quietly stops working when an
emoji or accented letter is added.  When you want a 0-based character count
that matches vector semantics, use `c#count`; when you want byte offsets for
slicing (e.g. `txt[c#index..c#next]`), use `c#index`.

Text iteration example — `#index` and `#next` are always consistent: `c#next == c#index + len(c)`:
```
// "Hi 😊!": H@0..1, i@1..2, ' '@2..3, '😊'@3..7, '!'@7..8
for c in "Hi 😊!" {
    // c#index = start byte of current character
    // c#next  = first byte of the next character
}
```

`v#remove` is only valid inside `for ... if ...` loops:
```
for v in x if v % 3 != 0 {
    v#remove;
}
```

**Mutation guard:** Appending to a collection while iterating over it is a compile error:

```
for e in v { v += [4]; }  // ERROR: Cannot add elements to 'v' while it is being iterated
```

This protects against infinite loops (vectors re-read their length each step) and data
corruption (sorted/index insertions invalidate stored iterator positions).

Exceptions:
- `e#remove` in a filtered loop is safe and allowed — it adjusts the iterator position after removal.
- Field accesses are not blocked: `db.items += x` is allowed even if `db.items` is iterated via a local variable.

### Break and continue

```
break
continue
```

Only valid inside a loop.

### Return

```
return value
return           // for void functions
```

The last expression in a block (without a trailing `;`) is automatically returned.

`?? return` (C56): if the left side of `??` is null, return from the function
immediately with the right-hand value:
```
id = param(req, "id") ?? return bad_request("missing id");
val = lookup(id)       ?? return;    // void function: return nothing
```

### Custom iterators (I13)

Any type with a `fn next(self: T) -> Item?` method can be used in a `for` loop.
Returning `null` from `next` terminates the loop:

```
struct Counter { current: integer, limit: integer }
fn next(self: Counter) -> integer {
    val = self.current;
    self.current = val + 1;
    if val >= self.limit { return null; }
    val
}

c = new_counter(5);
for x in c { }    // iterates 0, 1, 2, 3, 4
```

`#count` and `#first` work; `#index` and `#remove` are not available.

### Parallel blocks (A15)

`parallel { }` runs each top-level expression concurrently (currently sequential):

```
parallel {
    task_a();
    task_b();
}
// continues after both arms complete
```

No trailing `;` is required after the closing `}`.

### Match expressions

Pattern matching dispatches on enum variants, scalar values, or struct types:

```
result = match direction {
    North | South => "vertical",
    East | West => "horizontal"
}
```

**Enum match:** each arm names a variant. All variants must be covered or a `_` wildcard
must be present. Or-patterns (`|`) combine variants into a single arm. Struct-enum arms
can destructure fields:

```
match shape {
    Circle { radius } if radius > 0.0 => PI * radius * radius,
    Circle { radius }                 => 0.0,
    Rect { width, height }            => width * height
}
```

**Scalar match:** the subject is an integer, text, float, boolean, or character. Arms
are literal values, ranges, `null`, or `_`:

```
match score {
    null     => "absent",
    90..=100 => "A",
    80..90   => "B",
    1 | 2 | 3 => "low",
    _        => "other"
}
```

**Guard clauses:** any arm may have an `if` guard after the pattern. The guard is
evaluated when the pattern matches; if the guard is false, matching falls through to
the next arm. Guarded arms do **not** count toward exhaustiveness — because the guard
can fail at runtime, the compiler cannot guarantee the arm will handle that variant.
Even if every variant has a guarded arm, a wildcard `_ =>` or an unguarded arm covering
each variant is still required:
```
match color {
    Red if is_bright   => "bright red",
    Green if is_bright => "bright green",
    Blue               => "blue",
    _                  => "other"       // required — Red and Green guards may fail
}
```

**Match is an expression:** it produces a value that can be assigned or returned. All
arms must produce the same type (or void).

---

## Variables

Variables are declared implicitly on first assignment. Their type is inferred:
```
x = 42
name = "hello"
items = [1, 2, 3]
```

Variables may be explicitly initialized from expressions:
```
data = configuration as Program
```

---

## Vectors

```
v = [1, 2, 3]               // create with literal
v: vector<integer> = []     // empty vector with type annotation
buf: vector<single> = []    // empty vector of f32
v += [4]                    // append one element
v += [5, 6]                 // append multiple elements
for x in v { }             // iterate
v[i]                        // index (null if out of bounds)
v[2..-1]                    // slice (negative indices count from end)
v[start..end]               // slice range (end exclusive)
v[start..]                  // open-ended slice to end
v[..end]                    // open-start slice from 0 to end (exclusive)
[elem; 16]                  // repeat initializer: 16 copies of elem
[for n in 1..7 { n * 2 }]  // vector comprehension (builds [2, 4, 6, 8, 10, 12])
[for n in 1..10 if n % 2 == 0 { n }]  // comprehension with filter
```

**Empty vectors** require a type annotation so the compiler knows the element type.
Use `v: vector<T> = []` instead of the older `[for _ in 0..0 { default }]` pattern.

To remove elements while iterating, use `v#remove` inside a filtered loop (see [For loops](#for-loops)).

---

## Key-based collections (hash / index / sorted)

All three keyed collection types support single-element removal by assigning `null` to a subscript:

```loft
h[key] = null          // hash: remove element whose key field equals key
idx[nr, name] = null   // index: remove element by compound key
s[key] = null          // sorted: remove element by key field
```

Removing a key that is not present is a **no-op** (safe, no error).

`sorted` and `index` collections support forward and reverse iteration:
```loft
for v in sorted_col { }         // forward — visits elements in key order
for v in rev(sorted_col) { }    // reverse — visits elements in reverse key order
```

Lookup also returns `null` when an element is absent:
```loft
if h[key] { /* found */ }
elem = idx[42, "foo"]    // null if not present
```

---

## Structs and record initialization

Named form (recommended; type is explicit):
```
point = Point { x: 1.0, y: 2.0 }
```

Anonymous form (type is inferred from context):
```
point = { x: 1.0, y: 2.0 }
```

Fields not specified get their `= expr` default, or the zero value for their type.
Nullable fields default to `null`.

Field access uses `.`:
```
point.x
arg.long.len()
```

### Shared field names

Field names are type-scoped, not globally unique.  Different structs and enum
variants can share a field name — the compiler resolves the correct field by
the type of the receiver:

```loft
struct Point { x: float, y: float }
struct Rect { x: float, y: float, w: float, h: float }

p = Point { x: 1.0, y: 2.0 };
r = Rect { x: 10.0, y: 20.0, w: 30.0, h: 40.0 };
p.x;   // 1.0 — Point's x
r.x;   // 10.0 — Rect's x (different offset, same name)
```

This also works between struct-enum variants:
```loft
enum Shape {
  Circle { radius: float, label: text },
  Square { side: float, label: text }
}
c = Circle { radius: 5.0, label: "big" };
s = Square { side: 3.0, label: "small" };
c.label;  // "big"
s.label;  // "small"
```

Verified: works in vectors (`pts[0].x`), function parameters, and across
struct/enum boundaries.  See `tests/scripts/23-field-overlap-structs.loft`
and `tests/scripts/24-field-overlap-enum-struct.loft`.

---

## Methods and function calls

Functions whose first parameter is named `self` can be called with dot syntax:
```
text.starts_with("prefix")
text.to_uppercase()
```

Otherwise they are called as free functions:
```
len(collection)
round(PI * 1000.0)
```

### The `both` parameter name

When the first parameter is named `both` instead of `self`, the function is
registered as **both** a method and a free function:

```loft
pub fn exists(both: File) -> boolean {
  both.format != Format.NotExists
}

// Can be called as:
f.exists()      // method syntax
exists(f)       // free function syntax
```

Use `both` when a function should be equally natural as either form.
`self` registers as a method only; a plain parameter name registers as a
free function only.

### Named arguments

Any parameter can be passed by name using `name: value` syntax.  Positional arguments
come first; once a named argument appears, all subsequent must be named.  Parameters
not provided must have a default value.

```
fn connect(host: text, port: integer = 8080, tls: boolean = true) -> text
connect("example.com")                         // all defaults
connect("example.com", tls: false)             // skip port
connect(host: "example.com", port: 443)        // all named
```

---

## Assertions

```
assert(condition)
assert(condition, "message")
```

Panics at runtime if the condition is false.

---

## Sizeof

```
sizeof(integer)    // 4
sizeof(u8)         // 1 (packed field size)
sizeof(u16)        // 2
sizeof(MyStruct)   // sum of packed field sizes
sizeof(my_var)     // size of the variable's type
```

`sizeof(TYPE)` returns the packed byte size used when the type is stored as a struct
field or vector element. For range-constrained integer types (`u8`, `u16`, etc.) this
is the packed size (1 or 2 bytes), not the stack slot size. For polymorphic enums and
references, the size is computed at runtime from the actual variant.

---

## Random numbers

Three functions for pseudo-random integer generation. All use a thread-local PCG64 generator.

```loft
rand_seed(seed: long)                      // seed the generator
rand(lo: integer, hi: integer) -> integer  // uniform in [lo, hi]; null if lo > hi
rand_indices(n: integer) -> vector<integer>// shuffled [0..n-1]
```

`rand_seed` makes sequences reproducible:

```loft
rand_seed(42);
a = rand(1, 100);  // same value every run with seed 42
```

`rand_indices` is the idiomatic way to randomly visit all elements of a collection:

```loft
rand_seed(7);
items = ["a", "b", "c"];
for i in rand_indices(len(items)) { println(items[i]) }
```

---

## Polymorphism / dynamic dispatch

For struct-enum types, multiple functions may share the same name if each handles a
different variant as its `self` parameter. Loft generates a dispatch wrapper automatically:

```
enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}

fn area(self: Circle) -> float { PI * pow(self.radius, 2.0) }
fn area(self: Rect) -> float { self.width * self.height }

c = Circle { radius: 2.0 };
c.area()   // dispatches to the Circle overload
```

If a variant has no implementation, the compiler emits a `Warning` at the variant's
definition site. To silence the warning deliberately, provide an **empty-body stub**:

```
fn area(self: Rect) -> float { }   // explicit skip — no warning emitted
```

A stub with an empty body `{ }` and a `self` parameter is treated as an intentional
no-op: it emits no warnings, is callable at runtime (returns null for its return type),
and suppresses the unused-`self` warning.

Note: ordinary (non-enum) function overloading by argument type is **not** supported —
two functions with the same name and different non-variant parameter types are a compile error.

---

## Generic functions

A single type variable `<T>` lets you write a function body once for any type:

```
fn identity<T>(x: T) -> T { x }
fn pick_second<T>(a: T, b: T) -> T { _x = a; b }
```

**Rules:**
- T must appear in the first parameter (directly or as `vector<T>`, etc.).
- Only one type variable is allowed.
- At the call site, T is inferred from the first argument's concrete type.
- The compiler creates a specialised copy per concrete type automatically.

**Allowed on T:** assign, return, store in variables.

**Disallowed on T (compile-time errors):**
- Arithmetic: `x + y` → *"generic type T: operator '+' requires a concrete type"*
- Field access: `x.field` → *"generic type T: field access requires a concrete type"*
- Method calls: `x.method()` → *"generic type T: method call requires a concrete type"*
- Match, cast, struct construction on T.

```
identity(42)      // T = integer → returns 42
identity("hi")    // T = text → returns "hi"
```

---

## File structure

A loft file may contain (in any order):
- `use <library>;` imports (must appear at the top)
- `pub` / non-`pub` function definitions
- Struct definitions
- Enum definitions
- Type aliases
- Top-level constants

---

## External function annotations (`#rust`, `#iterator`)

Used only in default/library files to bind loft declarations to Rust implementations:

```
pub fn len(self: text) -> integer;
#rust "@self.len() as i32"

pub fn env_variables() -> iterator<EnvVar, integer>;
#iterator "stores.env_iter()" "stores.env_next(@0)"
```

---

## Operator definitions (internal)

Operators are defined as functions named `OpXxx` in default files and linked to
infix/prefix syntax by the parser. Examples: `OpAdd`, `OpEq`, `OpNot`, `OpConv`, `OpCast`.

---

## Shebang

Loft scripts support a Unix shebang line for direct execution:
```
#!/path/to/loft-interpreter
fn main() { ... }
```

---

## Summary of grammar (informal)

`use` declarations must appear before any other top-level declarations in a loft file.

```
file         ::= { use_decl } { top_level_decl }
use_decl     ::= 'use' identifier ';'
top_level    ::= [ 'pub' ] ( fn_decl | struct_decl | enum_decl | type_decl | constant )
fn_decl      ::= 'fn' ident '(' args ')' [ '->' type ] ( ';' | block )
struct_decl  ::= 'struct' CamelIdent '{' field { ',' field } [ ',' ] '}'
enum_decl    ::= 'enum' CamelIdent '{' variant { ',' variant } '}'
variant      ::= CamelIdent [ '{' field { ',' field } '}' ]
field        ::= ident ':' type { field_mod }
field_mod    ::= 'limit' '(' expr ',' expr ')'
               | 'not' 'null'
               | 'default' '(' expr ')' | '=' expr
               | 'virtual' '(' expr ')'
type_decl    ::= 'type' CamelIdent '=' type ';'
constant     ::= UPPER_IDENT '=' expr ';'
block        ::= '{' { stmt } '}'
stmt         ::= expr [ ';' ]
expr         ::= for_expr | match_expr | 'continue' | 'break' | 'return' [ expr ]
               | assignment
match_expr   ::= 'match' expr '{' match_arm { ',' match_arm } '}'
match_arm    ::= pattern { '|' pattern } [ 'if' expr ] '=>' expr
pattern      ::= '_' | 'null' | literal | range | CamelIdent [ '{' field_bind '}' ]
assignment   ::= operators [ ( '=' | '+=' | '-=' | '*=' | '/=' | '%=' ) operators ]
operators    ::= single { '.' ident [ '(' args ')' ] | '[' index ']' | '#' ident }
               { op operators }
single       ::= '!' single | '-' single | '(' expr ')' | block | '[' vector_lit ']'
               | 'if' expr block [ 'else' ( single | block ) ]
               | 'for' ident 'in' range_expr [ 'if' expr ] block
               | CamelIdent [ '{' field_init { ',' field_init } '}' ]
               | ident | integer | long | float | single | string | character
               | 'true' | 'false' | 'null'
range_expr   ::= expr '..' [ '=' ] expr   // exclusive or inclusive end
               | expr '..'                 // open-ended
               | 'rev' '(' range_expr ')' // reverse
```

---

## Best Practices

### String comparisons containing `{` or `}`

All string literals in loft are format strings — any `{...}` is interpreted as a
format expression. When comparing formatted output against a string that contains
literal braces, escape both sides with `{{` and `}}`:

```loft
// WRONG — {r:128,g:0,b:64} tries to look up variable r with format spec 128,...
assert("{p}" == "{r:128,g:0,b:64}", "...");

// CORRECT — double braces produce literal { and }
assert("{p}" == "{{r:128,g:0,b:64}}", "...");
```

Similarly for JSON format output:
```loft
assert("{o:j}" == "{{\"key\":1}}", "json format");
```

### ~~Unique field names across all structs in one file~~ (resolved)

Field lookups are type-scoped: `determine_keys()` and `position()` receive the
struct type number and search only within that struct's field list. Two structs
in the same file **may** share a field name at different byte offsets without
causing errors. Verified by `tests/scripts/23-field-overlap-structs.loft` and
`tests/scripts/24-field-overlap-enum-struct.loft`.

### Ref-param vector append

`v += items` inside a `&vector<T>` function parameter propagates back to the
caller. Both bracket-form literals and vector expressions work:

```loft
fn fill(v: &vector<Item>, extra: vector<Item>) {
    v += extra;          // appended elements are visible to the caller
}

fn add_one(v: &vector<Item>, x: Item) {
    v += [x];            // bracket-form also works
}
```

Field-level mutations via a ref-param also work as expected:

```loft
fn ok_mutate(v: &vector<Item>, idx: integer, val: integer) {
    v[idx].value = val;  // field mutation via ref-param is visible
}
```

Without `&`, element mutations on existing elements are also visible (the DbRef is shared),
but appending via `v += [x]` is local to the callee — the caller's vector length does not
change. Use `&vector<T>` whenever the function needs to grow the vector.

### Polymorphic text methods on struct-enum variants

Text-returning methods on struct-enum variants that use format strings work
correctly:

```loft
enum Shape {
    Circle { radius: float },
    Rect   { width: float, height: float }
}
fn describe(self: Circle) -> text { "r={self.radius}" }
fn describe(self: Rect)   -> text { "{self.width}x{self.height}" }
```

If a variant does not implement a method, declare an empty stub with `self` as the
first parameter to suppress the warning and return null:

```loft
fn describe(self: Circle) -> text { }   // stub: returns null, no warning
```

---

## Interfaces and bounded generics

Interfaces declare a set of required methods.  A type satisfies an interface
by defining the required methods — no `impl` declaration is needed (structural
satisfaction, like Go interfaces):

```loft
interface Comparable {
  fn less_than(self: Self, other: Self) -> boolean
}

struct Priority { value: integer }
fn less_than(self: Priority, other: Priority) -> boolean {
  self.value < other.value
}
// Priority now satisfies Comparable — no explicit declaration.
```

Bounded generics use `<T: InterfaceName>` to constrain the type variable:

```loft
fn find_min<T: Comparable>(v: vector<T>) -> T {
  result = v[0];
  for item in v {
    if item.less_than(result) { result = item; }
  }
  result
}
```

Operator interfaces use `op` syntax:

```loft
interface Summable {
  op + (self: Self, other: Self) -> Self
}
fn total<T: Summable>(a: T, b: T) -> T { a + b }
total(10, 20);  // integer satisfies Summable automatically
```

Multiple bounds: `<T: Ordered + Printable>`.

**Stdlib interfaces** (defined in `default/01_code.loft`): `Ordered`, `Equatable`,
`Addable`, `Numeric`, `Scalable`, `Printable`.  Built-in types (`integer`, `float`,
`text`) satisfy them automatically via their existing operator definitions.

Bounded generics work with for-loops, method calls, and operator dispatch
on all types including structs.

---

## Design decisions and constraints

A complete list of open issues is in [PROBLEMS.md](PROBLEMS.md).

### Error handling: null + FileResult, no exceptions

Loft uses two mechanisms instead of exceptions:

**Null returns** for simple fallible operations — handled with `??`, `!`, or `if`:

```loft
name = config.get("user") ?? "anonymous";  // fallback
f = file("data.txt");
if !f.exists() { println("not found"); return; }   // guard
clip = audio_load("hit.wav");
if clip { audio_play(clip, 0.5); }         // graceful skip
```

**`FileResult` enum** for filesystem operations that need specific error reasons:

```loft
result = delete("temp.dat");
if result == FileResult.NotFound { println("already gone"); }
if result == FileResult.PermissionDenied { println("access denied"); }
if !result.ok() { println("delete failed"); }
```

`FileResult` variants: `Ok`, `NotFound`, `PermissionDenied`, `IsDirectory`,
`NotDirectory`, `Other`.  Used by `delete`, `move`, `mkdir`, `mkdir_all`,
`set_file_size`.

There are no hidden exception paths — every function's failure mode is visible
at the call site.  `assert` and `panic` are for programmer errors (bugs), not
expected failures.  In production mode (`--production`), failed asserts are
logged instead of aborting.

### Closure capture: copy-at-definition, mutable within copy

Captured variables are copied into the closure at definition time (value semantics,
like Rust `move`).  Mutations after capture are not visible inside the lambda, and
mutations inside the lambda are not visible outside.  However, the closure's own
copy persists across invocations:

```loft
counter = 0;
inc = fn() -> integer { counter += 1; counter };
inc();   // 1
inc();   // 2
inc();   // 3
counter; // still 0 — outer variable unchanged
```

### Variable scoping: shared name table per file

All functions in a `.loft` file share one variable name table.  In practice this
works transparently — the compiler tracks which function each variable belongs to.
Collisions only occur in specific codegen edge cases:

- A function with `const vector<T>` parameters that calls itself recursively AND
  contains a `for` loop may panic with "Too few parameters" (PROBLEMS.md #84).
- Workaround: use function-prefixed loop variable names in library code
  (e.g. `wu_x` for Wu line algorithm, `bz_t` for Bezier).

Regular parameter and local variable reuse across functions works correctly.

### Hash collections: struct fields only, no iteration

Hash collections cannot be standalone local variables — wrap in a struct.
Lookup and mutation work; iteration does not:

```loft
struct Table { data: hash<Entry[name]> }
t = Table { data: [] };
t.data += [Entry { name: "x", value: 1 }];
e = t.data["x"];         // lookup — works
t.data["x"] = null;      // remove — works
for kv in t.data { }     // iteration — NOT supported
```

### Generics: single type variable

Only one type variable `<T>` is allowed, inferred from the first argument.
Multiple type variables (`<T, U>`) are not supported.

**Without bounds:** only assign, return, and store are allowed on `T`.
**With bounds (`<T: Interface>`):** method calls and operators declared
in the interface are allowed on `T`.  See § Interfaces above.

### Text: comprehensive operations

The stdlib provides `starts_with`, `ends_with`, `find`, `contains`, `replace`,
`trim`, `split(char)`, `join(separator)`, `to_uppercase`, `to_lowercase`,
`len`, and slicing.  `split` and `join` are inverses:
`"a,b,c".split(',').join(",") == "a,b,c"`.

## See also
- [STDLIB.md](STDLIB.md) — Standard library API (math, text, collections, file I/O, logging, parallel)
- [COMPILER.md](COMPILER.md) — Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode
