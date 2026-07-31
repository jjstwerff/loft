<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 123 — Give a store's file back

## Status

Open — problem space fully characterised, arc **A** designed, arc **B** needs its
relocation walk written down. No implementation yet.

Promoted from [loft#713](https://github.com/loft-lang/loft/issues/713) (closed in
favour of this plan), itself split out of [loft#710](https://github.com/loft-lang/loft/issues/710)
(closed — everything it reported is fixed). Every cheap alternative has been
measured and ruled out, so this is a **build** plan, not an investigation.

## Goal

A store's file size follows what it holds for the whole life of the store —
including a **bound (mmap) store**, which is what a long-lived store normally is.

## Effort + design

- **Effort:** M (A is S, B is M)
- **Design:** ~ — A settled, B's relocation walk not yet written
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

| | recovers | needs | risk |
|---|---|---|---|
| **A** — coalesce + truncate a bound store | the free TAIL above the last record | no record movement | low — everything above the last record is free by definition |
| **B** — compact when writing an image | the INTERIOR free space between records | relocation + pointer rewriting | contained — nothing holds a `DbRef` into a fresh copy |

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **A** — a bound store's file shrinks to its high-water mark | this README | Open — designed |
| **B** — compact the image when writing it, gated on `used%` | this README + [DATABASE.md](../../DATABASE.md) | Open — needs the relocation walk |

### A — truncate the tail

`resize_store` must stop refusing to shrink, and the tail has to be coalesced
first: after a mass removal it is thousands of unmerged free blocks (2,696
mergeable pairs measured), so a naive "is the top block free" check reclaims
almost nothing.

Safe by construction: everything above the last claimed block is free, so no live
record and no `DbRef` is affected. Recovers ~59% of the case above on its own.

The trigger wants to be cheap and rare — `used%` is already computed by
`Store::usage`, and this is the "bare minimum for actual changes" half: do nothing
until a store has actually given a lot back.

### B — compact the image

Compact while **writing** the image, not in the live arena. Nothing holds a
`DbRef` into a fresh copy, so there are no inbound pointers to fix and no way to
interfere with running code — which is what makes live-arena compaction dangerous
and this version not. Gate on `used%`.

Ceiling is the fresh-build number: 1,042,528 → ~179,072 bytes for the same 300
records.

## Phase ordering

1. **A first**, and stand-alone. It is the light half, it needs no design work,
   and it changes what B is worth: after A the interior is all that is left.
2. **Re-measure** with `store_memory()`'s `tail%` / `inner%` on a real consumer
   before starting B. B is speculative until a consumer's `inner%` says otherwise.
3. **B**, if the re-measurement justifies it.

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

1. **When does A fire?** On free when the freed block is the last one, at an
   explicit sync point, or on a `used%` threshold. Cheapest that still works.
2. **Does `MmapStorage::resize` shrink cleanly** on every target, and what happens
   to a reader that has the file mapped concurrently?
3. **Does B relocate, or re-insert?** Re-inserting into a fresh collection reuses
   the existing relocating-copy machinery (the paged loader already does this per
   entry) and needs no pointer rewriting at all — but it rebuilds indexes. Measure
   both before choosing.

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
