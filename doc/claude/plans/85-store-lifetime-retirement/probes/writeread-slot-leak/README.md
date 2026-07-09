<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Probes — write+read struct residual (`p9`, @PLN85)

Boundary-finding matrix for the interp-only struct binary-I/O leak that @PLN47 /
block-return-move surfaced. Run cache-off (manual runs otherwise replay stale
`.loft/cache` bytecode): `LOFT_NO_CACHE=1 loft --interpret <probe>`. Baseline
2026-07-09, hand-verified both backends.

## Matrix — baseline

| Probe | Axis / shape | interp | native |
|---|---|---|---|
| `a1-write-same-type` | write P, read P | ❌ leak 1, out `5` | ✅ clean |
| `a2-write-different-type` | write Q (≠P), read P | ❌ leak 1, out `1` | ✅ clean |
| `a3-inline-literal-write` | `f += P{…}` (literal, no var) | ❌ leak 1, out **`16`** (should be `5`) | ✅ clean, out `5` |
| `a4-call-return-then-read` | fn-return struct precedes | ❌ leak 1 | ✅ clean |
| `b1-println` | read + `println(q.x)` | ❌ leak 1 | ✅ clean |
| `b2-no-use` | read, **q never used** | ❌ leak 1 | ✅ clean |
| `b3-noncall-use` | read + `n = q.x` (no call) | ❌ leak 1 | ✅ clean |
| `c1-one-field` | `struct One{v}` | ✅ **clean**, out **`null`** (should be `42`) | ✅ clean, out `42` |
| `c2-mixed-widths` | i32/i16/u8/integer | ❌ leak 1 | ✅ clean |
| `c3-nested` | nested plain struct | ❌ leak 1 | ✅ clean |
| `d1-two-reads` | write, then TWO reads | ❌ **leak 2** | ✅ clean |
| `d2-write-read-loop` | write+read × 5 in a loop | ❌ **leak 5** | ✅ clean |
| `e1-extra-live-locals` | live locals around the read | ❌ leak 1 | ✅ clean |

## What the matrix pins (and what it REFUTED)

**Real boundary of the leak:** one leaked record **per struct read**, iff a
struct write occurred earlier in the program. Independent of: the written type
(a2), any use of the result (b2 — *no use at all still leaks*), whether the use
is a call (b1) or not (b3), and surrounding live locals (e1). Scales per read
(d1 = 2, d2 = 5). Interp only; native clean throughout.

**REFUTED** (the value of building the matrix): the earlier root-cause —
"`q` reuses the write `_wf` temp's slot and the `PutRef` value fails to survive
the intervening `println` call" — is WRONG. `b2` leaks with **no use and no
call**, and `e1` leaks **with extra live locals** occupying nearby slots. So the
leak is not the consumer's slot reverting across a call; it is the **read
buffer** (`_read_1`'s store) not being freed once a struct write has run earlier
— a write→read interaction on the interp store/free path, not a consumer-slot
issue. The next investigation must start from the read-buffer free, conditioned
on a prior write, NOT from `q`.

## Two MORE interp-only bugs the matrix surfaced (likely same neighbourhood)

- **`a3` — inline-literal struct write corrupts on interp.** `f += P{x:5,y:6}`
  (a literal operand, vs `a1`'s `p = …; f += p`) reads back **`16`** on interp
  but `5` on native — an interp/native write divergence for a non-`Var` struct
  operand (the `_wf` copy-temp path serialises wrong bytes).
- **`c1` — one-field struct read returns `null` on interp.** `read as One` where
  `One{v:integer}` yields **`null`** on interp, `42` on native — and is the ONE
  leak-free row (single-field records take a different read path). Possibly the
  same buffer-handling defect seen inside-out.

All three are interp-only, struct binary-I/O, and probably share the read-
buffer / write-serialise root — a focused investigation keyed on this matrix
(loft debugger on the read-buffer free with a prior write) is the next step.
