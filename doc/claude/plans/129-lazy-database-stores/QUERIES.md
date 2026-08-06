<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN129 — What a lazy binding can ASK

How a query is derived, what the collection kinds express, what they cannot, and the
requirement that one record is one record however it arrived. Companion to
[README.md](README.md) (the model) and [BINDING.md](BINDING.md) (the schema contract).

## What queries this model can express — the collection KIND is the query shape

A primary-key lookup is not enough. Historical records ask awkward questions — every position a
person held, the one valid on a date, the last before a date — and those must be possible here or
the model is a toy. They are, and still without anything enumerated ahead of time, because
**loft's collection kinds already are query shapes** and the descriptor records which kind a
collection is along with its key fields:

| the loft collection, and the operation on it | the query it derives |
|---|---|
| `hash<T[k]>` — `xs[k]` | `WHERE k = ?` — equality on an indexed column |
| `sorted<T[k]>` — `xs[lo..hi]` | `WHERE k BETWEEN ? AND ?`, in key order |
| `index<T[a, b]>` — slice | composite `WHERE a = ? AND b BETWEEN ? AND ?` + `ORDER BY`, asc/desc as declared |
| iteration over the collection | a full scan — the dangerous one (failure path 2) |

So *"every position person 42 held, ordered in time"* is an `index<Position[person_id, from]>`
range slice, and *"the one valid at D"* is the same slice bounded and taken to one. The
application declares the collection kind that matches the question, exactly as it would declare
the index in SQL — and the descriptor already carries kind + key fields + sort direction, so the
query is derived, not written.

**This is also the batching from failure path 8.** A range slice is ONE query returning many rows,
so the cure for N+1 is not a special bulk API bolted on: it is using the collection kind that
matches the question instead of N key lookups in a loop.

**What it cannot express, and the escape hatch.** A predicate on a non-key column (`name LIKE …`),
an aggregate, anything the declared keys do not cover. Deriving those from a layout is not
possible, and a scan-then-filter would silently fetch the table. So there must be an **explicit**
form — run this query, materialise the rows into this collection — which is the generalisation of
what `store_load_keys` already does for the paged reader. Implicit for what the kind expresses;
explicit and visible for everything else, and never a silent scan.

## One record, however it arrived — the requirement that ties it together

A `LIKE` query over persons and a walk from a company must reach **the same person records**
(owner's requirement, 2026-08-04). That is the load-bearing consequence of *the collection is the
cache*, and it constrains all three arrival paths to one destination:

| how a record arrives | where it lands |
|---|---|
| key lookup miss — `persons[42]` | the `persons` collection |
| explicit query — `name LIKE 'Ada%'` | the `persons` collection |
| navigation — from a company to its people | the `persons` collection |

So an explicit query is **not** a side channel returning a detached result set: it POPULATES the
collection, and a later `persons[42]` hits what the `LIKE` already brought in. Identity holds
across all three by the same mechanism as before — the collection is asked first, always — and no
path is allowed to materialise a record privately.

**Two consequences worth stating before they are discovered.**

*Iteration becomes history-dependent, on purpose.* After a `LIKE`, `for p in persons` walks that
result plus whatever else was touched. That is a coherent model — the collection is a working set,
populated by the queries you ran — but it means iteration answers *"what have I got"*, never
*"what exists"*. Failure path 2 is the rule that keeps it honest; this is the shape it takes.

*Navigation from a company is a query parameterised by its owner.* `company.people` is not a
stored vector — it is `WHERE company_id = <this company's key>`, a collection-valued field whose
query is fixed by the FK and the owning record. That needs the FK direction declared (the mapping
above), an index on the referencing column (the bind-time check above), and it is where a
`hash`/`index` kind on the *referencing* side earns its keep.

## Arc B — who EXECUTES the derived query

Deriving the SQL is the easy half. The hard half is who runs it, and one measurement settles it.

**The interpreter cannot make a synchronous loft call from inside a lookup.** `State::fn_call`
pushes a `CallFrame` and REDIRECTS the instruction pointer (`self.code_pos = to`); the opcode
handler then returns and execution continues into the callee. So `get_record` cannot do "call a
loft fetch function, take its result, carry on" — there is no nested interpreter. Making it
possible means re-running the lookup after a callback returns, which is a bytecode-level control
change and far larger than this arc. (The `to_text` hook is not a counter-example: the PARSER
resolves it and emits an ordinary `Call`, so it is compile-time dispatch, not re-entrancy.)

That rules out the shape most people reach for first — *the binding names a loft function, and a
miss calls it* — and it rules it out on a fact rather than a preference.

**So the source stays a RUST-side interface, and a SQL driver sits behind loft's existing `#c`
machinery, called from Rust.** `c_call::resolve` and the per-arity trampolines are already Rust
APIs: core can drive a C library with no crate, no rustc and no re-entrancy, which is exactly
what @PLN24 exists to provide. Arc A's file source is the same interface with a different
implementation.

This does NOT put general SQL knowledge in core. Core needs to send a derived string and read
back a row — a handful of C entry points — and WHICH library provides them is configuration, the
same shape `[c] optional-libs` already uses. The dialect differences that matter
(`BEGIN TRANSACTION` vs `BEGIN`, placeholder spelling) live where they already live: in the
sqldb libraries, which @PLN23 built and proved uniform across four backends.

**The cost, stated:** two implementations of "a source" (file, database) rather than one, and a
narrow C surface owned by core. The alternative — teaching the bytecode to resume a lookup after
a callback — buys a loft-implementable source and costs a control-flow change to the interpreter.
That trade should be re-opened only if a third source appears, because two is not yet a pattern.

## The derivation, concretely

Everything the query needs is in the descriptor, and the pieces join exactly:

| SQL part | descriptor source |
|---|---|
| table | `LayoutDesc.names[elem]` — the element type's name |
| columns | the elem's `LayoutNode::Record(fields)` → each `LayoutField.name` |
| `WHERE` | `Iterated::Hash { keys }` → each `Key`, matched to its column |
| `ORDER BY` | `Iterated::Sorted`/`Index` keys carry `(u16, bool)` — the bool IS the direction |

**A key maps to a column by POSITION.** `Key { type_nr, position }` and
`LayoutField { name, position, content }` both carry a byte offset into the record, so the key
field's name is the field whose `position` matches. Nothing has to be declared twice, and nothing
is matched by name-guessing.

So for `persons: hash<Person[id]>` where `Person { const id: integer, name: text }`:

```sql
SELECT id, name FROM person WHERE id = ?
```

and for `positions: index<Position[person_id, started]>` walked as a range:

```sql
SELECT person_id, company_id, started, ended FROM position
 WHERE person_id = ? AND started BETWEEN ? AND ?
 ORDER BY started ASC
```

— the `ASC` from the key's own direction bit, not from a convention.

**This example said `from` and `to` until step 2 built it**, which is worth
keeping because the correction is a rule rather than a typo: **every loft
identifier is a legal SQL identifier, and some of them are RESERVED words.**
`from` is a perfectly ordinary loft field name and the natural one for a history
row, and the query above with it in the column list does not parse on any engine.
Since nothing distinguishes a reserved word by shape, the derivation cannot dodge
it — so **it quotes everything**, and the queries it really emits are

```sql
SELECT "id", "name" FROM "person" WHERE "id" = ?
SELECT "person_id", "from", "to" FROM "spell" WHERE "person_id" = ?
```

That removes the whole class rather than one word of it: no reserved-word list to
carry, and none to keep current as engines add words. The cost is that the quote
CHARACTER is a dialect fact — `"x"` is an identifier in standard SQL and a string
literal in MySQL — so it is declared (`Quoting::Double` by default, `Backtick` for
MySQL/MariaDB, `Bare` for a caller who wants the query to read as they wrote it
and accepts a refusal for a name that cannot be written unquoted). Placeholders
are the same kind of fact: `?` by default, `$1`-numbered for PostgreSQL.

Quoting also settles the table name: the default is the type's name **lowercased**,
which is the spelling that means the same thing everywhere — PostgreSQL folds an
unquoted name down, so a table created the ordinary way is already lowercase and
`"person"` finds it. A table that really is mixed-case is what the override is for.

**Two facts the derivation had to learn from the descriptor rather than from this
document**, both found by building it:

- **An `index` element record carries its own tree links.** `Position` comes back
  with `#left_1`, `#right_1` and `#color_1` after its declared fields — the
  red-black bookkeeping, stored INSIDE the element. `#color_1` is an ordinary
  boolean, so a column filter written on field TYPE selects it and the SELECT
  names a column no table has. The non-data predicate (`enum`, no position,
  `#`-prefixed) is `LayoutField::is_data`, one home shared with
  `read_via_descriptor` and the browser delivery.
- **A key is an INDEX into the full field list**, synthetic fields included, so
  the key list and the column list are numbered in the same space and must not be
  re-based on the filtered columns.

**What the descriptor cannot give**, and what therefore has to be declared ([BINDING.md](BINDING.md)):
the table when it is not the type's name (`persoon` for `Person`), a column when it is not the
field's name, and the dialect facts above. The derivation is the DEFAULT; the mapping is the
override, and both feed one query builder rather than two paths — an empty mapping IS the
derivation, which is what keeps them from drifting.

A mapping is checked **where it is written**: naming a type or a field that does not exist is
refused at construction, not at query time. A typo is otherwise invisible — the derivation would
fall back to the default and query a column nobody meant.
