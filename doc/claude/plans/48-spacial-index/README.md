# Plan 45 — `spacial<T[x,y]>` / `spacial<T[x,y,z]>`: a Morton/Z-order radix spatial index

**Status:** Future (planned). Completes the long-reserved `spacial<T>`
keyed-collection (gated today with *"spacial<T> is planned for 1.1+; until
then use sorted<T> or index<T>"*).

## Why

Games need **proximity queries** — "find the near mobs from any mob's
perspective" (aggro/threat, interest management, flocking, broad-phase
collision) — over a chunked world.  Two dimensionalities, chosen by the
number of coordinate key fields:

- **`spacial<Mob[x,y]>`** — flat / top-down worlds (the common case).
- **`spacial<Mob[x,y,z]>`** — worlds with a *considerable vertical
  component* (towers, caves, flight).

The same Morton machinery serves both; 2 or 3 interleaved axes.

**loft already has the scaffolding for exactly this, both unfinished:**
- **`spacial<T[…]>`** (`Parts::Spacial(content, coord_fields)`,
  `src/database/types.rs::spacial`) — the type registers and takes N
  coordinate key fields, but every operation
  (`insert_record`/`find`/`remove`/`copy_claims`) is
  `panic!("Not implemented")`, and the parser gates it with the 1.1+
  diagnostic.
- **`src/radix_tree.rs`** — header: *"A radix tree implementation.  This is
  especially useful for spacial indexes."*  A binary radix tree (FALSE/TRUE
  bit branching, `MAX_DEPTH = 64`) whose `rtree_find(store, tree, key:
  Fn(u32) -> bool)` consumes the key **one bit at a time** — purpose-built
  for an interleaved Morton key.  `rtree_init/first/last/find/insert` exist;
  `RadixIter::next` (the bidirectional walk), `remove`, and the wiring are
  stubbed.

So this is **completion**, not greenfield.

## Design

- **Granularity** — quantize each coordinate to an integer at a chosen
  world resolution (cell size), configured per `spacial` type (sensible
  default, e.g. 1 world-unit).  Coarser cells pack more entities per code
  (bucket within a code); finer cells widen the key range.
- **Morton / Z-order key** — bit-interleave the N quantized axes into one
  key.  2D: interleave x,y (≤32 bits each → ≤64-bit).  3D: interleave x,y,z
  (≤21 bits each → ≤63-bit, fits `MAX_DEPTH 64`).  The radix tree's
  bit-key closure yields the interleaved bits on demand — no full key
  materialisation.
- **Radix tree storage** — entities keyed by Morton code.  Nearby codes
  share long prefixes (Z-order locality) → spatially close entities are
  tree-adjacent, compact, and **cache-resident** (the same locality lever as
  gridmesh).  Multiple entities per cell → a small bucket at a code.
- **Proximity iterator (the "loop left and right" core)** — from a query
  point's Morton code, walk **predecessor + successor simultaneously**,
  emitting entities in increasing Morton distance ≈ spatial proximity.
  - *Approximate near* (aggro/interest): the raw walk — fast, good-enough.
  - *Exact radius / k-nearest*: the walk yields candidates; verify with
    true distance (Euclidean or Chebyshev) and keep expanding the bit-window
    until the candidate ring's minimum-possible Morton bound exceeds the
    current k-th true distance (standard Morton-kNN termination).  Build the
    verification into the iterator so callers get correctness for free.

## Caveats (design around)

- **Z-order ≠ Euclidean.**  The curve has discontinuities at quadrant
  boundaries — Morton-adjacent is *usually* spatially close, not always.
  Approximate `near` tolerates it; `within`/`nearest` must distance-verify.
- **Granularity trade-off** — resolution vs key range vs entities-per-code.
  Support a per-code bucket so multiple entities can share a cell.

## Architecture fit (per-chunk vs global)

The world is a sparse `hash<Chunk[cx,cy,cz]>` of fixed 32×32 chunks (lib-plan
19).  Two placements, both expressible (it's just *where* the `spacial`
field lives):
- **Per-chunk index** (recommended) — a small `spacial` per chunk keeps the
  working set cache-resident; a cross-chunk `near` query touches the ≤9 (2D)
  / ≤27 (3D) neighbour chunks.  Best when local queries dominate.
- **Global index** — one tree; simplest for sparse worlds / large variable
  radii; less locality.

## Phases

| # | Scope |
|---|---|
| **S1** | Complete `src/radix_tree.rs`: bidirectional `RadixIter` (`next`/`prev` from an arbitrary position), `insert`, `remove`, the interleaved Morton bit-key.  Rust unit tests (2D + 3D keys, insert/find/walk/remove). |
| **S2** | Wire `spacial<T[…]>` to the radix tree — implement the four panicking ops + `copy_claims` (`structures.rs`/`search.rs`/`allocation.rs`); Morton-encode the coord key fields; lift the parser's "planned 1.1+" gate.  (This also finally implements the keyed kind @P295/@P305/@P309 had to exclude.) |
| **S3** | Proximity API (stdlib, `default/*.loft`): `coll.near(x, y[, z])` (nearest-first iterator), `coll.within(x, y[, z], radius)`, `coll.nearest(x, y[, z], k)` — with distance verification. |
| **S4** | Consumer validation — a "near mobs from a mob's perspective" demo in the moros/audience world; cross-mode; cache-locality + correctness vs brute-force measurement. |

## Critical files
- `src/radix_tree.rs` — the tree + bidirectional iterator + Morton bit-key.
- `src/database/{structures.rs,search.rs,allocation.rs}` — the `spacial` ops (currently `panic!`).
- `src/database/types.rs` — `spacial()` registration + the Morton key; keyed-kind handling.
- `src/parser/*` — lift the `spacial<T>` 1.1+ diagnostic; keyed-field handling (mirror hash/sorted/index from @P305/@P307/@P308).
- `default/*.loft` — the `near`/`within`/`nearest` API.
- Reuse: `src/keys.rs` (key extraction), the keyed-collection deep-copy machinery, the gridmesh chunk model (per-chunk indexing).

## Verification
- Rust unit tests for `radix_tree` (insert / find / bidirectional walk /
  remove; 2D + 3D Morton keys).
- loft cross-mode: insert N entities; `near(p)` returns them increasing in
  distance; `within(p,r)` and `nearest(p,k)` match a brute-force oracle on
  small sets; both backends.
- Memory/cache: `store_memory()` bounded; per-chunk index fits L1/L2.

Acceptance: `spacial<T[x,y]>` and `spacial<T[x,y,z]>` work; `near` (approx)
+ `within`/`nearest` (distance-verified exact) correct; the 1.1+ diagnostic
lifted; both backends green; reuses `radix_tree.rs` + `spacial<T>` rather
than inventing new structure.
