<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Audience-driven generative art — presentation

## Status

Open.  Scoped 2026-05-09.  Sibling **development plan** at
[`../../plans/future/36-audience-generative-art/`](../../plans/future/36-audience-generative-art/)
owns the engineering work (server, projector view, generation
script, hosting).  This file owns the **presentation work** (talk
shape, slides, audience-participation flow, presenter notes).

## Goal

Deliver a memorable, audience-participatory creative-coding talk
to a local meetup of game creators + art enthusiasts.

The audience walks away with concrete examples of "I could write
this" — short, readable loft code projected on screen, doing
something visibly *interesting that's hard to find elsewhere*:
collaborative shared state via a single URL + a generative script
that can be tweaked between rounds, all in one language across
server / client / generation.

## Demo concept

Audience members influence a **plant / crystal growth** simulation
on a hex map in two ways:

- **Tap a hex** in their browser → seeds growth at that location
- **Choose a color** → biases the dominant color in the direction
  growth spreads

Native projector view auto-cameras to recent activity.  Single
shared URL — everyone in the room participates from their phone or
laptop.  Spectacle is the **emergent collaborative patterns**, not
any single person's contribution.

(Full design: see the [development plan](../../plans/future/36-audience-generative-art/)
including the generation algorithm sketch, auto-camera tuning, and
6 open design questions.)

## Talk shape

The demo runs **continuously through most of the event** — no
rounds, no manual resets, no pauses for setup between segments.
The canvas evolves from blank at the start to whatever the
audience has built and the decay rule has eroded by the end.
Presenter weaves narration and code reveals into the ongoing
painting without breaking the spectacle.

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
| Set up the demo URL | brief | QR code + short URL; "open this on your phone, pick a color" |
| Audience starts painting | longer | Audience taps; first crystals grow on the projector; presenter narrates light enough to let the visual carry |
| Reveal the client code | shorter | Project the audience-page loft / HTML; ~30-50 lines; presenter reads through it while painting continues in the background |
| Reveal the server code | shorter | Project the server-state loft; same shape — read through while painting continues |
| Reveal the projector code (mesh + auto-camera) | shorter | Project the renderer loft; talk through the edge-detection rule that makes plant aesthetic emerge |
| Decay narration moment | brief | Point out the edges receding — "no one's removing them; the world erodes itself.  Watch where it eats from first" |
| Q&A | open | "Where do I get this?" / "What can it do beyond paint?" |

Narrative arc: spectacle (audience paints) → reveal (here is the
code that makes this happen, in three small loft files) →
spectacle continues (the world they painted is still there,
visibly being shaped by both their input and the decay rule).

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

1. **QR-code → URL onboarding** — single big QR on a "join here"
   slide?  Or persistent corner element on every slide?  First is
   cleaner; second helps late arrivals.
2. **Voice + visual rhythm during rounds** — silence during growth
   (let the visual carry it) vs. light narration ("watch how blue
   pushes north")?  Probably mix per round.
3. **When to reveal each of the three code files** (client /
   server / projector) — early (anticipation: audience knows
   what each layer does), late (mystery: audience figures out
   rules from observation first), or interleaved across the
   talk?  Recommend interleaved — reveal client first
   (smallest, most relatable), server next (the lifecycle that
   feels like magic), projector last (the spectacle code).
4. **Single demo or include a second moros-editor segment?** — The
   moros editor as "look, real creator tool exists in loft" could
   bookend the audience-generative segment.  Adds time but
   broadens "what loft is."

## Risks (presentation-side)

| Risk | Mitigation |
|---|---|
| Audience hesitates to participate | Presenter taps own phone first; "demo audience" of 2-3 pre-paired phones on the podium for stage-tap simulation if real audience is shy |
| QR code fails to scan from back of room | Print a poster-sized QR; provide short URL as fallback on every slide |
| Demo machine crashes mid-talk | Pre-recorded video as fallback; presenter narrates over it; rehearse this fallback path |
| Server crashes mid-talk | Local restart from a saved state-snapshot (or accept starting from blank); rehearsal pins the recovery time |
| Conference WiFi unreliable | Server on presenter's laptop + phone hotspot; mitigation in development plan's hosting phase |

(Engineering risks — server, generation, auto-camera — live in the
[development plan's risks section](../../plans/future/36-audience-generative-art/).)

## See also

- [`../../plans/future/36-audience-generative-art/`](../../plans/future/36-audience-generative-art/) —
  sibling development plan (engineering work + sub-arcs + cross-
  arc dependencies)
- [`../par/`](../par/) — reference for slide-deck file layout +
  presenter script structure
