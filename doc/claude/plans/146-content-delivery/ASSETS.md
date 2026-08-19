<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN144 — The asset route: a store on a file server, not a bundler

Reference for arc F. The plan and its phase gates are in [README.md](README.md).


The obvious first pass is a `--html` flag that bundles referenced files into the
page, the Flash `[Embed]` shape. **Do not build that.** The route already exists and
is better: an asset pack **is a loft store**, hosted on any dumb file server and read
by HTTP range so only the bytes a lookup touches cross the wire —
[REMOTE_STORES.md](../../REMOTE_STORES.md) documents this for exactly this case
(*"world chunks, meshes, textures, sounds, animations, dialogue, level data"*), and
the `routing` project already ships it for map tiles (`PLAN-TILES.md`): the store's
layout is schema-derived, so there is no codec, no parse step and no serialize seam —
the struct definition **is** the file layout.

Two constraints carry over from routing, and F3 exists to hold the first:

- **Plan → fetch → read, never fetch-on-miss inside a frame.** Synchronous wasm cannot
  await, and a frame blocking on a range read stutters visibly. Assets are requested at load
  or level boundaries, or as a ring around the player.
- **Verify the layout fingerprint across native and wasm before anything reads a pack**
  (routing's B.2). A silent divergence turns every asset into garbage at a byte offset,
  which reads as a corrupt file rather than a layout bug.

Embedding stays for the bytes a page needs before its first fetch — a boot font, a loading
sprite — the exception, not the pipeline.

