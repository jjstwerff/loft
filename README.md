
# loft — a language that reads like Python, ships like a distribution

[![CI](https://github.com/loft-lang/loft/actions/workflows/ci.yml/badge.svg)](https://github.com/loft-lang/loft/actions/workflows/ci.yml)
[![License: LGPL-3.0](https://img.shields.io/badge/License-LGPL_v3-blue.svg)](LICENSE)
[![Version](https://img.shields.io/github/v/release/loft-lang/loft?label=version&color=blue)](https://github.com/loft-lang/loft/releases)
[![Libraries](https://img.shields.io/badge/registry-42_libraries-brightgreen)](https://loft-lang.org/loft/)
[![Gallery](https://img.shields.io/badge/gallery-live-brightgreen)](https://loft-lang.org/loft/gallery.html)

[**▶ Try it in the browser**](https://loft-lang.org/loft/playground.html) · [**📘 Docs**](https://loft-lang.org/loft/) · [**🎮 Play Brick Buster**](https://loft-lang.org/loft/brick-buster.html) · [**🖼 Gallery**](https://loft-lang.org/loft/gallery.html)

---

**loft is a statically typed language with almost no type annotations.** The same source
compiles to a native binary through rustc, to WebAssembly for the browser, or runs on a
tree-walking interpreter for instant startup. It ships as a **distribution**: the compiler,
a standard library, and **42 libraries** in a registry — graphics and 3D, an HTTP server and
client, cryptography, text engines, geometry — each one itself written in loft, so they read,
debug and ship like your own code.

```loft
struct Point { x: float, y: float }

fn distance(a: Point, b: Point) -> float {
    dx = a.x - b.x;
    dy = a.y - b.y;
    sqrt(dx * dx + dy * dy) ?? 0.0
}

fn main() {
    p1 = Point { x: 0.0, y: 0.0 };
    p2 = Point { x: 3.0, y: 4.0 };
    println("distance: {distance(p1, p2)}")
}
```

```
$ loft hello.loft
distance: 5
```

`vector<T>`, `sorted<T>`, `index<T>` and `hash<T>` collections; bounded generics and
interfaces; `par(...)` for parallel loops; `yield` for generators; `match` for pattern
matching; `?? default` for null safety; format strings like Rust's. Types are inferred, and
mistakes are caught at compile time.

## Getting started

```sh
# one-line install (requires a Rust toolchain)
cargo install --git https://github.com/loft-lang/loft --bin loft

# or clone + build
git clone https://github.com/loft-lang/loft
cd loft
cargo build --release          # binary at target/release/loft
./target/release/loft examples/hello.loft
```

`make install-user` installs to `~/.local/bin` and `~/.local/share/loft` without root, and
tells you if `~/.local/bin` is not yet on your `PATH`. `make install` (system-wide
`/usr/local`) elevates only when the prefix is not writable. Pre-built binaries are on the
[Releases](https://github.com/loft-lang/loft/releases) page.

Then:

- **[Learn loft in 30 minutes](doc/learn-loft.md)** — a guided tour with runnable code for
  every concept.
- **[`examples/`](examples)** — seven self-contained programs: hello-world, fibonacci,
  fizzbuzz, structs, collections, pattern matching, file I/O.
- **[VS Code extension](editors/vscode)** — highlighting and snippets. The
  [TextMate grammar](syntaxes/loft.tmLanguage.json) works in Sublime Text and anything else
  that reads one.

```sh
loft prog.loft                 # compile to a native binary and run it
loft --interpret prog.loft     # skip the compile, start now
loft --html prog.loft          # a directory of static files you can host
loft repl                      # an interactive session
loft test                      # run a package's tests
loft debug prog.loft:12        # stop at line 12, read and edit the live frame
```

## The distribution

`loft install <name>` pulls a library from the registry into `~/.loft/lib/`. Every one of
them is written in loft — no C, no FFI glue — so they read, debug and ship like your own
code. Each is tested to produce identical results on the interpreter and the native backend,
and is cross-built for WebAssembly unless it needs a device the browser does not have — in
which case it says so, and CI prints the reason.

<!-- BEGIN GENERATED: registry-summary -->

| area | libraries |
|---|---|
| **graphics & 3D** | `graphics` · `stage` · `imaging` · `mesh3d` · `glb` · `text2d` · `drawing` · `gridmesh` · `lavition_ui` |
| **games** | `input` · `fixstep` · `tween` · `assets` · `game_protocol` |
| **geometry** | `shapes` · `hex_field` · `hex_shape` · `hex_form` · `hex_body` · `hex_edge` · and nine more `hex_*` |
| **networking** | `server` (HTTP + WebSocket) · `web` (client) · `ssh` |
| **text** | `regex` · `markdown` · `html` · `zttext` |
| **data & crypto** | `crypto` · `cbor` · `arguments` · `random` · `time` |

<!-- END GENERATED: registry-summary -->

```sh
loft api --registry            # the whole installable catalogue, with descriptions
loft api graphics              # one library's public API, with its documentation
loft doc graphics              # render that library's docs as HTML
loft install graphics          # …then `use graphics;` in your program
```

Bounded generics and interfaces keep the libraries type-safe without ceremony, and `loft test`
runs any package's suite.

## What it looks like when it runs

<p align="center">
  <a href="https://loft-lang.org/loft/brick-buster.html">
    <img src="doc/images/hero-brick-buster.png" alt="Brick Buster — a complete arcade game written in loft" width="640">
  </a>
</p>

**A complete arcade game** — paddle, ball, coloured bricks, seven powerups, chiptune music,
hand-designed levels, pause, game-over, an animated paddle explosion — running in your
browser. Click, press **Space**, play. No install, no sign-up, no download.

It is one file — [read all 1,849 lines](tools/brick-buster/25-brick-buster.loft). No engine and no
framework — that file *is* the game. Its sprite sheet is art, not code, drawn once by
[a build step](tools/brick-buster/pack_atlas.loft) into a content pack the game reads and the
browser page carries inside itself.

The [**playground**](https://loft-lang.org/loft/playground.html) runs loft in your browser
with nothing installed, and the [**gallery**](https://loft-lang.org/loft/gallery.html) is the
graphics stack rendering live.

## Maturity

loft uses calendar versions (`YYYY.M.N`) and is stabilising toward **contract 1** — the point
at which compatibility becomes absolute and no working program ever breaks again. Until then
the rule is narrower and written down: what a released version promises is kept, and a
breaking change needs a documented reason and a migration
([COMPATIBILITY.md](doc/claude/COMPATIBILITY.md)).

What that means in practice today:

- **The language and the standard library are stable in normal use** — thousands of tests run
  every change on both backends, and the 42 published libraries are recompiled against every
  change that reaches `main`.
- **The libraries version independently** and declare their own compatibility floors.
- **The edges are still moving**: some diagnostics, some newer library APIs, and the
  parts marked *open work* in [STDLIB.md](doc/claude/STDLIB.md).

Open issues are tracked publicly, and every fix records whether it strained the contract or
merely honoured it.

## Documentation

Full reference, tutorial, feature catalogue and a printable PDF at
<https://loft-lang.org/loft/>.

- **[Language reference](https://loft-lang.org/loft/)** — 40 topic pages. On the 37 numbered
  ones every code example is executed as a test on both backends, so what you read is what
  runs; the three comparison and performance pages are hand-written and checked by hand.
- **[Feature catalogue](https://loft-lang.org/loft/33-features.html)** — 117 entries: what
  each feature is, how it helps, and a runnable example.
- **`loft doc <package>`** — render any installed library's guide and API reference as HTML.

Build the site locally with `make wasm` (playground + gallery) then `cargo run --bin gendoc`,
and open `doc/index.html`.

## How loft is built — and why it cannot be orphaned

loft is developed almost entirely by AI coding agents, steered by one person who put
**documentation and tooling above writing code**. So everything needed to work on loft is in
this public repository: the full source, ~115,000 lines of documentation, and ten
**executable skills** that teach an agent *how* to fix a bug, change code generation, or ship
a library. Point any capable coding agent at the repo, let it load the skills, and it can
continue the work — investigate, fix, verify on both backends, and land through `make ci`.
The project's knowledge lives in the repository, not in a founder, so its **bus factor is
effectively zero**:
[How loft develops itself, and how to prove it yourself](doc/claude/BUS_FACTOR.md).

## Contributing

Highest-impact areas today:

- **Library guides** — most of the 42 libraries have a solid API and no introduction. One
  runnable getting-started page is the highest-value thing you can write
  ([USER_DOCS.md](doc/claude/USER_DOCS.md)).
- **Example programs** — small playable games and demos that show the language off.
- **Libraries** — publish a package; each is plain loft, so the bar is low
  ([LIBRARY_AUTHORING.md](doc/claude/LIBRARY_AUTHORING.md)).
- **WebGL backend** — closing the last gaps between native and browser rendering.

[DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) has the workflow; [ROADMAP.md](doc/claude/ROADMAP.md)
has what is planned.

## License

LGPL-3.0-or-later — see [LICENSE](LICENSE).
