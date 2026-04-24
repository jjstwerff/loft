---
render_with_liquid: false
---
# What's new in loft

A short, friendly log of what has changed in each release.  Read top-to-bottom
for a tour of how the language has grown.

Looking for the deep technical history (opcode renames, slot allocator
invariants, internal phase numbers)?  See
[doc/claude/CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md).

---

## Unreleased — heading toward 0.8.5

0.8.5 adds **learnability**: syntax highlighting, a VS Code extension,
and a "learn loft in 30 minutes" walkthrough so new users can get from
zero to a running demo without reading the reference.

## 0.8.4 — 2026-04-24 — Awesome Brick Buster

This release focuses on **the web**: your loft programs can now fetch
URLs, serve HTTP, parse JSON, and even run entirely inside a browser tab.
The headline is **Brick Buster** — a full arcade game, paddle + ball +
powerups + music + levels + high score, that you can share with a friend
via a single URL.

### JSON — read and write structured data

```loft
v = json_parse("{\"name\":\"Alice\",\"age\":30}")
println(v.field("name").as_text())   // Alice
println(v.to_json_pretty())          // formatted output
```

- `json_parse(text)` turns JSON into a value you can explore.
- Bad input returns a null value instead of crashing.  Ask
  `json_errors()` what went wrong.
- Build JSON from code with `json_number`, `json_string`,
  `json_array`, `json_object`, ...
- Read it back with `field("key")`, `item(index)`, `len()`, `keys()`.
- `MyStruct.parse(json_value)` fills a struct from JSON in one line.

### HTTP — talk to the web

```loft
use web
r = http_get("https://example.com")
if r.ok() { println(r.body) }
```

- `http_get`, `http_post`, `http_put`, `http_delete` — straightforward
  blocking calls that return an `HttpResponse` with `.status`, `.body`,
  and `.ok()`.
- `..._h` variants accept custom headers: `http_get_h(url, ["Accept: application/json"])`.
- A simple HTTP **server** is also available: `for req in listen(8080) { respond(req, ...) }`.

### Lighting that actually lights

The 3D renderer's PBR shader now uses the light colours and intensity
you pass in.  Previously the `Light` struct was accepted by the
scene-graph but the shader ignored `color_r/g/b`, `intensity`, and all
point lights — every scene looked as if lit by a single neutral-white
directional.

- A directional light's `intensity` scales its contribution.
- A scene's first **point light** is now rendered (quadratic
  attenuation; no shadow yet).
- Goldens for five of the graphics examples are checked in as
  regression guards — a shader tweak that breaks lighting is caught by
  a pixel-diff test.

### Games in the browser

- **Brick Buster** — a complete arcade game (paddle, ball, powerups,
  music, levels, high score) that runs in your browser and on the
  desktop.  Try it at
  <https://jjstwerff.github.io/loft/brick-buster.html>.
- **Graphics gallery** — 24 WebGL demos, from hello-triangle to
  physically-based rendering.
- `loft --html program.loft` produces a single folder you can drop on
  any static web host.

### Easier code, clearer errors

- `parallel { }` really runs in parallel now (one OS thread per arm).
- `x ?? return err` — one line instead of a two-line null check.
- `type Handler = fn(Request) -> Response` — name function and tuple
  types.
- Any type with `fn next(self) -> Item?` can be used in `for x in val`.
- When the interpreter hits a fatal error, it now tells you *which
  function and line* triggered it before exiting.

### A gentler language

- `integer` is now 64-bit everywhere.  Big numbers like
  `9_876_543_210` just work — no suffix required.
- The old `long` type and `33l` literal suffix are gone; use `integer`
  and `33`.
- Three crashes involving `match` on complex types are fixed —
  character interpolation, uneven match arms, and chained native calls
  no longer leak memory.

### Native editor & tooling

- **Native Moros editor** — a full OpenGL editor ships as a standalone
  app you can distribute without installing loft.
- `loft --dump file.loft` — show the compiled bytecode without running
  the program.  Handy when something compiles oddly.
- New test runner: `scripts/find_problems.sh --bg` runs the whole suite
  in the background; check in with `--peek` or `--wait`.

---

### Closures you can return

Functions that return a closure now work correctly, including when the
closure captures variables from the enclosing scope:

```loft
fn make_greeter(prefix: text) -> fn(text) -> text {
    |name| { "{prefix}, {name}!" }
}
hello = make_greeter("Hi")
println(hello("Ada"))   // Hi, Ada!
```

Capturing closures also work with `map` and `filter`:

```loft
factor = 10
big = map(nums, |x| { x * factor })
```

### Quality-of-life fixes

- **Typos stop compilation.**  `y = unknown_thing` now fails with a
  clear error instead of silently creating a garbage value.
- **`rev(vector)`** — you can now iterate a plain vector in reverse.
- **Format strings** — `"{n:<5}"` (left-align), `"{n:^5}"` (centre) and
  `"{f:.0}"` (zero decimals) all behave the way you'd expect.
- **File reading** — `file.lines()` now returns text after the last
  newline, not just full lines.
- **Sorted collections** — descending primary-key ranges return
  correct results in every mode.
- **Windows paths** — native compilation correctly escapes `\` in file
  paths.

### Faster programs

- The compiler does arithmetic at compile time where it can, so
  `[for i in 0..100 { i * 2 }]` becomes a ready-made vector instead of
  a loop.
- `par(...)` automatically picks a lighter, faster worker when your
  work doesn't need its own scratch memory — no syntax change.

### Better docs

- New pages on **pattern matching** and **format strings**.
- Expanded chapters on images, threading, and generics.
- 137-page PDF reference regenerated.

---

## 0.8.3 — 2026-03-27 — WebAssembly!

Loft now runs in the browser.  The playground at
<https://jjstwerff.github.io/loft/playground.html> compiles and executes
loft programs entirely in your browser tab — no server involved.

Behind the scenes:

- A virtual in-memory filesystem for browser tests.
- Captured `println` output for the playground.
- A stable plugin protocol so native extensions (imaging, random, web)
  can be loaded at runtime.
- String-heavy programs are faster thanks to format-string
  pre-allocation.

---

## 0.8.2 — 2026-03-24

### Lambdas

Write throw-away functions inline:

```loft
doubled = map([1, 2, 3], |x| { x * 2 })
```

The short form `|x| { ... }` infers types from where you use it.  Use
the long form `fn(x: integer) -> integer { ... }` when you want them
explicit.

### Named arguments and defaults

```loft
fn connect(host: text, port: integer = 80, tls: boolean = true) { ... }

connect("localhost")                       // uses both defaults
connect("localhost", tls: false)           // skips port by name
```

### Native compilation

Ship your loft program as a real native binary:

- `loft --native file.loft` — compile and run via `rustc`.
- `loft --native-emit out.rs` — save the generated Rust source.
- `loft --native-wasm out.wasm` — compile to WebAssembly.

### JSON, computed fields, field constraints

- `"{value:j}"` serialises any struct to JSON.
- `Type.parse(json_text)` parses JSON back into a struct.
- `computed(expr)` fields are recalculated on every read, no storage
  needed: `area: float computed(PI * $.r * $.r)`.
- `assert(...)` clauses on struct fields validate every write.

### Small but welcome

- Workers started with `par(...)` can now return `text` and enum
  values, not just numbers.
- `fn` prefix dropped on function references: write `apply(double, 7)`,
  not `apply(fn double, 7)`.
- `pub` is now required to expose a definition to other files — this
  keeps your module boundaries tidy.

### Clearer errors

- Using `string` as a type suggests `text` instead of a generic error.
- Six common mistakes now come with a fix suggestion.
- Several crashes on unusual input have become proper error messages.

### Bug fixes

- `c + d` on two characters now produces text, not a crash.
- Empty vector literal `[]` as an argument no longer crashes.
- `v += other_vec` on text-bearing vectors no longer corrupts data.
- `map`, `filter`, and `reduce` no longer trip over their own internal
  slots.

---

## 0.8.0 — 2026-03-17

### Match expressions

Pattern-match enums, structs, and scalars:

```loft
match shape {
    Circle { r } => PI * pow(r, 2.0),
    Rect { w, h } => w * h,
}
```

- The compiler checks that you cover every case.
- Supports `North | South =>` (or-patterns), `if r > 0.0` (guards),
  `1..=9` (ranges), null patterns, character patterns, and full
  `{ ... }` block bodies.

### Formatter

- `loft --format file.loft` — format in place.
- `loft --format-check file.loft` — fails if not formatted; useful in
  CI.

### Imports

- `use mylib::*` — bring in everything.
- `use mylib::Point, add` — pick out just what you need.
- Local definitions always win over imported ones.

### Higher-order helpers

```loft
doubles = map(numbers, fn double)
evens   = filter(numbers, fn is_even)
total   = reduce(numbers, fn add, 0)
```

### Testing made easier

- `loft --tests file.loft::test_name` — run a single test.
- `loft --tests 'file.loft::{a,b}'` — run a selection.
- `loft --tests --native` — compile tests to a native binary first.

### New standard-library helpers

- `now()` — milliseconds since 1970.
- `ticks()` — microseconds since program start, monotonic.
- `mkdir(path)` / `mkdir_all(path)` — make directories.
- `vector.clear()` — empty a vector.

### Clearer warnings

- Division or modulo by a constant zero.
- Unused loop variables (silence with `for _i in ...`).
- Unreachable code after `return`, `break`, or `continue`.
- Redundant null checks on `not null` fields.

### Bug fixes

- `x << 0` and `x >> 0` now return `x` instead of null.
- `NaN != x` now returns `true` (it was wrongly `false`).
- `??` works correctly with floats.
- Using `if` as an expression without `else` is now a compile error
  rather than silently returning null.
- Assigning `null` to a struct field no longer crashes.
- `sorted[key] = null` and `hash[key] = null` remove the entry, as
  documented.

---

## 0.1.0 — 2026-03-15 — First release

The core language, in one place.

### Types and values

- **Static types with inference** — no type annotations on locals; the
  compiler figures out the type from the first assignment.
- **Null safety** — every value may be null unless declared `not
  null`; null propagates through arithmetic; `?? default` supplies a
  fallback.
- **Primitives** — `boolean`, `integer`, `long`, `float`, `single`,
  `character`, `text`.
- **Structs** — named records: `Point { x: 1.0, y: 2.0 }`.
- **Enums** — both plain enums and struct-enums (variants with fields
  and per-variant methods).

### Control flow

- `if`/`else`, `for`/`in`, `break`, `continue`, `return`.
- For-loop extras — inline filter (`for x in v if x > 0`), loop
  attributes (`x#first`, `x#count`, `x#index`), in-loop removal
  (`v#remove`).

### Working with collections

- `[for x in v { expr }]` — vector comprehensions.
- `vector<T>` (dynamic array), `sorted<T>` (ordered tree),
  `index<T>` (multi-key tree), `hash<T>` (hash table).

### Text and formatting

- `"Hello {name}, score: {score:.2}"` — string interpolation with
  format specifiers.

### Other

- **Parallel execution** — `for a in items par(b=worker(a), 4) { ... }`
  spreads the work across CPU cores.
- **File I/O** — read, write, seek, directory listing, PNG images.
- **Logging** — `log_info`, `log_warn`, `log_error` with source
  location and rate limiting.
- **Libraries** — `use mylib;` imports from `.loft` files.

---

## Version comparison links

- [Unreleased vs 0.8.3](https://github.com/jjstwerff/loft/compare/v0.8.3...main)
- [0.8.3](https://github.com/jjstwerff/loft/compare/v0.8.2...v0.8.3)
- [0.8.2](https://github.com/jjstwerff/loft/compare/v0.8.0...v0.8.2)
- [0.8.0](https://github.com/jjstwerff/loft/compare/v0.1.0...v0.8.0)
- [0.1.0](https://github.com/jjstwerff/loft/releases/tag/v0.1.0)
