---
render_with_liquid: false
---
# loft

A programming language for making games that run in the browser.

[![License: LGPL-3.0](https://img.shields.io/badge/License-LGPL--3.0-blue.svg)](LICENSE)

---

## Vision

**Write a game in loft.  Share a link.  Anyone can play.**

Loft is a statically-typed language designed for small games and interactive
experiences that run directly in the browser — no install, no app store, no
engine download.  Your game compiles to WebAssembly + WebGL and deploys as
static files on any web host.

The same code also runs natively with OpenGL on desktop, and scenes export to
standard GLB files for 3D tools.  One language, three outputs.

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

## Highlights

- **Browser-first graphics** — WebGL rendering, native OpenGL, GLB export
- **Static types with inference** — no annotations required, errors caught at compile time
- **Null safety** — `?? default` recovery, null propagation
- **Collections** — `vector<T>`, `sorted<T>`, `index<T>`, `hash<T>`
- **Parallel for** — `par(...)` distributes work across cores
- **Generators** — `yield` for lazy sequences
- **Native compilation** — compile to native binary or WebAssembly

See the full [language reference](https://jjstwerff.github.io/loft/) and
[what makes loft different](doc/claude/DESIGN.md) for the store-based memory
model and performance design.

---

## Install

```sh
git clone https://github.com/jjstwerff/loft
cd loft
cargo build --release
# binary is at target/release/loft
```

Or: `cargo install --git https://github.com/jjstwerff/loft --bin loft`

Pre-built binaries on the [Releases](https://github.com/jjstwerff/loft/releases) page.

---

## Graphics examples

23 progressive examples from hello-triangle to PBR with shadow mapping
in `lib/graphics/examples/`:

```sh
./11-scene-graph.loft          # exports a GLB file with lights
./08-basic-lighting.loft       # Phong lighting in a window
./20-textured-cube.loft        # procedural texture with text
```

See the full [example list](doc/claude/WEB_EXAMPLES.md#example-gallery).

---

## Get involved

Loft is looking for contributors.  The highest-impact areas:

- **WebGL backend** — making games run in the browser
- **Example games** — small playable demos that showcase the language
- **Game library** — input, sprites, tilemaps, collision, audio

See [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) for the workflow and
[PLANNING.md](doc/claude/PLANNING.md) for the backlog.

---

## Documentation

Full docs at **<https://jjstwerff.github.io/loft/>** — tutorial, API reference,
and [printable reference](https://jjstwerff.github.io/loft/loft-reference.pdf).

Build locally: `cargo run --bin gendoc`, then open `doc/index.html`.

---

## License

LGPL-3.0-or-later — see [LICENSE](LICENSE).
