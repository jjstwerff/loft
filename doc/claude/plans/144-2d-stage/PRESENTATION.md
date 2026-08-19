<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN144 — The presentation model

Why a 2-D scene can present a 3-D world with three knobs, what the two scroll modes share,
and why occlusion gets no engine mechanism. Reference for arc **P**; the plan and its phase
gates are in [README.md](README.md).


**The 2D view is a PRESENTATION of the 3D world, not a second world** — which is what a
GameMaker-shaped game already is: a room whose instances carry a `depth`, sprites whose
**origin** sits at their feet, and `depth = -y` doing the 2.5D work. The hex is only the
footprint; the sprite stands *up* from it, so where it sorts is decided by its origin and
never by its artwork. Get that backwards and a tall sprite sorts by its own top edge, which
is the classic 2.5D wrong picture.

So `stage` stays a 2D presenter and the world stays 3D in the app, with **three knobs as the
entire contract between them**: a projected position, a sprite origin, and `layer` + `depth`.
A plain 2D game sets all three trivially. That is P1; A3's run-grouping is what keeps it true
once batched.

**Two scroll modes, one mechanism.** Scrolling the whole world in place and scrolling the
front faster than the back are not two code paths: they are one camera with a **per-layer
parallax factor**, and the flat mode is every factor at `1.0`. That is P2, and it is why the
camera belongs to `stage` rather than to the app — an app-owned camera means rewriting every
node's position on every scrolled frame, which is exactly the O(N) per-frame work the
retained tree exists to avoid.

A **frame event** — a footstep on frame 3, a hitbox live on frames 4–6 — stays app-side for
the same reason occlusion does: the node's current frame is readable, so the app can act on
it, and a callback table in the library would be a mechanism for something already
expressible.

**Occlusion is the level designer's rule, and the engine gets no mechanism for it —
settled, not open.** A character walking behind a fence, a tree trunk, a window or a low
wall stays visible, because those things are narrow or mostly transparent and alpha does the
work. The rule is simply *do not place large solid objects in the foreground*. So there is
no cutaway, no fade-when-occluding, no height ceiling, and not even a *what covers my
subject* query: each buys a runtime mechanism to rescue a placement that should not exist.
It does make A4 and A5 load-bearing rather than polish — the rule holds only if a fence's
soft edge composites correctly and a click passes through its gaps. Should it ever need
help, the help is an **authoring-time check in the editor** (flag a placed sprite whose
solid region could hide a character behind it), never a runtime feature: advice at author
time, silence at run time.

