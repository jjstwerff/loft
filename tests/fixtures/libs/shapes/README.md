<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# shapes — 2D shape drawing + collision detection for loft

Pure-loft library.  Defines `Rect`, `Circle`, and collision
helpers (overlap tests, containment, distance).

## Install

```sh
loft install shapes
```

## API surface

See [src/shapes.loft](src/shapes.loft).  Major primitives:

- `Rect`, `Circle` struct types.
- `rects_overlap(a, b) -> boolean`
- `circles_overlap(a, b) -> boolean`
- `rect_contains_point(rect, x, y) -> boolean`

## Provenance

Extracted from the loft monorepo's `lib/shapes/` 2026-05-24
as part of [@PLAN12](https://github.com/jjstwerff/loft/blob/main/doc/claude/lib_plans/12-library-extraction/README.md)
Phase 5 (loft-libs-graphics chunk).
