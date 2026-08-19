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

**Open — design ready, nothing built.** The runtime is not the gap. `loft --html`
already beats Flash on deployment (self-contained page, no plugin, WebGL2,
E2E-gated in CI), and static types, compiled-WASM speed and stackful coroutines
all beat ActionScript 3. What is missing is the **layer a game author writes
against**: `graphics` ships a complete immediate-mode GL surface and nothing above
it, so every game re-implements the scene graph, the text field, the tweens and
the widgets by hand.

The measurement is `tools/brick-buster/25-brick-buster.loft` — **1983 lines for a
Breakout clone**, of which ~190 (`build_atlas()`, lines 112–301) are hand-poked
pixel art and ~40 more pre-bake one GL texture per string before gameplay starts,
because changing a string costs a texture upload. The AS3 equivalent is ~500 lines
with the art drawn in a tool.

## Goal

A game is a **tree you mutate**, not a frame loop you draw: ship `stage`, `text2d`,
`tween`, `ui`, an asset route and browser audio, and rebuild Brick Buster on them
at **≤ 600 lines** while it gains rotation, per-node alpha, tint, music and live
text.

## Effort + design

- **Effort:** H overall — 22 phases, **none above M** (5 XS, 13 S, 4 M); see § Effort per phase
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
clip, batched. Give each derived fact — the world matrix, the composed alpha, the
clip rect — **one home** every path reads, so the cells cannot disagree; the matrix
is how that is proved rather than asserted. This is exactly the bug class plan-58
shipped (an invariant re-derived per code path, validated only where the derivations
coincided).

Hand-compute every expected value. Agreement between two backends is not a pass.

## Sub-arcs

`Verify` is what would go **red if the phase were done wrong** — filled when the
phase is cut, not when it is implemented.

| Item | Where | Verify | Status |
|---|---|---|---|
| **A0** — falsify the batching premise | probe only | frames/s + draw-call count at N = 100/1000/5000 sprites, per-sprite vs one `gl_draw_instanced` batch, on `--native` **and** in-browser. Red if the batch is not materially cheaper — which kills A3 before anything is built on it | Open |
| **A1** — node tree + transform composition | `stage` | composed world matrices for a 3-deep tree with rotation + non-uniform scale + pivot equal hand-computed `math::mat4_*` products; both backends. Red on multiply order or pivot handling | Open |
| **A2** — draw the tree through the **existing** per-sprite path | `stage` | a stage-drawn frame is **pixel-identical** to the hand-written immediate-mode draw of the same content (`save_png` + compare). Parallel run: both paths live | Open |
| **A3** — batched renderer behind the same API | `stage` | pixels identical to A2 **and** draw calls drop from N to O(atlases). Both halves — same pixels alone would pass a batch that silently fell back | Open |
| **A4** — z-order, hit-test, input routing | `stage` | a headless pick table over a known overlapping tree: every (x, y) resolves to the hand-computed node, including under rotation and inside a clip | Open |
| **A5** — per-node alpha + tint as instance attributes | `stage` | alpha 0.5 over a known background composites to the hand-computed RGBA; tint × texel matches. Today `draw_sprite` has neither uniform | Open |
| **A6** — clip / mask rect | `stage` | content outside the mask is absent at the exact pixel boundary, nested two deep | Open |
| **B1** — glyph atlas + `TextNode.text` | `text2d` | mutate `.text` every frame for 600 frames: GL texture count **constant** (today one upload per change), pixels equal the `create_text_texture` baseline | Open |
| **B2** — wrapping + alignment | `text2d` | a hand-computed break table (width → break positions) **per target**, **including multi-byte text** — `len(text)` counts characters and the byte-indexed read is the live trap. Not one shared table: native measures the real TTF through fontdue and the browser measures whatever family resolved, so the same string breaks in different places. The cross-target invariant is **self-consistency** — the drawn text fits the box that same target measured | Open |
| **C1** — tween core + easing set | `tween` | sampled values match a hand-computed easing table exactly; a completed tween lands **on** the end value, not end−ε; identical result at 30 Hz and 60 Hz | Open |
| **C2** — bind to node properties | `tween` | driving `node.x` through a tween yields the same pixel sequence as setting it by hand | Open |
| **D1** — Button (up/over/down/disabled) | `ui` | a replayed `gl_next_event` sequence drives the exact state sequence; press-then-leave-then-release does **not** fire | Open |
| **D2** — focus, keyboard nav, input field | `ui` | replayed keystrokes incl. IME text produce the exact buffer; tab order matches the declared order | Open |
| **E1** — browser audio bridge | this repo | headless-Chrome page loads a clip: handle non-null, `audio_play` returns a sink. **Run it on the current tree first** — today it returns `i32::MIN` / `-1`, so the harness must go red before the fix | Open |
| **E2** — loop, pan, seek, stop-all | `graphics` | each round-trips on native and in-browser | Open |
| **E3** — `audio_bus` | `audio_bus` | bus gain composition matches hand-computed values; ducking restores exactly | Open |
| **F1** — the pack **is** a loft store | `assets` | pack → read back: every asset byte-identical, **and** `type_layout_fingerprint` matches across native and wasm. If that check fails everything downstream is wrong | Open |
| **F2** — range-read loader | `assets` | the same game source runs from a local pack and from `python3 -m http.server` with only the URL changed; a byte-range log shows **only** the requested keys fetched | Open |
| **F3** — prefetch policy | `assets` | instrument the frame loop: **zero fetches inside a frame** during steady-state play | Open |
| **F4** — retire `build_atlas()` | vehicle | Brick Buster's 190 hand-poked lines become a packed asset; frames pixel-identical to the baked version | Open |
| **F5** — font sources: browser-resident, our server, or a CDN | `assets` | a page declaring each of the three sources resolves to the **requested** family, not the fallback. Assert the *resolved* family — text draws either way, so "text appeared" is not the gate. Red on a manifest that lets the declared `font-family` drift from the name the program passes | Open |
| **F6** — font readiness ordering | `assets` | with the font source **throttled**, the page still resolves to the requested family — i.e. the `document.fonts.load` await genuinely holds `loft_start`. Remove the await and this goes red while F5 stays green on a fast local font, which is why it is its own phase | Open |
| **G** — vector paths on the GPU | — | deferred behind a trigger (below) | Deferred |

## Effort per phase

Totals: **5 XS, 13 S, 4 M** — no phase above M, which is the § Cutting rule holding
rather than optimism. Three phases carry a design call that decides the effort, and
those calls are made here rather than discovered mid-build.

| Phase | E | What the effort actually is |
|---|---|---|
| **A0** | XS | Two probe programs — an N-times `draw_sprite` loop, and one instance-buffer upload plus a single `gl_draw_instanced`. Time 300 frames at N = 100/1000/5000 on both targets; the browser half reuses the headless-Chrome harness `tests/html_render.rs` already has. No library code. |
| **A1** | S | **Design call: the tree is a flat `vector<Node>` with integer parent indices, not pointer-linked.** A parent holding children while a child points back is a dependency cycle in loft's ownership model; the flat form sidesteps it entirely, keeps insert-order-is-parents-first as a checkable invariant, and hands A3 its iteration order for free. Then `world_matrix` as one forward pass, with pivot as `T(p)·R·T(−p)`. |
| **A2** | S | Walk the array in z-order, call `draw_sprite` (the mvp-taking form, **not** `draw_sprite_at`, whose helper is translate+scale only) so rotation works from the first frame. The effort is the comparison rig: the same content drawn twice, `gl_screenshot` both, compare bytes. |
| **A3** | M | Group by atlas, pack per-instance attributes (a 2×3 affine + uv rect + tint + alpha) into one float buffer, one `gl_draw_instanced` per group, new shader with instanced attributes. The cost is stride/offset bookkeeping — `gl_instance_attrib` takes `stride_floats`/`offset_floats` and a wrong one fails as garbage geometry, silently — plus re-uploading only what changed. A2's path stays alive to compare against. |
| **A4** | S | Stable z sort (insertion order breaks ties), reverse-iterate to pick, invert the world affine in closed form (no general 4×4 inverse) to take a screen point node-local. **The part that is always forgotten is capture** — the node that received the press receives the release even when the pointer has left it — and it is exactly what D1 tests. |
| **A5** | S | Two more instance attributes and `color = texel * tint * vec4(1,1,1,alpha)`. Small once A3's buffer exists; the real content is the compositing decision — the canvas packs straight 0xAARRGGBB and GL blending wants premultiplied, so pick one and hand-compute the expected RGBA against it. |
| **A6** | S | `gl_scissor` per clipped subtree, nested clips intersected with the parent rect. S rather than XS **because it interacts with A3**: a scissor change breaks a batch, so grouping becomes (atlas, clip) rather than atlas. |
| **B1** | M | Rasterize glyphs once into an atlas, keep a (font, size, codepoint) → uv map, build a text node as one quad per glyph fed through A3's buffer, so `.text =` re-lays-out quads and uploads nothing. Effort: shelf packing, atlas growth when it fills, and both backends producing the same atlas *shape* even where glyph pixels differ. |
| **B2** | S | Greedy breaking on measured advances, three alignments, and the character-vs-byte trap — `len(text)` counts characters, the indexed read is bytes. Per-target break tables (see the Verify column). |
| **C1** | S | A tween is (setter, from, to, duration, easing, elapsed) driven off `fixstep`'s step, plus the standard easing table and sequencing — chain, parallel, delay, on-complete. Pure loft, no GPU. The exactness gate is a clamp everyone forgets: a finished tween must land **on** the end value. |
| **C2** | XS | loft has no property references, so tweenable properties are a small enum plus a write switch. Unelegant and correct; closures are the alternative if one arrives cheaply. |
| **D1** | S | Four visual states over A4's routing-with-capture. The effort is the replay harness, and it constrains A4: the input path must be injectable. `input_tick_from_state` in the `input` package already exists for exactly this — reuse it rather than inventing a second seam. |
| **D2** | M | Focus ring and tab order are small; the **text field** is the phase. Caret placement needs B2's measurement, selection needs hit-test to a character index, insertion/backspace/IME arrive via `gl_event_text`, and multi-byte indexing returns for a third time. |
| **E1** | XS | ~40 lines of JS. **Design call: `audio_load` is synchronous and `decodeAudioData` is not**, so it returns a handle immediately and the buffer lands later; a `play` on a still-decoding clip drops rather than queues — the same plan-then-use shape as the asset store. `play` builds BufferSource → GainNode, sinks go in a table that `stop`/`set_volume` index. |
| **E2** | S | Native: widen the `#native` signatures and the cdylib (rodio already does all four). Browser: `loop` on the source, `StereoPannerNode`, `start(when, offset)`. Both together, or they drift. |
| **E3** | S | Buses as a gain graph with per-bus volume and ducking. Pure composition over E2. |
| **F1** | M | Asset record types, the packer (PNG in via `imaging`, audio bytes, level blobs), `store_persist_bind` / `durable_seal`. Effort: the native-vs-wasm layout fingerprint check, and choosing the key granularity so `store_load_key` fetches a sensible page rather than one sprite or the whole file. |
| **F2** | S | One call site — `store_load_key(s)` from a URL with a local-path fallback. The effort is *proving* only the requested ranges crossed the wire, which means a logging static file server in the test. |
| **F3** | S | An explicit request-these-keys call at load and level boundaries, a ring-around-player helper, and a counter that can assert zero fetches inside a frame. The instrumentation is the work; the policy is three lines. |
| **F4** | XS | Pack `build_atlas()`'s output as a PNG, load it from the pack, delete 190 lines, pixel-compare. |
| **F5** | S | Manifest fields (family, browser source, native path), page emission of the `@font-face` or `<link>`, and enforcing family-name-equals-lookup-key **at build time** instead of leaving it to be discovered as a silent fallback at runtime. |
| **F6** | XS | Emit the `document.fonts.load` await for each declared family ahead of `loft_start`. The fix is two lines; the throttled test is the phase. |
| **G** | H | Deferred. A path rasterizer with AA fills, gradients and stroke joins is the one genuinely research-shaped item here, which is why it is behind a trigger rather than in the queue. |

## The vehicle

**Brick Buster II**, rebuilt arc by arc, with the shipped 1983-line version kept as
the baseline. At each arc boundary record two numbers: frames pixel-comparable
against the baseline, and line count — which must go **down** and is written into
this table when it does. A rewrite that only moves lines between files is a failed
arc, and the count is what says so.

## Phase ordering

1. **E1 first** — it is XS, independent of everything, and turns silent browser
   games into games with music. Do not queue it behind the keystone.
2. **A0**, then A1 → A6 in order. A0 is the cheapest phase in the plan and the only
   one that can kill the design for the cost of a compile.
3. **B**, then **C**, then **D** — each needs A4's hit-test or A1's transforms; D2
   additionally needs B.
4. **F** runs in parallel with A–D (it touches no rendering), except F4, which needs
   A2. F5/F6 (fonts) are independent of F1–F3 and can land with B, which is the first
   arc that cares which font actually resolved.
5. **E2/E3** whenever a consumer asks; they are comfort, not capability.

## The asset route — why not an embedder

The obvious first pass is a `--html` flag that bundles referenced files into the
page, the Flash `[Embed]` shape. **Do not build that.** The route already exists and
is better: an asset pack **is a loft store**, hosted on any dumb file server and read
by HTTP range so only the bytes a lookup touches cross the wire —
[REMOTE_STORES.md](../../REMOTE_STORES.md) documents this for exactly this case
(*"world chunks, meshes, textures, sounds, animations, dialogue, level data"*), and
the `routing` project already ships it for map tiles (`PLAN-TILES.md`): the store's
layout is schema-derived, so there is no codec, no parse step and no serialize seam —
the struct definition **is** the file layout.

Two constraints carry over from routing and are what F3 exists to hold:

- **Plan → fetch → read, never fetch-on-miss inside a frame.** Synchronous wasm
  cannot await, and a frame that blocks on a range read stutters visibly. Assets are
  requested at load or level boundaries, or as a ring around the player.
- **Verify the layout fingerprint across native and wasm before anything reads a
  pack** (routing's B.2). A silent layout divergence turns every asset into garbage
  at a byte offset, which reads as a corrupt file rather than a layout bug.

Embedding stays available for the handful of bytes a page needs before its first
fetch (a boot font, a loading sprite) — but it is the exception, not the pipeline.

## Fonts — three sources, one declaration

`--html` ships **no font bytes today**, and for the reuse case it does not need to:
`gl_load_font("X.ttf")` never opens a file in the browser. `familyFor()`
(`doc/loft-gl-wasm.js:113`) resolves the path's base name to a CSS family — a family
the page has registered wins (`document.fonts.check`), else a name heuristic picks
`monospace` / `serif`, else `sans-serif` — and the browser's own `fillText` produces
the coverage bitmap the desktop shader already expects. Name a family the browser
already has and **nothing is downloaded**.

What is missing is the ability to *bring* a font. Three sources, one declaration:

| Source | Browser | Native / `--native-wasm` |
|---|---|---|
| a family the browser already has | nothing shipped, nothing fetched | the TTF beside the game |
| our own file server | `@font-face { src: url(…) }`, or the WOFF2 packed in the asset store and range-read like every other asset | the same store |
| Google Fonts, or any CDN | the provider's stylesheet `<link>`; zero bytes of ours | the TTF beside the game |

Two mechanics decide whether this works at all, and both are gates rather than notes:

- **The declared `font-family` must equal the base name the program passes to
  `gl_load_font`** — that string is the lookup key `familyFor` builds. A manifest that
  lets the two drift produces a silent fallback, never an error.
- **The page must await `document.fonts.load('16px "<family>"')` before `loft_start`.**
  `document.fonts.check` is synchronous and answers *false* for a webfont still
  loading, and `familyFor`'s answer is **cached per handle**
  (`fonts.push({ family: … })`), so one early `gl_load_font` locks that handle to
  `sans-serif` permanently — the page then renders in the wrong font with nothing on
  stderr. That is why F5 asserts the resolved family and F6 exists at all.

A remote font is a third-party dependency: offline, or with the CDN blocked, the chain
degrades to `sans-serif` rather than failing. That is the right behaviour, and the
reason the native source stays declared beside the browser one.

The mechanism already exists for a library that wants it **today** — a package can
carry its `@font-face` and the `fonts.load` await in `[wasm.bridge] host_js`, no engine
change. F5/F6 are about making it declarative so a game does not hand-write JS, and
about making the ordering gate automatic rather than remembered.

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
- **`lib_plans/72-renderer-backend-boundary`** (GFX.PORTABLE) — `stage` must reach the
  GPU through the `Renderer` contract, not raw `gl_*`, or it becomes the next thing
  blocking a wgpu backend.
- **`lib_plans/76-particles`, `lib_plans/75-physics-2body`** — both become cheap once
  A lands (a particle system is a batched node; a body is a node with a velocity), so
  neither needs its own renderer.

## See also

- [`lib_plans/58-graphics/`](../58-graphics/README.md) — the layer this builds on.
- [REMOTE_STORES.md](../../REMOTE_STORES.md) — the asset route (arc F).
- [`../../../tools/brick-buster/25-brick-buster.loft`](../../../tools/brick-buster/25-brick-buster.loft) — the baseline and the vehicle.
- [HTML_EXPORT.md](../../HTML_EXPORT.md) / [BROWSER_INTEROP.md](../../BROWSER_INTEROP.md) — the browser target arc E fixes.
- [loft-lang/plans#144](https://github.com/loft-lang/plans/issues/144).
