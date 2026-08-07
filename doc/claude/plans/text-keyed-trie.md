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

*Gate:* the build is the instrument. Separately, audit the ~24 wildcard arms by hand,
since those are the only sites the compiler cannot name; record the verdict for each in
the commit rather than leaving it implied.

### Step 3 — the oracle and the tree operations

Move the oracle in (from step 0), then insert / find / remove / records / iterate over
the tree, with the trie's own key reading.

*Gate:* in-order traversal is lexicographic with `kerk` before its three extensions;
exact lookup finds every inserted key and refuses `kerks`, `ker`, `kerkstraatx` — the
shapes a wrong terminator or a probe/oracle mismatch answers for.

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
