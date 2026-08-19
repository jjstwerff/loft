<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN144 — The 2-D stage

> Tracker: [loft-lang/plans#144](https://github.com/loft-lang/plans/issues/144)
> (`subject:libs`, `status:future`). Everything here sits **on top of the shipped
> packages** — `graphics` 0.5.5, `input`, `fixstep`, `shapes`, `imaging`. Arcs A–D
> and F need no engine change; arc E is one stub in this repo.
> **Part of the 2-D game stack** — four plans cut from one design: @PLN144 the stage ·
> @PLN145 text/tweens/widgets · @PLN146 content + delivery · @PLN147 the editor. Set overview,
> through-lines and where to start: [`plans/README.md` § Plan sets](../README.md#plan-sets--where-four-plans-are-one-piece-of-work).

## Status

**Open — arcs A and P COMPLETE and `L1` shipped (`stage` 0.13.0); `L2`/`L3` remain.** The runtime is not the gap: `loft --html` already
beats Flash on deployment (self-contained page, no plugin, WebGL2, E2E-gated in CI), and
static types, compiled-WASM speed and stackful coroutines all beat ActionScript 3. What is
missing is the **layer a game author writes against** — `graphics` ships a complete
immediate-mode GL surface and nothing above it, so every game re-implements the scene graph,
the text field, the tweens and the widgets by hand.

The measurement is `tools/brick-buster/25-brick-buster.loft` — **1983 lines for a Breakout
clone**, ~190 of them (`build_atlas()`) hand-poked pixel art and ~40 more pre-baking one GL
texture per string. The AS3 equivalent is ~500 lines with the art drawn in a tool.

## Goal

A game is a **tree you mutate**, not a frame loop you draw: ship `stage` — a retained scene,
the presentation model that lets it show a 3-D world, and light — and rebuild Brick Buster on
it while it gains rotation, per-node alpha, tint and a camera.

**Scope: 2-D games, at any scale — not a 3-D engine.** The 2.5-D half is a *sprite
presentation* of a 3-D world (a hex or grid footprint, sprites standing up from it) and it
stops there: no meshes, no camera projection. **Lighting, fog and background blur are in
scope** — a torch, a day/night cycle, a hazy blurred distance — as tint, a composite pass and
per-layer atmosphere (L1–L3), the Hollow Knight / Silksong pipeline rather than a 3-D
lighting model. dryopea's 3-D renderer and moros's 3-D editor are where lessons came
from, **not consumers this plan serves**: the test of any proposal here is whether a 2-D game
needs it, never how big the game is.

**`stage` is a library, so its per-frame loop runs NATIVE on every backend.** A `use`d
library auto-compiles its native-compilable subgraph to a cdylib and dispatches over the shared
store; `--interpret` selects the interpreter for the *program*, never for the libraries it calls
([NATIVE.md](../../NATIVE.md)). So "the interpreter is slow" is not a constraint on this stack's
hot path — which is why `A0`'s `--native` row is the only one worth planning against, and why a
game may be authored on `--interpret` without paying for the stage.

**No dependency probe is due here.** The 2026-08-19 library audit
([`PRIOR_ART.md`](PRIOR_ART.md) § Library integration) found this arc's dependencies —
`graphics`, `mesh3d`, its `math` submodule — carry real consumers in all five sibling trees, so
they are validated by use. @PLN145 and @PLN146 each gained an XS probe because they lean on a
**published but unadopted** package; this plan does not, and adding the ceremony anyway would
be cost without a question behind it.

**Targets: the interpreter, `--native`, `--html`, and `--native-android`.** Android is a real
target today — loft's @PLN106 shipped `--native-android` (signed APK, `src/android.rs`, on-
emulator goldens for rendering, touch, keyboard and audio), and **GLES 3.0 is WebGL2**, so a
program written for the browser GL surface runs there unchanged. That means this stage is
Android-capable with **no new rendering work** — but only once the graphics library's Android
backend, which currently lives *only in loft's test fixture*, is published
([loft-libs-graphics#32](https://github.com/loft-lang/loft-libs-graphics/issues/32)). iOS is a
different question and does sit behind `GFX.PORTABLE`'s wgpu backend.

**Two growth directions are deliberately out, and neither is an omission.** *Full 3-D* will
mature on its own track and this set is expected to grow toward it then — the groundwork
already has a home in [`lib_plans/72-renderer-backend-boundary`](../../lib_plans/72-renderer-backend-boundary/README.md)
(`GFX.PORTABLE`), which is also what unblocks native Android/iOS. And *broad standards
integration* — interchange formats, asset and tooling conventions, browser and graphics
standards beyond the ones already used — is wanted, but it is a direction rather than a phase:
folded into these four plans it would multiply their surface without moving their vehicles.
Both belong to later plans; recorded here so that a reader finding them absent knows the
absence was chosen.

## Effort + design

- **Effort:** MH — 18 phases in three arcs (`A3` split in two), **none above M**. **Arcs A and P complete 2026-08-19**, `L1` shipped; `L2`/`L3` and `G` remain; see § Effort per phase
- **Design:** ✓ for A–F, — for G
- **Last touched:** 2026-08-19

## Composition matrix — Stage A

`stage` introduces a new value (a node) and a new operation (compose-and-draw), so
the matrix is written as `/tmp` probes on `--interpret` **before** A1, and graduates
to `tests/scripts/144-stage.loft`. The axes it actually touches:

| Axis | Cells |
|---|---|
| tree depth | leaf · 1 parent · 3 deep |
| transform | translate · rotate · scale · non-uniform scale · pivot ≠ origin |
| composition | alpha · tint · both · neither |
| clip | none · parent rect · nested rects |
| draw path | per-sprite (A2) · batched (A3) |
| backend | `--interpret` · `--native` · `--html` |

The off-diagonal cells are the point: rotation **under** a scaled parent **inside** a
clip, batched, split across two atlases. Give each derived fact — world matrix, composed
alpha, clip rect, depth key — **one home** every path reads, so the cells cannot disagree;
the matrix proves it rather than asserting it. That is the bug class plan-58 shipped: an
invariant re-derived per code path, validated only where the derivations coincided.
Hand-compute every expected value — agreement between two backends is not a pass.

## Sub-arcs

`Verify` is what would go **red if the phase were done wrong** — filled when the
phase is cut, not when it is implemented.

| Item | Where | Verify | Status |
|---|---|---|---|
| **A0** — falsify the batching premise | [`probe-a0.loft`](probe-a0.loft) | **DONE — the premise holds on native, and the browser cannot run it at all.** Min-of-3 after warm-up, 60 frames, SwiftShader under `xvfb`: **1.20× / 1.53× / 2.50×** at N = 100 / 1000 / 5000, monotonic and growing with N — the shape A3 needs. ⚠ **Read the `--native` row only.** A `use`d library auto-compiles to a cdylib and runs native *whatever backend the program uses* ([NATIVE.md](../../NATIVE.md) — `--interpret` chooses the interpreter for **your program**, not for the libraries it calls), so `stage`'s per-frame loop is native in both modes. The probe's `--interpret` figures (1.32× / 2.03× / 3.74×) measure the loop sitting in the **application**, which is the one place that is still interpreted and is not where a game's loop will live | ✅ **Shipped** |
| **A1** — node tree + transform composition | `stage` | ✅ **Shipped** — `loft-libs-graphics` branch `stage-package`, 9 tests green on both backends. Gated on hand-computed **POINTS**, not matrix entries: `mesh3d` has no `mat4_rotate_z`, so a mat4 oracle cannot express a 2-D rotation, and entries check a convention where points check what a renderer needs. **Verified, not assumed:** reversing the composition to `W = L·P` fails *only* the 3-deep cell — the other five pass | ✅ **Shipped** |
| **A2** — draw the tree, emitting `DrawRect`/`DrawText` | `stage` | ✅ **Shipped** as `stage` 0.2.0 — a **parallel run** on a headless software canvas: stage output byte-identical to a hand-written `fill_rect` pass, child placement included. A panel-**shaped** list renders unmodified. ⚠ The *literal* cross-package gate waits on @PLN145 `D0`: `lavition_ui` is unpublished, so its `DrawRect` and `stage`'s are two nominal types with one shape, and one must eventually give | ✅ **Shipped** |
| **A3a** — the batcher: contiguous runs + instance buffer | `stage` | ✅ **Shipped** as `stage` 0.3.0 — run boundaries exactly at atlas changes, hand-computed; the packed floats exact at `INSTANCE_STRIDE`/`OFF_*`; and **interleaving two atlases asserts FOUR runs, not two** — a batcher answering 2 is sorting, and sorting reorders the frame under blending. Controls fire: gathering globally gives 2, stride 13 fails all three layout cells | ✅ **Shipped** |
| **A3b** — one `gl_draw_instanced` per run | `stage` | ✅ **Shipped.** Both halves, three consecutive runs: **`differing_pixels=0/12288`** against A2's software pass, and **2 draw calls** for 3 sprites across 2 atlases — same pixels alone would have passed a batch that silently fell back. Unblocked by fixing [loft-libs-graphics#33](https://github.com/loft-lang/loft-libs-graphics/issues/33) | ✅ **Shipped** |
| **A4** — depth order, hit-test, input routing | `stage` | ✅ **Shipped** as `stage` 0.4.0 — a hand-computed pick table: topmost-wins, half-open edges so abutting nodes never both claim a pixel, correct under rotation via a closed-form inverse, **a click over a transparent texel reaching what is behind**, and **capture** (a release belongs to the press). Three controls fire on their own tests | ✅ **Shipped** |
| **A5** — per-node alpha + tint, blending instead of discard | `stage` | ✅ **Shipped** as `stage` 0.5.0 — every expected byte hand-computed and classified by **exact colour with `other` = 0**: alpha 1 is the colour, alpha 0 paints nothing, half-alpha over black and over a non-black destination both exact, two translucent nodes stack in order, and `render` / `render_stage` agree wherever alpha is 1. **Premultiplied** on both paths; the straight-alpha control fails both half-alpha cells | ✅ **Shipped** |
| **A6** — clip / mask rect | `stage` | ✅ **Shipped** as `stage` 0.6.0 — hand-counted pixels: a child cut at the exact parent boundary, **nested clips intersecting** (not innermost-wins), disjoint clips showing nothing, a sibling untouched, and the unclipped scene unchanged. Plus the A3a interaction: **a run is `(atlas, clip)`** — one atlas under two clips is two runs. Three controls fire | ✅ **Shipped** |
| *— arc **P**: the presentation model —* | | | |
| **P1** — sprite origin, `layer` + `depth`, and the 2.5-D cue | `stage` | ✅ **Shipped** as `stage` 0.7.0 — layer outranks depth, depth is distance *into* the screen (larger draws first, so `depth = -y` works), ties stable. Plus `depth_cue`: distance is **smaller and hazier**, scaled about the **origin** so a mob's feet stay on its tile. ⚠ It found a defect carried since `A1` — the origin was a point inside the sprite to `compose` and the rect's corner to everything else, indistinguishable at `(0,0)` where 56 green tests all sat | ✅ **Shipped** |
| **P2** — camera with a per-layer parallax factor | `stage` | ✅ **Shipped** as `stage` 0.8.0 — a pan at factor 1.0 is **pixel-identical** to placing every node that much further left, so flat mode is the degenerate case rather than a second path; a layer moves by its own factor; picking **un-applies per layer**; and **100 camera moves change not one float** of the packed buffer. Three controls fire | ✅ **Shipped** |
| **P3** — world sprites: ambient motion, no per-instance state | `stage` | ✅ **Shipped** as `stage` 0.9.0 — **100 ticks over 500 sprites change not one float**; the phase is derived from position so neighbours are visibly out of step; and sway is **visual only**, so a swaying tree keeps its depth, bounds and hit area. Three controls fire | ✅ **Shipped** |
| **P4** — mob animation: sequences, rate, loop mode | `stage` | ✅ **Shipped** as `stage` 0.10.0 — a loop's worth of ticks lands on frame 0 exactly, 30 Hz equals 60 Hz at the same elapsed, `once` stops on its last frame, and **ping-pong runs `0 1 2 3 2 1`** — period 2n−2, each end shown once. ⚠ The frame-rate cell **could not fail**: at 12 fps the two rates agree at every sample even under float accumulation, out to 60000 ticks, so it pins rate independence and nothing about units. The units gate is a separate cell, **ticked rather than jumped** — 0.1 s added eight times is `0.7999…`, frame 7 where 8 is right. Six controls fire | ✅ **Shipped** |
| **P5** — facings, **two models chosen by projection** | `stage` | ✅ **Shipped** as `stage` 0.11.0 — one call, `face(node, angle)`, and the projection decides what it costs. Top-down: **24 facings at 15° are one cell turned 24 ways**, gated as 24 distinct affines over one sequence. Side-on: mirrored, never rotated, and the **footprint and origin do not move** — hand-computed against the unmirrored bounds; north and south have no partner. A facing change mid-walk **keeps the phase**. Resolution is by name, most-specific first, falling back rather than failing. ⚠ Found in build: the sequence name lived in a **vector beside `st_seqs`** that only the naming registrar appended to, so mixing the two registrars put every name on somebody else's cells — silently, both lists individually well-formed. Seven controls fire | ✅ **Shipped** |
| *— arc **L**: light and atmosphere —* | | | |
| **L1** — light sampled per sprite, applied as tint | `stage` | ✅ **Shipped** as `stage` 0.13.0 — falloff hand-computed at d = 0/50/100/150, equal distance equal tint **in four directions**, and a red sprite under a blue light goes **nearly black, not blue**. Sampled at the ORIGIN, proved with a 20-tall and a 60-tall sprite on one spot. Ambient defaults to 1.0, so the feature cannot dim a game that never asks for it; channels clamp, so two torches do not wrap to dark. ⚠ Honest cost, gated beside the camera that does not: a light **dirties the buffer**. Nine controls fire — **two by NOT firing**, exposing a render-path test that stood its sprite ON the light (where lit = material, so an unlit path matched) and a clamp masked by a second guard enforcing the same rule | ✅ **Shipped** |
| **L2** — light-map composite pass | `stage` | falloff along a ray from the light is **monotonic** and never undercuts its floor — the invariant, since hand-computing a curve per pixel is not a gate anyone maintains. Expectations **generated from the same constants the shader uses** (crawler's `light_cone.py` technique), so retuning happens in one place. And the HUD's pixels are **bit-identical with the light on and off**, which is how *the HUD draws after, unlit* stops being a comment | Open |
| **L3** — per-layer atmosphere: blur + fog | `stage` | fog at density 0 is **bit-identical to no fog** and at density 1 is exactly the fog colour — the degenerate cases proved, as with P2's factor `1.0`. A blurred layer **preserves total luminance** (edge clamping darkening the border is the classic bug). And blur is applied **per layer before it composites**, so a sharp foreground over a blurred background stays sharp — which a fullscreen blur cannot do and is the entire point | Open |
| **P6** — several views over one stage | `stage` | ✅ **Shipped** as `stage` 0.12.0 — view 0's frame is **byte-identical with a second view present**; two views cost **one upload**; each view picks through its own camera and a point outside its rect is not its to answer. A fresh stage already has one view, so the unsplit game is the degenerate case. ⚠ It found **two** defects: a clip did **not follow its content under a camera** (a panned clipped panel vanished — A6 and P2 each green alone, no test had moved a camera across a clip), and a `--native` **store corruption** of its own making, `view_at` answering a borrow on one path and a fresh record on the other (loft#1017). Nine controls fire — and one earned its keep by **not** firing, exposing an inert test | ✅ **Shipped** |
| **G** — vector paths on the GPU | — | deferred behind a trigger (below) | Deferred |

## Companion files

- **[RENDERER.md](RENDERER.md)** — arc A inherits `crawler/RENDER.md` rather than competing
  with it (crawler stopped building that renderer 2026-07-22). Adopted: *never reorder, merge
  adjacent only*; a premultiplied atlas with 1 px padding and mipmaps; an atlas that packs
  itself at load time with no programmer direction; a per-instance 2×3 affine; frame stats
  naming the reason for every batch break. Declined for now: SDF shapes, paths, gradients,
  post-fx — arc G's territory.
- **[PRIOR_ART.md](PRIOR_ART.md)** — what `moros` already built: `lavition_ui` **is** arc D,
  `font.loft` **is** B1m, and the editor is already 2D with 3D extracted through `hex_proj`.
- **[PRESENTATION.md](PRESENTATION.md)** — arc P: the three knobs, the two scroll modes,
  and why occlusion is a placement rule.

## The presentation model

The world stays 3-D in the app and `stage` stays a 2-D presenter, with **three knobs as the
entire contract**: a projected position, a sprite **origin** (bottom-centre, so a sprite
stands up from its footprint and sorts by it, never by its artwork), and `layer` + `depth`.
A flat 2-D game sets all three trivially. **Two scroll modes are one mechanism** — a camera
with a per-layer parallax factor, flat scrolling being every factor at `1.0`. And
**occlusion is the level designer's rule, not an engine mechanism**: fences, trunks, windows
and low walls are narrow or transparent, so alpha does the work. Full reasoning, and what
that decline costs A4/A5: [PRESENTATION.md](PRESENTATION.md).

## Effort per phase

Totals: no phase above M, which is the § Cutting rule holding rather than optimism — the two
bounds (an exact comparison half-way, and able to go red alone) both push size *down*. Three
phases carry a design call that decides the effort, made here rather than discovered.

| Phase | E | What the effort actually is |
|---|---|---|
| **A0** | XS ✅ | *Done.* Two probe programs — an N-times `draw_sprite` loop, and one instance-buffer upload plus a single `gl_draw_instanced`. Time 300 frames at N = 100/1000/5000 on both targets; the browser half reuses the headless-Chrome harness `tests/html_render.rs` already has. No library code. **Its first read was unusable** — batched looked 10× *slower* at N=100 and per-sprite at N=5000 came out faster than at N=1000, which cannot be true if the work scales with N; warm-up was landing inside whichever block ran first. Warm-up plus min-of-3 fixed it, and that calibration was most of the phase. Its second lesson is about **where the loop lives**: the probe's loop is app-level, so only there does the backend matter — move it into a library, as `A2` will, and it is native either way. |
| **A1** | S ✅ | *Done.* **Design call: the tree is a flat `vector<Node>` with integer parent indices, not pointer-linked.** A parent holding children while a child points back is a dependency cycle in loft's ownership model; the flat form sidesteps it entirely, keeps insert-order-is-parents-first as a checkable invariant, and hands A3 its iteration order for free. Then `world_matrix` as one forward pass, with the origin as `t = (x,y) − A·(ox,oy)`. In the event `node_add`'s nine parameters tripped `too-many-parameters`, and the cure — a `Place` struct with **declared defaults** so scale is `1.0` rather than the zero an omitted field takes — is the `omitted-field-zero` case answered at the declaration. |
| **A2** | S ✅ | *Done.* Walk the array in z-order, call `draw_sprite` (the mvp-taking form, **not** `draw_sprite_at`, whose helper is translate+scale only) so rotation works from the first frame. **Design call: the walk emits `DrawRect`/`DrawText`, the shape `lavition_ui` already produces** — adopting a proven command buffer instead of minting a rival one is what keeps a game's UI and the editor's UI on one pipeline. The effort is the comparison rig: the same content drawn twice, `gl_screenshot` both, compare bytes. |
| **A3a/b** | M | *A3a done.* Group by **contiguous runs of depth order that share an atlas** — *not* by atlas globally, which silently reorders two overlapping sprites drawn from different atlases and is a wrong picture, not a slow one (P1 is the cell that catches it). Under blending the order is **exact**, so only ADJACENT runs may merge: a scene interleaving two atlases sprite-by-sprite degenerates toward one draw call each. That is a packing problem, not a renderer one — see F1. Pack per-instance attributes (a 2×3 affine + uv rect + tint + alpha) into one float buffer, one `gl_draw_instanced` per run, new shader with instanced attributes. The cost is stride/offset bookkeeping — `gl_instance_attrib` takes `stride_floats`/`offset_floats` and a wrong one fails as garbage geometry, silently — plus re-uploading only what changed. A2's path stays alive to compare against. |
| **A4** | S ✅ | *Done.* Stable z sort (insertion order breaks ties), reverse-iterate to pick, invert the world affine in closed form (no general 4×4 inverse) to take a screen point node-local. **The part that is always forgotten is capture** — the node that received the press receives the release even when the pointer has left it — and it is exactly what D1 tests. |
| **A5** | S ✅ | *Done.* Two more instance attributes, plus the shader moving off alpha-discard to real blending. **Design call: premultiplied alpha.** The canvas packs straight 0xAARRGGBB, so the packer premultiplies once at pack time; straight alpha under linear filtering darkens every anti-aliased edge, and this stack draws overlapping soft-edged sprites all day. Hand-compute the expected RGBA against that choice. |
| **A6** | S ✅ | *Done.* `gl_scissor` per clipped subtree, nested clips intersected with the parent rect. S rather than XS **because it interacts with A3**: a scissor change breaks a batch, so grouping becomes (atlas, clip) rather than atlas. |
| **P1** | S | Three knobs, and **`stage` learns nothing about hexes or 3D**. GameMaker's, because a game author already has the vocabulary: a sprite **origin** (put it bottom-centre and the sprite stands up from its footprint), a per-node **`depth`** the app sets — `depth = -y` is the whole 2.5D idiom, and a hex world writes the projection of `(q, r, height)` instead — and a **`layer`** that outranks depth, so background / world / UI are bands rather than a global number every node has to get right. The effort is A3's run-grouping and getting *origin, not extent* right in both places; the sort itself is a comparison. |
| **P2** | S ✅ | *Done.* One `(dx, dy)` per stage and one float per layer, applied as a **shader uniform** at draw time. **Design call: parallax translates, it does not scale.** Scaling by depth is a perspective projection, and it would resize the footprint a base-anchored sprite is aligned to — so the world layer sits at `1.0`, backgrounds below, foreground overlays above, and every cell stays the same size. The cost this avoids is the point: baking the camera into node positions makes a scroll an O(N) rewrite **and a full instance re-upload every frame**, which is A3's whole budget spent on standing still. Picking pays for it — A4 must un-apply the camera **per layer**, since one screen point is a different world point in each. |
| **P3** | XS ✅ | *Done.* Phase from `(time, position)` in the shader — no stored state, so a field of grass is one recorded batch and a time uniform. This is the case P4's frame-index attribute exists to keep cheap, taken to its limit: **nothing per frame at all**. |
| **P4** | S ✅ | *Done.* A sequence is (first cell, count, fps, loop mode); a node carries (sequence, elapsed). Advanced from `fixstep`'s step, never from wall time — that is what makes it identical at any frame rate and replayable under a recorded input stream. **Design call: the instance attribute is a FRAME INDEX, not a uv rect**, and the shader derives uv from the atlas layout — so an animating sprite dirties one integer per frame instead of four floats, and A3's *upload only what changed* keeps meaning something with 500 animated tufts on screen. |
| **P5** | S ✅ | *Done.* Two models, and the projection picks — not the author. Top-down gets continuous rotation off the 2×3 affine, which is why crawler authors every mob facing *up* and pre-rotated frames never exist. Side-on gets the `(action, facing)` table with one rule: switching sequences **carries the elapsed phase over**. Crawler's by-name resolution comes with it — a mob loads `<key>.png`, an optional action variant, and a missing file falls back rather than failing. Walking between hexes stays composition: C tweens the position, P4 plays the cycle. |
| **L1** | S ✅ | *Done.* Sample each light at the sprite's **origin** — its footprint, the same point P1 sorts on — and fold the result into the tint attribute. Order-independent, one pass, no framebuffer. This is the whole feature for a flat-lit 2-D game and it is deliberately first: L2 is only worth its pass when lights must fall across the scene rather than across the sprites. |
| **L2** | M | World layers into an offscreen FBO, lights accumulated, one fullscreen composite, **HUD drawn after and unlit** — the shape crawler shipped as R6 and verified. Every entry point exists (`gl_create_framebuffer`, `gl_framebuffer_texture`, `gl_create_color_texture`, `gl_draw_fullscreen_quad`). A multiply composite after the scene is order-independent, so it does not fight *never reorder*. Visibility stays the app's: the light **presents**, it does not decide what is seen. |
| **L3** | M | A layer already carries a parallax factor (P2); give it a blur radius, a fog colour and a density, and *distant, hazy, out-of-focus* becomes **layer data rather than an effects pipeline**. Fog is a `lerp` toward the fog colour — a uniform, essentially free. Blur rides L2's FBO: render the layer at quarter resolution and upsample with linear filtering, the cheap approximation backgrounds are made of. **Default to a BAKED blur** — the packer pre-blurs a static layer, so it costs nothing at run time; runtime blur is opt-in for a radius that actually changes (a focus pull). Atmosphere in this style is also largely particles, which `lib_plans/76-particles` gets cheaply once A lands. |
| **P6** | S ✅ | *Done.* ⚠ Its stated lean on L2 did **not** apply — arc L is unbuilt, and it turned out P6 needs no FBO at all: a view is an offset plus a scissor, so the composite pass L2 will bring is an addition rather than a prerequisite. Two cameras over one stage. The split it enforces is the useful part: **world deterministic and replicated, presentation local and free** — window size, camera, particles and ambient sway may differ per client, and must be allowed to. |
| **G** | H | Deferred. A path rasterizer with AA fills, gradients and stroke joins is the one genuinely research-shaped item here, which is why it is behind a trigger rather than in the queue. |

## A third vehicle: dryopea's tower defence in 2.5-D

The consumer asked for it, and it is the case this arc was designed against, so it belongs in
the plan rather than in a conversation.

**The rules are the game's own, unchanged — only the presentation differs.** That is the
whole claim of this arc, arriving from a consumer rather than from the design: dryopea's flow
field, combat resolution, wave system and carry model are not forked, re-tuned or
reimplemented for a 2.5-D view. The simulation stays where it is and `stage` draws it.

**Buildings, walls and hills are authored in 3-D** — the intended long-term path, not a demo
shortcut: the 3-D routines already draw them and will keep doing so. Those meshes project to
a footprint and a depth; mobs, workers and the player are sprites that sort against them by
the same key. The world stays 3-D; only the view is flat.

**So the demo is also the strongest available gate.** With one simulation behind two
presentations, they must agree about what is where — a pick at one screen point resolves to
the same entity in both, and an entity standing on a tile is on that tile in either view. A
plan that only ever renders one way cannot ask that question; this consumer can.

Three things that shape follows from, all now built or named:

- **Depth bands are LAYERS.** "Very blurry in the far distance, sharp up close" is not a
  per-node blur — it is a handful of bands, each with its own parallax factor (`P2`), blur
  and fog (`L3`), and cue range. A mob crossing from background work to the fight migrates
  between bands. Per-node blur would cost a pass each; per-band costs one.
- **The cue scales about the origin**, so a shrinking distant worker keeps its feet on its
  tile. That is `P1`, and it only works because the origin is the anchor (`A1`).
- **Riding left/right is a camera pan**, which is `P2` — one uniform per layer, not an O(N)
  rewrite of every node. Distant mobs already working when the player arrives is then just
  scene content at a far depth, not a spawning system.

⚠ **`dryopea` is a consumer tree: read it, never write to it.** What this plan owes that demo
is the capability and the recipe; the game is theirs to build.

## The vehicle

**Brick Buster II**, rebuilt arc by arc across all three plans, with the shipped 1983-line
version kept as the baseline. At each arc boundary record two numbers here: frames pixel-comparable against the
baseline, and line count — which must go **down**. A rewrite that only moves lines between
files is a failed arc, and the count is what says so.

**A second vehicle, once P1 lands: a 2.5-D sample** — a hex footprint, sprites standing up
from it, mobs walking behind a fence. Brick Buster proves the stack is *enough* for a flat
game; this proves the presentation model. It is a sample rather than a port because a port
runs on someone else's tree and clock, not because of its size.

If lavition ever presents in 2-D it brings a gate no sample can — **the 2-D and 3-D views of
one world must agree about what is where**, so a pick at one screen point answers the same
`(q, r, height)` in both. Worth taking if offered; not something this plan waits on.

## Phase ordering

~~**`A0` first**~~ — **done.** It did not kill the design: batching wins on native and the win
**grows with N**, which is the shape A3 needs. It found something else instead — **the four
instancing primitives are not bridged to WebGL2 at all**, so A3's batched path would draw
nothing in a browser. That is now `A3`'s stated prerequisite. Next: `A1 → A2 → A3`.

**`A3` is the phase every later one can quietly ruin.** The camera (`P2`) and the animation
attribute (`P4`) each turned out to decide whether its upload path survives, and both were
only visible from the phase *after* it. So ask any new per-node property **what does this
dirty per frame?** before adding it.

| Arc | Waits on | Then |
|---|---|---|
| **A** scene | ✅ **complete** | `A0`–`A6` all shipped as `stage` 0.6.0 |
| **P** presentation | ✅ **complete** | `P1`–`P6` all shipped as `stage` 0.12.0 |
| **L** light | `A5`'s tint (`L1`), `A3` (`L2`/`L3`) | `L1` ✅ — the whole feature for a flat-lit game → `L2` · `L3` |
| **G** vector paths | deferred behind its trigger | — |

Sibling plans fan out from here: [@PLN145](../145-authoring-libs/README.md)'s `B1` needs
`A3`, `C` needs `A1`, `D` needs `A4`; [@PLN146](../146-content-delivery/README.md)'s `F4`
needs `A2`. Both have phases that wait on nothing and should start immediately — `B0` and
`E1`.

## Open design questions

1. ~~**Chunk ownership.**~~ **Settled: `stage` lives in `loft-libs-graphics`** (shipped there
   as v0.1.0). The rest as proposed: `text2d`, `ui` → `loft-libs-graphics`;
   `tween`, `audio_bus` → `loft-libs-game`; `assets` (pack tool + loader) →
   `loft-libs-assets`, which is already the headless/no-GPU chunk. Cross-chunk deps
   are normal — `graphics` already depends on `mesh3d`/`glb`.
2. **Does `stage` replace `Painter2D`?** No — it sits above it, and A2's parallel run
   needs both alive. `Painter2D` stays as the escape hatch for a consumer that wants
   the immediate-mode path.
3. **Who owns the frame loop?** The consumer, via `fixstep`. `stage` renders when
   asked; it does not pump. A library that owns the loop cannot be composed with one
   that also does.
4. **What does a store miss do under `--html` today?** [LAZY_STORES.md](../../LAZY_STORES.md)
   fetches on a miss, which is right for a document and wrong for a frame loop.
   Confirm the current behaviour before F3 designs around it.
5. **G's trigger.** Open vector paths when a consumer needs resolution-independent
   art — a UI that scales across DPI, a zoomable map. Until then sprites + atlas
   cover the cases, and a path rasterizer is the one genuinely research-shaped item
   in this plan.

## Cross-arc dependencies

- **`loft-libs-graphics` has five open PRs**, including work on `graphics` itself
  (`gap01-input-events`, `reconcile-graphics-0.3.0`, `tuxedo-canvas-writable-ref`).
  New packages are new subdirectories and do not collide, but A3 may need a
  `gl_draw_instanced` fix that lands **in** `graphics` — sequence that against those
  PRs rather than against this plan.
  See [LIBRARY_BRANCHES.md](../../LIBRARY_BRANCHES.md).
- **@PLN141** (worked examples) — every package this plan adds owes its `@TAG` worked
  examples on arrival, not as a later sweep.
- **moros plan 19** (extract lavition) — the lavition libraries already carry **no Moros
  dependency** (`layering.sh` silent with `KNOWN=""`); what blocks the move is the
  *program*, `editor_server.loft`, still importing `moros_render` (42 sites) and
  `moros_sim` (11). D0 rides that plan rather than competing with it.
- **moros plan 22** (the pages client) — the working precedent for F1/F3, and the tree
  where a regression in `stage` or `assets` would show up first.
- **Package authoring + the sandbox boundary** — both apply to every package across the
  three plans and are stated once in each. ⚠ **A TYPE name is global too, not just a module
  basename**: `stage`'s `Node` was refused outright because `mesh3d` (pulled in by `graphics`)
  already publishes one, so it shipped as `StageNode`. The compiler refusing is the good
  outcome — after publishing, the rename is one nobody can make. Also: `use <pkg>` inside `<pkg>` means the *package*
  (loft#976's lip), and each package declares **trusted engine** or **admissible loft** while
  its API is being written, because afterwards it is a re-architecture.
- **[`lib_plans/64-game-client`](../../lib_plans/64-game-client/README.md)** — **co-op lives there**, not
  here. Its rule falls out of this plan's presentation model: the stage is a pure function of
  world and camera, so replicate the **world** and let each client derive its own scene —
  which is what makes crew_punk's six-phones-six-panels shape work at all. The determinism it
  needs is already maintained by `fixstep` + `P4` + `P3`; what is missing is only the harness
  that would catch losing it, armed *before* its subject (hexbody's `L7`).
- **`lib_plans/72-renderer-backend-boundary`** (GFX.PORTABLE) — `stage` must reach the
  GPU through the `Renderer` contract, not raw `gl_*`, or it becomes the next thing
  blocking a wgpu backend.
- **`lib_plans/76-particles`, `lib_plans/75-physics-2body`** — both become cheap once A
  lands (a particle system is a batched node, a body is a node with a velocity), so neither
  needs its own renderer.

## See also

- [`lib_plans/58-graphics/`](../../lib_plans/58-graphics/README.md) — the layer this builds on.
- [REMOTE_STORES.md](../../REMOTE_STORES.md) — the asset route (arc F).
- [`../../../tools/brick-buster/25-brick-buster.loft`](../../../tools/brick-buster/25-brick-buster.loft) — the baseline and the vehicle.
- [HTML_EXPORT.md](../../HTML_EXPORT.md) / [BROWSER_INTEROP.md](../../BROWSER_INTEROP.md) — the browser target arc E fixes.
- [@PLN145](../145-authoring-libs/README.md) — text, tweens, widgets above this stage.
- [@PLN146](../146-content-delivery/README.md) — the asset pack, fonts, browser audio.
- [@PLN147](../147-content-editor/README.md) — the in-browser editor; `A4` and `P1` are its
  gates too, checked by two consumers rather than restated. Its arc `X` **produces** what `P4`
  plays: a walk cycle authored as keyframes on named marks, baked to atlas cells at pack time,
  so the runtime keeps knowing nothing about how the art was made.
- [loft-lang/plans#144](https://github.com/loft-lang/plans/issues/144).
