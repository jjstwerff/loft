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
| `index<T[k]>`, `radix<T[k]>` | own records | **ordinal**, key indexed | an index permits duplicates by definition |

Only `hash` earns its key as identity. For the rest the declared key becomes an
ordinary `KEY`, which is what it always was — an access path, not a name.

This is the over-unification failure exactly as predicted: the clean rule
absorbed four kinds that are not in the family, and it presented as elegance. No
amount of re-reading the prose would have caught it; one `INSERT` did.

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
| **S6** | sub-records, one kind per step | `vector<scalar>` → `vector<struct>` → `hash` → `sorted`. Each is one child table and one addressing rule |
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
