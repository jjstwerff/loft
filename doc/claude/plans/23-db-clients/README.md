<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 23 — database clients: a uniform API over the C libraries

## Status

**@PLN23 `status:active`** — S1–S3 built and in the repo; S4+ (the object mapping) is
designed, not built. Two language gaps it surfaced are tracked separately as @PLN124 and
@PLN125. Depends on @PLN24 (`#c`), also active.

## Goal

A MariaDB, PostgreSQL and sqlite client that calls the
platform's own C libraries through `#c` (@PLN24) — **no Rust crate, no `rustc` in the
library**. It is also the first serious consumer of `#c`, which is why the two plans move
together.

## What is built

`tests/fixtures/sqldb/` — `sql/` (the interface) plus `sqlite/`, `postgres/`, `maria/`,
driven from `tests/native.rs`.

The whole claim is two functions in `uniform.loft`: `seed` and `dump` are generic over
`SqlDb`, **never name a backend, and every backend runs them unchanged**. If the uniform
API were a fiction, those two could not exist.

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
| S4+ the object mapping | see [OBJECT_MAPPING.md](OBJECT_MAPPING.md) |

## The documents

- **[OBJECT_MAPPING.md](OBJECT_MAPPING.md)** — how a loft object with sub-records
  (vector, hash, index, array) becomes rows, and the S1–S7 ladder. Also says what the
  mapping does **not** cover yet.
- **[LIFETIME_AND_PROCEDURES.md](LIFETIME_AND_PROCEDURES.md)** — a drop at scope end
  (Part 1, now **@PLN125 arc B**), transactions (Part 1b), and procedures written as text
  (Part 2).
- **[INTERPOLATION_HOOK.md](INTERPOLATION_HOOK.md)** — the language change that makes a
  safe builder possible at all, now **@PLN124**.

## The ordering that matters

**T1–T3 (begin / commit / rollback, and nesting REFUSED) land before S5.** Writing a
collection non-atomically is not a smaller step, it is a wrong one — and a `db_begin`
inside a transaction that silently no-ops is the dangerous version, because the inner
"rollback" would discard the outer transaction's work.

## What loft itself was missing

Building this measured four gaps between a library type and a built-in one — recorded in
[INTERFACES.md § How first-grade a library type is](../../INTERFACES.md). Two are tracked:
**@PLN124** (the parts of an interpolation — the only one whose absence makes a library
*unsafe* rather than awkward) and **@PLN125** (associated types, the scope-end hook,
indexing). Neither belongs to @PLN23: the DB library is their first consumer and
motivating case, not their owner.

## Bugs this plan surfaced

loft#733 (a `text?` interface method through a bounded generic) and loft#734 (a
void-returning one) — both fixed, both with regression scripts under `tests/scripts/`
that were verified to fail on the released binary first.
