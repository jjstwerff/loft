<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN92 Pass 1 — proposed `@F`/`@I` mapping (REVIEW DRAFT — nothing created)

**Status: APPROVED — Pass 1 final (creating `loft-lang/features` issues to mint numbers).**
Approved as the *initial* catalogue; finer features get added later (the system makes
that cheap). The number → entry map is backfilled here once the issues exist. **No
source/doc tags yet** — those are Pass 2 / Pass 3. Open granularity calls in
[§ Decisions for review](#decisions-for-review) were left at "good enough".

## Classification axis (the test)

`@F` vs `@I` is **not** Rust-vs-loft or syntax-vs-tool. It is **"does a loft *user*
invoke or benefit from it?"** Concretely: **can you write a user-value sentence + a
usage example for it?**
- **Yes → `@F`** — language syntax, stdlib, *and the toolchain* (CLI, REPL, debugger,
  formatter, install, backends). The example IS the spec/demo.
- **No, pure machinery → `@I`** — lexer, parser, type resolver, deps, IR, VM, opcodes,
  codegen, store. You cannot write a user example for "the slot allocator".

A user capability and its implementing subsystem are **separate entries**: the `@F`
anchors to the user entry point + example; the `@I` covers the implementing source
region. (E.g. `@F` "compile to a native binary" vs `@I` "native Rust generator".)

**Grain:** `@F` ≈ capability-level (one LOFT.md section / doc topic), not per-function;
`@I` coarse, one per subsystem.

Counts as drafted: **`@F` ≈ 57** (incl. toolchain) · **`@I` ≈ 24** pure-internal / mechanism / maintainer-tooling.

---

## Proposed `@F` — user-facing features

### Types & the null model
- **Nullable value semantics — in-band sentinels + null-fallback arithmetic** — LOFT.md §Types/Null
- **`??` null-coalescing (incl. `?? return`)** — LOFT.md §Operators
- **Primitive scalar types (integer, float, single, boolean, character)** — LOFT.md §Types — *(split per doc topic?)*
- **Ranged/width integers (u8/i8/u16/i16/i32/u32)** — LOFT.md §Types
- **Type conversions — implicit / format-only / `as`** — LOFT.md §Operators

### Collections
- **`vector<T>` — append/index/slice + comprehensions + aggregates + map/filter/reduce** — LOFT.md §Vectors
- **`hash<T[keys]>`** — LOFT.md §Key-based collections
- **`sorted<T[keys]>`** — LOFT.md §Key-based collections
- **`index<T[keys]>` (B-tree, asc/desc)** — LOFT.md §Key-based collections
- **`iterator<T>`** — LOFT.md §Custom iterators — *(fold into coroutines?)*
- **Tuples `(T1, T2, …)`** — TUPLES.md

### Structs & enums
- **Struct records — fields, `= default`, `computed`, `limit`/`not null`/`assert`** — LOFT.md §Structs
- **Simple enums** — LOFT.md §Enum types
- **Polymorphic struct-enums** — LOFT.md §Enum types
- **Enum-scoped variant names + context inference** — LOFT.md §Enum-scoped variants — *(fold into enums?)*

### Functions & call conventions
- **Functions & declarations (`pub`, params)** — LOFT.md §Declarations
- **Named arguments + default parameter values** — LOFT.md §Named arguments
- **`const` parameters** — LOFT.md §Declarations
- **Method dispatch (`self`/`both`)** — LOFT.md §Methods
- **Variant-based dynamic dispatch** — LOFT.md §Polymorphism
- **References `&T` (params + write-back bindings)** — LOFT.md §References

### Closures, generics, interfaces
- **Closures & lambdas (value capture, cross-scope)** — LOFT.md §Closures
- **Function references as first-class values** — LOFT.md §Function ref — *(fold into closures?)*
- **Higher-order functions (map/filter/reduce)** — LOFT.md §Lambda — *(fold?)*
- **Generics — `<T>`, inferred** — LOFT.md §Generic functions
- **Interfaces & bounded generics (`<T: A + B>`, operator interfaces)** — LOFT.md §Interfaces

### Control flow
- **`if`/`else` as expression** — LOFT.md §Control flow
- **`for`-in loops — ranges, `#first`/`#count`/`#index`/`#remove`, filtered, `rev()`** — LOFT.md §For loops
- **Pattern matching — enum/scalar/tuple, guards, or-patterns, destructuring, exhaustiveness** — LOFT.md §Match
- **`is` variant check** — LOFT.md §is variant check — *(fold into match?)*
- **`break`/`continue` + labelled** — LOFT.md §Break and continue
- **Custom iterators (`fn next`)** — LOFT.md §Custom iterators — *(fold?)*

### Concurrency
- **`par(...)` parallel for-loop** — LOFT.md §Parallel / THREADING.md — *(mechanism: @I parallel runtime)*
- **Coroutines / generators (`yield`, `yield from`)** — COROUTINE.md

### Strings, operators
- **String literals — interpolation + backtick multiline** — LOFT.md §String literals
- **String formatting / format specifiers (+ for-expr)** — LOFT.md §String formatting
- **Operator set (arithmetic/comparison/logical/bitwise/unary, `**`)** — LOFT.md §Operators
- **Arithmetic safety (overflow/÷0 → null, nullable peers)** — LOFT.md §Arithmetic safety — *(fold into null model?)*

### Stdlib libraries
- **Math & trigonometry** — LOFT.md §Math
- **File & directory I/O (+ durable-store binding)** — LOFT.md §File System
- **Environment & args (env vars, `arguments()`, program dirs, path resolution)** — LOFT.md §Environment
- **JSON (json_parse / JsonValue / `Type.parse` / to_json)** — LOFT.md §Parsing
- **Random numbers** — LOFT.md §Random numbers
- **Logging & diagnostics API (log_*, print, assert, panic)** — LOFT.md §Logging — *(runtime: @I logger)*
- **`sizeof()`** — LOFT.md §Sizeof — *(fold into types?)*
- **Type aliases** — LOFT.md §Declarations — *(fold into types?)*

### Modules
- **Library imports / module system (`use` forms, `pub`)** — LOFT.md §Library imports

### Toolchain, CLI & backends  ← reclassified from `@I`
- **The `loft` CLI — run a program, `--interpret` / `--native`, `--timeout`, `--help`** — surface: `src/main.rs`
- **REPL — interactive sessions** — surface: `src/repl.rs`
- **Introspection — `loft introspect` (bytecode / native Rust / slot tables)** — surface: `src/introspect.rs`
- **Debugger — breakpoints, frame capture, scripted RPC** — surface: `src/debugger.rs`
- **Source formatter — canonical `.loft` output** — surface: `src/formatter.rs`
- **Native-binary backend (`--native` → rustc)** — *(mechanism: @I native generator)*
- **Browser / WASM target (`--html` / `--native-wasm`)** — *(mechanism across generation/ + state)*
- **Package management (`loft install`, `loft.toml`, lockfile)** — *(mechanism: @I registry/manifest)*
- **Live code reload (patch a running program)** — *(mechanism: @I live-reload dispatch)*

---

## Proposed `@I` — internal-only subsystems (coarse)

Pure machinery + the *mechanisms* implementing an `@F` surface + maintainer tooling.
None has a user-program-author usage example.

- **Lexer** — `src/lexer.rs`
- **Parser (two-pass recursive descent)** — `src/parser/*`
- **Type resolver** — `src/typedef.rs`
- **Scope & dependency/lifetime tracker (`deps`)** — `src/scopes.rs`
- **Stack slot allocator** — `src/variables/*`
- **IR data model (Value/Type/Data)** — `src/data.rs`
- **Store-resident IR (reader + handle + materializer)** — `src/data_store.rs`, `src/ir_node.rs`, `src/ir_store.rs`
- **Bytecode compiler (IR → bytecode)** — `src/compile.rs`
- **Bytecode code generator** — `src/state/codegen.rs`
- **Bytecode VM / executor** — `src/state/mod.rs` (+ text.rs, io.rs, debug.rs)
- **Opcode implementations** — `src/fill.rs`
- **Native Rust generator** *(mechanism of the native backend `@F`)* — `src/generation/*`
- **Word-addressed store** — `src/store.rs`
- **Database — alloc / persistence / journal / snapshot / schema** — `src/database/*`
- **DbRef pointers & collection keys** — `src/keys.rs`
- **Parallel execution runtime** *(mechanism of `par` `@F`)* — `src/parallel.rs`
- **Native function registry** — `src/native.rs`
- **CDylib extension loader** *(mechanism of package loading)* — `src/extensions.rs`
- **Diagnostics collector** *(mechanism behind user-facing error messages)* — `src/diagnostics.rs`
- **Logger runtime** *(mechanism of the logging `@F`)* — `src/logger.rs`
- **Registry / manifest / lockfile resolution** *(mechanism of package mgmt `@F`)* — `src/registry.rs`, `src/manifest.rs`, `src/lockfile.rs`
- **Live-reload dispatch** *(mechanism of live reload `@F`)* — `src/live_reload.rs`, `src/live_dispatch.rs`
- **Documentation generator (maintainer tooling)** — `src/gendoc.rs`, `src/documentation.rs`
- **Tracker indexer (maintainer tooling)** — `tools/indexer/src/scan.loft`

---

## Decisions for review

1. **Applied:** the toolchain (CLI, REPL, introspect, debugger, formatter, backends,
   package mgmt, live-reload) moved `@I → @F` — they have user-value + examples. Their
   *internal mechanisms* stay `@I` (annotated above). Confirm the split feels right.
2. **Scalar-type granularity.** One `@F` "Primitive scalar types", or split per doc
   topic (`integer`/`boolean`/`float`/`character`)? Per-type doc pages argue for splitting.
3. **Fold candidates (marked inline).** `is`→match? `iterator`→coroutines? `fn-refs`/
   `higher-order`→closures? `sizeof`/`type aliases`→types? `arithmetic safety`→null model?
4. **Maintainer tooling as `@I`?** gendoc + the indexer aren't user-facing *and* aren't
   language machinery — keep as `@I` (so the coverage gate covers `tools/`), or exclude
   from the catalogue entirely?
5. **Library-doc topics not in core.** `15-lexer`/`16-parser` doc the loft-level
   *library* (`lib/lexer.loft`); spacial/radix are library-scoped. Exclude from the core
   catalogue (own lib repos), or include?
6. **Coverage-gate scope.** All of `src/` + `tools/`, or just the language core
   (parser/compiler/VM/store/codegen)? Decides whether maintainer-tooling `@I` count.
7. **Dedicated WASM/HTML `@I`?** The browser target's machinery spans generation/ + state
   — give it its own `@I`, or leave it inside the native generator `@I`?

Next (Pass 1 final, after approval): create the agreed entries as `loft-lang/features`
issues to mint `@F###`/`@I###` numbers.
