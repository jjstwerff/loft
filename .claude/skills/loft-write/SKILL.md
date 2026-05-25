---
name: loft-write
description: Reference for writing correct loft code. Apply whenever writing, editing, or reviewing .loft files. Covers types, syntax, known bugs, workarounds, naming rules, and error→fix table.
user-invocable: false
---

# Loft Language Writing Reference

Always consult this before writing or reviewing `.loft` files.

---

## Naming conventions — enforced by the parser

| Construct | Convention | Examples |
|-----------|-----------|---------|
| Functions, variables | `lower_case` | `my_fn`, `item_count` |
| Types, structs, enums, variants | `CamelCase` | `Point`, `Color`, `Red` |
| Constants (file-scope) | `UPPER_CASE` | `PI`, `MAX_SIZE` |
| Operator definitions | `OpXxx` prefix | `OpAdd`, `OpEqInt` |

The parser **rejects** code that violates these rules.

---

## Primitive types

| Type | Description | Null sentinel |
|------|-------------|--------------|
| `integer` | **64-bit** signed int (the base integer type; i64 at rest) | `i64::MIN` |
| `float` | 64-bit float; literal must contain `.`: `1.0` | `NaN` |
| `single` | 32-bit float; literal suffix `f`: `1.0f` | `NaN` (32-bit) |
| `boolean` | `true` / `false` | `false` (so `!b` is true for both `null` and `false`) |
| `character` | Single Unicode char; literal: `'a'`, `'😊'`; `c as integer` → codepoint | `'\0'` |
| `text` | UTF-8 string (primary string type) | internal null pointer |

**There is no `long` type and no `l` literal suffix** — both were
removed.  `integer` is already 64-bit (i64), so integer literals are
plain (`86400000`, not `86400000l`), and `now()` / `File.size` return a
full-range `integer`.  Writing `long` or `10l` is a parse error
(*"Undefined type long"* / *"Expect token ;"*).

**Narrow / sized integers are aliases of `integer`** (defined in
`default/01_code.loft`), for byte-width-sensitive file / wire formats:
`i8` `size(1)`, `i16`/`u16` `size(2)`, `i32`/`u32` `size(4)`.  Cast with
`x as i32` etc.  Use plain `integer` for ordinary arithmetic — it has
the full 64-bit range.

**Integer sentinel warning:** Any arithmetic that produces exactly
`i64::MIN` becomes `null`. Division by zero also returns `null`. A
function returning `integer` may `return null` (it yields `i64::MIN`,
detectable with `!result`).  Use `not null` fields when a slot must
reject null.

**`text` vs `string`:** The canonical string type is `text`. Using `string` in struct fields causes errors.

---

## Field modifiers (struct fields only)

```loft
struct Point {
    x: float not null,
    y: float not null,
    r: integer limit(0, 255),
    label: text = "default",
    area: float virtual($.x * $.y),
}
```

Modifiers: `not null`, `limit(min, max)`, `default(expr)` / `= expr`, `virtual(expr)`

---

## Variable declarations

Variables declared by assignment — type is inferred from the initialiser.

```loft
x = 5;
s = "hello";
f = 3.14;
```

Explicit type annotations (sometimes required for empty collections):
```loft
v: vector<integer> = [];
n: integer = null;
```

### `const` variables

```loft
const x = 5;       // immutable local — reassignment is a compile error
const t = "hello";  // works for any type
```

---

## Constants

The `const` keyword is accepted at file scope as well, alongside the
bare-name form.  Both produce the same file-scope immutable constant:

```loft
PI       = 3.14159265358979;    // bare-name form
const E  = 2.71828182845905;    // const-keyword form (P246, 2026-05-11)
pub const MAX_SIZE = 256;       // pub + const combine for exported constants
```

Constants must be `UPPER_CASE`.  Prefer the `const` form when you
want the immutability intent visible at the declaration site
(the bare-name form relies on the UPPER_CASE convention as the only
visual signal); both forms behave identically.

**UPPER_CASE locals warn unless declared `const`.**  Inside a
function body, `FOO = 5;` (an assignment with an UPPER_CASE name and
no `const` keyword) emits a warning telling you to add `const` or
rename to `lower_case`.  The convention "UPPER_CASE means immutable"
is enforced by this check at every scope.

---

## Functions

```loft
fn name(param: Type) -> ReturnType { body }
fn name(a: integer, b: integer = 0) -> integer { a + b }  // default param
pub fn exported() { }   // pub = publicly visible
```

Parameter modifiers:
- `const T` — immutable (compile error to assign to it inside function)
- `&T` — mutable reference, mutations propagate to caller
- **`&` that is never mutated is a compile error** — drop it if the param is read-only
- Omit modifier — pass by value/copy

A function body ending in an expression (no `;`) returns that value. Functions without `->` return `void`.

**Nested fn definitions are forbidden.**  `fn` declarations must be at file
scope.  Code like

```loft
fn outer() {
    fn inner() { ... }   // PARSE ERROR
}
```

produces *"'fn' definitions must be at file scope, not inside a function or
block"*.  Move helper fns alongside the caller, not inside it.  Lambdas
(`|x| { ... }` or `fn(x: T) { ... }`) are the only "function-shaped" thing
allowed inside a fn body.

---

## Imports

```loft
use arguments;    // searches lib/, current dir, LOFT_LIB env var
```

**`use` declarations must appear before any other declarations in the file.**

---

## Composite types

| Syntax | Description |
|--------|-------------|
| `vector<T>` | Dynamic array |
| `hash<T[field]>` | Hash-map keyed by `field` on struct `T` |
| `index<T[field, -field2]>` | B-tree index; `-` = descending |
| `sorted<T[field]>` | Sorted collection |
| `reference<T>` | Pointer to a stored `T` record |
| `fn(T1, T2) -> R` | First-class function type |

---

## Structs

```loft
struct Item { name: text, count: integer not null }
item = Item { name: "foo", count: 0 };
item.count += 1;
```

Field names may overlap across structs — lookups are type-scoped.

---

## Tuples

Anonymous, fixed-arity, stack-allocated compound values.  Use them to
return multiple values without naming a struct.  Shipped in 0.8.3
(T1.1–T1.11); see [doc/claude/TUPLES.md](../../../doc/claude/TUPLES.md).

```loft
// Type notation — two or more element types (single-element tuples
// are not allowed; use the bare type instead).
t: (integer, text) = (3, "hi");

// Literal
pair = (1, "hello");

// Element access (zero-based integer literal — NOT a variable index).
a = pair.0;        // integer
b = pair.1;        // text

// Element assignment
pair.0 = 5;
pair.0 += 3;

// Destructuring
(lo, hi) = min_max(values);

// Function return
fn classify(t: (integer, text)) -> text {
    match t {
        (0, _)   => "zero",
        (n, msg) => "{n}: {msg}",
    }
}

// Nested tuples — chain the `.N` accessors.
nested: ((integer, integer), boolean) = ((1, 2), true);
inner_first  = nested.0.0;       // 1
inner_second = nested.0.1;       // 2
flag         = nested.1;          // true
```

**Restrictions:**
- **No single-element tuples** — `(integer)` is just `integer`.
- **No named tuple fields** — use a struct.
- **No tuple iteration / whole-tuple formatting** — access elements one
  by one.
- **`integer` elements default to nullable** — use `integer not null` if
  the slot must reject null.
- **Compound assignment on tuple LHS is rejected** — `(a, b) += (1, 2)`
  is a compile error; rewrite as `a += 1; b += 2;` or rebuild the tuple.

**Known native bug (P207):** comparing a `character` element read from a
tuple against a character literal under `--native` fails to compile
(`t.0 == 'a'` where `t` is `(character, integer)`).  Workaround: cast to
integer (`t.0 as integer == 97`) or destructure first
(`(c, _) = t; c == 'a'`).

---

## Enums

Simple enum (value type, no fields):
```loft
enum Color { Red, Green, Blue }
c = Color.Red;
// ordering follows declaration order: Red < Green < Blue
```

Struct-enum (each variant has fields; polymorphic dispatch via methods):
```loft
enum Shape {
    Circle { radius: float not null },
    Rect   { w: float not null, h: float not null },
}
fn area(self: Circle) -> float { 3.14159 * self.radius * self.radius }
fn area(self: Rect)   -> float { self.w * self.h }

s = Circle { radius: 2.0 };
a = area(s);   // dispatches to correct variant
```

Plain enums cannot have methods — use struct-enum variants for polymorphic dispatch.

Trailing commas in variant field lists are accepted: `Circle { radius: float, }`.

JSON round-trip: `"{shape:j}"` produces `{"Circle":{"radius":3.14}}`; `Shape.parse(json)` reconstructs the correct variant.

---

## Vectors

```loft
empty: vector<integer> = [];
nums  = [for i in 0..10 { i * 2 }];    // comprehension
items = [1, 2, 3];

v += [element];      // append
v += other_vec;      // concatenate
len(v);              // length
v[i];                // index read
```

**Empty vectors** need a type annotation so the compiler knows the element type.

**Slices return iterators, not vectors.** `arr[lo..hi]` cannot be passed where a `vector<T>` is expected — pass the array with index bounds instead.

---

## Hash collections

Hash **must be a struct field** — not a standalone local variable:

```loft
struct Entry  { key: text, value: integer }
struct Table  { data: hash<Entry[key]> }

t = Table { data: [] };
t.data += [Entry { key: "x", value: 1 }];
e = t.data["x"];
if e == null { /* not found */ }
else { e.value += 1; }
t.data["x"] = null;   // remove entry
```

**Hash cannot be iterated directly** — track aggregates separately.

Never use `key` as a field name in a hash-value struct — it conflicts with hash iteration internals.

---

## Interfaces and bounded generics

```loft
interface Comparable {
  fn less_than(self: Self, other: Self) -> boolean
}

fn find_min<T: Comparable>(v: vector<T>) -> T { ... }
```

Structural satisfaction: if the methods exist, the type satisfies the interface.
No `impl` block needed. Built-in types satisfy `Ordered`, `Equatable`, `Addable`,
`Numeric`, `Scalable`, `Printable` automatically.

---

## The `both` parameter name

Name the first parameter `both` instead of `self` to register a function as
both a method and a free function:

```loft
pub fn exists(both: File) -> boolean { both.format != Format.NotExists }
// f.exists()  — method
// exists(f)   — free function
```

---

## Operators

| Precedence | Operators | Notes |
|-----------|-----------|-------|
| 0 (lowest) | `??` | null-coalescing |
| 1 | `\|\|`, `or` | logical OR |
| 2 | `&&`, `and` | logical AND |
| 3 | `==`, `!=`, `<`, `<=`, `>`, `>=`, `is` | comparison, variant check |
| 4–7 | `\|`, `^`, `&`, `<<`, `>>` | bitwise |
| 8 | `+`, `-` | |
| 9 | `*`, `/`, `%` | |
| 10 | `as` | type cast/conversion |

Unary: `!` (logical not / null check), `-` (negation), `~` (bitwise NOT, integer only)
Assignment: `=`, `+=`, `-=`, `*=`, `/=`, `%=`

```loft
name = record.field ?? "default"   // null-coalescing
x as i32                           // cast to a 4-byte sized integer
flags & ~32                        // bitwise NOT — clears bit 5
```

### `is` variant check

```loft
if d is North { ... }              // boolean check
assert(!(shape is Rect));          // negation

// field capture — binds variant fields as locals scoped to the if-body
if shape is Circle { radius } {
  area = PI * radius * radius;
}

// multiple fields + else
if shape is Rect { width, height } {
  area = width * height;
} else {
  area = 0.0;
}
```

---

## String / text literals

### Double-quoted strings (`"..."`)

Single-line. Supports `{expr}` interpolation and `\n`, `\t`, `\\`, `\"` escapes.

```loft
println("hello {name}");
println("hex={n:#x}");            // hex with 0x prefix
println("float={f:4.2}");         // width 4, 2 decimal places
println("json={o:j}");            // JSON format
println("pretty={o:#}");          // pretty-printed multi-line
println("padded={n:>5}");         // right-align width 5
println("zero={n:03}");           // zero-padded width 3
println("{{literal braces}}");    // escape { } by doubling
```

### Backtick strings (`` `...` ``)

**Multi-line.** Supports `{expr}` interpolation. Bare `"` is literal.
Auto-strips common leading indentation (based on closing backtick column).

```loft
SHADER = `
  #version 330 core
  void main() {
      gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
  }
`;
```

Use `println()` for line-oriented output and `print()` for output without a newline.

---

## Control flow

```loft
if cond { } else if cond { } else { }
result = if x > 0 { x } else { -x }    // if as expression

// null check
if !x { }            // x is null (or false for boolean)
val = a ?? b         // null-coalescing

while cond { }
return expr;
break;
break expr;          // break with value (requires non-void function)
```

---

## For loops

```loft
for i in 0..n { }         // exclusive: 0 to n-1
for i in 0..=n { }        // inclusive: 0 to n
for item in collection { }
for c in some_text { }    // character iteration
for item in col if item.active { }   // filtered iteration
for i in rev(0..n) { }    // reverse range
```

### Loop attributes

```loft
for v in collection {
    v#index    // 0-based position (vector/sorted); byte offset (text)
    v#count    // iterations completed so far (all types)
    v#first    // true for first element (all types)
    v#next     // byte offset after current char (text only)
    v#remove   // remove current element (filtered loops only)
}
```

`#index` is NOT supported on `index` collections — use `#count` there.

---

## Match

```loft
match color {
    Red   => println("red"),
    Green | Blue => println("cool"),
    _     => {},
}

match shape {
    Circle { radius } => println("r={radius}"),
    Rect { w, h } if w == h => println("square"),
    _ => {},
}

// Tuple match — destructure into element patterns.
match (3, 7) {
    (0, _)   => "zero",
    (n, m)   => "{n},{m}",
}
```

Match is an expression — all arms must produce the same type (or void).

**The arm separator is `=>`, never `->`.**  `->` is the lambda /
function return-type arrow; using it in a match arm produces a clear
diagnostic ("match arm separator is `=>`, not `->`") — but only because
the parser was hardened against it.  Older drafts of TUPLES.md showed
`->` for arms; that was always wrong.  If a `match` arm in your code
uses `->`, fix it before running anything.

**Scalar-match arms need commas between them**; enum and tuple match
also accept newline-separated arms.  When in doubt, comma-separate —
it's universally accepted.

---

## Higher-order functions

Two lambda syntaxes, each with a clear job:

- **Shorthand `|x|`** — types inferred from the call-site context
  (e.g. the `map`/`filter` signature).  Use inside higher-order
  calls where the types flow in.
- **Explicit `fn(...)`** — full type annotations.  Use when the
  lambda is stored in a local variable, or anywhere types can't
  be inferred.  Omit `->` for void-returning lambdas (`-> void`
  is not valid syntax — there is no `void` type).

```loft
fn double(x: integer) -> integer { x * 2 }

doubled  = map(nums, fn double);          // named function ref
positive = filter(nums, |x| { x > 0 });  // inferred lambda
total    = reduce(nums, 0, |a, b| { a + b });

// Method form on vectors
doubled  = nums.map(|x| { x * 2 });
evens    = nums.filter(|x| { x % 2 == 0 });

// Typed lambda stored in a local: use the explicit fn(...) form.
emit = fn(x: integer, y: integer) { total += x + y; };
emit(1, 2);
```

**Type annotations on `|x|` shorthand are rejected by design
(see `doc/claude/DESIGN_DECISIONS.md § C62`).**  If you need
types, switch to `fn(name: <type>) { ... }` — the shorthand
exists specifically *because* the types are inferred; adding
annotations collapses the distinction between the two forms.

---

## CRITICAL — flat namespace (interpreter limitation)

**All variable names across every function in a file share one global namespace.** This is an interpreter limitation.

Rules to avoid codegen panics:
- Use **unique loop variable names** across all functions (e.g. `fib_i`, `mb_x`) — defensive default that sidesteps every collision class below
- Descriptive parameter names help avoid collisions

**Precise rule for *reusing* a loop-var name (@P344):** reuse is allowed as long
as the type stays consistent — `for i in [1,2,3] {…}` then `for i in [4,5,6] {…}`
works, and the same name is fine in different functions.  It FAILS only when the
element type differs: `for i in [1,2,3]` then `for i in ["a","b"]` →
`loop variable 'i' has type text but was previously used as integer` (one slot +
type per name in the per-function flat table).  When two loops iterate different
types, give them distinct names.  (Loop variables are also inference-only —
`for i: integer in …` does not parse, @P345.)

**Unused loop variable = exit 1.** Use `_` when the value is not needed.

---

## Builtin names — do not shadow

`len`, `ticks`, `round`, `sorted`, `null`, `map`, `filter`, `reduce`, `rev`

---

## Text pitfalls

**`character == text` is a compile error** — use `"{c}" == t` to compare as text.

**Cannot reassign text parameter** — copy to local first: `local = param; local = ...`

**Prefer `h += expr`** over `h = h + expr` for text building.

---

## File I/O patterns

### Text files (UTF-8)

```loft
f = file("path/to/file.txt");
content = f.content();         // full text content (UTF-8)
lines = f.lines();             // vector<text> of lines

out = file("output.txt");
out.write(content);

dir = file("some/directory");
for ef in dir.files() {
  path = ef.path;
}

if f.exists() { }
size_bytes = f.size;            // integer (i64) — works for any file
```

**`f.content()` is UTF-8-only.**  It silently returns `""` on a
binary file.  For non-text data, use the binary idiom below.

### Binary files (structured reads and writes)

Set `f#format` to `LittleEndian` or `BigEndian`, then use `f#read`
for reads and `f += value` for writes.  `#next` seeks to an
absolute byte offset.  All file-handle operations should live
inside a `{ ... }` scope block so the handle flushes/closes at
block exit:

```loft
// --- Read a 12-byte GLB header ---
{
  f = file("model.glb");
  f#format = LittleEndian;
  magic   = f#read as i32;            // 0x46546C67 = 'glTF'
  version = f#read as i32;
  total   = f#read as i32;            // declared file length
  // Seek past the header + JSON data to a later chunk:
  f#next = (20 + json_len) as integer;
  bin_len = f#read as i32;
}

// --- Write a binary chunk-structured file ---
{
  f = file("model.glb");
  f#format = LittleEndian;
  f += (0x46546C67 as i32);   // 4 bytes: i32 magic
  f += (2 as i32);            // 4 bytes: i32 version
  f += (32 as u8);            // 1 byte (ASCII space)
  f += "chunk of text";       // raw UTF-8 bytes
  f += my_float_vector;       // vector<single> → 4 bytes per element
}
```

Notes:
- **Prefer `f#read as <type>` (no parens) for fixed-width reads.**
  The byte count is inferred from the type — `as i32` reads 4
  bytes, `as u8` reads 1, `as u16` reads 2, `as integer` reads 8.
  The legacy `f#read(n) as T` form still works but the `(n)` must
  match the type's storage width exactly or the runtime panics in
  `src/database/io.rs:276`.  The inferred form makes that mismatch
  impossible.  `as text` still needs `f#read(n) as text` because
  text has no fixed width.
- **`s.field = f#read` (no `as T`) infers width from the LHS field's
  declared type** — symmetric with `f += s.field`.  For a struct
  `S { a: i32, b: u8, c: u16 }`, both sides become:
  `f += s.a; f += s.b; f += s.c` to write, `s.a = f#read; s.b = f#read;
  s.c = f#read` to read.  Changing a field's declared type
  (`i32` → `i64`) automatically updates both sites at the next compile —
  no manual cast edits needed.
- **Always cast scalar writes to the intended width.**  Bare
  `f += int_var` writes 8 bytes (loft stores integers as i64).  To
  write 4 bytes use `f += (int_var as i32)`; for 1 / 2 bytes use
  `as u8` / `as u16`.  Strongly-typed struct fields (`u8`, `u16`,
  `integer not null` with range) write at their declared width
  automatically.
- `f += expr` appends `expr` to the file, respecting the `#format`
  endianness.  `text` → raw bytes, `vector<T>` → each element in
  sequence at its declared width.
- `f.size` returns an `integer` (i64); compare with `0`.
- `f#next = offset as integer` seeks.  Reading position advances
  automatically after each `f#read` — don't manually advance it
  between sequential reads.
- **No `f.bytes()` API** — there's no "read all N bytes into a
  vector" helper.  If you need the whole buffer, call `f#read(n)`
  in a loop or read into a typed record via `OpReadFile`.

Example binary reader/writer patterns live in
`lib/graphics/src/glb.loft` (writer) and
`lib/graphics/tests/glb.loft` (reader).

---

## Known error messages → fixes

| Error message | Fix |
|--------------|-----|
| `Too few parameters on n_<fn>` | Flat namespace collision — unique loop variable names; avoid `for` in `const vector<T>` recursive fns |
| `Variable <x> is never read` (exit 1) | Use the variable, or name loop var `_` |
| `Indexing a non vector` | Variable name shadows `sorted` keyword — rename it |
| `Not implemented operation = for type null` | Variable shadows builtin (e.g. `len = 1`) — rename |
| `Cannot iterate a hash directly` | Track aggregate separately |
| `Undefined type string` | Use `text`, not `string` |
| `Allocating a used store` | Field named `key` in hash-value struct — rename the field |
| `<fn> is not found` for `say(...)` | Use `println()` |
| `Unknown record N` on nested field access | Avoid deep chaining on vector elements (P105) |
| `Cannot pass a literal or expression to a '&' parameter` | Assign to a named variable first, then pass it. `v[i]` and `s.field` work directly (P160). |
| `match arm separator is \`=>\`, not \`->\`` | Replace `->` with `=>` in the arm.  (P206 — was a parser hang before the recovery helper landed.) |
| `'fn' definitions must be at file scope, not inside a function or block` | Move the helper fn out of the enclosing fn body.  Lambdas (`|x| { … }` or `fn(x: T) { … }`) are the only function-shaped values allowed inside a fn body. |
| `compound assignment is not supported for tuple destructuring — use (a, b) = expr instead` | Rebuild the tuple: `(a, b) = (a + 1, b + 2)` — or update each element directly. |
| Native E0308 on `t.0 == 'a'` where `t` is `(character, …)` | P207 — known native codegen bug.  Workaround: cast to integer (`t.0 as integer == 97`) or destructure first (`(c, _) = t; c == 'a'`). |

---

## CLI invocation

```bash
loft --path /path/to/repo/ file.loft                        # interpreter
loft --native --path /path/to/repo/ file.loft               # compile + run native
loft --native-wasm out.wasm --path /path/to/repo/ file.loft # compile to wasm
```

**`--path` must end with a trailing slash.**

---

## Pre-flight checklist

- [ ] All loop variables are unique across the entire file
- [ ] No nested `fn` definitions — helpers live at file scope
- [ ] Hash collections are struct fields, not standalone locals
- [ ] No `arr[lo..hi]` passed as `vector<T>` argument
- [ ] `len`, `sorted`, `ticks`, `round`, `map`, `filter`, `reduce` not used as variable names
- [ ] All `use` imports appear before any other declarations
- [ ] No `long` type / no `l` literal suffix — `integer` is i64; literals are plain (`86400000`), `f.size` compares with `0`
- [ ] `--path` ends with `/` in CLI calls
- [ ] String type in struct fields is `text`, not `string`
- [ ] No `character == text` comparisons — use `"{c}" == t`
- [ ] Never reassign a text parameter — copy to local first
- [ ] `v[i]` and `s.field` can be passed directly as `&` parameters
- [ ] Match arm separator is `=>`, never `->` (the parser used to hang on this — P206)
- [ ] No single-element tuples; `(integer)` is just `integer`
- [ ] Tuple element access uses an integer literal (`t.0`, not `t.i`)
- [ ] No compound assignment on a tuple LHS (`(a, b) +=` is rejected)
- [ ] If comparing a tuple-element `character` against a literal under `--native`, cast to integer first (P207 workaround)
