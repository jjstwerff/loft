<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN146 — Game content and delivery

**Status — SHIPPED 2026-08-25.** All 18 phases: `E1`–`E3` · `W0`–`W6` · `F1`–`F7`.
Tracker: [loft-lang/plans#146](https://github.com/loft-lang/plans/issues/146).
Split out of [@PLN144](../144-2d-stage/README.md); part of the four-plan 2-D game set
(@PLN144 stage · @PLN145 text/tweens/widgets · **@PLN146** content + delivery ·
@PLN147 editor).

Everything a game needs that is not the frame: content in, sound out, native and
browser alike. **Parity between the two targets was the through-line**, and the gates
said so — a byte-range log, a headless-Chrome audio handle, a throttled font source.

## Where the reference content lives now

| What | Home |
|---|---|
| The asset route — why a pack is a **store on a dumb file server** rather than an `[Embed]`-style bundler, and the two constraints (plan → fetch → read; verify the layout fingerprint across targets) | [REMOTE_STORES.md § An asset pack is a store, not a bundle](../../REMOTE_STORES.md) |
| Fonts — the three sources, the `[[font]]` declaration, the build-time family check, and why `document.fonts.check` is the wrong question | [HTML_EXPORT.md](../../HTML_EXPORT.md) |
| The `assets` / `drawing` / `graphics` / `audio_bus` APIs | `make libcatalogue` → `LIBRARIES.md` |
| Which library branches are still in flight | [LIBRARY_BRANCHES.md](../../LIBRARY_BRANCHES.md) (generated) |

## What shipped

The pack round-trips byte-identically on the interpreter, `--native` and wasm, pages
over HTTP range at 9 % of the file per read, and takes **zero** fetches inside a frame.
A page declares the font it draws with and gets that font rather than a fallback, and
declares the pack it reads and can carry it. Brick Buster's sprite sheet is a packed
asset rather than 180 lines of drawing per launch. A sprite is content this stack builds
rather than one Python draws: `drawing` renders a `.draw` scene pixel-identically to the
tool that made the corpus, over 37 scenes with 0 pixels different on both backends. A
browser game loops, pans and seeks its audio the way a desktop one does, and one slider
moves a whole category of sounds without the game keeping the list.

**What is left is landing, not building** — three library PRs
([loft-libs-graphics#46](https://github.com/loft-lang/loft-libs-graphics/pull/46),
[loft-libs-assets#11](https://github.com/loft-lang/loft-libs-assets/pull/11),
[loft-libs-game#12](https://github.com/loft-lang/loft-libs-game/pull/12)) and then the
registry: `drawing` 0.3.0, `assets` 0.3.0, `graphics` 0.9.0, `audio_bus` 0.1.0. ⚠ Two
things stay RED until a **publish** rather than until a fix — `audio_bus`' CI job (it
needs `graphics` 0.9.0 to resolve) and `tools/brick-buster/pack_atlas.loft`'s local
`rgba_bytes` (it can call `assets::texels` the day 0.3.0 lands).

## Five findings changed the plan rather than following it

Each is written up in its phase's own doc; kept here because each one is a claim about
the tree that a later plan can reuse.

- **A pack is TWO stores** — the paged loaders refuse a wrapper-struct root.
- **`Petals` and `landmark` have no user anywhere**, so arc W came out smaller than cut.
- **`imaging` DROPPED alpha**, which blocked `F7` and `F1`'s premultiplication both;
  fixed in `imaging` 0.3.0.
- **`document.fonts.check` cannot say whether a page has a font** — true for a family
  nothing declares, false for one that is loading. That is how the browser text bridge
  came to take its exact-font branch for every page *except* the one that had brought a
  font.
- **The packer `W6` was cut on did not exist.** `F1` shipped the schema, `F7` the proxy
  derivation, `F4` one whole-page vehicle — nothing placed a cell. A plan row is a claim
  about the tree, and this one had gone stale between being written and being worked.

## Seven loft defects were found by these gates

Six fixed, one filed: a `store_load` that never returned · a paged load that refused any
entry type with an `enum` field · a refusal message naming a type the record did not have
· `familyFor` resolving a declared webfont to a generic and caching it for the run ·
[loft#1063](https://github.com/loft-lang/loft/issues/1063) (filed) ·
[loft#1085](https://github.com/loft-lang/loft/issues/1085) — the interpreter's `ref = null`
ALLOCATED a store where `--native` writes the sentinel, so a callee freed a buffer its
caller still owned — and the tuple element leak that fix uncovered, which had been masked
all along. The pack is the shape that found the store ones: several keyed collections plus
values big enough to reach the allocator's linear scan.

`W5` found two more, both since fixed:
[loft#1086](https://github.com/loft-lang/loft/issues/1086) and
[loft#1087](https://github.com/loft-lang/loft/issues/1087) — the latter a
`formal/formatting.md` deviation an oracle comparing two backends could not see, because
both dropped the flag identically.

## Phase records

| Arc | Phases | Written up in |
|---|---|---|
| **F** — assets, packs, fonts | `F1` schema · `F2` range-read · `F3` prefetch · `F4` retire `build_atlas()` · `F5` font sources · `F6` readiness ordering · `F7a`/`F7` collision proxy | [F1](F1.md) [F2](F2.md) [F3](F3.md) [F4](F4.md) [F5](F5.md) [F6](F6.md) [F7a](F7a.md) |
| **W** — sprite authoring in loft | `W0` corpus + oracle · `W1` filled polygon · `W2` parser + primitives · `W3` fills · `W4` `Petals`/`Fronds` · `W5` the check channel · `W6` scene straight into the pack | [W0](W0.md) [W2](W2.md) [W3](W3.md) [W4](W4.md) [W5](W5.md) [W6](W6.md) |
| **E** — audio | `E1` browser bridge · `E2` loop/pan/seek · `E3` `audio_bus` | [E2](E2.md) |

`W1` has no doc of its own — it is `graphics` 0.6.0/0.7.0 and is recorded in
[W2.md](W2.md), which is where its consequence (a polygon that agrees with
`fill_triangle` is not one that agrees with Pillow) was measured.

## See also

- [@PLN144](../144-2d-stage/README.md) — the stage;
  [`RENDERER.md`](../144-2d-stage/RENDERER.md) holds the atlas doctrine `F1` implements.
- [@PLN145](../145-authoring-libs/README.md) — text, which `F5`/`F6` feed.
- [@PLN147](../147-content-editor/README.md) — the editor writes **this** pack. Its arc
  `X` turns arc `W`'s renderer into a browser sprite editor with animation, so `W2`'s
  oracle gate is what `X1` extends across targets, and `W6`'s scene-to-pack route is what
  `X5` bakes through.
