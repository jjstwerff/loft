<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Universal hex-world editor + library extraction

A coherent set of loft libraries that together provide a
**universal editor for hex-world games**: paint terrain in
multi-layer hex grids, place items, author walls/bridges,
compose stencils into reusable prefabs and standalone unit
meshes, render via a shared 3D pipeline.  The editor and
its underlying substrate are **game-agnostic**: each game
provides its own palette, item registry, and gameplay hooks
on top of the shared layer.

**Moros is the first partner project** — its existing
hex-map + editor + render + stencil + undo code is the
seed material for the extraction.  Moros is rough-but-
unit-tested; expect bugs to surface during extraction and
get fixed in the shared library.

The strategic goal is to **lower the bar for building new
hex-world games on the loft suite** to the point where an
indie / starting dev can ship a full game on the
substrate alone — see [REFERENCE.md § Audience model](REFERENCE.md#audience-model).

The full architecture, slice-by-slice extraction plan,
per-package API outlines, design principles, and open
questions live in [`REFERENCE.md`](REFERENCE.md).  **This
README is status + forward path only.**

## Status

**FUTURE.**  Plan drafted 2026-05-27; no extraction work
shipped yet.

**Trigger to start:** the first partner consumer that
isn't moros wants a shared substrate.  Dryopea's plan 06
(editor-to-stencil pipeline) is the explicit second
consumer; it currently reimplements parts of what moros
already has.  Either:
- Dryopea plan 06 S1 trigger fires, OR
- Indie / strike-path interest surfaces (per dryopea plan
  06 § Who this serves), OR
- The shared substrate becomes worth the extraction cost
  on its own merits.

## Forward path — phase index

Slice-based extraction; each phase ships a small, useful
library + adapts moros to consume it + lets the second
consumer (dryopea or other) integrate and find bugs.

| Phase | Library / scope | Source in moros | Status |
|---|---|---|---|
| **L0** | Architecture spike + naming + package layout | n/a (planning) | not started |
| **L1** | `hex_grid` — pure math primitives | moros_map types + `hex_distance` + scattered | not started |
| **L2** | `hex_map` — multi-layer data + paint verbs | moros_map (most of it) | not started |
| **L3** | `hex_render` — mesh emitters + 3D camera | moros_render + lib/wall.loft | not started |
| **L4** | `hex_stencil` — stencil format + stamp + save/load | moros_editor stencil_* family | not started |
| **L5** | `hex_editor` — editor tools + undo + UI | moros_editor + moros_ui + moros_sim/tools | not started |
| **L6** | `hex_entity` — baked-mesh entity runtime (NEW) | nothing in moros — greenfield | not started |
| **L7** | Onboarding documentation + indie strike-path examples | n/a (docs) | not started |

Phase ordering is **suggested**, not strictly sequential.
A second-consumer pull-request may pull a later slice
forward when its integration trigger fires.

## Cross-project links

- **moros** — the partner whose code gets extracted.  Lives
  outside loft today; relevant subset is currently mirrored
  in `lib/moros_*` packages.
- **dryopea** — the second consumer.  See its
  [`plans/future/06-editor-stencil-pipeline/`](https://github.com/jjstwerff/dryopea/tree/main/plans/future/06-editor-stencil-pipeline)
  — plan 06 explicitly relies on this extraction landing.
- **lib_plans/12-library-extraction** — the SEPARATE,
  ALREADY-ACTIVE work to move `lib/*` packages out of the
  monorepo into per-family external chunks via the
  registry.  Plan 24 (this one) governs **which packages
  exist + their shape + their consumers**; plan 12 governs
  **where they live (monorepo vs external repo) + how
  they're distributed**.  Coordinate but don't conflate.
- **PKG_REGISTRY.md** — the registry MVP.  New libraries
  born from this plan get published via the registry as
  they shipped per plan 12's process.

## Open work — at a glance

See [REFERENCE.md § Open questions](REFERENCE.md#open-questions)
for the active question list.  Highlights worth flagging
at the README level:

- **Package naming** — `hex_*` family vs. a different
  umbrella (`buildkit_*`, `world_*`, `tile_*`, …).  Naming
  decision is L0 work.
- **Moros's continued operation through extraction** — each
  slice must leave moros working.  This is a non-negotiable
  constraint, not a question.
- **Per-game customisation surface** — how a game registers
  its palette / item kinds / wall semantics on top of the
  shared substrate.  Sketched in REFERENCE.md, refined per
  phase as moros + dryopea push on it.

## See also

- [`REFERENCE.md`](REFERENCE.md) — architecture, slice plan,
  per-package API outlines, design principles, audience
  breakdown
- [`../12-library-extraction/`](../12-library-extraction/README.md) —
  monorepo-to-external-repo extraction process (sibling
  arc; coordinate)
- [`../../PACKAGES.md`](../../PACKAGES.md) — loft package
  format
- [`../../PKG_REGISTRY.md`](../../PKG_REGISTRY.md) — the
  package registry
- Dryopea's plan 06 — the second-consumer pull that motivates
  this plan starting
