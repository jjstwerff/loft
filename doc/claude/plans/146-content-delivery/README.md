<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN146 — Game content and delivery

> Tracker: [loft-lang/plans#146](https://github.com/loft-lang/plans/issues/146)
> (`subject:libs`, `status:future`). Split out of [@PLN144](../144-2d-stage/README.md).
> **Part of the 2-D game stack** — four plans cut from one design: @PLN144 the stage ·
> @PLN145 text/tweens/widgets · @PLN146 content + delivery · @PLN147 the editor. Set overview,
> through-lines and where to start: [`plans/README.md` § Plan sets](../README.md#plan-sets--where-four-plans-are-one-piece-of-work).

## Status

**Open — design ready, nothing built.** Everything a game needs that is not the frame:
content in, sound out, native and browser alike. **Parity between the two targets is the
through-line**, and the gates say so — a byte-range log, a headless-Chrome audio handle, a
throttled font source.

## Goal

Ship `assets` (a pack that is a loft store, range-read from any file server, holding scenes as
well as art), `drawing` (author a sprite **in loft**, not in Python), and close browser audio —
so the same source runs from disk and from a URL with nothing changed but the URL, and the art
it loads was made with the same toolchain.

## Effort + design

- **Effort:** H — 19 phases in three arcs, none above M. **Design:** ✓. **`E1` shipped 2026-08-19; `W0` + `F7a` 2026-08-21; `F1` 2026-08-22.**
- **Scope:** 2-D games. Follows @PLN144's scope exactly.

## Sub-arcs

`Verify` is what would go **red if the phase were done wrong** — filled when the phase is
cut, not when it is implemented.

| Item | Where | Verify | Status |
|---|---|---|---|
| **F7a** — will `shapes` accept a derived proxy at all? | probe only | hand-build one proxy of the kind alpha-derivation produces and feed it to `shapes`' overlap test. Red if the shape kinds do not meet — `shapes` ships `Rect`/`Circle` and a derived hull is neither. **`shapes` has no consumer today** except loft's own demo, so this asks the question its absence of adoption already raises, for the cost of a compile | ✅ **Shipped** — [F7a.md](F7a.md): derive **16 `Rect` bands**, not a hull |
| **F7** — a collision proxy derived from the sprite's alpha | `assets` | hexbody's contract, in 2-D: the proxy **contains** every opaque texel and its overshoot is **bounded** — `proxy ⊇ opaque ∧ overshoot ≤ X`, measured per sprite over the corpus rather than asserted. Re-art a sprite and its proxy follows with no hand edit; that is the whole point | Open |
| **E1** — browser audio bridge | this repo | headless-Chrome page loads a clip: handle non-null, `audio_play` returns a sink. **Run it on the current tree first** — it returned `i32::MIN` / `-1`, so the harness went red before the fix | ✅ **Shipped** |
| **E2** — loop, pan, seek, stop-all | `graphics` | each round-trips on native and in-browser | Open |
| **E3** — `audio_bus` | `audio_bus` | bus gain composition matches hand-computed values; ducking restores exactly | Open |
| *— arc **W**: sprite authoring, in loft —* | | | |
| **W0** — the corpus and its oracle | probe only | every `.draw` scene in `crawler/assets/sprites/src/` and loft's `sketch/` renders under the **existing Python** `draw.py` to a committed golden. Red on a scene that will not parse — which is how the grammar the port owes gets *measured* rather than guessed | ✅ **Shipped** — [W0.md](W0.md), 37 scenes green |
| **W1** — filled polygon in `graphics` | `graphics` | the one primitive genuinely missing (only `fill_triangle` exists today): hand-computed pixel counts for a convex, a concave and a self-intersecting polygon under even-odd, **and** a triangle drawn through the polygon path is pixel-identical to `fill_triangle` | Open |
| **W2** — parser + `size` / `Background` / `name` / `Line` / `Circle` / `Poly` | `drawing` | **pixel-identical to `draw.py`** over the corpus subset using only these. The Python renderer is the oracle, so this is a parallel run rather than a judgement | Open |
| **W3** — fills: solid, linear gradient, radial gradient | `drawing` | pixel-identical over the corpus subset that uses them | Open |
| **W4** — `Petals` and `Fronds` | `drawing` | pixel-identical over the corpus subset — **and the seeded field reproduces**: same seed, same pixels, on `--interpret` and `--native` alike, because a jittered array whose backends disagree is not a renderer | Open |
| **W5** — the `check` channel and `--once` | `drawing` | a scene with a deliberately failing `check` exits **non-zero and names the check**; an unparsed line is reported and fails. Both run red first — a report channel that cannot fail is what makes an agent trust a bad sprite | Open |
| **W6** — a `.draw` scene straight into the pack | `drawing` + `assets` | the atlas entry built from a scene is pixel-identical to the one built from that scene's PNG, with **no PNG on disk in between** | Open |
| **F1** — the pack **is** a loft store, and it holds **scenes** as well as assets | `assets` | pack → read back: every asset byte-identical, **and** `type_layout_fingerprint` matches across native and wasm. If that check fails everything downstream is wrong. A scene is **definitions + placed instances** (GameMaker's object/room split), not a flat node dump — and a definition carries its **animation table**, `(action, facing) → sequence`, since a walk cycle is asset data and not code. A **light is a placed instance** like any other — the shape a prefab and an editor both need. In the first schema, because retrofitting costs a format break; and once scenes are in, reloading the store **is** hot reload | ✅ **Shipped** — [F1.md](F1.md): identical on all three targets, and it found a `store_load` that never returned |
| **F2** — range-read loader | `assets` | the same game source runs from a local pack and from a range-honouring static server with only the URL changed; a byte-range log shows **only** the requested keys fetched. (`python3 -m http.server` IGNORES `Range` and answers 200 with the whole body — loft reads correctly through that, so it proves the URL path works and nothing about what crossed the wire; the gate ships its own logging server) | In progress |
| **F3** — prefetch policy | `assets` | instrument the frame loop: **zero fetches inside a frame** during steady-state play | Open |
| **F4** — retire `build_atlas()` | vehicle | Brick Buster's 190 hand-poked lines become a packed asset; frames pixel-identical to the baked version | Open |
| **F5** — font sources: browser-resident, our server, or a CDN | `assets` | a page declaring each of the three sources resolves to the **requested** family, not the fallback. Assert the *resolved* family — text draws either way, so "text appeared" is not the gate. Red on a manifest that lets the declared `font-family` drift from the name the program passes. Field evidence rather than deduction: `moros/probe/b1` measured a desktop fixed-pitch font arriving as a **proportional** browser fallback | Open |
| **F6** — font readiness ordering | `assets` | with the font source **throttled**, the page still resolves to the requested family — i.e. the `document.fonts.load` await genuinely holds `loft_start`. Remove the await and this goes red while F5 stays green on a fast local font, which is why it is its own phase | Open |
## Effort per phase

| Phase | E | What the effort actually is |
|---|---|---|
| **E1** | XS | ~40 lines of JS. **Design call: `audio_load` is synchronous and `decodeAudioData` is not**, so it returns a handle immediately and the buffer lands later; a `play` on a still-decoding clip drops rather than queues — the same plan-then-use shape as the asset store. `play` builds BufferSource → GainNode, sinks go in a table that `stop`/`set_volume` index. |
| **E2** | S | Native: widen the `#native` signatures and the cdylib (rodio already does all four). Browser: `loop` on the source, `StereoPannerNode`, `start(when, offset)`. Both together, or they drift. |
| **E3** | S | Buses as a gain graph with per-bus volume and ducking. Pure composition over E2. |
| **W0** | XS | Collect and render. The corpus *is* the specification: whatever `draw.py` accepts today is what the port owes, and a scene that fails under the **existing** tool is a finding before a line of loft is written. |
| **W1** | S | Scanline fill with an even-odd rule, in `graphics` beside `fill_triangle`. The only real primitive gap — lines, circles, ellipses, beziers, blending and `save_png` all ship already. |
| **W2** | M | The line grammar (`Poly (x,y)… rgb=…`, normalised 0–1 coordinates) and the basic marks. Most of the effort is the comparison rig rather than the drawing: render both ways, diff bytes, and treat every difference as the port's bug rather than the oracle's. |
| **W3** | S | Two interpolations and an `at=`/`dir=` parameterisation. Small — and pixel-exactness is what keeps it honest, because a gradient that is *nearly* right is one nobody can diff again later. |
| **W4** | M | The two array primitives, and the interesting half is that they are **deliberately non-uniform**: a seeded low-frequency field plus per-mark jitter and frayed ends, with `depth=2` growing a fractal sub-array. Use the published `random` (PCG-64, seedable) rather than a second generator, or the cross-backend gate cannot pass. |
| **W5** | S | `landmark` / `check` with `~ < > <= >=` and a tolerance, plus the `--once` exit contract. It draws nothing and is worth having anyway: a metric report costs nothing to read where a PNG costs a look, which is what makes iteration cheap for an agent. |
| **W6** | S | Skip the file. `F1`'s packer already premultiplies, pads and places; this hands it a rendered canvas instead of a decoded PNG, and the gate proves the two routes agree. |
| **F1** | M | Asset **and scene** record types, the packer (PNG in via `imaging`, audio bytes, scene records), `store_persist_bind` / `durable_seal`. **It REPLACES the atlas builders that already exist** — `brick-buster`'s `build_atlas()` and `crawler/src/gpuatlas.loft` — rather than becoming a third; `F4` is the first half of retiring them. Use the **published** `imaging` for PNG (`routing`'s local copy is a 2024 predecessor, not a fork). Effort: the native-vs-wasm layout fingerprint check, and choosing the key granularity so `store_load_key` fetches a sensible page rather than one sprite or the whole file. **The packer also decides A3's batch count** — depth order cannot be rearranged under blending, so sprites that draw near each other must share an atlas, and premultiplication happens here, once. Scene records do not raise the M — they raise the schema's stakes, which is why they belong in the first cut. |
| **F2** | S | One call site — `store_load_key(s)` from a URL with a local-path fallback. The effort is *proving* only the requested ranges crossed the wire, which means a logging static file server in the test. |
| **F3** | S | An explicit request-these-keys call at load and level boundaries, a ring-around-player helper, and a counter that can assert zero fetches inside a frame. The instrumentation is the work; the policy is three lines. |
| **F4** | XS | Pack `build_atlas()`'s output as a PNG, load it from the pack, delete 190 lines, pixel-compare. |
| **F5** | S | Manifest fields (family, browser source, native path), page emission of the `@font-face` or `<link>`, and enforcing family-name-equals-lookup-key **at build time** instead of leaving it to be discovered as a silent fallback at runtime. |
| **F6** | XS | Emit the `document.fonts.load` await for each declared family ahead of `loft_start`. The fix is two lines; the throttled test is the phase. |
| **F7a** | XS | One probe. `shapes` is published and unadopted; `F7` is the first thing that would depend on it, so the shape mismatch is worth finding now rather than inside `F7`. |
| **F7** | S | Derive at pack time from the same alpha A4 already reads to pick — the art contains the answer, so nobody hand-authors a hitbox per sprite. **[F7a](F7a.md) settled the shape:** 16 `Rect` bands on the tighter axis, which beats the convex hull on mean, median and worst case while needing no new kind in `shapes`; gate at overshoot ≤ +100 %, which 35 of 36 corpus sprites meet, and REPORT the one that does not. Produces *data*; `shapes` and `lib_plans/75-physics-2body` consume it, so this is not a physics engine arriving by the back door. Containment is what makes substitution safe (a system validated against the proxy stays valid when the art changes); the bound is what stops containment being satisfied by a screen-sized rectangle. |

## Targets

Follows @PLN144: interpreter, `--native`, `--html`, `--native-android`. Android already has
audio through the fixture backend (oboe/AAudio via `audio_play_raw`) and takes the **native**
side of every file and HTTP path, so a pack range-read from static hosting works there as it
does on desktop — `E1`'s browser stub is a `--html` problem only. Both depend on
[loft-libs-graphics#32](https://github.com/loft-lang/loft-libs-graphics/issues/32) landing, and
**`E2`/`F2` should be measured on Android as well as the two desktop targets** once it does.

## Phase ordering

**`W0` before any of arc W** — it measures the grammar the port owes instead of guessing it,
and it is the phase that can shrink the arc. **`E1` first** — ~30 lines of JS, independent of everything, and it turns silent browser games
into games with music. Its gate runs on the current tree *before* the fix, so the harness is
proven red. **`F1` next**, because everything else stores into its schema. `F4` needs
@PLN144's `A2`; `F5`/`F6` land with @PLN145's `B`. `E2`/`E3` whenever a consumer asks — they
are comfort, not capability.

## The sandbox boundary

Every package here declares which side of loft's admission boundary it is on — **trusted
engine** (unbounded internals, an admitted-safe API) or **admissible loft**. @PLN86 shipped
admission (`src/sandbox.rs`), and the choice is a **design-time property of an API**: cheap
while the signatures are being written, a re-architecture afterwards. Get it right and a mod
is just more admitted code, with no second code path to keep in step. See
[LIBRARY_AUTHORING.md](../../LIBRARY_AUTHORING.md).

## Package authoring

loft#976 makes a bare `use <mod>` bind the package's own file, so a sibling's basename cannot
amputate a public surface. The lip: `use <pkg>` **inside** `<pkg>` means the *package*, and a
suite written as `tests/<pkg>.loft` is what amputated nine published libraries. Copy moros's
`tools/basenames.sh` guard rather than reinventing it.

## Companion files

- **[ASSETS.md](ASSETS.md)** — why the pack is a store on a dumb file server rather than an
  `[Embed]`-style bundler, and the two constraints that carry over from `routing`.
- **[FONTS.md](FONTS.md)** — F5/F6: reusing a font the browser has, and bringing one it does not.

## See also

- [@PLN144](../144-2d-stage/README.md) — the stage; [`RENDERER.md`](../144-2d-stage/RENDERER.md)
  holds the atlas doctrine `F1` implements.
- [@PLN145](../145-authoring-libs/README.md) — text, which `F5`/`F6` feed.
- [@PLN147](../147-content-editor/README.md) — the editor writes **this** pack; `F1` is its
  only hard prerequisite, so its first phase can start before this plan finishes. Its arc `X`
  turns arc `W`'s renderer into a **browser sprite editor with animation**, so `W2`'s oracle
  gate is what `X1` extends across targets, and `W6`'s scene-to-pack route is what `X5` bakes
  through.
- [REMOTE_STORES.md](../../REMOTE_STORES.md) · [loft-lang/plans#146](https://github.com/loft-lang/plans/issues/146).
