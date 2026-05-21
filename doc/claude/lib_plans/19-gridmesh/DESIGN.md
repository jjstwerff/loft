# gridmesh — full design (Phase B + crystal consumer)

Status: design for implementation.  Phase A (coord + spatial-index
primitives) already landed and is consumed by `audience_crystal` +
projector.  This document specifies the rest: the chunk pipeline (halo,
bounded extent, wired-in dirty index, parallel build) and how the crystal
routines are re-expressed as a gridmesh consumer.

## 1. Principles

- **Toolkit, not framework.**  gridmesh owns *where* cells are (coords,
  spatial index, chunk partition, dirty index) and the *pipeline* (chunk
  input extraction, parallel build, incremental rebuild).  The consumer
  owns *what* cells are (payload: crystal colours / moros `Hex`) and the
  *mesh* it emits.  The per-cell RULE is a consumer function.
- **Chunk-local, bounded extent.**  A rule for chunk C sees only C's cells
  + a ≤k-ring halo and emits geometry owned by C's cells — so a chunk
  mesh is spatially bounded by construction.
- **Materialise only when needed.**  Native partitions where possible; the
  only vector built is the transient par work-list of dirty chunk inputs.
- **Parallel from the start.**  The chunk build is a `par(...)` map of the
  consumer's `fn(ChunkInput) -> Mesh` over dirty chunk inputs (par
  supports fn-ref workers + struct returns + context args — validated).
- **Ordering + determinism.**  Per-chunk results are independent; within a
  chunk the rule is sequential and deterministic.

## 2. The where/what split

gridmesh is **payload- and mesh-agnostic**.  It tracks cell COORDS and
their chunk membership + dirtiness; it never sees colours, heights, or
vertices.  The consumer keeps its payload arrays and its per-chunk meshes,
and indexes into them by the cell indices gridmesh hands it.

```
gridmesh owns:                      consumer owns:
  cell coords (xs, ys)                payload (e.g. colors[], or Hex[])
  spatial index (coord -> cell ix)    per-chunk meshes (CrystalMesh / TriMesh)
  chunk partition (cell -> chunk)     the per-cell RULE
  wired-in dirty-chunk index          the (trivial) par build loop
  halo / chunk-input extraction
```

## 3. Core types (illustrative loft signatures)

```loft
// Hex coordinate layout — crystal is offset/pointy, moros is axial/flat.
// Phase A's step_x/step_y/enc_coord are the offset variant; add the axial
// variant and select via this tag.  (Until moros consumes gridmesh, only
// OffsetPointy is implemented; AxialFlat is added in Phase C.)
pub enum HexLayout { OffsetPointy, AxialFlat }

// CellRef + enc_coord + build_index + idx_at + step_x/y + axial_dq/dr +
// nbr_count_idx — ALREADY in gridmesh (Phase A).

// A chunk key (chunk-space coord).  chunk_of(x,y) = (x>>shift, y>>shift)
// or chunk_idx(v) per the moros 32-grid helper.
pub struct ChunkKey { ck: integer not null, cx: integer not null, cy: integer not null }

// The field: WHERE cells are.  Placement-ordered cell coords + the spatial
// index + chunk size + the WIRED-IN dirty-chunk index.  Payload-agnostic.
pub struct ChunkField {
  layout:     integer,                  // HexLayout tag
  xs:         vector<integer>,          // cell coords (index 0 = oldest)
  ys:         vector<integer>,
  cidx:       hash<CellRef[ck]>,        // coord -> cell index (Phase A)
  chunk_shift: integer,                 // chunk = coord >> chunk_shift (1<<shift cells/axis)
  dirty:      hash<ChunkKey[ck]>,       // WIRED-IN dirty-chunk index (set; dedup)
}

// What a RULE needs for ONE chunk: the in-chunk cell indices, the halo
// cell indices (≤k-ring border neighbours, possibly cross-chunk), and the
// shared read-only index for neighbour stepping.  COMPACT + self-contained
// (so par's clone_for_worker copies little, and the mesh is bounded).
pub struct ChunkInput {
  key:       ChunkKey,
  cell_ixs:  vector<integer>,           // cells owned by this chunk (emit for these)
  halo_ixs:  vector<integer>,           // border-neighbour cells (read-only context)
  cidx:      hash<CellRef[ck]>,         // shared index (read-only in the par region)
}
```

The consumer's MESH type is its own (gridmesh ships two ready accumulators
but the driver is agnostic):
- **`SegMesh`** (line/segment consumers — crystal): the current
  `CrystalMesh` shape lifted into gridmesh — `kinds: vector<u8>`,
  `x0s..z1s: vector<single>` (M1 narrow), `colors: vector<u8>`,
  `cell_ix: vector<integer>` — + `emit_segment(m, kind, x0,y0,z0,
  x1,y1,z1, color, owner)` and a bounded-extent debug check.
- **`TriMesh`** (surface consumers — moros, Phase C): `pos/normal/uv/
  color` (narrow) + `cell_ix` + `emit_tri`/`emit_quad`.

## 4. The rule contract

A consumer rule is **`fn(ChunkInput) -> Mesh`**: it loops `input.cell_ixs`,
reads its own payload by index, queries neighbours via `input.cidx`
(`idx_at`, `step_x/y`, `nbr_count_idx` — and `halo_ixs` for cross-chunk
context), and emits primitives into a fresh `Mesh`, tagging each with the
owning cell index.  It must:
- read only (no shared mutation) — so it is a safe `par(...)` worker;
- emit geometry only for `cell_ixs` (halo cells are context only) — so the
  mesh stays bounded to the chunk;
- be deterministic in the cell payload + neighbourhood (no global state).

## 5. Pipeline API (gridmesh)

```loft
// Build/refresh the field's spatial index + chunk partition from coords.
pub fn field_new(layout: integer, xs, ys, chunk_shift) -> ChunkField;

// Edits — maintain coords, index, and the wired-in dirty index.
pub fn field_add_cell(f: &ChunkField, x, y);     // append; mark chunk(+halo border) dirty
pub fn field_mark_dirty(f: &ChunkField, x, y);   // recolor/erase: mark chunk(+halo border) dirty

// Collect the transient par work-list: one ChunkInput per dirty chunk
// (extract cell_ixs + ≤k halo).  O(dirty).
pub fn collect_dirty_inputs(f: ChunkField, halo_k: integer) -> vector<ChunkInput>;
pub fn clear_dirty(f: &ChunkField);

// Full (initial) build: one ChunkInput per chunk.
pub fn all_inputs(f: ChunkField, halo_k: integer) -> vector<ChunkInput>;
```

**Edit → dirty propagation:** `field_mark_dirty(f, x, y)` inserts
`chunk_of(x,y)` into `f.dirty`, and — when `(x,y)` is within `halo_k` of a
chunk border — the adjacent chunk key(s) too (their meshes read `(x,y)` via
the halo).  Idempotent set-insert ⇒ dedup.

**Consumer's parallel build loop (the whole thing):**
```loft
inputs = gridmesh::collect_dirty_inputs(field, HALO_K);   // vector<ChunkInput>
for ci in inputs par(cm = build_crystal_chunk(ci), N) {
  chunk_meshes[chunk_slot(ci.key)] = cm;   // replace this chunk's mesh
}
gridmesh::clear_dirty(field);
// upload/refresh only the replaced chunks' VBOs (consumer side)
```
The whole-world VBO (or per-chunk VBOs) = the union of `chunk_meshes`.
(Per-chunk VBOs + frustum culling are a moros Phase-C concern; the crystal
demo concatenates chunk meshes for its single VBO.)

## 6. Crystal consumer (the validation target)

`audience_crystal` becomes a full gridmesh consumer:

- **Field:** wrap `CellSnap` (xs/ys) in a `ChunkField`; keep `colors` as the
  crystal's own payload array (indexed by cell ix).  `chunk_shift` chosen so
  the demo's small fields are 1 chunk and the stress sizes (block-500 ≈
  23×23) are a handful of chunks (e.g. shift 3 → 8-cell chunks → ~9 chunks).
- **Rule `build_crystal_chunk(ci: ChunkInput) -> CrystalMesh`:** the current
  `crystal_segments_aged` per-cell body (mains + starbursts + branches),
  but looping `ci.cell_ixs` instead of `0..n`, reading neighbours via
  `ci.cidx` (already the Phase-A helpers) including `ci.halo_ixs` for
  cross-chunk on-axis/colour lookups, and emitting into a `SegMesh` with
  `owner = cell ix`.  Crystal's `cell_h_at` / `nbr_colors_idx` /
  nearest-older probe stay — they already take `cidx`.
- **Full build / backward compat:** `crystal_segments_aged(snap, state,
  tick)` becomes a thin wrapper — `field_new` over the snapshot, `all_inputs`,
  run the rule over every chunk (sequentially or `par`), concatenate into one
  `CrystalMesh`.  Output is byte-identical to today when chunk_shift puts
  everything in one chunk; with multiple chunks the only change is segment
  ORDER (grouped by chunk) — the projector keys vertices by `cell_ix`/birth,
  so order is immaterial to rendering, but the cross-mode regression test
  compares the SET (sort by (cell_ix, kind, coords)) not the raw order.
- **Incremental + parallel:** the projector's `apply_frame` calls
  `field_add_cell`/`field_mark_dirty` on paint/erase/recolor (marking the
  cell's chunk + halo-border neighbours), then on rebuild runs the
  `collect_dirty_inputs` + `par` loop above and replaces only the dirty
  chunks' meshes — turning the per-edit cost from O(all cells) to O(dirty
  chunks), parallel across threads.

**Halo for crystal:** the crystal rule reads up to 2 rings
(`nbr_colors_idx` dist 2 + 2-step nearest-older) → `HALO_K = 2`.  The halo
also needs OLDER cells across the border for the nearest-older-on-axis
rule, so `halo_ixs` carries them (they're in `ci.cidx` regardless; halo_ixs
just bounds the gather).

## 7. moros consumer (Phase C, sketch)

`moros_render` adds `build_chunk_mesh(map, cx,cy,cz) -> TriMesh` as the
rule: surface fans + walls + **edge rounding** from the `Hex` neighbour
pattern (the crystal technique applied to real geometry), reading the halo
via `map_get_hex` across chunk borders so seams/walls between chunks are
correct.  The `Map` gains the wired-in dirty index (it has none today);
edits (`map_set_height`/`map_paint_material`/`map_set_wall`) call
`field_mark_dirty`.  Per-chunk VBOs + dirty IDs + frustum culling replace
the global `build_hex_meshes`.  Axial `HexLayout` is implemented here.

## 8. Implementation steps (ordered)

1. **gridmesh B1 — `SegMesh` accumulator** (lift `CrystalMesh` shape with
   M1 narrow types: `single` coords, `u8` kinds/colours) + `emit_segment` +
   bounded-extent debug check.
2. **gridmesh B2 — `ChunkField` + chunk partition + wired-in dirty index**
   (`field_new`, `field_add_cell`, `field_mark_dirty`, `clear_dirty`).
3. **gridmesh B3 — `ChunkInput` + `collect_dirty_inputs` / `all_inputs`**
   (cell_ixs + ≤k halo extraction, reusing `step_x/y`/`idx_at`).
4. **crystal C1 — rule `build_crystal_chunk(ChunkInput) -> SegMesh`**: port
   the per-cell body to loop `cell_ixs` + emit into `SegMesh`.  Keep
   `crystal_segments_aged` as the full-build wrapper (`all_inputs` + run +
   concat).  Validate output SET == today, cross-mode.
5. **crystal C2 — parallel full build**: wrap the per-chunk run in `par(cm =
   build_crystal_chunk(ci), N)`.  Validate identical SET + measure speedup
   on the stress harness.
6. **crystal C3 — incremental**: projector `apply_frame` marks dirty;
   rebuild via `collect_dirty_inputs` + par; replace dirty chunk meshes.
   Validate incremental result == full rebuild for the same final field;
   measure per-edit cost O(dirty) vs O(N).
7. **(later) moros Phase C** — own sub-plan.

## 9. Verification

- **Equivalence:** for each stress pattern + size, the gridmesh-built
  segment SET (sorted by `(cell_ix, kind, x0,y0,z0,x1,y1,z1, color)`)
  equals the pre-gridmesh `crystal_segments_aged` set — interp AND native.
- **Incremental == full:** after a sequence of edits, the incrementally
  maintained mesh set equals a from-scratch full build of the final field.
- **Parallel == sequential:** `par` build set equals the sequential build
  set (order-independent compare).
- **Perf:** stress harness — full-build time unchanged or better; per-edit
  time drops to O(dirty chunks); `par` speedup with N.
- **Memory:** `store_memory()` bounded under edit churn (P5/P6 + the
  transient work-list freed each rebuild).
- Commands: `./target/release/loft [--native] --lib lib
  tools/audience-demo/crystal_stress.loft`; new
  `tests/scripts/12N-gridmesh-*.loft` (equivalence + incremental, cross-mode
  via wrap + native_scripts); `cargo test --release --test wrap --test
  native --test issues --test leak --test doc_hygiene`.

## 10. Loft-capability checks (confirm during impl; all expected OK post-recent-fixes)

- `ChunkField`/`ChunkInput` structs with `hash<…[ck]>` + `vector` fields,
  reassigned/cleared (@P290/@P295/@P302 fixed) — confirm the `dirty` hash
  field add/clear works.
- `fn(ChunkInput) -> SegMesh` as a `par(...)` worker with struct return +
  the field/rule as context — validated in THREADING.md; confirm SegMesh
  (a struct of vectors) deep-copies cleanly out of the worker.
- Returning a `ChunkField`/index-bearing struct from `field_new`
  (hash-bearing struct return) — @P300/@P301 fixed; confirm.
- Narrow vector element types (`single`/`u8`) in `SegMesh` with `as`
  casts at emit (M1).
