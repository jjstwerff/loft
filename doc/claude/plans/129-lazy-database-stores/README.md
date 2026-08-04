<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN129 — Lazy database-backed stores

## Status

**Open — designed, not built.** Nothing is implemented. The design was pinned by probing and by
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

> **A record that is not yet resident is indistinguishable from one that is, except in latency —
> because residency is decided where `(rec, fld)` becomes an address, and nowhere else.**

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

**Two fault kinds follow from that:**

1. **A keyed lookup that misses** — `persons[42]` not yet resident. The collection-level hook
   above; its exposure is priced below.
2. **A reference-field dereference** — `person.employer`. The column holds a foreign key, so
   touching it becomes a lookup on the *companies* collection, which is fault kind 1 again one
   level down. This is where the address-level N = 1 measurement applies.

**The one genuinely new question this raises: a miss that is a real absence.** If person 42 is not
in the database either, every lookup re-queries forever, because "absent" and "not yet fetched"
are the same state in a collection that only records what it HAS. That needs a negative entry, or
an accepted re-query cost, and it must be decided rather than discovered.

**The traversal IS the join.** Following a reference is another indexed lookup, so no join has to
be derived from the layout — which was this plan's sharpest open question and is now closed. The
price is round trips: one query per hop, the N+1 pattern by construction (see failure path 8).

## Where the address-level fault belongs — measured, not chosen

**At the address (N = 1).** All 14 typed getters on `Store` (`get_int`, `get_byte`, `get_str`, …)
funnel through `self.addr(rec, fld)` behind `self.valid(rec, fld)`. One site decides residency,
below every collection kind. **Verified on both backends**: the native `#rust` bodies call the
same accessors (`stores.store(&db).get_int(db.rec, db.pos + fld)` and peers), so there is no
second path.

**Not at the lookup, wherever there is a choice (N ≥ 5, and silent).** Fault kind 1 has no choice
— the database index has to serve it — so that path must PAY for the exposure rather than dodge
it. The cure is in the repo already: model laziness as a new `Parts` kind and follow
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

1. **A failed fetch cannot raise.** C80: no runtime errors, ever. If an unreachable database
   answers `null`, then "no such person" and "the network is down" become indistinguishable **and
   unstable across runs** — the worst class, because it looks like data. The value channel cannot
   carry this; the failure belongs on a store-level channel a program can ask (the `store_verify`
   / `#errors` shape).
2. **`len()` and iteration.** `for p in persons` over a lazily-bound hash is a full table scan,
   and `len(persons)` is either the resident count or the real one. Both are silently defensible
   and wrong half the time. The collection root must carry the true length, and iteration must
   stream or be refused on a lazy root — never answer from the working set.
3. **Snapshot.** A store *is* a consistent image; a live database moves. Two faults at different
   points in a traversal seeing different worlds breaks the invariant directly, so the binding has
   to pin a read snapshot (a transaction, an MVCC point, or a source that is simply immutable).
   **This is about the database mutating under a reader — nothing to do with time-modelling in the
   data.** Records that carry their own validity dates (`from`/`to` columns on an employment row)
   are ORDINARY records: loft loads them like any other and never interprets a date. Whether the
   application models history is the application's business, and this plan must not acquire a
   notion of "as of" from the shape of one example schema.
4. **Writes.** Read-only for v1; a write to a lazily-backed record is refused loudly. Silently
   diverging from the source of truth is the failure this design exists to avoid.
5. **Unbounded working set.** Faults only ever add. `store_reclaim` gives a store's tail back, but
   a lazy store needs eviction or an explicit release — and eviction re-opens residency, so it
   touches the invariant directly.
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
| **A** — residency at `addr`/`valid`: represent "not resident", fault, fill via `claim_at` | Open |
| **B** — schema→query from `LayoutDesc`: equality from `hash`, ranges from `sorted`, composite+ordered from `index`; nothing enumerated ahead of time | Open |
| **B2** — the explicit escape hatch: run a query, materialise rows INTO the collection (what the keys cannot express) | Open |
| **B3** — the declared mapping + the `T: DbKeyed` bound + the bind-time schema/index check ([BINDING.md](BINDING.md)) | Open |
| **B4** — collection-valued fields as owner-parameterised queries (`company.people`) | Open |
| **C** — the C80-compatible failure channel, and `len`/iteration honesty | Open |
| **D** — a pinned read snapshot, so a traversal sees one consistent world | Open |
| **E** — eviction / a bounded working set | Open |
| **F** — a real consumer: the persons / companies / positions graph | Open |

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
- **@PLN127** — type reflection reads the same descriptor from loft code. If both land, the schema
  has one home and two readers.
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
