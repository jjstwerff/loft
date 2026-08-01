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
| F9 | Arc B rebuilds indexes and the key order changes | **Not a live hazard** (B0): loft iterates a keyed collection through a KEY-SORTED SNAPSHOT, so no rebuild can reorder it — established by probing with an order-dependent fold across 3 insertion orders and 2 bucket seeds. The digest is order-independent anyway, for free |

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

### A1 — a shrink primitive with no caller (inert) — **DONE**

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

**Two things the sketch above did not have**, both found by building it:

*The tail has to be re-tiled, not just cut.* Truncating to somewhere ABOVE the
mark leaves free space that must become ONE block ending exactly at the new
size — otherwise the chain no longer partitions the store and every later walk
breaks on it. `retile_tail` does that, then `fl_rebuild`. Skipping the rebuild
is not a hygiene issue: with the free tree still naming words that were cut, the
unit test **SIGSEGVs**, which is F2 happening rather than being argued about.

*The 1024-word floor is a FILE fact, not a store fact.* It exists because
`Store::open` lifts a small file straight back up. A heap store has no such
floor — `Store::new` insists on two words — and clamping there would have made
the primitive refuse on every store under 8 KB, unit tests included. The floor
now has one home (`MIN_BOUND_WORDS`), which `Store::open` uses as well.

### A2 — reclaim = coalesce, then shrink (inert) — **DONE**

**Correction — the sweep does not move the mark.** This step was written on
"the tail after a mass removal is thousands of unmerged free blocks, so the mark
is far below where it could be, and shrinking alone reclaims almost nothing."
That is true of a *is the top block free* test, and this is not one:
`live_end_words` is the end of the last CLAIMED block, and merging free blocks
never moves a claimed one. The same store gives back the same tail swept or
unswept — pinned by the A2 test, which reclaims twice from identical stores.

The sweep stays, paying for a different thing: those unmerged pairs are in the
**interior**, and merging them is what decides whether the next large claim
reuses space or grows the store. An explicit, rare call is the one moment an
O(blocks) sweep is welcome — and it is the reason nothing on the free path ever
needs to walk.

- **Code point:** `src/store.rs`, next to `coalesce_free`.
- **Added:** `Store::reclaim_tail(&mut self) -> u32` — words given back:
  ```
  if self.needs_coalesce { self.coalesce_free(); }
  let before = self.size;
  let mark = self.usage().live_end_words;
  if self.shrink_to(mark) { before - self.size } else { 0 }
  ```
  Measured against `self.size` after the fact, never `before - mark`:
  `shrink_to` clamps to its floor, so the mark is not where it necessarily
  landed.
- **Verify:** two unit tests — the fragmented shape (tail returned, interior
  merged, store still usable, and the swept-first control), and a dense store
  reporting 0 while touching nothing.
- **Does not:** get called from anywhere.

**A test-shape trap worth keeping.** Freeing every OTHER record produces free
blocks that are not adjacent, so `mergeable_free_pairs` is already 0 and the
"the interior was swept" assertion says nothing. The fragmented shape has to
free in FORWARD runs of two — `delete` merges only with the block after it, and
that one is still claimed both times, which is what leaves a pair unmerged.

### A3 — expose it as an explicit call: `store_reclaim(collection)` — **DONE**

**It needed no compiler special-case, and no counter.** Two subtractions from the
sketch below, both worth keeping:

*Not a parser special-case.* `reserve` is one because it needs the element's
stored width — a static type fact available nowhere else. `store_reclaim` needs
only the reference's `store_nr`, which every call already carries, so it is a
plain stdlib declaration with a `#rust` body beside `store_verify`
(`default/02_files.loft`) plus the interpreter's registry handler
(`native.rs::n_store_reclaim`). Both backends print byte-identical numbers.

*No `freed_words_since_reclaim` counter.* The counter existed to keep `delete`
from walking — and `delete` does not walk, because A is opt-in and nothing on the
free path calls any of this. As an early-out for the explicit call it would also
be **unsound on its own**: a store that only ever grew has a tail (the 7/3
over-allocation) with no delete behind it, so "nothing freed since" does not mean
"nothing to give". Making it sound needs a second producer (growth), and a missed
producer silently under-reclaims. Dropped; the explicit call pays its one walk.

*F4 is decided and enforced at the truncation itself* (`Store::shrink_to`): a
store with a live `.dmeta` sidecar refuses, because the sidecar records byte
length + CRC and truncating behind its back reports a healthy store as corrupt.

**F7, measured.** 40 cycles of +300/−300 records over a 2,000-record hash:

| | end capacity | used | tail | resize traffic |
|---|---|---|---|---|
| left alone | 0.286 MB | 56% | 40% | — |
| `store_reclaim` every cycle | 0.173 MB | 93% | 0% | **9,537,312 bytes** |

So the thrash is real but not free-standing: per-cycle calling *does* hold the
store 1.65× denser, and pays 55× the store's own size in grow-and-shrink traffic
to save 0.11 MB. That number is the argument for opt-in, better than the
qualitative one: called once after a permanent drop it costs a single walk;
called on a cycle it buys a re-grow every time.

**The A-vs-B re-measurement** (phase ordering step 2), same bound store, before
and after one `store_reclaim`:

```
before   cap 0.262 MB  used 25%  tail 28%  inner 47%   free-blocks 2701 (2696 mergeable)
after    cap 0.188 MB  used 35%  tail  0%  inner 65%   free-blocks    4 (0 mergeable)
```

A takes the whole tail and the sweep collapses 2,701 free blocks to 4. What is
left is **65% interior** — B's target, now measured rather than assumed, on the
plan's own workload. B is justified.

#### The original step, for the record

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
  filesystem, 0 when there was nothing to give. ~~A compiler special-case beside
  `reserve`~~ → a stdlib declaration with a `#rust` body (see above).
- ~~**Code point for the counter:** `src/store.rs::Store::delete`~~ → dropped
  (see above).
- **Decided:** F4 — a store with a live `.dmeta` sidecar refuses to shrink.
- **Verified:** `tests/scripts/store_reclaim_123.loft` + the driver
  `store_reclaim_shrinks_a_bound_file_both_backends` — bind 300 → grow 3000 →
  drop back to 300: the file falls by exactly the bytes the call reports
  (247,944 → 124,872), the digest over every surviving record is unchanged, a
  second call returns 0 without touching the file, and the truncated store still
  takes new records and reads old ones. Both backends, byte-identical.

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

### B0 — the oracle first — **DONE**

`tests/scripts/store_digest_b0.loft` + `store_digest_b0_oracle_sees_loss_not_layout`.
A per-field digest over records carrying every field kind a rebuild must
relocate (integer, `i32`, variable-length `text`, variable-length `vector<i32>`,
nested struct), combined across records by sum / xor / count, plus
`store_verify` for invariant B's other half — the digest says the values are
right, `store_verify` says every internal pointer still targets a live record.

The oracle is worth exactly what its calibration is worth, so the test pins two
things and proves each can fail:

**It sees every loss.** Five injuries — a dropped record, a changed scalar, a
shortened text, a truncated vector, a changed nested field — each must move the
numbers. *This caught its own first draft*: the vector injury truncated a record
whose vector was ALREADY one element, so the digest came back identical. That
reads exactly like a blind oracle and was a blind injury. Damage that does not
damage proves nothing and looks like proof.

**It is a function of the data, not the representation.** Five stored
representations of identical records (three insertion orders × two bucket
seeds) must digest the same, since a rebuild changes where everything sits and
anything the digest picks up from layout would read as data loss.

*The controls are the load-bearing part here.* **Pin the seed** — an unseeded
hash draws a random one per process (P253), so unpinned, the five images differ
no matter what else you vary and the control passes while attributing nothing
(observed: all five reps forced identical still gave 5 distinct files). Pinned,
each row differs from the reference by one named lever, a byte-identical repeat
run proves the harness is deterministic, and each lever is asserted to actually
reach the stored bytes. Reverse insertion even changes the file SIZE
(22,944 vs 22,936).

**F9 is NOT a live hazard, and that took establishing.** The step assumed a
rebuild reorders iteration, so the digest was written order-independent. Probing
with a deliberately order-DEPENDENT fold showed that neither insertion order nor
the bucket seed changes the order records come out in: loft iterates a keyed
collection through a **key-sorted snapshot**, not a bucket walk, so a rebuild
cannot reorder it. The order-independent form stays — it costs nothing and keeps
the oracle off that implementation detail — but the *assertion* had to go, since
no available lever can make it fail. An assertion that cannot fail teaches a
later reader that something is checked when it is not.

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

So B1 is no longer "which approach" but **"what does it cost"**. Measured with
`tests/scripts/store_rebuild_b1.loft`, guarded by
`store_rebuild_b1_recovers_the_interior_and_is_idempotent`, digest-checked at
every stage by B0's oracle. **DONE.**

### What it costs — the verdict is cheap and effective

300 → 6,000 surviving records, each grown 10× and dropped back, `store_reclaim`
already applied so the baseline is what arc A leaves:

| keep | post-A file | rebuilt | recovered | rebuild time (`--native`) |
|---|---|---|---|---|
| 300 | 213,016 | 48,760 | **77%** | 175 µs (583 ns/rec) |
| 1,500 | 1,715,136 | 240,896 | **86%** | 908 µs (605 ns/rec) |
| 6,000 | 6,865,056 | 969,120 | **86%** | 3,757 µs (626 ns/rec) |

Linear across a 20× range, ~0.6 µs/record native and ~1.0 µs interpreted, and
the loft-level figure is an **upper bound** — the Rust form at image-composition
time skips the interpreter and the per-record dispatch entirely. Both backends
produce byte-identical sizes.

**The standing warning does not reproduce.** A record-by-record rebuild did not
produce a larger file; it produced one 4–7× smaller. The old
634,264-vs-437,416 number came from a different path and should not be carried
forward as a caution against this one.

**It is IDEMPOTENT** — a second and third rebuild land on the same byte count, at
every vector length tested. That is the property B2's automatic mode rests on: a
compaction that grew the file a little each time would be worse than none.

### The gap B2 has to close, and where it is

A rebuild does not reach the from-scratch ceiling — 1.24× at 300 records, 1.38×
at 6,000 — and the whole gap is the **vector field**:

- shapes with only scalars, or only variable-length text, rebuild to **exactly**
  the fresh size (ratio 1.000);
- with vectors, the rebuilt size is **flat across vector lengths 1–9** (40,200
  bytes at every one) while a from-scratch build scales with them smoothly.

So the copy path claims a quantised block per vector where a fresh build claims
by length. **B2 should claim each destination vector at its LENGTH.** `reserve`
on the source side does not do it — measured, identical bytes with and without,
because it sizes the local being copied FROM, not the block copied INTO.

Arc A composes on top: `store_reclaim` after a rebuild takes a further ~9%
(48,760 → 44,568), because a fresh arena still carries its own 7/3 growth slack.

### The instrument trap, worth keeping

**Iterating a bound collection grows its file.** A keyed collection iterates
through a key-sorted snapshot, and for a bound collection that snapshot is
claimed inside the store — so a digest loop inflates the very file it is
measuring (observed: 40,200 → 86,240 across two extra traversals, which read as
`store_reclaim` making the file BIGGER). Every measurement here stats the file
the instant `store_persist_bind` returns, before anything walks the collection.
It is also a fourth producer of tail, and an argument for `store_reclaim`: a
read-only traversal of a persisted collection permanently grew its file before
arc A existed.

### Prior art — checked, and it changes something

Three findings, and the third is the one the plan did not have:

1. **`OPTIMIZE TABLE` under `innodb_file_per_table` is rebuild-and-swap**, in the
   literal sense assumed here: create a new empty table, copy row by row, land it
   in a fresh `.ibd`. B's approach is the established one.
2. **In-place defragmentation exists as an option** — MariaDB 10.1.1 merged the
   Facebook/Kakao patch (`innodb_defragment`), which moves records to fill pages
   and frees the ones that end up empty. It is opt-in via a system variable and
   works at PAGE granularity. That is the relocate branch this plan ruled out,
   shipped as a switch rather than a default — consistent with ruling it out for
   the default path.
3. **MariaDB 11.2.0 shrinks the InnoDB system tablespace by reclaiming unused
   space AT STARTUP.** This is automatic reclamation, and it is arc A's
   operation — but triggered at neither of the two moments this plan considered.
   See [README.md § Open design questions](README.md) 4.

Also confirming the shared model: InnoDB deletes only mark rows, and the freed
space is never returned to the OS on its own. Same shape as loft.

Sources: [MariaDB OPTIMIZE TABLE](https://mariadb.com/docs/server/ha-and-performance/optimization-and-tuning/optimizing-tables/optimize-table) ·
[Defragmenting InnoDB Tablespaces](https://mariadb.com/docs/server/ha-and-performance/optimization-and-tuning/optimizing-tables/defragmenting-innodb-tablespaces) ·
[MariaDB 10.1.1 defragmentation](https://mariadb.org/defragmenting-unused-space-on-innodb-tablespace/) ·
[Percona: reclaiming space with file-per-table](https://www.percona.com/blog/how-to-reclaim-space-in-innodb-when-innodb_file_per_table-is-on/)

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
