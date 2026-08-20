<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN144 — The 2-D stage

> **Status — SHIPPED 2026-08-20.** Seventeen of eighteen phases delivered as
> **`stage` 0.6.0 → 0.15.0** on `loft-libs-graphics` branch `stage-package`; arc **G**
> stays DEFERRED behind a trigger whose full design is kept below. Reference for
> everything that shipped is the library's own **[`stage/README.md`](https://github.com/loft-lang/loft-libs-graphics/blob/stage-package/stage/README.md)** —
> this file is a closure record. Tracker: [loft-lang/plans#144](https://github.com/loft-lang/plans/issues/144).
> Part of the 2-D game stack: @PLN145 text/tweens/widgets · @PLN146 content + delivery ·
> @PLN147 the editor ([set overview](../README.md#plan-sets--where-four-plans-are-one-piece-of-work)).

## What shipped

A game is a **tree you mutate**, not a frame loop you draw. `stage` is a retained scene
that presents a 3-D world through three knobs — a projected position, a sprite **origin**,
and `layer` + `depth` — with a camera, light and atmosphere on top. 135 tests, green on
**both backends**.

| Arc | Phases | Delivered |
|---|---|---|
| **A** scene | `A0`–`A6` | The batching premise falsified first (1.20×/1.53×/2.50× at N = 100/1000/5000, growing with N), then a flat node array, the draw, the batcher, one `gl_draw_instanced` per run, depth + picking + capture, alpha and tint, clipping |
| **P** presentation | `P1`–`P6` | Origin + `layer`/`depth` + the 2.5-D cue · the camera as a uniform · ambient motion costing nothing per frame · animation advanced by a step · facings chosen by the projection · several views over one stage |
| **L** light | `L1`–`L3` | Light per sprite folded into the tint · the light-map composite with the HUD unlit · per-layer fog and blur |
| **G** paths | — | **Deferred** — trigger re-checked 2026-08-20 and rewritten so it can be checked (below) |

## The design calls worth keeping

- **A flat `vector<StageNode>` with integer parents, not pointer links.** A parent holding
  children while a child points back is a dependency cycle in loft's ownership model; the
  flat form sidesteps it and hands the batcher its iteration order for free.
- **Merge adjacent, never reorder.** Grouping by atlas *globally* silently reorders two
  overlapping sprites and is a wrong picture, not a slow one.
- **The camera is a uniform, not a rewrite** — baking it into node positions makes a scroll
  an O(N) re-upload every frame. Parallax translates; it does not scale.
- **The instance attribute is a frame INDEX, not a uv rect** — one float per animating
  sprite per frame instead of four.
- **`advance` takes integer MICROSECONDS from the simulation**, never a clock: identical at
  any frame rate and replayable under a recorded input stream.
- **The projection picks the facing model, not the author** — top-down rotates one sprite
  continuously (15° steps cost no atlas entries), side-on mirrors at most.
- **Light rides A5's tint attribute** — no pass, no framebuffer, order-independent by
  construction. The map replaces per-sprite lighting rather than stacking with it.
- **A type name is global across the build**: `Node` and `Light` were both refused because
  `mesh3d` publishes them, so the package ships `StageNode` and `StageLight`. The compiler
  refusing is the good outcome — after publishing, that rename is one nobody can make.

## What the phases found — the headline value

Three defects the plan surfaced in **already-green** work, each invisible to the arc that
shipped it:

- **`P1`** — the origin was a point inside the sprite to `compose` and the rect's corner to
  everything else, indistinguishable at `(0,0)` where 56 green tests all sat.
- **`P5`** — a sequence's NAME lived in a vector beside `st_seqs` that only one registrar
  appended to, so mixing the two registrars put every name on another sequence's cells.
  Both lists stayed individually well-formed; only the correspondence was wrong.
- **`P6`** — a **clip did not follow its content under a camera**: the clip is derived in
  world space while what it cuts has already moved, so a pan wider than the node cut all of
  it and a clipped panel silently vanished. `A6` and `P2` were each green alone.

And the methodological one, which is the plan's most transferable output: **a control that
does not fire is a finding, and it lies in four distinct ways.** Across `P4`–`L3` each shape
appeared at least once — a gate that could not fail on its own subject (`P4`), a test whose
expected value equalled the unchanged value (`L1`), a rule with two homes so neither could be
falsified alone (`L1`, `L3`), and a mutation that never touched the code under test (`L2`).
Recorded in `CLAUDE.md`'s debugging policy terms: prove the harness can fail, and check
*which* test goes red, not merely that one does.

## Engine defects filed on the way through

| Issue | One line |
|---|---|
| [loft#1013](https://github.com/loft-lang/loft/issues/1013) | A `??` fallback that CALLS a function leaks the record it returns, once per index miss, both backends — and the compiler's refusal of a struct-valued constant prescribes exactly that leaking form |
| [loft#1017](https://github.com/loft-lang/loft/issues/1017) | `--native` store corruption from an accessor returning a BORROW on one path and a FRESH record on its bounds guard; `rec=0xFF000000` was canvas pixels read as a record number |
| [loft#1018](https://github.com/loft-lang/loft/issues/1018) | The GL/wasm target sizes its canvas in CSS pixels and never reads `devicePixelRatio`, so every `--html` page is soft on a high-density display |

## Arc G — vector paths on the GPU (DEFERRED, design retained)

**H effort**, and the one genuinely research-shaped item in this plan: a path rasterizer
with AA fills, gradients and stroke joins. `RENDERER.md` puts SDF shapes, paths, gradients
and post-fx in its territory.

**Evaluated 2026-08-20 — NOT fired, and the trigger itself was unobservable as
written.** It said *open vector paths when a consumer needs resolution-independent
art — a UI that scales across DPI, a zoomable map*. The check found no consumer
asking, and something more useful: **nothing in the
stack is DPI-aware**, so the DPI half could never fire by observation. A
consumer on a high-density display gets a uniformly soft picture and nothing
tells them why — so *waiting for a consumer to feel it* was waiting for a
signal that cannot arrive. What the evidence shows instead:

| Checked | Found |
|---|---|
| Consumer asks (`crawler/LOFT-HANDOFF.md`, moros, dryopea docs) | None. crawler's handoff is three ENGINE defects; no art or rendering ask anywhere |
| `--html` / wasm DPI handling | **`devicePixelRatio` appears nowhere** in `doc/loft-gl-wasm.js` or `src/*.rs` — the target is not DPI-aware at all |
| moros, the consumer with the most icon art | Solves it **at author time**: `tools/svg_to_3d.py` extrudes game-icons.net SVGs to meshes, and item icons ship as a 32 px atlas (`DEVELOPER_ART.md`) |
| @PLN145 `D` widgets | An **extraction** of moros's existing kit, which renders through the raster path today and asks for nothing more |
| @PLN147 the browser editor | `status:future`, nothing built; its stated invariant is about the STORE, not about crisp rendering |

**So the real finding is that resolution-independence in this stack is an
AUTHOR-TIME concern that @PLN146's packer already owns**, not a runtime
rasterizer — moros proves the route works. Arc G opens only when author-time
rasterisation is shown to be *insufficient*, which is a far sharper bar than
"someone wants crisp art".

**The trigger, restated so it can actually be checked.** Any ONE of these,
each observable rather than felt:

- **T1 — continuous zoom over authored art.** A consumer needs more scale
  steps than an atlas can hold: the same asset packed at > 4 sizes, or a zoom
  that is continuous rather than stepped (a map). Measurable in the pack.
- **T2 — DPI, once it is observable.** `--html` honours `devicePixelRatio`
  (it does not today — **loft#1018**, filed from this check; it is a
  prerequisite and belongs to the html target, not here), AND a consumer's UI
  measures soft at ratio ≥ 2. ⚠ That issue's own trap is worth knowing before
  anyone calls T2 satisfied: scaling the backing store without scaling the
  input space gives a crisp picture where every click lands at `1/dpr` of
  where the user pointed.
- **T3 — author-time rasterisation proven insufficient.** @PLN146's packer
  ships, a consumer uses it for scalable art, and the atlas cost or the
  quality is measured unacceptable. This is the one that actually decides it.

Until one of those is true, sprites + atlas cover the cases and a path
rasterizer with AA fills, gradients and stroke joins stays the one genuinely
research-shaped item in this plan — H effort, no consumer, and building it
unasked would make it the fourth thing nobody chose (@PLN145 `D0b`'s lesson).

## Companion files

Kept in place — all three are linked from the sibling plans and are shared reference:

- **[RENDERER.md](RENDERER.md)** — what arc A adopted from crawler's `RENDER.md` (never
  reorder, merge adjacent only; a premultiplied atlas that packs itself; a per-instance 2×3
  affine) and what it declined to arc G.
- **[PRIOR_ART.md](PRIOR_ART.md)** — what `moros`, `dryopea`, `crawler`, `hexbody` and
  `crew_punk` already built, plus the library-integration audit.
- **[PRESENTATION.md](PRESENTATION.md)** — arc P's design: the three knobs, the two scroll
  modes, and **why occlusion is a placement rule and gets no engine mechanism** (settled,
  not open — the help, should it ever be needed, is an authoring-time check in the editor).

## See also

- **[`stage/README.md`](https://github.com/loft-lang/loft-libs-graphics/blob/stage-package/stage/README.md)** — the reference for everything above
- [@PLN145](../145-authoring-libs/README.md) `B`/`C`/`D` sit on this stage · [@PLN146](../146-content-delivery/README.md) `F4` needs `A2` · [@PLN147](../147-content-editor/README.md) `T1`–`T3` need `A4`/`P1`
- [`lib_plans/64-game-client`](../../lib_plans/64-game-client/README.md) — *replicate the world, never the presentation*, which `P6` makes structural
