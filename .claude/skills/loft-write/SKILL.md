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
empty = [for dummy in 0..0 { dummy }];  // empty vector<integer>
nums  = [for i in 0..10 { i * 2 }];    // comprehension → vector

v += [element];      // append one element
v += other_vec;      // concatenate
len(v);              // length
v[i];                // index read
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

## String / text formatting

```loft
say("hello {name}");          // basic interpolation
say("hex={n:#x}");            // hex with 0x prefix
say("float={f:4.2}");         // width 4, 2 decimal places
say("json={o:j}");            // JSON format
say("pretty={o:#}");          // pretty-printed multi-line
say("padded={n:>5}");         // right-align width 5
say("zero={n:03}");           // zero-padded width 3
say("{{literal braces}}");    // escape { } by doubling
```

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
- [ ] ~~Struct field names unique~~ (no longer required — field lookups are type-scoped)
- [ ] All `use` imports appear before any other declarations
- [ ] Long literals use `l` suffix where needed (`0l`, `1000l`)
- [ ] `--path` ends with `/` in CLI calls
- [ ] String type in struct fields is `text`, not `string`
