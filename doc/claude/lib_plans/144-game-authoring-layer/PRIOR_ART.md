<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN144 — Prior art: the lavition tree next door

What `moros` already built that this plan extracts, adopts, or is validated by.
The plan and its phase gates are in [README.md](README.md).
Read that tree; **never write to it** — findings come back here.


`moros` carries a ~55 k-line editor (`hex_editor` 26.8 k, `hex_part` 10.6 k, `hex_voxel`
7.9 k, `hex_mesh` 5.3 k) and three of its results land directly on this plan. Read that
tree; **never write to it** — the findings come back here.

- **`lavition_ui` (2.1 k lines) is arc D, already built and proven by a 26.8 k-line
  consumer.** `UiRect` / `rect_contains`, `Button`, `Entry`, `ListBox`, `StatusStrip`,
  `Panel` with `panel_hit_test → UiHit`, a hotkey `VerbBar`, a `Theme` — with a
  **deliberately empty dependency list**: no graphics, no GL, no world, so it is
  headless-testable and registry-ready as it stands. D1 extracts it; D2 adds the focus and
  text-field half it lacks.
- **Its architecture is this plan's, already running.** `panel_build(spec, w, h) → Panel`,
  then `panel_draw_list → vector<DrawRect>` and `panel_text_list → vector<DrawText>` —
  retained spec, flat command list, hit-test on the same structure. A2 emits that shape
  rather than a rival one.
- **`font.loft` is B1m, and it was paid for.** Two measured runs to detect the browser's
  proportional substitution, a 1/64-px advance because whole pixels accumulated a 31 px
  error on one line, and outward rounding so `fit_text` never claims a fit that overflows.

**`lavition_ui` is unpublished**, so D0 is a request to that tree, not work here.

**And the editor core transfers further than it looks.** It is already 2D with 3D
*extracted*: `hex_editor`'s work is axial-lattice editing — `gesture.loft` alone is 7 027 of
its 12 304 source lines — and the third dimension lives in `hex_proj`, a seam of
`hex_to_world(q, r, height) → Vec3` plus mesh emitters. Give that seam a screen sibling and
the same world renders as sprites, with gestures, session, keymap and UI untouched —
which is what § The presentation model in [README.md](README.md) builds on.


The page shell is the other half worth watching: moros plan 22 has a `--html` editor whose
world is bytes (`W1`), whose page has a filesystem (`P6`), and where *build something, close
the tab, come back and it is there* has been true since `B4`. That is F1's scene-in-a-store
with hot reload, running — which is why scenes are in the first schema rather than a later
one.

