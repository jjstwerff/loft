<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 23 — database clients: a uniform API over the C libraries

## Status

**@PLN23 `status:active`** — S1–S4 and T1–T3 built and in the repo; S5+ (the object
mapping) is designed, not built. Of the two language gaps it surfaced, **@PLN124 is now built** and S4
is its first consumer; @PLN125 is not. Depends on @PLN24 (`#c`), also active.

## Goal

A MariaDB, PostgreSQL and sqlite client that calls the
platform's own C libraries through `#c` (@PLN24) — **no Rust crate, no `rustc` in the
library**. It is also the first serious consumer of `#c`, which is why the two plans move
together.

## What is built

`tests/fixtures/sqldb/` — `sql/` (the interface) plus `sqlite/`, `postgres/`, `maria/` and
`duckdb/`, driven from `tests/native.rs`.

The whole claim is three functions in `uniform.loft`: `seed`, `dump` and `bound` are
generic over `SqlDb`, **never name a backend, and every backend runs them unchanged**. If
the uniform API were a fiction, those three could not exist.

**sqlite is the cell that keeps it honest** — it needs no server, so a machine with no
database still proves the interface, the bindings, the shim loft compiled, and the NULL
crossing. postgres and maria are conditional and print `SKIP`; a skip is never counted as
a pass. The seeded rows are `1 'ada'` / `2 NULL` / `3 ''` because the empty string and SQL
NULL are **not** the same thing, and preserving that distinction is most of why a binding
exists at all.

| step | state |
|---|---|
| S1 loft calls `libmariadb`, versioned soname links | done |
| S2 the handle round-trips, C's own error comes back with it | done |
| S3 a real cursor, and NULL is not the empty string | done |
| S4 prepared statements, on all four backends | done |
| T1–T3 begin / commit / rollback, and nesting REFUSED | done |
| S5+ the object mapping | see [OBJECT_MAPPING.md](OBJECT_MAPPING.md) |

## S4 — a value cannot become syntax

The statement is built by **loft's own format strings**, not by writing `?`
placeholders:

```loft
q: SqlText = "SELECT id FROM loft_p WHERE name = {name}";
d.db_rows(q)
```

The parser hands `SqlText` the literal/hole boundary it already knows (@PLN124):
the author's bytes go through `lit`, `name` goes through `hole_text`. **The only
path into the statement text is `lit`, and `lit` is only ever called with bytes
from the source file** — so a value has no route into SQL syntax, by construction
rather than by discipline.

That is also why there are no placeholders to count. Writing `?` would mean
re-deriving, in a second place, a boundary the parser already had — and a `?`
inside a quoted literal turns that second derivation into a SQL-parsing problem.
This design does not have that class of bug: each backend joins the chunk list
with its OWN placeholder (`?` for sqlite/mariadb/duckdb, `$1 $2 …` for postgres),
and no backend ever searches SQL it did not write.

**Every backend cross-checks the two derivations.** The driver's own parameter
count (`sqlite3_bind_parameter_count`, `PQnparams` after `PQdescribePrepared`,
`mysql_stmt_param_count`, `duckdb_nparams`) is compared against the count the
parser produced, and a disagreement fails the prepare rather than binding a
short list.

**The shims own MEMORY; loft makes every library call.** `mysql_stmt_bind_param`
and `mysql_stmt_bind_result` take arrays of structs, and `PQexecPrepared` takes a
`char *const *` — layouts loft cannot express. The shims provide exactly those,
and reference not one library symbol, which is what keeps them compilable with
`cc` on a machine that has never had the library (the property @PLN24 arc G
depends on).

`maria/src/stmt.c` hand-declares `MYSQL_BIND` because a consumer machine has the
runtime library, not the headers. That declaration was **verified, not recalled**:
compiled against the authoritative `mariadb_stmt.h` comparing `sizeof` and
`offsetof` field by field — 112 bytes, all 19 offsets equal. MariaDB's layout is
not MySQL's (`flags` where MySQL has `param_number`), which is why guessing was
not an option; `libmariadb.so.3` in the manifest is what pins the ABI it matches.
The procedure is committed beside this file —
re-run it with `apt-get download libmariadb-dev` when the connector major changes.

### What S4 proves, and how it cannot pass vacuously

One generic `bound<D: SqlDb>` in `uniform.loft` names no backend. Every backend
returns the identical line:

```
p=2 [ada] <null> [] ['); DROP TABLE loft_p; --] hit=4 big=1000
```

- `['); DROP TABLE loft_p; --]` — spliced into SQL this closes the `VALUES` list
  and drops the table; bound, it is stored verbatim. **That the following SELECT
  returns rows at all is the proof the table survived.**
- `hit=4` — the same hostile text finds its own row by EQUALITY, which it could
  only do by arriving intact as data.
- `big=1000` — a value far past the 256-byte buffer mariadb's result binds start
  with, round-tripped at full length: the truncation re-fetch is exercised, not
  merely written.

The gate was proven to FIRE: replacing sqlite's bind path with a faithful
concatenating one (integers bare, text quoted, NULL as the keyword) fails the
attack cell loudly.

### Named gaps

- **duckdb is unproven here.** libduckdb is not installed on the machine this was
  built on, so its cells were read and never run, and the fixture prints SKIP.
  Said plainly rather than left to look covered.
- **A float binds as TEXT** on every backend. `sqlite3_bind_double` takes a double
  by value, which travels in an SSE register the fixed caller does not write
  (@PLN24), and a shim wrapping it would have to link the library. Precision is
  therefore whatever loft's float→text rendering gives — the exact `NUMERIC`
  answer is still an open item above.
- **One statement per connection**, and one parameter array per process: the
  shims' slots are static. The same single-slot limit S1–S3 already carried.
- **duckdb's transactions are unproven** for the same reason its S4 cells are —
  libduckdb is not installed here, and it is the one backend of the four whose
  keyword differs (`BEGIN TRANSACTION`).

## The documents

- **[OBJECT_MAPPING.md](OBJECT_MAPPING.md)** — how a loft object with sub-records
  (vector, hash, index, array) becomes rows, and the S1–S7 ladder. Also says what the
  mapping does **not** cover yet.
- **[LIFETIME_AND_PROCEDURES.md](LIFETIME_AND_PROCEDURES.md)** — a drop at scope end
  (Part 1, now **@PLN125 arc B**), transactions (Part 1b), and procedures written as text
  (Part 2).
- **[INTERPOLATION_HOOK.md](INTERPOLATION_HOOK.md)** — the language change that makes a
  safe builder possible at all. Built as **@PLN124**; what shipped is in
  [plans/124-interpolation-hook.md](../124-interpolation-hook.md).
- **[mysql-bind-layout-check.c](mysql-bind-layout-check.c)** — the check that keeps the
  hand-declared `MYSQL_BIND` in `maria/src/stmt.c` honest against the real header.

## The ordering that matters

**T1–T3 (begin / commit / rollback, and nesting REFUSED) land before S5.** Writing a
collection non-atomically is not a smaller step, it is a wrong one — and a `db_begin`
inside a transaction that silently no-ops is the dangerous version, because the inner
"rollback" would discard the outer transaction's work.

## What loft itself was missing

Building this measured four gaps between a library type and a built-in one — recorded in
[INTERFACES.md § How first-grade a library type is](../../INTERFACES.md).

**@PLN124** — the parts of an interpolation, the only one whose absence made a library
*unsafe* rather than awkward — is **built**, and S4 is its first consumer. The remaining
three (associated types, the scope-end hook, indexing) are **@PLN125**. Neither belongs to
@PLN23: the DB library is their first consumer and motivating case, not their owner.

## Bugs this plan surfaced

loft#733 (a `text?` interface method through a bounded generic) and loft#734 (a
void-returning one) — both fixed, both with regression scripts under `tests/scripts/`
that were verified to fail on the released binary first.
