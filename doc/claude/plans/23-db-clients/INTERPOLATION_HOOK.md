<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# A library-implementable interpolation, and safe procedures on top of it

## The problem, measured

A safe SQL builder wants `"SELECT … {id}"` to put `id` in as a *bound parameter*,
never as text. Two mechanisms were tried and **both fail**, measured rather than
assumed:

- **The type cannot carry it.** `"{x}"` renders any value to text, so a
  validated `SqlIdent` and an attacker's `text` are indistinguishable once
  formatted.
- **`const` cannot carry it either.** A const string interpolates *runtime* data
  quite happily:

  ```loft
  x = taint("evil");
  const BAD = "DELETE FROM {x}";     // compiles
  // -> DELETE FROM evil
  ```

  `const` in loft means an immutable binding, not a compile-time constant, so it
  is not a gate.

So no discipline applied *after* formatting can recover what formatting threw
away. The fix is not to throw it away.

## What the parser already knows

A format string does **not** lower to one opaque text today. `formatted_string`
(`src/parser/objects.rs`) emits a `Block["Formatted string"]` of alternating
operations:

```
OpAppendText(var, "SELECT * FROM ")   <- literal the AUTHOR wrote
append_data(var, tbl, <spec>)         <- a HOLE
OpAppendText(var, " WHERE id = ")     <- literal
append_data(var, id, <spec>)          <- a HOLE
```

**The literal/hole boundary exists at parse time.** It is erased only because
every branch appends into the same text buffer. That erasure is the bug, and the
boundary is exactly the fact a safe builder needs: literals are code the author
wrote, holes are data.

## The invariant

> **A format expression preserves which bytes the AUTHOR wrote and which came
> from a VALUE, and hands both to the type being built.**

Text keeps today's behaviour by being the type that concatenates them. A library
type chooses differently — and injection safety stops being a rule anyone can
forget, because a hole has no way to become SQL syntax.

This is the construction C# reaches with `FormattableString`, Scala with string
interpolators, JS with tagged templates and Python with PEP 750 t-strings. loft
is unusual only in already having the parts in hand.

## Shape

Type-directed, so it needs no new syntax — the target type decides:

```loft
interface Interpolated {
  fn lit(self: Self, s: text)          // a literal chunk the author wrote
  fn hole_text(self: Self, v: text)    // an interpolated value
  fn hole_int(self: Self, v: integer)
  …
}

q: SqlText = "SELECT * FROM {tbl} WHERE id = {id} AND name = {name}";
```

lowers to `q.lit("SELECT * FROM "); q.hole_…(tbl); q.lit(" WHERE id = "); …`
instead of appending into a text buffer.

`SqlText` then builds two things at once: SQL text in which every hole became a
placeholder (`?` / `$n`), and the ordered values to bind. **A value cannot reach
SQL syntax, because the only path into the text is `lit`, and `lit` is only ever
called with bytes from the source file.**

An identifier — a table name, genuinely part of the syntax — is the deliberate
exception, and it is typed: `hole` on a `SqlIdent` validates and quotes it. That
is one method, one validating constructor, one place to audit.

### The constraint worth naming

loft interfaces are static-dispatch with no associated types, so `hole` cannot be
generic over the value. It needs either one method per scalar kind
(`hole_text` / `hole_int` / …) or a single boxed value type. The per-kind form is
uglier and needs no new language machinery; the boxed form is nicer and needs a
value type that does not exist yet. **Per-kind first** — it can be collapsed
later without changing what the author writes.

### Re-assertion sites

One: `formatted_string`. It already chooses between `OpAppendText` and
`OpAppendStackText` by target kind, so a third branch is a branch it already has.
The emitted calls are ordinary method calls, so **neither backend needs a new
path** — which is why this is cheaper than it sounds. `N = 1` is unusually good
for a feature this load-bearing.

### What it buys beyond SQL

Every injection family becomes a type rather than a review item: shell commands,
HTML and attribute escaping, path building, log records with structured fields.
Each is the same construction with a different `lit`/`hole` pair. A hook that
serves only `sql` would be a hook with one user; this one is not.

## Stored procedures on top

With the hook, a procedure is *a named, parameterised statement*:

```loft
proc = procedure("archive_orders",
                 "DELETE FROM {tbl} WHERE created < {cutoff}");
```

`tbl` is a `SqlIdent` (validated, quoted, inline); `cutoff` is a parameter
(placeholder + binding). The body's SQL text is therefore always author-written
plus validated identifiers — by construction, not by convention.

### sqlite has no stored procedures, so we implement them

Rather than let sqlite answer "unsupported" and lose the uniform API, the
backend provides the same contract:

| | mariadb / postgres | sqlite |
|---|---|---|
| `deploy(proc)` | `CREATE OR REPLACE PROCEDURE` / `FUNCTION` | register name → statement in a client-side table |
| `call(name, values)` | `CALL name(?, …)` | prepare the registered statement, bind, step |
| where the definition lives | the server catalog | the process |

**The safety property is identical on all three**, because it comes from the
type, not from where the definition is stored: values are bound, identifiers are
validated, literals come from the source.

What genuinely differs, and must be **refused rather than degraded**:

- **Procedural control flow.** MariaDB and PostgreSQL bodies may contain `IF` /
  loops / multiple statements; sqlite has no procedural language. A body using
  them must be rejected by the sqlite backend **at deploy**, so a program never
  silently gets different semantics on a different backend. This is the same rule
  as `#c`'s wasm column: a defined answer everywhere, not the same answer.
- **Atomicity.** A multi-statement server-side procedure is one round trip; the
  emulation is several. Anything relying on that must wrap itself in a
  transaction, which the mapping already requires for object writes.

## Build ladder — small, safe steps

Each step lands green on its own and is verifiable before the next. The language
change comes first and lands **inert**: nothing existing may change behaviour.

| step | what it proves | how it is proved |
|---|---|---|
| **H1** | the contract exists and changes nothing | declare `Interpolated`; every existing format string must emit **byte-identical IR and native Rust** — `loft introspect` before/after over a corpus with literals, holes, specs, `text?` holes, nested formats. An empty diff is the whole proof |
| **H2** | one hole kind, one backend | a target type implementing the contract receives `lit`/`hole_text` in source order; `--interpret` only. Assert the SEQUENCE, not just the result — order is the fact |
| **H3** | both backends agree | the same corpus on `--native`, byte-identical output to `--interpret` |
| **H4** | the remaining scalar kinds | `integer` / `float` / `boolean` / `character` holes, one per step, each with a spec (`{x:>8}`) so formatting options still reach the hole |
| **H5** | **a value cannot become syntax** | `SqlText` builds placeholders + bindings. The cell that matters: interpolate `'; DROP TABLE t; --` and assert the table still exists and the value came back as DATA. Non-vacuous by construction — it fails loudly if the hole ever reaches the text |
| **H6** | the deliberate exception | `SqlIdent`: validated, quoted, inline. Assert a non-identifier is REFUSED at construction, not at deploy |
| **H7** | procedures | `deploy` / `call` on mariadb + postgres; the sqlite registry emulation; a body with procedural control flow REFUSED at deploy rather than degraded (and by every backend — see Status) |

`text` keeps its behaviour throughout — H1 is the step that proves it, and every
later step re-runs that corpus.

## Transaction ladder

Cheap, because all three backends spell it identically — which is exactly why it
should land before the mapping needs it, not after.

| step | what it proves | how |
|---|---|---|
| **T1** | begin / commit / rollback exist | the three verbs on `SqlDb`, all three backends, `db_commit` returning a status |
| **T2** | rollback actually rolls back | write rows, roll back, assert they are GONE — on each backend. sqlite is the always-on cell again |
| **T3** | nesting is refused, not ignored | `db_begin` inside a transaction is an error. A silent no-op is the dangerous version: the inner "rollback" would discard the OUTER transaction's work |
| **T4** | an object-graph write is atomic | S5/S6's collection write wrapped in one transaction; kill the process mid-write and assert the collection is absent rather than half-present |

**T1–T3 belong before S5**, and T4 is part of S5/S6 rather than after them —
writing a collection non-atomically is not a smaller step, it is a wrong one.

## Status

**H1–H7 are BUILT** — see [plans/124-interpolation-hook](../124-interpolation-hook/README.md)
for what shipped, what the build corrected, and the proof. The design above stands
as written; three details changed on contact:

- **The hole methods take the value, and an unsupported kind is a compile error**
  rather than a fall back to `hole_text`. Falling back would put a value onto the
  text path, which is the hole this exists to close.
- **A format spec on a hole is refused.** `"{x:>8}"` has nothing to format when
  the value is handed over rather than rendered, so H4's "each with a spec" became
  "each kind, no spec".
- **Procedural control flow is refused on ALL FOUR backends**, not only on the two
  that emulate procedures. The table above expected the line to fall between
  sqlite and the servers; measuring moved it. MariaDB writes procedural bodies in
  SQL/PSM and PostgreSQL in plpgsql or a `BEGIN ATOMIC` body, and neither reads
  the other's — a `BEGIN … END` MariaDB accepts is a syntax error to PostgreSQL.
  There is therefore no procedural body a uniform API could carry across even the
  two backends that HAVE one, so the contract is one statement per procedure. The
  reasoning is unchanged (refuse rather than degrade); only its reach grew.

The companion constraint — `hole` needing one method per scalar kind — is
**@PLN125 arc A** (associated types; arc B is the scope-end hook, arc C
indexing). The hook is a **language feature** and belongs to loft rather than to
@PLN23 — the DB library is its first consumer and its motivating case, not its
owner. The measurement above (neither types nor `const` can carry the
distinction) is the argument for building it rather than working around it.
