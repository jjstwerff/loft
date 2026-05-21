# lib-plan 19 — `gridmesh`: chunk-local, bounded-extent grid→mesh primitives

**Status:** Phase A landed (2026-05-21). Phases B–C open.

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
