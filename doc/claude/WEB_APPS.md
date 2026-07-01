<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Building web applications with loft

A task-oriented guide: how to take a `.loft` program and run it in a web
browser. Read this first — it tells you which build target to pick, exactly what
a browser program can and cannot do, and gives one recipe per kind of web app,
so you do not have to reverse-engineer the host surface yourself.

For the pipeline internals see [HTML_EXPORT.md](HTML_EXPORT.md); for the design
of how loft and JavaScript exchange data see [BROWSER_INTEROP.md](BROWSER_INTEROP.md).

---

## 1. Which build target? (the one that trips people up)

loft has two different WebAssembly builds. They are **not** interchangeable — pick
by where the program runs:

| You want to run… | Use | Output | Size (typical) |
|---|---|---|---|
| **in a browser** | `--html` | one self-contained `.html` (base64 WASM + JS bridge) | ~1.1 MB / ~330 KB gzipped |
| **headless / on a server** (via `wasmtime`) | `--native-wasm` | a `wasm32-wasip2` `.wasm` module | ~5.4 MB / ~1.5 MB gzipped |
| on the desktop | `--native` | a native binary | — |
| quick check, no compile | `--interpret` | runs in the tree-walking VM | — |

**For the browser, use `--html`.** It is the small, no-std engine built for the
web. `--native-wasm` is ~4× larger, links full `std` + WASI, needs an external
runtime (`wasmtime`) to run, and **only compiles** — it does not run the program
for you. Do not reach for it to target a browser.

## 2. Your first browser program

```loft
// hello.loft
fn main() {
  println("Hello from loft in the browser!");
}
```

```bash
loft --html hello.loft        # writes hello.html next to the source
```

Open `hello.html` in any modern browser (2020+). The text appears in a `<pre>`
box on the page. The file is fully self-contained — no server, no other files.

## 3. What a `--html` program can and cannot do (the host surface)

A `--html` program talks to the browser through a small, fixed set of **host
imports** — JavaScript functions the generated page provides. This surface is
**closed**: anything not on it is not available in the browser, even if it works
on native.

**Available today:**

- **Text output** — `print` / `println` go to a `<pre>` box (or the JS console).
- **Graphics + input** — the full WebGL2 canvas surface (`lib/graphics`): draw
  calls, shaders, textures, plus keyboard and mouse polling. This is what games
  use.
- **Audio** — raw PCM playback via the Web Audio API.
- **WebSocket** — a live network socket, through the `web` library (see §4b).

**NOT available in `--html`** (these silently do nothing or return empty):

- **No filesystem** — `file(...)` returns "not found".
- **No program arguments** — `arguments()` returns an empty list.
- **No HTTP client** — `web`'s `http_get` / `http_post` / … are **native-only**;
  they do not work in the browser. WebSocket is the only browser network
  transport today. (Let JavaScript do `fetch` and hand loft the bytes — §5.)
- **No generic "bytes in from JavaScript" yet** — see §5.

> This closed surface is the thing the first browser consumer had to discover by
> dumping the WASM imports. That is exactly what this section is for — you should
> never have to do that.

(There is a *second*, unrelated browser build — the IDE's `make wasm`
[wasm-bindgen] build — which *does* expose a filesystem/args bridge. That bridge
belongs to that build, **not** to `--html`. See [WASM.md](WASM.md) § Host Bridge
API, and do not assume it applies here.)

## 4. The three shapes of a loft web app

### 4a. A canvas / GL app (games, visualisations) — shipped, the main path

loft owns the frame loop; the browser renders each frame. Your source is
**identical** to the native version — the `gl_swap_buffers()` call yields to the
browser automatically (the "frame yield", see [HTML_EXPORT.md](HTML_EXPORT.md)).

```loft
use graphics::(create_window, clear, swap_buffers, poll_events, key_pressed);
fn main() {
  create_window(800, 600, "demo", true);
  while poll_events() {
    clear(0x101820);
    // draw your frame here
    swap_buffers();          // yields to the browser on --html, blocks ~16ms native
  }
}
```

```bash
loft --html game.loft
```

Worked example: `make game` builds `doc/brick-buster.html`. Input comes from
`key_pressed` / `mouse_x` / `mouse_y` / `mouse_button`.

### 4b. A WebSocket client (live data from a server) — via the `web` library

```bash
loft install web            # from the registry
```

```loft
use web::(ws_handler, try_recv, send, frame_yield);
fn main() {
  h = ws_handler("wss://example.com/socket");
  loop {
    msg = try_recv(h);       // "" when nothing is waiting
    if msg != "" { println("got: {msg}"); }
    send(h, "ping");
    frame_yield();           // hand the frame back so the socket can deliver — REQUIRED
  }
}
```

`frame_yield()` is load-bearing in the browser: a loop that never yields freezes
the page and never receives. On native it is a no-op. The `web` library ships the
browser bridge for this (its `[wasm.bridge]` + `wasm/host.js`); you do not write
any JavaScript.

### 4c. A headless compute kernel (JavaScript drives the UI, loft computes)

This is the "loft as a browser compute service" shape: JavaScript owns the page
and the network, calls a pure-loft kernel with some input, and shows the result.

- **Output — works today.** The kernel `print`s its result; the page reads it.
- **Input — the current gap.** There is no shipped generic "hand bytes from
  JavaScript to a running loft program" channel under `--html` yet. The designed
  primitive is `host_input()` — see
  [BROWSER_INTEROP.md § the input half](BROWSER_INTEROP.md). Until it ships, feed
  input by baking it into the program, or with a small per-app `[wasm.bridge]`
  crate (the WebSocket bridge in §4b is the template).

The kernel itself stays pure loft and runs unchanged on every target — you
develop and test it on `--interpret` / `--native` / `--native-wasm` (§7) and
only the browser leg waits on `host_input`.

## 5. Getting data in and out — the byte channel

The model is simple and one-directional in each direction: **the browser host
moves only opaque bytes** between loft and JavaScript; neither side reads the
other's data structures. Your loft library decides the meaning.

- **loft → JavaScript (out):** `println` is the shipped path; the page appends the
  text. A library can serialize any value to JSON or bytes and push it the same
  way.
- **JavaScript → loft (in):** `host_input()` (designed — §4c). Until it ships,
  browser input needs a per-app bridge.
- **Networking:** let JavaScript do it. JS `fetch(...)` is trivial; hand the
  response bytes to loft through the channel. loft stays pure compute and needs no
  HTTP in the browser at all.

## 6. Build, size, and serve

- `loft --html app.loft` → `app.html` (override with `loft --html app.loft out.html`).
- The `.html` is one self-contained file — serve it as a static asset, or open it
  directly. No bundler, no ES modules, no server runtime.
- `LOFT_HTML_TITLE="My App"` sets the page `<title>`.
- Install `wasm-opt` on your `PATH` for a smaller binary (the pipeline runs `-O1`
  automatically when present; it warns and skips if absent).
- **Cache gotcha:** if you switch a program between `--interpret` and `--html` and
  see stale behaviour, clear `~/.cache/loft` (`rm -rf ~/.cache/loft`). See
  [WASM.md](WASM.md).

## 7. Run the same source on every target (parity)

A pure-loft program (no `#native`, no I/O beyond the channel) compiles to the
interpreter, `--native`, `--native-wasm`, and `--html` from **one source**. Prove
they agree before shipping the browser leg — build and test the kernel on the
non-browser targets, where iteration is fast, and assert byte-identical output:

```bash
loft --interpret   kernel.loft < input.txt
loft --native-wasm kernel.wasm ; wasmtime run kernel.wasm < input.txt   # WASI stdin
```

A worked parity harness (interpreter vs `--native-wasm`, byte-for-byte) lives in
the `routing` consumer at `tools/kernel_headless_test.sh` — copy its shape.

## See also

- [HTML_EXPORT.md](HTML_EXPORT.md) — the `--html` pipeline internals: cdylib
  codegen, the WebGL2 bridge, the asyncify resume loop.
- [BROWSER_INTEROP.md](BROWSER_INTEROP.md) — the design of the loft↔JavaScript
  byte channel, the four tiers, and the `host_input` input primitive.
- [WASM.md](WASM.md) — the broader WASM runtime (the separate `make wasm`
  wasm-bindgen build, VFS, threading).
- The `web` library — HTTP (native) + WebSocket (native + browser) client.
