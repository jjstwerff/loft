<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 23 — a canonical SQL shape for a loft object

How a loft value with sub-records — `vector`, `hash`, `sorted`, `index`, nested
structs — becomes tables and rows, and comes back unchanged.

Written before the code, because the failure paths are where the invariant
surfaces. One claim in here is already **falsified by measurement**, and it was
the cleanest one.

## The instance the design is read off

Not reasoned from the type system downward — plotted for one concrete object and
run against a real MariaDB, because a round-trip is an exact-invariant domain:
the construction already exists and has to be recovered, not explored.

```loft
struct Tag   { const name: text, weight: integer }
struct Point { x: integer, y: integer }
struct Doc {
  const id: integer,
  title: text,
  origin: Point,             // inline struct
  scores: vector<integer>,   // inline scalar elements
  tags: vector<Tag>,         // inline struct elements
  seen: hash<Tag[name]>,     // own records, unique key
  rank: sorted<Tag[weight]>, // own records, ordered by key — NOT unique
}
```

```sql
Doc        (id PK, title, origin_x, origin_y)
Doc_scores (doc_id, ord, value,          PK (doc_id, ord))
Doc_tags   (doc_id, ord, name, weight,   PK (doc_id, ord))
Doc_seen   (doc_id, name, weight,        PK (doc_id, name))
Doc_rank   (doc_id, ord, weight, name,   PK (doc_id, ord), KEY (doc_id, weight))
```

Three facts fall out of the plot, and none of them needed inventing:

- **An inline struct has no identity, so it has no table.** `origin` flattens to
  `origin_x` / `origin_y`. Identity is what earns a row.
- **The field path lives in the TABLE NAME, not in a column.** `Doc_scores` and
  `Doc_tags` cannot collide, and a collection nested inside an inline struct
  (`Doc.origin.marks` → `Doc_origin_marks`) needs no extra key.
- **Every child table's primary key is (owner's key, the element's address).**

## The invariant

> **A loft value is a column of the nearest enclosing record that has identity.
> A value with its own identity is a row, keyed by (the owner's key, the address
> its collection guarantees to be unique).**

The second half is the whole design, and the word `guarantees` is what the probe
put there.

## What the probe falsified

The first version read *"loft's collection types already declare their address —
ordinal where they declare no key, the declared key fields where they do."* It
is elegant, it absorbs all six collection kinds under one rule, and it is
**wrong**:

```
INSERT INTO Doc_rank VALUES (7,3,'m'),(7,3,'n');
ERROR 1062 (23000): Duplicate entry '7-3' for key 'PRIMARY'
```

A `sorted<Tag[weight]>` may hold two elements of the same weight. So the split is
not *keyed vs positional*. It is **unique vs not**:

| loft kind | element records | address | why |
|---|---|---|---|
| `vector<T>`, `array<T>` | inline / referenced | **ordinal** | position is the only identity there is |
| `hash<T[k]>` | own records | **the declared key** | a hash key is unique by construction |
| `sorted<T[k]>`, `ordered<T[k]>` | own records | **ordinal**, key indexed | ordered BY the key, duplicates allowed |
| `index<T[k]>`, `trie<T[k]>` | own records | **ordinal**, key indexed | the key is not declared unique, so nothing may lean on it |
| `spatial<T[k…]>` | own records | **none** | a Morton order over several axes is not a SQL order |

Only `hash` earns its key as identity. For the rest the declared key becomes an
ordinary `KEY`, which is what it always was — an access path, not a name.

This is the over-unification failure exactly as predicted: the clean rule
absorbed four kinds that are not in the family, and it presented as elegance. No
amount of re-reading the prose would have caught it; one `INSERT` did.

### What S6c measured, and the row it corrected

The table above was written from what the collection kinds MEAN. Building S6c
measured what they DO, one axis at a time on both backends — and one row was
wrong for a reason worth keeping:

| kind | equal key, only view of `T` | equal key, `T` has another keyed view |
|---|---|---|
| `hash` | replaces | **replaces** |
| `index` | replaces | **replaces** |
| `sorted` / `ordered` | replaces | **keeps both** |

- **`index` does not permit duplicates.** The old row said it did "by
  definition"; a loft `index` replaces on an equal key, in both configurations.
  The ANSWER stays the ordinal, and the reason changes: an address may rest only
  on a uniqueness the type DECLARES, and `index` declares none. A guarantee is a
  declaration, not an observed behaviour — leaning on the behaviour would put
  the schema at the mercy of an insert path nobody promised.
- **`sorted` really can hold two elements of one key**, so the sentence the
  falsifying `INSERT` put into this design is true. But it is true for a reason
  the design did not state: the duplicate survives only when the element type has
  ANOTHER keyed collection over it — merely DECLARED is enough, never
  instantiated. Two keyed collections over one element type are views of one
  record set (loft#843), so `sorted<Beat[bar]>` shows two elements of one bar
  because `hash<Beat[note]>` is what tells them apart.

That last point is the one to carry forward: **whether a `sorted` can hold
duplicates is not a property of the collection**. It depends on what else in the
program keys that element type, which another file can change. So the ordinal is
not merely the conservative answer for a non-hash collection — it is the only
answer a derivation is entitled to, because the alternative depends on a fact
outside the type it is deriving from.

A `hash` is unaffected: it enforces its key in both configurations, which is what
makes `AddrKey` sound.

### Two views, two child tables, the same records

The same measurement has a direct consequence for the mapping, and it surprises:
a record with `beats: sorted<Beat[bar]>` and `byname: hash<Beat[note]>` writes
**every one of its records into BOTH child tables**, addressed two different
ways — and adding to only one of the two collections fills both. That is not
duplication the mapping introduced; it is what the loft value is. `children_live`
measures it:

```
beats  1|0|1|y;1|1|1|x;1|2|2|z     ordinal-addressed, two elements of bar 1
byname 1|1|x;1|1|y;1|2|z           the same three, addressed by the other key
```

It belongs beside "Sharing is lost" below: a tree mapping cannot represent a
graph, and two views of one record set are the case where the copying is
visible within a single owner.

## The re-assertion sites, counted before any code

The address rule has to be restated by: (1) DDL generation, (2) the write path,
(3) the read path's reassembly, (4) schema migration when a struct changes, and
(5) each backend, if the mapping is per-backend. Omission is **silent at every
one** — wrong rows, or a reassembly that quietly drops or duplicates elements.

`5 × silence` is the whole risk, so the design collapses it to **one function**:
given a loft type, it answers *what addresses an element of this collection*. DDL,
write, read and migrate all consult that one answer; nothing re-derives it. A
backend never sees the question — the mapping is backend-agnostic, and only the
SQL dialect differs below it.

## Failure paths, and what each one costs

- **Ordinals are not stable under mutation.** Inserting at the front of a vector
  shifts every ordinal after it, so a write-through mapping rewrites O(n) rows for
  one insert. This mapping is therefore defined for **whole-collection writes**
  (replace the child rows for one owner) and read; incremental element mutation is
  a separate problem, not a free consequence.
- **Sharing is lost.** Two `Doc`s referencing one `Tag` record get two rows. A
  tree mapping cannot represent a graph. A `Reference(T)` becomes a foreign key
  only when the referent has identity of its own — i.e. it is a member of a
  `hash` somewhere. Otherwise it has no addressable home and is copied.
- **A struct with no key has no primary key.** `Doc` without `const id` needs a
  surrogate, and a surrogate is not stable across runs, so it cannot be the thing
  a `Reference` points at.
- **`text` as a key column needs a bounded type.** `PRIMARY KEY (doc_id, name)`
  requires `VARCHAR(n)`, not `TEXT`. The bound is a declaration the loft type does
  not carry today.

## Build ladder — small, safe steps

Each step lands green on its own, is useful on its own, and is verifiable before
the next begins. The schema design is deliberately **last**: the risky part is
the mapping, and it should be built on a binding that is already proven, not
alongside one.

| step | what it proves | how it is proved |
|---|---|---|
| **S0** | a scoped test server exists | `loft` user restricted to `loft_test*`; anything else is `ERROR 1044`. Tests self-skip when absent — **done** |
| **S1** | `#c` reaches libmariadb at all | **done** — `mysql_get_client_info() -> text`, zero rustc, both backends identical. Also closes **@PLN24 arc F** |
| **S2** | the handle lifecycle | **done** — `mysql_init` / `mysql_real_connect` / `mysql_close`; the handle round-trips and C's own error crosses back as `text` |
| **S3** | the cursor model | **done** — `mysql_query` / `mysql_store_result` / `mysql_fetch_row`, a real result set walked through a loft-built shim, SQL NULL distinct from `''` |
| **S3b** | one contract, several libraries | **done** — `SqlDb` over sqlite, postgres, mariadb **and duckdb**; one generic `dump` that never names a backend. duckdb is in the tree now rather than proven-and-discarded: `[c] optional-libs` (@PLN24 arc G) means the fixture builds and runs without its 70 MB `.so`, so keeping the backend costs nothing |
| **S4** | prepared statements | **done** — all four backends, both loft backends, one generic `bound<D: SqlDb>`. `MYSQL_BIND` is an array of structs and this is where the ANSI-C shim earned its keep (@PLN24 arc D). Statements are built by loft's own format strings (@PLN124), so there are no `?` placeholders for a caller to write or a backend to find |
| **T1–T3** | transactions | begin / commit / rollback on all three backends; nesting refused. See [INTERPOLATION_HOOK.md § Transaction ladder](INTERPOLATION_HOOK.md) — cheap, and S5 needs it |
| **S5** | a FLAT struct round-trips | **done** — one loft struct ↔ one table, written and read back, compared by content digest. `round_trip.loft` no longer lists the row's values by hand; `row_of` walks the DEFINITION's columns, each of which carries the byte position of the field that fills it, and reads them with `field_value`. See § S5 below |
| **S6a** | a `vector<scalar>` field | **done** — one child table, columns `(owner key, ord, value)`, and `element_address` is the one place the address rule is stated |
| **S6b** | a `vector<record>` field | **done** — the same address columns, and one column per stored field of the element. Written to sqlite and read back on both loft backends: § S6b below |
| **S6c** | a keyed sub-collection | **done** — `hash` addresses by its declared key and carries no ordinal; `sorted` / `index` / `trie` take the ordinal and their key gets its own index; `spatial` refuses. § S6c below |
| **S7** | the mapping generalises | the single address function drives DDL, write, read; migration on a changed struct |

S1–S3 are worth doing even if the mapping is never built: they are the proof
@PLN24 has been waiting for since arc F was written, on a real library rather
than a fixture.

## S5 — the row comes off the VALUE

`insert_row` renders the STATEMENT from the definition; S5 is the other half, the
VALUES. @PLN133 S13 stopped exactly here — *"the GENERIC struct walk stays @PLN23
S5: reflection reports types, not values"* — because reflection described a TYPE
and nothing could read a value's field. `field_value(x, position)` is the half
that was missing, and it is now in the stdlib beside `type_of`
([STDLIB.md § Reflection](../../STDLIB.md)).

**The walk is over the DEFINITION's columns, not the type's fields.** Each
`ColumnDef` already carries the byte `position` of the loft field that fills it,
so reading that byte produces the values in the order `insert_row` requires — by
construction, with no second traversal to keep in step and no count to get wrong.
`bound_of` is keyed on the COLUMN rather than on the reflected kind for the same
reason: `derive` already decided that a `character` is a text column, and
dispatching on the value's kind would be a second mapping free to disagree with
the first.

**It cannot be one generic function over the struct type, and that is a language
fact.** A generic body is parsed once against its type variable, so reflection is
refused there (a compile error, since answering would give an ORM an empty row).
The write half therefore sits where the concrete type is known — beside the lazy
READ driver (@PLN133 S8), which already sits there. Both halves are
per-element-type and neither names a column, which is the property S5 claims.

### The digest, and why it is not decoration

S5's step says "compared by content digest", and the digest earns its place
rather than restating what the gate already checked. `round_trip.loft`'s other
tokens name only grace's fields and alan's NAME. Measured: a driver that returns
`flag=false` for every row keeps every one of them — grace's flag genuinely is
false — and turns `second=alan touched=2 digest=true` FALSE. The digest is the
only channel that sees it.

It compares against the SOURCE list, never against the value under test: a
fallback to the record being checked would compare a thing with itself and pass
whatever the database did.

## S6b — the element's own columns, and where each value comes from

A `vector<Tag>` differs from a `vector<integer>` in the LAST columns only. The
address half — `(owner key, ord)` — is identical, which is what says the two are
one rule rather than two shapes that happen to look alike:

```sql
CREATE TABLE "doc_scores" ("doc_id" INTEGER NOT NULL, "ord" INTEGER NOT NULL,
                           "value" INTEGER NOT NULL)
CREATE TABLE "doc_tags"   ("doc_id" INTEGER NOT NULL, "ord" INTEGER NOT NULL,
                           "name" TEXT NOT NULL, "weight" INTEGER NOT NULL,
                           "note" TEXT)
```

**A child row is drawn from three places, and the COLUMN says which.** S6a could
leave every child column at `position = -1` because no field filled any of them.
With a record element that stops being true: the element's columns carry the
element's byte offsets, the owner's columns carry the owner's, and a writer that
told them apart by COUNTING the owner's key columns would be a second derivation
of a fact the definition already holds — the exact re-assertion this design
collapses. So `ColumnDef` carries a `ColumnSource`:

| `src` | the value is | `position` is |
|---|---|---|
| `SrcField` | `field_value(<this table's record>, position)` | that record's offset |
| `SrcOwnerKey` | `field_value(<the owner>, position)` | the OWNER's offset |
| `SrcOrdinal` | the element's index in the collection | nothing — no record holds it |
| `SrcElement` | the element itself | nothing — a scalar has no field |

One number with one meaning: `position` is always an offset into the record
`src` names. The write path is then one loop over the columns with no counting,
so a composite owner key needs no change to it — which is why the pure gate
carries a two-key owner.

### The failure paths peculiar to a record element

Both are silent without a check, and neither exists for a scalar element:

- **A name collision.** The address columns are named by the derivation and the
  element's by its author, so a `Tag` with a field called `ord` — or one called
  `doc_id` — produces a `CREATE TABLE` naming one column twice. Only the engine
  would notice, and only the engines that reject it.
- **A collection inside the element.** That is a grandchild table, addressed by
  `(owner, ord, ord)`. The rule extends to it; the derivation does not recurse
  yet, so it refuses and names S7 rather than dropping the field.

Both refuse the WHOLE answer rather than one table, for the reason the ladder
already gives: a partial schema builds a database whose own reader disagrees
with its writer.

### What the live gate measures

`tests/fixtures/sqldb/children_live.loft` writes three documents to sqlite
through this derivation and reads them back, byte-identical on both loft
backends. The data is chosen so a wrong rule reads DIFFERENTLY rather than
merely reading less:

```
docs   7|seven;9|nine;11|eleven
scores 7|0|10;7|1|20;11|0|30
tags   7|0|a|1|~;7|1|b|2|;9|0|a|1|x;9|1|a|1|x;9|2|c|3|~
```

- **Doc 9 holds the same tag twice.** Under the falsified key rule those are one
  row; under the ordinal rule they are rows 0 and 1. The refuted claim is now a
  test rather than a paragraph.
- **`a/1` is also doc 7's**, so an owner column that went missing MERGES two
  documents' children into one run instead of losing anything — a failure that
  reads as extra data, which is the kind a count would not catch.
- **Doc 11's tag vector is empty**, and doc 9's score vector is: a parent with no
  children is still a row.
- **`~` is SQL NULL beside `b`'s empty string.** Not the same value, and keeping
  them apart is most of why a binding exists at all.

One thing that had to be MEASURED rather than assumed: **an omitted `text?`
field is the empty string, not loft's null.** `Tag { name: "a", weight: 1 }`
meaning "no note" stores `''`, so the first version of the null cell compared two
empty strings and passed while testing nothing. The gate writes `note: null`
outright.

## S6c — the two keyed shapes, and the index a key still owes

The collection's KIND decides how many address columns a child table has, and it
is the only thing that decides:

```sql
CREATE TABLE "doc_seen" ("doc_id" INTEGER NOT NULL, "label" TEXT NOT NULL,
                         "score" INTEGER NOT NULL)
CREATE INDEX "doc_seen_by_owner" ON "doc_seen" ("doc_id" ASC, "label" ASC)

CREATE TABLE "doc_rank" ("doc_id" INTEGER NOT NULL, "ord" INTEGER NOT NULL,
                         "at" INTEGER NOT NULL, "what" TEXT NOT NULL)
CREATE INDEX "doc_rank_by_owner" ON "doc_rank" ("doc_id" ASC, "ord" ASC)
CREATE INDEX "doc_rank_by_key"   ON "doc_rank" ("doc_id" ASC, "at" ASC)
```

**A `hash` carries no ordinal.** Its address is the declared key, and those are
fields of the element — already columns. An ordinal there would be a second
identity for a row that has one.

**An ordinal-addressed collection still owes its key an index**, and that is the
half S6a and S6b silently left open. A `sorted` sub-collection was `AddrOrdinal`,
so it fell straight through to the record path and derived a table with the
address index and nothing on its key — no refusal, no index. @PLN129 refuses a
bind whose lookup no index serves, so that table was a database its own
collection's reader could not bind to. The `_by_key` index closes it.

The key index is named `_by_key` rather than after its columns, which is what a
parent's index does. Two indexes share a child table, and a fixed suffix cannot
collide with `_by_owner` whatever the element's fields are called — the
duplicate-index-name failure is removed rather than detected.

### Positions from two records in one table

A child table now carries `position` values from the OWNER and from the ELEMENT,
and finding the key column by position alone reads one as the other. This is not
a corner case: measured, `Doc.id` is at byte 0 and `Step.at` is at byte 0, so a
`sorted<Step[at]>` under a `hash<Doc[id]>` indexes `doc_id` and calls it the key
— a plausible index over the wrong column, and no index on the key at all. Byte 0
is where a struct's first stored field lives.

`ColumnSource` is what makes the lookup answerable: the search is restricted to
the columns whose value comes from the element. The same field that removed the
counting from the write path removes the ambiguity from the read path, which is
the sign it is at the right level.

### The index and the ORDINARY-vs-UNIQUE deviation

Every child index is ordinary, including a `hash`'s, where the pair genuinely is
unique. `IndexDef` cannot spell `UNIQUE` and `introspect` cannot read one back,
so rendering it would make a table loft MADE and a table loft FOUND differ in a
field — the single property @PLN133 rests on. The deviation stays recorded rather
than half-closed; closing it means teaching `introspect` to read uniqueness
first.

## S6 writes N graphs REGROUPED BY TABLE, not one graph at a time

The ladder above walks one owner's collections. A real write is 500 owners at
once — a `Doc` with five `rank` children, five hundred times — and the shape that
takes is **all 500 parents, then all 2500 children**, not parent-children-
parent-children.

This reads as a performance note and is not one. Per-object writes acquire locks
in **data-dependent order**, so two concurrent writers take the same rows in
different orders and deadlock; the retries are a different performance regime,
not a constant factor. Grouping by table gives every writer one order — parent
table, then child table — which is the ordinary deadlock-avoidance discipline.
On top of that, a child INSERT's foreign-key check takes a SHARED lock on the
parent row, so interleaving holds and re-takes 2500 parent locks across the
whole write instead of running them against a parent table that is already
complete.

The reads matter too, and they are the smaller half: a unique index cannot defer
its maintenance (InnoDB's change buffer covers non-unique secondaries only), and
every child table's primary key here is `(owner's key, address)` — unique by
construction. Batched, those lookups hit a hot, fully-built parent index;
interleaved, each one evicts what the next needs.

**Client-side keys are what make it possible.** Identity comes from the loft
value, never from the database — no `AUTO_INCREMENT`, no `RETURNING`. So all
2500 child rows are computable before the first statement is sent, and
parent-before-child is a foreign-key ordering requirement rather than a data
dependency that would force a round trip into the middle of the batch.

### What that means for the interface

`exec_many(rows)` is not what the mapping calls. It batches ONE statement over
many rows; the mapping needs a **write plan** that owns the grouping: collect
every pending graph's rows, group by table, order the tables parent-before-child,
chunk each group to the backend's parameter cap, and run the whole plan in one
transaction. `exec_many` becomes the executor of a single chunk underneath it.

Chunking is backend knowledge, like the `BEGIN TRANSACTION` keyword: PostgreSQL
takes at most 65535 bind parameters per statement and has `COPY` for exactly
this; MariaDB is bounded by `max_allowed_packet`. 500 parents x M columns is
comfortable, 2500 children x M is not always.

Atomicity is already in place — T1-T3 landed before this for the reason stated
there, and the whole plan is one transaction.

### Still to be measured

The shape is decided; the number is not recorded. Write the 500x5 graph both
ways — interleaved and grouped-by-table — on sqlite, postgres and mariadb, and
record wall-clock plus statement count.

Two reasons it is worth doing even though the design does not wait on it. It
becomes a REGRESSION guard: both shapes are correct, so nothing in the suite
would notice a later refactor quietly reverting to per-object writes. And it
says where the cost actually falls — sqlite has no foreign-key trigger machinery
and often runs with `PRAGMA foreign_keys` off, so if it shows little difference
that is worth knowing too: it means the shape is for the servers and sqlite pays
nothing for it.

## What S1 and S2 already changed

**The raw C surface is not the loft API surface, and loft says so itself.**
Declaring `mysql_real_connect` verbatim trips loft's own advice at the
declaration:

> `db_connect` takes 8 required parameters — every caller has to get all 8 right,
> in order. Parameters that travel together are usually one thing.

It is right, and it is the argument for the plan's `connect(opts)` shape rather
than a restatement of the C prototype. The `#c` layer stays a faithful binding of
what the library actually exports; the `sql` interface above it is where the API
is designed. Keeping those two apart is what stops the C API's ergonomics
becoming loft's.

**A NULL pointer argument needs a non-`text` parameter.** `mysql_real_connect`
takes `unix_socket` as `const char *` that is normally NULL, and loft `text` is
non-null with no way to spell a null pointer. It is declared `integer` (0 = NULL),
which the handle convention already allows in both directions. Any C parameter
that is "a string, or NULL" meets this — it is a recurring shape, not a one-off.

**The error path crosses.** `mysql_errno` / `mysql_error` bring C's own diagnosis
back as an `integer` and a `text` — measured: `errno=1045`, `Access denied for
user 'loft'@'localhost' (using password: YES)`. So the error taxonomy the plan
asks for has a real source; it does not need inventing.

## Beyond the ladder

[LIFETIME_AND_PROCEDURES.md](LIFETIME_AND_PROCEDURES.md) designs two things that
sit after S7 — a **drop at scope end** that ends a transaction, and **stored
procedures written with string formatting**. Both put a side effect where the
reader is not looking (one at a brace, one inside a string), which is what those
designs have to make safe. Design only; nothing built.

## What is NOT designed yet

The addressing rule above answers *"where does a sub-record go"*, and the ladder
carries it to a working mapping. It is **not** a finished database library. These
gaps are named here rather than discovered later:

**Types with no mapping.**
- **Data enums** (`Enum(_, true, _)` — a tagged union with per-variant payloads).
  Relationally that is the classic choice between one wide nullable table, one
  table per variant, or a discriminator plus a payload, and each loses something
  different. loft has them as a first-class type, so a mapping that cannot carry
  them is not finished.
- **Tuples** — anonymous and positional, with no field names to become columns.
- **`boolean` is THREE-state** (false / true / **null**, @PLN17). SQL `BOOLEAN`
  is two-state plus NULL, which happens to line up — but loft's null is a *value*
  (255), not an absence, so the round trip needs stating rather than assuming.
- `ChildRec` / stored `DbRef` — they exist in `Parts` and are unaddressed here.
- `float` / `single` precision across a `NUMERIC` column.

**Atomicity** — now designed, in
[LIFETIME_AND_PROCEDURES.md § Part 1b](LIFETIME_AND_PROCEDURES.md). One
object-graph write is one transaction; the mapping is unsound otherwise, because
a crash leaves a collection half written and the read path cannot tell. It
belongs in **S5/S6**, not S7: S6 writes collections, and writing them
non-atomically is not a smaller step, it is a wrong one.

**Concurrency is unaddressed.** Ordinals are not stable under mutation (noted
above), and two writers to one owner's child rows interleave. There is no design
for optimistic versioning or row locking.

**The reverse direction is absent.** Everything here maps a loft object INTO SQL.
Reading a schema someone else owns — the ETL half of the BROADENING.md gap —
needs SQL types to become loft types, a different problem with its own lossiness
(unsigned, `DECIMAL`, arrays, `JSONB`).

**Migration is one word in S7.** A struct that gains, loses or renames a field
needs a defined answer, and "the address function drives it" is not one.

None of these blocks S4–S6, which is why the ladder still stands: each of those
is provable on flat structs and single-kind collections without an answer here.
They do block calling the library finished.

## Open

1. A bounded-text declaration, for `text` used as a key column.
2. Whether `Reference(T)` maps to a foreign key at all, or is always copied.
3. Incremental element mutation, if whole-collection writes prove too coarse.
