<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN92 Pass 1 — proposed `@F`/`@I` mapping (REVIEW DRAFT — nothing created)

**Status: Pass 1 COMPLETE (2026-07-01).** 80 catalogue issues minted in
[`loft-lang/features`](https://github.com/loft-lang/features) — **`@F1`–`@F56`** (features)
+ **`@I57`–`@I80`** (subsystems), 0 failed; `kind:feature` / `kind:infra` labels created.
Those issues (+ `idx features`) are now the **canonical registry**; the grouped list
below is the proposal/rationale, and [§ Minted numbers](#minted-numbers) is a
creation-time snapshot. **No source/doc tags yet** — Pass 2 (source) / Pass 3 (docs) next.
Open granularity calls in [§ Decisions for review](#decisions-for-review) were left at
"good enough" (finer features added later — cheap).

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
- **`par(...)` parallel for-loop** — LOFT.md §Parallel / THREADING.md — *(mechanism: @I72 parallel runtime)*
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
- **Logging & diagnostics API (log_*, print, assert, panic)** — LOFT.md §Logging — *(runtime: @I76 logger)*
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
- **Native-binary backend (`--native` → rustc)** — *(mechanism: @I68 native generator)*
- **Browser / WASM target (`--html` / `--native-wasm`)** — *(mechanism: @I68 native generator + @I66 VM/state)*
- **Package management (`loft install`, `loft.toml`, lockfile)** — *(mechanism: @I77 registry/manifest + @I74 cdylib loader)*
- **Live code reload (patch a running program)** — *(mechanism: @I78 live-reload dispatch)*

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
- **Native Rust generator** *(mechanism of the native backend @F53)* — `src/generation/*`
- **Word-addressed store** — `src/store.rs`
- **Database — alloc / persistence / journal / snapshot / schema** — `src/database/*`
- **DbRef pointers & collection keys** — `src/keys.rs`
- **Parallel execution runtime** *(mechanism of `par` @F33)* — `src/parallel.rs`
- **Native function registry** — `src/native.rs`
- **CDylib extension loader** *(mechanism of package loading — @F55)* — `src/extensions.rs`
- **Diagnostics collector** *(mechanism behind user-facing error messages)* — `src/diagnostics.rs`
- **Logger runtime** *(mechanism of the logging @F44)* — `src/logger.rs`
- **Registry / manifest / lockfile resolution** *(mechanism of package mgmt @F55)* — `src/registry.rs`, `src/manifest.rs`, `src/lockfile.rs`
- **Live-reload dispatch** *(mechanism of live reload @F56)* — `src/live_reload.rs`, `src/live_dispatch.rs`
- **Documentation generator (maintainer tooling)** — `src/gendoc.rs`, `src/documentation.rs`
- **Tracker indexer (maintainer tooling)** — `tools/indexer/src/scan.loft`

---

## Minted numbers

Creation-time snapshot (canonical source: the `loft-lang/features` issues + `idx features`).
Pass 2 places `// @F<n>` / `// @I<n>` at the source sites for these.

```
@F1  Nullable value semantics — in-band sentinels + null-fallback arithmetic
@F2  ?? null-coalescing operator (incl. ?? return)
@F3  Primitive scalar types (integer, float, single, boolean, character)
@F4  Ranged/width integer types (u8/i8/u16/i16/i32/u32)
@F5  Type conversions — implicit, format-only, and explicit `as`
@F6  vector<T> — append, index, slice, comprehensions, aggregates, map/filter/reduce
@F7  hash<T[keys]> keyed collection
@F8  sorted<T[keys]> collection
@F9  index<T[keys]> B-tree index (asc/desc, multi-key)
@F10 iterator<T> values
@F11 Tuples — anonymous fixed-arity (T1, T2, …)
@F12 Struct records — fields, = default, computed, limit/not null/assert
@F13 Simple enums (ordered value types)
@F14 Polymorphic struct-enums (per-variant fields)
@F15 Enum-scoped variant names + context inference
@F16 Functions & declarations (pub, parameters, return)
@F17 Named arguments + default parameter values
@F18 const parameters
@F19 Method dispatch via self / both
@F20 Variant-based dynamic dispatch
@F21 References &T — parameters + write-back bindings
@F22 Closures & lambdas (value capture, cross-scope)
@F23 Function references as first-class values
@F24 Higher-order functions (map / filter / reduce)
@F25 Generics — single type variable <T>, inferred
@F26 Interfaces & bounded generics (<T: A + B>, operator interfaces)
@F27 if / else as an expression
@F28 for-in loops — ranges, loop attributes, filtered, rev()
@F29 Pattern matching — enum/scalar/tuple, guards, or-patterns, exhaustiveness
@F30 is variant check (+ field capture)
@F31 break / continue + labelled forms
@F32 Custom iterators via fn next(self) -> T?
@F33 par(...) parallel for-loop
@F34 Coroutines / generators — yield, yield from
@F35 String literals — {expr} interpolation + backtick multiline
@F36 String formatting / format specifiers (+ for-expressions)
@F37 Operator set — arithmetic/comparison/logical/bitwise/unary, **
@F38 Arithmetic safety — overflow/÷0 → null, nullable peers
@F39 Math & trigonometry library
@F40 File & directory I/O (+ durable-store binding)
@F41 Environment & arguments (env vars, arguments(), program dirs, path resolution)
@F42 JSON — json_parse, JsonValue, Type.parse, to_json
@F43 Random numbers (rand_seed / rand / rand_indices)
@F44 Logging & diagnostics API (log_*, print, assert, panic)
@F45 sizeof()
@F46 Type aliases (type X = …)
@F47 Library imports / module system (use forms, pub)
@F48 The loft CLI — run a program, --interpret / --native, --timeout, --help
@F49 REPL — interactive sessions
@F50 Introspection — loft introspect (bytecode / native Rust / slot tables)
@F51 Debugger — breakpoints, frame capture, scripted RPC
@F52 Source formatter — canonical .loft output
@F53 Native-binary backend (--native → rustc)
@F54 Browser / WASM target (--html / --native-wasm)
@F55 Package management (loft install, loft.toml, lockfile)
@F56 Live code reload — patch a running program
@I57 Lexer
@I58 Parser (two-pass recursive descent)
@I59 Type resolver
@I60 Scope & dependency/lifetime tracker (deps)
@I61 Stack slot allocator
@I62 IR data model (Value/Type/Data)
@I63 Store-resident IR (reader + handle + materializer)
@I64 Bytecode compiler (IR -> bytecode)
@I65 Bytecode code generator
@I66 Bytecode VM / executor
@I67 Opcode implementations
@I68 Native Rust generator
@I69 Word-addressed store
@I70 Database — alloc / persistence / journal / snapshot / schema
@I71 DbRef pointers & collection keys
@I72 Parallel execution runtime
@I73 Native function registry
@I74 CDylib extension loader
@I75 Diagnostics collector
@I76 Logger runtime
@I77 Registry / manifest / lockfile resolution
@I78 Live-reload dispatch
@I79 Documentation generator (maintainer tooling)
@I80 Tracker indexer (maintainer tooling)
```

## `@F` ↔ `@I` mechanism map

The catalogue keeps a user **capability** (`@F`) and its implementing
**mechanism** (`@I`) as separate entries (see the classification axis above);
this table is the canonical join between them, so `idx tag:@F33` and
`idx tag:@I72` surface each other. It is derived from the `*(mechanism …)*`
annotations in the `@F`/`@I` lists above, now carrying minted numbers.

Only clean *"this `@I` **is the mechanism of** that `@F`"* pairs are listed.
Cross-cutting subsystems — lexer (@I57), parser (@I58), type resolver (@I59),
slot allocator (@I61), IR model (@I62), bytecode compiler/generator
(@I64/@I65), opcode impls (@I67), word-addressed store (@I69) — implement
*most* features and are deliberately left unpaired (pairing them would join
everything to everything).

| user feature `@F` | implementing mechanism `@I` |
|---|---|
| @F7 hash · @F8 sorted · @F9 index (keyed collections) | @I71 DbRef pointers & collection keys · @I70 database |
| @F21 references `&T` | @I60 scope & dependency/lifetime tracker (`deps`) |
| @F33 par | @I72 parallel execution runtime |
| @F40 file & directory I/O (durable-store binding) | @I70 database (alloc/persistence/journal) |
| @F44 logging & diagnostics API | @I76 logger runtime |
| @F53 native-binary backend | @I68 native Rust generator |
| @F54 browser / WASM target | @I68 native Rust generator · @I66 bytecode VM/state |
| @F55 package management | @I77 registry/manifest/lockfile · @I74 cdylib extension loader |
| @F56 live code reload | @I78 live-reload dispatch |

Add a row when a new `@I` is genuinely one feature's mechanism; leave
cross-cutting infrastructure unpaired. (This is the in-repo join layer, like
the `**Catalogue:**` anchors in DESIGN_DECISIONS.md — not content added to the
zero-deferral issue bodies.)

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
