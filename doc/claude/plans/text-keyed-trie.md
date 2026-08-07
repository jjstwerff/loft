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

**Done, with the gate adapted.** A `clean/` case needs a trie a `.loft` program can
DECLARE, and that is step 6 — so the gate is asserted where a trie can be built today:
through `Stores::remove_claims`, with `claims_count` as the observable (`claim` inserts,
`delete` removes, so a teardown that works returns it to baseline). The `.loft` leak case
still lands at step 6, when it becomes writable.

The fault was real and exactly the predicted shape: `for_each_owned_child` had no `Trie`
arm, so a trie fell into its catch-all, the keystone yielded no children and no
container, and `remove_claims` freed **nothing** — every element and the tree, silently.
`copy_claims` got its own body rather than sharing the Radix one, because the two differ
precisely where the kinds differ (whose `add` re-inserts each element).

### What the audit instrument missed, twice

Step 2's wildcard scan used a 120-line window and only looked at `match`. Both bounds
were wrong, and each hid a live site:

| gap | hid | found by |
|---|---|---|
| 120-line window | `for_each_owned_child` (its match spans ~195 lines) — the teardown fault above | brace-matching instead of a window |
| `match` only | `zero_field`, a `matches!` macro — without it the field is never cleared, so a re-bind re-reads a freed tree | scanning `matches!` bodies too |

Re-running both corrected scans found 9 wildcard `match` sites and 6 `matches!` sites.
Of those, the load-bearing ones are fixed; the rest are diagnostics, or places where
`Radix` is absent too — so a trie falling through behaves exactly like a spatial does
today, which is the consistent answer rather than an omission.

The lesson is the skill's own: a scan that reports "N sites" with a bound nobody stated
is a blind instrument, and its silence reads exactly like coverage.

### Step 5 — the prefix range

The capability that earns the kind its place, and the one `sorted` cannot offer.

*Design question this step answers rather than assumes:* the surface should be
`c["kerk"..]` — an open-ended prefix, which the tree gives natively (a probe carries
id `0`, so seeking a prefix lands on its first extension). Requiring a successor string
(`c["kerk".."kerl"]`, the `sorted` spelling) would make this a slower `sorted` and
forfeit the reason to build it.

*Gate:* `c["kerk"..]` yields exactly the keys bearing that prefix, in order, `kerk`
included; an absent prefix yields nothing rather than its neighbour.

**Done at the runtime level** — `trie_db::prefix`. The surface spelling is step 6; this
is the operation behind it, and the design question is answered rather than assumed: the
prefix IS the query. Seek to it, then walk while the key still begins with it. Both
halves rest on facts the tree already guarantees — `rtree_seek` lands on the first record
BEARING the prefix (a probe carries id `0`), and in-order traversal is increasing key
order, so every extension is contiguous from there. That makes the first key not
beginning with the prefix an exact stop rather than a heuristic one, and it is why no
successor string is needed.

The gate is hand-computed from the bytes, and `"kerx"` is the cell that matters: it sorts
between `kerkweg` and `lonneker`, so a seek lands on `lonneker` and a missing stop-check
answers **that**. Proven non-vacuous by breaking the stop-check on purpose — `lonneker`
then leaks into every prefix answer and two tests catch it.

### Step 6 — the keyword

`trie<T[w]>`. `spatial` is untouched.

**Started, and the measurement says split it.** `Type::Trie` (the runtime twin) is in,
its nine compiler-named sites are handled, and three IR-codec sites carry loud stubs.
But the assumption this step inherited from step 2 — *let the compiler produce the work
list* — **does not hold for `Type`**:

| | `Parts::Trie` (step 2) | `Type::Trie` (step 6) |
|---|---|---|
| compiler-named | 16 | **9** |
| silent (`_` wildcard or `matches!`) | ~12 | **141** |

`Parts` is matched exhaustively almost everywhere, so adding a variant is loud. `Type`
is matched non-exhaustively almost everywhere — 92 wildcard `match`es and 49 `matches!`
macros name two or more collection kinds without naming `Trie`. The compiler enumerates
6% of the work here, against roughly half in step 2.

So step 6 must not be done the way step 2 was. Proposed split:

- **6a — the audit. DONE.** Clustering did the work; reading 141 sites individually was
  never the job. Two questions cut it down:

  **1. Does the site already name `Radix`?** If not, `spatial` is excluded there too, so
  a trie falling through behaves exactly like its sibling — consistent, not an omission.
  That is 24 sites, no action.

  **2. Of the 117 that do name `Radix`: does `Radix` share an `|` ALTERNATION with its
  keyed siblings, or does it have its own arm?** An alternation means the site already
  treats every keyed collection alike, so one verdict covers all of them.

  | | sites | verdict |
  |---|---|---|
  | no `Radix` | 24 | none — a trie matches spatial's treatment |
  | `Radix` in an alternation | **104** | add `Trie` to the alternation; mechanical, and safe because the site already makes no distinction |
  | `Radix` has its own arm | **13** | read individually — below |

  The 13 turned out to be mostly ONE question, which is why they are not 13 decisions:

  - **8 ask "what is this type's db-type id?"** and answer `database.spatial(c, key)` —
    `typedef.rs` ×2, `state/codegen.rs` ×2, `parser/{vectors,expressions,objects}.rs`,
    `parser/definitions.rs`. Each becomes `database.trie(c, key)`, and every one of them
    is a site **6b has to touch anyway** to wire the keyword. They are not separate work.
  - **2 are small own arms**: `data.rs`'s `spatial<…>` renderer and `scopes.rs`'s
    dep-stripping for a keyed type. Both take a `Trie` twin.
  - **1 is the IR decoder** (`ir_schema.rs`'s `"Radix" =>` tag) — **6c**.
  - **1 is the range diagnostic** (`parser/fields.rs`: *"a `spatial` range is a
    COORDINATE slice"*). A trie must NOT inherit that refusal — its range is a prefix,
    which is the whole point — so this is where the step-5 operation attaches to the
    surface, and it is also the #799 message's other half.

  So the real remaining work is smaller than 141 suggested: one mechanical sweep, and a
  handful of sites 6b already has to visit.
- **6b — the parser surface. DONE.** `typedef.rs`'s keyword list, `definitions.rs`'s
  `"trie"` arm, `type trie;` in the stdlib, and `len(both: trie)`, with `Stores::trie`
  behind them. The audit's 13 own-arm sites landed with it, and three things the audit
  had NOT predicted came out of building it:

  - **The prefix surface is 6b's, not step 5's.** The audit called `parse_key`'s range
    diagnostic *"where the step-5 operation attaches to the surface"* and left it at
    that. It is a whole path: `parse_trie_slice` (a trie subscript is exact-or-prefix,
    and which one is only known after the key expression is parsed), `n_trie_prefix`,
    `build_trie_prefix_vec`, and the arm in both backends. `t[a..b]` is refused with
    the reason rather than silently answering a key interval, and the `spatial`
    COORDINATE-slice refusal now records that a trie's absence from it is deliberate.
  - **`fill.rs` is generated, and a new opcode shifts every one after it.** `len(trie)`
    added `OpLengthTrie`; without `make fill` the interpreter's `OPERATORS` table was
    one position off from `length_spatial` onward, and `for p in spatial<…>` — nothing
    to do with a trie — SIGSEGV'd on an unrelated program. The oracle that caught it
    was the INSTALLED binary answering correctly on the same file.
  - **Two silent alternations the sweep had already "handled".** A trie took
    `n_hash_sorted` for whole-collection iteration (the hash BUCKET walk over a radix
    tree), and its `-=` never reached `OpHashRemove`. Both sites name every keyed kind
    and both had been counted as mechanical.
- **6c — the IR codec. DONE.** `ir_schema.rs`'s JSON side landed without regeneration
  (self-describing, so encoder and decoder are both just the `"k"` tag). The store codec
  needed the generator: `TyTrie` and `PtTrie` in `tools/ir_schema/ir.loft`, `TY_TRIE = 26`
  / `PT_TRIE = 18`, a regenerated `ir_schema_gen.rs`, and the three stubs retired.

  **Appended, not slotted beside their Radix siblings** — a discriminant is the variant's
  POSITION in that enum, so inserting in the middle renumbers every variant after it. And
  `TyTrie` carries `key: text` rather than `TyRadix`'s `vector<NameRef>`: the shape says
  "exactly one" instead of leaving a round trip free to produce none or two.

  It also stopped being optional the moment 6b landed. `corpus_store_codec_round_trips`
  walks `tests/scripts/`, so 6d's own gate put a trie through the codec and the stub
  fired — the step ORDER was wrong, not the step. A codec stub survives only while
  nothing can construct the thing it decodes.
- **6d — the `.loft` gate. DONE** — `tests/scripts/801-trie-text-keyed.loft`, both
  backends, with the leak case step 4 could not write (a trie as a struct field, torn
  down with its container).

The `Type`-level silence is also the argument for keeping the keyword last: with no way
to declare a trie, all 141 are unreachable, so the audit can be done deliberately rather
than under a live-defect clock.

*Gate:* every operation the keyword exposes answers on both backends — declare, insert,
exact lookup, prefix range, iterate, free — against hand-computed values, with a
`sorted` control alongside so a regression cannot hide in "both are empty".

**Met.** Proven non-vacuous by breaking the prefix stop-check on purpose: `lonneker`
leaks into the `kerk` answer and the gate fails on that assertion by name.

*Beyond the parser, three consumers read a collection's KIND rather than its `Parts`* —
`LayoutDesc`, `type_of(…).collection`, and the lazy-store query deriver. `Iterated::Trie`
is exhaustive in all three, so each omission was a compile error — except in
`ffi_deliver`, whose `collect_keyed` is `#[cfg(target_arch = "wasm32")]` and so is dead
on the host that compiles the audit, and whose `rewrite_iterated` closes with `_ =>
continue`. A wasm-only silent skip is the residual this design's own instrument could
not see, and it is the same shape as *"a probe written in one platform's spelling"*.

### Step 7 — the diagnostics (loft#799) — DONE

Only now is the advice true: a text key under `spatial` says use `trie`, a numeric key
under `trie` says use `spatial` (or `sorted`/`index` for an order on a number).

Refused at DECLARATION, which is the half #799 asked for and the worse half to leave: a
refused range is a compile error fixed in seconds, while a point lookup answering NULL
is indistinguishable from "not found" at the call site. Pass 2 only, per loft#683 — pass
1 has an incomplete definition table by construction, so an element type declared further
down the file would read as unknown there.

Four `tests/parse_errors.rs` cases: the text-under-spatial and numeric-under-trie
mirrors, plus the trie's own arity (exactly one key) and its bare form.

## The residual

The axis this design cannot see is **scale**. Every measurement here is six words in a
test. The reason to want the kind at all — a 518 804-record name index — is where a
structure's real behaviour appears, and no gate above touches it. Step 6 should be
followed by a dogfood run against that data before the kind is documented as ready.

**Confirmed, and it is the only one left.** Every step above is built and gated, so what
remains is exactly the axis the design said it could not see. The half-million-record
run is the next move.

A second axis surfaced during the build and is worth recording because no probe here
would have found it: **a cfg-gated consumer is invisible to an exhaustiveness audit.**
`Iterated` is matched exhaustively in three places, which is why a missing kind is a
compile error — except `ffi_deliver::collect_keyed`, which is
`#[cfg(target_arch = "wasm32")]`, so the host that compiles the audit never sees it. The
instrument that catches this is not a better read but a target: the suite's own wasm32 +
wasip2 rlib builds. "Both backends" is not "every compile target".
