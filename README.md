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

- **Static types** with inference — no type annotations required, mismatches caught at compile time
- **Null safety** — nullable values, null propagation, `?? default` recovery
- **Structs and enums** — variants, methods, pattern matching
- **Collections** — `vector<T>`, `sorted<T>`, `index<T>`, `hash<T>` with iterators and comprehensions
- **String interpolation** — `"{expr}"` with format specifiers
- **Parallel for** — `par(...)` distributes work across CPU cores
- **JSON** — serialise with `"{value:j}"`, deserialise with `Type.parse(json)`
- **Lambdas** — `fn(x: integer) -> integer { x * 2 }` or `|x| { x * 2 }`
- **Native compilation** — compile to a native binary via `rustc` or to WebAssembly
- **File I/O, logging, PNG images** — batteries included

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

```sh
loft program.loft                     # run a program (entry point: fn main())
loft --native program.loft            # compile to native binary and run
loft --tests                          # run all fn test*() functions
loft --tests file.loft::test_name     # run a specific test
loft --format file.loft               # format in-place
loft --help                           # full option list
```

---

## Documentation

Full documentation is at **<https://jjstwerff.github.io/loft/>** — language tutorial, standard library API, and a [printable reference](https://jjstwerff.github.io/loft/loft-reference.pdf).

To build locally: `cargo run --bin gendoc`, then open `doc/index.html`.

---

## Known limitations (0.8.x)

- **No closure capture** — lambdas work but cannot read variables from the surrounding scope; pass needed values as extra arguments
- **No REPL** — interactive mode is planned for 0.9.0

---

## License

LGPL-3.0-or-later — see [LICENSE](LICENSE).
