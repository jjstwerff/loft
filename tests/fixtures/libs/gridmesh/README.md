<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# gridmesh — chunk-local mesh-generation primitives for loft

Pure-loft library.  Reusable building blocks for chunk-local,
bounded-extent grid→mesh generation: spatial-index neighbour
queries, neighbour gather, bounded mesh accumulation keyed by
owning cell, dirty-region / per-chunk rebuild.

Used by world-building algorithms (wall placement, edge
rounding, surface generation) that run as local routines over
a limited set of world chunks and produce meshes that don't
extend much outside their chunk.

## Install

```sh
loft install gridmesh
```

## API surface

See [src/gridmesh.loft](src/gridmesh.loft) for the full type
+ function reference.  Core abstractions:

- Spatial index for chunk-local neighbour lookup.
- Mesh accumulator keyed by owning cell.
- Dirty-region tracking for incremental rebuilds.

## Provenance

Extracted from the loft monorepo's `lib/gridmesh/` 2026-05-24
as part of [@PLAN12](https://github.com/jjstwerff/loft/blob/main/doc/claude/lib_plans/12-library-extraction/README.md)
Phase 5.  Originally prototyped for the audience-generative-art
crystal demo ([@PLN6](https://github.com/jjstwerff/loft/blob/main/doc/claude/plans/future/6-audience-generative-art/README.md)).
