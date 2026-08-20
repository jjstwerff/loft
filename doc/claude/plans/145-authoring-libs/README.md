<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN145 — Text, tweens and widgets

> Tracker: [loft-lang/plans#145](https://github.com/loft-lang/plans/issues/145)
> (`subject:libs`, `status:future`). Split out of [@PLN144](../144-2d-stage/README.md), whose
> scene arcs share a gate family these do not.
> **Part of the 2-D game stack** — four plans cut from one design: @PLN144 the stage ·
> @PLN145 text/tweens/widgets · @PLN146 content + delivery · @PLN147 the editor. Set overview,
> through-lines and where to start: [`plans/README.md` § Plan sets](../README.md#plan-sets--where-four-plans-are-one-piece-of-work).

## Status

**Open — arcs `B` and `C` COMPLETE (`text2d` 0.4.0, `tween` 0.1.0, `stage` 0.17.0), `D0b` answered, `D1` 3-of-4; the `D1` extraction is held on a fork decision and `D2` remains (`D0` is moros's clock).** These are the libraries a game author writes against
*above* the stage. Each has its own gate family — a metrics seam, a headless font, an event
replay — which is why they are not @PLN144's phases.

## Goal

Ship `text2d`, `tween` and `ui` so a game sets `.text`, tweens a property and places a button
without writing a rasteriser, an integrator or a hit test.

## Effort + design

- **Effort:** MH — 11 phases, none above M. **Arcs `B` and `C` complete: `B0` + `B1` 2026-08-19, `B1m` + `B2` 2026-08-20; `C1` + `C2` + `D0b` 2026-08-20.** **Design:** ✓, except D0, which is another tree's call.
- **Scope:** 2-D games. Follows @PLN144's scope exactly.

## Sub-arcs

`Verify` is what would go **red if the phase were done wrong** — filled when the phase is
cut, not when it is implemented.

| Item | Where | Verify | Status |
|---|---|---|---|
| **B0** — a built-in fallback font | `text2d` | ✅ **Shipped** as `text2d` 0.1.0 — a 5×7, 56-glyph face carried as data; pure loft, no file, no `#native`, no GL. Gated on ink under plain `loft test`, and on **every digit drawing AND `'1'` carrying less ink than `'8'`** — a coverage assertion alone would pass a face that drew one blob per character. Three controls fire | ✅ **Shipped** |
| **B1** — glyph atlas | `text2d` | ✅ **Shipped** as `text2d` 0.2.0 — **600 relayouts over ten digits: ten sheet writes, then zero.** `atlas_writes` counts exactly what a GL consumer uploads on, so the property is asserted rather than described. Baseline: the sheet path and the direct blitter agree pixel-for-pixel, with a second assertion that they are not blank together. Three controls fire | ✅ **Shipped** |
| **B1m** — the metrics seam | `text2d` | ✅ **Shipped** as `text2d` 0.3.0 — two runs decide fixed-pitch and the advance comes from the **wide** one. The 1/64 gate is stated as a **property, not a war story**: a 60-char line of a 9.6 px face measures 576, and **no integer advance can produce 576 over 60 characters** — swept in the test. The whole-pixel control answers **540**, the 36 px accumulation exactly. `metrics_builtin` answers through the same seam, which also pins **advance extent vs ink extent** — one trailing gap apart, different questions. Nine controls fire | ✅ **Shipped** |
| **B2** — wrapping + alignment | `text2d` | ✅ **Shipped** as `text2d` 0.4.0 — the trap is **measured, not assumed**: `"héllo"[0..5]` is `"héll"`, four characters, and loft snaps a byte cut outward so it under-fills rather than corrupting. ⚠ **`fit_text` had shipped that way in `B1m`** and is fixed here. Hand-computed table on the **built-in** face (the one target measurable without a font); self-consistency gated over fixed-pitch, fractional-advance and proportional metrics. Two design calls stated: a line always takes **≥1 character** (its control HANGS rather than asserting, and the timeout names the termination test), and an overlong line starts **at** its box under every alignment. Nine controls fire | ✅ **Shipped** |
| **C1** — tween core + easing set | `tween` | ✅ **Shipped** as `tween` 0.1.0 — the two gates turned out to be **one defect with two faces**: float seconds sum to `0.99999999999999989` at 30 Hz and `1.00000000000000133` at 60 Hz, so the animation both parts by rate AND never arrives. Integer base units make the two rates equal *to the bit*. The endpoint needed the **other spelling of a lerp**: `a+(b−a)·t` lands on `b` for 5 of 8 pairs and answers **0.0** for `(1e17, 1.0)`; `a(1−t)+bt` lands 8 of 8. Eleven curves cover 33 by reflection, with the clamp **underneath** the reflections — in front of them an in-out at its midpoint asks the raw curve for `in(1)` and answers `0.5000000000000001`. Chains carry their leftover: discarding runs **43 % long at 30 Hz**. 20 tests, both backends, three controls fire | ✅ **Shipped** |
| **C2** — bind to node properties | **`stage`** (0.16.0) | ✅ **Shipped** — and `advance` settled the placement: `stage.advance(dt_us)` already walks every node once a frame, so a bound tween **rides it** and one call steps the sequences and the property tweens together. That keeps `tween` dependency-free and decides the unit too — **microseconds**, because that is what `advance` takes. THE GATE is a parallel run (`draw_list.loft`'s A2 shape): eight frames of a tweened `x` compared **pixel for pixel** against a hand-moved node, with numbers picked so the hand side needs **no library call** (64 px / 800 000 µs sampled every 100 000 µs is exactly 8 px a frame). ⚠ The switch is gated **cell by cell** — each of nine properties must move its own field and no other. ⚠⚠ **At most one tween per (node, property)**; a second replaces it, and a finished one releases the property. 9 tests, suite 144 on both backends, three controls fire | ✅ **Shipped** |
| **D0** — publish `lavition_ui` | upstream | the package resolves from the registry and its own tests pass unchanged after the move. **Not our work and not our clock** — moros promotes a library once it is battle-tested *there*, by rule | Blocked on moros |
| **D0b** — does `input` fit a consumer that already exists? | probe only | ✅ **Answered** ([`probe-d0b.loft`](probe-d0b.loft), both backends identical) — **adopt it, and do not expect it to resolve modifiers.** dryopea's 33-row action set expresses through `Bindings` and replays through `input_tick_from_state` exactly as `D1` needs; edge detection is correct headless. ⚠ But a **modifier is not expressible**, and it is structural: `ActionBinding` is a name + key list, `AxisBinding` is two key *codes*, and neither has a slot for *"suppressed while Ctrl is held"*. Measured: `input` answers **identically** for plain `S` and `Ctrl+S` (`pan_south=true, save=true` both times), so all five of dryopea's Ctrl-combos are indistinguishable from their plain twins — and an axis cannot carry the rule either, which is why dryopea's `bnd_axes` is **empty** and its four pan keys are four actions. ⚠⚠ **The premise was wrong**: dryopea ADOPTED `input`, so the widgets would be its *second* consumer, not the fourth — [PRIOR_ART](../144-2d-stage/PRIOR_ART.md) corrected | ✅ **Answered** |
| **D1** — Button + Panel over stage routing | **`stage`** (0.17.0) for the states; the extraction is ⛔ **held** | 🟡 **3 of 4 clauses shipped.** ✅ a replayed stream drives the exact state sequence (8 events, both buttons, hand-computed); ✅ press-then-leave-then-release does **not** fire, while press-then-drift-a-pixel does; ✅ touch never shows `Over` — **one** recorded stream replayed both ways fires identically, so the pointer kind changes only what is *shown*. ⚠⚠ **The invariant is that a node reads `Down` exactly when a release would FIRE it** — one predicate serves the picture and the click, so a control that breaks the capture check does not even redden the down-means-fires gate: the class is unrepresentable, not caught. ⛔ **Clause 4 — `panel_hit_test` answering the same `UiHit`** — is held: see the note below | 🟡 **Partly shipped** |
| **D2** — focus, tab order, text field | `ui` | replayed keystrokes incl. IME text produce the exact buffer; tab order matches the declared order. **The genuinely new half** — the kit has neither today. ⚠⚠ **`D0b`'s limit lands squarely here**: a text field needs `Ctrl+C`/`Ctrl+V` and `Shift`+arrow, and `input` cannot express a modifier — so this phase must carry a ctrl rule of its own (dryopea's `ea_ctrl` column is the worked example) or `input` must grow one first. Decide which **before** the phase is cut | Open |
## Effort per phase

| Phase | E | What the effort actually is |
|---|---|---|
| **B1** | M ✅ | *Done.* Rasterize glyphs once into an atlas, keep a (font, size, codepoint) → uv map, build a text node as one quad per glyph fed through A3's buffer, so `.text =` re-lays-out quads and uploads nothing. Effort: shelf packing, atlas growth when it fills, and both backends producing the same atlas *shape* even where glyph pixels differ. |
| **B0** | S ✅ | *Done.* A compact bitmap face baked in as data plus a pure-loft blitter — no file, no `#native`, no GL. Small, and it is the phase that unblocks a shipped consumer rather than one that makes an unshipped one faster: today the text path needs a GL context **and** a native rasteriser **and** a font file, so a repo that tests its UI headlessly answers by having no text. |
| **B1m** | XS ✅ | *Done.* Two measured runs at startup, a 1/64-px advance, and three derived helpers — shipped as METHODS on `Metrics`, because `text_width` already exists in `text2d` and two free functions of that name collide. Note also Nearly free — it is `lavition_ui/src/font.loft` lifted, and its shape is a **finding**, not a preference: one run cannot distinguish a fixed-pitch font from the browser's proportional stand-in, and whole-pixel truncation cost that tree a 31 px error on a single line. |
| **B2** | S ✅ | *Done.* Greedy breaking on measured advances, three alignments, and the character-vs-byte trap — `len(text)` counts characters, the indexed read is bytes. Per-target break tables (see the Verify column). |
| **C1** | S ✅ | *Done.* Shipped WITHOUT the setter (that is `C2`) and without a `fixstep` dependency: a duration is an integer count of whatever unit the consumer's clock already runs in, so a second definition of *how long a second is* never gets made. ⚠ The predicted effort — "a clamp everyone forgets" — was the wrong half. The clamp is real but it is a **placement** question, not a forgetting one, and the accumulation defect underneath it is the one that makes a 30 Hz animation never arrive. Sequencing is eleven lines because `advance` answers a **leftover** rather than a boolean. |
| **C2** | XS ✅ | *Done, and the enum-plus-switch prediction held exactly.* What the note did not see is that the switch is also the first public way to **move** a node — `stage` could place one and never shift it, so a consumer had to write `st_nodes[i].nd_x` directly. So `set_prop` ships beside `tween_prop`, through the same switch. ⚠ Filed on the way: [loft#1039](https://github.com/loft-lang/loft/issues/1039) — `lib::Enum.Variant` does not parse, though `lib::Struct{}`, `lib::CONST`, `std::abs()` and `lib::Enum` *as a type* all do. |
| **D0** | — | A request, not an effort: `lavition_ui` is unpublished and lives in a tree this stream reads and never writes. Costs a conversation and their release cycle. |
| **D0b** | XS ✅ | *Done.* One probe program, and it did the job a probe is for — it did **not** retire the dependency, it retired the *reason to doubt it* and replaced it with a named limit `D1` and `D2` can plan around. ⚠ The phase's own framing was the thing that broke: two of the three "wrote their own" consumers were miscounted, one of them because a file was judged by its NAME (`framekey.loft` is a frame-reuse digest, not a keyboard file). |
| **D1** | S 🟡 | *States done; extraction held.* The predicted effort — *"the effort is the replay harness"* — was **wrong in a good way**: A4's path was already injectable (every entry point takes coordinates and reads no device), so the harness cost nothing and no second seam was invented. The real effort was deciding what `Down` MEANS, and the answer made a bug class unrepresentable. ⚠ The extraction is the part that stalled, on a question the plan did not ask — see below. |
| **D2** | M | The half the kit does not have. Focus ring and tab order are small; the **text field** is the phase. Caret placement needs B2's measurement, selection needs hit-test to a character index, insertion/backspace/IME arrive via `gl_event_text`, and multi-byte indexing returns for a third time. |

## Targets

Follows @PLN144: interpreter, `--native`, `--html`, `--native-android`. ⚠ **`D2`'s IME gate
cannot pass on Android today** — NativeActivity delivers `KeyEvent`s and has no `TextEvent`, so
*composing* text needs a new `gl_text_input()` text-stream API in `graphics`. Discover that
before `D2` is cut, not inside it; the desktop and browser halves of the same gate are
unaffected.

## Phase ordering

**`B0` first — it depends on nothing and unblocks a shipped consumer today.** Two UI surfaces
in dryopea have no text at all (`hud.loft` draws digits as rectangles, `picker.loft` shipped
without labels) because the text path needs a GL context *and* a native rasteriser *and* a
font file. Everything else waits on @PLN144: `B1` on its atlas, `C` on its transforms, `D` on
its hit-test — and `D0` on moros.

**@PLN144 is complete (2026-08-20), so nothing in this plan waits on it any more.** `B2`,
`C1`/`C2` and `D0b` are all unblocked and independent of each other; only `D0` is still
someone else's clock, and `D0b` is the phase that must run before `D1` commits to `input`.

**Arc `C` is complete and only arc `D` is left.** `C1` shipped `tween` 0.1.0 and `C2` bound it
to the stage in `stage` 0.16.0; `D0b` cleared `input` for `D1`'s replay harness with one named
limit (no modifiers) that `D2` has to plan around. `D1` is the next phase that depends on
nothing — `D0` (publishing `lavition_ui`) is moros's clock and does not gate it, since `D1`'s
extraction can be prepared against the shape `lavition_ui` already has.

⚠ **A finding from `C1`'s CI check, and it belongs to @PLN144: `stage` does not pass the
repo's gate.** `LOFT_DENY_WARNINGS=1 loft test` reddens all 16 of its files on four
`never-read` parameters (`stage.loft:882` `screen_w`, `:931` `self`, `:1087` `sx`/`sy`).
Nothing is red today only because `loft-libs-graphics`'s `library-ci.yml` still lists
`["graphics", "gridmesh", "imaging", "shapes"]` — but that list is **computed from the
package dirs** by `scripts/deploy-library-ci.sh`, so the next refresh adds `stage` and the
gate goes red. `text2d` (35 tests) and `tween` (20) both pass it today. The `:931 self` case
is not a rename away: a method's receiver has to be spelled `self`.

### ⛔ D1 clause 4 — the extraction is held on a question this plan never asked

`D1`'s fourth clause is *"`panel_hit_test` answers the same `UiHit` it answers today"*, which
presumes we build a `ui` package by extracting moros's `lavition_ui`. Three facts found while
doing `D1` say that presumption needs a decision before any code:

1. **`lavition_ui` is moros's to promote, and `D0` is that promotion.** @PLN147 § says
   *"`lavition_ui` (@PLN145 `D`) is what the editor's panels are made of"* — so arc D is
   *about that package*, not about a rival. Their rule is that a library is promoted once
   battle-tested **there**; a copy of it published from here is the shape that rule rejects.
2. **Its name was chosen deliberately.** moros's `EDITOR_UI.md` records `lavition_ui` over a
   generic `ui` precisely because a generic name is *"generic enough to collide in a shared
   registry"*. Publishing `ui` ourselves would mint the collision they avoided.
3. ⚠⚠ **The new half cannot live there anyway.** `lavition_ui`'s `loft.toml` states an empty
   dependency list as **its claim** — *"nothing here needs a world, a lattice, a window or a
   GL context"*. The four states need `stage`'s routing, so putting them in `lavition_ui`
   would break the one property that package advertises. That is why `D1`'s states shipped in
   `stage` instead, where the press/release/capture they extend already live.

**So the three behavioural clauses are shipped and the extraction is a fork decision**, which
is the user's rather than this stream's. The options, with what each costs:

| | what it means | cost |
|---|---|---|
| **A — don't fork** | `lavition_ui` stays moros's; consumers use it *with* `stage`'s states. `ui` never exists | nothing; clause 4 is struck as based on a false premise |
| **B — fork as `ui`** | copy `Button`/`Panel`/`ListBox`/`VerbBar`/`Theme` (~800 lines) into `loft-libs-graphics` | a rival to a deliberately-named package, and two derivations to keep in step — the exact shape that cost this branch a 75-commit rebase |
| **C — ask moros to publish** | that IS `D0`, already Blocked-on-moros | their clock; a conversation, not an effort |

⚠ Recorded because it is cheap to state now and expensive to discover after a fork: nothing in
`stage` 0.17.0 depends on the answer.

## The sandbox boundary

Every package here declares which side of loft's admission boundary it is on — **trusted
engine** (unbounded internals, an admitted-safe API) or **admissible loft**. @PLN86 shipped
admission (`src/sandbox.rs`), and the choice is a **design-time property of an API**: cheap
while the signatures are being written, a re-architecture afterwards. Get it right and a mod
is just more admitted code, with no second code path to keep in step. See
[LIBRARY_AUTHORING.md](../../LIBRARY_AUTHORING.md).

`tween` (`C1`) is **admissible loft** and says so in its README: no `#native`, no I/O, and
its one collection loop is a `for` over a bound written down at the loop — `len(steps) + 1`,
which is its exact worst case. ⚠ That shape was chosen for admission, not for style: an
unbounded `while` in a sandboxed def is refused at load, and the natural spelling of a chain
walk is a `while`.

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
