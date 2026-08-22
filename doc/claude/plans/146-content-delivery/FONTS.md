<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN146 — Fonts: reusing one the browser has, and bringing one it does not

Reference for arc F phases **F5** (the declaration) and **F6** (the readiness ordering),
both shipped — their gates are [F5.md](F5.md) and [F6.md](F6.md). The plan is in
[README.md](README.md).


`--html` ships **no font bytes of its own**, and for the reuse case it does not need to:
`gl_load_font("X.ttf")` never opens a file. `familyFor()` (`doc/loft-gl-wasm.js`) turns
the path's base name into a CSS font list — the requested family first, then a generic
guessed from the name (`mono|courier|consol|code` → `monospace`, `serif` → `serif`, else
`sans-serif`) — and the browser's own `fillText` produces the coverage bitmap the desktop
shader expects. Name a family the browser has and **nothing is downloaded**.

Bringing one it does not have is a declaration in `loft.toml`. Three sources, one shape:

| Source | Declared | Browser | Native / `--native-wasm` |
|---|---|---|---|
| a family the browser already has | `family` (+ `native`) | nothing shipped, nothing fetched | the TTF beside the game |
| our own file server | `url = "…"` | `@font-face { src: url(…) }`, or the WOFF2 packed in the asset store and range-read like every other asset | the same store |
| Google Fonts, or any CDN | `stylesheet = "…"` | the provider's stylesheet `<link>`; zero bytes of ours | the TTF beside the game |

```toml
[[font]]
family = "PressStart2P"
native = "fonts/PressStart2P.ttf"
url    = "fonts/PressStart2P.woff2"
```

A library declares fonts the same way, and its declarations reach the page by the same
route as `[wasm.bridge] host_js`. `src/html_fonts.rs` validates them and emits both the
`<head>` fragment and the boot await.

## The two mechanics that decide whether it works

The declared `font-family` must **equal** the base name the program passes to
`gl_load_font` — that string is the key `familyFor` builds, and drift is a silent
fallback, never an error. `native` is that path, so the two are checked against each
other **at build time** and a mismatch is refused before the wasm compile (F5).

And the page must await `document.fonts.load('16px "<family>"')` **before** `loft_start`:
a webfont is still in flight while the first frame draws, and `familyFor`'s answer is
cached per handle, so an early `gl_load_font` paints in the fallback with nothing on
stderr (F6). `--html` emits that await for every declared family.

## `document.fonts.check` is not the question you want to ask

It answers **true** for a family nothing declares at all, and **false** for an
`@font-face` that is still loading. So it cannot say whether a page has a font — and
using it that way had `familyFor` taking its exact-font branch for every page except the
one that had brought a font, then caching the generic it fell to. Measured in headless
Chromium; the whole table is in [F5.md](F5.md).

To ask whether a family actually resolved, **measure it against two generics**: a family
the browser has overrides both, so `"X", monospace` and `"X", sans-serif` come out the
same width; one it does not have follows each and they differ. `gl_load_font` does this
once per font and `console.warn`s when the answer is no, naming the family and the
generic that will draw instead. `globalThis.loftFonts` records every resolution, which is
what the browser gate reads.

A remote font is a third-party dependency: offline, or with the CDN blocked, the chain
degrades to the generic rather than failing — which is right, and is why the native
source stays declared beside the browser one. What was wrong was doing it quietly.

## The escape hatch that still works

A library can carry its own `@font-face` and `fonts.load` await in `[wasm.bridge]
host_js`, exactly as before. `[[font]]` makes the common case declarative so a game
writes no JS and the ordering is automatic rather than remembered.
