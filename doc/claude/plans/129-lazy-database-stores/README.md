<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN129 — Lazy database-backed stores

## Status

**Open — the FILE source is built, the DATABASE source is not.** A collection binds to a
`.store` image and faults on a miss (arc A), tells an unreachable source from a genuine absence
(C), pins that source so a traversal sees one world (D), releases what it holds on `= []` (E's
blunt form), and carries a graph consumer as its gate (F) — both backends, catalogued as
[`@F108`](https://github.com/loft-lang/features/issues/108).

**What the plan is NAMED for is still open:** arcs **B / B2 / B3 / B4**, deriving the query from
the store's own schema so the source can be a real relational database. `bind_lazy` takes a path
or URL to an image; no query is derived anywhere yet. Read the sub-arc table below for the
per-arc state — it is the authority, and this paragraph is a summary of it.

The design was pinned by probing and by
counting rather than by reasoning, and two of its drafts died that way: an image/page backing
(ruled out by the owner — the database holds real rows), and a `(type, key) → rec` identity map
(unnecessary — the resident collection already IS the cache). Both are recorded below with why,
because the reasons are the useful part.

**Issue:** [loft-lang/plans#129](https://github.com/loft-lang/plans/issues/129).

## Goal

A store can be bound to a database and read **lazily**: a lookup that touches a record which is
not yet resident fetches exactly that record, from a query **derived from the store's own
schema**, and every later touch is an ordinary read.

The motivating shape is a **graph, not a table** — persons, companies, and the employment
relations between them, history included. Reaching a person by two different paths must give the
SAME record, because traversal is the entire point of loading a graph lazily.

That history is **ordinary data**: a position row carrying its own `from`/`to` columns is a record
like any other, and loft never interprets a date. The example schema models time; loft does not.
Nothing in this plan may acquire a notion of "as of" from the shape of one test database.

**The backing is a REAL relational schema** (owner's decision, 2026-08-04): every loft record is
an actual database record, each `text` a varchar, each `integer` a column, and a keyed lookup is
served by an **index inside the database**. The database is not loft's private format — other
tools query it. That choice is what the rest of this document is shaped by, and it rules out the
image/page backing that an earlier draft assumed.

**And the DATABASE owns that schema** (same decision): loft binds to tables that already
exist and did not come from loft's types — it is not a schema manager. What must be declared,
what is checked at COMPILE time versus at BIND time, and what loft gives up for it:
[BINDING.md](BINDING.md).

## Effort + design

- **Effort:** H
- **Design:** ~ (the mechanism is pinned; the failure channel and the query mapping are not)
- **Last touched:** 2026-08-04

## The invariant

> **A record is present WHOLE or not at all, and the collection is the only thing that knows
> which — so a lookup that misses is the one place the outside world is consulted.**

Everything below either tries to falsify that sentence or pays for it.

## The resident set IS the cache, and it is already a loft collection

The store holds exactly the records fetched so far, which makes it automatically the cached data
set — so **the cache needs no structure of its own.** A `hash<Person[id]>` mapping key → record is
already "what have I fetched", and it starts empty and grows with what is touched.

That kills a side `(type, key) → rec` map before it is built, and with it the divergence hazard
that a second structure brings (a record evicted while its map entry survives). **Identity falls
out of the collection**: after the first fetch, every path to person 42 finds the one record the
hash holds.

It also corrects the obvious-but-wrong move of replacing the lookup with a query. loft's hash is
**not** the whole index — it is the resident subset — so walking it is cheap and right:

1. Look in the loft collection. **Hit → done: no query, no database, ordinary loft speed.**
2. **Miss → the source is consulted**: a schema-derived `SELECT`, materialise the record, insert
   it into the collection.
3. Every later lookup hits.

So there is one hook, and it is narrow: **the MISS path**. Not "the lookup is a query" — "a miss
asks before answering". A hit never leaves the process.

**Every fault is a collection lookup — there is no address-level fault.** An earlier draft had a
second kind, faulting when a not-yet-resident record was dereferenced. Arc A's first read killed
it, and the reason is worth keeping:

- **`valid()` returns `true` unconditionally in release.** Every check inside it is a
  `debug_assert!`, so it compiles out; a release `get_int` is `if rec != 0 { *addr } else { MIN }`.
  There is no per-read check today, and adding one would put a branch on the hottest path in the
  language — paid by every program, lazy store or not.
- **Nothing would ever hit it.** A reference at this boundary is a KEY, not a pointer
  ([BINDING.md](BINDING.md)), so following `person.employer` is a LOOKUP on the companies
  collection — fault kind 1 again — not a dereference of a dangling rec. Reading the FK itself is
  an ordinary field read of a record that is fully present.

So residency is **per record, all-or-nothing**, and "not resident" needs no representation in the
store at all: it is simply absence from the collection, which `find` already answers. Q1 — how to
encode a third block state cheaply enough for `valid()` — is not answered but **dissolved**, and
with it the cost on every read in every program.

The granularity that buys this is the relational row: a row is the unit, so a record is fetched
whole. The price is that a wide column (a blob) cannot be left behind — do not map one.

**The traversal IS the join.** Following a reference is another indexed lookup, so no join has to
be derived from the layout — which was this plan's sharpest open question and is now closed. The
price is round trips: one query per hop, the N+1 pattern by construction (see failure path 8).

## The one hook, and what it costs — measured, not chosen

**The address was the candidate, and it lost on measurement.** All 14 typed getters funnel through
`addr` behind `valid` — one site, and the native `#rust` bodies call the same accessors, so N would
have been 1 on both backends. It is not used, for the reason above: in release `valid()` is
`true`, so the check does not exist yet and creating it taxes every read in every program. The
counting is kept because it is what makes the rejection a decision rather than an omission.

**So the hook is the collection's MISS path, and that is where the exposure is.** It has no choice
— the database index has to serve the lookup — so the path must PAY rather than dodge. The cure is in the repo already: model laziness as a new `Parts` kind and follow
`DATABASE.md`'s own rule — *"spell the non-collection variants out; never close one of these
matches with `_` … the verbosity is the point — it turns 'someone must remember' into a compile
error."* That converts the five silent sites into five compile errors: the protocol's *make
omission loud*, since *collapse N* is unavailable here.

The exposure is real and worth stating in full.
[DATABASE.md § Adding or changing a collection kind](../../DATABASE.md) documents the per-kind
dispatch list — `get_keys`, `find` / `remove` / `remove_owned`, `set_keyed`, the parser lowering —
and states that an omission **"does not read as a missing feature"**. loft#720 was three such
omissions at once, each failing differently, and the interpreter and `--native` derive those lists
*separately*. A lazy collection kind has to be named at every one of them, and `N × silence` is
the brittleness — known now, before a line is written.

Two things keep it payable. **Read-only v1** means the mutating sites (`set_keyed`, `remove`,
`remove_owned`) do not need a lazy implementation at all — they need a loud refusal, which is one
line each and cannot be forgotten silently. And **exhaustive matches** turn the rest into compile
errors. What is left needing real work is `find`'s miss path and `get_keys`.

## Arc A — what reading the code settled before writing any

Three facts, each of which changed the build:

1. **`valid()` is `true` in release** — the address-level fault was rejected on this, and the
   second fault kind deleted with it (above).
2. **`Stores::find` has exactly TWO call sites, and both already hold `&mut Stores`** —
   `State::get_record` in the interpreter and `codegen_runtime`'s lookup for `--native`. `find`
   itself is `&self`, so it cannot materialise anything, but a `&mut` sibling can be dropped in at
   both sites without disturbing a caller. This is the chokepoint the design needs, and it exists.
3. **A miss is already spelled `rec: 0`** — every `Parts` arm returns that on a miss, so the fault
   point is a value the code already produces rather than a new state to invent.

**The first increment**, therefore: a `&mut` find that, on a `rec: 0` from a collection carrying a
binding, consults the source and retries — with the source being a FILE image, where
`store_load_key` already does the fetch-and-insert. That makes arc A a wiring job over machinery
that ships, which is exactly why the phase ordering put the file image before the database.

## What already exists — the probes that pinned this

| piece | mechanism | evidence |
|---|---|---|
| fault point | `Store::addr` / `valid` | all 14 typed getters funnel through it |
| placement at an image-chosen rec | **`Store::claim_at(pos, size)`** | shipped; the journal runs on it (`apply = claim_at + write`, `database/journal.rs`), and replay depends on it reproducing the exact extent |
| schema → query | **`LayoutDesc`** (`database/descriptor.rs`) | proven *sufficient*: `read_via_descriptor` reproduces `read_data`'s bytes driven ONLY by the descriptor (@PLN105); persisted in the `.dschema` sidecar |
| address-space reservation | the `store_persist_bind` shape | shipped — the store IS the file, resident only where touched |

**`claim_at` does not apply to this backing, and does not need to.** The probe was run for an
*image* backing, where the source carries loft rec numbers and faulting rec **R** can claim **R**,
making identity fall out of the address space. A relational schema has **no rec numbers** — it has
primary keys — so there is nothing to `claim_at`. Records are materialised by ordinary allocation
(`claim`), exactly as `store_load_key` already does, and **identity comes from the collection**
(above) rather than from the address space.

The finding is kept because it is true and cheap to re-reach: it is what an image-backed lazy
store (a `.store` file, an HTTP range source — [REMOTE_STORES.md](../../REMOTE_STORES.md)) would be
built on, and that remains a plausible second source. It is simply not this one's foundation.

**What must stay bounded is the collection itself**, since it is the cache. It grows with records
*touched*, not records *existing* — which is the property that makes this viable — but nothing
shrinks it, so arc E is a real arc and not a nicety. Evicting is coherent here in a way it would
not be with a side map: removing the record from the collection removes the cache entry, because
they are the same thing.

## Failure paths — written before the code, because this is where the rest of the invariant lives

1. **A failed fetch cannot raise, and a later success must not erase it.** C80: no runtime errors, ever. If an unreachable database
   answers `null`, then "no such person" and "the network is down" become indistinguishable **and
   unstable across runs** — the worst class, because it looks like data. The value channel cannot
   carry this; the failure belongs on a store-level channel a program can ask (the `store_verify`
   / `#errors` shape) — **shipped as `store_lazy_error`**.

   And it must be **STICKY**. The first version cleared on a later success, which recreated the
   very bug in a subtler place: a traversal whose first lookup could not reach the source and
   whose second could is MISSING data, and it reported healthy. Reachability now says nothing
   about what an earlier failure already lost. So a fault survives until `store_lazy_clear`
   acknowledges it, `store_lazy_faults` says how many (how incomplete), and a genuine absence does
   not clear it either.
2. **`len()` and iteration answer "what have I got", never "what exists" — and that is now
   SETTLED rather than a hazard.** An earlier draft of this line said the collection root "must
   carry the true length" and that iteration must stream or be refused. That was written before
   the cache insight and it contradicts it: once the collection IS the working set, the resident
   count is the honest answer and there is no "true length" for it to carry. The source's row
   count is a DIFFERENT question, and a program that wants it should ask the source (an explicit
   query), not a collection pretending to know.

   So `len(persons)` is the resident count and `for p in persons` walks what has been touched —
   history-dependent, on purpose, because the history is the working set. Arc A ships this and its
   test asserts it (`resident=1` then `2` from a three-entry image). The rule that keeps it honest
   is documentation, not machinery: a lazily-bound collection is a working set, and anyone reading
   `len` as a population count is asking the wrong object.

3. **Snapshot — a traversal sees ONE world, or is told it did not.** *(arc D, shipped for a file
   source.)* A store *is* a consistent image; a live source moves. Two faults at different
   points in a traversal seeing different worlds breaks the invariant directly, so the binding has
   to pin a read snapshot (a transaction, an MVCC point, or a source that is simply immutable).
   **This is about the database mutating under a reader — nothing to do with time-modelling in the
   data.** Records that carry their own validity dates (`from`/`to` columns on an employment row)
   are ORDINARY records: loft loads them like any other and never interprets a date. Whether the
   application models history is the application's business, and this plan must not acquire a
   notion of "as of" from the shape of one example schema.
4. **Writes.** Read-only for v1; a write to a lazily-backed record is refused loudly. Silently
   diverging from the source of truth is the failure this design exists to avoid.
5. **Unbounded working set — the blunt form already works.** *(arc E, measured.)* Faults only ever
   add, so the working set needs a way down. `persons = []` is it, and three things about it were
   MEASURED rather than assumed:

   - It **reclaims**: 2000 entries went from `records 4011, data 97%` to `records 3, data 6%`.
   - The **binding survives** — it is keyed by the collection root, which the assignment reuses —
     so the next lookup re-faults. Emptying does not silently unbind.
   - A **held reference survives with its value**, because the deps system keeps a referenced
     record alive while the rest is reclaimed. Checked properly: allocating 500 fresh records over
     the freed space left the held value unchanged, so it is a live record and not a stale read.
     That is what makes eviction safe to offer at all.

   All three are now pinned by the regression test, because none of them was asserted anywhere and
   each would regress silently.

   **What is NOT done** is a *bounded* working set — evicting selectively (keep N, drop
   least-recently-used) rather than all-or-nothing. That needs a policy owner: loft has no notion
   of "recently used" on a collection, and adding one is a per-record cost paid by every
   collection. The blunt form is enough for a working set the program drops at a known boundary;
   the selective form should wait for a consumer that actually needs it (arc F), so the policy is
   chosen against a real access pattern rather than invented.

   One inconsistency worth recording while it is fresh: an explicit single-entry removal
   (`coll[k] = null`) makes a held reference read `null`, while clearing the whole collection
   leaves it valid. Both are defensible in isolation; that they differ is a fact about loft, not
   about this plan.
6. **Cycles.** person → company → person is the normal case, not the edge. Faulting on
   dereference terminates naturally (a resident record is never re-fetched); any design that
   eagerly follows pointers does not.
7. **The `#c` boundary bypasses the fault site — measured.** `output_c_direct_call`
   ([`generation/mod.rs`](../../../../src/generation/mod.rs)) hoists a RAW base pointer for a
   `vector<T>` argument and hands C `(pointer, count)`; its own comment pins the contract — *"the
   elements live in a loft store the allocator may grow, so this pointer is valid FOR THE CALL and
   no longer."* Every element C touches after that skips `valid()` entirely, and @PLN128 measured
   that C **writes back** through it and loft sees the result. Under laziness that write-back is
   the sharper hazard: a store into a non-resident region lands nowhere, or on the wrong bytes.
   **A lazily-bound store's vector must be forced resident before it crosses to `#c`, or the
   crossing must be refused.** This is the one place the invariant is escaped by design rather
   than by omission, which is why it belongs in the contract instead of in a guard.
8. **Round trips, not bytes — the N+1 pattern by construction.** One indexed query per hop means
   walking 500 persons' employers is 501 queries. This is the honest cost of lazy traversal and it
   decides whether the feature is *usable*, not merely correct. It is why the matrix row
   `queries issued == records touched` is the load-bearing one, and why a batching form (fault a
   set of keys as one `WHERE id IN (…)`) is part of the **contract** rather than a later
   optimisation: without it, the natural way to write a traversal is the pathological one.
9. **Schema drift — a failure mode the paged sibling does not have.** The relational backing is a
   FOREIGN schema owned by someone else: a column renamed, a type widened, an index dropped. A
   dropped index is the cruel one — nothing breaks, every lookup silently becomes a table scan,
   and the feature degrades from lazy to catastrophic without a single wrong answer. The binding
   has to state what it requires of the schema and check it once at bind, the way
   `layout_gate_ok` already gates the paged loader.

## Why this is NOT the paged/HTTP sibling

Worth stating plainly, because the two look alike from a distance and the resemblance is
misleading. [REMOTE_STORES.md](../../REMOTE_STORES.md) and `store_load_key` already read a store
lazily from far away — but there, **loft's own data structures survive the boundary**:

| | paged image (`store_load_key`, HTTP range) | relational rows (this plan) |
|---|---|---|
| what is remote | loft's OWN image — same bytes, same layout | a foreign schema — tables, columns, SQL types |
| unit of fetch | a page (byte range), by offset | a row, by key |
| how a lookup is answered | **loft walks its own hash**, fetching the pages that walk touches | **the database's index answers it**; loft's hash is not there |
| forming the request | `Range: bytes=X-Y` — no schema needed | SQL, derived from the descriptor |
| identity | rec numbers are the image's → exact, free | no rec numbers → needs a key→rec map |
| the far end | does nothing; a static file with `Range` support | parses SQL, plans, uses indexes |
| semantics | faithful by construction — it IS the image | **reconstructed** from a foreign schema |

The last row is the one that costs. In the paged model `len`, iteration and ordering are simply
correct, because the bytes are loft's. Here every one of them has to be re-derived from a schema
that owes loft nothing — which is why failure paths 2 and 9 exist at all and have no analogue
over there. A static image also cannot drift or mutate under the reader; a live database does
both.

## Composition matrix — Stage A

Each cell asks the same two questions: does the lazily-read value equal the eagerly-read one, and
is the **number of queries** the one the design predicts?

| axis | cells |
|---|---|
| field kind | inline scalar · narrow int · `text` · nested struct · inline vector · `vector<struct>` · `DbRef` |
| reach | direct key hit · one hop (person→company) · two hops · cycle back to the origin |
| identity | the same record via key lookup, via an explicit `LIKE`, and via navigation from a company → all three `is_same` |
| collection kind / query shape | `hash` equality · `sorted` range · `index` composite+ordered · `spatial` · iteration — each must derive the query its kind implies, and a kind with no mapping must refuse rather than scan |
| absence | key absent · fetch fails · fetch times out — each must be **distinguishable** |
| count | queries issued == records **touched**, not records reachable |
| backend | `--interpret` vs `--native` (they derive key handling separately) |

The count row is what keeps the feature honest: a lazy read that fetches the transitive closure is
an eager read with extra steps.

## Sub-arcs

| Item | Status |
|---|---|
| **A** — the miss path: a bound collection fetches on a miss (`store_bind_lazy`) | **shipped** — file source, both backends |
| **B** — schema→query from `LayoutDesc`: equality from `hash`, ranges from `sorted`, composite+ordered from `index`; nothing enumerated ahead of time | Open |
| **B2** — the explicit escape hatch: run a query, materialise rows INTO the collection (what the keys cannot express) | Open |
| **B3** — the declared mapping + the `T: DbKeyed` bound + the bind-time schema/index check ([BINDING.md](BINDING.md)) | Open |
| **B4** — collection-valued fields as owner-parameterised queries (`company.people`) | Open |
| **C** — the C80-compatible failure channel (`store_lazy_error` / `_faults` / `_clear`), and `len`/iteration honesty | **shipped** — unreachable told from absent, faults STICKY across a later success, both backends; `len` settled as the resident count |
| **D** — a pinned source, so a traversal sees one consistent world | **shipped** for a file source: pinned at bind, drift REFUSED and reported through arc C. A database source pins a transaction instead — the one case where consistency can be provided rather than checked |
| **E** — eviction / a bounded working set | **partly shipped** — the blunt form (`= []`) reclaims, keeps the binding and preserves held refs, all now pinned by test. Selective/bounded eviction deferred to arc F, where a real access pattern can choose the policy |
| **F** — a real consumer: the persons / companies graph | **shipped against a FILE source** — identity across the graph and fetches == records touched, both backends. The DATABASE consumer waits on arc B's implementation |

## Phase ordering

1. **A first, against a FILE image rather than a database.** The residency mechanism is the risk;
   the source is not. If a store can be bound to its own `.store` image and faulted through
   `claim_at`, the design is proven and SQL becomes a second source rather than a new mechanism.
2. **C immediately after A** — before any consumer exists. A failure channel retrofitted once
   callers already depend on `null` is a breaking change.
3. **B** — the descriptor already drives a foreign reader; the work is issuing a query instead of
   reading bytes.
4. **D**, then **E**.
5. **F last, and it is the gate.** Until a real graph traverses lazily with the query count the
   matrix predicts, this is a hypothesis.

   **Passed, against a file source.** `persons -> employer -> companies`, both backends:
   one hop fetches one person and one company; a SECOND person at the same company leaves the
   company count at 1 (the hop hit the working set) and `c1 == c2` — **one record, two paths**,
   which is the identity the whole design rests on; a different company does fetch. Three hops
   cost 3 persons + 2 companies = 5 fetches, and the person and company nobody asked for stay
   out. Falsified by making every lookup re-fetch: the counts went red, the values did not — which
   is exactly why the counts are the assertions that matter.

   What this does NOT yet prove is the same traversal over SQL, which is arc B's implementation.
   The shape is proven; the source is not.

## Open design questions

1. **How is "not resident" represented?** A free block already has a negative header and a claimed
   record a positive one. A third state must be *cheap to test in `valid()`* — the hot path of
   every read in the language — and impossible to confuse with either. **The most load-bearing
   unknown.**
2. ~~**Does `--native` route reads through the same `Store` accessors?**~~ **ANSWERED — yes.**
   The native `#rust` bodies for record field reads call the same accessors the interpreter does
   (`stores.store(&db).get_int(db.rec, db.pos + fld)`, and the `get_byte` / `get_float` /
   `get_str` peers), all of which funnel through `addr`/`valid`. No inlined byte reads, no second
   path. **N = 1 holds on both backends.** The falsification attempt did find one bypass — the
   `#c` argument crossing — and it is failure path 7 above rather than a second fault site.
3. **Reserving the image's address space.** `claim_at` refuses to run past the store end, so the
   store must span the image up front. Address space is cheap; the question is whether a bound
   store can be created at a declared size without materialising it.
4. ~~**What is a query, exactly?**~~ **ANSWERED** — one table per type, columns from the
   descriptor's fields, `WHERE` from its key fields; a reference field is a foreign key followed by
   a further lookup. **No join has to be derived**, because the traversal is the join. What remains
   open underneath it is narrower: what names the table and the columns when a loft field name is
   not a legal SQL identifier, and who owns that mapping when the schema was not created by loft.
5. **Does eviction break the invariant?** Making a resident record non-resident again invalidates
   nothing structurally (the rec stays claimed) but changes an answer's latency mid-traversal.
   Probably fine; needs saying out loud.

## Cross-arc dependencies

- **@PLN105** — `LayoutDesc` and its sufficiency oracle; arc B is its second consumer after the
  browser bridge.
- **@PLN127** (closed) — type reflection reads the same descriptor from loft code, and has
  landed. Arc B is the second reader; once it lands the schema has one home and two readers.
- **@PLN23 / @PLN24** — where a SQL source and the `#c` bindings come from.
- **@PLN97** — the layout contract the descriptor is pinned against; a lazy image must not become
  a second description of the same layout.

## The documents

- **README.md** (this file) — the model: the invariant, the cache, the fault kinds, the failure
  paths, the matrix and the arcs.
- **[QUERIES.md](QUERIES.md)** — what a binding can ASK: collection kinds as query shapes, the
  explicit escape hatch, and one-record-however-it-arrived.
- **[BINDING.md](BINDING.md)** — binding to a schema loft does not own: the declared mapping, the
  `T: DbKeyed` compile-time check, the bind-time schema check, and what loft gives up.

## See also

- [`src/store.rs`](../../../../src/store.rs) — `addr` / `valid` / `claim` / `claim_at`.
- [`src/database/journal.rs`](../../../../src/database/journal.rs) — the shipped `claim_at` consumer.
- [`src/database/descriptor.rs`](../../../../src/database/descriptor.rs) — the schema.
- [REMOTE_STORES.md](../../REMOTE_STORES.md) — the sibling that reads a store lazily over HTTP range.
- [DATABASE.md](../../DATABASE.md) § Adding or changing a collection kind — why the fault is not at
  the lookup.
