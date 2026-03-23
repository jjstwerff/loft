# loft

A statically-typed scripting language with null safety, built-in collections, and parallel execution.

[![License: LGPL-3.0](https://img.shields.io/badge/License-LGPL--3.0-blue.svg)](LICENSE)

---

## Quick look

```loft
struct Point { x: float, y: float }

fn distance(a: Point, b: Point) -> float {
    dx = a.x - b.x
    dy = a.y - b.y
    sqrt(dx * dx + dy * dy)
}

fn main() {
    p1 = Point { x: 0.0, y: 0.0 }
    p2 = Point { x: 3.0, y: 4.0 }
    println("distance: {distance(p1, p2)}")
}
```

```
$ loft hello.loft
distance: 5.0
```

---

## Language features

- **Static types** — every variable has a type inferred at first assignment; mismatches are compile errors
- **Null safety** — all values are nullable by default; arithmetic propagates null; use `?? default` to recover
- **Structs and enums** — struct-enum variants with per-variant method dispatch (polymorphism without vtables)
- **Collections** — `vector<T>`, `sorted<T>`, `index<T>`, `hash<T>` with iterator loop attributes
- **String formatting** — `"{expr}"` interpolation with format specifiers (`{x:06.2}`)
- **Parallel for** — `for a in items par(b=worker(a), threads) { ... }` distributes work across CPU cores
- **Structured logging** — `log_info` / `log_warn` / `log_error` with source location and rate limiting
- **File I/O** — read, write, seek, directory listing, PNG images
- **Lambda expressions** — write a function inline with `fn(x: integer) -> integer { x * 2 }` or the short form `|x| { x * 2 }`; pass directly to `map`, `filter`, and `reduce`
- **Native compilation** — `loft --native file.loft` compiles your program to a native binary via `rustc` for maximum speed; `--native-emit out.rs` saves the generated Rust source
- **Rust integration** — emit typed Rust code from loft type definitions via `gendoc`

---

## Install

### From source (requires Rust 1.85+)

```sh
git clone https://github.com/jjstwerff/loft
cd loft
cargo build --release
# binary is at target/release/loft
```

### With cargo install

```sh
cargo install --git https://github.com/jjstwerff/loft --bin loft
```

### Pre-built binaries

Download a release binary from the [Releases](https://github.com/jjstwerff/loft/releases) page
(Linux x86-64, macOS Intel, macOS ARM, Windows x86-64).

---

## Usage

```
loft [options] <file.loft>
loft --tests [dir | file.loft | file.loft::name]

Options:
  --help                    Show this help
  --version                 Print version
  --path <dir>              Override the project root (where default/ is found)
  --project <dir>           Sandbox file I/O and search lib/ for imports
  --lib <dir>               Add a 'use' import search path (repeatable)
  --native                  Compile to a native binary via rustc and run it
  --native-release          Like --native with full optimisations (-O)
  --native-emit [out.rs]    Write the generated Rust source (do not compile)
  --native-wasm [out.wasm]  Compile to WebAssembly (wasm32-wasip2)
  --tests [dir|file]        Discover and run fn test*() functions
  --tests --native          Same but compile to native Rust first
  --format <file>           Format a .loft file in-place
  --no-warnings             Suppress warnings in --tests output
```

The interpreter loads the standard library from `default/` relative to the binary, then
parses and executes `<file.loft>`. The entry point is `fn main()`.

### Test runner

```sh
loft --tests                          # all .loft files in current dir (recursive)
loft --tests tests/scripts            # specific directory
loft --tests file.loft                # single file
loft --tests file.loft::test_name     # single function
loft --tests 'file.loft::{a,b}'      # multiple functions
loft --tests --native tests/scripts   # compile to native and run
```

---

## Documentation

The user documentation is hosted at **<https://jjstwerff.github.io/loft/>**.
A single-page version is at [print.html](https://jjstwerff.github.io/loft/print.html)
and a printable reference at [loft-reference.pdf](https://jjstwerff.github.io/loft/loft-reference.pdf).

To build locally: run `cargo run --bin gendoc`, then open `doc/index.html`.

**Language tutorial** (each page is also a live test):

| Page | Topic |
|------|-------|
| [vs Python](https://jjstwerff.github.io/loft/00-vs-python.html) | Loft compared to Python |
| [vs Rust](https://jjstwerff.github.io/loft/00-vs-rust.html) | Loft compared to Rust |
| [Keywords](https://jjstwerff.github.io/loft/01-keywords.html) | Control flow: if, for, break, continue |
| [Texts](https://jjstwerff.github.io/loft/02-text.html) | Strings, slicing, iteration |
| [Integers](https://jjstwerff.github.io/loft/03-integer.html) | Arithmetic, bitwise, null |
| [Functions](https://jjstwerff.github.io/loft/06-function.html) | Parameters, return, fn-refs, map/filter/reduce |
| [Vectors](https://jjstwerff.github.io/loft/07-vector.html) | Dynamic arrays, comprehensions, clear |
| [Structs](https://jjstwerff.github.io/loft/08-struct.html) | Fields, methods, sizeof |
| [Enums](https://jjstwerff.github.io/loft/09-enum.html) | Variants, polymorphism, match expressions |
| [Collections](https://jjstwerff.github.io/loft/10-sorted.html) | Sorted, [Index](https://jjstwerff.github.io/loft/11-index.html), [Hash](https://jjstwerff.github.io/loft/12-hash.html) |
| [Libraries](https://jjstwerff.github.io/loft/17-libraries.html) | Imports, wildcard imports, extending types |
| [Parallel](https://jjstwerff.github.io/loft/19-threading.html) | par(...) for-loop parallelism |
| [Safety](https://jjstwerff.github.io/loft/23-safety.html) | Language traps and how to avoid them |

**Standard library API:** [stdlib.html](https://jjstwerff.github.io/loft/stdlib.html)

**For contributors:** [DEVELOPERS.md](https://jjstwerff.github.io/loft/DEVELOPERS.md) — feature proposals, quality gates, diagnostic guide

---

## Known limitations (0.8.x)

- **No closure capture** — lambdas work but cannot read variables from the surrounding scope; pass needed values as extra arguments
- **No REPL** — interactive mode is planned for 0.9.0

---

## License

LGPL-3.0-or-later — see [LICENSE](LICENSE).
