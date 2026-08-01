<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 123 — Give a store's file back

## Status

Open — **arc A is COMPLETE and shipped** (`store_reclaim(collection)`, opt-in,
both backends). Arc **B** is next and now has the number it was waiting for: after
A takes the tail, **65% of the remaining file is interior free space**, so B is
justified rather than speculative. What B still lacks is a cost measurement, not a
decision.

Promoted from [loft#713](https://github.com/loft-lang/loft/issues/713) (closed in
favour of this plan), itself split out of [loft#710](https://github.com/loft-lang/loft/issues/710)
(closed — everything it reported is fixed). Every cheap alternative has been
measured and ruled out, so this is a **build** plan, not an investigation.

## Goal

A store's file size follows what it holds for the whole life of the store —
including a **bound (mmap) store**, which is what a long-lived store normally is.

## Effort + design

- **Effort:** M (A is S, B is M)
- **Design:** ✓ — A opt-in via `store_reclaim`, B a rebuild-and-swap at persist
- **Last touched:** 2026-08-01

## The measurement this plan exists for

A bound store: bind at 300 records, grow to 3000, shrink back to 300.

```
after bind (300 live)   file   177,696   used 89%   tail 11%   inner  0%
grown to 3000           file 2,257,392   used 70%   tail 30%   inner  0%
shrunk back to 300      file 2,257,392   used  8%   tail 59%   inner 33%
```

**12.7× the file for the same 300 records**, and it never comes back down:
`resize_store` returns early on `to_size <= self.size`, so a bound store's file
only ever grows.

The waste splits in two, and the split is only visible with the `tail%` / `inner%`
reading `store_memory()` now carries:

| | recovers | needs | risk | when |
|---|---|---|---|---|
| **A** — coalesce + truncate a bound store | the free TAIL above the last record | no record movement | low — everything above the last record is free by definition | **opt-in**, `store_reclaim(collection)` |
| **B** — compact when writing an image | the INTERIOR free space between records | a rebuild into a fresh store, then swap | low — a fresh store has no inbound pointers to rewrite | automatic, gated on the ratio |

**A is opt-in on purpose.** Its benefit is near-zero in every workload measured
here except one (churn and refill both hold capacity flat, so a reclaimer would
walk and find nothing), its trigger cannot be made cheap enough to sit on the
free path, and a store that shrinks then grows again would be truncated and
re-grown at 7/3 — thrashing the very case it targets. Whether a drop is permanent
is something the program knows and the runtime cannot infer. B stays automatic
because persist already walks the chain, so its gate costs nothing new.

## Sub-arcs

Build order, failure paths and code points: **[DESIGN.md](DESIGN.md)**.

| Item | Source | Status |
|---|---|---|
| **A0** — trust the high-water mark (`walk_complete`) | [DESIGN.md](DESIGN.md) | **Done** — inert |
| **A1** — `Store::shrink_to`, no caller | [DESIGN.md](DESIGN.md) | **Done** — inert |
| **A2** — `reclaim_tail` = coalesce + shrink, no caller | [DESIGN.md](DESIGN.md) | **Done** — inert |
| **A3** — expose `store_reclaim(collection)` (opt-in) | [DESIGN.md](DESIGN.md) | **Done** — the behaviour change |
| **A4** — docs + probes graduate (no default to flip) | [DESIGN.md](DESIGN.md) | **Done** |
| **B0** — the digest oracle, before any compaction code | [DESIGN.md](DESIGN.md) | Open — arc A's driver already digests |
| **B1** — measure what rebuild-and-swap costs | [DESIGN.md](DESIGN.md) | Open — approach settled |
| **B2/B3** — implement behind a flag, then default on | [DESIGN.md](DESIGN.md) | Open |

### A — truncate the tail

`resize_store` keeps refusing to shrink — it is the growth path — so the shrink
is a sibling, `Store::shrink_to`. A naive "is the top block free" check would
reclaim almost nothing after a mass removal (2,696 unmerged mergeable pairs
measured), which is why the cut is made at the **high-water mark** instead;
that reads the same swept or unswept, so coalescing is about the interior, not
about how much tail comes back (DESIGN.md § A2).

Safe by construction: everything above the last claimed block is free, so no live
record and no `DbRef` is affected. Measured on the graduated probe, it halves the
file — 247,944 → 124,872 bytes for the same 300 records, digest unchanged.

Reached through `store_reclaim(collection)` — the program says when. "Bare
minimum for actual changes" taken literally: do nothing at all unless asked, and
nothing on the free path ever walks the chain. The planned freed-words tally in
`Store::delete` turned out to be unnecessary AND unsound as an early-out (a store
that only ever grew has a tail with no delete behind it) — dropped, see
DESIGN.md § A3.

### B — compact the image

Compact while **writing** the image, not in the live arena. Nothing holds a
`DbRef` into a fresh copy, so there are no inbound pointers to fix and no way to
interfere with running code — which is what makes live-arena compaction dangerous
and this version not. Gated on the live-vs-mark ratio, which persist already has
from the chain walk it does anyway — so a dense store pays one comparison.

Ceiling is the fresh-build number: 1,042,528 → ~179,072 bytes for the same 300
records.

## Phase ordering

1. ~~**A first**, and stand-alone.~~ **Done.**
2. ~~**Re-measure** with `store_memory()`'s `tail%` / `inner%`.~~ **Done**, on
   this plan's own bound-store shape (a consumer measurement would still be worth
   having, and would only sharpen the number):

   ```
   before store_reclaim   cap 0.262 MB  used 25%  tail 28%  inner 47%   2701 free blocks
   after                  cap 0.188 MB  used 35%  tail  0%  inner 65%      4 free blocks
   ```

   A takes the whole tail; the sweep collapses 2,701 free blocks to 4. **65%
   interior remains** — that is what B is worth, now measured.
3. **B** — justified. Start at B1 (cost of rebuild-and-swap).

## Composition matrix — Stage A

This plan changes an *operation* (when a store's file is sized), not a value or
type, so the axes that matter are lifecycle and backend, not type-shape:

| Axis | Cells |
|---|---|
| binding | unbound (heap) · bound at create · bound then grown · bound then shrunk |
| shrink shape | oldest survive · newest survive · interleaved survivors |
| coalescing | lazy (never swept) · swept |
| backend | `--interpret` · `--native` |
| reload | file re-opened after truncation · re-bound and grown again |

The done-bar: every cell green on both backends, with the digest unchanged across
truncate/compact — a smaller file that lost data would pass every size assertion.
Probes graduate to `tests/scripts/`.

## Open design questions

1. ~~When does A fire?~~ **Settled: the program calls it.** Automatic was
   rejected — the benefit is near-zero in every measured workload but one, the
   trigger cannot be cheap on the free path, and a shrink-then-grow store would
   thrash. See DESIGN.md § A3.
2. **Does `MmapStorage::resize` shrink cleanly** on every target, and what happens
   to a reader that has the file mapped concurrently?
3. ~~Does B relocate, or re-insert?~~ **Settled: rebuild into a fresh store and
   swap.** It removes compaction's whole risk (a fresh store has no inbound
   pointers to rewrite), and it is what InnoDB does — `OPTIMIZE TABLE` is a
   rebuild-and-swap, not an in-place relocation. What remains is measuring its
   cost. See DESIGN.md § B1.
4. **Is any of A's stance contradicted by recent MariaDB?** Not verified. A is the
   arc with no prior art behind it, so it is worth a read before finalising.

## Ruled out — measured, do not re-chase

- **Memory growth.** There is none. 3000 → shrink to 300 → refill to 3000 held
  capacity at 1.57 MB throughout (97% → 12% → 96%); steady-state churn held
  0.30 MB flat across 36,000 insert+remove operations. The store is a peak
  allocator, not a leaking one.
- **What looked like unbounded growth** was a leak — removal released neither the
  record nor an owned `text`/`vector`. Fixed; no compactor could have reclaimed
  it, because those blocks were still marked LIVE.
- **Coalescing as a size fix on the persist path.** Forcing the sweep took 2,700
  free blocks to 6 and left `inner` unchanged at 45%: merging free blocks never
  moves a live one. It is a prerequisite for **A** and irrelevant to **B**.
- **Serialise/deserialise as a workaround.** `store_load` → persist is
  byte-identical (437,416 → 437,416) because `Store::load` replaces the slot's
  bytes wholesale; re-inserting record-by-record gives 634,264 — *larger*.
- **In-place growth.** `Store::resize` already absorbs an adjacent free block; it
  rarely applies when every vector is followed by another, which is what `reserve`
  sidesteps.

## See also

- [DATABASE.md § What the file's SIZE and BYTES mean](../../DATABASE.md) — the
  shipped half (high-water-mark image, `LOFT_HASH_SEED`).
- [STDLIB.md § `store_memory`](../../STDLIB.md) — `tail%` / `inner%`, the reading
  this plan is triggered by, and `reserve(v, n)`.
- [LIFETIME.md § Removing from a collection](../../LIFETIME.md) — the removal leak
  that had to be fixed before any of this could be measured.
- `@PLN123` — [loft-lang/plans#123](https://github.com/loft-lang/plans/issues/123).
- Promoted from [loft#713](https://github.com/loft-lang/loft/issues/713); source
  report [loft#710](https://github.com/loft-lang/loft/issues/710) (`hit-by:routing`).
