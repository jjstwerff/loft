<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 5b — Stage A handoff

State (2026-05-31):

- ✓ Chunk repo `loft-libs-graphics` updated (commits `40e0a3d`
  graphics + `86613ca` imaging on `main`).
- ✓ Tags pushed: `graphics-v0.1.0`, `imaging-v0.1.0`.
- ✓ GitHub releases live with deterministic tarballs:
  - https://github.com/loft-lang/loft-libs-graphics/releases/tag/graphics-v0.1.0
  - https://github.com/loft-lang/loft-libs-graphics/releases/tag/imaging-v0.1.0
- ⏳ Registry PR pending — paste the two `index.json` entries
  below into a `loft-lang/registry` PR.
- ⏳ Subsequent monorepo steps (5b-4..6) wait on registry merge.

## Stage A artifacts ready

| Package | Tarball | sha256 | Size | Staged at |
|---|---|---|---|---|
| graphics 0.1.0 | `/tmp/graphics-0.1.0.tar.gz` | `762271f637cbedaff4c3cda73ef9c6da0bcc826b8c81d9bb08eae03a645510db` | 88,828 B | `/tmp/loft-libs-graphics/graphics/` |
| imaging 0.1.0 | `/tmp/imaging-0.1.0.tar.gz` | `1eceb3771cfbefed9809b2efec39fd8180bba0336cc959e2e0ed6ddb369cf4ce` | 12,320 B | `/tmp/loft-libs-graphics/imaging/` |

Both tarballs were built by `loft package` (deterministic — zero
mtime in gzip + tar headers).  Re-run from the staging dir produces
the byte-identical sha256 — the registry gate-3 reproducible-build
check will reproduce.

## Chunk-repo content notes

**`graphics/`** — copied from monorepo `lib/graphics/` with:
- Dropped: `examples/` (consumers, not part of the published API),
  `js/loft-gl.js` (no `[wasm.bridge]` declaration to drive it yet),
  `.gitignore` (chunk repo has its own).
- Kept: `loft.toml` (unchanged), `src/`, `tests/` (incl.
  `tests/gold/` PNG fixtures for visual-regression), `native/`
  (incl. `native/tests/`), `docs/` (loft-doc input).
- Added: `README.md` (extraction blurb + surface inventory).
- `native/Cargo.toml` rewrote `loft-ffi` + `loft-ffi-macros` from
  `path = "../../../<crate>"` to `"0.1"` registry versions.  Both
  are on crates.io (`loft-ffi 0.1.0` shipped earlier; `loft-ffi-macros
  0.1.0` published 2026-05-31 15:59 UTC).

**`imaging/`** — copied from monorepo `lib/imaging/` with:
- Dropped: `wasm/` (Rust bridge crate depends on the unpublished
  `loft` crate via `path = "../../.."`; chunk-resident wasm bridge
  blocked until `loft-host-ffi` ships to crates.io).  The
  `[wasm.bridge]` section in `loft.toml` was also removed.  Note
  added in chunk-repo `loft.toml` documenting the deferral.
  No `--html` consumer currently uses imaging, so this is a follow-up,
  not a regression.
- Kept: `loft.toml` (sans `[wasm.bridge]`), `src/`, `tests/`,
  `native/`.
- Added: `README.md` documenting the Stage A constraint.
- `native/Cargo.toml` rewrote `loft-ffi` / `loft-ffi-macros` /
  `loft-ffi-build` from path deps to registry versions
  (`"0.1"` / `"0.1"` / `"0.1"`).

## User actions to ship Stage A

For each package (graphics first since it has no cross-package deps,
then imaging which is fully independent):

```sh
cd ~/path/to/loft-libs-graphics/   # the chunk-repo clone

# 1. Copy the staged content in
cp -R /tmp/loft-libs-graphics/graphics .

# 2. Commit + tag + push
git add graphics
git commit -m "Add graphics 0.1.0 — extracted from loft monorepo lib/graphics/ (@PLAN12 5b)"
git tag graphics-v0.1.0
git push origin main --tags

# 3. GitHub release with the deterministic tarball
gh release create graphics-v0.1.0 /tmp/graphics-0.1.0.tar.gz \
    --title "graphics v0.1.0 — 2D canvas + 3D rendering for loft" \
    --notes "First chunk-resident release.  Extracted from loft monorepo lib/graphics/ as part of @PLAN12 Phase 5b.  Stage A — interpreter + native targets; --html via the existing in-monorepo loft-gl.js bridge (chunk-resident wasm bridge deferred pending loft-host-ffi crate publish)."

# 4. (Repeat 1-3 for imaging — same shape with `imaging-v0.1.0` tag.)
```

Then open the registry PR with both entries:

```sh
cd ~/path/to/registry/
git checkout -b add-graphics-imaging-stage-a
# Edit index.json — add the two package blocks below at the appropriate
# alphabetical positions.  Both add a NEW top-level package entry
# (description + homepage + categories + yanked + versions); paste the
# blocks verbatim.  Then commit + PR.
```

## Registry index.json entries

```json
"graphics": {
  "description": "2D canvas + 3D rendering for loft (Canvas pixel surface, Mesh / Scene / glTF, OpenGL bindings)",
  "homepage": "https://github.com/loft-lang/loft-libs-graphics/tree/main/graphics",
  "categories": ["graphics"],
  "yanked": [],
  "versions": {
    "0.1.0": {
      "url": "https://github.com/loft-lang/loft-libs-graphics/releases/download/graphics-v0.1.0/graphics-0.1.0.tar.gz",
      "sha256": "762271f637cbedaff4c3cda73ef9c6da0bcc826b8c81d9bb08eae03a645510db",
      "size": 88828,
      "loft": ">=0.8",
      "subpath": "graphics",
      "published": "<ISO-8601 UTC at PR-open time>"
    }
  }
}
```

```json
"imaging": {
  "description": "PNG load/save + pixel manipulation for loft (Stage A — interpreter + native; wasm bridge deferred)",
  "homepage": "https://github.com/loft-lang/loft-libs-graphics/tree/main/imaging",
  "categories": ["graphics"],
  "yanked": [],
  "versions": {
    "0.1.0": {
      "url": "https://github.com/loft-lang/loft-libs-graphics/releases/download/imaging-v0.1.0/imaging-0.1.0.tar.gz",
      "sha256": "1eceb3771cfbefed9809b2efec39fd8180bba0336cc959e2e0ed6ddb369cf4ce",
      "size": 12320,
      "loft": ">=0.8",
      "subpath": "imaging",
      "published": "<ISO-8601 UTC at PR-open time>"
    }
  }
}
```

## Pending monorepo work (5b-4..6) — after registry merge

The graphics warning sweep already shipped; remaining steps are:

### 5b-4 — Migrate consumer loft.tomls

Consumers currently use `path = "../<pkg>"` for graphics-chunk libs.
After registry merge, swap each to registry versions:

- `lib/audience_crystal/loft.toml`: `gridmesh = { path = "../gridmesh" }` → `gridmesh = ">=0.1"`.
- `lib/moros_render/loft.toml`: `graphics = { path = "../graphics" }` → `graphics = ">=0.1"`.
- `lib/moros_sim/loft.toml`: `graphics = { path = "../graphics" }` → `graphics = ">=0.1"`.
- `tools/audience-demo/loft.toml`, `tools/audience-demo-50/loft.toml`,
  `tools/viewer/loft.toml`: add `graphics = ">=0.1"` if missing (some
  already added in 6b).
- `lib/moros_ui/loft.toml`: indirect via moros_sim — no direct change needed.

Sibling moros_* deps stay as path deps (not being extracted yet —
Phase 7-series).

### 5b-5 — Remove monorepo dirs + update hygiene/wasm

```sh
rm -rf lib/shapes lib/gridmesh lib/graphics lib/imaging
```

Then:
- `tests/extraction_hygiene.rs::manifest_native_functions_cover_drained_libraries`:
  add graphics + imaging native symbols to `FORBIDDEN_LIBRARY_SYMBOLS_MANUAL`
  (same pattern net used for `n_http_*` / `n_ws_*` / `n_pack_*` in 6b).
- `src/wasm.rs::BUNDLED_LIB_FILES`: graphics-chunk files were already
  routed to `tests/fixtures/libs/web/...` for web (6b).  For graphics,
  the analogous fixture clones land in `tests/fixtures/libs/graphics/`
  + `tests/fixtures/libs/imaging/` if `--html` paths need them.  When
  `--html` doesn't need the bundle (currently true for imaging — no
  consumer; for graphics, brick-buster uses it), keep the include OR
  fixture-route as appropriate.
- Update `scripts/sync-fixtures.sh` PINNED_REFS to include
  `loft-libs-graphics  graphics-v0.1.0  graphics` and
  `loft-libs-graphics  imaging-v0.1.0  imaging`.  Then re-run to
  populate `tests/fixtures/libs/graphics/` + `tests/fixtures/libs/imaging/`.

### 5b-6 — Green CI

- `make ci` clean.
- Library suite green (each removed lib's tests now run from
  registry-installed copy via consumer's `loft test --deps`).
- Wrap + extraction_hygiene green.
- The brick-buster example may need a path update: was at
  `lib/graphics/examples/25-brick-buster.loft`; if shipped as a
  monorepo demo, move to `tools/brick-buster/` with its own
  `loft.toml` declaring `graphics = ">=0.1"`.

## Out-of-scope follow-ups surfaced

- **Chunk-resident wasm bridges blocked on `loft-host-ffi` crate publish.**
  Imaging's `wasm/` bridge depends on `loft = { path = "../../.." }`
  (our `loft` crate isn't on crates.io).  Same constraint will apply
  to any future library wasm bridge.  Resolution path: factor out the
  host-FFI surface (`Stores`, `DbRef`, `Store::*`, `vector::*` — what
  `lib/imaging/wasm/src/lib.rs::use loft::...` actually needs) into a
  separate `loft-host-ffi` crate, publish it, then rewrite chunk-resident
  wasm bridges against it.  Filing as a follow-up needs a sub-plan
  slot under @PLAN12 or its own lib_plan.
