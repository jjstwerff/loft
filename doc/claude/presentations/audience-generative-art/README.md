<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Audience-driven generative art — presentation

## Status

Open.  Scoped 2026-05-09.  Sibling **development plan** at
[`../../plans/6-audience-generative-art/`](../../plans/6-audience-generative-art/)
owns the engineering work (server, projector view, generation
script, hosting).  This file owns the **presentation work** (talk
shape, slides, audience-participation flow, presenter notes).

## Goal

Deliver a memorable, audience-participatory creative-coding talk
to a local meetup of game creators + art enthusiasts.  Frame:
**art show with loft footnotes**, not technical walk-through.

The audience walks away with the spectacle of a 3D crystal world
they collectively painted, plus a handful of small loft snippets
that show why loft was the right tool for *this* shape of demo
— compact patterns that would be awkward in JS / Python / Rust.

## Demo concept

Audience members **paint directly** onto a shared hex world from
their phones (or laptops), and a 3D crystal mesh grows out of
the painted hexes on the projector for everyone to watch:

- **Tap a hex** in their browser → places a crystal at that
  location in the chosen colour
- **Swipe** → paints a line of crystals
- **Tap own-colour crystal** → erases it
- **Choose a color** from the 9-tile palette (RGB primaries +
  CMY mixes + white / grey / brown)

The projector's auto-camera follows recent activity; sluggish-
by-design pacing means each placed crystal grows over ~5 seconds
and old crystals decay slowly after 5 minutes — the canvas
self-cleans rather than requiring resets.  Single shared URL —
everyone in the room participates from their phone or laptop.
Spectacle is the **emergent collaborative patterns**, not any
single person's contribution.

(Full design: see the [development plan](../../plans/6-audience-generative-art/)
including the three-view roles, the chunked world data layout,
the dual JSON-events + binary-blobs protocol, and the design
decisions log.)

## Talk shape

This is **more an art show than a technical session**.  The
demo runs continuously through most of the event — no rounds,
no manual resets, no pauses for setup between segments.  The
canvas evolves from blank at the start to whatever the audience
has built and the decay rule has eroded by the end.  Code is
supporting evidence, not the main act: the presenter highlights
**small loft-specific snippets** (things that read compactly in
loft and would be awkward in JS / Python / Rust) at chosen
moments — never a full file walk-through.

### Two-screen staging

| Screen | Shows |
|---|---|
| **Main projector (the "beamer")** | The 3D crystal-world view: the spectacle, full-screen, auto-camera following activity |
| **Secondary screen** (laptop / second monitor / smaller side display) | The smartphone client UI mirrored or rendered separately so non-phone audience members can see what the input surface looks like, and so the presenter can demonstrate gestures (swipe, jump-to-active, color-pick) without obscuring the spectacle |

The secondary screen is what the presenter points at when
showing how to play; the main projector is what everyone watches
to see the result.  The two-screen setup means the audience can
follow both the input language and the output spectacle at the
same time without context-switching.

| Beat | Time-share | What happens |
|---|---|---|
| Open with the project goal | brief | "Browser games anyone can play via a shared link" — frame loft's lane |
| Set up the demo URL | brief | Big QR code + short URL slide; "open this on your phone, pick a color" |
| Audience starts painting | longer | Audience taps; first crystals grow on the projector; presenter narrates lightly to let the visual carry |
| Loft snippet highlight 1 | brief (30-60 s) | Small slide showing one compact loft pattern that powers something the audience just saw — e.g. the chunked sparse-storage one-liner, or the typed-binary blob read.  Read 5-15 lines aloud; back to painting |
| Loft snippet highlight 2 | brief (30-60 s) | Another one — e.g. an edge-detection pattern, an auto-camera snippet, a generation step that fits in a few lines.  Same shape: read, return to spectacle |
| Loft snippet highlight 3 | brief (30-60 s) | Closing snippet — the kind of thing that would be a hundred lines in another language.  Sells the "loft was the right tool" thesis without becoming a code review |
| Decay narration moment | brief | Point out the edges receding — "no one's removing them; the world erodes itself.  Watch where it eats from first" |
| Q&A | open | "Where do I get this?" / "What can it do beyond paint?" |

Narrative arc: spectacle (audience paints) → footnote (here is
a small loft thing that made *that* compact) → spectacle
continues.  The art show carries the room; the snippets land
quickly enough to stay out of the way.

Pick the 3 snippets at CI-3 once the actual code exists — choose
whichever read most cleanly as standalone, not whichever cover
the most subsystems.

## Audience-participation flow

1. **Onboarding** (30 seconds): QR code on slide → URL.  Browser
   page loads.  Color picker visible.  Hex grid visible.
2. **First tap**: presenter taps own phone first to break the ice;
   shows the projected hex appear; "now everyone try."
3. **Continuous painting**: audience paints throughout the talk.
   Presenter narrates lightly during high-activity moments and
   uses quieter periods for code reveals.
4. **Decay moment**: at one or two points presenter explicitly
   draws attention to the receding edges so the audience
   notices the inverse-growth aesthetic.
5. **Cool-down**: keep the URL open at the end so people can play
   while leaving.

## Slides + supporting materials

To be produced as siblings of this README (mirrors the
[`../par/`](../par/) precedent):

| Artifact | Purpose |
|---|---|
| `slides.md` | Marp / reveal-style markdown source for the slide deck |
| `slides.html` | Built deck (committed for offline-on-stage reliability) |
| `slides.pdf` | PDF backup |
| `script.md` | Presenter script (what to say at each beat) |
| `presenter-notes.md` | What can go wrong + how to recover; URLs / IPs / colors / dial-in info for the demo machine; two-screen staging notes (which output goes where, fallback if the secondary screen drops) |

Build script + content land when the development plan reaches
phase 5 readiness.

## Open presentation-side questions

1. ~~**QR-code → URL onboarding**~~ — RESOLVED 2026-05-10:
   **big dedicated "join here" slide at the start**.  One large
   QR + one short URL filling a slide.  Cleanest visual; easiest
   to scan from the back of the room.  Late arrivals can ask a
   neighbour or wait for the next presenter pause; the demo URL
   stays consistent across the whole event so re-onboarding is
   trivial.
2. ~~**Voice + visual rhythm**~~ — RESOLVED 2026-05-10:
   **mix — light narration during paint, silent during code
   reveals**.  Light commentary while the audience paints
   ("watch how blue pushes north", "see the bridges forming",
   "look where the edges are starting to recede") keeps the
   room engaged.  Code-reveal beats stay silent so the audience
   can read the projected source.  Adapts to room energy.
3. ~~**When to reveal each of the three code files**~~ —
   RESOLVED 2026-05-10: **no full file reveals**.  This is an
   art show, not a technical session.  Replace the three
   file-reveal beats with **3 small loft snippet highlights**
   (~30-60 s each) showing patterns that read compactly in loft
   and would be awkward in JS / Python / Rust.  Pick the actual
   3 snippets at CI-3 once the code exists — choose whichever
   read cleanest as standalone, not whichever cover the most
   subsystems.
4. ~~**Single demo or include a second moros-editor segment?**~~
   — RESOLVED 2026-05-10: **single demo, audience-generative
   only**.  The art-show framing (Q3) reinforces this — a second
   demo would dilute the through-line.  The moros-editor pitch
   stays a sentence in passing ("this same renderer ships in our
   3D editor too" — surfaces the renderer-reuse story without
   competing for attention).

## Risks (presentation-side)

| Risk | Mitigation |
|---|---|
| Audience hesitates to participate | Presenter taps own phone first; "demo audience" of 2-3 pre-paired phones on the podium for stage-tap simulation if real audience is shy |
| QR code fails to scan from back of room | Print a poster-sized QR; provide short URL as fallback on every slide |
| Demo machine crashes mid-talk | Pre-recorded video as fallback; presenter narrates over it; rehearse this fallback path |
| Server crashes mid-talk | Local restart from a saved state-snapshot (or accept starting from blank); rehearsal pins the recovery time |
| Conference WiFi unreliable | Server on presenter's laptop + phone hotspot; mitigation in development plan's hosting phase |

(Engineering risks — server, generation, auto-camera — live in the
[development plan's risks section](../../plans/6-audience-generative-art/).)

## See also

- [`../../plans/6-audience-generative-art/`](../../plans/6-audience-generative-art/) —
  sibling development plan (engineering work + sub-arcs + cross-
  arc dependencies)
- [`../par/`](../par/) — reference for slide-deck file layout +
  presenter script structure
