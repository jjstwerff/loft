# A text-keyed trie collection, in steps that cannot half-land

Status: design. Nothing is built to this design yet; see *Step 0 is in the wrong place*.

## The invariant

> **A trie and a spatial index share the radix TREE and nothing above it.**

`radix_tree.rs` is a PATRICIA tree over an abstract bit-key oracle, with per-record
`bits()` and a `TERM_BITS` suffix whose stated job is that a shorter key sorts before a
longer one extending it. That is the shared part, it is already generic, and it is
already proven for text (`r8`, `r8b`, `r8c`, `r2e`). Everything above it — the schema
kind, the oracle, the operation set, the surface — is separate.

## The design this replaces, and why it was wrong

The first draft reused `Parts::Radix` and selected the oracle by key type, so a text key
and a coordinate key were "one collection with two oracles". Its argument was a measured
one: a new variant re-states itself at ~130 sites, and collapsing that to a single
`DbOracle::of` chokepoint made the re-assertion count `N = 1`.

That is over-unification, and it presented exactly as the design protocol says it does —
as elegance. **`spatial` is not called `radix` on purpose.** Its properties are
geometric: Morton/Z-order interleaving of 1–3 axes, bounding boxes, near / within /
nearest. None of that means anything for a word. Sharing the storage structure is not
sharing the kind, and the rename to `Parts::Radix` was storage-honesty about the tree,
not a claim that the two families are one.

The first draft half-admitted this itself: its own step 3 conceded *"a text range is not
a coordinate box … a sibling entry point, not an argument change"*. Once the operation
sets diverge, the shared kind is a false invariant that would break the moment each case
asserted its real difference.

### The measurement that makes the separate kind safe

`N = ~130` looked like the brittleness. It is not, because **`N × silence` has two
factors and this language kills the second one for free**:

| | |
|---|---|
| `Parts::` references in `src/` | **765** |
| wildcard (`_ =>` / `other =>`) arms near them | **~24** |

A new `Parts` variant is a **compile error** at every exhaustive match. The compiler
enumerates the work list; there is nothing to remember and nothing to forget. The silent
risk is confined to the wildcard arms — a short, auditable list, and the only place a
new kind can be quietly mishandled.

So the cure here is not collapsing `N`; it is that `N` is already loud. That is what
makes a genuinely separate implementation the cheap option as well as the correct one.

## What must hold across the boundary

- A **probe and its oracle read a key the same way.** Divergence presents as *"inserted,
  then not found"* — indistinguishable from an ordinary miss.
- A **shorter key sorts before a longer one that extends it** (`kerk` before
  `kerkstraat`).
- A kind that is **declarable must answer** on every operation its surface offers.
  Anything less rebuilds loft#799.
- A collection that is **freed frees its records.**
- **No trie concept appears in the spatial path, and no geometry concept in the trie
  path.** The two share a tree, not a vocabulary. This is the invariant restated as a
  review rule.

## Step 0 is in the wrong place — undo it first

A `TextOracle` and a `DbOracle::of` selector were committed into `src/radix_db.rs`
(74f35ccf). The oracle itself is right and proven at the DB record layout; **its home is
wrong** — it makes the spatial module know about text keys, which is precisely the
coupling this design removes.

*First action:* move `TextOracle` and its tests into the new trie module and restore
`radix_db.rs` to Morton-only. The tests move unchanged; they are the proof the oracle
still works after the move.

## The steps

Each is independently landable and leaves the tree green. The keyword is last: it is
what makes everything before it reachable from loft source, so nothing may be speakable
before it answers.

### Step 1 — fix the spatial box query (loft#800)

Unrelated to text, and first because it is a **live wrong answer** for existing users:
`radix_db::range` walks the Morton interval and stops on a code comparison, with no
per-axis containment test, so every non-degenerate box returns a superset.

Doing it first also keeps the two families honestly separate — it is the geometry side
being finished on its own terms, before anything text-shaped is near it.

*Gate:* a box the Z-order visibly leaves — `(1,1)..(3,9)` over `(1,1) (5,2) (3,9)` —
answers `n10 n30`, hand-computed, both backends. Keep the degenerate box and the
all-enclosing box as controls: they pass today, which is why this survived undetected.

### Step 2 — the kind

Add the `Parts` variant and its runtime `Type` twin. Then **let the compiler produce the
work list** — build, and fix each non-exhaustive match it names.

*Gate:* the build is the instrument. Separately, audit the wildcard arms by hand, since
those are the only sites the compiler cannot name.

**Done.** The compiler named 16 sites. The hand audit found 55 matches on `.parts` with a
wildcard, but most name only scalars (`Byte`/`Short`/`Int`/`Struct`), where "not a narrow
int" is a correct answer for a trie. **Twelve** enumerate the COLLECTION kinds, which is
where a trie falls through into the wrong answer:

| verdict | sites | what falling through would have done |
|---|---|---|
| mechanical arm added | 9 | the element type never marked LINKED or reachable; the key never registered, so `keys(db)` is empty and every key read is wrong; the container record never claimed; layout never validated; iteration never routed |
| loud stub, step 3 | 2 | an insert that does nothing; a deep copy that drops the collection |
| loud stub, step 6 | 1 | the trie's IO omitted from `--native` only — a one-backend divergence |

The nine were real: a trie whose key is not registered answers every lookup wrong, and
nothing would have said so. That is the omission class this audit exists to find, and it
is why the wildcard count — not `N` — was the number that mattered.

### Step 3 — the oracle and the tree operations

Move the oracle in (from step 0), then insert / find / remove / records / iterate over
the tree, with the trie's own key reading.

*Gate:* in-order traversal is lexicographic with `kerk` before its three extensions;
exact lookup finds every inserted key and refuses `kerks`, `ker`, `kerkstraatx` — the
shapes a wrong terminator or a probe/oracle mismatch answers for.

**Done.** `add` / `find` / `remove` / `records` / `count` in `trie_db.rs`, wired into the
three step-2 stubs (`search.rs` find + remove, `structures.rs` insert), which are gone.
Tested at two levels deliberately: through the TREE (the oracle reads a record's key
right) and through the COLLECTION surface the callers use, so a mistake in the plumbing —
the tree id written back into the field, an empty key list, a probe built differently
from the oracle — cannot hide behind a correct tree. Removal is asserted on `kerklaan`
specifically, because unlinking one member of a prefix family is where a radix removal
goes wrong.

Still stubbed after this step: `copy_claims` (a deep copy that would drop the
collection), the layout descriptor and the IR discriminant. Teardown is step 4; the two
FORMAT surfaces wait for step 6, when a trie can be constructed and their arms tested.

### Step 4 — teardown

Free the collection's records. The spatial side is unimplemented here too and guarded as
a loud failure because a silent no-op leaks; the trie must not inherit that hole.

*Gate:* a `clean/` leak case that builds and drops a trie reports no unfreed stores.

### Step 5 — the prefix range

The capability that earns the kind its place, and the one `sorted` cannot offer.

*Design question this step answers rather than assumes:* the surface should be
`c["kerk"..]` — an open-ended prefix, which the tree gives natively (a probe carries
id `0`, so seeking a prefix lands on its first extension). Requiring a successor string
(`c["kerk".."kerl"]`, the `sorted` spelling) would make this a slower `sorted` and
forfeit the reason to build it.

*Gate:* `c["kerk"..]` yields exactly the keys bearing that prefix, in order, `kerk`
included; an absent prefix yields nothing rather than its neighbour.

### Step 6 — the keyword

`trie<T[w]>`. `spatial` is untouched.

*Gate:* every operation the keyword exposes answers on both backends — declare, insert,
exact lookup, prefix range, iterate, free — against hand-computed values, with a
`sorted` control alongside so a regression cannot hide in "both are empty".

### Step 7 — the diagnostics (loft#799)

Only now is the advice true: a text key under `spatial` says use `trie`, a coordinate
key under `trie` says use `spatial`.

## The residual

The axis this design cannot see is **scale**. Every measurement here is six words in a
test. The reason to want the kind at all — a 518 804-record name index — is where a
structure's real behaviour appears, and no gate above touches it. Step 6 should be
followed by a dogfood run against that data before the kind is documented as ready.
