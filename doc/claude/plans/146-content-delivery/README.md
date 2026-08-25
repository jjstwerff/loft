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

**Complete — all three arcs are built and gated end to end.** Everything a game
needs that is not the frame: content in, sound out, native and browser alike. **Parity
between the two targets is the through-line**, and the gates say so — a byte-range log, a
headless-Chrome audio handle, a throttled font source.

**All 18 shipped** — `E1` · `E2` · `E3` · `W0`–`W6` · `F1`–`F7`, none blocked and none
open. The pack exists, round-trips byte-identically on the interpreter, `--native` and
wasm, pages over HTTP range at 9 % of the file per read, and takes **zero** fetches inside
a frame; a page declares the font it draws with and gets that font rather than a fallback,
and declares the pack it reads and can now carry it. Brick Buster's sprite sheet is a
packed asset rather than 180 lines of drawing per launch. A sprite is content this stack
builds rather than one Python draws: `drawing` renders a `.draw` scene pixel-identically
to the tool that made the corpus, a scene now says what it MEANS as well as what it draws
(`W5` — the check channel is the oracle's report byte for byte, and the exit status is the
verdict), and it reaches the pack without becoming a file on the way (`W6`). Arc E closed
last: a browser game loops, pans and seeks its audio the way a desktop one does, and one
slider moves a whole category of sounds without the game keeping the list.

**What is left is landing, not building.** Four PRs carry the last two arcs —
[loft-libs-graphics#46](https://github.com/loft-lang/loft-libs-graphics/pull/46) (`drawing`
W4/W5 + `graphics` 0.9.0), [loft-libs-assets#11](https://github.com/loft-lang/loft-libs-assets/pull/11)
(`assets` 0.3.0), [loft-libs-game#12](https://github.com/loft-lang/loft-libs-game/pull/12)
(`audio_bus` 0.1.0), and this repo's own branch. Then the registry: `drawing` 0.3.0,
`assets` 0.3.0, `graphics` 0.9.0, `audio_bus` 0.1.0. ⚠ Two things are RED until a publish
rather than until a fix — `audio_bus`' CI job (it needs `graphics` 0.9.0 to resolve) and
`tools/brick-buster/pack_atlas.loft`'s local `rgba_bytes` (it can call `assets::texels`
the day 0.3.0 lands).

**The schema is now a package.** `assets` — merged and published through 0.2.0
([#7](https://github.com/loft-lang/loft-libs-assets/pull/7),
[#9](https://github.com/loft-lang/loft-libs-assets/pull/9),
[#10](https://github.com/loft-lang/loft-libs-assets/pull/10)) — carries
the F1 schema, `pack_write`/`pack_read`, `keys_near`/`prefetch`, the layout fingerprint and
F7's proxy derivation, and W6 adds the placer (`texels` / `Tile` / `page_grid`), with 23
tests on both backends. The promotion found two things the probes could not: the two
halves accept **different URL spellings** (the metadata half reads a `file://` URL, the paged
half refuses one), so one base would have loaded a game's scenes and silently none of its
art; and the layout fingerprint is now **pinned**, which is what makes a format change say
so. `F4` reads packs from there once it lands.

⚠ **A pack's round-trip test needed a loft newer than the library CI's** — the `store_load`
hang F1 found. It is on `main` now, so the package's CI reaches it; while it was not,
measured both ways on the exact CI command, a loft binary from before the fix hung and the
300 s watchdog killed it. The pack is the shape that triggers it, so the package could not
avoid it and should not have tried: shrinking the fixture past the allocator's linear scan
would have hidden a defect every real pack will hit.

**Five findings changed the plan rather than following it.** A pack is TWO stores because
the paged loaders refuse a wrapper-struct root; `Petals` and `landmark` have no user anywhere,
so arc W is smaller than it was cut; `imaging` DROPPED alpha, which blocked `F7` and F1's
premultiplication both, and is fixed in `imaging` 0.3.0; and `document.fonts.check` cannot say whether a page has a font —
it answers **true** for a family nothing declares and **false** for one that is loading, which
is how the browser text bridge came to take its exact-font branch for every page except the
one that had brought a font; and the packer `W6` was cut on — *"`F1`'s packer already
premultiplies, pads and places"* — **did not exist**, so the placer is part of that phase
rather than a caller of one. Each is written up in its phase's own doc.

**Seven loft defects were found by these gates and six are fixed** — a `store_load` that
never returned, a paged load that refused any entry type with an `enum` field, a refusal
message that named a type the record did not have, `familyFor` resolving a declared webfont
to a generic and caching it for the run ([F5](F5.md)), and
[loft#1063](https://github.com/loft-lang/loft/issues/1063) (filed). Arc W added two more:
[loft#1085](https://github.com/loft-lang/loft/issues/1085) — the interpreter's `ref = null`
ALLOCATED a store where `--native` writes the sentinel, so a callee freed a buffer its
caller still owned and the recycled slot silently overwrote a live local — and the tuple
element leak that fix uncovered, which had been masked all along. The pack is the shape
that found the store ones: several keyed collections plus values big enough to reach the
allocator's linear scan.

## Goal

Ship `assets` (a pack that is a loft store, range-read from any file server, holding scenes as
well as art), `drawing` (author a sprite **in loft**, not in Python), and close browser audio —
so the same source runs from disk and from a URL with nothing changed but the URL, and the art
it loads was made with the same toolchain.

## Effort + design

- **Effort:** H — 18 phases in three arcs, none above M. **Design:** ✓. **`E1` shipped 2026-08-19; `W0` + `F7a` 2026-08-21; `F1` + `F2` + `F3` + `F5` + `F6` 2026-08-22; `F4` + `F7` + `W1` + `W2` 2026-08-23; `W3` + `W4` 2026-08-24; `W5` + `W6` + `E2` + `E3` 2026-08-25.**
- **Scope:** 2-D games. Follows @PLN144's scope exactly.

## Sub-arcs

`Verify` is what would go **red if the phase were done wrong** — filled when the phase is
cut, not when it is implemented.

| Item | Where | Verify | Status |
|---|---|---|---|
| **F7a** — will `shapes` accept a derived proxy at all? | probe only | hand-build one proxy of the kind alpha-derivation produces and feed it to `shapes`' overlap test. Red if the shape kinds do not meet — `shapes` ships `Rect`/`Circle` and a derived hull is neither. **`shapes` has no consumer today** except loft's own demo, so this asks the question its absence of adoption already raises, for the cost of a compile | ✅ **Shipped** — [F7a.md](F7a.md): derive **16 `Rect` bands**, not a hull |
| **F7** — a collision proxy derived from the sprite's alpha | `assets` | hexbody's contract, in 2-D: the proxy **contains** every opaque texel and its overshoot is **bounded** — `proxy ⊇ opaque ∧ overshoot ≤ +100 %`, measured per sprite over the corpus rather than asserted ([F7a](F7a.md) set the bound and the shape). Re-art a sprite and its proxy follows with no hand edit; that is the whole point | ✅ **Shipped** — `assets` 0.2.0. The blocker is FIXED, not worked around: `imaging` 0.3.0 gives `Pixel` an alpha channel ([loft-libs-graphics#37](https://github.com/loft-lang/loft-libs-graphics/issues/37), closed), so a decoded PNG answers which texels are opaque instead of the 6.0 %-wrong colour-as-alpha guess. `Cell.ce_proxy` is 16 `shapes::Rect` bands on the tighter axis, derived at pack time; containment is tested **texel by texel** and proven able to fail (shorten every box by one texel → three assertions red), and overshoot is bounded at +100 %. `shapes` 0.5.0 carries `Proxy` / `proxy_hits`, so the set test has one home |
| **E1** — browser audio bridge | this repo | headless-Chrome page loads a clip: handle non-null, `audio_play` returns a sink. **Run it on the current tree first** — it returned `i32::MIN` / `-1`, so the harness went red before the fix | ✅ **Shipped** |
| **E2** — loop, pan, seek, stop-all | `graphics` | each round-trips on native and in-browser | ✅ **Shipped** — [E2.md](E2.md): `graphics` 0.9.0 (`audio_play` gains defaulted `looping` / `pan` / `start`; `audio_set_pan` / `audio_seek` / `audio_stop_all` join it), and the browser bridge with it. ⚠ **The E1 gate was passing on a page that never finished** — the harness fails on a `console.error`, so saying nothing read as green, and the `looping` flag could be dropped with the test still green. Fixed by polling for each decode and asserting the probe's completion text; the same control now reddens it, as do controls on the start offset and the pan. Where the two targets could differ, the native side takes **Web Audio's** answer: `StereoPannerNode`'s two pan laws (a stereo clip panned by the mono one loses its far channel), `start` skipping the first pass only, and a sink id that carries its slot's generation — the native side had been reusing slot numbers, so a `stop` on a finished sound stopped whatever took its place |
| **E3** — `audio_bus` | `audio_bus` | bus gain composition matches hand-computed values; ducking restores exactly | ✅ **Shipped** — `audio_bus` 0.1.0 in loft-libs-game ([#12](https://github.com/loft-lang/loft-libs-game/pull/12)), 12 tests on both backends. A bus is a number that multiplies, arranged in a tree; **a duck REMEMBERS the level it replaced** rather than dividing the gain back out, so a thousand duck/unduck pairs on `0.35` end on the same bits (a restore that multiplies by `1.0000000001` reddens three tests). Not one of the tests plays a sound — CI has no audio device, so a suite that needed one would gate nothing, and `attach` is the seam that makes every rule measurable. Spatial audio is deliberately out: placing a sound needs a listener and a world, and a mixer has neither |
| *— arc **W**: sprite authoring, in loft —* | | | |
| **W0** — the corpus and its oracle | probe only | every `.draw` scene in `crawler/assets/sprites/src/` and loft's `sketch/` renders under the **existing Python** `draw.py` to a committed golden. Red on a scene that will not parse — which is how the grammar the port owes gets *measured* rather than guessed | ✅ **Shipped** — [W0.md](W0.md), 37 scenes green |
| **W1** — filled polygon in `graphics` | `graphics` | the one primitive genuinely missing (only `fill_triangle` exists today): hand-computed pixel counts for a convex, a concave and a self-intersecting polygon under even-odd, **and** a triangle drawn through the polygon path is pixel-identical to `fill_triangle` | ✅ **Shipped — both primitives.** [W0](W0.md) finding 5 was right that this row undercounts: the oracle draws at 3× and resamples with Pillow's LANCZOS, and nothing in `graphics` or `imaging` resampled at all. `graphics` 0.6.0 is the polygon, **0.7.0 is `resize_lanczos`** — byte-identical to Pillow on eight reference cases (the oracle's 3× downscale, non-integer ratios, degenerate axes, an upscale). The trap was that a faithful port of Pillow's coefficients still disagreed on RGBA: `Image.resize` with alpha is *defined* as `convert("RGBa") → resize → convert("RGBA")`, so premultiplication is contract, and neither rounding is the obvious one. Polygon counts hand-computed (60 / 112 / 60), even-odd pinned by the property that *distinguishes* it (a star's centre is crossed twice, so it is outside — non-zero would fill it), and the `fill_triangle` tie is a **sweep**: every triangle on a 5×5 lattice, 15 625 shapes, 0 pixels differ. Proven able to fail — leave the closing row half-open and 15 000 of them disagree. The agreement was the whole difficulty: a half-open edge rule is required for through-vertices and drops the polygon's lowest row, which is exactly and only where the two fills differed |
| **W2** — parser + `size` / `Background` / `name` / `Line` / `Circle` / `Poly` | `drawing` | **pixel-identical to `draw.py`** over the corpus subset using only these. The Python renderer is the oracle, so this is a parallel run rather than a judgement | ✅ **Shipped + PUBLISHED** (`drawing` 0.1.0, registry `ac55480`) — [W2.md](W2.md): **28 of 28 scenes, 0 pixels different**, and the subset is COMPUTED from the scenes so the gate cannot quietly stop covering one. Proven able to fail (a one-pixel error turns all 28 red), and the package's own suite is green on **both backends**. ⚠ **W1's polygon could not be used.** `graphics::fill_polygon` agrees with `fill_triangle` — its own gated contract — where Pillow fills the pixels whose CENTRES are inside, and the two agree on **4 of 400** random polygons (35 px apart on W1's own reference triangle). Neither rule is wrong; `.draw` is defined by the oracle's, so `drawing` carries a ported Pillow rasteriser and `graphics` is untouched. Two more numeric facts were measured rather than assumed: Pillow's C `float` is load-bearing (widen it and 12 of 500 polygons move), and its `ROUND_DOWN` puts the sign OUTSIDE the `ceil` — invisible until a shape hangs off the left edge |
| **W3** — fills: solid, linear gradient, radial gradient | `drawing` | pixel-identical over the corpus subset that uses them | ✅ **Shipped** — [W3.md](W3.md): **29 of 29 scenes, 0 pixels different** (the corpus increment is one scene, `player`, and it is the only one that *can* test this — the other six gradient scenes also use `Fronds`). ⚠ **The filter is chosen by OMISSION**: `draw.py` computes each ramp at 100x100 and calls `Image.resize(size)` with no `resample=`, and Pillow's default is **BICUBIC** — a numeric contract that appears nowhere in the source it is specified by. The enlargement is the contract too, not an optimisation: bicubic's negative lobes overshoot (a 40→200 step upscaled 4x spans 28..212), so a ramp computed directly at final size is smoother and wrong. `graphics` 0.8.0 adds `resize_bicubic`, sharing ONE resample body with `resize_lanczos` — only support and kernel differ — with the refactor proved behaviour-preserving by Lanczos's own ten goldens plus the whole corpus. Third measurement: the polygon mask holds exactly `{0, 255}`, so the paste is a hard SELECT and not the blend `paste` is documented as |
| **W4** — `Petals` and `Fronds` | `drawing` | pixel-identical over the corpus subset — **and the seeded field reproduces**: same seed, same pixels, on `--interpret` and `--native` alike, because a jittered array whose backends disagree is not a renderer | ✅ **Shipped** — [W4.md](W4.md): **37 of 37 scenes, 0 pixels different** on BOTH lanes, `UNBUILT` down to `Petals` alone (no scene uses it). The seeded arithmetic needed no new dependency. The phase's recorded blocker was already fixed upstream ([loft#1082](https://github.com/loft-lang/loft/issues/1082)), so the WIP's rewrite came out. Then the gate itself turned out to be the finding: `w2.sh`'s two backends were **one binary**, because a `use`d library resolves to its `native-auto/` cdylib EVEN UNDER `--interpret`. Separated with `LOFT_NO_NATIVE_LIBS=1`, the interpreter drew a different picture — `661.7595` hand-computed from the oracle against `663.464032166672`, 3 scenes red. That is [loft#1085](https://github.com/loft-lang/loft/issues/1085), fixed: the interpreter's `ref = null` ALLOCATED a store where `--native` writes the sentinel, which is what let a callee free a buffer its caller still owned. Fixing it uncovered a second leak the old form had been masking — a tuple's owned elements were never freed at all (`get_free_vars`' T1.3 branch was an empty loop) — closed in the same change |
| **W5** — the `check` channel and `--once` | `drawing` | a scene with a deliberately failing `check` exits **non-zero and names the check**; an unparsed line is reported and fails. Both run red first — a report channel that cannot fail is what makes an agent trust a bad sprite | ✅ **Shipped** — [W5.md](W5.md): 7 scenes, and the report is the oracle's `stats.txt` **byte for byte** while the exit status is `draw.py --once`'s. Six of the seven scenes are new, because the corpus has ONE scene with checks in it and every other form the grammar has had to be written down to be measured. The report is pinned rather than described — it is a wire format two tools read — and that is what found the tolerance clause coming OFF the echoed claim, `tol` being the one case-SENSITIVE word in the grammar, and the element table being ordered by first INK rather than first mention. ⚠ One difference is deliberate: a mark this build recognises and does not draw (`Petals`) FAILS the scene, where the oracle has no such notion — a renderer that is short a mark cannot answer "fine". Found [loft#1086](https://github.com/loft-lang/loft/issues/1086) (a field default naming a constant declared below it is an ICE) and [loft#1087](https://github.com/loft-lang/loft/issues/1087), a `formal/formatting.md` deviation: the `+` format flag is silently dropped on a float, on both backends |
| **W6** — a `.draw` scene straight into the pack | `drawing` + `assets` | the atlas entry built from a scene is pixel-identical to the one built from that scene's PNG, with **no PNG on disk in between** | ✅ **Shipped** — [W6.md](W6.md): three scenes, both routes, compared out of a WRITTEN pack — same page, same cell rects, same bytes, same derived proxy — on both backends, with a control that moves one texel and turns the page bytes and eleven proxy boxes red. ⚠ **The packer this row was cut on did not exist.** `F1` shipped the schema, `F7` the proxy derivation over caller-supplied texels, `F4` one whole-page vehicle — nothing placed a cell. So `assets` grew the placer too: `texels` (ONE words→bytes conversion, which is what makes the two routes' agreement a fact about the routes), `Tile`, and `page_grid`, deriving each cell's proxy as it lands. `drawing` needed nothing, and `assets` still depends on nothing but `shapes`. `assets` 0.3.0, additive over 0.2.0 in API and data alike |
| **F1** — the pack **is** a loft store, and it holds **scenes** as well as assets | `assets` | pack → read back: every asset byte-identical, **and** `type_layout_fingerprint` matches across native and wasm. If that check fails everything downstream is wrong. A scene is **definitions + placed instances** (GameMaker's object/room split), not a flat node dump — and a definition carries its **animation table**, `(action, facing) → sequence`, since a walk cycle is asset data and not code. A **light is a placed instance** like any other — the shape a prefab and an editor both need. In the first schema, because retrofitting costs a format break; and once scenes are in, reloading the store **is** hot reload | ✅ **Shipped** — [F1.md](F1.md): identical on all three targets, and it found a `store_load` that never returned |
| **F2** — range-read loader | `assets` | the same game source runs from a local pack and from a range-honouring static server with only the URL changed; a byte-range log shows **only** the requested keys fetched. (`python3 -m http.server` IGNORES `Range` and answers 200 with the whole body — loft reads correctly through that, so it proves the URL path works and nothing about what crossed the wire; the gate ships its own logging server) | ✅ **Shipped** — [F2.md](F2.md): 9 % of the file, two pages per key |
| **F3** — prefetch policy | `assets` | instrument the frame loop: **zero fetches inside a frame** during steady-state play | ✅ **Shipped** — [F3.md](F3.md): 60 frames, 0 fetches, and a control that costs 7 |
| **F4** — retire `build_atlas()` | vehicle | Brick Buster's 190 hand-poked lines become a packed asset; frames pixel-identical to the baked version | ✅ **Shipped** — [F4.md](F4.md). Both engine halves (`tests/html_page_store.rs`, `tests/html_embed.rs`) **and the vehicle**: `build_atlas()` moved out of the game into `pack_atlas.loft`, which draws the atlas once at build time into an `assets` pack the game reads and the page carries. **1983 → 1849 lines**, and the same hash `219174857032355` off `build_atlas()`, off the pack, and off the game's own loader, on both backends — with the harness proven able to fail on one byte |
| **F5** — font sources: browser-resident, our server, or a CDN | this repo | a page declaring each of the three sources resolves to the **requested** family, not the fallback. Assert the *resolved* family — text draws either way, so "text appeared" is not the gate. Red on a manifest that lets the declared `font-family` drift from the name the program passes. Field evidence rather than deduction: `moros/probe/b1` measured a desktop fixed-pitch font arriving as a **proportional** browser fallback | ✅ **Shipped** — [F5.md](F5.md): `[[font]]` in `loft.toml`, and the drift is refused before the build. It found `familyFor` taking its **generic** branch for the one page that had brought a font, and caching it |
| **F6** — font readiness ordering | this repo | with the font source **throttled**, the page still resolves to the requested family — i.e. the `document.fonts.load` await genuinely holds `loft_start`. Remove the await and this goes red while F5 stays green on a fast local font, which is why it is its own phase | ✅ **Shipped** — [F6.md](F6.md): two pages, one delayed server, and the control fires |
## Effort per phase

| Phase | E | What the effort actually is |
|---|---|---|
| **E1** | XS | ~40 lines of JS. **Design call: `audio_load` is synchronous and `decodeAudioData` is not**, so it returns a handle immediately and the buffer lands later; a `play` on a still-decoding clip drops rather than queues — the same plan-then-use shape as the asset store. `play` builds BufferSource → GainNode, sinks go in a table that `stop`/`set_volume` index. |
| **E2** | S ✅ | *Done, and "both together, or they drift" was the right instinct — but the drift was already there.* Native: widen the `#native` signatures and the cdylib. ⚠ rodio does NOT do all four: it has no pan on a `Sink` at all, so the native half needed a source wrapper reading an atomic per frame, with `StereoPannerNode`'s two laws in it. Browser: `loop`, a `StereoPannerNode`, and a rebuilt source behind the same handle for `seek` (a buffer source is one-shot). The effort the row missed was the GATE: it was green with the feature removed. |
| **E3** | S ✅ | *Done, and the S held.* Buses as a gain graph with per-bus volume and ducking, pure composition over E2 — plus the one decision that is not composition: ducking restores by REMEMBERING, not by dividing back out, because a game ducks a thousand times an hour and a computed restore drifts. Testable with no audio device because `attach` takes a handle rather than making one. |
| **W0** | XS | Collect and render. The corpus *is* the specification: whatever `draw.py` accepts today is what the port owes, and a scene that fails under the **existing** tool is a finding before a line of loft is written. |
| **W1** | S | Scanline fill with an even-odd rule, in `graphics` beside `fill_triangle`. The only real primitive gap — lines, circles, ellipses, beziers, blending and `save_png` all ship already. |
| **W2** | M ✅ | *Done, and right about where the effort would go — not about what the rig would find.* The comparison rig was most of it, and the first thing it measured was that **the primitive W1 had just shipped answers a different question**: a polygon filler that agrees with `fill_triangle` is not one that agrees with Pillow. So the drawing half grew a ported rasteriser (`Draw.c`'s `polygon_generic`, verbatim down to its three surprises), and that port needed two numeric facts nobody would guess — 32-bit floats, and a rounding whose sign lives outside the `ceil`. The parser was the small half: a byte scanner shaped after the regex constructs rather than after a tidier grammar, because which lines are ACCEPTED is part of the contract too. |
| **W3** | S ✅ | *Done, and the S held — because W2 had already parsed `grad=` / `radial=` with their specs, so this was two draw paths and a version bump.* The prediction that pixel-exactness keeps it honest was the right instinct aimed at the wrong risk: the interpolations are four lines each and none of them was where the difficulty sat. It sat in what the oracle does AROUND them — a ramp computed at 100x100 and enlarged by a filter the call site never names, whose ringing is part of the picture. Both halves were pinned against Pillow separately (nine bicubic goldens; a three-gradient scene from the oracle) before being composed, which is why the corpus was exact on the first run. |
| **W4** | M ✅ | The two array primitives, deliberately non-uniform: a seeded low-frequency field plus per-mark jitter and frayed ends, with `depth=2` growing a fractal sub-array. ⚠ **The `random` advice below was wrong and is struck.** This row said *"use the published `random` (PCG-64, seedable) … or the cross-backend gate cannot pass"*. `draw.py` uses neither, and says why in its own docstring — *"never random()/hash() (those break reproducible renders)"*. The corpus is defined by a hand-rolled integer hash, so PCG-64 would move every jittered frond and **fail** the gate it was meant to enable: W4 needs no new dependency and is smaller than cut. The seeded arithmetic is already proved to match Python exactly — the hash including its widest product (5.47e18, inside i64 by a hair), the fractal seeds at depth 3, half-to-even rounding, and `sin`; the trap avoided is that `x / 0xFFFFFFFF` in INTEGER division is 0 for every input, a perfectly smooth and entirely wrong field. `Petals` stays unbuilt — W0 measured that no scene uses it. **UNBLOCKED 2026-08-24 — and now FIXED upstream, so no workaround is needed.** [loft#1082](https://github.com/loft-lang/loft/issues/1082) is closed: a `__ref_N` return-buffer name that pass 1 substituted away and pass 2 re-minted for a different call kept a stale `skip_free`, which the vector null-init reads as *do not allocate* — so the buffer reached `smooth_vals` as `DbRef::NULL`. The historical note follows.

*Done, and neither the effort nor the blocker was where this row put it.* The array and
the seeded hash were the small half; both blockers were upstream and neither needed a
workaround in `drawing`. The historical notes follow.

**UNBLOCKED 2026-08-24 — the workaround is one word.** [loft#1082](https://github.com/loft-lang/loft/issues/1082) was filed as a mixed-ownership `if` join typing the binding as a borrow; measured, that is not what panics. Replacing BOTH joins with imperative loops leaves the fault exactly where it was. It is `smooth_vals`' bare TAIL return — `return sv_out` in place of `sv_out` makes the whole scene parse clean (`ops=2`). Minimal crashing body, bisected: append to a local vector INSIDE A LOOP, then hand it back as a bare tail; one append is clean. `bow=` is still the sole route in (it makes a frond three points, routing it into the smoothing) — but it routes into the tail return, not into the join. So W4 can proceed against the shipped loft: spell the return. |
| **W5** | S ✅ | *Done, and the S held.* `landmark` / `check` with `~ < > <= >= ==` and a tolerance, plus the `--once` exit contract. It draws nothing and is worth having anyway: a metric report costs nothing to read where a PNG costs a look, which is what makes iteration cheap for an agent. The effort went where the row did not look: the evaluation is a faithful port of forty lines of Python, and the *gate* had to invent its own corpus — one scene in 37 carries checks at all. |
| **W6** | S ✅ | *Done, and the S held — but not for the reason given.* ⚠ **`F1`'s packer does not exist**: `F1` shipped the schema and the two-store write, `F7` the proxy derivation over texels a caller supplies, `F4` a single whole-page vehicle with a ten-line converter of its own. Nothing placed a cell. So the phase is the placer as well as the route (`texels` + `Tile` + `page_grid` in `assets`, fifty lines over `vector<u8>`, no new dependency) and the gate proves the two routes agree. A plan row is a claim about the tree, and this one had gone stale between being written and being worked. |
| **F1** | M | Asset **and scene** record types, the packer (PNG in via `imaging` — ⛔ it drops ALPHA, see F7's row, so premultiplication is blocked with it; audio bytes, scene records), `store_persist_copy` / `durable_seal`. **It REPLACES the atlas builders that already exist** — `brick-buster`'s `build_atlas()` and `crawler/src/gpuatlas.loft` — rather than becoming a third; `F4` is the first half of retiring them. Use the **published** `imaging` for PNG (`routing`'s local copy is a 2024 predecessor, not a fork). Effort: the native-vs-wasm layout fingerprint check, and choosing the key granularity so `store_load_key` fetches a sensible page rather than one sprite or the whole file. **The packer also decides A3's batch count** — depth order cannot be rearranged under blending, so sprites that draw near each other must share an atlas, and premultiplication happens here, once. Scene records do not raise the M — they raise the schema's stakes, which is why they belong in the first cut. |
| **F2** | S | One call site — `store_load_key_text` from a URL with a local-path fallback. The effort was *proving* only the requested ranges crossed the wire, which meant writing the logging static file server (`python3 -m http.server` ignores `Range`) **and** padding the fixture: a 20 KB pack is a third of one page, so a paged read of it fetches everything and a gate on it measures nothing. |
| **F3** | S | An explicit request-these-keys call at load and level boundaries, a ring-around-player helper, and a counter that can assert zero fetches inside a frame. The instrumentation was the work, and it turned out to need no counter: run the same program with 0 frames and with 60 and compare the server's request count. The policy is three lines. |
| **F4** | XS → **M**, engine done | Pack `build_atlas()`'s output as a PNG, load it from the pack, delete 190 lines, pixel-compare. ⚠ The XS assumed a page can read a pack; it could not. `Store::load` read through `std::fs`, absent on `wasm32-unknown-unknown`, so `store_load` on a `--html` page answered **false** — politely, no panic, measured. The only browser route was the HTTP one F2 built, and taking it costs the gallery its single self-contained page, which is the one thing [ASSETS.md](ASSETS.md) says embedding is *for*. ✅ **The loader half is done:** `store::image_bytes` / `image_at_least` fall back to the `loft_host_fs_*` bridge, so a page reads a store out of `globalThis.loftBaseFS` — `load=true read=true` where it was `load=false`, on the same wasm, with an absent-file control still answering false. Native is one `metadata` call as before (98 store tests unchanged). ✅ **The emitting half is done too:** `[[embed]] path = "assets/game.pack"` seeds `loftBaseFS` under `/` + `path`, which is what `loft-fs.js` resolves the program's own relative string to — so one `store_load` call reads the pack on the desktop and in the page. Strict about the spelling for F5's reason: an absolute or non-normal `path` would be carried faithfully under a key nothing asks for, and a page that cannot find its pack says nothing at all. **Still to do:** the vehicle. |
| **F5** | S ✅ | *Done.* Manifest fields (family, browser source, native path), page emission of the `@font-face` or `<link>`, and enforcing family-name-equals-lookup-key **at build time** instead of leaving it to be discovered as a silent fallback at runtime. ⚠ The predicted effort missed the half that mattered: `document.fonts.check` cannot say whether a page has a font, so the *instrument* had to be built before the feature — measure the family against two generics — and it immediately found the bridge resolving backwards. |
| **F6** | XS ✅ | *Done, and the two-line prediction held exactly.* Emit the `document.fonts.load` await for each declared family ahead of `loft_start`. The throttled test is the phase, and its control — the same page without the await, against the same delayed server — is what makes the green half a reading. |
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
@PLN144's `A2`. `E2`/`E3` were cut as *"whenever a consumer asks — comfort, not
capability"*, and landed last for that reason; `E2` then found the E1 gate green with the
feature removed, which is not comfort.

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

- **[F4.md](F4.md)** — the page's own filesystem: the invariant, the six cases, and why
  `[[embed]]` is strict about how a path is spelled.
- **[ASSETS.md](ASSETS.md)** — why the pack is a store on a dumb file server rather than an
  `[Embed]`-style bundler, and the two constraints that carry over from `routing`.
- **[E2.md](E2.md)** — arc E's two phases: the gate that was green with the feature
  removed, and why a duck remembers rather than divides.
- **[W6.md](W6.md)** — the two routes into a pack, and the packer this plan said already
  existed.
- **[W5.md](W5.md)** — the metric channel: why a report is pinned rather than described,
  and the one place the port deliberately refuses to copy the oracle.
- **[W3.md](W3.md)** — arc W's two gradients: the filter chosen by omission, why the
  enlargement is the contract, and the mask that turned out to be a hard select.
- **[W2.md](W2.md)** — arc W's renderer: the gate, and the three findings that decided how the
  rasteriser is built — why it is not `graphics::fill_polygon`, why the 32-bit float is
  load-bearing, and the rounding that is invisible until a shape hangs off the canvas.
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
