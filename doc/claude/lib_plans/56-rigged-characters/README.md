<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 31 — Rigged characters: persons & animals with animation → glB

Status: open

## Goal

A loft library to **author persons and animals as rigged, skeletally-animated
meshes and export them to glB** (binary glTF 2.0) — so they drop into the lavition
engine and any industry tool (Blender, three.js, the glTF validator) unchanged.

Scope is **stylized / low-poly / parametric** characters built procedurally — the
sweet spot the [`draw` skill](../../../../.claude/skills/draw/references/3d.md) names.
Photoreal organic humans/animals are explicitly **out of scope** (perception-dense,
topology-subtle — the same hard edge as realistic 2D faces); the target is readable,
posable, animatable creatures, not sculpted realism.

This is the **3D instantiation of the drawing method** ([`DRAWING.md`](../../DRAWING.md),
the `draw` skill): the same loop — frozen intent → block-in → exact metric checks →
render → cold-observe critique → iterate — now where the *renderer does the
value/form modelling* that was the 2D ceiling, and the work is mostly specification
(the strength). It splits into two halves: **authoring offline via Blender**
(build / rig / animate / export — § Architecture) and the genuinely useful
loft/engine part — **playing an animated glB inside the game at runtime**
(§ The useful loft half), where the real dogfood lives (a glB loader: binary I/O,
JSON, matrix/quaternion math).

## What already works — we build on this, not from scratch

The two things a 3D pipeline usually has to build first — a **glB writer** and a
**renderer** — already exist and are tested. The remaining work is the skeletal /
animation layer on top.

| Component | State | Where |
|---|---|---|
| **glB 2.0 export** (static meshes + materials + camera + lights) | **working** | `lib/graphics` `save_scene_glb()` (native Rust backend); driven by `lib/moros_render` `map_export_glb()` |
| **3D renderer** (OpenGL) + 2D `Canvas` raster + image I/O | **working** | `lib/graphics` (native) — *this is the visual-feedback channel the loop needs* |
| Scene graph: `Scene` / `Mesh` / `Material` / `Node` / `Camera` / `Light`, `Vertex {position, normal, uv}`, `Vec3`/`Vec2`/`Mat4` | **working** | `lib/graphics` public API |
| Geometry primitives + assembly (boxes, cylinders, slopes, stairs, hex surfaces, ramps), occlusion cull, ray-pick | **working, ~1340 LOC, 100+ tests** | `lib/moros_render/src/moros_render.loft` |
| **Static player avatar mesh** (`emit_player_avatar`, `avatar_add_to_scene`) | **working, tested** | `lib/moros_render` (`tests/avatar.loft`) |
| Player physics / collision (rigid body; no rig — octahedron placeholder) | working | `lib/moros_sim` (`src/player.loft`, `src/collide.loft`) |
| Time-driven *procedural* animation pattern (age/tick growth) — a reference for cycle scheduling, not skeletal | working | `lib/audience_crystal` |
| `lib/shapes`, `lib/game_protocol` | stubs (no `.loft` source yet) | — |

**The precise gap** (what this plan adds):

- **No skeleton** — bones/joints, parent-child hierarchy, bind pose.
- **No skinning** — per-vertex joint indices + weights; the current `Vertex` is
  static (position/normal/uv only).
- **No keyframe animation** — `Animation`/`Keyframe`, TRS tracks, interpolation
  (quaternion slerp), sampling a pose at time `t`.
- **No glB skin/animation export** — `save_scene_glb()` emits static scenes only;
  glTF `skins` + `animations` are not written.
- **No character/animal builders** — no parametric biped/quadruped templates.

## Architecture decision: drive Blender from a loft spec

**Use Blender as the universal back-end — build, rig, animate, render, *and* export —
driven by a loft front-end.** This collapses the three hardest pieces (a glB writer,
a skinned/animated renderer, and skin/animation export) into one mature, correct,
off-the-shelf engine. It's the truest form of "don't build too many pieces at once":

- **Blender does:** construct meshes + armatures (skeletons) + skin weights +
  keyframed actions via `bpy`; render the look (the contact sheet, § Looking); and
  **export glB via its reference glTF exporter** (guaranteed valid, engine-loadable).
- **loft does:** author a **declarative character/animation spec** and
  (progressively) compute the procedural generation. **Me:** the intent, the
  metric/composition reasoning, and the cold-observe critique on the rendered frames.

**Interface = data, not code.** loft emits a declarative spec (JSON: skeleton, mesh
params or explicit geometry, cycles); a *stable* `bpy` back-end script interprets it,
builds the Blender objects, renders, and exports. loft never generates Python — it
emits data a reusable interpreter consumes. Cleaner, reviewable, and it keeps loft's
job = specification (its strength).

**Spectrum — migrate generation from Blender into loft over phases:**

- *Start Blender-heavy* — loft passes high-level params, `bpy` builds everything.
  Proves the pipe with the least new code.
- *Target middle* — loft computes the mesh + skeleton + weights + keyframes (the
  algorithmic, reasoning-heavy, dogfood-worthy part) and emits them as explicit data;
  a thin `bpy` importer reconstructs them, Blender renders + exports. loft owns the
  *creative* work; Blender owns the *format + render* drudgery.

**Honest trade-off.** This defers loft's hardening value (binary I/O, a real glB
writer) and makes Blender a hard *authoring-time* dependency — acceptable: asset
generation is offline, non-loft at the start is fine, and a correct animated glB +
look-path arrive far faster. The **pure-loft glB writer** (extend native
`lib/graphics`, or a loft-side writer) is the *"eventually"* path — and because loft
already emits explicit geometry/rig/anim data, adding it later is *additive, not a
rewrite*. Keep that door open; don't walk through it now.

## The useful loft half: render an animated glB *in the game*

Offloading authoring to Blender clarifies where the real loft/engine value is — the
half Blender can't do for you, because **you don't ship Blender inside a game**:

> **Load a glB (skinned mesh + skeleton + animations) in lavition and play it live** —
> sample the animation at time `t`, compute the joint matrices, skin the vertices,
> draw via `lib/graphics`.

This is what the games (moros / dryopea) actually need to show a character moving,
and it's the genuinely valuable deliverable. It also brings the **dogfood value back
on the *read* side**: a glB **loader** is real loft work — JSON + binary buffer
parsing + matrix/quaternion math — and runtime skinning + animation sampling exercise
the language and the engine exactly where it counts.

The split is clean:

- **Authoring (offline) → Blender** — build / rig / animate / export. Not the loft
  value; don't reimplement it.
- **Runtime (in-game) → loft + `lib/graphics`** — load + play animated glB. **The
  loft deliverable.** `lib/graphics` renders *static* meshes today; the gap is
  runtime **skinned-animation playback** (load skin+anim, sample, skin, draw).

Bonus: once the in-game renderer plays animated glB it *doubles as a feedback
renderer* (engine frames to look at) — but at the start Blender renders the authoring
look (§ Looking); the engine renderer is the runtime goal, not the bootstrap.

## Method (how every phase runs)

The `draw` loop, in 3D:

1. **Freeze intent** — the creature spec (proportions, the *feeling*/read, and the
   poses/cycles wanted), as checkable predicates.
2. **Block in** — skeleton + primitive masses co-generated (mesh built *around* the
   bones, so skin weights assign automatically — auto-skinning a co-generated mesh
   is easy where auto-skinning an arbitrary one is hard).
3. **Measure on the cheap channel** — glTF **structural validation** (valid
   accessors, weights sum to 1, joints referenced, channels valid) + a text facts
   report (skeleton tree, joint positions, bbox, tri count, animation summary). This
   is the metric channel, native to 3D and a strength.
4. **Render (sparingly)** with an off-the-shelf glTF renderer (Blender headless —
   see § Looking at an animated glB) — a **contact sheet** of cycle frames +
   orientation views — and `Read` them. *Not* `lib/graphics` at the start: it can't
   skin/animate yet (that's the deliverable) and is display-bound.
5. **Cold-observe critique** — recognition ("does it read as a wolf?") and motion
   ("does the walk read as natural?"), judged from the *rendered frames*, not the
   spec. The perceptual bar; the only part not reducible to a check.
6. Selective fix, iterate; **earned rules graduate into the builders/the tool**.

## Looking at an animated glB — the feedback render

The perceptual half of the loop needs to *see* the result, and I read **stills, not
video**. So "look at an animated glB" = render a **contact sheet**: ~6–8 frames
sampled across one cycle (reads the gait) + 2–3 orientation views (front / side /
3-quarter) for the static read. Structure goes on the cheap text channel (the glTF
**validator** + a facts dump: skeleton tree, joint positions, bbox, counts) — no
render needed.

Blender (the back-end, § Architecture) renders this **headlessly** via **Cycles-CPU**
(no GL needed) or **`xvfb-run`** for EEVEE. Fallback look-path if Blender is
unavailable: **three.js + puppeteer** (headless-Chrome SwiftShader software GL). A
near-free *motion* complement: a stick-figure of the skeleton across the cycle via
the 2D `draw.py` projection — not a substitute for the skinned render.

**Phase-0 smoke test:** render a *known* Khronos sample (Fox / CesiumMan / BrainStem —
all skinned+animated) into a contact sheet and `Read` it — proving the look-path
works *before* we generate any of our own glB.

## Phases

Each phase is the smallest end-to-end slice that *renders*, so the loop can close.
**All phases run on the Blender back-end (§ Architecture); the procedural generation
migrates from `bpy` into loft across the phases.** Phase files (`00-*.md` …) carry
`Status:` and detail; this README is the index.

| Phase | Slice | Proves |
|---|---|---|
| **0 — foundations** | Stand up the **Blender back-end**: a loft spec → `bpy` → **glB export + contact-sheet render** round-trip on a *trivial* rig (two boxes, one joint, one keyframe), and smoke-test the look on a Khronos animated sample (§ Looking). Confirm Blender installs + runs headless here; confirm quaternion + `Mat4` math in loft (for generation later). | build + rig + animate + export + **look** all work end-to-end, off-the-shelf, before any real character |
| **1 — skeleton + skinning** | `Bone`/`Skeleton`/`SkinnedMesh` (joint indices + weights, inverse-bind matrices); one bone, one posed frame → glB → validate → render the deformation | a vertex deforms with a joint, exported correctly |
| **2 — keyframe animation** | `Animation`/`Keyframe`, TRS samplers, quaternion slerp, sample-at-`t`; glB animation channels → render animated frames | a joint rotates over time, in-engine and in glB |
| **3 — biped builder** | Procedural person: skeleton template (hips→spine→neck→head, arms, legs) + co-generated low-poly mesh + auto weights; multi-view metric checks (proportions, symmetry) | a person reads as a person, rigged |
| **4 — biped cycles** | Parametric idle / walk / run (procedural gait, optional IK foot-lock); render animated frames → critique naturalness → iterate | a walk reads as walking |
| **5 — animals** | Quadruped template + a small animal set + a cycle library, parameterised by size/gait | persons *and* animals, animated |

**Runtime track (R) — the in-game renderer (§ The useful loft half).** Runs in
parallel, on Blender-made glB assets; this is the genuinely valuable loft deliverable:

| Phase | Slice | Proves |
|---|---|---|
| **R0 — load static glB** | loft glB **loader** (JSON + binary buffers) → a static mesh drawn in lavition via `lib/graphics` | the engine ingests a Blender-made glB |
| **R1 — skinned bind pose** | load skin (joints + weights) + skeleton; draw the bind pose | skin data ingested, skeleton built |
| **R2 — play animation** | sample animation at `t` (keyframe interp + slerp) → joint matrices → skin → draw, looping | **an animated character plays in-game** — the deliverable |

## Ground rules / acceptance

- Every exported asset **passes `gltf-validator` clean** and **loads + animates** in
  the lavition engine and a standard glTF viewer (multi-view + the cycle playing).
- The creature **reads as itself** (recognition) and the **motion reads as natural**
  (the perceptual bar) — judged on rendered frames, per the method.
- Tests follow the existing `lib/moros_render` pattern (geometry/avatar suites);
  each phase lands its own test file before the next opens.
- Stylized/low-poly scope held: no chase of organic realism (named floor).
- Reuse `lib/graphics` + `lib/moros_render` rather than reimplementing; new code is
  the loft spec/generation, the `bpy` interpreter, and (R-track) the loader + runtime
  skinned-animation renderer.
- **Runtime acceptance:** a Blender-made animated glB **loads and plays in lavition**
  via the loft loader + `lib/graphics` (the R-track) — not only in an external viewer.

## Open questions

- **Blender as authoring dependency:** installable + headless on this box (Cycles-CPU
  vs `xvfb-run`+EEVEE)? version pin? (Phase-0 probe).
- **Spec interface:** the JSON schema loft emits and the stable `bpy` interpreter
  consumes (skeleton / mesh params or explicit geometry / cycles).
- **Generation split:** how much loft generates vs `bpy` for v1, and when to migrate
  each piece into loft (the § Architecture spectrum).
- **Runtime renderer in `lib/graphics`:** does it expose enough to add skinned-mesh +
  per-frame pose drawing, or does the native backend need extending (Rust)? — gates
  the R-track, the loft deliverable.
- **glB loader scope (R0):** parse in loft (the read-side dogfood), or lean on a
  `lib/graphics` glTF import if one exists?
- Quaternion + `Mat4` math in loft: present, or to be added? — gates loft-side
  generation *and* runtime skinning.
- **Pure-loft glB *writer*** (the "eventually" authoring path): trigger to build it
  once generation lives in loft.
- Animal coverage: how many templates / which gaits for v1.
