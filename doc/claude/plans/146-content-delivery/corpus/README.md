<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN146 W0 — the `.draw` corpus and its oracle

The specification arc W's loft port is measured against. Findings and the grammar
census are in [../W0.md](../W0.md).

```sh
./w0.sh            # render all 37 scenes, compare against golden/ — the gate
./w0.sh --bless    # rewrite golden/ from the current oracle
./census.py        # which commands and options the corpus actually uses
```

`oracle/draw.py` is a **copy of `crawler/tools/draw.py`**, not a fork: crawler owns it and
edits belong there. It is here so the gate has an oracle that cannot move under it, and so
arc W can be diffed against the renderer it replaces. Refresh it by copying again — never
by editing in place.

`scenes/` is likewise a copy: 36 sprites from `crawler/assets/sprites/src/` and loft's own
`sketch/scene.draw` as `old_woman.draw`.
