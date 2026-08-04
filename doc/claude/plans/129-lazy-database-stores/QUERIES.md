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
