# lib-plan 19 — `gridmesh`: chunk-local, bounded-extent grid→mesh primitives

**Status:** ACTIVE (promoted from `future/` to a top-level `lib_plans/`
slot 2026-05-21). Phase A landed; Phase B in progress, Phase C to follow.

**Full implementation design:** [DESIGN.md](DESIGN.md) — concrete types,
the rule contract, the pipeline API, the crystal-consumer mapping, ordered
build steps (gridmesh B1-B3 → crystal C1-C3), and verification.

## Why

The crystal routine (`lib/audience_crystal`) is the prototype for a CLASS
of world-building algorithms — wall placement, edge rounding, surface
generation — that must run as **local routines over a limited set of
world chunks** and produce **meshes that don't extend much outside their
chunk**, so chunks build / update / cull independently. Two consumers
need the identical pipeline but share no code:

- `lib/audience_crystal` — generative-art prototype (offset/pointy hex);
  already O(N) + memory-bounded (P5/P6).
- `lib/moros_render` — the real world: `build_hex_meshes` meshes the
  WHOLE world monolithically into per-material buffers, with no per-chunk
  meshes, no halo (seams ignored), flat per-hex geometry, no culling, no
  dirty tracking.

`gridmesh` houses the shared pipeline as a **toolkit of primitives, not a
framework**: spatial-index neighbour queries → per-cell pattern
classification → bounded geometry emission keyed by owning cell →
dirty-region / per-chunk rebuild. Each consumer supplies only its
per-cell RULE. Mostly extraction + generalization of existing code, not
greenfield.

## Reused primitives (extraction sources)
- `lib/moros_map`: `chunk_idx_32`/`hex_idx_32` (global↔chunk-local),
  `hex_distance`, `map_get_hex` (free cross-chunk halo reads), 32×32
  `Chunk`/`Hex`.
- `lib/audience_crystal`: `enc_coord`, `axial_dq/dr`, `step_x/step_y`,
  `idx_at`, `nbr_count_idx`, `CellRef`, the `cidx` spatial-index pattern,
  the `CrystalMesh` SoA layout + per-segment `cell_ix`.
- `lib/graphics`: `loft_gl_upload_mesh`/`gl_upload_vertices` (f32 VBO),
  `Mesh`/`Vertex`.
- Substrate: keyed collections (efficient via P5/P6), amortised vectors,
  narrow element types (M1: `single`/`u8`/`u16` for compact meshes).

## Phases

| Phase | Scope | Status |
|---|---|---|
| **A** | Extract coord + spatial-index primitives (`CellRef`, `enc_coord`, `build_index`, `idx_at`, `step_x/step_y`, `axial_dq/dr`, `nbr_count_idx`) into `lib/gridmesh`; re-point `audience_crystal` + the projector onto them. `build_index` replaces the inline @P300 workaround (now fixed). | ✅ **Done** — crystal output + memory byte-identical on both backends (block/100 = 2379 segs; block-500 = 12 738 segs / 2.83 MB / 9 free-blocks). |
| **B** | Chunking + halo + bounded extent + dirty rebuild: `build_chunk(layout, field, cx,cz,cy, rule) -> Mesh` reading a ≤k-ring halo, emitting geometry owned by in-chunk cells bounded to the chunk; per-chunk/dirty-cell incremental rebuild (O(dirty) compute + O(N) copy). A `HexLayout` adapter (axial flat-top for moros, offset pointy-top for crystal). Carry M1 narrow types into the mesh accumulator. | Open |
| **C** | moros consumer: `build_chunk_mesh(map, cx,cy,cz)` over `gridmesh` (replacing the global `build_hex_meshes` path incrementally) with a moros RULE — surface + wall placement + edge rounding from the `Hex` neighbour pattern (the crystal technique applied to real world geometry); per-chunk VBOs + dirty IDs + frustum culling. Own sub-plan; the payoff. | Open |

## Parallelism — baked in from Phase B (chunks are embarrassingly parallel)

Chunks are independent (bounded meshes, read-only halo), so building many
chunk meshes is a parallel map.  loft's `par(...)` for-loop clause
supports exactly this and is designed in from the start:

```loft
for ci in chunk_inputs par(cm = build_chunk_mesh(ci, rule), N) {
  meshes += [cm];   // per-chunk Mesh, collected in order
}
```
- **Struct returns work** — `par(...)` deep-copies a worker-built struct
  (the chunk `Mesh`) inline into the result vector (THREADING.md
  "Supported return types … `struct`/reference"); the rule may be passed
  as a forwarded **context arg**.
- **Workers get locked read-only store copies** (`clone_for_worker`), so
  reading the shared field/halo in parallel is safe; the only rule is
  *workers may not write shared state* — chunk-local generation never
  does (each builds its OWN bounded mesh).

**Design invariants that make parallelism cheap AND keep meshes bounded
(the same discipline):**
1. `build_chunk_mesh(input, rule) -> Mesh` is a PURE function — no shared
   mutation — so wrapping the chunk loop in `par(...)` is trivial.
2. The worker INPUT is a COMPACT, self-contained `ChunkInput` = the
   chunk's cells + a ≤k-ring halo apron, extracted BEFORE the par loop —
   NOT the whole map.  This keeps `clone_for_worker`'s per-worker store
   copy small (it clones in-use stores per worker), and it structurally
   guarantees a chunk's mesh can only depend on its chunk + halo → bounded
   extent by construction.  Locality and parallelism reinforce each other.
3. Dirty-region rebuild = a `par(...)` map over the dirty chunks' inputs.

### Dirty tracking — a wired-in dirty INDEX on the chunk container

The chunk-holding structure carries a **first-class, wired-in index of
dirty chunks** — a keyed SET of dirty chunk coords
(`dirty: hash<ChunkCoord[ck]>`), maintained as part of the structure, NOT
a per-chunk bool that has to be scanned for, and NOT a plain append
vector that would accumulate duplicates.  This gives O(dirty) enumeration
directly, with dedup by construction:

- **Marker:** marking a chunk dirty is an idempotent **set-insert** into
  the wired-in dirty index — many edits to the same chunk coalesce to one
  entry.  (A per-chunk `built_gen` vs the field's edit generation can
  still validate "is this chunk's mesh current?", but the dirty index is
  the canonical "what to rebuild" — no scan of all chunks.)
- **Edit → propagation:** an edit at cell `(q,r)` inserts its containing
  chunk into the dirty index AND, when `(q,r)` is within the halo radius
  of a chunk border, the adjacent chunk(s) too (their mesh reads `(q,r)`
  via the halo).  This border coupling is the only cross-chunk dependency.
- **Rebuild:** iterate the dirty index DIRECTLY (O(dirty)) to build the
  `par(...)` work-list (a transient `vector<ChunkInput>` — extract cells +
  halo per dirty chunk), run the parallel map, replace those chunks'
  meshes/VBOs, then CLEAR the dirty index.

This belongs in `gridmesh`'s chunk-field abstraction so every consumer
(moros, crystal) gets it; moros's `Map` (which has no dirty tracking
today) gains it via the field.  The only vector is the short-lived par
work-list, collected by iterating the dirty index — so the
`par`-over-any-iterator extension is still not a prerequisite (vector-`par`
handles the work-list today).  Once par-over-keyed lands (THREADING.md
materialise path) you could `par` the dirty index directly and skip the
collect — a convenience, not a requirement.

Caveats: `par(...)` input must be a `vector<T>` (chunk inputs are a vector
— fine); tune `N` to cores (chunks ≫ cores → good balance); the @P229
worker-stack-snapshot flake is Linux-clean (Windows half still open).

## Phase A notes
- New: `lib/gridmesh/{loft.toml,src/gridmesh.loft}`.
- `audience_crystal/loft.toml` depends on `gridmesh`; `crystal.loft`
  `use gridmesh::*`, deleted its local copies, and builds the index via
  `gridmesh::build_index(snap.xs, snap.ys)`.
- `tools/audience-demo/projector.loft` `use gridmesh;` and builds `pidx`
  via `gridmesh::build_index` (was `audience_crystal::CellRef`/`enc_coord`
  inline).
- Verified: `crystal_stress` segment counts unchanged interp + native;
  projector compiles native; `store_memory` profile unchanged.

## Verification
```bash
./target/release/loft --interpret --lib lib tools/audience-demo/crystal_stress.loft   # segs unchanged
./target/release/loft --native    --lib lib tools/audience-demo/crystal_stress.loft   # same, both backends
cargo test --release --test wrap --test native --test issues --test leak --test doc_hygiene
```
