<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Probes — move-on-block-return (@PLN85 residual)

Executable spec for [../../block-return-move.md](../../block-return-move.md).
Run each on BOTH backends; a value/leak divergence from the "target" column is
the bug. Baseline captured 2026-07-09 (interp == native on every probe).

```bash
loft --interpret probes/block-return-move/p1-sibling-reused-name.loft
LOFT_NATIVE_LEAK_CHECK=1 loft --native probes/block-return-move/p1-sibling-reused-name.loft
```

| Probe | Shape | Baseline | Target |
|---|---|---|---|
| `p1-sibling-reused-name` | two sibling blocks, local name reused | ❌ leak 1 | ✅ no leak |
| `p2-same-scope-used-after` | `a`,`b` bound, both read later | ❌ `3 4 3 4` corruption | ✅ `1 2 3 4` |
| `p3-loop-single-site` | pure block-temp in a loop | ✅ clean (boundary) | ✅ unchanged |
| `p3b-file-read-loop` | `f#read as P` write+read per iter (PLN47) | ❌ leak N | ✅ no leak |
| `p4-distinct-names` | distinct local names | ✅ clean (control) | ✅ unchanged |
| `p5-fn-return` | `a = mk(v)` (the move we mirror) | ✅ clean | ✅ unchanged |
| `p8-negative-borrow-outer` | block returns an OUTER var | ✅ `9 8` (must stay borrow) | ✅ unchanged |

**p8 is the guard against over-moving** — a block returning an outer binding is a
genuine borrow; moving it would free a still-owned store (UAF). Re-check p8 at
every step.
