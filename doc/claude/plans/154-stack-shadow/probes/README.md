<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN154 phase 3 probes — the stale-handle matrix

These test the DETECTOR, not the language, which is why they live here and not in
`tests/scripts/`: three of them (`f-issue-*`) are the open issues loft#1373 / #1377 / #1384,
so a corpus guard asserting the right answer would be red until those are fixed.

```bash
for f in doc/claude/plans/154-stack-shadow/probes/*.loft; do
  echo "== $f"; LOFT_VERIFY_STACK=1 ./target/release/loft --interpret "$f" 2>&1 | tail -3
done
```

| probe | axis it moves | expected |
|---|---|---|
| `a-bound-after` | the view is bound AFTER the growth | silent |
| `b-rebound-each-iteration` | the view is re-bound after every growth | silent |
| `c-container-read-after-growth` | the CONTAINER's handle is what is read | silent |
| `d-growth-inside-allocation` | the growth moves no record | silent, **and no relocation line** |
| `e-view-across-a-call` | the view is read in a CALLEE's frame | REPORTS |
| `f-issue-1373` | a struct-element view | REPORTS |
| `f-issue-1377` | a collection-element view | REPORTS |
| `f-issue-1384` | a view of a struct FIELD's element | REPORTS |

`d` is the negative control that makes the rest mean something: it proves the mechanism is
silent because nothing moved, not because nothing is armed.
