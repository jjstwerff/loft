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

Options:
  --help       Show this help
  --version    Print version
  --path <dir> Override the project root (where default/ is found)
```

The interpreter loads the standard library from `default/` relative to the binary, then
parses and executes `<file.loft>`. The entry point is `fn main()`.

---

## Documentation

The user documentation is generated HTML — run `cargo run --bin gendoc` to build it,
then open `doc/index.html` in a browser. A single-page version is at `doc/print.html`
and a printable reference at [doc/loft-reference.pdf](doc/loft-reference.pdf).

**Language tutorial** (each page is also a live test):

| Page | Topic |
|------|-------|
| [Loft Language](doc/00-general.html) | First program, core concepts |
| [Keywords](doc/01-keywords.html) | Control flow: if, for, break, continue |
| [Texts](doc/02-text.html) | Strings, slicing, iteration |
| [Integers](doc/03-integer.html) | Arithmetic, bitwise, null |
| [Functions](doc/06-function.html) | Parameters, return, fn-refs, map/filter/reduce |
| [Vectors](doc/07-vector.html) | Dynamic arrays, comprehensions, clear |
| [Structs](doc/08-struct.html) | Fields, methods, sizeof |
| [Enums](doc/09-enum.html) | Variants, polymorphism, match expressions |
| [Collections](doc/10-sorted.html) | Sorted, [Index](doc/11-index.html), [Hash](doc/12-hash.html) |
| [Libraries](doc/17-libraries.html) | Imports, wildcard imports, extending types |
| [Parallel](doc/19-threading.html) | par(...) for-loop parallelism |
| [Safety](doc/23-safety.html) | Language traps and how to avoid them |

**Standard library API:** [doc/stdlib.html](doc/stdlib.html)

**For contributors:** [doc/DEVELOPERS.md](doc/DEVELOPERS.md) — feature proposals, quality gates, diagnostic guide

---

## Known limitations (0.8.x)

- **No lambda expressions** — anonymous functions are planned for 1.1
- **No REPL** — interactive mode is planned for 1.1
- **No in-place vector sort** — `sorted<T>` keeps insertion order; a `sort()` function is planned for 1.1

---

## License

LGPL-3.0-or-later — see [LICENSE](LICENSE).
