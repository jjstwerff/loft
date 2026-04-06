---
render_with_liquid: false
---
# loft

A programming language for making games that run in the browser.

[![License: LGPL-3.0](https://img.shields.io/badge/License-LGPL--3.0-blue.svg)](LICENSE)

---

## Vision

**Write a game in loft.  Share a link.  Anyone can play.**

Loft is a statically-typed scripting language designed for small games and
interactive experiences that run directly in the browser — no install, no app
store, no engine download.  Your game compiles to WebAssembly + WebGL and
deploys as static files on any web host.

For developers who prefer desktop, the same code runs natively with OpenGL.
For 3D artists, scenes export to standard GLB files viewable in Blender or
any glTF viewer.  One language, three outputs.

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

## Why loft?

### For game developers

- **Browser-first** — games compile to WASM+WebGL and run in any modern browser
- **Native option** — same code renders with OpenGL on desktop for low-latency development
- **GLB export** — scenes with PBR materials and lights export to standard glTF 2.0
- **Batteries included** — 2D canvas drawing, 3D mesh primitives, text rendering, PNG export
- **No engine lock-in** — loft is a language, not a framework; you own your rendering pipeline

### For language enthusiasts

- **Static types with inference** — no annotations required, mismatches caught at compile time
- **Null safety** — nullable values, null propagation, `?? default` recovery
- **Structs and enums** — variants, methods, pattern matching
- **Collections** — `vector<T>`, `sorted<T>`, `index<T>`, `hash<T>` with iterators and comprehensions
- **String interpolation** — `"{expr}"` with format specifiers
- **Parallel for** — `par(...)` distributes work across CPU cores
- **Generators** — `iterator<T>` with `yield` for lazy sequences
- **Lambdas and closures** — `|x| { x * 2 }` with capture
- **Tuples** — multi-value returns with destructuring
- **JSON** — serialise with `"{value:j}"`, deserialise with `Type.parse(json)`
- **Native compilation** — compile to a native binary via `rustc` or to WebAssembly

### For performance nerds

Loft uses a **store-based memory model**: each collection lives in its own
contiguous word-addressed block.  All records of a type sit together, so
iterating a vector or walking a tree stays cache-local.  No pointer chasing,
no per-element allocation.

Parallel workers each receive a shallow copy with private allocation state,
so `par(...)` loops avoid lock contention entirely — each core writes to its
own slice without coordination.

---

## Graphics examples

The `lib/graphics/examples/` directory contains 23 progressive examples from
hello-triangle to PBR rendering with shadow mapping:

| # | Example | What it shows |
|---|---|---|
| 01 | Hello Window | Window creation and event loop |
| 02 | Hello Triangle | First triangle with vertex buffers |
| 03 | Shaders | Per-vertex colors with interpolation |
| 04 | Textures | Texture loading and UV mapping |
| 05 | Transformations | MVP matrices, rotating cube |
| 06 | Coordinate Systems | Multiple objects with transforms |
| 07 | Camera | Orbiting camera with sin/cos |
| 08 | Basic Lighting | Phong ambient + diffuse + specular |
| 09 | Materials | Different shininess per object |
| 10 | 2D Canvas | Software rasterization, PNG export |
| 11 | Scene Graph | Multi-object GLB export with lights |
| 12 | Multiple Lights | Directional + point lights |
| 13 | Depth Testing | Depth buffer visualization |
| 14 | Blending | Alpha transparency |
| 15 | Face Culling | Back-face optimization |
| 16 | Shadow Mapping | Two-pass depth shadows |
| 17 | Post-Processing | Render-to-texture effects |
| 18 | PBR | Physically based rendering grid |
| 19 | Complete Scene | All techniques combined |
| 20 | Textured Cube | Procedural texture with text |
| 21 | Keyboard Camera | First-person controls |
| 22 | Wireframe | Alternative draw modes |
| 23 | Cleanup | GPU resource lifecycle |

Run any example:
```sh
cd lib/graphics/examples
./11-scene-graph.loft          # exports a GLB file
./08-basic-lighting.loft       # opens a window with Phong lighting
```

---

## Get involved

Loft is an active project looking for contributors.  Areas where help is most
valuable:

- **WebGL backend** — implementing the browser rendering path so games run online
- **Game library** — input handling, sprite sheets, tilemaps, collision detection
- **Audio** — Web Audio API integration for music and sound effects
- **Example games** — small playable demos that showcase the language
- **Documentation** — tutorials, guides, and the online reference
- **Language features** — REPL, package manager, IDE integration

See [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) for the contribution workflow
and [PLANNING.md](doc/claude/PLANNING.md) for the feature backlog.

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
- **WebGL not yet implemented** — native OpenGL and GLB export work; browser rendering is the next major milestone

---

## License

LGPL-3.0-or-later — see [LICENSE](LICENSE).
