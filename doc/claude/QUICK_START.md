# Quick Start — loft Codebase

A compact orientation for starting analysis. Follow the links to the full doc files for detail.

---

## What this project is

**loft** is a tree-walking interpreter for the **loft** programming language, written in Rust. Loft is a statically typed, expression-oriented language with struct/enum support, a store-based heap, and a standard library loaded from `default/*.loft`.

---

## Running the interpreter

```
cargo run --bin loft -- myprogram.loft
cargo run --bin loft -- --help
cargo run --bin gendoc         # regenerate doc/*.html
```

---

## Execution path (start to finish)

```
src/main.rs              CLI entry; loads default/ then user file
  └─ src/parser/         Two-pass recursive-descent parser → Value IR
       ├─ mod.rs            Parser struct, constructors, core helpers
       ├─ definitions.rs    Enum/struct/typedef/function parsing
       ├─ expressions.rs    Expressions, assignments, format strings
       ├─ collections.rs    Iterators, for-loops, map/filter, parallel-for
       ├─ control.rs        Control flow, parse_call, parse_method
       └─ builtins.rs       Parallel worker helpers
       ├─ src/lexer.rs      Tokeniser
       ├─ src/typedef.rs    Type resolution + field offsets
       ├─ src/variables.rs  Per-function variable table
       └─ src/scopes.rs     Scope/lifetime analysis
  └─ src/compile.rs      Drives IR → flat bytecode; initialises native registry
  └─ src/state/          Executes bytecode
       ├─ mod.rs            State struct, execute, stack primitives
       ├─ text.rs           String/text operations
       ├─ io.rs             File I/O, database record ops
       ├─ codegen.rs        Bytecode generation (generate, gen_* helpers)
       └─ debug.rs          Dump/trace helpers
       └─ src/fill.rs       233 opcode implementations
```

---

## Key data structures

| Type | File | Purpose |
|---|---|---|
| `Value` (enum) | `src/data.rs` | IR tree node |
| `Type` (enum) | `src/data.rs` | Static type of a `Value` |
| `Data` | `src/data.rs` | Table of all named definitions |
| `State` | `src/state/mod.rs` | Bytecode stream + runtime stack |
| `Stores` | `src/database/mod.rs` | All stores + type schema |
| `Store` | `src/store.rs` | Raw word-addressed heap |
| `DbRef` | `src/keys.rs` | Universal pointer: (store_nr, rec, pos) |

---

## Null sentinels

| Type | Null value |
|---|---|
| `integer` | `i32::MIN` |
| `long` | `i64::MIN` |
| `float` | `f64::NAN` |
| `boolean` | n/a (always non-null) |
| references | `store_nr == 0 && rec == 0` |

---

## Important conventions

- User functions are stored as `"n_<name>"` (not bare `"<name>"`). `data.def_nr("foo")` returns `u32::MAX`; use `data.def_nr("n_foo")`.
- Operators have `OpCamelCase` loft names → `op_snake_case` Rust names in `fill.rs`.
- Native stdlib functions in `native.rs` use the naming scheme `n_<func>` (global) or `t_<LEN><Type>_<method>` (method; LEN = number of characters in the type name). Example: `t_4text_starts_with`, `t_9character_is_numeric`.
- `#rust "..."` annotations in `default/*.loft` supply the Rust implementation body for code generation.

---

## Common loft patterns — quick reference

### Variables and types
```loft
x = 5;                              // integer (inferred)
y: float = 3.14;                    // explicit type annotation
z: text = null;                     // nullable by default
```
Naming rules enforced by the parser: `lower_case` for functions/variables, `CamelCase` for types/enums/variants, `UPPER_CASE` for constants.

### Functions
```loft
fn add(a: integer, b: integer) -> integer { a + b }
fn greet(name: text) { say("hello {name}"); }
fn fill(v: &vector<Item>) { v += [Item{x: 1}]; }  // & needed to propagate append
```

### Structs and enums
```loft
struct Point { x: float not null, y: float not null }
p = Point{x: 1.0, y: 2.0};
p.x = 3.0;

enum Color { Red, Green, Blue }                        // plain enum — no methods
enum Shape { Circle{radius: float}, Rect{w: float, h: float} }  // struct-enum
fn area(self: Circle) -> float { PI * self.radius * self.radius }
fn area(self: Rect)   -> float { self.w * self.h }     // polymorphic dispatch
```

### Format strings — ⚠ CRITICAL
```loft
say("hello {name}");               // basic interpolation
say("val={x:.2}");                 // 2 decimal places
say("json={o:j}");                 // JSON format
say("{{literal braces}}");         // escape { } by doubling
// WRONG:  assert("{p}" == "{r:128,g:0,b:64}")  — r/g/b parsed as variables!
// CORRECT: assert("{p}" == "{{r:128,g:0,b:64}}")
```

### Control flow
```loft
if x > 0 { } else if x < 0 { } else { }
for i in 0..10 { }                 // 0..9 exclusive
for i in 0..=10 { }               // 0..10 inclusive
for item in my_vector { }
for c in some_text { }            // character iteration; c#index, c#next available
for kv in my_hash { }             // kv.key, kv.value (field names of the value type)
for rev(i in 0..n) { }            // reverse iteration
```

### Match
```loft
match color {
    Red => say("red"),
    Green | Blue => say("cool"),
    _ => say("other"),
}
match shape {
    Circle{radius} => say("r={radius}"),
    Rect{w, h} if w == h => say("square"),
    _ => {},
}
```

### Collections
```loft
v: vector<integer> = [];
v += [1, 2, 3];
len(v);  v[0];

h: hash<Value[name]>;             // hash keyed by .name field
h["key"] = Value{name: "key", count: 0};
h["key"].count += 1;
h["key"] = null;                  // removes entry

s: sorted<Item[priority]>;        // sorted by priority field
idx: index<Item[id]>;             // unique index by id field
```

### Null checks
```loft
if !x { }                         // x is null (or false for boolean)
if x != null { }                  // explicit null check
x = if !fallback { default_val } else { fallback };
```

### Key gotchas
- **Format strings**: every `{...}` in a string literal is interpolation — escape literal braces as `{{` / `}}`
- **Field uniqueness**: field names must be unique across all structs in a single file; duplicate names at different offsets cause "Unknown field" errors in collections
- **Vector append ref**: `v += [item]` inside a function only propagates to the caller if the param is `&vector<T>`; without `&`, append is local
- **Integer null sentinel**: `i32::MIN` is `null` — arithmetic that produces that exact value becomes null; use `long` or `not null` fields for full 32-bit range
- **`use` ordering**: all `use` declarations must appear before any other top-level declaration in a file
- **Plain enum methods**: plain enums (`enum Color { Red }`) cannot have methods — use struct-enum variants for dispatch

---

## Default library load order

```
default/01_code.loft    — operators, math, text, collections
default/02_images.loft  — Image, Pixel, File, Format types
default/03_text.loft    — text utilities
```

---

## Full documentation index

| File | Topic |
|---|---|
| [QUICK_START.md](QUICK_START.md) | This file — orientation, conventions, debug logging |
| [LOFT.md](LOFT.md) | Loft language reference (syntax, types, operators, control flow) |
| [STDLIB.md](STDLIB.md) | Standard library API (math, text, collections, file I/O, logging, parallel) |
| [COMPILER.md](COMPILER.md) | Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode |
| [INTERMEDIATE.md](INTERMEDIATE.md) | Value/Type enums in detail; 233 bytecode operators; State layout |
| [DATABASE.md](DATABASE.md) | Store allocator, Stores schema, DbRef, vector/tree/hash/radix implementations |
| [INTERNALS.md](INTERNALS.md) | calc.rs, stack.rs, create.rs, native.rs, ops.rs, png_store.rs, parallel.rs, main.rs, logger.rs |
| [THREADING.md](THREADING.md) | Parallel for-loop (`par(...)`), `fn <name>` references, runtime parallel execution |
| [LOGGER.md](LOGGER.md) | Runtime logging framework (log_info/warn/error/fatal, config, rate limiting, production mode) |
| [TESTING.md](TESTING.md) | Test framework, `LogConfig` debug-logging presets, `LOFT_LOG` env var, suite files |
| [DOC.md](DOC.md) | HTML documentation generation (gendoc.rs + documentation.rs) |
| [DESIGN.md](DESIGN.md) | Algorithm catalog with complexity analysis and enhancement priorities |
| [CODE.md](CODE.md) | Code quality rules (naming, functions, doc comments, clippy) |
| [DEVELOPMENT.md](DEVELOPMENT.md) | Development workflow — branching, WIP commit, rebase sequence, CI |
| [ASSIGNMENT.md](ASSIGNMENT.md) | Stack slot assignment algorithm — status and design decisions |
| [PROBLEMS.md](PROBLEMS.md) | Known bugs, limitations, workarounds, and fix plans |
| [FORMATTER.md](FORMATTER.md) | Source formatter design and implementation notes |
| [INCONSISTENCIES.md](INCONSISTENCIES.md) | Known language design inconsistencies and asymmetries |
| [OPTIMISATIONS.md](OPTIMISATIONS.md) | Planned and implemented runtime/compiler optimisations |
| [PLANNING.md](PLANNING.md) | Priority-ordered enhancement backlog |
| [ROADMAP.md](ROADMAP.md) | Items in implementation order, grouped by milestone (0.9.0 / 1.0.0 / 1.1+) |
| [MATCH.md](MATCH.md) | Match expression design — pattern types, binding, phase breakdown |
| [NATIVE.md](NATIVE.md) | Native code generation (`src/generation.rs`) design and fix plans |
| [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) | External library loading and `loft.toml` package manifest |
| [BYTECODE_CACHE.md](BYTECODE_CACHE.md) | Bytecode cache (`.loftc`) design notes (deferred) |
| [DEBUG.md](DEBUG.md) | Debugging utilities and tools |
| [DEVELOPMENT.md](DEVELOPMENT.md) | Contribution workflow — branch → phases → structured commit sequence |
| [RELEASE.md](RELEASE.md) | Release checklist and version history |
| [WEB_IDE.md](WEB_IDE.md) | Web IDE integration design notes |
| [../PROMPTS.md](../PROMPTS.md) | Working with Claude — practices and when to use each prompt in `prompts.txt` |

## Reading by goal

| Goal | Start here |
|---|---|
| Understand the language syntax | [LOFT.md](LOFT.md), then [STDLIB.md](STDLIB.md) |
| Add a feature to the compiler | [COMPILER.md](COMPILER.md) → [INTERMEDIATE.md](INTERMEDIATE.md) → [INTERNALS.md](INTERNALS.md) |
| Debug a runtime crash | [PROBLEMS.md](PROBLEMS.md) (check open issues) → [TESTING.md](TESTING.md) § LogConfig → [INTERNALS.md](INTERNALS.md) |
| Add a native (Rust) standard library function | [INTERNALS.md](INTERNALS.md) § Native Function Registry, then `default/01_code.loft` |
| Plan or review enhancements | [PLANNING.md](PLANNING.md), then [OPTIMISATIONS.md](OPTIMISATIONS.md) |
| Implement a PLANNING.md item | [DEVELOPMENT.md](DEVELOPMENT.md) — branching, commit order, CI |
| Understand the parallel execution model | [THREADING.md](THREADING.md), then [INTERNALS.md](INTERNALS.md) § Parallel Execution |
| Set up logging in a loft program | `STDLIB.md § Logging`, then [LOGGER.md](LOGGER.md) |
| Understand the heap / memory model | [DATABASE.md](DATABASE.md), then `INTERMEDIATE.md § DbRef` |
| Improve the test suite | [TESTING.md](TESTING.md), then `tests/scripts/` and `tests/docs/` |

## Debug logging — `LOFT_LOG` quick reference

Set this env var before `cargo test` to control what appears in `tests/dumps/*.txt`:

| Value | What you get |
|---|---|
| *(unset)* or `full` | IR + bytecode + execution, slot annotations (default) |
| `static` | IR + bytecode only — fastest for codegen debugging |
| `minimal` | Execution trace for `test` only — cleanest for runtime bugs |
| `ref_debug` | Full + stack snapshots after every Ref/CreateStack op |
| `bridging` | Execution + bridging-invariant warnings |
| `crash_tail:N` | Last N execution lines; flushed on panic |
| `fn:<name>` | Only the named function |
| `variables` | Variable table (name, type, scope, slot, live interval) per function |
| `all_fns` | Bytecode of all functions including `default/` built-ins (large; use when crash address falls inside a built-in) |

See [TESTING.md](TESTING.md) § LogConfig and `src/log_config.rs` for the full API.
