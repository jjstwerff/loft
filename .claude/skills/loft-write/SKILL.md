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
| `integer` | 32-bit signed int | `i32::MIN` (-2 147 483 648) |
| `long` | 64-bit signed int; literal suffix `l`: `10l` | `i64::MIN` |
| `float` | 64-bit float; literal must contain `.`: `1.0` | `NaN` |
| `single` | 32-bit float; literal suffix `f`: `1.0f` | `NaN` (32-bit) |
| `boolean` | `true` / `false` | `false` (so `!b` is true for both `null` and `false`) |
| `character` | Single Unicode char; literal: `'a'`, `'😊'` | `'\0'` |
| `text` | UTF-8 string (primary string type) | internal null pointer |

**Integer sentinel warning:** Any arithmetic that produces exactly `i32::MIN` becomes `null`. Division by zero also returns `null`. Use `long` or `not null` fields for the full 32-bit range.

**`text` vs `string`:** The canonical string type is `text`. `string` may appear in some contexts but `text` is the documented name — use `text` in struct definitions and signatures.

---

## Field modifiers (struct fields only)

```loft
struct Point {
    x: float not null,               // disallows null; enables full numeric range in storage
    y: float not null,
    r: integer limit(0, 255),        // constrain range
    label: text = "default",         // stored default (shorthand for default(...))
    area: float virtual($.x * $.y),  // read-only computed field; $ = record being initialised
}
```

Modifiers: `not null`, `limit(min, max)`, `default(expr)` / `= expr`, `virtual(expr)`

---

## Variable declarations

Variables declared by assignment — type is inferred from the initialiser.

```loft
x = 5;               // integer
s = "hello";         // text
f = 3.14;            // float
```

Explicit type annotations are **also** valid (and sometimes required for clarity):

```loft
v: vector<integer> = [];
n: integer = null;
```

**However:** type annotations on local collection variables with generic parameters sometimes cause parse errors in certain interpreter versions. When in doubt, drop the annotation and let the type be inferred.

---

## Constants

```loft
PI = 3.14159265358979;    // file-scope UPPER_CASE constant
```

Constants must be `UPPER_CASE` and defined at file scope.

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

**Field names may overlap across structs.** Field lookups are type-scoped, so two structs can share a field name even at different byte offsets. Confirmed by `tests/scripts/23-field-overlap-structs.loft` and `24-field-overlap-enum-struct.loft`.

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

s = Shape.Circle { radius: 2.0 };
a = area(s);   // dispatches to correct variant
```

**Plain enums cannot have methods** — use struct-enum variants for polymorphic dispatch.

---

## Vectors

```loft
empty: vector<integer> = [];            // empty typed vector (preferred)
nums  = [for i in 0..10 { i * 2 }];    // comprehension → vector
items = [1, 2, 3];                       // literal

v += [element];      // append one element
v += other_vec;      // concatenate
len(v);              // length
v[i];                // index read
```

**Empty vectors** need a type annotation so the compiler knows the element type:
```loft
buf: vector<single> = [];     // empty vector of f32
names: vector<text> = [];     // empty vector of strings
```

**Slices return iterators, not vectors.** `arr[lo..hi]` cannot be passed where a `vector<T>` is expected. Use index bounds instead:

```loft
// WRONG
fn process(sub: const vector<integer>) { ... }
process(arr[lo..hi]);   // type error — slice is an iterator

// CORRECT — pass full array with bounds
fn process(arr: const vector<integer>, lo: integer, hi: integer) { ... }
```

---

## Hash collections

Hash **must be a struct field** — not a standalone local variable (causes errors):

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

**Hash cannot be iterated directly** (`for kv in hash` is not supported in the interpreter). Track aggregates separately in a normal variable during the loop.

---

## Interfaces and bounded generics

```loft
interface Comparable {
  fn less_than(self: Self, other: Self) -> boolean
}

// Bounded generic — T must satisfy Comparable
fn find_min<T: Comparable>(v: vector<T>) -> T { ... }

// Operator interfaces use 'op' syntax
interface Summable {
  op + (self: Self, other: Self) -> Self
}
```

Structural satisfaction: if the methods exist, the type satisfies the interface.
No `impl` block needed. Built-in types satisfy `Ordered`, `Equatable`, `Addable`,
`Numeric`, `Scalable`, `Printable` automatically.

Bounded generics work with for-loops, method calls, and operator dispatch on all types.

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
| 0 (lowest) | `??` | null-coalescing: `a ?? b` returns `a` if not null, else `b` |
| 1 | `\|\|`, `or` | logical OR |
| 2 | `&&`, `and` | logical AND |
| 3 | `==`, `!=`, `<`, `<=`, `>`, `>=` | comparison |
| 4–7 | `\|`, `^`, `&`, `<<`, `>>` | bitwise |
| 8 | `+`, `-` | |
| 9 | `*`, `/`, `%` | |
| 10 | `as` | type cast/conversion |

Unary: `!` (logical not / null check), `-` (negation)
Assignment: `=`, `+=`, `-=`, `*=`, `/=`, `%=`

```loft
name = record.field ?? "default"   // null-coalescing
x as long                          // cast integer to long
x as float                         // cast integer to float
"json-text" as Program             // deserialize text as struct
```

---

## String / text literals

Loft has **two** string literal syntaxes:

### Double-quoted strings (`"..."`)

Single-line only. Supports `{expr}` interpolation and `\n`, `\t`, `\\`, `\"` escapes.

```loft
println("hello {name}");          // basic interpolation
println("hex={n:#x}");            // hex with 0x prefix
println("float={f:4.2}");         // width 4, 2 decimal places
println("json={o:j}");            // JSON format
println("pretty={o:#}");          // pretty-printed multi-line
println("padded={n:>5}");         // right-align width 5
println("zero={n:03}");           // zero-padded width 3
println("{{literal braces}}");    // escape { } by doubling
```

### Backtick strings (`` `...` ``)

**Multi-line.** Supports `{expr}` interpolation. Bare `"` is literal (no escaping needed).
Auto-strips common leading indentation (based on closing backtick column).
First and last lines are trimmed if whitespace-only.

```loft
SHADER = `
  #version 330 core
  void main() {
      gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
  }
`;

greeting = `Hello, {name}!
  You have {count} messages.`;
```

**Use backtick strings for:**
- GLSL shader source code
- Multi-line templates (HTML, JSON, SQL)
- Any text containing `"` characters
- Heredoc-style blocks

**Use `println()`** for line-oriented output and `print()` for output without a newline. The function `say()` does not exist in the stdlib — it appears in some older documentation examples but will produce a "not found" error at runtime.

---

## Control flow

```loft
if cond { } else if cond { } else { }

// if as expression
result = if x > 0 { x } else { -x }

// null check
if !x { }            // x is null (or false for boolean)
if x != null { }
val = a ?? b         // null-coalescing

return expr;
break;
```

---

## For loops

```loft
for i in 0..n { }         // exclusive: 0 to n-1
for i in 0..=n { }        // inclusive: 0 to n
for item in collection { }
for c in some_text { }    // character iteration; c is character
for item in col if item.active { }   // filtered iteration

for i in rev(0..n) { }    // reverse range
for x in rev(sorted_col) { }  // reverse sorted/index collection
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

## CRITICAL — flat namespace (interpreter limitation)

**All variable names across every function in a file share one global namespace**, including parameters, locals, and loop variables. This is an interpreter limitation, not a language design goal.

Rules to avoid codegen panics:
- Use **unique loop variable names** across all functions (e.g. `fib_i`, `mb_x`, `col_i`, `sort_j`)
- Never reuse the same loop variable name in a different function
- Descriptive parameter names help avoid parameter collisions

**Unused loop variable = exit 1.** If a loop variable is declared but never read, loft exits with code 1. Use `_` when the value is not needed. But `_` also participates in the flat namespace — if two functions both use `for _ in ...` and interact, use unique named variables instead.

### `const vector<T>` recursive call bug

When a function has `const vector<T>` parameters, calls itself recursively, **and** the function contains a `for` loop, the codegen can panic: *"Too few parameters on n_xxx"*. Workaround: implement the loop as recursion, or restructure to avoid the `for` loop inside the `const vector<T>` function.

---

## Higher-order functions

```loft
fn double(x: integer) -> integer { x * 2 }
fn is_pos(x: integer) -> boolean { x > 0 }
fn add(a: integer, b: integer) -> integer { a + b }

doubled  = map(nums, fn double);
positive = filter(nums, fn is_pos);
total    = reduce(nums, 0, fn add);

// Lambda short form (types inferred from context)
doubled  = map(nums, |x| { x * 2 });
positive = filter(nums, |x| { x > 0 });
total    = reduce(nums, 0, |a, b| { a + b });

// Lambda long form (explicit types)
f: fn(integer) -> integer = fn(x: integer) -> integer { x * 2 }
```

---

## Match

```loft
match color {
    Red   => say("red"),
    Green | Blue => say("cool"),
    _     => say("other"),
}

match shape {
    Circle { radius } => say("r={radius}"),
    Rect { w, h } if w == h => say("square"),
    _ => {},
}
```

---

## Builtin names — do not shadow

| Name | What it is |
|------|-----------|
| `len` | `len(collection) -> integer` |
| `ticks` | `ticks() -> long` — microseconds |
| `round` | `round(float) -> long` |
| `sorted` | keyword |
| `null` | null literal |
| `map`, `filter`, `reduce` | higher-order stdlib functions |
| `rev` | reverse-iteration modifier |

---

## Text assignment pitfalls

**`t = t[3..]` clears t** — self-referencing text slice assignment clears the variable before reading the slice. Use a fresh variable:
```loft
// WRONG: produces empty string
t = t[3..];

// CORRECT: intermediate variable
s = t[3..];
t = s;
```

**`h = h + expr` works (fixed)** — the parser detects this self-append pattern and skips the clear. But `h += expr` is always safe and preferred.

**Text in function return accumulates** — in a text-returning function, `t = s` may APPEND instead of replace due to the text_return work buffer. Use early `return` instead of reassignment:
```loft
// WRONG: may produce "// hellohello" in text-returning function
fn strip(line: text) -> text {
  t = line;
  if t.starts_with("// ") { s = t[3..]; t = s; }
  t
}

// CORRECT: return directly, don't reassign
fn strip(line: text) -> text {
  if line.starts_with("// ") { return line[3..]; }
  line
}
```

**`character == text` always returns true** — the operator resolver falls through to `OpEqBool`. Workaround: format the character as text first:
```loft
// WRONG: always true
if c == some_text { }

// CORRECT
if "{c}" == some_text { }
```

---

## File I/O patterns

```loft
// Read a file
f = file("path/to/file.txt");
content = f.content();         // full text content
lines = f.lines();             // vector<text> of lines

// Write a file
out = file("output.txt");
out.write(content);            // overwrites entire file

// List directory
dir = file("some/directory");
for ef in dir.files() {
  path = ef.path;              // full path
  // Extract filename from path (no .name field)
  fname = path;
  for c in path {
    if c == '/' { fname = path[c#next..]; }
  }
}

// Check existence
f = file("test.txt");
if f.exists() { }
```

---

## Known error messages → causes → fixes

| Error message | Cause | Fix |
|--------------|-------|-----|
| `Too few parameters on n_<fn>` | Flat namespace: `const vector<T>` param + recursive call + `for` loop in same function | Remove the `for` loop; implement as recursion |
| `Variable <x> is never read` (exit 1) | Declared loop/local variable not used | Use it trivially, or name loop var `_` |
| `Indexing a non vector` | Variable name conflicts with `sorted<>` keyword | Rename variable (e.g. `sorted` → `out`, `result`) |
| `Not implemented operation = for type null` | Variable name shadows builtin (e.g. `len = 1`) | Rename variable |
| `Unknown definition` / panic on `boolean` struct field | Some interpreter versions panic on `boolean` in structs | Use `integer` with 0/1 sentinel, or `not null` |
| `Cannot iterate a hash directly` | `for kv in some_hash` | Track aggregate in a separate variable during the loop |
| `Invalid index key` / `Undefined type string` | Using `string` type name in struct field | Use `text` — that is the canonical string type (see PROBLEMS.md #82) |
| `Allocating a used store` (interpreter) | Struct field used as hash-value type is named `key` — conflicts with hash iteration pseudo-field | Rename the field (e.g. `word`, `name`, `label`) — never use `key` as a field name in a hash-value struct (see PROBLEMS.md #83) |
| `Too few parameters on n_<fn>` (any params, not just const) | Any `for` loop in a function that is called from a recursive function — flat namespace corrupts parameter count for the recursive caller | Replace the `for` loop in the helper with recursion, or inline the loop into the recursive function (see PROBLEMS.md #84) |
| `<fn> is not found` for `say(...)` | `say()` does not exist in stdlib | Use `println()` |
| Parse error on local type annotation | Complex generic annotation on local var | Drop the annotation; let type be inferred |
| `Unknown record N` on nested field access | `vec[i].struct_field.nested` — deep chained access on vector elements | Avoid deep chaining on vector elements (P105) |
| `fl_validate: positive header` | Complex nested struct assignment (3+ levels) | Simplify nested struct operations (P106) |
| `Variable 'x' cannot change type from text to null` | `x: text = null;` — typed null init rejected | Use `x = "";` or avoid initializing to null |
| ~~Empty result from `t = t[3..]`~~ | ~~Text self-slice clears before read~~ | **Fixed** — work text for self-referencing assignments |
| ~~Doubled text in function return~~ | ~~Text-returning fn accumulates~~ | **Fixed** — always clear RefVar(Text) before append |
| `character == text` always true | Now a compile error | Use `"{c}" == t` to compare as text |
| `Cannot reassign text parameter` | Text params are 12-byte Str, not 24-byte String | Copy to local first: `local = param; local = ...` |
| `Cannot pass a literal or expression to a '&' parameter` | Passing `[]`, struct literal, or expression to `&vector<T>` / `&StructType` | Assign to a named variable first, then pass the variable |

---

## CLI invocation

```bash
loft --path /path/to/repo/ file.loft                        # interpreter
loft --native --path /path/to/repo/ file.loft               # compile + run native
loft --native-wasm out.wasm --path /path/to/repo/ file.loft # compile to wasm
```

**`--path` must end with a trailing slash** — loft concatenates `"default"` directly onto the string.

---

## Pre-flight checklist before finishing a .loft file

- [ ] All loop variables are unique across the entire file
- [ ] No local variables have complex generic type annotations (drop them if unsure)
- [ ] Hash collections are struct fields, not standalone locals
- [ ] No `arr[lo..hi]` passed as `vector<T>` argument — use index bounds
- [ ] `len`, `sorted`, `ticks`, `round`, `map`, `filter`, `reduce` not used as variable names
- [ ] All `use` imports appear before any other declarations
- [ ] Long literals use `l` suffix where needed (`0l`, `1000l`)
- [ ] `--path` ends with `/` in CLI calls
- [ ] String type in struct fields is `text`, not `string`
- [ ] ~~No `t = t[N..]` self-slice~~ — **fixed**, works directly now
- [ ] No `character == text` comparisons — use `"{c}" == t` (compile error)
- [ ] ~~Text-returning functions~~ — **fixed**, `t = expr; t` works now
- [ ] Prefer `h += expr` over `h = h + expr` for text building
- [ ] Never reassign or `+=` a text parameter — copy to local first
- [ ] Struct params are passed by reference — mutations visible to caller
