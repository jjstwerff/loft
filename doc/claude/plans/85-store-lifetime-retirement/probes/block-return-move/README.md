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

Fixed by the block_result materialize (switch ON), verified both backends:

| Probe | Shape | Baseline | Switch ON |
|---|---|---|---|
| `p1-sibling-reused-name` | two sibling blocks, local name reused | ❌ leak 1 | ✅ no leak |
| `p2-same-scope-used-after` | `a`,`b` bound, both read later | ❌ `3 4 3 4` corruption | ✅ `1 2 3 4` |
| `p3-loop-single-site` | pure block-temp in a loop | ✅ clean (boundary) | ✅ unchanged |
| `p3b-read-only-loop` | `f#read as P` read-only loop | ✅ clean (read is empty-dep) | ✅ unchanged |
| `p4-distinct-names` | distinct local names | ✅ clean (control) | ✅ unchanged |
| `p5-fn-return` | `a = mk(v)` (the move we mirror) | ✅ clean | ✅ unchanged |
| `p8-negative-borrow-outer` | block returns an OUTER var | ✅ `9 8` (must stay borrow) | ✅ unchanged |

**p8 is the guard against over-moving** — a block returning an outer binding is a
genuine borrow; the fix leaves it alone (`block_defines_var` is false), so the
store is not freed while its owner still holds it. Re-check p8 at every step.

## Separate residual (NOT block-return-move) — `p9-writeread-slot-leak`

`f += struct` then `f#read as struct` in one program leaks the WRITE's copy-temp
on **interp only** (native clean), struct-specific, pre-existing — a write-path
slot/free divergence, not the block-return borrow. The read block-return in
isolation (p3b) is clean, so this is a distinct cluster. Left for a follow-up.
