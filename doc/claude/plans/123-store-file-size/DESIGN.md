<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN123 — design and implementation steps

Overview + measurements: [README.md](README.md). This file is the build order.

Every step below lands on its own, is verifiable on its own, and leaves the tree
green. The first three land **inert** — analysis and mechanism with no caller — so
the behaviour change is one reviewable commit that can be reverted without taking
the machinery with it.

---

## The two invariants

**A — the capacity floor.**

> A store's capacity never falls below its **high-water mark**: the word past its
> last claimed block. Everything above the mark is free by construction, so
> lowering capacity to it cannot orphan a live record, and no `DbRef` — which is
> `(store_nr, rec, pos)`, a position, not a pointer — can name a word above it.

That is the whole safety argument for arc A, and it is why A needs no reference
tracking. It is *only* true if the mark is computed from the block chain itself
(`Store::usage`), never from a cached count.

**B — the image is the live graph.**

> A compacted image contains exactly the records reachable from the collection
> root, and loading it back yields a value-identical collection.

The check is a digest over every field, not a size: a smaller file that dropped
data satisfies every size assertion anyone would write.

---

## Failure paths (enumerate first — this is where the invariants came from)

| # | Path | Answer |
|---|---|---|
| F1 | A `DbRef` names a word above the new capacity | Impossible under invariant A — but ASSERT it (step A0), because "impossible" is the claim being made |
| F2 | The free tree still holds the truncated tail block | `fl_rebuild` **after** every shrink. A stale `free_root` hands out a block past the end |
| F3 | `set_len` down succeeds, the remap fails | `MmapStorage::resize` already restores the old mapping on error and returns `Err`; the caller must treat `Err` as "did not shrink" and leave `self.size` alone |
| F4 | The `.dmeta` durable sidecar records byte length + CRC | Truncation invalidates it. Refuse to shrink a store with a live sidecar, or re-seal — decide in A3, do not silently break `store_durable_check` |
| F5 | Another process has the file mapped | Out of scope: shrink only a store this process owns and has bound. Write it down rather than discovering it |
| F6 | Shrinking below `bind_path`'s 1024-word floor | Clamp; `Store::open` resizes anything smaller to 8192 bytes anyway |
| F7 | The store keeps growing after a shrink | It re-grows at 7/3. Acceptable, but measure the churn: a threshold that fires every free would thrash |
| F8 | Coalescing merges blocks the free tree indexes | `coalesce_free` already rebuilds the tree; keep that ordering |
| F9 | Arc B rebuilds indexes and the key order changes | Digest must be order-independent, or compare per key |

---

## Arc A — a bound store's file shrinks to its high-water mark

### A0 — trust the mark (inert) — **DONE**

`StoreUsage::live_end_words` and the `tail%` / `inner%` report already landed with
the measurements. What was missing is the **calibration**: a blank or wrong mark
reads as "nothing to reclaim", which is indistinguishable from a healthy store.

**The assertion this step set out to add cannot fail.** `Store::usage` raises the
mark at every claimed block its walk passes, so "no claimed block begins at or
after `live_end_words`" is a restatement of the loop, not a check on it — and so
is every other phrasing against what that same walk saw (word accounting
included: `claimed + free` is built by the additions that move `pos`). Worth
writing down because it retires a whole family of assertions someone would
otherwise keep proposing here.

What the mark's trustworthiness actually rests on is **outside** the arithmetic:
whether the chain tiled the store at all. So A0 shipped a fact, not a check:

- **Code point:** `src/store.rs::Store::usage`.
- **Added:** `StoreUsage::walk_complete` — did the walk reach the store's end?
  It is false when a zero header stops the walk ("malformed / uninitialised
  tail") and when a `free` or under-one-block store is never walked. In those
  cases the mark is a **lower bound**, and a live record can sit above it.
  **A1 must refuse to shrink unless this is true.** Reporting is content with a
  lower bound; deciding is not, and that gap is the field's whole reason to exist.
- **Added:** `debug_assert!(live_end_words <= capacity_words)` — the one check
  here that *is* falsifiable: a final block whose header claims more words than
  remain drives the walk, and the mark with it, past the arena.
- **Verify:** three unit tests in `src/store.rs`'s test module — the mark's value
  on a healthy store, the malformed shape where the mark sits below a live
  record, and a `#[should_panic]` non-vacuity proof for the assertion.
- **Does not:** change any behaviour.

**The verification build matters here.** `[profile.dev.package.loft]` sets
`debug-assertions = false`, so *every* `cargo test` — `--profile dev` included —
strips the lib's `debug_assert!`s. An earlier draft of this step named that suite
as its verification, which would have been a calibration failure of its own:
green there says nothing about a DA-guarded claim. Run the
[`target-da` calibration lens](../../DEBUG.md#the-debug-assertions-calibration-run-target-da).

The non-vacuity test is `#[cfg(debug_assertions)]` for the same reason: it
compiles away together with the instrument it proves, so it can never pass by
not running.

### A1 — a shrink primitive with no caller (inert)

- **Code point:** `src/store.rs::resize_store` (1140) refuses `to_size <=
  self.size` and must keep doing so — it is the growth path and every caller
  relies on grow-only.
- **Add:** `Store::shrink_to(&mut self, words: u32) -> bool`, a sibling, not a
  change to `resize_store`:
  1. compute `usage()`; return `false` unless **`walk_complete`** (A0 — an
     incomplete walk reports the mark as a lower bound, and shrinking to a lower
     bound cuts live data) **and** `words >= live_end_words` (invariant A,
     checked against the chain, not against an argument);
  2. clamp to `>= 1024` (F6);
  3. mmap branch — `f.resize(words * 8)`; on `Err` return `false` and leave
     `self.size` untouched (F3); on `Ok` refresh `self.ptr` from
     `f.as_slice()`, exactly as the grow branch does;
  4. heap branch — `A.realloc` down, refresh `self.ptr`;
  5. set `self.size = words`, then **`fl_rebuild()`** (F2);
  6. bump `self.generation` (the same reason `resize` does: a suspended coroutine
     must be able to notice).
- **Verify:** a Rust unit test in `src/store.rs`'s test module — build a store,
  claim, free the tail, `shrink_to(live_end)`, assert capacity fell, assert every
  surviving record still reads its value, assert `fl_validate()` passes. And a
  negative: `shrink_to(live_end - 1)` returns `false` and changes nothing.
- **Does not:** get called from anywhere.

### A2 — reclaim = coalesce, then shrink (inert)

The tail after a mass removal is **thousands of unmerged free blocks** (2,696
mergeable pairs measured), because `coalesce_free` is lazy — `claim` runs it only
when an allocation would otherwise grow the store. So the mark is far below where
it could be, and shrinking alone reclaims almost nothing.

- **Code point:** `src/store.rs`, next to `coalesce_free` (1676).
- **Add:** `Store::reclaim_tail(&mut self) -> u32` — returns words freed:
  ```
  if self.needs_coalesce { self.coalesce_free(); }
  let end = self.usage().live_end_words;
  if self.shrink_to(end) { before - end } else { 0 }
  ```
- **Verify:** the unit test from A1 extended with the fragmented shape — free
  every other block, assert `reclaim_tail` returns ~the tail, and that
  `mergeable_free_pairs` is 0 afterwards.
- **Does not:** get called from anywhere.

### A3 — expose it as an explicit call: `store_reclaim(collection)`

**A is OPT-IN, and stays opt-in.** The earlier draft of this step gated it on
`used_pct()` inside `Store::delete`; that is wrong twice over, and both are worth
writing down because the second is the load-bearing one.

*It cannot be cheap enough to be automatic.* `used_pct()` comes from `usage()`,
an O(blocks) chain walk — per delete. Any always-on form needs an O(1) trigger
instead: a `freed_words_since_reclaim` counter maintained in `delete` (an add,
nothing more), and only when it crosses a threshold may anything walk. That is
the shape to use even for the explicit call's internals, so `delete` never grows
a walk.

*And the benefit is near-zero in almost every workload.* Measured, this plan's
own numbers: steady-state churn held capacity flat at 0.30 MB across 36,000
insert+remove operations, and refill (3000 → shrink to 300 → refill to 3000)
reused every byte with capacity never moving. Only a live set that drops far
below its peak **and stays there** has anything to give back. An always-on
reclaimer would walk, coalesce, and find nothing — for everyone.

*The thrash case is the one it targets.* A store that shrinks and then grows
again — a world unloading and reloading regions, the most plausible long-lived
shrink pattern there is — would be truncated and immediately re-grown at 7/3.
Whether a drop is permanent is something the program knows and the runtime
cannot infer, so the program says when.

- **Surface:** `store_reclaim(collection) -> integer` — bytes returned to the
  filesystem, 0 when there was nothing to give. A compiler special-case beside
  `reserve` (`src/parser/collections.rs`), lowering to an op that calls
  `Store::reclaim_tail` on the collection's store.
- **Code point for the counter:** `src/store.rs::Store::delete` (846), which
  already sets `needs_coalesce` — add the freed-words tally there, nothing else.
- **Decide here:** F4 — a store with a live `.dmeta` sidecar either re-seals
  (`store_durable_seal`) or refuses to shrink. Refusing is the safe default.
- **Verify:** the loft-level probe (bind 300 → grow 3000 → shrink 300) on both
  backends: the file falls when `store_reclaim` is called, is byte-identical when
  it is not, and the digest is unchanged either way.
- **Measure:** F7 — call it in a loop against the steady-state churn probe and
  confirm capacity does not thrash.

**The cost of opt-in, stated plainly:** a knob nobody knows about is a knob
nobody uses, so a shrunk store stays 5.8× in the wild. What makes that
acceptable is that the condition is already self-diagnosing — `store_memory()`
reports `inner%` / `tail%` — so the path is "notice the number, call the thing"
rather than loft unmapping a file under a running program. That fits *loft is
boring: noticed only in its absence* better than the automatic form does.

### A4 — document and graduate the probes

Document `store_reclaim` in [STDLIB.md](../../STDLIB.md) beside `reserve`, and in
[DATABASE.md](../../DATABASE.md) beside the high-water-mark image, each pointing
at the `inner%` reading that tells a consumer whether to bother. Probes graduate
to `tests/scripts/`. No default to flip — there is no automatic mode for A.

---

## Arc B — compact the interior when writing an image

**Do not start B until A3 has been measured on a real consumer.** A recovers the
tail; what B is worth is whatever `inner%` still reads afterwards, and that number
does not exist yet.

### B0 — the oracle first

- **Add:** a fixture that builds a store, records a per-field digest, persists,
  reloads and re-digests. This is the only thing that can tell a correct
  compaction from a lossy one (invariant B), and it must exist before any
  compaction code does.
- **Code point:** `tests/store_persist_loft.rs` beside the existing
  `persisted_size_tracks_content_not_construction`.

### B1 — measure what rebuild-and-swap costs

**Presumed approach: rebuild into a fresh store and swap.** Not relocation in
place.

| | Reuses | Risk |
|---|---|---|
| **rebuild + swap** — build a fresh collection, copy each record in, swap | `copy_block_cross_store` (`src/database/structures.rs:263`) and the sibling insert path | none — a fresh store has no inbound pointers at all |
| ~~relocate~~ — move live blocks down, rewrite inbound pointers | `relocate_ptr_fields` (`src/database/allocation.rs:3334`), bound to `PagedReader` so it needs generalising | must find every inbound pointer |

Two independent reasons, and the second is why this is no longer an open choice.

*It removes the risk rather than managing it.* Relocation's whole difficulty is
finding every pointer AT a moved record; a freshly built store has none, so there
is nothing to rewrite and nothing to get wrong.

*It is what the established engines do.* InnoDB does not relocate rows to reclaim
space — `OPTIMIZE TABLE` maps to `ALTER TABLE … FORCE`, which **rebuilds the
table into a new file and swaps it**, and the space only returns to the
filesystem with `innodb_file_per_table`. The whole shape of this plan lands on
the same model: deletes free space *within* the file and never shrink it, an
explicit operation reclaims it, and a free-space number (`DATA_FREE`, our
`inner%`) tells you whether it is worth running. Prior art at that scale is worth
more than a local preference.

So B1 is no longer "which approach" but **"what does it cost"**: build it against
B0's oracle and measure size, time and digest. The one number already in hand is
a warning, not a verdict — a *loft-level* re-insert produced a LARGER file
(634,264 vs 437,416) because writing records one at a time through the normal
path becomes the fill-then-insert shape. A rebuild that claims each vector at its
final size (what `reserve` exists for) should not; if it does, that measurement
is the finding.

**Not verified here:** whether recent MariaDB versions added any automatic
reclamation. If they did, it is worth reading before A's opt-in stance is final —
A is the arc with no prior art behind it.

### B2 — implement the winner behind a flag, off

- **Code point:** `src/database/allocation.rs::bind_path` (2514), the fresh-image
  branch, next to `store_image_live_end` / `build_padded_store_image` — this is
  already the one place an image is composed.
- Gate on the live-vs-mark ratio so a dense store is never rewritten.

**B is AUTOMATIC, and that is not inconsistent with A being opt-in.** The
difference is who already pays for the information: persist ALREADY walks the
chain — `store_image_live_end` does it today to size the image — so the ratio is
a subtraction on a walk that happens regardless. A dense store pays one
comparison and is written exactly as it is now. A's trigger, by contrast, would
have to walk on a path that does not otherwise walk at all.

The second difference is timing. B runs at a moment the program already chose,
once, and produces a fresh file; there is no mapping to invalidate and no
re-grow to thrash, because the live store is untouched. A mutates the store a
running program is using.

### B3 — on by default, documented, probes graduated

---

## Composition matrix

The done-bar for both arcs. Every cell on `--interpret` **and** `--native`, with
the digest unchanged in every one.

| Axis | Cells |
|---|---|
| binding | unbound (heap) · bound at create · bound then grown · bound then shrunk |
| shrink shape | oldest survive · newest survive · interleaved survivors |
| coalescing | lazy (never swept) · already swept |
| after reclaim | re-grown · re-bound · reloaded via `store_load` |
| durability | plain · with a `.dmeta` sidecar (F4) |
| size | below the 1024-word floor (F6) · at it · far above |

A no-output cell is vacuous: assert the digest AND the file size AND
`fl_validate()`, and prove the harness can fail by running it against a build with
the gate off.

## See also

- [README.md](README.md) — status, measurements, ruled-out list.
- [DATABASE.md](../../DATABASE.md) — the shipped high-water-mark image.
- [LIFETIME.md](../../LIFETIME.md) — removal frees what the element owned, without
  which none of this is measurable.
