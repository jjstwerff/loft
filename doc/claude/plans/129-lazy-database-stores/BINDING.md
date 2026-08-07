<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN129 — Binding to a schema loft does not own

> **This is the design record, kept for its reasoning.** What is TRUE of the built system —
> the mapping, the schema and index check, what is refused — is in
> [LAZY_STORES.md](../../LAZY_STORES.md). Read this one for why it took that shape, and for the
> `T: DbKeyed` bound, which is still open work.

Who owns the tables, what has to be declared, what is checked **when**, and what loft gives up
for it. Companion to [README.md](README.md) (the closure record) and [QUERIES.md](QUERIES.md)
(what a binding can ask).

## The decision

**And the DATABASE owns that schema** (same decision): loft binds to tables that already exist and
did not come from loft's types. It is not a schema manager — no DDL, no migrations, no opinion
about what a `text` becomes on four engines. Two things follow, and they are the difference
between this and a loft-owned schema:

- **A declared mapping is an artefact, not an assumption.** `Person.name` may be `persoon.naam`;
  the primary key may be composite. Type ↔ table and field ↔ column have to be stated somewhere
  the descriptor alone cannot supply.
- **A bind-time check is mandatory, not optional.** The columns must exist with compatible types
  and the index a `hash`/`index` collection assumes must be present — verified once at bind, and
  the bind REFUSED otherwise. Failure path 9 stops being a caveat here and becomes the main risk
  surface: a missing index is not an error, it is a silent collapse from lazy to table-scan.
## What loft gives up to bind to someone else's schema

Binding to a foreign schema costs loft something, and it should be stated rather than discovered.
A `Person` has to carry the database's primary key as an ordinary field — `const id: integer` —
because the collection is keyed by it, other tables reference it, and every derived query puts it
in the `WHERE`. It is declared, never inferred.

**Most of that is honesty rather than compromise.** The row really does have an `id`, and other
tools really do use it; modelling it as data is describing what is there. `const` is the right
marker and already exists (@F12): a primary key is written once at construction and never after.

**The real cost is that two identities now coexist.** loft's identity is a `DbRef` — store, rec,
pos. The database's is the key. Nothing in the language ties them together, so the same row
materialised twice would be two records with equal `id`s and different `DbRef`s, and `is_same`
would answer false for what is obviously one person. That is not hypothetical: it is what happens
the moment any path materialises a record without consulting the collection first.

So the rule the whole design leans on has to be absolute rather than conventional: **the
collection is keyed by the DATABASE's key, and every arrival path asks it before materialising.**
That single rule is what keeps the two identities in agreement — and it is why *the collection is
the cache* is load-bearing rather than a convenience.

A second, smaller concession: **a reference is a KEY at this boundary, not a pointer.**
`person.employer` holds a foreign key value, and navigation resolves it into the companies
collection. Composite primary keys are fine (`hash<T[a, b]>`), which is the usual shape for
history tables anyway.

## Can the Id column be enforced? — compile time for our half, bind time for theirs

The question splits, and each half has a different answer.

**loft's half is a TYPE-SYSTEM check, and the existing generics already carry it.** An interface
is a set of bare method signatures, a generic takes `<T: Interface>`, and satisfaction is
**structural** — [INTERFACES.md](../../INTERFACES.md) is explicit that "no declaration is
required; any type that has all the required methods" satisfies a bound. So:

```loft
interface DbKeyed {
    fn db_key(self: Self) -> integer
}

fn bind_lazy<T: DbKeyed>(coll: hash<T>, table: text) -> boolean
```

A type with no key cannot be passed — a **compile error**, not a runtime refusal, with no
registration boilerplate. The author writes one line per bound type:

```loft
fn db_key(self: Person) -> integer { return self.id; }
```

That line does **double duty**, which is why this is the right tool rather than merely an
available one: it proves a key exists *and* it names WHICH field is the key. The descriptor
cannot supply the second — a struct with `id`, `company_id` and `year` has three integer fields
and no way to know which one the table is keyed by. The method is the mapping.

**Two limits, both honest.**

- *Composite keys.* `fn db_key(self: Self) -> integer` cannot express `(person_id, from)`, which
  is exactly the shape a history table has. Tuples exist (@F11) and would carry it, but the bound
  would then be per-arity. The clean answer is an **associated type** on the interface — which is
  @PLN125's first item and is not built. So: single-column keys work with what exists today;
  composite keys are a stated dependency on @PLN125, not a workaround to invent here.
- *An interface requires METHODS, not fields.* So this enforces "there is a key accessor", never
  "there is an `id` column". That is a feature: the accessor may compute, and loft never needed
  the field to be called `id`.

**Their half cannot be a type-system check at all.** Whether the *table* has that column, with a
compatible type, and an index on it, is a fact about a system loft does not compile. It is a
**bind-time** check: interrogate the schema once when the store is bound, and REFUSE the bind
otherwise. That refusal is loud and early, which is the only way failure path 9 stays survivable
— a missing index is not a wrong answer, it is a silent collapse from lazy to table-scan.

So the contract reads: **the type system guarantees loft brought a key; the bind guarantees the
database has somewhere to put it.** Neither check can do the other's job, and the design is
weakest if either is skipped.
