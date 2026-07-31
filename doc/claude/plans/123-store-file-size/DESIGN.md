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

### A0 — trust the mark (inert)

`StoreUsage::live_end_words` and the `tail%` / `inner%` report already landed with
the measurements. What is missing is the **calibration**: a blank or wrong mark
reads as "nothing to reclaim", which is indistinguishable from a healthy store.

- **Code point:** `src/store.rs::Store::usage` (~1088).
- **Add:** a `debug_assert!` that no claimed block begins at or after
  `live_end_words`, and that `live_end_words <= capacity_words`.
- **Verify:** the debug-assertion suite (`cargo test --profile dev --test wrap
  loft_suite`) exercises it across every existing store shape.
- **Does not:** change any behaviour.

### A1 — a shrink primitive with no caller (inert)

- **Code point:** `src/store.rs::resize_store` (1140) refuses `to_size <=
  self.size` and must keep doing so — it is the growth path and every caller
  relies on grow-only.
- **Add:** `Store::shrink_to(&mut self, words: u32) -> bool`, a sibling, not a
  change to `resize_store`:
  1. compute `usage()`; return `false` unless `words >= live_end_words` (invariant
     A, checked against the chain, not against an argument);
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

### A3 — call it, behind an env gate (behaviour, off by default)

- **Code point:** `src/database/allocation.rs::Stores::remove_owned`'s callers are
  the wrong altitude — the decision is per-STORE, not per-removal. Put the check
  where a store learns it lost something: `src/store.rs::Store::delete` (846),
  which already sets `needs_coalesce`.
- **Trigger:** only when all of — the store is file-backed (`self.file.is_some()`,
  the "long-lived" gate, free to test), `used_pct()` is below a threshold, and at
  least N words would come back. Cheap enough to sit on the free path: one
  `Option::is_some` for every store that is not bound.
- **Gate:** `LOFT_STORE_RECLAIM=1` reads the threshold; unset = today's behaviour
  exactly.
- **Decide here:** F4 — a store with a live `.dmeta` sidecar either re-seals
  (`store_durable_seal`) or refuses to shrink. Refusing is the safe default and
  can be relaxed later.
- **Verify:** the loft-level probe (bind 300 → grow 3000 → shrink 300) run with
  the gate on and off, both backends, asserting the file falls with the gate on,
  is byte-identical without it, and the digest is unchanged either way.
- **Measure:** F7 — the churn cost. Run the steady-state churn probe (36,000
  insert+remove) with the gate on and confirm capacity does not thrash.

### A4 — default it on, document, graduate the probes

Flip the default once A3's measurements are in, keeping `LOFT_STORE_RECLAIM=0` as
the escape hatch. Probes graduate to `tests/scripts/`. Document in
[DATABASE.md](../../DATABASE.md) beside the high-water-mark image section.

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

### B1 — choose relocate vs re-insert, by measuring

Two candidates, both already have machinery:

| | Reuses | Cost | Risk |
|---|---|---|---|
| **relocate** — walk the chain, move live blocks down, rewrite inbound pointers | `relocate_ptr_fields` (`src/database/allocation.rs:3334`) — but it is bound to `PagedReader`, so it needs generalising | one pass | must find every inbound pointer |
| **re-insert** — build a fresh collection and copy each record in | `copy_block_cross_store` (`src/database/structures.rs:263`), and `Stores::remove_owned`'s sibling insert path | rebuilds indexes | none — a fresh store has no inbound pointers at all |

Re-insert is the honest first choice: it needs **no** pointer rewriting, which is
the entire risk of compaction. Its known cost is measured and bad — a loft-level
re-insert produced a *larger* file (634,264 vs 437,416) because it becomes the
fill-then-insert shape — so B1 is a measurement, not an assumption: build both
against B0's oracle on the same fixture and compare size, time and digest.

### B2 — implement the winner behind a flag, off

- **Code point:** `src/database/allocation.rs::bind_path` (2514), the fresh-image
  branch, next to `store_image_live_end` / `build_padded_store_image` — this is
  already the one place an image is composed.
- Gate on `used_pct()` so a dense store is never rewritten.

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
