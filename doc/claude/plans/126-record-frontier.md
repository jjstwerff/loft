# @PLN126 — tell a bound store a record is finished

> Promoted from loft#747. The issue is `status:future` and opens on a **measurement**,
> not an API: *does ordered insertion leave a finished record contiguous?* This file is
> the design half — the invariant that measurement is a test of, written down before the
> instrument existed, so the number has something to falsify.

## Goal

Let a program say **"this record is finished"** to a bound store, so a generator's
resident set follows its working set by intent rather than by the kernel discovering it
through eviction.

## What is already settled — do not re-chase

DATABASE.md § *Binding FIRST is the low-memory choice* carries the measured table.
Bind-first is 4.4× lower RSS than bind-last, a 4 M-feature build completes under a
32 MB cap, and a bound store's pages are reclaimable where anonymous heap is not. So
dataset size does not set a hard memory requirement. What remains is the cost of the
kernel learning the working set **by evicting the wrong pages first** — ~2× wall at a
modest cap, 271 s at an aggressive one.

`MemoryMax` alone proves nothing on a box with swap; `MemorySwapMax=0` is what makes a
cap a cap.

## The invariant the design rests on

The issue narrows three candidate shapes to one — append-only / ordered-write — because
`madvise(MADV_DONTNEED)` works on **pages**, and a per-record release cannot honour
"drop this record" when the record's bytes share pages with live neighbours. That
narrowing buys the plan a single sentence to defend:

> **A record finished in key order occupies a contiguous byte range that nothing writes
> to again — so "everything below the write frontier" names exactly the finished set,
> and the whole API is one call.**

Everything the plan proposes is downstream of that. It decomposes into three claims that
fail independently:

* **C1 — contiguity.** A finished record's own bytes (its block, its inner container
  blocks, its texts) form one range: `span ≈ live`, where `span` is the interval between
  its lowest and highest occupied word.
* **C2 — no re-touch.** Nothing written after the record is finished lands below the
  frontier that existed when it finished.
* **C3 — page exclusivity.** At the 4 KB granularity `madvise` actually operates on, a
  finished record's pages hold no bytes of a record still being written.

C1 is the claim the issue states. **C2 and C3 are the ones the API depends on**, and C1
can hold while both fail: a record can be perfectly contiguous and still sit on pages
that a later record's allocations reach back into.

## The failure mode this plan actually risks

Getting the frontier wrong is **not** a correctness fault. `MADV_DONTNEED` on a shared
file-backed mapping, after `msync`, drops resident pages and re-reads them from the file
on the next access — so a hint that drops the wrong page produces the right answer,
slowly. The whole risk is therefore a **silent green**: the call ships, the API reads
well, and it buys nothing that a counter would have shown was not there.

That is why the measurement comes first, and why it has to report re-touch and page
exclusivity rather than only the ratio the issue named.

## Counting the re-assertion sites

For C2 to hold, *every* site that decides where bytes land must respect the frontier.
Reading `src/store.rs`, placement is decided in four places:

| site | what it does |
|---|---|
| `claim` → `fl_take_ge` | **best fit**: the *smallest* free block ≥ the request, wherever it sits |
| `claim` → `coalesce_free` → `fl_take_ge` | the same, after merging adjacent frees |
| `claim_scan` → `claim_grow` | first fit by linear scan, growing the arena only at the end |
| `resize` in-place | absorbs the physically-next block when it is free, at 7/4 |

Three of the four can place a write **below** an established frontier, and none of them
knows a frontier exists. So `N = 3`, and omission is silent by construction (see above).
That is the brittleness, known before any code: the design's premise is not a property
the allocator has, it is a property someone would have to give it.

## The prediction — written before the instrument

From reading the allocator, not from running it:

* **P1.** With a strictly sequential build (one record open at a time), C1 holds and
  `span/live ≈ 1`. A growing vector at the top of the arena has the free tail as its
  physical successor, so `resize` absorbs it in place and never relocates.
* **P2.** With `W > 1` records open at once, a record's vector has a live neighbour
  behind it, so `resize` falls through to `claim` + `copy` + `delete`. The relocation
  goes to the frontier and leaves a hole. `span/live` grows with `W`.
* **P3.** C2 fails even at `W = 1`, and best fit is why: the hole a relocation leaves is
  the *smallest block that fits* for some later small allocation, so later records get
  pulled back down into it. The outer collection's own growth leaves such holes low in
  the arena from the very start.
* **P4.** C3 is strictly weaker than C2 and fails wherever C2 does, plus wherever a
  record's block merely *shares* a 4 KB page with a live neighbour — which at these
  record sizes is most of them.

If P1–P4 hold, the issue's own decision rule fires: *"the append-only shape collapses
into the same granularity problem as the per-record one, and this plan should be
re-scoped or declined rather than built."*

The prediction is written to be falsifiable in the direction that matters: if
`span/live` stays at 1 and re-touch stays at 0 across the whole matrix, the frontier
primitive is straightforward and the API is one call.

## The instrument

`src/database/spans.rs` (`#[cfg(test)]`, `#[ignore]`) — a measurement, not a gate.

It walks a built collection through `Stores::for_each_owned_child`, the Cluster-C
ownership keystone, so "the blocks a record owns" is answered by the same code that
frees them. A second enumeration would be a second definition of ownership, and the
whole question is which bytes belong to whom.

Per top-level record it reports `live` words, `span`, the **foreign live words inside
that span**, and the 4 KB pages it touches split into exclusive and shared. Across the
build it replays allocation order to report what fraction of writes land below the
frontier that existed when each record finished.

The axes it varies are recorded with the results below, including the ones held fixed.

## Results

`cargo test --release --lib database::spans -- --ignored --nocapture`. 2 000 tiles,
8 ways each, 12 vertices a way, unless a row says otherwise. `W` is how many tiles the
stream keeps open at once; `W=1` is the plan's premise taken at its word.

| | span/live mean | p95 | foreign in span | exclusive pages | droppable below frontier |
|---|---|---|---|---|---|
| W=1 | **356×** | 918× | 98.9% | **0.0%** (1/6052) | **98.7%** |
| W=2 | 364× | 935× | 98.8% | 0.0% | 96.9% |
| W=4 | 382× | 1030× | 98.4% | 0.0% | 95.4% |
| W=16 | 391× | 974× | 98.5% | 0.0% | 93.0% |
| W=64 | 540× | 1107× | 98.1% | 0.1% | 86.9% |
| W=1, 8 000 tiles | 1348× | 3555× | 99.6% | 0.0% | 99.4% |

**C1 is false by two to three orders of magnitude, and at `W=1` most of all.** The cause
is not the one the issue expected. Vector reallocation barely matters; what separates a
tile's bytes is that the outer `hash` keeps its entries in a chunked ARENA. A chunk is
claimed early and holds ~63 tiles; each of those tiles' vectors are claimed later, at
whatever the frontier was. So a tile's lowest word is in a chunk near the bottom of the
arena and its highest is near the top, with the entire store in between.

The lone-tile cell shows it with nothing else alive at all — 5 words of arena slot, then
**318 words of the collection's own spine**, then 28 words of vectors:

```
[     1..    22)   21w  spine      (the root record)
[    22..    27)    5w  tile 0     (its arena slot)
[    27..   345)  318w  spine      (the rest of the chunk, and the bucket table)
[   345..   373)   28w  tile 0     (vector<TStep> 18w + vector<TRoad> 10w)
```

**C3 is false, and it is the sharper refusal.** Of the 4 KB pages one tile touches,
**0.0%** hold only that tile — not at any window, not at any tile count. Exclusivity
appears only when a single tile grows past a page of its own (128 ways per tile → 71.7%),
which is not the shape a tile index has.

**C2 is true, and my own P3 predicted it false.** 87–99.4% of the pages below a tile's
finish-frontier hold nothing that is written afterwards. Best fit does backfill freed
holes, but a relocating vector doubles, so the block it frees is too small for the next
request and the reuse stays local in time rather than reaching back across the arena.

## What that changes

The issue reasoned: a per-record release fails on page granularity → therefore the
append-only shape → therefore contiguity is the condition to check. The measurement
splits that chain in the middle.

* **The per-record shape is dead**, and now measured rather than argued: 0.0% exclusive
  pages is the granularity problem in a number.
* **The frontier shape does not need contiguity.** It needs the region below the mark not
  to be written again — a property of the ALLOCATOR on an append-only workload, not of
  any record's layout. That property holds. So the plan is re-scoped, not declined, and
  the condition it re-scopes onto is the one that was measured true.

## What it cost to make the frontier call work

Three things the design could not have known, each found by an instrument rather than by
reading (`a_frontier_release_moves_the_resident_set`, peak RSS off `/proc/self/statm`):

1. **Reading the frontier cost the very thing the call saves.** `Store::usage` derives
   the high-water mark by walking the block chain, one header per block — which touches
   every page of the arena. Asking it for the mark faulted the whole store back in to
   decide what to drop: peak RSS became the *entire file* (80.9 MB against 44.3 MB for
   the same build with no call at all). The mark is now carried forward in
   `Store::claimed_end` at the one site where a block becomes claimed.
2. **Re-flushing from zero each time is quadratic.** Flushing "everything below the
   frontier" per record re-syncs a region that grows with the run: 208× the wall clock.
   `Store::released_bytes` bounds each call to the bytes since the last one.
3. **`MS_SYNC` is 200× and `MS_ASYNC` is free.** Both reach the same resident set;
   waiting for the writeback costs ~1.5 ms a call. Asynchronous, the identical drop
   measures 0.8–1.1× wall — occasionally faster than not calling it, because writeback
   starts early instead of arriving all at once at the end.

With all three: **44.3 MB → 2.2 MB peak RSS (20×) on an 89 MB build, at 1.0× wall**, one
call per record.

| build | file | peak RSS, no call | with a call per record | wall |
|---|---|---|---|---|
| 4 000 tiles, W=1 | 16.4 MB | 10.2 MB | **0.9 MB** (11×) | 0.6× |
| 20 000 tiles, W=1 | 89.3 MB | 44.3 MB | **2.5 MB** (17×) | 1.1× |
| 4 000 tiles, W=16 | 7.0 MB | 6.5 MB | 6.5 MB (1.00×) | 1.0× |
| 20 000 tiles, W=16 | 38.3 MB | 26.6 MB | 26.3 MB (1.01×) | 1.0× |

**It pays for an ordered build and does nothing for an interleaved one**, and the
attribution is the free-block count, not the layout: 10 free blocks at `W=1` against
3 691 at `W=16` for the same data. The free-space tree is an LLRB whose nodes live
*inside the freed blocks*, so a scattered arena gives the allocator itself a scattered
working set — and that working set is exactly the region the release is dropping. The
page-fault counter confirms the pages are dropped and read straight back rather than
never dropped.

This is why the docs say to stream in key order, and it is a stronger reason than the
plan had: ordering is not a nicety that makes the hint tidier, it is the condition under
which the hint does anything at all.

## Decision — re-scope and build, on the measured premise

Built:

* `Store::release_resident` — `msync(MS_ASYNC)` + `madvise(MADV_DONTNEED)` over the
  whole pages below the carried mark that have not been released yet.
* `Stores::release_store`, and `store_release(collection) -> integer` in
  `default/02_files.loft` + `src/native.rs`.
* `Store::claimed_end` / `Store::released_bytes` — the two pieces of bookkeeping without
  which the call is quadratic and self-defeating.

Not built, and now refused on evidence rather than on a hand-wave: a per-record release.
0.0% of a record's pages are its own.

Gates: `tests/scripts/126-store-release-keeps-everything.loft` +
`store_release_keeps_every_record_and_reference_both_backends` (content, references and
file length survive, both backends, values hand-checked);
`database::spans::one_tile_footprint_is_the_blocks_it_owns` (the span instrument against
arithmetic); the two `#[ignore]` measurements above.

## The axis this could not see

Every cell here is a generator that only ever APPENDS. A workload that revisits finished
records — an editor, a simulation — would re-dirty pages below the frontier, and nothing
measured says what that costs. `store_release` is documented as a hint for streaming
writers for that reason, and the residue is what the dogfood loop is for.
