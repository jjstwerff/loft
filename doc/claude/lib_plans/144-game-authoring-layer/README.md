<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN144 — The 2D game-authoring layer

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

A game is a **tree you mutate**, not a frame loop you draw: ship `stage`, `text2d`,
`tween`, `ui`, an asset route and browser audio, and rebuild Brick Buster on them at
**≤ 600 lines** while it gains rotation, per-node alpha, tint, music and live text.

**Scope: 2-D games, at any scale — not a 3-D engine.** The 2.5-D half is a *sprite
presentation* of a 3-D world (a hex or grid footprint, sprites standing up from it) and it
stops there: no meshes, no camera projection. **Lighting, fog and background blur are in
scope** — a torch, a day/night cycle, a hazy blurred distance — as tint, a composite pass and
per-layer atmosphere (A11–A13), the Hollow Knight / Silksong pipeline rather than a 3-D
lighting model. dryopea's 3-D renderer and moros's 3-D editor are where lessons came
from, **not consumers this plan serves**: the test of any proposal here is whether a 2-D game
needs it, never how big the game is.

## Effort + design

- **Effort:** H overall — 36 phases, **none above M** (9 XS, 21 S, 6 M), plus **D0**, an upstream request rather than work; see § Effort per phase
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
| **A4** — depth order, hit-test, input routing | `stage` | a headless pick table over a known overlapping tree: every (x, y) resolves to the hand-computed node, under rotation and inside a clip — **and a point over a sprite's TRANSPARENT texel resolves to what is behind it**, not to the sprite. A rect test cannot: you can see through the tree, so you must be able to click through it. Under A8 the un-projection is **per layer** — one screen point is a different world point in each — and the answer is the topmost layer whose alpha test passes | Open |
| **A5** — per-node alpha + tint, and **blending instead of discard** | `stage` | alpha 0.5 over a known background composites to the hand-computed RGBA; a sprite's **anti-aliased edge** composites without a fringe; tint × texel matches. Gate by **exact-colour bucketing with an `other` bucket that must read 0** (dryopea's `RENDERER.md` § R0 technique) over deliberately flat colours — a byte-diff says *different*, a classification says *what*. The shipped `SPRITE_FRAG` does `if (c.a < 0.01) discard` and writes everything else opaque, with `GL_BLEND` never enabled — a binary cutout, so soft edges and semi-transparency are wrong today and per-node alpha is unimplementable on it | Open |
| **A7** — sprite origin, `layer` + `depth`, so a 2D scene presents a 3D world | `stage` | a hand-computed occlusion table over a lattice with stacked heights: a sprite whose origin sits on row `r` occludes `r−1` and never `r+1` **however far up its pixels reach**, within a cell order is by height, `(row, height)` ties break by `q`, and a layer always outranks any depth inside another — **all of it with the sprites split across two atlases**, the cell A3 gets wrong if batching groups globally. Second gate, run by the vehicle: picking one screen point in the **2D and 3D presentations of the same world answers the same `(q, r, height)`** | Open |
| **A8** — camera with a per-layer parallax factor | `stage` | with every factor `1.0`, a camera pan is **pixel-identical** to translating every node by hand — the flat mode proved to be the degenerate case, not a second path. With factors varying, a pan of `d` moves each layer by `d × factor`, hand-computed per layer. **And a pure camera move re-uploads no instance data** (assert zero uploads), which is the entire reason the camera lives here and not in the node positions | Open |
| **A9w** — world sprites: ambient motion, no per-instance state | `stage` | 500 wind-swayed sprites cost **zero per-frame instance updates** — the phase is derived from time and position, not stored and stepped — and two instances at different positions are visibly out of phase rather than marching in lockstep | Open |
| **A9** — mob animation: sequences, rate, loop mode | `stage` | one loop's worth of ticks returns to frame 0 exactly; the frame sequence at **30 Hz equals the sequence at 60 Hz** sampled at the same times, so it is frame-rate independent and replayable; a `once` animation stops on its last frame without wrapping; **ping-pong reverses without repeating the end frame**, the classic off-by-one | Open |
| **A10** — facings, **two models chosen by projection** | `stage` | *Top-down* (crawler's): one sprite authored in a locked orientation, **rotated continuously** to the facing — 15° steps must cost **no extra atlas entries**, since pre-rotated frames are what this avoids. *Side-on / 2.5D*: an `(action, facing)` table, because a standing sprite cannot be rotated into another facing — at most mirrored — and there a facing change mid-walk must **keep the frame phase** rather than snapping the legs back to frame 0 | Open |
| **A11** — light sampled per sprite, applied as tint | `stage` | a sprite at distance `d` from a light takes the hand-computed falloff; two at equal distance take equal tint; and light **composes with** A5's material tint rather than overwriting it. Rides A5's existing attribute, so it costs no pass and cannot disturb draw order | Open |
| **A12** — light-map composite pass | `stage` | falloff along a ray from the light is **monotonic** and never undercuts its floor — the invariant, since hand-computing a curve per pixel is not a gate anyone maintains. Expectations **generated from the same constants the shader uses** (crawler's `light_cone.py` technique), so retuning happens in one place. And the HUD's pixels are **bit-identical with the light on and off**, which is how *the HUD draws after, unlit* stops being a comment | Open |
| **A13** — per-layer atmosphere: blur + fog | `stage` | fog at density 0 is **bit-identical to no fog** and at density 1 is exactly the fog colour — the degenerate cases proved, as with A8's factor `1.0`. A blurred layer **preserves total luminance** (edge clamping darkening the border is the classic bug). And blur is applied **per layer before it composites**, so a sharp foreground over a blurred background stays sharp — which a fullscreen blur cannot do and is the entire point | Open |
| **A6** — clip / mask rect | `stage` | content outside the mask is absent at the exact pixel boundary, nested two deep | Open |
| **B0** — a built-in fallback font | `text2d` | under `loft test`, with **no font file and no native library loaded**, a known string draws a known non-zero coverage — the state in which `graphics::draw_text` answers *native function not loaded* today. Consumer outcome, not a unit test: `dryopea/src/hud.loft` draws its digits as **rectangles** because of this, and `picker.loft` shipped with no labels for the same reason | Open |
| **B1** — glyph atlas + `TextNode.text` | `text2d` | mutate `.text` every frame for 600 frames: GL texture count **constant** (today one upload per change), pixels equal the `create_text_texture` baseline | Open |
| **B1m** — the metrics seam | `text2d` | a **wide run and a narrow run** of `n` characters, measured at startup through whichever backend resolved, answer *fixed-pitch or not* — one run cannot, and the browser's proportional substitution is exactly what it must catch. Advance carried in **1/64 px**: a whole-pixel field truncates 9.6→9 and the error accumulates per character | Open |
| **B2** — wrapping + alignment | `text2d` | a hand-computed break table (width → break positions) **per target**, **including multi-byte text** — `len(text)` counts characters and the byte-indexed read is the live trap. Not one shared table: native measures the real TTF through fontdue and the browser measures whatever family resolved, so the same string breaks in different places. The cross-target invariant is **self-consistency** — the drawn text fits the box that same target measured. Every estimate rounds **outward**, since an under-estimate overflows a box just proved to fit | Open |
| **C1** — tween core + easing set | `tween` | sampled values match a hand-computed easing table exactly; a completed tween lands **on** the end value, not end−ε; identical result at 30 Hz and 60 Hz | Open |
| **C2** — bind to node properties | `tween` | driving `node.x` through a tween yields the same pixel sequence as setting it by hand | Open |
| **D0** — publish `lavition_ui` | upstream | the package resolves from the registry and its own tests pass unchanged after the move. **Not our work and not our clock** — moros promotes a library once it is battle-tested *there*, by rule | Blocked on moros |
| **D1** — Button + Panel over stage routing | `ui` | a replayed `gl_next_event` sequence drives the exact state sequence; press-then-leave-then-release does **not** fire. **And `panel_hit_test` answers the same `UiHit` it answers today**, which is what makes this an extraction rather than a rewrite wearing its name | Open |
| **D2** — focus, tab order, text field | `ui` | replayed keystrokes incl. IME text produce the exact buffer; tab order matches the declared order. **The genuinely new half** — the kit has neither today | Open |
| **E1** — browser audio bridge | this repo | headless-Chrome page loads a clip: handle non-null, `audio_play` returns a sink. **Run it on the current tree first** — today it returns `i32::MIN` / `-1`, so the harness must go red before the fix | Open |
| **E2** — loop, pan, seek, stop-all | `graphics` | each round-trips on native and in-browser | Open |
| **E3** — `audio_bus` | `audio_bus` | bus gain composition matches hand-computed values; ducking restores exactly | Open |
| **F1** — the pack **is** a loft store, and it holds **scenes** as well as assets | `assets` | pack → read back: every asset byte-identical, **and** `type_layout_fingerprint` matches across native and wasm. If that check fails everything downstream is wrong. A scene is **definitions + placed instances** (GameMaker's object/room split), not a flat node dump — and a definition carries its **animation table**, `(action, facing) → sequence`, since a walk cycle is asset data and not code. A **light is a placed instance** like any other — the shape a prefab and an editor both need. In the first schema, because retrofitting costs a format break; and once scenes are in, reloading the store **is** hot reload | Open |
| **F2** — range-read loader | `assets` | the same game source runs from a local pack and from `python3 -m http.server` with only the URL changed; a byte-range log shows **only** the requested keys fetched | Open |
| **F3** — prefetch policy | `assets` | instrument the frame loop: **zero fetches inside a frame** during steady-state play | Open |
| **F4** — retire `build_atlas()` | vehicle | Brick Buster's 190 hand-poked lines become a packed asset; frames pixel-identical to the baked version | Open |
| **F5** — font sources: browser-resident, our server, or a CDN | `assets` | a page declaring each of the three sources resolves to the **requested** family, not the fallback. Assert the *resolved* family — text draws either way, so "text appeared" is not the gate. Red on a manifest that lets the declared `font-family` drift from the name the program passes. Field evidence rather than deduction: `moros/probe/b1` measured a desktop fixed-pitch font arriving as a **proportional** browser fallback | Open |
| **F6** — font readiness ordering | `assets` | with the font source **throttled**, the page still resolves to the requested family — i.e. the `document.fonts.load` await genuinely holds `loft_start`. Remove the await and this goes red while F5 stays green on a fast local font, which is why it is its own phase | Open |
| **H1** — the game's own logic runs **admitted**, not just its mods | `assets` + host | the negative gate, because a policy that admits everything proves nothing: **remove a capability from `[sandbox]` and the corresponding call must fail to LOAD**, with an actionable error. @PLN86 admission ships here (`src/sandbox.rs`) | Open |
| **H2** — every library in this plan declares its side of the boundary | all | game code compiled under the policy reaches `stage` / `tween` / `ui` / `assets` **only** through allow-listed capabilities, and a deliberately unbounded loop in game code is rejected at load rather than at frame 900 | Open |
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
- **[ASSETS.md](ASSETS.md)** — arc F: why the pack is a loft store on a dumb file server
  rather than an `[Embed]`-style bundler, and the two constraints that carry over from
  `routing`.
- **[FONTS.md](FONTS.md)** — F5/F6: reusing a font the browser has, and bringing one it does
  not.

## The presentation model

**The 2D view is a PRESENTATION of the 3D world, not a second world** — which is what a
GameMaker-shaped game already is: a room whose instances carry a `depth`, sprites whose
**origin** sits at their feet, and `depth = -y` doing the 2.5D work. The hex is only the
footprint; the sprite stands *up* from it, so where it sorts is decided by its origin and
never by its artwork. Get that backwards and a tall sprite sorts by its own top edge, which
is the classic 2.5D wrong picture.

So `stage` stays a 2D presenter and the world stays 3D in the app, with **three knobs as the
entire contract between them**: a projected position, a sprite origin, and `layer` + `depth`.
A plain 2D game sets all three trivially. That is A7; A3's run-grouping is what keeps it true
once batched.

**Two scroll modes, one mechanism.** Scrolling the whole world in place and scrolling the
front faster than the back are not two code paths: they are one camera with a **per-layer
parallax factor**, and the flat mode is every factor at `1.0`. That is A8, and it is why the
camera belongs to `stage` rather than to the app — an app-owned camera means rewriting every
node's position on every scrolled frame, which is exactly the O(N) per-frame work the
retained tree exists to avoid.

A **frame event** — a footstep on frame 3, a hitbox live on frames 4–6 — stays app-side for
the same reason occlusion does: the node's current frame is readable, so the app can act on
it, and a callback table in the library would be a mechanism for something already
expressible.

**Occlusion is the level designer's rule, and the engine gets no mechanism for it —
settled, not open.** A character walking behind a fence, a tree trunk, a window or a low
wall stays visible, because those things are narrow or mostly transparent and alpha does the
work. The rule is simply *do not place large solid objects in the foreground*. So there is
no cutaway, no fade-when-occluding, no height ceiling, and not even a *what covers my
subject* query: each buys a runtime mechanism to rescue a placement that should not exist.
It does make A4 and A5 load-bearing rather than polish — the rule holds only if a fence's
soft edge composites correctly and a click passes through its gaps. Should it ever need
help, the help is an **authoring-time check in the editor** (flag a placed sprite whose
solid region could hide a character behind it), never a runtime feature: advice at author
time, silence at run time.

## Effort per phase

Totals: **9 XS, 21 S, 6 M** — no phase above M, which is the § Cutting rule holding
rather than optimism. **D0** carries no letter: it is a request to another tree. Three phases carry a design call that decides the effort, and
those calls are made here rather than discovered mid-build.

| Phase | E | What the effort actually is |
|---|---|---|
| **A0** | XS | Two probe programs — an N-times `draw_sprite` loop, and one instance-buffer upload plus a single `gl_draw_instanced`. Time 300 frames at N = 100/1000/5000 on both targets; the browser half reuses the headless-Chrome harness `tests/html_render.rs` already has. No library code. |
| **A1** | S | **Design call: the tree is a flat `vector<Node>` with integer parent indices, not pointer-linked.** A parent holding children while a child points back is a dependency cycle in loft's ownership model; the flat form sidesteps it entirely, keeps insert-order-is-parents-first as a checkable invariant, and hands A3 its iteration order for free. Then `world_matrix` as one forward pass, with pivot as `T(p)·R·T(−p)`. |
| **A2** | S | Walk the array in z-order, call `draw_sprite` (the mvp-taking form, **not** `draw_sprite_at`, whose helper is translate+scale only) so rotation works from the first frame. **Design call: the walk emits `DrawRect`/`DrawText`, the shape `lavition_ui` already produces** — adopting a proven command buffer instead of minting a rival one is what keeps a game's UI and the editor's UI on one pipeline. The effort is the comparison rig: the same content drawn twice, `gl_screenshot` both, compare bytes. |
| **A3** | M | Group by **contiguous runs of depth order that share an atlas** — *not* by atlas globally, which silently reorders two overlapping sprites drawn from different atlases and is a wrong picture, not a slow one (A7 is the cell that catches it). Under blending the order is **exact**, so only ADJACENT runs may merge: a scene interleaving two atlases sprite-by-sprite degenerates toward one draw call each. That is a packing problem, not a renderer one — see F1. Pack per-instance attributes (a 2×3 affine + uv rect + tint + alpha) into one float buffer, one `gl_draw_instanced` per run, new shader with instanced attributes. The cost is stride/offset bookkeeping — `gl_instance_attrib` takes `stride_floats`/`offset_floats` and a wrong one fails as garbage geometry, silently — plus re-uploading only what changed. A2's path stays alive to compare against. |
| **A4** | S | Stable z sort (insertion order breaks ties), reverse-iterate to pick, invert the world affine in closed form (no general 4×4 inverse) to take a screen point node-local. **The part that is always forgotten is capture** — the node that received the press receives the release even when the pointer has left it — and it is exactly what D1 tests. |
| **A5** | S | Two more instance attributes, plus the shader moving off alpha-discard to real blending. **Design call: premultiplied alpha.** The canvas packs straight 0xAARRGGBB, so the packer premultiplies once at pack time; straight alpha under linear filtering darkens every anti-aliased edge, and this stack draws overlapping soft-edged sprites all day. Hand-compute the expected RGBA against that choice. |
| **A7** | S | Three knobs, and **`stage` learns nothing about hexes or 3D**. GameMaker's, because a game author already has the vocabulary: a sprite **origin** (put it bottom-centre and the sprite stands up from its footprint), a per-node **`depth`** the app sets — `depth = -y` is the whole 2.5D idiom, and a hex world writes the projection of `(q, r, height)` instead — and a **`layer`** that outranks depth, so background / world / UI are bands rather than a global number every node has to get right. The effort is A3's run-grouping and getting *origin, not extent* right in both places; the sort itself is a comparison. |
| **A8** | S | One `(dx, dy)` per stage and one float per layer, applied as a **shader uniform** at draw time. **Design call: parallax translates, it does not scale.** Scaling by depth is a perspective projection, and it would resize the footprint a base-anchored sprite is aligned to — so the world layer sits at `1.0`, backgrounds below, foreground overlays above, and every cell stays the same size. The cost this avoids is the point: baking the camera into node positions makes a scroll an O(N) rewrite **and a full instance re-upload every frame**, which is A3's whole budget spent on standing still. Picking pays for it — A4 must un-apply the camera **per layer**, since one screen point is a different world point in each. |
| **A9w** | XS | Phase from `(time, position)` in the shader — no stored state, so a field of grass is one recorded batch and a time uniform. This is the case A9's frame-index attribute exists to keep cheap, taken to its limit: **nothing per frame at all**. |
| **A9** | S | A sequence is (first cell, count, fps, loop mode); a node carries (sequence, elapsed). Advanced from `fixstep`'s step, never from wall time — that is what makes it identical at any frame rate and replayable under a recorded input stream. **Design call: the instance attribute is a FRAME INDEX, not a uv rect**, and the shader derives uv from the atlas layout — so an animating sprite dirties one integer per frame instead of four floats, and A3's *upload only what changed* keeps meaning something with 500 animated tufts on screen. |
| **A10** | S | Two models, and the projection picks — not the author. Top-down gets continuous rotation off the 2×3 affine, which is why crawler authors every mob facing *up* and pre-rotated frames never exist. Side-on gets the `(action, facing)` table with one rule: switching sequences **carries the elapsed phase over**. Crawler's by-name resolution comes with it — a mob loads `<key>.png`, an optional action variant, and a missing file falls back rather than failing. Walking between hexes stays composition: C tweens the position, A9 plays the cycle. |
| **A11** | S | Sample each light at the sprite's **origin** — its footprint, the same point A7 sorts on — and fold the result into the tint attribute. Order-independent, one pass, no framebuffer. This is the whole feature for a flat-lit 2-D game and it is deliberately first: A12 is only worth its pass when lights must fall across the scene rather than across the sprites. |
| **A12** | M | World layers into an offscreen FBO, lights accumulated, one fullscreen composite, **HUD drawn after and unlit** — the shape crawler shipped as R6 and verified. Every entry point exists (`gl_create_framebuffer`, `gl_framebuffer_texture`, `gl_create_color_texture`, `gl_draw_fullscreen_quad`). A multiply composite after the scene is order-independent, so it does not fight *never reorder*. Visibility stays the app's: the light **presents**, it does not decide what is seen. |
| **A13** | M | A layer already carries a parallax factor (A8); give it a blur radius, a fog colour and a density, and *distant, hazy, out-of-focus* becomes **layer data rather than an effects pipeline**. Fog is a `lerp` toward the fog colour — a uniform, essentially free. Blur rides A12's FBO: render the layer at quarter resolution and upsample with linear filtering, the cheap approximation backgrounds are made of. **Default to a BAKED blur** — the packer pre-blurs a static layer, so it costs nothing at run time; runtime blur is opt-in for a radius that actually changes (a focus pull). Atmosphere in this style is also largely particles, which `lib_plans/76-particles` gets cheaply once A lands. |
| **A6** | S | `gl_scissor` per clipped subtree, nested clips intersected with the parent rect. S rather than XS **because it interacts with A3**: a scissor change breaks a batch, so grouping becomes (atlas, clip) rather than atlas. |
| **B1** | M | Rasterize glyphs once into an atlas, keep a (font, size, codepoint) → uv map, build a text node as one quad per glyph fed through A3's buffer, so `.text =` re-lays-out quads and uploads nothing. Effort: shelf packing, atlas growth when it fills, and both backends producing the same atlas *shape* even where glyph pixels differ. |
| **B0** | S | A compact bitmap face baked in as data plus a pure-loft blitter — no file, no `#native`, no GL. Small, and it is the phase that unblocks a shipped consumer rather than one that makes an unshipped one faster: today the text path needs a GL context **and** a native rasteriser **and** a font file, so a repo that tests its UI headlessly answers by having no text. |
| **B1m** | XS | Two measured runs at startup, a 1/64-px advance, and three derived helpers (`text_width`, `fits`, `fit_text`). Nearly free — it is `lavition_ui/src/font.loft` lifted, and its shape is a **finding**, not a preference: one run cannot distinguish a fixed-pitch font from the browser's proportional stand-in, and whole-pixel truncation cost that tree a 31 px error on a single line. |
| **B2** | S | Greedy breaking on measured advances, three alignments, and the character-vs-byte trap — `len(text)` counts characters, the indexed read is bytes. Per-target break tables (see the Verify column). |
| **C1** | S | A tween is (setter, from, to, duration, easing, elapsed) driven off `fixstep`'s step, plus the standard easing table and sequencing — chain, parallel, delay, on-complete. Pure loft, no GPU. The exactness gate is a clamp everyone forgets: a finished tween must land **on** the end value. |
| **C2** | XS | loft has no property references, so tweenable properties are a small enum plus a write switch. Unelegant and correct; closures are the alternative if one arrives cheaply. |
| **D0** | — | A request, not an effort: `lavition_ui` is unpublished and lives in a tree this stream reads and never writes. Costs a conversation and their release cycle. |
| **D1** | S | Four visual states over A4's routing-with-capture, on top of an **extracted** `Button`/`Panel`/`ListBox`/`VerbBar`/`Theme` rather than a written one. The effort is the replay harness, and it constrains A4: the input path must be injectable. `input_tick_from_state` in the `input` package already exists for exactly this — reuse it rather than inventing a second seam. |
| **D2** | M | The half the kit does not have. Focus ring and tab order are small; the **text field** is the phase. Caret placement needs B2's measurement, selection needs hit-test to a character index, insertion/backspace/IME arrive via `gl_event_text`, and multi-byte indexing returns for a third time. |
| **E1** | XS | ~40 lines of JS. **Design call: `audio_load` is synchronous and `decodeAudioData` is not**, so it returns a handle immediately and the buffer lands later; a `play` on a still-decoding clip drops rather than queues — the same plan-then-use shape as the asset store. `play` builds BufferSource → GainNode, sinks go in a table that `stop`/`set_volume` index. |
| **E2** | S | Native: widen the `#native` signatures and the cdylib (rodio already does all four). Browser: `loop` on the source, `StereoPannerNode`, `start(when, offset)`. Both together, or they drift. |
| **E3** | S | Buses as a gain graph with per-bus volume and ducking. Pure composition over E2. |
| **F1** | M | Asset **and scene** record types, the packer (PNG in via `imaging`, audio bytes, scene records), `store_persist_bind` / `durable_seal`. Effort: the native-vs-wasm layout fingerprint check, and choosing the key granularity so `store_load_key` fetches a sensible page rather than one sprite or the whole file. **The packer also decides A3's batch count** — depth order cannot be rearranged under blending, so sprites that draw near each other must share an atlas, and premultiplication happens here, once. Scene records do not raise the M — they raise the schema's stakes, which is why they belong in the first cut. |
| **F2** | S | One call site — `store_load_key(s)` from a URL with a local-path fallback. The effort is *proving* only the requested ranges crossed the wire, which means a logging static file server in the test. |
| **F3** | S | An explicit request-these-keys call at load and level boundaries, a ring-around-player helper, and a counter that can assert zero fetches inside a frame. The instrumentation is the work; the policy is three lines. |
| **F4** | XS | Pack `build_atlas()`'s output as a PNG, load it from the pack, delete 190 lines, pixel-compare. |
| **F5** | S | Manifest fields (family, browser source, native path), page emission of the `@font-face` or `<link>`, and enforcing family-name-equals-lookup-key **at build time** instead of leaving it to be discovered as a silent fallback at runtime. |
| **F6** | XS | Emit the `document.fonts.load` await for each declared family ahead of `loft_start`. The fix is two lines; the throttled test is the phase. |
| **H1** | S | Turn the `[sandbox]` policy on over the game's own function surface and find where it is too tight — which is the point of doing it early. Doing it late is a re-architecture: the boundary decides which library internals may be unbounded, and that is a design-time property of every package this plan ships. |
| **H2** | S | Each package declares **trusted engine** (unbounded internals, an admitted-safe API) or **admissible loft**. Cheap while the APIs are being written, and the reason mods cost nothing later: a mod is then just more admitted code, with no second code path to keep in step. |
| **G** | H | Deferred. A path rasterizer with AA fills, gradients and stroke joins is the one genuinely research-shaped item here, which is why it is behind a trigger rather than in the queue. |

## The vehicle

**Brick Buster II**, rebuilt arc by arc, with the shipped 1983-line version kept as
the baseline. At each arc boundary record two numbers: frames pixel-comparable
against the baseline, and line count — which must go **down** and is written into
this table when it does. A rewrite that only moves lines between files is a failed
arc, and the count is what says so.

**A second vehicle, once A7 lands: a 2.5-D sample** — a hex footprint, sprites standing up
from it, mobs walking behind a fence. Brick Buster proves the stack is *enough* for a flat
game; this proves the presentation model. It is a sample rather than a port because a port
runs on someone else's tree and clock, not because of its size.

If lavition ever presents in 2-D it brings a gate no sample can — **the 2-D and 3-D views of
one world must agree about what is where**, so a pick at one screen point answers the same
`(q, r, height)` in both. Worth taking if offered; not something this plan waits on.

## Phase ordering

1. **E1 first** — it is XS, independent of everything, and turns silent browser
   games into games with music. Do not queue it behind the keystone.
2. **A0**, then A1 → A6 in order. A0 is the cheapest phase in the plan and the only
   one that can kill the design for the cost of a compile.
3. **B0 early** — it needs nothing from arc A and it unblocks dryopea today. Then the rest
   of **B**, **C**, **D** — each needs A4's hit-test or A1's transforms; D2 also needs B.
4. **F** runs in parallel with A–D (it touches no rendering), except F4, which needs
   A2. F5/F6 (fonts) are independent of F1–F3 and can land with B, which is the first
   arc that cares which font actually resolved.
5. **E2/E3** whenever a consumer asks; they are comfort, not capability.

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
- **Package authoring** — loft#976 makes a bare `use <mod>` bind the package's own file, so
  the six packages here are safe from a sibling's basename; the lip is that `use <pkg>`
  inside `<pkg>` means the *package* and a suite named `tests/<pkg>.loft` amputated nine
  libraries. Copy moros's `tools/basenames.sh` guard rather than reinventing it.
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
- [loft-lang/plans#144](https://github.com/loft-lang/plans/issues/144).
