<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 135 — A hash insert that allocates once and never re-hashes

## Status

Open — **arcs A, B and C shipped** (all layout-neutral), **Q2 shipped**, and the only arc
left is **H**. **Q1 is answered H, not D**; **Q2's refusal mechanism is in the tree**;
**Q3 dissolved**.

**H still needs a design cycle before it is built, and the reason is not the byte
layout.** H makes an entry a position inside one contiguous array, so growing that array
moves every entry and invalidates every outstanding reference into it — a stability
callers have today and would lose (**Q5**, new). That is an ownership question in the area
ranked weakness #1, not plumbing. Q5 has a leading candidate — a **chunked arena**, which
keeps every live entry at the address it was given while still dropping the per-record
header that is most of the gap — and a probe that measures it without building H. **Q4**
(load factor) is settled inside H.

**Q1 — answered: H, not D.** See
[§ The Q1 measurement (2026-08-09)](#the-q1-measurement-2026-08-09--it-is-one-access-and-it-is-a-byte-problem).
The record read is **82%** of a 1M random lookup and the bucket read is 8%, so D spends
the one format break available on the small term; H's ceiling (measured against a dense
`vector<Entry>`, which IS its layout) is roughly half of today's cost. **Retire D** unless
H plus the abandoned-table fix still leave a gap D would close.

**Q2 — shipped: a store written before a placement change now REFUSES instead of
misreading.** The @PLN97 layout identity commits to how bytes are shaped, not to where a
keyed collection puts an entry — so before this, a store written pre-H and read post-H
passed the gate and was then misread, with lookups finding nothing or a neighbour and no
error anywhere. `src/placement.rs` carries a token per collection KIND into that same
identity: `placement::tag` → `layout_dump` → `layout_algo_hash` → the `.dschema` sidecar →
`schema_gate_ok` on every `store_load`, with the paged and remote loaders gating on the
same value before range-reading foreign bytes. **Absence means the baseline**, so the dump
is byte-identical, no recorded layout hash moves, and not one store already on disk is
invalidated — which is why it could ship before H rather than with it. Per KIND, so
bumping the hash token leaves plain structs and vectors loading exactly as before.
`sorted` / `ordered` deliberately get no token: their placement is key order, which a
reader re-derives by comparing keys it can already read. Guards:
`placement_contract_is_pinned` (arc D and H move the bucket constants; arc E moves the
`key_hash` digests) and `a_changed_placement_token_refuses_the_store`.

Filed and fixed while pinning that contract: [loft#827](https://github.com/loft-lang/loft/issues/827)
— `key_hash` ran on `std::hash::DefaultHasher`, whose algorithm std does not promise across
releases, while the seed stored in the file makes every reader re-derive buckets from it.
So a toolchain upgrade alone could move placement, with no loft change and no token to
bump. `keys.rs` now owns a byte-identical `SipHasher13`, so `placement::HASH` does not bump
and no store is invalidated.

**The layout-neutral half is DONE (2026-08-09).** The same probes found that growing a
hash **abandoned every previous bucket table** — neither replacement site returned the old
claim, and `hash.rs` never called `Store::delete` at all. Fixed: `install_table` repoints
the field and frees the predecessor in one step. A grown 1M hash went **49.26 MB → 34.02
MB (−31%)**, within 3% of the 33.00 MB the same content costs pre-sized, with **no lookup
regression** (244 ns vs 252 ns min, 1M shuffled — the denser table offsets the longer
probe chains). Guard: `data_structures.rs::hash_growth_frees_the_table_it_replaces` counts
claimed records; `tests/scripts/135-hash-table-rebuild-frees-the-old-table.loft` is the
both-backend correctness half. **So arc H must now be measured against 34 MB, not 49.**

**Also measured:** arc E's lookup win is 1 ns of 36 (step 4), so **Q3 dissolves** — the
hash-DoS argument only ever existed to license E. **Q4 is NOT settled** by these probes.
And **lookup ORDER outweighs every arc**: 87 ns ascending vs 225 ns shuffled at 1M, and
every figure in this plan before 2026-08-09 was taken in insertion order.

Measured on `--native-release`, 1M `integer` keys, before → after A + B + C:

| | before | after | C# |
|---|---|---|---|
| insert | ~933 ms | **~505 ms** (~**350 ms** with `reserve`) | ~28 ms |
| lookup | ~90–101 ms | **~66–84 ms** | ~22 ms |
| bucket table | 10.2 MB | 10.2 MB (**5.3 MB** with `reserve`) | — |

The measurement below is the plan's foundation and was taken on one machine with both
runtimes present — it is not a cross-machine ratio. Re-read it before re-deriving
anything: two of #809's own claims did not survive it.

**Arc A's matrix found three key-width defects that had nothing to do with
performance** — the equivalence cells asked "does a `float` / `u8` / `i16` key behave
the same before and after?" and the answer was that none of them behaved at all. Two
are fixed here, one is filed. See [§ What the matrix found](#what-the-matrix-found).

## Goal

Close the integer-key `hash<T[key]>` gap to `Dictionary<long,long>`, starting with the
77% of it that is not the hash function and needs no store-format change.

## Effort + design

- **Effort:** MH (A: S · B: S · C: S · D: M · E: S · H: MH) — **only H remains**
- **Design:** ~ (Q1 answered 2026-08-09 → H; Q2 shipped 2026-08-09; Q3 dissolved; **Q5 open and blocks H** — entry-reference stability is an ownership question; **Q4 open**, settle it inside H)
- **Value category:** Q (internal quality — performance with a clear payoff)
- **Last touched:** 2026-08-09

## The measurement (2026-08-08)

Ubuntu 24.04 x86_64, 24 cores · loft 2026.8.0 `--native-release` · .NET 9.0.316 ·
1M inserts + 1M lookups, `integer` keys. Corpus:
`Moros-Economy-Development/loft_eval/bench/hash_bench.loft` and a matched
`Dictionary<long,long>` port. The consumer's reported ~980 ms reproduces here.

| | loft | C# | ratio |
|---|---|---|---|
| insert | ~874–954 ms | ~28 ms | **~32x** |
| lookup | ~67–101 ms | ~22 ms | ~4x |
| total | ~980–1010 ms | ~50 ms | ~20x |

**Attribution — measured by timing `hash::add` from inside, not modelled:**

| term | cost | share of the 954 ms insert |
|---|---|---|
| `hash::add` in total (bucket table + SipHash + probing + all 17 resizes) | 215 ms | 23% |
| — of which resize re-hash (1.79M re-inserted entries) | 171 ms | 18% |
| — of which ordinary per-insert hash work | ~44 ms | 5% |
| **everything outside the hash** — scratch record + field writes + `OpCopyRecord` | **~740 ms** | **77%** |

Supporting figures:

- `vector<Entry>` inline append (same records, no hashing): ~100 ms/1M — so the hash
  table is not what makes an `Entry` expensive; the **scratch-plus-copy** is.
- 17 resizes; final load **0.42**; linear probing averages **1.37 probes** for both
  insert and hit. An initial ~8.5-probe estimate was falsified by simulating loft's own
  growth policy — probe length is not a cost here.
- SipHash in isolation: 26.5 ns/op through the `Content` slice; a seeded murmur
  finalizer: 2.36 ns/op. Bucket distribution of the two is equivalent (max bucket and
  empty-slot counts within noise across `elms` 16 … 2,097,150, and across strided key
  families).
- Arc E prototyped end-to-end behind an env flag: `hash::add` 215 → 121 ms, insert
  954 → 840 ms, lookup 88 → 67 ms, combined bench 982–1011 → 849–872 ms (~13%).
  Patch kept out of the tree; it moves bucket placement (see Q1).

**Candidates measured and REJECTED** — recorded so they are not re-proposed:

- *Drop the per-insert `keys.clone()`.* `structures.rs`'s `Parts::Hash` arm clones the
  `Vec<Key>` on every insert to dodge a borrow conflict, which reads like one heap
  allocation per insert. Replacing it with a split field borrow is a three-line change
  and measures **no effect** (insert ~583–600 ms vs a ~540–623 ms baseline — inside the
  noise). A one-element `Vec` clone is not what this path is spending on.
- *Give insert one hash instead of two.* Each insert hashes the same logical key twice
  through two derivations — `dedup_keyed` → `find` → `key_hash` (Content side), then
  `hash::add` → `hash_set` → `keys::hash` (record side). Collapsing them is layout-neutral
  and sound, but it is worth ~4%: per-insert hash work is ~44 ms of the 215 ms, the
  rest being resize. Fold it into arc D — which will already have the hash in hand — and
  do not spend a standalone arc on it.

**What the generated code actually does per `cache += Entry { key: i, val: i*2 }`:**

```rust
let elm = OpNewRecord(cell, var_hb, 78, 65535);   // claim the entry record
ref_1   = OpDatabase(cell, ref_1, 77);            // claim a SECOND scratch record
set_int(ref_1, 0, i); set_int(ref_1, 8, i*2);     // write the fields into the scratch
OpCopyRecord(ref_1, elm, 32845);                  // copy scratch -> entry
OpFinishRecord(cell, var_hb, elm, 78, 65535);     // link into the hash
```

`OpDatabase` is `clear` + `claim` (an LLRB free-space search) + `set_known_type` +
`set_default_value`. C# writes into `_entries[_count++]` — a bump-pointer store into one
contiguous array, no allocator involved, and its resize is an `Array.Copy` plus a
sequential re-bucket off the **cached** `hashCode`, so it never re-hashes a key.

**Two #809 claims the measurement contradicts** — recorded so they are not re-derived:

1. #809 frames this as a lookup problem. Lookup is 4x; **insert is 32x** and ~90% of the
   wall clock.
2. `hash.rs`'s comment says load factor 0.75. The *trigger* is 0.75; `room*2-1` then
   overshoots to 0.42. The tradeoff already spent is memory, not speed.

## Composition matrix — Stage A

Arcs A / B / D / E / H are **behaviour-preserving**, so the matrix is an *equivalence*
matrix, not a feature matrix: every cell asserts identical results before and after, on
both backends, plus no leak. Arc C adds a surface (a capacity hint) and needs a real
feature matrix.

Axes that must move together — the composition the fix touches:

| axis | cells |
|---|---|
| key type | `integer`, narrow (`u8` / `i32` / `i16`), `text`, `float`, multi-key |
| entry shape | scalar-only fields, a `text` field, a `reference` field, a nested collection |
| value expression | fresh struct literal (the retarget case), a variable, a function call, a copy of an existing record |
| collection op | `+=` insert, `[k]` hit, `[k]` miss, remove-then-reinsert, iterate |
| size | below first resize (< 12), across ≥ 2 resizes, ≥ 1M |
| persistence | in-memory, persisted then re-read **in a second process** |
| backend | `--interpret`, `--native` |

The persistence row is the one that catches D / E / H: a store written by the old
binary and read by the new one must **refuse loudly**, never miss silently. Probes go in
`probes/`, graduate to `tests/scripts/` per arc as each arc lands.

Arc A's matrix is `probes/a-retarget-matrix.loft`, graduated to
`tests/scripts/135-keyed-insert-retarget.loft`. Arc C's feature matrix (it adds a
surface, so it needs one) is `tests/scripts/135-reserve-hash.loft`: every cell fills a
reserved collection and its unreserved twin and demands they agree on length, on every
record, and on every miss. Arc B needs no matrix of its own — it changes how a
comparison is reached, not what it answers, and every keyed test in the suite is its
equivalence check.

## What the matrix found

The **key type** row of the matrix — `integer`, narrow, `text`, `float`, multi-key —
was written to prove the retarget changed nothing. It found instead that three of
those key widths had never worked, silently, on shipped loft. All three predate this
plan (verified against a pristine `HEAD` worktree, and on insert paths arc A does not
touch), and all three lose records without a diagnostic:

| key width | symptom | status |
|---|---|---|
| `float` / `single` | `keys::get_key` had no arm, so it read ONE BYTE and called it a `Content::Long`. Every key whose low byte is zero — 0.5, 1.5, 2.0, … — became the SAME key: a `sorted<T[k]>` collapsed to its last insert, a `hash<T[k]>` probed a bucket `hash_ref` never fills. Both backends. | **fixed here** |
| `u8` / `u16` / `i32` / `u32` | the native generator's `emit_content` had no arm, so every `--native` lookup searched for `Content::Long(0)`: a `hash` missed present records, a `sorted` answered a reference whose fields all read null. Interpreter correct. | **fixed here** ([loft#811](https://github.com/loft-lang/loft/issues/811)) |
| `i8` / `i16` / `integer limit(min, max)` | the record side decodes through `get_short`/`get_byte` with the field's start hardcoded to `0`, while the lookup side reads the raw value — so the two differ by exactly that start and never compare Equal. Both backends. | **fixed** ([loft#812](https://github.com/loft-lang/loft/issues/812)) — `Key` carries `start`, derived in `determine_keys_for` from the one place that already knew it. The scope was wider than filed: `i16` is `Parts::ShortRaw`, which was read as a sign-extended `i16`, so ordering did NOT survive it (a `sorted` came out `0,1,2,-3,-2,-1` on `--native`) and a `u16` key ≥ 32768 was equally unfindable |

The regression guard for the two fixes is `tests/scripts/811-collection-key-widths.loft`.

The lesson worth keeping: an **equivalence** matrix is not a weaker instrument than a
feature matrix. Asking "same before and after?" across the composition axes exercised
key widths no performance question would have reached, and the cells that had never
worked answered "same" — identically broken on both sides — only because the cell
asserted a VALUE and a LENGTH rather than agreement between two runs.

## Sub-arcs

| Item | Worth | Format break | Status |
|---|---|---|---|
| **A** — build the entry in place (retarget the struct literal at the entry record; drop the scratch + `OpCopyRecord`) | measured **933 → 536 ms** on 1M inserts (−43%) | no | **Done** |
| **B** — hoist the key dispatch out of the probe loop (no new opcode; see below) | measured **~10 ns of a ~33 ns** cache-resident lookup (−20…25%) | no | **Done** |
| **C** — capacity hint: `reserve(h, n)` pre-sizes the bucket table | measured **618 → 352 ms** on 1M inserts, and half the table memory | no (additive) | **Done** |
| **Q2** — placement token in the @PLN97 layout identity, so an old store refuses instead of misreading | nothing on its own; it is what lets H spend a format break safely | no (absence = baseline) | **Done** |
| **D** — cache the hash in the bucket slot (`(u32 rec, u32 hash)`) | 171 ms + most per-probe cost | **yes** | **Retired by Q1** — attacks the 8% term and adds bytes to the 82% one |
| **E** — seeded integer hash + division-free bucket index | ~13% combined (measured); **insert-only — the lookup half is 1 ns of 36**, and arc D removes most of the insert half too | **yes** | **Retired by Q1** — revive only with a written P253 argument (Q3) |
| **H** — inline contiguous entry array behind `Parts::Hash` | subsumes A/B/D + locality; ceiling ~2x on random lookup | **yes** | **The only arc left, and it needs a design cycle** — Q5 (entry-reference stability, an OWNERSHIP question) blocks it; settle Q4 inside it; measure against 34 MB, not 49 |

## Phase ordering

1. **A — done.** The retarget turned out to be one branch, not a new mechanism: the
   BRACKETED spelling `h += [Entry { … }]` already built the entry in place (@P277
   intercepts before the RHS parse and hands it the element var as its target), so the
   working bytecode existed as a real runnable source shape and only the BARE
   `h += Entry { … }` was missing it. The bare form reached `parse_object` with the
   COLLECTION local as its target; `parse_object` rejects a target that is not
   `Reference(<this struct>)` and allocates a throwaway work-ref instead, which is why
   the P188 branch's intended `substitute_value(Var(collection) → Var(elm))` retarget
   never fired. `parser/expressions.rs::parse_assign_op` now takes the same pre-parse
   branch for a bare literal, gated on peeking `<element-name> {`. A variable, call or
   field-read RHS keeps the deep copy, as does a qualified (`pkg::Entry {`) or generic
   (`Pair<integer> {`) spelling — slower, never wrong.
2. **B — done, but NOT as a new opcode.** The plan proposed `OpGetRecordLongKey`; the
   measurement says the win does not need one. What costs is that `hash::find` re-runs
   `key_compare`'s `(Content, type_nr)` match *for every record probed*, while a probe
   only ever asks *is this the key* about the *same* key. `keys::fast_key` resolves the
   field offset and the value once, before the loop, and `FastKey::matches` reads the
   field directly. So it is a runtime change in `hash.rs` + `keys.rs`: no opcode, no
   bytecode surface, no parser change — and **both backends get it**, since `hash::find`
   is shared. It also pays on INSERT, because dedup runs one `find` per insert.

   The estimate that argued *against* building it was wrong, and cheaply so: the
   arithmetic said "≈5 ns of 97, not worth it", the 25-line env-gated probe said 10 ns
   of 33 at cache-resident sizes. Build the probe.
3. **C — done.** `reserve(h, n)`, extending the vector builtin (loft#710) to a hash's
   bucket table. Additive, contract-safe, and it buys memory as well as time: the
   growth ladder doubles *past* its trigger and lands at load 0.42, while a reserved
   table sits at the 0.75 it asked for.
4. **Re-measured — and the plan's expectation for this step did not survive.** A + B + C
   put insert where it predicted (~350 ms reserved, ~505 ms not), but **lookup is not
   near C# and cannot be got there from here**:

   | table size | ns / lookup |
   |---|---|
   | 1,000 | 30 |
   | 10,000 | 26 |
   | 100,000 | 33 |
   | 1,000,000 | **83** |

   The ~50 ns a 1M table adds over a cache-resident one is **cache misses** — the bucket
   slot and then the record, two random accesses over ~24 MB. No layout-neutral arc
   touches that: it is exactly what arc D (cache the hash in the bucket slot, so a probe
   rejects without reading the record) and arc H (contiguous entries) are for.

   **The fixed ~26 ns is NOT mostly SipHash — measured in situ, and this line used to say
   it was.** Swapping SipHash for a seeded 2-multiply splitmix at BOTH hash sites moves a
   cache-resident lookup by **1 ns of 36** (36 → 35, reproduced exactly across runs), and
   nothing resolvable at 1M. The 26.5 ns/op figure above is `key_hash` timed *in
   isolation*; it does not survive being measured where it actually runs.

   The instrument was falsified before the number was believed: with the same probe made
   degenerate (every key to bucket 0) the same benchmark goes 37 ns → 16,244 ns, so the
   fast path is unquestionably live. Do not re-derive this from the isolation figure — an
   isolated hash microbenchmark overstates this path by more than an order of magnitude.

   **Consequence for Q1:** arc E is an INSERT optimisation, not a lookup one. Its measured
   value is ~24% of insert, and ~80% of *that* is the resize re-hash — which arc D removes
   anyway by caching the hash. So D largely subsumes E's real win, while E alone carries
   the P253 security question (Q3). Rank accordingly: E is not the cheap lookup win the
   ordering above implied, and shipping D may leave E not worth its format break at all.

   So the lookup gap is now **measured to live entirely in D / E / H**, all three of
   which cost a persisted-store format break. That makes Q1 the next thing, not an
   optional one.
5. **Q1 and Q2 — both done (2026-08-09).** Q1 chose H over D+E. Q2 extended the @PLN97
   layout identity with a per-kind `placement` token, so an old store refuses rather than
   misreads; it shipped ahead of H because absence means the baseline, which invalidates
   nothing already on disk.
6. **H — next, and it needs a design cycle.** Q5 (entry-reference stability) blocks it:
   it is an ownership question, not a layout one. Settle Q4 (load factor) inside it — a
   load-factor change is a layout change either way, so it costs nothing extra there and
   cannot be spent separately.

## The Q1 measurement (2026-08-09) — it is ONE access, and it is a byte problem

Step 4 left Q1 resting on "two random accesses — the bucket slot and then the record".
That split had never been measured, and it is what chooses between D and H. Measured, it
is not a split: **the record read is 82% of a 1M random lookup and the bucket read is
8%.** Probes: `probes/h-locality.loft` (the ablation), `probes/h-random-access-floor.loft`
(what the same payload costs read densely), `probes/q1-store-footprint.loft` (bytes per
entry, read off a persisted image). Ablation patch: `probes/q1-ablation.patch`.

Machine: AMD Ryzen AI 9 HX 370, 24 threads, 61 GB · `--native-release` · min of 5.
**Not the box the 2026-08-08 table was taken on** — read the deltas, not the absolutes.

### Where a lookup's time goes

`LOFT_PROBE_HASH` drops one memory access at a time (`skiprec` = no record read,
`floor` = bucket 0 and no record read, so neither can miss). Subtracting gives the split.
The rows below are the `nofield` shape — the loft program tests the `DbRef` and never
reads a field, so the only record access left is the one inside `find`.

| n, order | total | fixed | bucket read | **record read** |
|---|---|---|---|---|
| 10k ascending | 35 | 22 | 10 | 3 |
| 10k shuffled | 37 | 22 | 11 | 4 |
| 100k shuffled | 61 | 22 | 12 | 27 |
| 1M ascending | 87 | 23 | 18 | 46 |
| **1M shuffled** | **225** | 22 | 18 | **185 (82%)** |

The `field` variant's subtraction is NOT valid — its program-level `e.val` survives
`skiprec`, so `off − skiprec` there measures a second, already-cached read. Use `nofield`.

**Lookup ORDER is the axis that was missing, and it is worth more than every arc.**
Keys go in as 0..n and records are claimed in that order, so looking them up as 0..n
walks memory ascending and prefetches. Every figure in this plan before today was taken
that way. A real consumer does not: the icosahedron edge-midpoint cache behind #809 looks
up in mesh order. 1M ascending 87 ns, 1M shuffled 225 ns. (The first cut of the probe
computed the permutation INSIDE the timed loop and charged its 64-bit modulo — ~35 ns —
to the shuffled row; both orders now walk a precomputed vector, so what is left between
them is the access pattern alone.)

### What arc H can buy, measured without building it

A `vector<Entry>` IS arc H's layout — elements inline, insertion-ordered, word-addressed
— so the same 1M elements read in the same shuffled order out of a dense vector is H's
ceiling. (`e = v[k]` emits `OpGetVectorNullable`, a cursor: no copy, confirmed with
`LOFT_REPORT_COPIES`.)

| 1M, shuffled | ns |
|---|---|
| `vector<integer>` (8 MB) | 26 |
| **`vector<Entry>` (16 MB) — H's ceiling** | **93** |
| `hash<Entry[key]>` | 204 |

So H's ceiling is roughly half of today, and the floor it stops at is the machine: a
dense 16 MB array still costs 93 ns to read randomly, four times C#'s reported 22 ns for
the whole lookup. Whatever remains after H is not addressable by layout.

### Why: a hash spends 2–3x the bytes its payload needs

`store_persist_bind` writes the collection's store, so the file is the store image —
every claimed block, reachable or not. That makes bytes-per-entry an exact, external
measurement rather than an RSS guess (peak RSS cannot tell a live table from the second
copy a rehash transiently holds).

| 1M entries of `{integer, integer}` | store | B/entry | of which buckets |
|---|---|---|---|
| `vector<Entry>` (dense) | 16.00 MB | 16 | — |
| `hash`, `reserve`d | 33.00 MB | 33 | 5.33 MB |
| `hash`, grown — **before** the table-free fix | 49.26 MB | 49 | 10.24 MB |
| `hash`, grown — **after** | 34.02 MB | 34 | 6.24 MB |

Run one configuration per process. An earlier cut of `q1-store-footprint.loft` ran every
shape from one `main` and its grown row was WRONG — a collection's store SLOT is reused
between iterations, so the second fill inherited the first's buffer and growth ladder. It
read 49.26 MB both before and after the fix while a single-shot run read 49.26 → 34.02.
The harness was measuring itself; the probe now takes its shape and size as arguments.

That is the mechanism behind the 185 ns. It is a working-set problem, and **arc D does
not shrink the working set** — it adds 4 bytes per bucket slot. H is the only arc that
touches the term that carries the cost.

### Found while measuring: growing a hash ABANDONS every previous bucket table

`add`'s growth path and `reserve` both `claim` a new table, `rehash_into` it, and
repoint the field — and neither returns the old claim. `hash.rs` never calls
`Store::delete` (`radix_tree.rs` does, so the convention exists; this is an omission).
The blocks stay CLAIMED and unreachable, which is why the grown row above costs 16.26 MB
more than the identical content pre-sized while its bucket table is only 4.9 MB bigger.

Two independent confirmations: the code has no `delete`, and `store_reclaim` on a
freshly grown, never-bound 1M hash recovers **0 bytes** — the space is not free, it is
claimed. A persisted grown hash carries its dead tables to disk forever.

**FIXED (2026-08-09).** `install_table` repoints the hash's field and frees the
predecessor as one step — the order is load-bearing, because `Store::delete` repurposes
the block's body as a free-tree node, so a free before the repoint would leave the field
naming bytes that are already something else. Both replacement sites route through it.

Measured: a grown 1M hash 49.26 MB → **34.02 MB**, and the freed space is reused rather
than returned, so the final table also lands smaller (10.24 → 6.24 MB) and the load factor
rises 0.39 → 0.64. That last part is a real behaviour change and was checked rather than
assumed: 1M shuffled lookup 244 ns after vs 252 ns before (min of 3) — the denser table
pays for the longer probe chains. `reserve`d fills are byte-identical before and after,
which is the control: `reserve` on an EMPTY hash has no predecessor, so the fix must not
move it.

The other half of the question — freeing a block someone still reaches is worse than
leaking it — is what the guards are for. Iteration snapshots record numbers up front
(`build_hash_sorted_vec`), so `for x in h { h += … }` rebuilds the table under a running
loop; `check_iter_safety` does not list `Type::Hash`, so that is reachable user code and
it is a cell in the script test.

### What this settles

- **Q1 → H, and D is not worth a format break.** D attacks 8% and adds bytes to the term
  that carries 82%. H's measured ceiling is ~2x on random lookup and it subsumes A/B/D.
- **Q3 dissolves.** It only existed to license arc E, which step 4 already measured at
  1 ns of 36 on lookup; nothing here revives it. No security argument needs writing.
- **Q4 is not resolvable from this data.** Reserved-vs-grown at 1M shuffled came out 225
  vs 200 on the instrumented binary and 213 vs 226 on the clean one — both orderings
  observed, so the load-factor question stays open and needs its own probe.
- **Order belongs in every future measurement.** A benchmark that looks up in insertion
  order understates this path by ~2.5x, and every figure in this plan predating today
  was taken that way.

## Open design questions

1. **(blocks D, E, H) Approach `Dictionary` or match it?** — **measured 2026-08-09; the
   answer is H.** See § The Q1 measurement above: the record read is 82% of a random 1M
   lookup, D does not shrink it, and H's ceiling is roughly half of today's cost. Retire
   D unless the abandoned-table fix plus H still leave a gap D would close.
   Original framing follows. D+E keeps per-entry records
   and buys back the resize and hash terms. H restructures `Parts::Hash` onto an inline
   contiguous entry array — C#'s shape, still word-addressed and pointer-free, so still
   mmap-able and range-fetchable — and subsumes A/B/D while fixing the locality that
   none of the others touch. **They conflict:** a bucket-slot change spent on D is
   thrown away by H, and each costs one persisted-store break. Decide before spending it.
2. **~~How does an old store refuse?~~ — SHIPPED (2026-08-09).** Resolved as the first of
   the two options below: a per-kind `placement` token in the layout dump, not a
   `SIGNATURE` bump, which would have refused every store ever written including ones
   with no hash. See § Status for the mechanism and its guards. Original framing: The
   @PLN97 layout identity covers storage layout
   (field positions, sizes, endianness), not bucket placement — so today a changed hash
   would be *misread*, not rejected. Options: add a bucket-algorithm token to the layout
   dump (rejects only stores that use a hash), or bump `SIGNATURE` "Sto1" → "Sto2"
   (rejects every store, including ones with no hash). Prefer the former.
3. **~~Does arc E's seeded integer hash preserve the P253 property?~~ — DISSOLVED
   (2026-08-09).** The question exists only to license arc E, whose lookup win is 1 ns of
   36 (step 4) and whose insert win arc D/H takes anyway. Nothing needs to be argued
   unless E is revived. Original framing: The seed is mixed in
   before a full murmur3 finalizer, so bucket collisions still depend on a seed the
   program never reveals. Distribution was measured equivalent to SipHash. What is NOT
   yet argued: whether xor-then-mix resists a differential attack as well as SipHash's
   keyed construction. Needs a written argument before it ships, not just a histogram.
4. **Should the load factor move? — still open, and the 2026-08-09 probe did NOT settle
   it** (reserved-vs-grown flipped between the instrumented and clean binaries; needs a
   probe that varies load factor directly rather than via `reserve`). 0.42 wastes ~2.4x the bucket memory it needs. Raising
   it costs probe length (currently 1.37) and buys cache locality. Only worth touching
   inside D or H, since it is a layout change either way.
5. **(blocks H) Do entry references survive growth?** — **new, 2026-08-09.** Today they
   do, and H removes that. Growth reallocates only the BUCKET table: `rehash_into` moves
   `u32` record numbers between tables and never touches an entry's bytes, so an entry
   keeps its own claim and its `DbRef{rec, pos: 8}` stays valid for as long as the entry
   lives. Verified: `e = h[1]` read back correctly after ~2000 further inserts (many
   doublings).

   H makes an entry a POSITION inside one contiguous array — `DbRef{rec: <array claim>,
   pos: <slot offset>}`, the shape a vector element already uses — so growing the array
   moves every entry, and every outstanding reference into it goes stale. `e = h[k];
   h += [other]; e.val` then reads moved-or-freed bytes. That is the vector-reallocation
   hazard, which the deps/lifetime system already governs for vectors, arriving somewhere
   it has never applied.

   So H is not only a format break, it is an OWNERSHIP change, and it lands in the area
   [`OWNERSHIP_MODEL.md`](../../OWNERSHIP_MODEL.md) and CLAUDE.md rank as weakness #1.
   **This question, not the byte layout, is the expensive half of H.**

   **Leading candidate — a CHUNKED arena, not one array.** Making the entry reference
   borrow (growth invalidates it, as a vector's does) is the obvious answer and the wrong
   one: it silently breaks programs that work today, which
   [`COMPATIBILITY.md`](../../COMPATIBILITY.md) forbids, and it breaks them into a *wrong
   read* rather than an error. But the dilemma is false. H needs entries DENSE; it does
   not need them in ONE allocation. An arena that grows by appending a new chunk — never
   reallocating a chunk that already holds entries — keeps every live entry at the address
   it was given, so a `DbRef{rec: <chunk claim>, pos: <offset>}` stays valid for the
   entry's whole life, exactly as today's per-entry claim does. The bucket table keeps
   doubling and rehashing; bucket slots are indices and cost nothing to move.

   That buys most of what H is for. The measured gap is ~27.7 B/entry against a dense
   vector's 16 (33 total − 5.33 of buckets), and the difference is per-record header — the
   thing chunking removes. What it gives up is cross-chunk contiguity, so the chunk size
   sets how close H gets to the 93 ns `vector<Entry>` ceiling. **Probe before building:**
   a `vector<Entry>` read in shuffled order, chunked at several sizes, measures the whole
   candidate without writing any of H (the same trick § What arc H can buy already used —
   the layout it proposes IS a shape the language can already build).

## Cross-arc dependencies

- **@PLN97** (layout contract) — Q2 extended the layout identity with a per-kind
  `placement` token (`src/placement.rs`, shipped). H cannot ship without it; it is now
  there, so H is free to move the bucket constants and bump `placement::HASH`.
- **loft#808** (shipped) — arc A is the same defect class at a third boundary. Compound
  values materialise into heap records at the local (free for tuples), at the **return**
  (#808, fixed), and at the **collection insert** (arc A). Reuse its technique.
- **@PLN102 arc A** (`COMPATIBILITY.md`) — `CONTRACT_VERSION` is 0, so a format break is
  permitted now; this is the pre-freeze window the doctrine names for exactly this.

## See also

- [`loft-lang/plans#135`](https://github.com/loft-lang/plans/issues/135) — `@PLN135`, the tracker issue.
- [loft#809](https://github.com/loft-lang/loft/issues/809) — the source report.
- [loft#808](https://github.com/loft-lang/loft/issues/808) — the shipped sibling; arc A's technique.
- [`DATABASE.md`](../../DATABASE.md) — stores / `DbRef` / `Parts`.
- [`COMPATIBILITY.md`](../../COMPATIBILITY.md) — the store-layout surface and the contract-0 window.
- [`PERFORMANCE.md`](../../PERFORMANCE.md) — benchmark homes.
- `Moros-Economy-Development/loft_eval/` — the consumer corpus (`bench/hash_bench.loft`,
  `probes/csharp-baseline/Program.cs`, `bytecode-comparisons/809-integer-key-hash.*`).
