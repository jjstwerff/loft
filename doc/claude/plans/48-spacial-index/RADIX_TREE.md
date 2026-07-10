# @PLN48 · Deliverable R — the radix tree

**Status:** landed — `src/radix_tree.rs`, steps R0–R8c green (15), clippy clean.
Allocation-free, and no input reaches a panic.
**Scope:** `src/radix_tree.rs` — a store-backed binary PATRICIA tree, standalone and
unit-tested in Rust, with *no* dependency on the database type layer.
**Consumers:** deliverable S2 (`spacial<T[x,y]>`), and any later `radix<T[key]>`.

Deliverable R is separated from the rest of @PLN48 precisely so the tree can be
tested against a plain `Store` and a hand-written key oracle, with no schema, no
parser, and no `Parts::Spacial` in the picture.  Everything downstream inherits a
structure that is already proven.

---

## 0. The sketch is not a foundation — it is falsified

The @PLN48 README claims *"So this is **completion**, not greenfield."*  That claim
is the most load-bearing one in the plan, and it is **false**.  Two probes against
the 245-line `src/radix_tree.rs`, run before any redesign:

| Probe | Claim under test | Result |
|---|---|---|
| `set_bits`/`get_bits` round-trip of a non-zero skip count | path compression stores anything at all | **stores `0`.** `set_byte(rec, fld, min, val)` is called as `set_byte(bits, 3+node, nr, 0)` — the skip count is passed as `min` and the value written is `0`.  For `nr > 0` the guard `val >= min` fails and *nothing is written*. |
| insert a **second** record, then `rtree_first` | the tree holds two elements | **SIGSEGV.**  `rtree_insert` at `LEN == 1` calls `rtree_find`, which evaluates `get_bits(store, bits, (-node) as u32)` while `node` is still a *positive record id* → byte offset `3 + 4294967000ish`. |

A structure whose second insert corrupts memory has no behaviour to preserve.
Reading the rest of the file confirms the shape: `RadixIter::next` returns `None`
unconditionally (so the bidirectional walk — the entire reason @PLN48 wants this
tree — does not exist), `remove()` takes no arguments and has an empty body,
`rtree_optimize` is a no-op, and `rtree_validate` counts with the broken `next()`
so it can never have passed for a non-empty tree.  There are zero tests, and the
module is `#![allow(dead_code)]` with no callers.

**Conclusion.** Deliverable R is a **rewrite against a written invariant**, not a
completion.  The file's *interface intent* (a bit-at-a-time key oracle, so an
interleaved Morton key never has to be materialised) is sound and is kept.  Its
*representation* is replaced.

---

## 1. The invariant

> **I1 (PATRICIA descent).**  Every internal node `n` stores an **absolute** bit
> index `bit(n)`.  Along any root→leaf path `bit` is **strictly increasing**.  Every
> record in `n`'s FALSE child subtree has `key(bit(n)) = 0`; every record in its TRUE
> subtree has `key(bit(n)) = 1`.

Everything else is a corollary of I1, which is why it is the only rule an
untested case has to obey:

- **find** — descending on `key(bit(n))` reaches the unique leaf whose key shares
  the longest prefix with the search key.  Bits *skipped* by path compression are
  never checked, so the leaf is a **candidate**, not a match; the caller compares
  full keys.  This is the classic PATRICIA property and it is what makes insert
  correct.
- **insert** — the first bit `d` at which the search key differs from that
  candidate leaf is the first bit at which it differs from *every* key in the
  tree.  So the split point is the unique edge on the path with
  `bit(parent) < d < bit(child)`.
- **order** — in-order traversal (FALSE subtree, then TRUE subtree) emits records
  in strictly increasing key order.  For a Morton key that *is* Z-order, which is
  the property deliverable S3's proximity walk stands on.

`bit(n) == d` can never occur on the descent path.  Proof from I1: descent follows
the search key's bits, so a node testing bit `d` would have sent the search key and
the candidate leaf to the same side, i.e. they agree at `d`, contradicting `d` =
first differing bit.  This is a `debug_assert`, not a branch.

### 1.1 The key the tree keys on — why there are no preconditions

An earlier revision made prefix-freeness and distinctness **caller obligations** (P2,
P3), enforced by `debug_assert!`.  That was wrong twice over.

First, the enforcement did not exist: this crate builds with
`[profile.dev.package.loft] debug-assertions = false`, so every `debug_assert!` is
compiled out *even under `cargo test`* (probed directly; it is also why the old sketch
SIGSEGV'd instead of tripping `Store::addr`'s bounds checks).  In release a duplicate
key made `rtree_insert` return with `LEN` unincremented — the record was **silently
dropped** — and `rtree_seek` read the same `None` as "exact match" and answered the
wrong record.  One `None` was overloaded across three unrelated outcomes.

Second, and more to the point: a failure mode you have to *react* to is a failure mode
you should *delete*.  The tree now composes the key itself.  Conceptually it keys on
the **infinite bit string**

```text
    user bits ‖ 0x00 terminator ‖ 32-bit record id ‖ zeros forever
```

so the obligations become theorems:

- **No key ever ends** ⇒ a comparison never decides what "one key stopped" means.
  Prefix-freeness is a consequence, not a precondition.
- **Distinct records always differ** (their ids do) ⇒ there is always a bit to split
  on; `rtree_insert` cannot fail.  Re-inserting the *same* record is a no-op.
- **Order is lexicographic.**  The terminator buys this, not the prefix-freeness:
  without it `"ab" ‖ id` versus `"abc" ‖ id` compares a record id against `'c'`, so
  which sorts first would depend on allocation.  `0x00` is below every UTF-8 byte.
- **A probe carries id `0`**, so it sorts to the head of its own bucket:
  `seek("ab")` lands on the first key with prefix `"ab"`.  Prefix queries need no
  separate entry point — this closes a gap an earlier revision had to list as future
  work.
- **Fixed-width keys pay nothing.**  All Morton codes are the same length, so no two
  records diverge inside the terminator and path compression creates no node there.
  One rule, no modes.

Two records may share a user key — several entities in one cell.  They differ only in
the id suffix, so they land **adjacent**: @PLN48's per-code bucket is a contiguous
run, and no bucket structure exists.

The caller now supplies only a `KeySpec { bit, bits }`.  `MAX_KEY_BITS`, the `Option`,
and both panic paths are gone.

**The single remaining assumption:** no key byte is `0x00` — true for UTF-8 text,
vacuous for fixed-width numeric keys.  Violated, the ids still differ, so the tree
stays a structurally valid PATRICIA and only lexicographic order degrades for that
pair.  It degrades; it does not corrupt, and it does not panic.

---

## 2. Why absolute bit indices — the re-assertion count

The quantity every operation needs is *"which bit does this node test?"*.  Two
representations, and the choice is the whole design:

**(a) relative skip counts** (what the sketch stores, one byte per node in a side
vector).  Every site must *accumulate* while descending:
`bit += 1 + skip(child)`.  The sites are `find`, `insert`-descend,
`insert`-split-search, `iter.next`, `iter.prev`, `remove`, `validate` — **7
independent accumulations, and dropping the `+1` at any one of them is silently
wrong.**  The sketch already drops it, at `rtree_find` (`bit += get_bits(..)`).

**(b) absolute bit index per node.**  Nobody accumulates.  Every site reads
`bit(n)`.  **N = 0.**

We take (b).  `N × silence` goes to zero *by construction* rather than by care.  The
cost is 4 bytes per node instead of 1; the gain is that an entire bug class cannot
be written.  The side `bits` vector disappears with it — one fewer `claim`, one
fewer pointer chase per node (which is the cache-locality lever @PLN48 wants), and
one fewer block to leak.

This is the design's central subtraction, and it is the reason the rewrite is
expected to come out **shorter** than a corrected version of the sketch.

### 2.1 So how *are* bits skipped?  They are derived, never stored

Nothing records a skip, because nothing needs to:

- **The run length is the gap.**  A node skips exactly `bit(n) - bit(parent) - 1`
  bits.  Both numbers are already there.
- **The skipped values live in the subtree.**  Every record under `n` agrees on
  every bit below `bit(n)`, so if the values are ever wanted, read them off *any*
  leaf below `n`.

That second point is the **licence to skip**, and it is a claim, so it is asserted
rather than assumed:

> **I2** — all records under node `n` agree on every bit `b < bit(n)`.

I1 and I2 are independent.  A tree can satisfy I1 (increasing bits), give every leaf
a key that matches each *tested* bit on its path, and still be wrong — put two keys
that first diverge at bit 0 under a root that tests bit 1, and every I1-shaped check
passes while `seek` quietly returns the wrong neighbour.  I2 is the only thing that
sees it.

Descent never *checks* a skipped bit — which is precisely why `find` returns a
**candidate** and `insert` compares full keys once, at the end, against that concrete
record.  "Skipping" is not an operation the tree performs; it is the absence of a
node at bits where nothing diverges.

`rtree_validate` checks I2 by comparing one representative leaf from each child
subtree across the skipped range.  That is sufficient by induction: the recursive
call establishes that every leaf under a child agrees with its representative on all
bits below the child's bit, which strictly exceeds this node's.

### 2.2 Where bit-at-a-time actually costs

One place, and it is not the descent: `first_diff_bit` scans a bit at a time from
`0`, so an insert costs `O(key_bits)` oracle calls — 96 for a 2D Morton key, but
unbounded for text.  If S4's measurement points here, the fix is to widen the oracle
with a chunk accessor (`key_word(rec, i) -> Option<u64>`) and find the first
difference with `XOR` + `leading_zeros`, turning the scan into `O(key_words)`.  That
is a **compare**-side optimisation and it does not touch the representation: the
absolute bit index stays, and no skip count comes back.

---

## 3. Attacking the cleanest claims

Two claims in this design read as elegant.  Elegance is the failure mode that
presents as success, so each gets a probe rather than applause.

**Claim A — *"one tree serves integer keys, text keys, and Morton keys."***
Probe: text keys are variable-length, so `"ab"` is a proper prefix of `"abc"` and
**P2 fails natively**.  The unification survives *only* under the virtual-`NUL`
construction, and that construction is legal *only* because loft text is UTF-8 and
cannot contain `0x00`.  So the claim holds **with a named precondition**, not for
free.  Recorded consequence: if loft ever admits binary blobs as radix keys, P2
breaks and this family genuinely splits.  The claim survived; it did not survive
unconditionally.

**Claim B — *"duplicates come free via a record-id suffix, and the per-code bucket
@PLN48 asks for falls out as key-order adjacency."***
Probe: records sharing a Morton code differ only in the id suffix, so they are
contiguous in key order — a "bucket" is a contiguous range, and no bucket structure
is needed.  That part holds.  But the probe also shows the cost: the longest path is
now `W + 32` bits, and for a 2D Morton code `W = 64` ⇒ **96**.

> **The existing `MAX_DEPTH = 64` and the `[i32; MAX_DEPTH]` iterator stack are a
> silent-truncation hazard.**

Found by attacking my own clean claim, not by reading.  Cure by subtraction: the
iterator's path is a `Vec`, the fixed array and `MAX_DEPTH` are **deleted**.  A path
is bounded by the key length in bits (bit indices strictly increase), which for text
keys is not bounded by any constant — so no constant is the right constant.

---

## 4. Failure paths

Enumerated *before* the code, because this is where the invariant became nameable.
Each maps to a design element; none is left to care.

| # | Failure | Design answer |
|---|---|---|
| 1 | duplicate keys ⇒ no differing bit ⇒ undefined split | P3 + loud `debug_assert` in `first_diff_bit` |
| 2 | one key a proper prefix of another ⇒ must branch on "ended" | P2 + virtual `NUL` for text |
| 3 | descent drops the branch bit while accumulating | absolute `bit(n)` ⇒ unwritable (§2) |
| 4 | path deeper than the iterator stack | `Vec` path ⇒ unwritable (§3) |
| 5 | node id vs record id confused in a child slot | one `Child` accessor, §5 — the *only* place the sign encoding is read or written |
| 6 | node array full ⇒ `resize` **relocates the record** | tree rec-id is returned; host field is repointed, exactly as `hash::add` does (`src/hash.rs:53`) |
| 7 | removal leaves a one-child internal node | splice: parent's slot takes the sibling; node returns to the free list |
| 8 | teardown leaks the node block | `rtree_free`; later, the `for_each_owned_child` `container_rec` slot (`src/database/allocation.rs:95`) |
| 9 | `copy_claims` byte-copies the node block | rebuild by re-insert — the destination's `claim` may over-size, same reason `copy_claims_hash_body` re-inserts (`src/database/allocation.rs:1572`) |
| 10 | predecessor/successor of a key **not in the tree**: the candidate leaf is not necessarily the neighbour | re-descend from the divergence node, §6 — the subtlety @PLN48's *"walk predecessor + successor"* glosses over |

---

## 4.1 The cursor allocates nothing

A `next`/`prev` step has to **climb**.  Storing the descent path to climb it means a
heap `Vec` per `seek` — a malloc inside @PLN48's per-frame proximity query, since
entities move and every frame is a remove-plus-insert.  There are exactly two ways to
climb without a stack:

- **pay time** — re-descend from the root each step, `O(depth)` per step, turning a
  full iteration into `O(n·depth)`;
- **pay space** — a **parent index** in each node, `O(1)` amortised.

We pay the space: 4 bytes per node (12 → 16, which also makes a node exactly two store
words instead of straddling).  `RadixIter` becomes three `u32`s — `Copy`, so one seek
clones into two cursors for the outward walk, with no allocation anywhere.  A test
pins `size_of::<RadixIter>() <= 16`, and a `Vec` is 24.

The parent link is maintained at **three** sites (insert's split, the displaced
subtree, remove's splice) and getting it wrong is silent.  So it gets its own
invariant, checked by the validator that already runs after every mutation:

> **I3** — `parent(child(n, d)) == n`, and the root's parent is `0`.

### The bound that keeps a corrupt tree from hanging

Production must never panic and must chug along — but an **infinite loop is worse than
a panic**, and this is a real hazard the parent chain introduces.

Bounding the parent *climb* is **not enough**, and the probe proved it: with a stale
parent link the child pointers still form a valid tree, so `next` keeps returning
records and it is the *caller's* loop that never ends.  A ceiling on one `step()`
never fires; `r4` hung anyway.

The correct bound is per-**cursor**: a walk over `LEN` records cannot legitimately
yield more than `LEN` of them.  A cursor carries that budget, decrements per step, and
simply stops when it runs out.  It costs 4 bytes and a decrement — no store read —
and every intended walk (iteration, and the monotone outward proximity scan) never
approaches it.  With the budget, the same corruption makes `r4` and `r6` **fail fast**
instead of spinning.

## 5. Representation

`Store::claim(n)` counts **8-byte words**; `fld` is a **byte** offset, valid for
`4 <= fld < 8*words`; `fld 0..4` is the store's own claim header.

**Tree container record** — one claim, in the host record's store, its rec-id held
in the host's 4-byte field (exactly the `hash` bucket-record convention):

```
fld  0   i32   claim header                       (Store-owned)
fld  4   i32   TOP    0 = empty | >0 = record id | <0 = node id
fld  8   u32   LEN    number of records
fld 12   u32   NODES  node high-water mark (ids 1..=NODES)
fld 16   u32   CAP    node capacity
fld 20   u32   FREE   head of the free-node list (0 = none)
fld 24   ...   node array — node n (1-based) at 24 + 12*(n-1)
```

**Node — 16 bytes**, exactly two store words (header 24, so every node is
word-aligned).

```
+0   u32  bit     absolute bit index tested
+4   u32  parent  the node above; 0 at the root  (I3)
+8   i32  false   child
+12  i32  true    child
```

A **free** node is threaded through its `false` slot (next free id).  Node ids are
1-based so `0` can mean *empty child*, which a well-formed tree never contains.

**Child encoding — the single chokepoint (failure path 5).**  Every read and write
of a child slot goes through one accessor pair; the sign trick is never open-coded:

```rust
enum Child { Empty, Rec(u32), Node(u32) }   //  0  |  >0  |  <0
```

The sketch open-coded it in five places and got it wrong in two (`straight` negates a
value that is already negative; `rtree_insert` applies `-node` to path entries whose
TRUE-side entries are stored *positive*).

**Free structural check.**  A binary tree whose every internal node has exactly two
children has precisely `LEN - 1` internal nodes.  So

> `live_nodes == LEN - 1` (for `LEN >= 1`)

is a cheap, total invariant for `rtree_validate` — it catches a leaked node, a
double-free, and a failed splice with one `assert_eq!`.

---

## 6. Predecessor / successor of an absent key

Failure path 10, written out because deliverable S3 depends on it and the plan's
one-line description of it is wrong.

Descending on a search key `k` that is **not** in the tree reaches a candidate leaf
`c` that shares the longest prefix with `k` — but `c` may be neither the predecessor
nor the successor of `k`, because descent skipped the bits path compression elided.
The correct procedure:

1. descend by `k` → candidate `c`, recording the path;
2. `d = first_diff_bit(k, c)`; let `dir = key(k, d)`;
3. re-ascend to the **deepest node `p` on the path with `bit(p) < d`**.  Every record
   under `p` agrees with `k` on all bits `< d`;
4. if `dir == 1`, `k` sorts **after** everything in `p`'s subtree reachable below the
   divergence: the predecessor is the **right-most** leaf of that subtree, and the
   successor is the in-order successor of `p`'s subtree.  If `dir == 0`, mirrored.

Both `next()` and `prev()` are then the ordinary in-order step, and the "loop left
and right" proximity walk of @PLN48 is two iterators seeded at step 4.

---

## 7. Steps — each one lands with the check that proves it

Small and independently verifiable, in order.  Every step is a `cargo test`; none
depends on the database, the parser, or a loft program.  The test oracle throughout
is a `u32` key MSB-first with a 32-bit record-id suffix (P2/P3 discharged), plus
`std::collections::BTreeMap` as a differential oracle.

All eight are green (`cargo test --lib radix_tree::`), and `rtree_validate` runs
after **every** mutation in R3, R6 and R7.

| Step | Lands | The check that proves it | |
|---|---|---|---|
| **R1** | header + node alloc/free list; `rtree_init`, `rtree_free`; the `Child` accessor | tree **and host** freed ⇒ `store.claims_empty()`.  The host is claimed first so the tree cannot land on `PRIMARY`, whose deletion would empty `claims` on its own and hide a leak | ✅ |
| **R2** | `rtree_insert` for `LEN` 0→1→2; `rtree_find` | insert 2 distinct keys; `find` reaches each; `NODES == LEN - 1` | ✅ |
| **R3** | general `insert` + `rtree_validate` | 1 000 deterministic pseudo-random keys, `validate` after **each** insert | ✅ |
| **R4** | `rtree_first` / `rtree_last` / `next` / `prev` | in-order walk is strictly increasing and yields exactly `LEN` records; the backward walk is its exact reverse | ✅ |
| **R5** | `rtree_seek` — lower bound, incl. keys never inserted (§6) | 300 absent probe keys against a sorted-`Vec` oracle; plus every present key seeks to itself | ✅ |
| **R6** | `rtree_remove` + free-list reuse | remove in shuffled order against a `BTreeMap` oracle, `validate` after each; `rtree_free` ⇒ `claims_empty()`.  R6b proves a freed node id is **reused**, not re-minted | ✅ |
| **R7** | growth: `resize` past `CAP`, rec-id repoint | 200 inserts from `CAP = 0`; asserts the record actually **relocated**, then that order and membership survived it | ✅ |

`rtree_validate` checks I1 structurally (strictly increasing bits, two children per
node, each leaf's key agreeing with every branch decision above it), **I2** (§2.1 —
the skipped bits are common to the subtree), plus the counts a two-children-per-node
tree forces: `leaves == LEN`, `live_nodes == LEN - 1`, and `live + freed == NODES`.
A leaked node, a double-free and a failed splice each break one of them.

### Proving the harness can fail

A green step that cannot go red is vacuous, so each subtle piece was mutated and the
suite re-run:

| Mutation | Predicted | Observed |
|---|---|---|
| `rtree_seek` returns the candidate leaf directly — i.e. the "candidate is the neighbour" bug §6 exists to prevent | only R5 reddens | **exactly R5**, 7 others green |
| `split_index` returns the wrong edge | the structural checks catch it | 6 red; `rtree_validate` names it: *"I1: bit 2 does not exceed parent bit 5"* |
| the new node takes bit `d + 1` | something catches it | the per-leaf branch constraint catches it — *not* I2 |
| `first_diff_bit` starts at bit 1, so a divergence at bit 0 is missed — an I2 violation that leaves I1 and the constraints intact | only I2 reddens R3 | **I2 fires at insert time**: *"node 1 skips bit 0 on its way to bit 1, but records 4 and 5 below it disagree there."*  With I2 removed, **R3 passes clean** and the corruption only surfaces downstream in R4/R5/R6 |

| the `0x00` terminator is dropped from the key string | text ordering breaks | **only R8c** reddens.  R8 stays green — small record ids lead with `0x00`, which silently plays the terminator's part, so R8 alone is *vacuous* for this claim.  R8c uses ids leading with `0x70` (above `'c'`) |
| the id suffix is dropped from the key string | duplicate keys collapse | exactly the two duplicate-key tests redden (R2d, R8b) |
| a parent link is left stale (the displaced subtree) | I3 catches it | **I3 fires at insert time**: *"node 2 does not point back at the node above it."*  And without the cursor budget the same corruption made R4 **hang** rather than fail — see §4.1 |

That I2 row is why I2 is in the validator: without it the structural pass is blind
to a bad skip, and the failure reappears three steps later as an inexplicable
ordering bug.  Each design element now has exactly one witness that dies without it.

**The mutation also caught a hole in the test data.**  The first attempt at that
mutation changed nothing, because `lcg` returned `(seed >> 33) as u32` — a **31-bit**
code, pinning the key's most significant bit to `0` for every record.  No node ever
branched on bit 0, so "miss bit 0" was a no-op and the cell was vacuous.  Shifting by
32 exercises the full key width.  A probe that cannot fail proves nothing, and this
one nearly didn't.

### Validating against the prediction (design-protocol step 6)

Written before the code: *"roughly **350–450 lines** including `validate`."*
Actual: **407 lines** of real code (excluding doc comments, blanks, and the 288-line
test section), against the sketch's 191 code lines that could not insert twice.
Inside the band — no alarm to route, and the extra mechanism over the sketch is the
free list, the growth path, `seek`, `remove`, and `validate`, all of which the sketch
simply did not have.

---

## 7.2 Measurements — the benchmark and the sanitizer gate

### Benchmark

`cargo test --release --lib radix_tree::tests::bench -- --ignored --nocapture`
(`#[ignore]` by default, as the repo does for heavy Rust work).  Absolute ns/op is
machine-specific, so every figure sits beside `std::collections::BTreeMap` doing the
same work on the same keys — the structure a loft user would otherwise reach for via
`sorted<T[k]>`.  The *ratio* is what travels.

n = 100 000 random 32-bit codes, ns/op:

| op | radix | BTreeMap | ratio |
|---|---:|---:|---:|
| insert | 541.7 | 79.4 | 6.8× |
| get (exact) | 429.3 | 69.1 | 6.2× |
| walk (in-order) | 17.0 | 2.3 | 7.5× |
| **remove** | **117.0** | 79.0 | **1.5×** |

16 bytes of node per record (`LEN-1` nodes, 16 B each).

**The numbers corrected a claim I had been making.**  I had said the bit-at-a-time
`first_diff` was the dominant constant factor.  `remove` is the control: it descends
but never compares keys — and it is only **1.5×** BTreeMap.  Everything that does
compare keys is 6–7×.  So the cost is not the descent and not the fan-out at this
size; it is the **per-bit loops** (`first_diff`, and `rtree_key_eq`'s 32-bit scan in
`rtree_get`), each iteration an un-inlinable `fn`-pointer call plus a bounds-checked
store read.  A chunked oracle (`word(rec, i) -> u64`, `XOR` + `leading_zeros`) would
collapse both.  A cheaper win first: `rtree_seek` already computes the first differing
bit `d`, and `d >= probe_bits + TERM_BITS` proves the landed record's user key equals
the probe — so `rtree_get` need not re-scan all 32 bits.

### The workload that decides S4

512 entities, every one moving each frame (remove + reinsert), then a proximity query
seeded from one `seek` and walked 8 records each way:

| | |
|---|---:|
| move (remove + insert) | 215.6 ns |
| proximity query (seek + 8 each way) | 247.2 ns |
| **per-frame, 512 entities** | **110.6 µs** |

Comfortably inside a frame budget, and allocation-free: the second cursor is a `Copy`
of the first.

### What the sanitizer gate actually covers

`./scripts/asan.sh --lib -E 'test(radix_tree)'` → **15/15 clean**.  But a green
sanitizer proves nothing until it can go red, and probing it produced a caveat worth
writing down:

| deliberate fault | ASan | `-C debug-assertions=on` |
|---|---|---|
| node read **past the arena** (a wild offset, like the old sketch's `3 + (-node) as u32`) | **SEGV — caught** | caught |
| node read past the tree **record**, still inside the arena | **passes silently** | **caught**: *"Fld 4808 is outside of record 4097 size 40"* |

loft's `Store` is one arena allocation and ASan checks *allocation* bounds, so the
intra-arena overrun — exactly what a node-index bug produces — is **invisible to
ASan**.  The gate that covers this module is `Store::valid()`, i.e. CI's
`debug-asserts` job; the suite is clean there too.  Which is also why deliverable R
puts its structural weight on `rtree_validate` rather than on a sanitizer.

**Repaired along the way:** `scripts/asan.sh` could not run at all.  It used a bare
`+nightly` (a recent nightly fails to compile `curve25519-dalek`) and omitted
`--target x86_64-unknown-linux-gnu`, so `-Zsanitizer=address` was applied to host
proc-macros and the build died with `E0463: can't find crate for zerofrom_derive` —
despite the script's header claiming it mirrored CI's flags.  It now pins CI's
nightly and scopes the flag to the target.

---

## 7.1 Known gaps — what R does *not* yet prove

Stated plainly, because each is a place a downstream deliverable would otherwise
discover the hard way:

- **The Morton key is untested.**  R proves the tree against a fixed-width oracle and
  a variable-length text oracle.  Interleaving is S1's job, and S1 must re-run R3–R7
  with a Morton `KeySpec` before S2 trusts it.
- **The per-bit loops are the measured bottleneck** (§7.2), not the descent: `remove`,
  which never compares keys, is 1.5× BTreeMap while `insert`/`get` are 6–7×.  The fix
  is a chunk accessor (`word(rec, i) -> u64`) plus `XOR` + `leading_zeros` — a
  **compare-side** change that leaves the representation alone.  (The per-bit
  re-derivation of the key length is already hoisted out.)
- **Binary fan-out.**  Depth goes as `log₂ n` where ART or a qp-trie would give
  `log₁₆ n` / `log₂₅₆ n`.  Deliberate: those want byte- or nibble-addressable keys,
  which means materialising the Morton code — the thing the bit oracle exists to
  avoid.  For a per-chunk index (a few hundred entities, depth ≈ 9) it does not bite.
  The upgrade path, if ever needed, is to widen the oracle to a nibble.
- **No bulk load, no compaction, no concurrency.**  The free list never returns memory
  to the store; a sorted bulk build would be `O(n)` rather than `n × descent`.
- **No fuzzing and no property tests.**  The benchmark and the ASan/debug-asserts runs
  now exist (§7.2); the mutation testing behind §7 was done by hand, not
  `cargo-mutants`.
- **Two undocumented bounds, now documented.**  `Child::Rec` packs a record id into an
  `i32`, safe *only* because `MAX_STORE_WORDS == i32::MAX`.  And `node_off` overflows
  `u32` above ~268M nodes, a size the store limit technically permits.

Two gaps an earlier revision listed here are **closed**: `rtree_seek` now accepts a
shorter probe (a prefix query — §1.1), and the cursor no longer allocates (§4.1).

## 8. What deliverable R does **not** decide — and the number S2 needs

Deliverable R is identical whether or not `radix<T[key]>` becomes loft-visible,
because the key oracle is the whole interface.  But S2 has to choose, so the cost was
measured now rather than discovered later.

**Adding a new `Parts::Radix` / `Type::Radix` variant costs ~9 compile errors and
~120 silent catch-alls.**  Exhaustive matches that would *force* an arm: `ir_store`
(`write_type`, `write_parts`), `ir_schema::type_to_json`, `ir_read::read_type`,
`snapshot::parts_to_json`, `io::{read,write}_data`, `search::{find,iterate,remove}`.
Everything else — ~107 `Type::Spacial` arms across 36 files and ~29 `Parts::Spacial`
arms — is a grouped arm or `matches!` in front of a `_ =>`, so a new variant
compiles clean and is silently mis-handled.  The worst is
`allocation.rs::for_each_owned_child` (`_ => {}`): `Spacial`'s safety there comes
from *ad-hoc explicit panics at the call sites* (`allocation.rs:1806`, `:2072`) that a
new variant would simply bypass — so `Radix` teardown would **silently leak** instead
of failing loud.  `N × silence` ≈ 120 × silent.

**Recommendation for S2: do not add a variant.**  A radix tree over `N = 1` key field
*is* a plain radix tree, because interleaving one axis is the identity — so
`Parts::Spacial(content, fields)` already has exactly the right shape, and the key
oracle is derivable from the key fields' types (`types[tp].keys`): one field ⇒ that
field's bits; N fields ⇒ Morton interleave.  `radix<T[k]>` can then be a
**parser-level alias** that lowers to the same runtime kind, holding the reserved
`type radix;` name (`default/01_code.loft:1265`) at zero new silent sites.

This is one family, not two compressed into one: the only thing that varies is which
bits the oracle yields.  The claim worth watching is the *name* — a `radix<T[text]>`
whose diagnostics say "spacial" is a UX wart S2 must handle, and `sorted<T[k]>`
already covers ordered lookup on one key, so a loft-visible `radix` earns its keep
through **text prefix queries**, the **finger move** (cheap incremental re-index), and
**spatial** — not through plain ordering.  Its first concrete consumer, and the
build-and-test reference, is the tracker/symbol index:
[RADIX_TEXT_INDEX.md](RADIX_TEXT_INDEX.md).

Also left to S2/S3: the Morton interleave, the `spacial` arms in `search.rs`
(`find`/`iterate`/`remove` currently panic as "non-collection"), a
`for_each_owned_child` arm, `copy_claims` by re-insert, and the proximity API.

### 8.1 Settled: one runtime variant, named for the storage

`spacial` and `radix` share **one** `Type` / `Parts` variant — because the thing that
distinguishes them is the **key signature the variant already stores**, not the
storage.  The wiring (`insert`/`find`/`remove`/`copy_claims`/teardown) is
oracle-agnostic; everything that differs is a *function of the key fields*:

| distinction | derived from the key fields |
|---|---|
| which oracle | count + types: 1 int → ordered, 1 text → prefix, 2–3 int → Morton |
| which methods are legal | `near`/`within`/`nearest` need 2–3 coord fields; `prefix`/`range` need 1 |
| surface type name | 1 field → `radix`, N coord fields → `spacial` |
| parse-time arity | `radix` = exactly 1 key field; `spacial` = 2–3 coord fields |

This is *not* over-unification: the shared part (storage + wiring) is genuinely
identical, and the divergent part is computed from the variant's own data.  The one
discipline it demands: **choose the oracle in exactly one chokepoint** —
`oracle_for(type) -> impl KeyOracle`, which reads the keys — never re-derived at the
wiring sites (the same single-chokepoint rule the tree follows internally).

**The variant is named `Radix`, not `Spacial`.**  The storage *is* a radix tree;
`spacial` and `radix` are both *uses* of it, so the honest name is the storage's.  The
rename lands before the wiring — it is cheapest while the variant is still unwired
(mechanical, mostly grouped arms), and only gets more expensive once it is threaded
through working code.  The **surface** stays untouched: users still write
`spacial<T[x,y]>` (and, later, `radix<T[k]>`); the keyword, the `Database::spacial`
constructor, and the printed type name are surface-facing and keep the `spacial`
spelling.

---

## 9. The proximity API (`src/spatial.rs`) — @PLN48 S3

The tree is geometry-free: it only sees a 64-bit key.  `src/spatial.rs` supplies the
geometry — a point is a record with `u32` `x`@4, `y`@8, keyed by its Morton (Z-order)
code — and the three queries S3 owes, two exact and one deliberately not.

Both exact queries rest on one property, which `s0` tests directly:

> **Morton monotonicity.**  `morton(x,y) = spread(x) + 2·spread(y)`, and `spread` is
> increasing, so the code rises with each axis on its own.  A box `[x0,x1]×[y0,y1]`
> thus has its least code at `(x0,y0)`, its greatest at `(x1,y1)`, and every point of
> the box has a code between them — so scanning that code interval **cannot skip a box
> point**, which is what makes a scan a sound basis for an exact query.

| query | guarantee | how |
|---|---|---|
| `within(c, r)` | **exact** | the disc's bounding box lies in the scanned code interval; keep points with `dist² ≤ r²` |
| `nearest(c, k)` | **exact** | expanding box: scan half-width `r`; stop once `k` are found and the `k`-th is within `r`, since anything *outside* the box is farther than `r` ≥ that `k`-th distance |
| `near(c)` | **approximate** | two cursors walk outward in Morton order — cheap and allocation-free, but Z-order jumps at quadrant boundaries, so a near point can arrive late |

`s1`/`s2` diff the exact queries against brute force; each exactness mechanism has a
mutation witness (shrinking the box reddens `s1`; dropping the k-NN guard reddens
`s2`).  `s3b` asserts `near` *does* disagree with true nearest — if it ever stops, the
"approximate" contract was silently broken.

### The crossover (measured — a spatial index is not free)

`ns/query`, best of 3, index vs brute force, in a 2048² field:

| points | `within(r=24)` | `nearest(k=8)` | `near(k=8)` |
|---:|---|---|---:|
| 200 | 139 vs 133 | 3382 vs 2549 | 205 |
| 1 000 | 298 vs **618** | 7124 vs **13691** | 262 |
| 5 000 | 1183 vs **3014** | 15934 vs **73986** | 318 |
| 20 000 | 5434 vs **12704** | 33267 vs **317056** | 395 |

Read it as guidance, not decoration:

- **Below a few hundred points the index does not beat brute force** — the scan pays a
  descent and per-step store reads while brute force is a cache-friendly sweep.  A
  per-chunk index of a hundred mobs is near break-even for the *exact* queries.
- **The index wins once the field is a few hundred–thousand**, and `nearest` pulls away
  fastest (brute force sorts the whole field per query): 9.5× at 20k.
- **`near` is the standout — flat ~200–400 ns regardless of field size**, because it is
  `O(k)`, not `O(n)`.  For aggro / interest management, where approximate is fine, it
  is the reason to have the index at chunk scale at all.
- **Cheapest win is upstream:** quantize coordinates to a cell (`@PLN48`), and ~3/4 of
  drifts change no key, so the index is never touched (`bench_move_finger`).

### Left to S2 proper

`spatial.rs` proves the *algorithm* at the Rust layer; wiring it to the loft language —
`spacial<T[x,y]>` insert/find/remove/`copy_claims`, lifting the parser's 1.1+ gate, and
the `for_each_owned_child` arm so teardown does not leak — is S2, costed in §8.  3D
(`spacial<T[x,y,z]>`) is the same machinery with a third interleaved axis.
