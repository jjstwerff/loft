<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN144 — Fonts: reusing one the browser has, and bringing one it does not

Reference for arc F phases **F5** (the declaration) and **F6** (the readiness ordering).
The plan and its phase gates are in [README.md](README.md).


`--html` ships **no font bytes today**, and for the reuse case it does not need to:
`gl_load_font("X.ttf")` never opens a file. `familyFor()` (`doc/loft-gl-wasm.js:113`)
resolves the path's base name to a CSS family — one the page registered wins
(`document.fonts.check`), else a name heuristic picks `monospace` / `serif`, else
`sans-serif` — and the browser's own `fillText` produces the coverage bitmap the desktop
shader expects. Name a family the browser has and **nothing is downloaded**.

What is missing is the ability to *bring* one. Three sources, one declaration:

| Source | Browser | Native / `--native-wasm` |
|---|---|---|
| a family the browser already has | nothing shipped, nothing fetched | the TTF beside the game |
| our own file server | `@font-face { src: url(…) }`, or the WOFF2 packed in the asset store and range-read like every other asset | the same store |
| Google Fonts, or any CDN | the provider's stylesheet `<link>`; zero bytes of ours | the TTF beside the game |

Two mechanics decide whether it works at all, and F5/F6 gate them. The declared
`font-family` must **equal** the base name the program passes to `gl_load_font` — that
string is the key `familyFor` builds, and drift is a silent fallback, never an error. And
the page must await `document.fonts.load('16px "<family>"')` **before** `loft_start`:
`check` is synchronous and answers *false* while a webfont loads, and `familyFor`'s answer
is cached per handle, so one early call locks that handle to `sans-serif` for good, with
nothing on stderr.

A remote font is a third-party dependency: offline, or with the CDN blocked, the chain
degrades to `sans-serif` rather than failing — which is right, and is why the native source
stays declared beside the browser one.

A library can do all of this **today** by carrying its `@font-face` and the `fonts.load`
await in `[wasm.bridge] host_js`. F5/F6 make it declarative, so a game writes no JS and the
ordering gate is automatic rather than remembered.

