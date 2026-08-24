<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN146 — the `.draw` corpus, its oracle, and arc W's gate

The specification arc W's loft port is measured against, and the gate that measures
it. `W0` collected the corpus and its findings ([../W0.md](../W0.md), with the grammar
census); `W2` renders it in loft and diffs ([../W2.md](../W2.md)).

```sh
./w0.sh            # render all 37 scenes through the ORACLE, compare against golden/
./w0.sh --bless    # rewrite golden/ from the current oracle
./census.py        # which commands and options the corpus actually uses

./w2.sh            # render the W2 subset through LOFT and diff it against golden/
./w2.sh --control  # …with a one-pixel error injected; the diff must see it

./w5.sh            # measure every scene's CHECKS in loft and diff the report + exit code
./w5.sh --control  # …with the default tolerance widened; the diff must see it

./w6.sh            # build one atlas page from a scene both with and without a PNG
./w6.sh --control  # …with one texel moved; the diff must see it
```

`w0.sh` is the oracle's gate — it says `golden/` is still what `draw.py` renders.
`w2.sh` is arc W's, and reads those same goldens: it renders every scene the loft
`drawing` package's grammar covers and compares DECODED PIXELS, not file bytes,
because two PNG encoders agreeing byte for byte is a weaker claim than two
renderers agreeing.  The subset is computed from the scenes rather than listed, so
it follows the corpus instead of drifting from it.  `W2_DRAWING` points at the
package's `src/`, `LOFT` at the binary, `W2_BACKEND` at the backend (`--native` by
default: the same run takes minutes on the interpreter and 30 seconds compiled).

`oracle/draw.py` is a **copy of `crawler/tools/draw.py`**, not a fork: crawler owns it and
edits belong there. It is here so the gate has an oracle that cannot move under it, and so
arc W can be diffed against the renderer it replaces. Refresh it by copying again — never
by editing in place.

`w5.sh` is W5's, and it is the only gate here with a corpus of its own:
`checks/` holds six scenes written for it, because exactly one scene in `scenes/` carries
`check` lines at all and a grammar cannot be measured by the one form that happens to be
in use.  It compares the loft package's report against the oracle's `stats.txt` blocks
byte for byte, and its exit status against `draw.py --once`'s.

`w6.sh` is W6's: it builds one atlas page out of the same scenes twice — once through a
PNG on disk, once straight from the rendered canvas — and compares the cells that come
back out of a written pack.

`scenes/` is likewise a copy: 36 sprites from `crawler/assets/sprites/src/` and loft's own
`sketch/scene.draw` as `old_woman.draw`.
