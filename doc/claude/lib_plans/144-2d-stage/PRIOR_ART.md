<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Prior art: what the sibling trees already built

> Shared by [@PLN144](README.md) (the stage), [@PLN145](../145-authoring-libs/README.md)
> (text, tweens, widgets), [@PLN146](../146-content-delivery/README.md) (content and delivery)
> and [@PLN147](../147-content-editor/README.md) (the in-browser editor). It lives here because the stage plan is the oldest of the three, not because it
> belongs to it more than the others.

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


---

## dryopea — what it needs, and the requirement it exposed

`dryopea` is the 3-D consumer (tower defence, hex lattice, `mesh3d` towers, a dynamic
camera). It wants arcs **B** and **D** — text and widgets — not the 2.5-D sprite
presentation. Its outbound queue (`QUESTIONS_FOR_LOFT.md`, 48 entries) has **nothing open**,
so what follows is read off the code rather than asked for.

**Two UI surfaces in that repo ship with no text at all, and each arrived there on its own.**

- `src/hud.loft` draws its digits as **rectangles**, and says why: `graphics::draw_text`
  rasterises through `#native rasterize_text_into`, which answers *"native function not
  loaded"* under `loft test`, and it needs **a font file the repo does not have**. Its
  comment marks this a constraint and not a style — *"a HUD nothing headless can draw is a
  HUD no test and no `snap` can see"*.
- `src/picker.loft` reached the same place a plan earlier: *"text labels + hotkey hints will
  arrive when E1's live GL window lands (`gl_draw_text` is GL-only)"*.

So the text path today requires a GL context **and** a native rasteriser **and** a font file,
and a consumer that tests its UI headlessly answers by not having text. That is [@PLN145](../145-authoring-libs/README.md)'s **B0**: a
built-in fallback font, in pure loft, needing no file and no native call. It is the phase
that unblocks a shipped consumer, which is why it goes first in arc B.

**And their headless-GL probe is a technique to adopt.** `docs/RENDERER.md` § R0 proves a GL
context exists with no display (`xvfb-run` + `gl_screenshot`), then decodes the capture with
`imaging::png` and **buckets every pixel by exact colour, requiring the `other` bucket to be
0**. A byte-diff says *different*; a classification says *what* changed — so A5's compositing
gate uses flat colours deliberately, keeping the expected RGBA set small enough that
`other == 0` means something.

---

## moros — what it needs, and the rule that reframes arc D

**It does not want libraries from upstream, and says so.** `doc/claude/LOFT_HANDOFF.md`
scopes its outbound queue to *"loft the LANGUAGE and its TOOLING — never a library gap. We
create and update libraries ourselves, because upstream cannot verify one against our use —
verification is only possible where the consumer lives. Build it under `lib/<name>/`, gate
it, and promote it once battle-tested."*

Two consequences for this plan, and they are not small:

- **[@PLN145](../145-authoring-libs/README.md)'s D0 is their promotion decision, on their clock.** Publishing `lavition_ui` is not a
  dependency this plan can schedule, and arc D must be honest that its first phase is
  someone else's call rather than a queued task.
- **A library built here and handed over is the shape their rule rejects.** So `stage`,
  `text2d` and `tween` have to be verified *in* a consumer, not beside one — which is what
  the vehicles are for — with a 2.5-D sample rather than a lavition port, since a port runs
  on their tree and their clock.

**Their upstream record lags the tracker, in the safe direction.** All four entries
`LOFT_HANDOFF.md` marks ⛔ OPEN — #948, #949, #950, #976 — are **closed upstream**. That
matters most for **#950** (`--html` traps `RuntimeError: unreachable`, `sev:high`,
`wa:none`): read off their doc it is a live blocker for every browser phase in arcs E and F,
and it is not one. Their own rule is that status is re-run and never read off a label, so
this is a re-measure prompt rather than a contradiction.

**#976 is fixed, so the collision this plan would otherwise ship is already cured.**
`graphics/src/render.loft` and `lavition_ui/src/render.loft` are the exact shape #976 was
filed about — two packages each holding one basename — and a bare `use render;` now binds
the package's own file first. What survives is an authoring checklist for the six packages
this plan adds, because the fix has a lip: `use <pkg>` **inside** `<pkg>` means the package,
`use self::<pkg>` means the file, and a suite written as `tests/<pkg>.loft` was what
amputated nine published libraries' public surface. moros carries a static guard for the
family (`tools/basenames.sh`, in its fast tier) that is worth copying rather than reinventing.

---

## hexbody — a contract to steal, and a discipline

`hexbody` is 3-D (movable, breakable geometry bodies) and consumer-only by its own rule —
*"loft is upstream and consumer-only; hexbody never fixes loft"* — so, like dryopea's
renderer and moros's editor, it is **not a consumer this plan serves**. It contributes two
things anyway.

**The proxy contract, which transfers to 2-D unchanged.** Its load-bearing invariant is
`proxy ⊇ footprint ∧ overshoot ≤ X`: a collision proxy is **derived** from the structured
representation rather than hand-authored, must **contain** the true shape, and its overshoot
is **bounded**. That is exactly [@PLN146](../146-content-delivery/README.md)'s F7 in two dimensions — derive a sprite's proxy from the alpha
A4 already reads for picking, require containment, bound the slop. Containment is what makes
the substitution safe (a system validated against the proxy stays valid when the art
changes); the bound is what stops containment being satisfied by a screen-sized rectangle.
Their framing of *why* is worth keeping too: where all you have is a bag of triangles,
collision volumes get authored by hand because a mesh cannot be reasoned about — the same
reason a hand-authored hitbox per sprite is the norm in 2-D, and the same reason it need not
be.

**Arm the gate before its subject.** Their frontier item 0 was *"arm a forward gate before
writing body code — there is currently none"*: `tests/joint.loft` was written and **held red**
before a line of body code, with both controls verified, and it caught what they had feared.
Their `L7` says determinism/replay is *built from line one*. The co-op determinism gate in `lib_plans/64-game-client` adopts that ordering — a
determinism gate written after co-op works only proves that day's build.

*(`../stories` is an empty directory — nothing to read.)*

---

## crew_punk — no code yet, and the case that proves the co-op rule

A co-operative bridge simulator, **design documents only**, derived from moros and sibling to
crawler. It is not a consumer yet, but `SCOPE.md` states a constraint that lands squarely on
co-op (which lives in [`lib_plans/64-game-client`](../64-game-client/README.md)) and is worth
capturing before anything is built:

> **A pure phone interface must be possible.** … Six players, six phones, six consoles. This
> is the *Spaceteam* / *Artemis* / *Keep Talking and Nobody Explodes* pattern.

**It makes the replicate-the-world rule obviously right rather than merely principled.** Each player's phone
shows a *different* view of one world, and the difference is presentation driven by per-client
state: your own station shows labelled controls, and taking someone else's shows *the same
panel with the labels gone*. Replicate the **stage** and six clients need six replicated
scenes that must not drift; replicate the **world** and each client derives its own panel from
what it is rated for. That gate therefore varies **role** as well as window size and camera —
same world hash, deliberately different frames.

It also names two things arc D would otherwise learn late. **Touch has no hover**, and the
extracted kit has an `over` state — so any affordance that lives in hover is invisible on a
phone, and [@PLN145](../145-authoring-libs/README.md)'s D1 replay gate must drive a touch
stream and not only a mouse one. And **six
people opening a link** is exactly what `--html` is good at, which puts
[@PLN146](../146-content-delivery/README.md) on the critical path for this consumer rather
than at the end.

## japanese — not a consumer

A JavaScript/HTML graded reader (`dict.js`, `server.py`); no loft anywhere, so it needs
nothing from this plan. One transferable input: a Japanese text needs **thousands** of glyphs
where a Latin one needs about a hundred, which is why @PLN145's B1 atlas carries LRU
eviction for dynamic glyph entries rather than assuming a small fixed set. Ruby/furigana — a
reading set above the line — is a text-layout feature B2 does not have and no loft consumer asks for; out
of scope, recorded so the next person does not have to re-derive that it is missing.

---

## Library integration — what exists, and what must not be built twice

Audited 2026-08-19 across `moros`, `dryopea`, `crawler`, `routing`, `hexbody` and loft's own
`tools/`, against a freshly built `LIBRARIES.md`.

**The finding is not that a plan duplicates a library. It is that these plans lean on three
published packages and only one of them has ever been adopted.**

| Package | Real consumers | What the plans do with it |
|---|---|---|
| `fixstep` v0.2.0 | **dryopea** (6 files) + **moros** `editor_server` | @PLN145 `C1` drives tweens off it — safe, no probe needed |
| `input` v0.2.0 | **none** | @PLN145 `D1` reuses `input_tick_from_state` |
| `shapes` v0.4.1 | **none** — only loft's own `brick-buster` demo and a test fixture | @PLN146 `F7` produces proxies for it |

`input` is the sharp one. Three consumers wrote their own instead — `moros/lib/hex_editor/src/keymap.loft`
(685 lines), `dryopea/src/bindings.loft` (461), `crawler/src/framekey.loft` — and none reached
for the package. **That is evidence, not an accident**, and adopting it without asking why would
make @PLN145's widgets the *fourth* input layer rather than the first shared one. Hence `D0b`
and `F7a`: an XS probe that expresses one **existing consumer's** need through the package,
run *before* the phase commits to the dependency. If the package cannot express it, that is
worth knowing for the cost of a compile — the same shape as `A0`.

**Atlases: three builders already exist.** `brick-buster`'s `build_atlas()` (~190 hand-poked
lines, in this repo), `crawler/src/gpuatlas.loft` (67 lines), and @PLN146's `F1`. `F4` already
retires the first; the rule for `F1` is that it **replaces** rather than joins, and the check is
that no consumer still keeps its own.

**`imaging`: use the published package.** `routing/lib/imaging/` is a 2024 *predecessor* (56
lines, read-only PNG, a `Pixel` with `limit()` bounds) that pre-dates the 73-line package rather
than forking it. dryopea already uses the published one. Retiring routing's copy is that tree's
call, not this plan's — recorded so nobody reads two `imaging`s as a design.

**Cameras are not a duplication.** `moros/lib/hex_cam` (102 lines) and `dryopea/src/camera.loft`
(86) are 3-D orbit/follow cameras; @PLN144's `P2` is a 2-D per-layer parallax camera. Different
thing, same word — checked so the next reader does not merge them.

### crawler's sprite pipeline — ported, not depended on

`crawler/tools/draw.py` (783 lines, extended from loft's own `sketch/draw.py`) is a working
sprite-authoring tool, and crawler's `SPRITES.md` already states the intent: *"built to be
extractable as a reusable 2D sprite library — the 2D stack stands on its own for 2D-preferring
devs."* @PLN146 arc **W** takes it up, **in loft**, as the `drawing` package.

The grammar is small and line-based — `size WxH` · `Background topc= botc=` (or transparent) ·
`name <tag>` · `Line (x,y)-(x,y) w=` · `Circle (c) r= [flat=]` · `Poly (x,y)…` · `Petals` ·
`Fronds` · fills `rgb=` / `grad=a>b [dir=]` / `radial=a>b [at=]` · `landmark` · `check`. All
coordinates normalised 0–1, so a scene is resolution-independent.

Three things make the port unusually safe, and they are why the arc's phases are comparisons
rather than judgements:

- **The Python renderer is the oracle.** Every phase gates on *pixel-identical to `draw.py`*
  over a committed corpus, so "does it look right" never has to be answered by looking.
- **Only one primitive is actually missing** — a filled polygon. `graphics` has
  `fill_triangle` but no general scanline fill; lines, circles, ellipses, beziers, blending
  and `save_png` all ship. That is `W1`, and it belongs in `graphics` rather than in `drawing`.
- **`--once` and `check` are why an agent can use it.** Render-and-exit, non-zero on an
  unparsed line or a failed check, plus a metric text report — a channel that costs nothing to
  read where a PNG costs a look. `W5` keeps that contract exactly.

The *authoring discipline* in `SPRITES.md` — stop when a cold read names the form uniquely,
monsters read as a threat and show a legitimate attack means, and every sprite is authored in a
**locked orientation (front = up)** so the engine can rotate it to the facing — is content
guidance, not code, and stays where it is. @PLN144's `P5` is the half that consumes it: the
locked orientation is exactly what makes continuous rotation possible without pre-rotated frames.
