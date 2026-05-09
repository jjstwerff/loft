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

| Beat | Time-share | What happens |
|---|---|---|
| Open with the project goal | brief | "Browser games anyone can play via a shared link" — frame loft's lane |
| Set up the demo URL | brief | QR code + short URL; "open this on your phone, pick a color" |
| Round 1 — let the world grow | longer | Audience taps; growth spreads; auto-camera tracks the action; presenter narrates |
| Show the generation code | shorter | Project the loft script; ~30-50 lines; presenter reads through it |
| Round 2 — presenter tweaks the script | longer | Live edit between rounds: "watch what changes when growth-rate doubles" / "what if older cells die" |
| Round 3 — switch generation variant | longer | Plant → crystal (or vice versa); "same audience inputs, different aesthetic" |
| Q&A | open | "Where do I get this?" / "What can it do beyond paint?" |

The split between "round" beats and "show the code" beats is the
narrative arc: spectacle → reveal → re-spectacle.

## Audience-participation flow

1. **Onboarding** (30 seconds): QR code on slide → URL.  Browser
   page loads.  Color picker visible.  Hex grid visible.
2. **First tap**: presenter taps own phone first to break the ice;
   shows the projected hex appear; "now everyone try."
3. **During growth**: no presenter narration on top of activity —
   let the audience see the result of their input.
4. **Between rounds**: presenter takes the floor, edits script,
   resets the world.  Audience knows to wait + watch.
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
| `presenter-notes.md` | What can go wrong + how to recover; URLs / IPs / colors / dial-in info for the demo machine |

Build script + content land when the development plan reaches
phase 5 readiness.

## Open presentation-side questions

1. **QR-code → URL onboarding** — single big QR on a "join here"
   slide?  Or persistent corner element on every slide?  First is
   cleaner; second helps late arrivals.
2. **Voice + visual rhythm during rounds** — silence during growth
   (let the visual carry it) vs. light narration ("watch how blue
   pushes north")?  Probably mix per round.
3. **Reveal the code BEFORE or AFTER round 1?** — Showing first
   builds anticipation (audience knows what they're influencing);
   showing after leverages mystery (audience figures out the rules
   from observation, then sees the rules).  Recommend AFTER for the
   first round, BEFORE for round 2 (when presenter tweaks it).
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
| Live script edit crashes | 3-5 known-good scripts pre-staged; switch via hotkey if a tweak crashes |
| Conference WiFi unreliable | Server on presenter's laptop + phone hotspot; mitigation in development plan's hosting phase |

(Engineering risks — server, generation, auto-camera — live in the
[development plan's risks section](../../plans/future/36-audience-generative-art/).)

## See also

- [`../../plans/future/36-audience-generative-art/`](../../plans/future/36-audience-generative-art/) —
  sibling development plan (engineering work + sub-arcs + cross-
  arc dependencies)
- [`../par/`](../par/) — reference for slide-deck file layout +
  presenter script structure
