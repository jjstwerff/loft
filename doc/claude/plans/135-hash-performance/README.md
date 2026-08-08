<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 135 — A hash insert that allocates once and never re-hashes

## Status

Open — **arcs A, B and C shipped** (all layout-neutral); **one fork undecided**
(Q1: arc D vs arc H), and it is now the ONLY thing left, because the re-measurement
puts the whole remaining lookup gap inside D / E / H. Those three each cost a
persisted-store format break and must not be taken twice.

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

- **Effort:** MH (A: S · B: S · C: S · D: M · E: S · H: MH)
- **Design:** ~ (partial — Q1 open)
- **Value category:** Q (internal quality — performance with a clear payoff)
- **Last touched:** 2026-08-08

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
| `i8` / `i16` / `integer limit(min, max)` | the record side decodes through `get_short`/`get_byte` with the field's start hardcoded to `0`, while the lookup side reads the raw value — so the two differ by exactly that start and never compare Equal. Ordering survives (the offset is monotonic); only lookup fails. Both backends. | **filed** ([loft#812](https://github.com/loft-lang/loft/issues/812)) — `Key` must carry the field's start, which is a schema surface |

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
| **D** — cache the hash in the bucket slot (`(u32 rec, u32 hash)`) | 171 ms + most per-probe cost | **yes** | Blocked on Q1 |
| **E** — seeded integer hash + division-free bucket index | ~13% combined (measured) | **yes** | Blocked on Q1 |
| **H** — inline contiguous entry array behind `Parts::Hash` | subsumes A/B/D + locality | **yes** | Blocked on Q1 |

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
   rejects without reading the record) and arc H (contiguous entries) are for. The
   remaining fixed ~26 ns is mostly SipHash, which is arc E.

   So the lookup gap is now **measured to live entirely in D / E / H**, all three of
   which cost a persisted-store format break. That makes Q1 the next thing, not an
   optional one.
5. **Q1, then one of D+E or H.** Whichever is chosen also extends the @PLN97 layout
   identity so an old store refuses rather than misreads.

## Open design questions

1. **(blocks D, E, H) Approach `Dictionary` or match it?** D+E keeps per-entry records
   and buys back the resize and hash terms. H restructures `Parts::Hash` onto an inline
   contiguous entry array — C#'s shape, still word-addressed and pointer-free, so still
   mmap-able and range-fetchable — and subsumes A/B/D while fixing the locality that
   none of the others touch. **They conflict:** a bucket-slot change spent on D is
   thrown away by H, and each costs one persisted-store break. Decide before spending it.
2. **How does an old store refuse?** The @PLN97 layout identity covers storage layout
   (field positions, sizes, endianness), not bucket placement — so today a changed hash
   would be *misread*, not rejected. Options: add a bucket-algorithm token to the layout
   dump (rejects only stores that use a hash), or bump `SIGNATURE` "Sto1" → "Sto2"
   (rejects every store, including ones with no hash). Prefer the former.
3. **Does arc E's seeded integer hash preserve the P253 property?** The seed is mixed in
   before a full murmur3 finalizer, so bucket collisions still depend on a seed the
   program never reveals. Distribution was measured equivalent to SipHash. What is NOT
   yet argued: whether xor-then-mix resists a differential attack as well as SipHash's
   keyed construction. Needs a written argument before it ships, not just a histogram.
4. **Should the load factor move?** 0.42 wastes ~2.4x the bucket memory it needs. Raising
   it costs probe length (currently 1.37) and buys cache locality. Only worth touching
   inside D or H, since it is a layout change either way.

## Cross-arc dependencies

- **@PLN97** (layout contract) — Q2 extends the layout identity; D/E/H cannot ship
  without it.
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
