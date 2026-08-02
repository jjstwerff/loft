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
| **S3b** | one contract, several libraries | **done** — `SqlDb` over sqlite, postgres and mariadb; one generic `dump` that never names a backend. duckdb proven too, not vendored (70 MB) |
| **S4** | prepared statements | `mysql_stmt_*`. `MYSQL_BIND` is an array of structs, so this is where the ANSI-C shim earns its keep (@PLN24 arc D) |
| **S5** | a FLAT struct round-trips | one loft struct ↔ one table, written and read back, compared by content digest |
| **S6** | sub-records, one kind per step | `vector<scalar>` → `vector<struct>` → `hash` → `sorted`. Each is one child table and one addressing rule |
| **S7** | the mapping generalises | the single address function drives DDL, write, read; migration on a changed struct |

S1–S3 are worth doing even if the mapping is never built: they are the proof
@PLN24 has been waiting for since arc F was written, on a real library rather
than a fixture.

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

**Atomicity is missing, and it is not optional.** The mapping is defined for
whole-collection writes — replace the child rows for one owner — which is several
statements. Without a transaction around them a crash leaves a collection half
written, and the read path cannot tell. Every write of an object graph is one
transaction, or the mapping is unsound.

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
