---
render_with_liquid: false
---
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
- **Match expressions** — pattern matching on enums, integers, text with guards and destructuring
- **Lambdas and closures** — `fn(x: integer) -> integer { x * 2 }` or `|x| { x * 2 }`; lambdas capture outer variables and work with `map`/`filter`/`reduce`
- **Generators** — `iterator<T>` return type with `yield`; lazy sequences and `yield from` delegation
- **Tuples** — multi-value returns `(integer, text)`, destructuring `(a, b) = fn()`
- **Native compilation** — compile to a native binary via `rustc` or to WebAssembly
- **File I/O, logging, PNG images** — batteries included

---

## What makes loft different

Most languages allocate each object separately on the heap — a struct here, a vector there, a string somewhere else. Traversing a data structure means chasing pointers scattered across memory, pulling in unrelated allocations that happen to live nearby. Cache lines fill with data you don't need.

Loft uses a **store-based memory model**: each named collection of records lives in its own contiguous word-addressed block (`Store`). All records of a given type sit together, so iterating a vector, walking a sorted tree, or scanning a hash table touches only the memory that matters. Unrelated structures stay out of the cache.

This has compounding benefits for collection-heavy algorithms:

- **Vectors** — elements are packed consecutively; no per-element allocation overhead.
- **Sorted / index** — the red-black tree nodes for a `sorted<T>` or `index<T>` live in one store; tree traversal stays local.
- **Hash tables** — the open-addressing table for `hash<T>` is a single flat array; probe chains don't escape the store.
- **Structs in collections** — a `vector<Point>` stores all `Point` records back-to-back, not a vector of pointers to individually-allocated structs.

The allocator (`store.rs`) is a simple first-fit free-list over a doubling buffer. Allocation is fast, and because all records of a type share one store, the working set for any algorithm fits in fewer cache lines.

Parallel workers each receive a shallow copy of the store layout with private allocation state (`clone_for_worker`), so `par(...)` loops avoid lock contention entirely — each core writes to its own slice without coordination.

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
loft --native-wasm out.wasm program.loft  # compile to WebAssembly
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

- **No REPL** — interactive mode is planned for 0.9.0
- **No package manager** — libraries are loaded from local directories; `loft install` is planned for 0.8.4

---

## License

LGPL-3.0-or-later — see [LICENSE](LICENSE).
