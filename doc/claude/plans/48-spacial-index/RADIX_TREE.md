# @PLN48 · Deliverable R — the radix tree

**Status:** landed — `src/radix_tree.rs`, steps R1–R7 green, clippy clean.
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

### 1.1 Preconditions on the key oracle

The oracle is `key(store, rec, bit) -> Option<bool>` — bit `0` is the **most
significant** bit, `None` means "the key has ended".  I1 holds only if:

- **P1 — prefix-closed.** `key(..,b)` is `Some` for every `b < len(rec)` and `None`
  for every `b >= len(rec)`.
- **P2 — prefix-free.** No key is a proper prefix of another.  A node branches on a
  *bit*; it cannot branch on "the key ended here".
- **P3 — distinct.** No two live records have equal keys, or no differing bit `d`
  exists and the split is undefined.

These are the **only** obligations a caller has, and each key mode discharges them
explicitly rather than by hand-wave:

| key mode | P2 discharged by | P3 discharged by |
|---|---|---|
| fixed-width integer, Morton code | all keys are `W` bits ⇒ none is a proper prefix | append the 32-bit record id ⇒ length `W+32` |
| text (UTF-8) | append one virtual `NUL` byte — legal *because* loft text is UTF-8 and excludes `0x00` | append the 32-bit record id |

**Loudness (design-protocol step 2, second cure).**  A violated P3 is otherwise a
*silent* corruption.  `first_diff_bit` therefore ends in
`debug_assert!(found, "duplicate key")` — forgetting the tie-break suffix panics in
test builds instead of quietly producing a tree that fails I1.

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

**Node — 12 bytes.**  Header is 24 (a multiple of 8, so nodes are word-aligned) and
`12*(n-1)` is 4-aligned, which is what `addr::<u32>` requires.

```
+0  u32  bit     absolute bit index tested
+4  i32  false   child
+8  i32  true    child
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

That last row is why I2 is in the validator: without it the structural pass is blind
to a bad skip, and the failure reappears three steps later as an inexplicable
ordering bug.

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

## 7.1 Known gaps — what R does *not* yet prove

Stated plainly, because each one is a place a downstream deliverable will otherwise
discover the hard way:

- **The Morton key is untested.**  R proves the tree against a 64-bit fixed-width
  oracle.  Interleaving is S1's job, and S1 must re-run R3–R7 with a Morton `KeyFn`
  before S2 trusts it.
- **Text keys are designed, not exercised.**  The virtual-`NUL` construction (§1.1)
  discharges P2 on paper; no test yet builds a text oracle.
- **`rtree_seek` cannot take a *shorter* search key.**  It compares the probe against
  the candidate via `first_diff_bit`, which trips the P2 `debug_assert` when the probe
  ends first.  So a **prefix** query — `seek("ab")` over keys `"abc"`, `"abd"` — is
  *not* supported by today's entry point, even though `descend` already treats a
  `None` bit as `0`.  This matters: §8 argues a loft-visible `radix<T[text]>` earns
  its keep precisely through prefix queries, so that use needs a separate
  `rtree_seek_prefix` whose contract admits an exhausted probe key.  `spacial` is
  unaffected — its keys are fixed width.
- **Iterator allocation is unmeasured.**  Each `rtree_first`/`rtree_seek` allocates a
  `Vec<Step>`.  S3's proximity walk seeds two iterators per query per frame, so this
  is the first place to look if S4's measurement disappoints.  The fixed `[i32; 64]`
  it replaced was a correctness hazard (§3), so the allocation is deliberate, not an
  oversight — but it is a bet, and S4 is where it gets called.

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
only through **text prefix queries**, not through ordering.

Also left to S2/S3: the Morton interleave, the `spacial` arms in `search.rs`
(`find`/`iterate`/`remove` currently panic as "non-collection"), a
`for_each_owned_child` arm, `copy_claims` by re-insert, and the proximity API.
