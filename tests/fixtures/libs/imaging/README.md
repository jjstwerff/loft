<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# imaging — PNG load/save + pixel manipulation for loft

## Install

```sh
loft install imaging
```

## Surface

- `Image` / `Pixel` types.
- `load_png(path) -> Image` / `save_png(img, path) -> boolean`.
- Format helpers (`format_image`, etc.).

Native code (cdylib `loft_imaging`) backs PNG codec via the `png`
crate.

## Stage A constraints

This chunk-resident release ships **interpreter + native** targets.
The browser-WASM bridge (`--html`) is **not** included — it depends
on an unpublished `loft-host-ffi` crate; deferred until that crate
ships on crates.io.  No `--html` consumer currently uses imaging,
so this is a deferred follow-up, not a regression.

## Provenance

Extracted from the loft monorepo's `lib/imaging/` 2026-05-31 as part
of [@PLAN12](https://github.com/jjstwerff/loft/blob/main/doc/claude/lib_plans/12-library-extraction/README.md)
Phase 5b.
