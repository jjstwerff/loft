<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN145 — Text, tweens and widgets

> Tracker: [loft-lang/plans#145](https://github.com/loft-lang/plans/issues/145)
> (`subject:libs`, `status:future`). Split out of [@PLN144](../144-2d-stage/README.md), whose
> scene arcs share a gate family these do not.

## Status

**Open — design ready, nothing built.** These are the libraries a game author writes against
*above* the stage. Each has its own gate family — a metrics seam, a headless font, an event
replay — which is why they are not @PLN144's phases.

## Goal

Ship `text2d`, `tween` and `ui` so a game sets `.text`, tweens a property and places a button
without writing a rasteriser, an integrator or a hit test.

## Effort + design

- **Effort:** MH — 10 phases, none above M. **Design:** ✓, except D0, which is another tree's call.
- **Scope:** 2-D games. Follows @PLN144's scope exactly.

## Sub-arcs

`Verify` is what would go **red if the phase were done wrong** — filled when the phase is
cut, not when it is implemented.

| Item | Where | Verify | Status |
|---|---|---|---|
| **B0** — a built-in fallback font | `text2d` | under `loft test`, with **no font file and no native library loaded**, a known string draws a known non-zero coverage — the state in which `graphics::draw_text` answers *native function not loaded* today. Consumer outcome, not a unit test: `dryopea/src/hud.loft` draws its digits as **rectangles** because of this, and `picker.loft` shipped with no labels for the same reason | Open |
| **B1** — glyph atlas + `TextNode.text` | `text2d` | mutate `.text` every frame for 600 frames: GL texture count **constant** (today one upload per change), pixels equal the `create_text_texture` baseline | Open |
| **B1m** — the metrics seam | `text2d` | a **wide run and a narrow run** of `n` characters, measured at startup through whichever backend resolved, answer *fixed-pitch or not* — one run cannot, and the browser's proportional substitution is exactly what it must catch. Advance carried in **1/64 px**: a whole-pixel field truncates 9.6→9 and the error accumulates per character | Open |
| **B2** — wrapping + alignment | `text2d` | a hand-computed break table (width → break positions) **per target**, **including multi-byte text** — `len(text)` counts characters and the byte-indexed read is the live trap. Not one shared table: native measures the real TTF through fontdue and the browser measures whatever family resolved, so the same string breaks in different places. The cross-target invariant is **self-consistency** — the drawn text fits the box that same target measured. Every estimate rounds **outward**, since an under-estimate overflows a box just proved to fit | Open |
| **C1** — tween core + easing set | `tween` | sampled values match a hand-computed easing table exactly; a completed tween lands **on** the end value, not end−ε; identical result at 30 Hz and 60 Hz | Open |
| **C2** — bind to node properties | `tween` | driving `node.x` through a tween yields the same pixel sequence as setting it by hand | Open |
| **D0** — publish `lavition_ui` | upstream | the package resolves from the registry and its own tests pass unchanged after the move. **Not our work and not our clock** — moros promotes a library once it is battle-tested *there*, by rule | Blocked on moros |
| **D1** — Button + Panel over stage routing | `ui` | a replayed `gl_next_event` sequence drives the exact state sequence; press-then-leave-then-release does **not** fire. **And `panel_hit_test` answers the same `UiHit` it answers today**, which is what makes this an extraction rather than a rewrite wearing its name. **On touch there is no `over` state** — the kit has four, so a widget whose affordance lives in hover is invisible on a phone; the gate replays a touch stream, not only a mouse one | Open |
| **D2** — focus, tab order, text field | `ui` | replayed keystrokes incl. IME text produce the exact buffer; tab order matches the declared order. **The genuinely new half** — the kit has neither today | Open |
## Effort per phase

| Phase | E | What the effort actually is |
|---|---|---|
| **B1** | M | Rasterize glyphs once into an atlas, keep a (font, size, codepoint) → uv map, build a text node as one quad per glyph fed through A3's buffer, so `.text =` re-lays-out quads and uploads nothing. Effort: shelf packing, atlas growth when it fills, and both backends producing the same atlas *shape* even where glyph pixels differ. |
| **B0** | S | A compact bitmap face baked in as data plus a pure-loft blitter — no file, no `#native`, no GL. Small, and it is the phase that unblocks a shipped consumer rather than one that makes an unshipped one faster: today the text path needs a GL context **and** a native rasteriser **and** a font file, so a repo that tests its UI headlessly answers by having no text. |
| **B1m** | XS | Two measured runs at startup, a 1/64-px advance, and three derived helpers (`text_width`, `fits`, `fit_text`). Nearly free — it is `lavition_ui/src/font.loft` lifted, and its shape is a **finding**, not a preference: one run cannot distinguish a fixed-pitch font from the browser's proportional stand-in, and whole-pixel truncation cost that tree a 31 px error on a single line. |
| **B2** | S | Greedy breaking on measured advances, three alignments, and the character-vs-byte trap — `len(text)` counts characters, the indexed read is bytes. Per-target break tables (see the Verify column). |
| **C1** | S | A tween is (setter, from, to, duration, easing, elapsed) driven off `fixstep`'s step, plus the standard easing table and sequencing — chain, parallel, delay, on-complete. Pure loft, no GPU. The exactness gate is a clamp everyone forgets: a finished tween must land **on** the end value. |
| **C2** | XS | loft has no property references, so tweenable properties are a small enum plus a write switch. Unelegant and correct; closures are the alternative if one arrives cheaply. |
| **D0** | — | A request, not an effort: `lavition_ui` is unpublished and lives in a tree this stream reads and never writes. Costs a conversation and their release cycle. |
| **D1** | S | Four visual states over A4's routing-with-capture, on top of an **extracted** `Button`/`Panel`/`ListBox`/`VerbBar`/`Theme` rather than a written one. The effort is the replay harness, and it constrains A4: the input path must be injectable. `input_tick_from_state` in the `input` package already exists for exactly this — reuse it rather than inventing a second seam. |
| **D2** | M | The half the kit does not have. Focus ring and tab order are small; the **text field** is the phase. Caret placement needs B2's measurement, selection needs hit-test to a character index, insertion/backspace/IME arrive via `gl_event_text`, and multi-byte indexing returns for a third time. |

## Phase ordering

**`B0` first — it depends on nothing and unblocks a shipped consumer today.** Two UI surfaces
in dryopea have no text at all (`hud.loft` draws digits as rectangles, `picker.loft` shipped
without labels) because the text path needs a GL context *and* a native rasteriser *and* a
font file. Everything else waits on @PLN144: `B1` on its atlas, `C` on its transforms, `D` on
its hit-test — and `D0` on moros.

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

## See also

- [@PLN144](../144-2d-stage/README.md) — the stage these sit on; [`PRIOR_ART.md`](../144-2d-stage/PRIOR_ART.md)
  carries what moros and dryopea already built, including `lavition_ui` and the metrics seam.
- [@PLN146](../146-content-delivery/README.md) — fonts arrive through its asset pack.
- [@PLN147](../147-content-editor/README.md) — the editor's panels are `D`'s widgets, so `D0`
  gates that plan as well as this one.
- [loft-lang/plans#145](https://github.com/loft-lang/plans/issues/145).
