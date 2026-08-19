<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN144 — The 2-D stage

> Tracker: [loft-lang/plans#144](https://github.com/loft-lang/plans/issues/144)
> (`subject:libs`, `status:future`). Everything here sits **on top of the shipped
> packages** — `graphics` 0.5.5, `input`, `fixstep`, `shapes`, `imaging`. Arcs A–D
> and F need no engine change; arc E is one stub in this repo.

## Status

**Open — design ready, nothing built.** The runtime is not the gap: `loft --html` already
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

## Effort + design

- **Effort:** MH — 17 phases in three arcs, **none above M**; see § Effort per phase
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
| **A0** — falsify the batching premise | probe only | frames/s + draw-call count at N = 100/1000/5000 sprites, per-sprite vs one `gl_draw_instanced` batch, on `--native` **and** in-browser. Red if the batch is not materially cheaper — which kills A3 before anything is built on it | Open |
| **A1** — node tree + transform composition | `stage` | composed world matrices for a 3-deep tree with rotation + non-uniform scale + pivot equal hand-computed `math::mat4_*` products; both backends. Red on multiply order or pivot handling | Open |
| **A2** — draw the tree through the **existing** per-sprite path, emitting `DrawRect`/`DrawText` | `stage` | a stage-drawn frame is **pixel-identical** to the hand-written immediate-mode draw of the same content (`save_png` + compare). Parallel run: both paths live. **Second gate:** a `lavition_ui` panel's `panel_draw_list` renders unmodified through this path — if it needs a shim, the command shape is wrong | Open |
| **A3** — batched renderer behind the same API | `stage` | pixels identical to A2 **and** draw calls drop from N to O(atlases). Both halves — same pixels alone would pass a batch that silently fell back | Open |
| **A4** — depth order, hit-test, input routing | `stage` | a headless pick table over a known overlapping tree: every (x, y) resolves to the hand-computed node, under rotation and inside a clip — **and a point over a sprite's TRANSPARENT texel resolves to what is behind it**, not to the sprite. A rect test cannot: you can see through the tree, so you must be able to click through it. Under P2 the un-projection is **per layer** — one screen point is a different world point in each — and the answer is the topmost layer whose alpha test passes | Open |
| **A5** — per-node alpha + tint, and **blending instead of discard** | `stage` | alpha 0.5 over a known background composites to the hand-computed RGBA; a sprite's **anti-aliased edge** composites without a fringe; tint × texel matches. Gate by **exact-colour bucketing with an `other` bucket that must read 0** (dryopea's `RENDERER.md` § R0 technique) over deliberately flat colours — a byte-diff says *different*, a classification says *what*. The shipped `SPRITE_FRAG` does `if (c.a < 0.01) discard` and writes everything else opaque, with `GL_BLEND` never enabled — a binary cutout, so soft edges and semi-transparency are wrong today and per-node alpha is unimplementable on it | Open |
| **A6** — clip / mask rect | `stage` | content outside the mask is absent at the exact pixel boundary, nested two deep | Open |
| *— arc **P**: the presentation model —* | | | |
| **P1** — sprite origin, `layer` + `depth`, so a 2D scene presents a 3D world | `stage` | a hand-computed occlusion table over a lattice with stacked heights: a sprite whose origin sits on row `r` occludes `r−1` and never `r+1` **however far up its pixels reach**, within a cell order is by height, `(row, height)` ties break by `q`, and a layer always outranks any depth inside another — **all of it with the sprites split across two atlases**, the cell A3 gets wrong if batching groups globally. Second gate, run by the vehicle: picking one screen point in the **2D and 3D presentations of the same world answers the same `(q, r, height)`** | Open |
| **P2** — camera with a per-layer parallax factor | `stage` | with every factor `1.0`, a camera pan is **pixel-identical** to translating every node by hand — the flat mode proved to be the degenerate case, not a second path. With factors varying, a pan of `d` moves each layer by `d × factor`, hand-computed per layer. **And a pure camera move re-uploads no instance data** (assert zero uploads), which is the entire reason the camera lives here and not in the node positions | Open |
| **P3** — world sprites: ambient motion, no per-instance state | `stage` | 500 wind-swayed sprites cost **zero per-frame instance updates** — the phase is derived from time and position, not stored and stepped — and two instances at different positions are visibly out of phase rather than marching in lockstep | Open |
| **P4** — mob animation: sequences, rate, loop mode | `stage` | one loop's worth of ticks returns to frame 0 exactly; the frame sequence at **30 Hz equals the sequence at 60 Hz** sampled at the same times, so it is frame-rate independent and replayable; a `once` animation stops on its last frame without wrapping; **ping-pong reverses without repeating the end frame**, the classic off-by-one | Open |
| **P5** — facings, **two models chosen by projection** | `stage` | *Top-down* (crawler's): one sprite authored in a locked orientation, **rotated continuously** to the facing — 15° steps must cost **no extra atlas entries**, since pre-rotated frames are what this avoids. *Side-on / 2.5D*: an `(action, facing)` table, because a standing sprite cannot be rotated into another facing — at most mirrored — and there a facing change mid-walk must **keep the frame phase** rather than snapping the legs back to frame 0 | Open |
| *— arc **L**: light and atmosphere —* | | | |
| **L1** — light sampled per sprite, applied as tint | `stage` | a sprite at distance `d` from a light takes the hand-computed falloff; two at equal distance take equal tint; and light **composes with** A5's material tint rather than overwriting it. Rides A5's existing attribute, so it costs no pass and cannot disturb draw order | Open |
| **L2** — light-map composite pass | `stage` | falloff along a ray from the light is **monotonic** and never undercuts its floor — the invariant, since hand-computing a curve per pixel is not a gate anyone maintains. Expectations **generated from the same constants the shader uses** (crawler's `light_cone.py` technique), so retuning happens in one place. And the HUD's pixels are **bit-identical with the light on and off**, which is how *the HUD draws after, unlit* stops being a comment | Open |
| **L3** — per-layer atmosphere: blur + fog | `stage` | fog at density 0 is **bit-identical to no fog** and at density 1 is exactly the fog colour — the degenerate cases proved, as with P2's factor `1.0`. A blurred layer **preserves total luminance** (edge clamping darkening the border is the classic bug). And blur is applied **per layer before it composites**, so a sharp foreground over a blurred background stays sharp — which a fullscreen blur cannot do and is the entire point | Open |
| **P6** — several views over one stage | `stage` | split-screen: two cameras, two composites, one scene — each view's frame is identical to the same camera rendered alone, so a second view cannot perturb the first | Open |
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
| **A0** | XS | Two probe programs — an N-times `draw_sprite` loop, and one instance-buffer upload plus a single `gl_draw_instanced`. Time 300 frames at N = 100/1000/5000 on both targets; the browser half reuses the headless-Chrome harness `tests/html_render.rs` already has. No library code. |
| **A1** | S | **Design call: the tree is a flat `vector<Node>` with integer parent indices, not pointer-linked.** A parent holding children while a child points back is a dependency cycle in loft's ownership model; the flat form sidesteps it entirely, keeps insert-order-is-parents-first as a checkable invariant, and hands A3 its iteration order for free. Then `world_matrix` as one forward pass, with pivot as `T(p)·R·T(−p)`. |
| **A2** | S | Walk the array in z-order, call `draw_sprite` (the mvp-taking form, **not** `draw_sprite_at`, whose helper is translate+scale only) so rotation works from the first frame. **Design call: the walk emits `DrawRect`/`DrawText`, the shape `lavition_ui` already produces** — adopting a proven command buffer instead of minting a rival one is what keeps a game's UI and the editor's UI on one pipeline. The effort is the comparison rig: the same content drawn twice, `gl_screenshot` both, compare bytes. |
| **A3** | M | Group by **contiguous runs of depth order that share an atlas** — *not* by atlas globally, which silently reorders two overlapping sprites drawn from different atlases and is a wrong picture, not a slow one (P1 is the cell that catches it). Under blending the order is **exact**, so only ADJACENT runs may merge: a scene interleaving two atlases sprite-by-sprite degenerates toward one draw call each. That is a packing problem, not a renderer one — see F1. Pack per-instance attributes (a 2×3 affine + uv rect + tint + alpha) into one float buffer, one `gl_draw_instanced` per run, new shader with instanced attributes. The cost is stride/offset bookkeeping — `gl_instance_attrib` takes `stride_floats`/`offset_floats` and a wrong one fails as garbage geometry, silently — plus re-uploading only what changed. A2's path stays alive to compare against. |
| **A4** | S | Stable z sort (insertion order breaks ties), reverse-iterate to pick, invert the world affine in closed form (no general 4×4 inverse) to take a screen point node-local. **The part that is always forgotten is capture** — the node that received the press receives the release even when the pointer has left it — and it is exactly what D1 tests. |
| **A5** | S | Two more instance attributes, plus the shader moving off alpha-discard to real blending. **Design call: premultiplied alpha.** The canvas packs straight 0xAARRGGBB, so the packer premultiplies once at pack time; straight alpha under linear filtering darkens every anti-aliased edge, and this stack draws overlapping soft-edged sprites all day. Hand-compute the expected RGBA against that choice. |
| **A6** | S | `gl_scissor` per clipped subtree, nested clips intersected with the parent rect. S rather than XS **because it interacts with A3**: a scissor change breaks a batch, so grouping becomes (atlas, clip) rather than atlas. |
| **P1** | S | Three knobs, and **`stage` learns nothing about hexes or 3D**. GameMaker's, because a game author already has the vocabulary: a sprite **origin** (put it bottom-centre and the sprite stands up from its footprint), a per-node **`depth`** the app sets — `depth = -y` is the whole 2.5D idiom, and a hex world writes the projection of `(q, r, height)` instead — and a **`layer`** that outranks depth, so background / world / UI are bands rather than a global number every node has to get right. The effort is A3's run-grouping and getting *origin, not extent* right in both places; the sort itself is a comparison. |
| **P2** | S | One `(dx, dy)` per stage and one float per layer, applied as a **shader uniform** at draw time. **Design call: parallax translates, it does not scale.** Scaling by depth is a perspective projection, and it would resize the footprint a base-anchored sprite is aligned to — so the world layer sits at `1.0`, backgrounds below, foreground overlays above, and every cell stays the same size. The cost this avoids is the point: baking the camera into node positions makes a scroll an O(N) rewrite **and a full instance re-upload every frame**, which is A3's whole budget spent on standing still. Picking pays for it — A4 must un-apply the camera **per layer**, since one screen point is a different world point in each. |
| **P3** | XS | Phase from `(time, position)` in the shader — no stored state, so a field of grass is one recorded batch and a time uniform. This is the case P4's frame-index attribute exists to keep cheap, taken to its limit: **nothing per frame at all**. |
| **P4** | S | A sequence is (first cell, count, fps, loop mode); a node carries (sequence, elapsed). Advanced from `fixstep`'s step, never from wall time — that is what makes it identical at any frame rate and replayable under a recorded input stream. **Design call: the instance attribute is a FRAME INDEX, not a uv rect**, and the shader derives uv from the atlas layout — so an animating sprite dirties one integer per frame instead of four floats, and A3's *upload only what changed* keeps meaning something with 500 animated tufts on screen. |
| **P5** | S | Two models, and the projection picks — not the author. Top-down gets continuous rotation off the 2×3 affine, which is why crawler authors every mob facing *up* and pre-rotated frames never exist. Side-on gets the `(action, facing)` table with one rule: switching sequences **carries the elapsed phase over**. Crawler's by-name resolution comes with it — a mob loads `<key>.png`, an optional action variant, and a missing file falls back rather than failing. Walking between hexes stays composition: C tweens the position, P4 plays the cycle. |
| **L1** | S | Sample each light at the sprite's **origin** — its footprint, the same point P1 sorts on — and fold the result into the tint attribute. Order-independent, one pass, no framebuffer. This is the whole feature for a flat-lit 2-D game and it is deliberately first: L2 is only worth its pass when lights must fall across the scene rather than across the sprites. |
| **L2** | M | World layers into an offscreen FBO, lights accumulated, one fullscreen composite, **HUD drawn after and unlit** — the shape crawler shipped as R6 and verified. Every entry point exists (`gl_create_framebuffer`, `gl_framebuffer_texture`, `gl_create_color_texture`, `gl_draw_fullscreen_quad`). A multiply composite after the scene is order-independent, so it does not fight *never reorder*. Visibility stays the app's: the light **presents**, it does not decide what is seen. |
| **L3** | M | A layer already carries a parallax factor (P2); give it a blur radius, a fog colour and a density, and *distant, hazy, out-of-focus* becomes **layer data rather than an effects pipeline**. Fog is a `lerp` toward the fog colour — a uniform, essentially free. Blur rides L2's FBO: render the layer at quarter resolution and upsample with linear filtering, the cheap approximation backgrounds are made of. **Default to a BAKED blur** — the packer pre-blurs a static layer, so it costs nothing at run time; runtime blur is opt-in for a radius that actually changes (a focus pull). Atmosphere in this style is also largely particles, which `lib_plans/76-particles` gets cheaply once A lands. |
| **P6** | S | Two cameras over one stage, two composite passes (L2 already builds one). The split it enforces is the useful part: **world deterministic and replicated, presentation local and free** — window size, camera, particles and ambient sway may differ per client, and must be allowed to. |
| **G** | H | Deferred. A path rasterizer with AA fills, gradients and stroke joins is the one genuinely research-shaped item here, which is why it is behind a trigger rather than in the queue. |

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

**`A0` first — it can kill the design for the cost of a compile**, which no other phase here
can. Then the critical path `A1 → A2 → A3`.

**`A3` is the phase every later one can quietly ruin.** The camera (`P2`) and the animation
attribute (`P4`) each turned out to decide whether its upload path survives, and both were
only visible from the phase *after* it. So ask any new per-node property **what does this
dirty per frame?** before adding it.

| Arc | Waits on | Then |
|---|---|---|
| **A** scene | — | `A0` → `A6` in order |
| **P** presentation | `A3` | `P1` → `P6` |
| **L** light | `A5`'s tint (`L1`), `A3` (`L2`/`L3`) | `L1` alone is the whole feature for a flat-lit game |
| **G** vector paths | deferred behind its trigger | — |

Sibling plans fan out from here: [@PLN145](../145-authoring-libs/README.md)'s `B1` needs
`A3`, `C` needs `A1`, `D` needs `A4`; [@PLN146](../146-content-delivery/README.md)'s `F4`
needs `A2`. Both have phases that wait on nothing and should start immediately — `B0` and
`E1`.

## Open design questions

1. **Chunk ownership.** Proposed: `stage`, `text2d`, `ui` → `loft-libs-graphics`;
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
  three plans and are stated once in each: `use <pkg>` inside `<pkg>` means the *package*
  (loft#976's lip), and each package declares **trusted engine** or **admissible loft** while
  its API is being written, because afterwards it is a re-architecture.
- **[`lib_plans/64-game-client`](../64-game-client/README.md)** — **co-op lives there**, not
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

- [`lib_plans/58-graphics/`](../58-graphics/README.md) — the layer this builds on.
- [REMOTE_STORES.md](../../REMOTE_STORES.md) — the asset route (arc F).
- [`../../../tools/brick-buster/25-brick-buster.loft`](../../../tools/brick-buster/25-brick-buster.loft) — the baseline and the vehicle.
- [HTML_EXPORT.md](../../HTML_EXPORT.md) / [BROWSER_INTEROP.md](../../BROWSER_INTEROP.md) — the browser target arc E fixes.
- [@PLN145](../145-authoring-libs/README.md) — text, tweens, widgets above this stage.
- [@PLN146](../146-content-delivery/README.md) — the asset pack, fonts, browser audio.
- [@PLN147](../147-content-editor/README.md) — the in-browser editor; `A4` and `P1` are its
  gates too, checked by two consumers rather than restated.
- [loft-lang/plans#144](https://github.com/loft-lang/plans/issues/144).
