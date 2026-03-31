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
- [Tuples](#tuples)
- [Vectors](#vectors)
- [Key-based collections (hash / index / sorted)](#key-based-collections-hash--index--sorted)
- [Structs and record initialization](#structs-and-record-initialization)
- [Methods and function calls](#methods-and-function-calls)
- [Assertions](#assertions)
- [Sizeof](#sizeof)
- [Polymorphism / dynamic dispatch](#polymorphism--dynamic-dispatch)
- [Coroutines / generators](#coroutines--generators)
- [Generic functions](#generic-functions)
- [Interfaces and bounded generics](#interfaces-and-bounded-generics)
- [File structure](#file-structure)
- [External function annotations (`#rust`, `#iterator`)](#external-function-annotations-rust-iterator)
- [Operator definitions (internal)](#operator-definitions-internal)
- [Shebang](#shebang)
- [Summary of grammar (informal)](#summary-of-grammar-informal)
- [Best Practices](#best-practices)
- [Known Limitations](#known-limitations)

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
| `(T1, T2, ...)`                    | Tuple: stack-allocated compound value, 2+ elements   |
| `vector<T>`                        | Dynamic array of `T`                                  |
| `hash<T[field1, field2]>`          | Hash-indexed collection of `T` on the given fields    |
| `index<T[field1, -field2]>`        | B-tree index (ascending/descending)                   |
| `sorted<T[field]>`                 | Sorted vector on the given fields                     |
| `reference<T>`                     | Reference (pointer) to a stored `T` record            |
| `iterator<T>`                      | Generator / coroutine that yields values of type `T`  |
| `fn(T1, T2) -> R`                  | First-class function type                             |

The key fields are declared **inside** the angle brackets with the element type.
A `-` prefix on a field name means descending order:
```
sorted<Elm[-key]>           // single key, descending
index<Elm[nr, -key]>        // two keys: nr ascending, key descending
hash<Count[c, t]>           // compound hash key
```

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
```

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
| 0 (lowest) | `??`                                   | null-coalescing         |
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

**Note (V1 limitation):** For complex LHS expressions (not a variable or field), the expression
is evaluated twice at runtime — once for the null check and once for the result.  Use a named
temporary if double evaluation causes side effects.

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
| Tuple            | `(1, "hello")`, `(x, y, z)`         |
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

See [THREADING.md](THREADING.md) § fn Expression for how function references are used with `par(...)`.

### Closures (capturing lambdas)

A lambda that references a variable from its enclosing function captures it by value
at the point of lambda creation.  The captured values are stored in a hidden closure
record allocated on the heap.

```loft
fn make_greeter(prefix: text) -> fn(text) -> text {
    fn(name: text) -> text { "{prefix} {name}" }
}

greet = make_greeter("Hello");
println(greet("world"));    // prints "Hello world"
```

The closure captures `prefix` at the time `make_greeter` is called.  Later calls to
the returned lambda see the captured value regardless of how the enclosing scope has
changed.

**Mutable captures** — a captured variable can be mutated inside the lambda; the
mutation is written back to the closure record and visible in subsequent calls to the
same lambda instance.

**Reassignment** — assigning a new lambda to a variable that already holds a capturing
lambda frees the previous closure and creates a fresh one:
```loft
x = 10;
f = fn(y: integer) -> integer { x + y };
f = fn(y: integer) -> integer { x * y };   // old closure freed; new one created
```

**Current limitation (C31):** storing a capturing lambda in a `vector<fn(...)>` or as
a struct field is not yet supported.  Pass closures as function arguments or return
values instead.

---

## String formatting

Strings support inline expressions and format specifiers using `{...}`:

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

**Field iteration** — `for f in s#fields` iterates over a struct's stored primitive
fields at compile time (the loop is unrolled by the parser; no runtime allocation):

```loft
struct Config { host: text, port: integer not null, debug: boolean }
c = Config{ host: "localhost", port: 8080, debug: true };

for f in c#fields {
    match f.value {
        FvText { v } => println("{f.name} = '{v}'"),
        FvInt  { v } => println("{f.name} = {v}"),
        FvBool { v } => println("{f.name} = {v}"),
        _            => {}
    }
}
```

The loop variable has type `Field` with:
- `f.name: text` — the field name (compile-time constant)
- `f.value: FieldValue` — a struct-enum wrapping the typed value
  (`FvBool{v}`, `FvInt{v}`, `FvLong{v}`, `FvFloat{v}`, `FvSingle{v}`, `FvChar{v}`, `FvText{v}`)

Reference, collection, and nested-struct fields are skipped silently.
The source expression must be a plain identifier; for complex expressions assign a
temporary first: `tmp = get_config(); for f in tmp#fields { ... }`.

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

**Nested field sub-patterns** (L2) — a field position in a struct arm can itself carry
a pattern instead of just a binding name.  Supported sub-patterns: enum variant name,
scalar literal, wildcard `_`, or or-pattern (`A | B`):

```
enum Status { Pending, Paid, Refunded }
struct Order { status: Status, amount: integer }

match order {
    Order { status: Paid, amount } => charge(amount),
    _                              => 0,
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

**Tuple match** (T1.9) — a tuple subject can be matched with element-level patterns.
Each element position may be a binding, a literal, or a wildcard:

```
pair = (3, "hello");
match pair {
    (0, _)    => "starts at zero",
    (n, "hi") => "greeting at {n}",
    (n, s)    => "got {n} and {s}",
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

## Tuples

Tuples are stack-allocated compound values containing two or more elements of
potentially different types.  They are created with parenthesised expression lists:

```loft
pair = (42, "hello")              // type: (integer, text)
triple = (1, 2.0, true)           // type: (integer, float, boolean)
```

**Type annotation:**
```loft
fn min_max(v: vector<integer>) -> (integer, integer) {
    // ...
}
```

**Element access** uses `.0`, `.1`, … (zero-based):
```loft
p = (10, 20);
x = p.0           // 10
y = p.1           // 20
p.0 = 99          // element assignment
```

**LHS destructuring** unpacks all elements in one assignment:
```loft
(a, b) = pair(3, 7)
(lo, hi) = min_max(nums)
```

**`not null` elements** — tuple element types accept the `not null` modifier;
assigning a nullable value to such an element is a compile-time error:
```loft
p: (integer not null, integer not null) = (0, 0)
```

**Ref-param tuples** — passing a tuple by `&` reference lets the callee write back
individual elements to the caller.  A `&tuple` parameter that is never written
produces a warning (not an error), consistent with other ref-param checks.

**Current limitations:**
- Tuples may not be stored as struct fields.
- Functions returning a tuple cannot yet be called in tail position from another
  function (T1.8 open item).
- Struct-reference (`DbRef`) elements inside tuples have known lifetime issues
  after destructuring (T1.8c); prefer primitive and text elements.

---

## Vectors

```
v = [1, 2, 3]               // create
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

To remove elements while iterating, use `v#remove` inside a filtered loop (see [For loops](#for-loops)).

### Vector aggregates

Reduction functions over `vector<integer>`:
```loft
sum_of(nums)            // sum of all elements
min_of(nums)            // minimum element (null if empty)
max_of(nums)            // maximum element (null if empty)
```

Predicate aggregates (short-circuit; work with any `vector<T>` and a predicate lambda):
```loft
any(nums, |x| { x > 0 })           // true if at least one element satisfies pred
all(nums, |x| { x > 0 })           // true if every element satisfies pred
count_if(nums, |x| { x > 0 })      // count of elements satisfying pred
```

Generic bounded versions (usable with any type satisfying `Ordered` or `Addable`):
```loft
min_of(scores)          // works if Score satisfies Ordered
max_of(scores)          // works if Score satisfies Ordered
sum(scores, zero)       // works if Score satisfies Addable; zero is the identity value
```

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

## Coroutines / generators

A function declared with return type `iterator<T>` is a **generator**.  Instead of
computing all values and returning a collection, it suspends at each `yield` and
resumes on the next call to `next()`.

```loft
fn count_up(n: integer) -> iterator<integer> {
    for i in 0..n {
        yield i;
    }
}
```

**Consuming a generator with `for`:**
```loft
for x in count_up(5) {
    println(x);           // prints 0 1 2 3 4
}
```

**Consuming manually with `next()` and `exhausted()`:**
```loft
gen = count_up(3);
v = next(gen);
for i in 0..1000000 {
    if exhausted(gen) { break; }
    println(v);
    v = next(gen);
}
```

Or more idiomatically, just use `for x in gen()` — the for-loop handles `next()` and
exhaustion automatically.

`next(gen)` returns the next yielded value, or `null` when the generator is exhausted.
`exhausted(gen)` returns `true` once the generator body has returned.

**`yield from`** delegates to a sub-generator, forwarding each of its values:
```loft
fn flatten(outer: vector<vector<integer>>) -> iterator<integer> {
    for inner in outer {
        yield from each_of(inner);   // each_of is another generator
    }
}
```

**Text and struct parameters** survive across `yield`/resume: the serialisation
layer copies text values and struct-ref cursors into the generator frame.

**Nested helper calls** between yields are supported: the full call stack is saved
and restored on each yield/resume cycle.

**`e#remove` inside a generator for-loop** is a compile error — the iterator
position cannot be adjusted across yield boundaries.

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

**Allowed on T (unconstrained):** assign, return, store in variables.

**Disallowed on T without a bound (compile-time errors):**
- Arithmetic: `x + y` → *"generic type T: operator '+' requires a concrete type"*
- Field access: `x.field` → *"generic type T: field access requires a concrete type"*
- Method calls: `x.method()` → *"generic type T: method call requires a concrete type"*
- Match, cast, struct construction on T.

```
identity(42)      // T = integer → returns 42
identity("hi")    // T = text → returns "hi"
```

To allow operations on `T`, add an interface bound — see the next section.

---

## Interfaces and bounded generics

Interfaces declare a set of operations that a type must support.  A generic function
can then require `<T: InterfaceName>` to use those operations on `T`.

### Declaring an interface

```loft
interface Ordered {
    fn OpLt(self: Self, other: Self) -> boolean
    fn OpGt(self: Self, other: Self) -> boolean
}
```

`Self` inside an interface body refers to the concrete satisfying type.

### Satisfying an interface

A type satisfies an interface **implicitly** — no declaration is needed.  It just
needs the required methods or operators:

```loft
struct Score { value: integer }
fn OpLt(self: Score, other: Score) -> boolean { self.value < other.value }
fn OpGt(self: Score, other: Score) -> boolean { self.value > other.value }

// Score now satisfies Ordered automatically.
```

Built-in types (`integer`, `long`, `float`, `text`, …) satisfy the standard
interfaces automatically.

### Bounded generic functions

```loft
fn max_of<T: Ordered>(v: vector<T>) -> T {
    result = v[0];
    for item in v {
        if result < item { result = item; }
    }
    result
}

best = max_of([Score{value: 3}, Score{value: 7}, Score{value: 1}]);  // Score{value:7}
best_int = max_of([4, 1, 9, 2]);    // 9
```

Multiple bounds use `+`: `<T: Ordered + Printable>`.

### Standard library interfaces

Declared in `default/01_code.loft`:

| Interface   | Required methods                                      | Example types              |
|-------------|-------------------------------------------------------|----------------------------|
| `Ordered`   | `OpLt(Self, Self) -> boolean`, `OpGt(...)`            | `integer`, `long`, `float`, `text` |
| `Equatable` | `OpEq(Self, Self) -> boolean`, `OpNe(...)`            | all primitives             |
| `Addable`   | `OpAdd(Self, Self) -> Self`                           | `integer`, `long`, `float` |
| `Scalable`  | `scale(Self, integer) -> Self`                        | custom types               |
| `Numeric`   | `OpMul`, `OpSub` in addition to `Addable`             | `integer`, `long`, `float` |
| `Printable` | `to_text(Self) -> text`                               | all types with `to_text`   |

Generic stdlib functions using these bounds:
```loft
min_of<T: Ordered>(v: vector<T>) -> T
max_of<T: Ordered>(v: vector<T>) -> T
sum<T: Addable>(v: vector<T>, zero: T) -> T
```

### Diagnostics

When a type is used as a bounded generic but does not satisfy the interface, the
compiler reports which method is missing and what its expected signature is:
```
Score does not satisfy Ordered: missing fn OpLt(Score, Score) -> boolean
```

### Design notes

- **Static dispatch only** — interfaces are generic constraints, not types.
  `x: Ordered` as a variable type is a compile error; there are no vtables.
- **Single bound per type parameter** — consistent with the single `<T>` restriction.
- **Op-sugar** — inside an interface body, `op < (self: Self, other: Self) -> boolean`
  is shorthand for `fn OpLt(self: Self, other: Self) -> boolean`.

---

## File structure

A loft file may contain (in any order):
- `use <library>;` imports (must appear at the top)
- `pub` / non-`pub` function definitions
- Struct definitions
- Enum definitions
- Interface definitions
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
top_level    ::= [ 'pub' ] ( fn_decl | struct_decl | enum_decl | type_decl | constant | iface_decl )
fn_decl      ::= 'fn' ident [ '<' T ':' iface_list '>' ] '(' args ')' [ '->' type ] ( ';' | block )
iface_decl   ::= 'interface' CamelIdent '{' { iface_method } '}'
iface_method ::= 'fn' ident '(' iface_args ')' [ '->' type ]
iface_list   ::= CamelIdent { '+' CamelIdent }
struct_decl  ::= 'struct' CamelIdent '{' field { ',' field } [ ',' ] '}'
enum_decl    ::= 'enum' CamelIdent '{' variant { ',' variant } '}'
variant      ::= CamelIdent [ '{' field { ',' field } '}' ]
field        ::= ident ':' type { field_mod }
field_mod    ::= 'limit' '(' expr ',' expr ')'
               | 'not' 'null'
               | 'default' '(' expr ')' | '=' expr
               | 'init' '(' expr ')'
               | 'computed' '(' expr ')'
               | 'virtual' '(' expr ')'
type_decl    ::= 'type' CamelIdent '=' type ';'
constant     ::= UPPER_IDENT '=' expr ';'
type         ::= primitive_type | CamelIdent | 'fn' '(' types ')' '->' type
               | 'vector' '<' type '>'
               | 'hash' '<' type '[' fields ']' '>'
               | 'iterator' '<' type '>'
               | '(' type ',' type { ',' type } ')'   // tuple type
block        ::= '{' { stmt } '}'
stmt         ::= expr [ ';' ]
expr         ::= for_expr | match_expr | 'yield' expr | 'yield' 'from' expr
               | 'continue' | 'break' | 'return' [ expr ]
               | assignment
match_expr   ::= 'match' expr '{' match_arm { ',' match_arm } '}'
match_arm    ::= pattern { '|' pattern } [ 'if' expr ] '=>' expr
pattern      ::= '_' | 'null' | literal | range
               | CamelIdent [ '{' field_bind { ',' field_bind } '}' ]
               | '(' elem_pat { ',' elem_pat } ')'        // tuple pattern
field_bind   ::= ident [ ':' sub_pattern ]                // L2 sub-pattern
sub_pattern  ::= '_' | literal | CamelIdent { '|' CamelIdent }
elem_pat     ::= '_' | ident | literal
assignment   ::= operators [ ( '=' | '+=' | '-=' | '*=' | '/=' | '%=' ) operators ]
               | '(' ident { ',' ident } ')' '=' operators   // tuple destructuring
operators    ::= single { '.' ident [ '(' args ')' ] | '[' index ']' | '#' ident }
               { op operators }
single       ::= '!' single | '-' single | '(' expr { ',' expr } ')' | block
               | '[' vector_lit ']'
               | 'if' expr block [ 'else' ( single | block ) ]
               | 'for' ident 'in' range_expr [ 'if' expr ] block
               | 'for' ident 'in' ident '#' 'fields' block   // field iteration
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

### Unique field names across all structs in one file

When two structs in the same file share a field name at different positions, the
compiler can confuse them when resolving collection key fields. This causes wrong
results or "Unknown field" errors in sorted/index range iteration.

Use distinct field names per struct, or isolate conflicting structs in separate files:

```loft
// RISKY — both structs have a 'key' field but at different positions
struct SortElm { key: text, value: integer }
struct IdxElm  { nr: integer, key: text, value: integer }

// SAFE — field names are unique
struct SortElm { s_key: text, s_value: integer }
struct IdxElm  { i_nr: integer, i_key: text, i_value: integer }
```

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

## Known Limitations

A complete list with workarounds is in [PROBLEMS.md](PROBLEMS.md) at the repository root.
The most commonly encountered limitations are summarised here.

### Exit codes

`loft` exits with code 0 even when a parse error occurs. To detect failures in
shell scripts, capture output and check for `Error:` or `panicked`:

```sh
out=$(loft myfile.loft 2>&1)
if [ $? -ne 0 ] || echo "$out" | grep -q "^Error:\|panicked"; then
    echo "FAILED: $out"
fi
```

### Tuples: struct-reference elements and function return

Tuple elements of struct-reference type (`DbRef`) have known lifetime issues after
destructuring; avoid them until T1.8 is resolved.  Functions that return a tuple
cannot yet be called in tail position from another tuple-returning function.

### Closures: no collection storage

A capturing lambda (`fn(y: integer) -> integer { x + y }`) cannot be stored in a
`vector<fn(...)>` or as a struct field (C31 — open issue).  Pass closures as
function arguments or return values instead.

### Coroutines: `e#remove` not available

`v#remove` is not available inside a generator for-loop body.  Use a post-processing
step or collect into a vector first.

### Integer null sentinel

Arithmetic that produces exactly `i32::MIN` (-2 147 483 648) is indistinguishable
from `null`.  Use `long` or mark fields `not null` when the full 32-bit range is needed.

---

## See also
- [STDLIB.md](STDLIB.md) — Standard library API (math, text, collections, file I/O, logging, parallel)
- [COMPILER.md](COMPILER.md) — Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode
