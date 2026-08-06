<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN133 — one SQL boundary: a universal table definition behind one connection string

## Status

**Design only — nothing built. BOTH GATING PROBES ARE RUN, and both answered
2026-08-06** (§ Probe results). P1 **passes**, so **option B is viable** and the
architecture is decided. P2 **passes for reads and fails for writes on sqlite**,
which is a bounded, documented limit rather than a blocker.

Every measurement below is from the current tree and is cited so it can be
re-checked rather than believed.

**Issue:** [loft-lang/plans#133](https://github.com/loft-lang/plans/issues/133).

## Effort + design

- **Effort:** H
- **Design:** the invariant is named and its load-bearing claims have probes; the
  reconcile rules for a foreign schema are stated but untested against a real one.
- **Last touched:** 2026-08-06

## Goal

Two requirements, and they compose into one design.

1. **One configuration string switches every SQL consumer in the process to a
   different database** — a lazily-bound collection (@PLN129), a client routine
   (@PLN23), and an ORM write (@PLN23 S5–S7) — with no other edit anywhere.
2. **A structure written to the database is IMMEDIATELY usable through lazy
   loading.** Write a graph, bind a collection to the same string, traverse it.
   No export step, no schema hand-off, no second description of the same tables.
3. **A loft structure has ONE universal table definition.** Where the database has
   nothing, loft writes that definition itself so lazy loading is possible at all.
   Where the database already holds data — **ours, or another project's** — loft
   FOLLOWS the structure that is there.

Today the first is impossible in the strongest sense — the lazy path speaks only
sqlite and cannot reach the other three backends at all — and the second has
never been possible, because the writer and the reader derive their SQL from the
same descriptor **in two different languages**.

## The invariant

> **A loft type has exactly one table definition, and that definition is a VALUE —
> derived from the type, or read back from the database, in the SAME shape.
> Creating, following, querying and writing are all functions of that one value,
> so a table loft made and a table loft found are indistinguishable to everything
> downstream. A connection string selects which driver renders it.**

Everything below either tries to falsify that sentence or pays for it.

### `TableDef` is the single currency

The invariant is only worth stating if one value really carries all four jobs. It
does, as three producers and four consumers of one type:

| | |
|---|---|
| `derive(T) -> TableDef` | the UNIVERSAL definition, from the loft type |
| `introspect(conn, name) -> TableDef?` | what the database actually has |
| `reconcile(want, have) -> Binding \| Refusal` | the match, naming exactly what is wrong — and carrying the per-column CONVERSION |
| `render(TableDef, dialect) -> DDL` | `CREATE TABLE` **and its index** |
| `select(TableDef, key)` / `insert(TableDef)` | the statements |

Requirement 3 is then two lines rather than a mode: **absent → `render(derive(T))`;
present → `reconcile(derive(T), introspect())`.** No provenance flag, no "is this
ours" state to keep — a database loft created takes the same path as a foreign
one, and simply reconciles trivially. Removing that state is the point: a
provenance marker would be a second description of the same fact, which is the
failure this design exists to prevent.

**This reverses a recorded decision, and that has to be explicit.** BINDING.md
carries the owner's 2026-08-04 decision that *"the DATABASE owns that schema…
loft is not a schema manager — no DDL, no migrations."* Requirement 3 revises it:
loft DOES emit DDL, but only into absence. The revised rule is **loft never
overwrites a structure it did not find missing**, which keeps the original
decision's substance — the existing database remains the authority — while
allowing the empty case to bootstrap itself.

### The asymmetry that makes this cheap: we CHOOSE, or we CONVERT

`reconcile` does not answer yes/no. It answers **a binding carrying one conversion
per column**, and that single idea removes most of the hard cases:

- **Where loft defines the table, it picks the cleanest and most efficient column
  type** — `BIGINT`, `DOUBLE PRECISION`, `TEXT`, `BOOLEAN`, `NOT NULL` where the
  field is not nullable. The conversion is then identity, and nothing is paid.
- **Where loft follows someone else's table, it accepts what is there and
  converts** — a number kept in a `VARCHAR`, a boolean as `0`/`1`, a float as
  `NUMERIC`. The conversion is a property of the binding, computed once at
  reconcile, not a branch taken per row read.

**This is what dissolves the float problem.** A `double` returned BY VALUE cannot
cross the `#c` boundary (the interpreter's trampolines are integer-class), which is
why the library binds floats as TEXT — and reading it as text and parsing it is a
conversion like any other. So:

- **Measured (P2): reading is exact**, with `printf('%!.17g', …)`. So @PLN128's
  E3 float refusal is **not** a prerequisite for the read path, which is the whole
  of the lazy loader.
- **Measured (P2): writing is not**, on sqlite, for extreme-exponent values —
  its decimal parser can store a literal one ULP off, and more digits do not fix
  it. **E3 therefore pays for the WRITE path on sqlite specifically**, and
  nowhere else. 0 of 3000 values in the ±1e6 range were affected.
- On PostgreSQL this is not even a workaround — the wire protocol returns every
  value as text already, so "parse it" is the native path. **Expected to be exact;
  not measured** (no server), and the gate must check it.
- What replaces the capability requirement is a **contract**: a float written by
  loft and read back by loft must compare EQUAL. A rendering that loses the low
  bits is a wrong value, and P2 shows the obvious spelling (`CAST(v AS TEXT)`)
  loses it 94% of the time.

The same asymmetry retires a second refusal for free: @PLN129 today refuses a
**narrow integer field** (`i32`, `u8`, `size(2)`) because there are four encodings
and their null sentinels to get right. Under a conversion plan that is one
per-column decision made once at reconcile, not four code paths at read time.

### Following a foreign structure — what reconcile must decide

The interesting half of requirement 3 is the table loft did not write. `reconcile`
is where the design earns its keep, and each answer is a rule rather than a
heuristic:

- **Column matching is by NAME, not position.** Position is how loft finds which
  of its own fields is the KEY (`Key.position == LayoutField.position`); the
  column is then that field's name, overridable by the declared mapping. A
  foreign table's column order means nothing and must never be read as meaning.
- **Extra columns in the table are FINE for reading** — `SELECT` names only the
  columns loft wants. They are not fine for writing: an unknown `NOT NULL` column
  with no default makes `INSERT` impossible, so a write must be refused there,
  naming the column. Reading and writing therefore reconcile to different
  verdicts against the same table, and `Binding` has to carry which it permits.
- **A missing column, or an incompatible type, refuses** — @PLN129 already does
  this and reports through arc C's channel.
- **A missing INDEX on a foreign table refuses; it does not silently ALTER.**
  Creating a table where there was none is filling absence. Adding an index to
  someone else's populated table is mutating their schema, can take a long time,
  and can change their write performance. It stays opt-in, and the refusal names
  the index it wanted so a DBA can add it.
- **Our own struct gaining a field is MIGRATION, not reconciliation.** This is
  the one case where "is it ours" genuinely matters, and the uniform rule does not
  absorb it: under reconcile a table missing a column simply refuses. Migration
  (@PLN23 S7) stays an explicit, separate operation. Naming this boundary is what
  keeps the uniform rule honest instead of quietly wrong.

**Requirement 2 is what makes the shared derivation load-bearing rather than
tidy.** If the writer creates `person(id, name)` and the reader derives
`SELECT "id","name" FROM "person" WHERE "id" = ?`, a round trip works only while
the two derivations agree — and nothing checks that they do. Worse, @PLN129's
bind is REFUSED when no index serves the lookup (`EXPLAIN QUERY PLAN` answering
`SCAN`), so a writer that emits a table without the index the collection kind
implies produces a database its own reader cannot bind to. **The collection kind
has to decide both**: the index the writer creates and the query the reader
derives. One derivation is the only way that holds.

## What is actually duplicated — measured, not recalled

| | core (@PLN129) | the loft library (@PLN23) |
|---|---|---|
| sqlite symbols bound | **15**, hand-written typed `extern "C"` in `src/database/sql_source.rs` | **14**, `#c` declarations in `tests/fixtures/sqldb/sqlite/` |
| other backends | **none** | postgres, mariadb, duckdb — one `SqlDb` interface, byte-identical output |
| quoting / placeholders | `Quoting::{Double,Backtick,Bare}`, `Placeholder::{Question,Numbered}` in Rust | per backend, in loft (`$1 $2 …`, `?`, backticks) |
| `double` by value | **yes** — `extern "C" fn(*mut c_void, c_int) -> f64` | **no** — floats bind as TEXT |
| schema → SQL | `sql_query.rs` derives SELECT from `LayoutDesc` | S5–S7 would derive DDL/INSERT from the same |

Twelve sqlite symbols are bound twice. The dialect facts were measured twice,
independently. And the two halves disagree about floats.

## Why it happened — a capability gap, not carelessness

Three C-call mechanisms exist, and they do not agree:

| caller | mechanism | can pass a `double` |
|---|---|---|
| `--native` | rustc-emitted typed `extern "C"` from `CSignature` | yes (refused for parity) |
| the interpreter | `call_at_arity(f, &[u64])` — a 0..=12 ladder, integer class only | **no** |
| core's lazy source | hand-written typed externs | **yes** |

The interpreter's ladder is `u64`-only for a reason that is not laziness: a
signature is known at **runtime**, and Rust cannot build a typed `extern "C"`
from runtime data. A full cross-product of arity × argument class is ~2^13 × 13
function types.

So core needed floats, the public `#c` path could not give them, and core wrote
its own driver. **The SQL duplication is downstream of the C-call capability
gap.** That is the thing to fix first, whichever architecture wins.

## The option that looks cleanest, and why it is wrong

*Let the loft library DECLARE the symbols and the dialect; let core read those
declarations and execute them itself.* One declaration, two executors, no
re-entrancy needed.

**It fails on the protocol.** The four backends are not one protocol with
different symbol names:

- sqlite — `prepare_v2` / `step` / `column_*`
- postgres — `PQexec` returns a result object, then `PQgetvalue`
- mariadb — a statement API over `MYSQL_BIND` **arrays of structs**, which needs
  a hand-written ANSI-C shim to express at all

What unifies them is the `SqlDb` **interface**, and that interface is loft code.
A symbol table is not enough, so core would re-implement the per-backend logic —
the duplication returns one layer down, with the backends now stated twice each.

## The real choice: which side owns the driver

**Option A — drivers move to Rust in core.** The loft library becomes a thin
wrapper over core builtins.

**Option B — core gains the ability to call loft**, and uses the library that
already exists.

**Counting decides it.** Under A, every backend is restated in Rust: N = 4 today,
+1 per backend forever, and the loft versions must either be deleted (losing
@PLN24's flagship consumer and the "no rustc in a library" property) or kept in
sync by hand — silent divergence, the exact `N × silence` this project counts
before writing code. Under B, a backend is written **once**, in loft, where four
of them already are and are already proven byte-identical across both loft
backends.

**Option B, on the counting.** But it rests on one unproven claim, below.

## How far B goes — the derivation moves to loft too

Requirement 2 wants one derivation. Under B it can be **loft code**, and that is
the answer to "rewrite Rust into loft or the other way": `src/database/sql_query.rs`
(415 lines of Rust) becomes a loft module beside the `SqlDb` interface, and serves
both directions —

| from one derivation | consumer |
|---|---|
| `SELECT … WHERE key = ?` | the lazy fault (@PLN129) |
| `CREATE TABLE …` **and its index** | the ORM write (@PLN23 S7) |
| `INSERT … VALUES (…)` | the ORM write |

**This is possible because @PLN127 already put the descriptor in loft's hands.**
`type_of(v)` answers a `TypeInfo` carrying each field's `name`, `type_name`,
`position`, `kind` and `nullable` — and the reflection doc already names the use:
*"It is what a generated `CREATE TABLE` needs for `NOT NULL`."*

**One gap, and it is additive.** Reflection reports a keyed collection as
`KeyedKind` plus its element type "and nothing more" — it does **not** expose
which kind it is (hash / index / sorted / ordered / radix), its **key fields**, or
the sort-direction bit. The derivation needs exactly those: core reads them from
`Iterated::Hash { keys }` and the `(u16, bool)` direction. So moving the
derivation to loft requires extending @PLN127's surface with the collection kind
and its keys, **carrying `position` so the key→column match stays by position and
never by name-guessing**. Additive, and it is the same fact reflection already
publishes for ordinary fields.

Core then keeps only what cannot leave it: detect the miss, hand the collection's
type and key to loft, and materialise the returned row into the store through
`record_new` + `record_finish`.

## The probe that decides it

> **Can `State` run a loft function to completion re-entrantly, from inside an
> opcode handler, and return its value?**

@PLN129 costed this as "a bytecode-level control change… far larger than this
arc", and that was right *for the shape it considered* — a callback from inside
the lookup. The requirement is weaker than that, and the code is already most of
the way there:

`Stores::find_or_fetch` (`src/database/allocation.rs:3997`) already does
**miss → fetch → re-run the lookup**. Re-running is deliberate and documented:
the collection stays the single source of truth. What is missing is only that the
fetch is Rust.

So the change is to lift `find_or_fetch` out of `Stores` into its **two** callers
— `State::get_record` and `codegen_runtime`'s lookup, the count @PLN129
established — so the `&mut Stores` borrow is released around the fetch:

```
found = stores.find(…)
if found.rec == 0 && stores.is_lazy(coll) {
    run the loft fetch          // Stores borrow released here
    found = stores.find(…)      // retry — the rule that already exists
}
```

For `--native` this is trivial: generated code is Rust calling Rust. For the
interpreter it needs `State::run_until_return(d_nr, args)` — push the frame, run
the existing loop until `call_depth` drops back to the entry depth, take the
value. That is a **re-entrant execute loop, not a bytecode change**.

**Write that function and call one trivial loft `fn` from an opcode handler.** If
the eval stack, `raise` unwinding, or a `par` block cannot survive re-entry,
option B is dead and A is the answer. This is cheap and it decides the whole
architecture, so nothing else should be built first.

## Failure paths — written before the code

1. **A backend the build does not have.** Must be "unreachable", never "no such
   row". `c_library_available` (@PLN24 arc G) and `store_lazy_error` (@PLN129 arc
   C) already exist; the string parser must route into them rather than answering
   null.
2. **Dialect drift.** Quoting and placeholders must come from the BACKEND, not
   from core. Otherwise the string switches the driver but not the quoting, and
   mariadb gets ANSI double quotes — measured to answer
   *`error in your SQL syntax … near '"loft_p"'`*. A wrong query, not a wrong
   result, but only because mariadb happened to reject it.
3. **A float loses its low bits on the way through text.** Core does exact `f64`
   today through typed externs; under B a float crosses as text and is parsed,
   which is the accepted model — but only if the rendering carries enough
   significant digits to round-trip IEEE-754 exactly. sqlite's default text
   rendering of a `REAL` does not, so the driver has to ask for more. **The test
   is a round trip that compares EQUAL, per backend**, and it must be written
   before any consumer stores a coordinate. This is the failure that looks like
   data: values that are almost right.
4. **Re-entrancy inside `par`.** A fault taken on a worker thread would run loft
   there. Either the fetch is serialised, or `par` + lazy is refused.
5. **Two notions of "one world".** @PLN129 arc D pins a source per traversal;
   @PLN23 T1–T3 has transactions. Under one driver these must become one concept,
   or a traversal inside a transaction has two answers about what it can see.
6. **The derivation, read and write.** `sql_query.rs` derives SELECT from
   `LayoutDesc`; S5–S7 would derive DDL and INSERT from the same descriptor. One
   home, or a row written and a row read disagree about what the table is. Note
   the two traps already paid for on the read side: keys map to columns **by
   position**, and `LayoutField::is_data` excludes the red-black links
   (`#left_1`/`#right_1`/`#color_1`) that a `sorted`/`index` child table would
   otherwise name as columns. **A loft-side derivation must inherit both**, which
   means reflection has to filter the synthetic fields the way `is_data` does —
   if `type_of` reports `#color_1`, the generated `CREATE TABLE` grows a column no
   reader wants and the round trip breaks on the writer's side.
7. **The writer omits the index the reader requires.** @PLN129 refuses a bind
   whose lookup no index serves, and it is right to: a missing index is a silent
   collapse from lazy to table-scan. So DDL generation must emit the index the
   collection kind implies — `hash` an equality index on its keys, `sorted`/`index`
   an ordered one — or requirement 2 fails on the first bind after a write. This
   is the strongest argument for one derivation: the same fact chooses both.
8. **Visibility — "immediately" has an ordering rule.** @PLN129 arc D PINS the
   source when the collection binds, and for a database that pin is a
   transaction. A collection bound before the write commits will not see it, and
   arc D will REFUSE the fetch as drift rather than answer stale — correct, but
   it turns requirement 2 into a puzzle unless the rule is stated: **bind after
   commit, or make the pin re-establishable on demand.** Whichever is chosen has
   to be written down, because both are defensible and they differ in what a
   long-running traversal sees.
10. **A fault inside the nested fetch must not halt the program — MEASURED, and
   it does today.** P1c ran a loft fetch function that overflowed the call stack:
   the fault propagated and the program halted, and the caller read a garbage
   value. For an ordinary call that is right; for a fetch it violates C80, where a
   failed fetch reports through `store_lazy_error` and the lookup answers null.
   So the nested run must CONTAIN the fault and convert it into arc C's channel —
   otherwise moving the source from Rust to loft turns a class of source bug into
   a program halt, which is a regression against what @PLN129 ships today.
9. **Lazy stays READ-ONLY, and requirement 2 does not change that.** The write
   goes through the ORM/client; the lazy binding remains a read path that refuses
   writes loudly (@PLN129 failure path 4). "Write then read lazily" is two
   operations on one database, not a read-write collection. Making the binding
   writable would reopen the exact divergence-from-source-of-truth failure the
   whole design exists to avoid.

## Probe results — run 2026-08-06

Both gating probes were built as throwaways, run, and the code reverted. The
scripts are in this directory's history only; what matters is the answers.

### P1 — can the interpreter run loft re-entrantly from a lookup? **YES.**

**The mechanism already existed.** `State::execute_at` runs a loft function to
completion from Rust for the `par` workers — push a frame, run the dispatch loop,
read the result. The probe reused the ORDINARY call machinery instead: `fn_call`
pushes the frame and stores the return address, then the loop runs until that
frame pops. So the callee returns through exactly the path every other call uses.

**The borrow dissolves as predicted.** `Stores::find` RETURNS a value, so the
`&mut self.database` borrow ends before the nested call. The split in
`State::get_record` is three lines:

```rust
let first = self.database.find(&data, db_tp, &key);
if first.rec == 0 { /* nested loft call here */ self.database.find(&data, db_tp, &key) }
else { first }
```

| cell | result |
|---|---|
| a zero-arg loft fn called from inside `get_record`'s miss path | ran, returned its value (`6` from `3+2+1`) |
| that fn making its own nested + recursive calls | fine, 3 deep |
| outer locals across the nested run (`integer`, `text`, `vector`) | **all intact** — `a=111`, `b=outer-text`, `v=7,8,9` |
| repeated, including inside a `while` loop | 3 invocations, all clean |
| a resident HIT | no nested call — the miss path is the only entry |
| **the fetch shape**: the nested fn receives the COLLECTION, allocates text and INSERTS into the very collection the outer lookup is walking | **the outer retry finds the inserted record**; a second lookup is a resident hit with no nested call; the pre-existing entry is intact; no leak under `LOFT_STORES=warn` |

**So @PLN129's "far larger than this arc" was right about the shape it
considered and wrong about the one needed.** It assumed a callback *inside* the
lookup; what the design needs is a nested run at the CALLER, and the retry it
requires is already how `find_or_fetch` works.

**P1c found a failure path the prose had not listed.** When the nested function
FAULTS (runaway recursion → call-stack overflow), the fault propagates and
**halts the program**, and the probe read a garbage value off the stack. For an
ordinary call that is correct. For a FETCH it is not: @PLN129's contract is C80 —
a failed fetch reports through `store_lazy_error`, never a halt. So a buggy or
unlucky loft fetch function would turn a lookup into a program halt where the
Rust source reported through the channel.

**That is a requirement on the design, not a defect in the probe:** the nested
call must be run with the fault CONTAINED and converted into arc C's channel.
Recorded as failure path 10.

### P2 — does a float survive a text round trip? **READS yes, WRITES no (sqlite).**

Measured with sqlite 3.45.1, 2000 uniformly random bit-pattern doubles plus 3000
in the ±1e6 range.

| rendering | inexact, of 2000 random doubles |
|---|---|
| `CAST(v AS TEXT)` — the obvious choice | **1887** |
| `printf('%.17g', v)` | **928** |
| `printf('%!.17g', v)` — note the `!` | **1** |

**loft's own half is exact.** `float → "{v}" → as float` round-tripped 12/12
hand-picked hard cases on BOTH backends: loft renders the shortest form that
round-trips.

**`CAST(v AS TEXT)` is the trap**, and it is the spelling anyone would reach for
first: it renders `%!.15g`, so `123456789012345.6` comes back as
`123456789012346.0` — a different number, not a rounding artefact. `%.17g`
*without* the bang looks fine on hand-picked values and still loses 46% of random
ones, which is exactly why this was swept rather than sampled.

**The one remaining failure is at INSERT, not SELECT.** Isolated: sqlite's own
round trip (`v = CAST(printf('%!.17g', v) AS REAL)`) is **exact 2000/2000**, so
the reader is sound. The single mismatch is sqlite's decimal PARSER storing a
value one ULP from the literal it was given — and **more digits does not fix it**
(shortest, 20-digit and 25-digit forms all store the same wrong double).

**Bounded, and the boundary is worth knowing:** 0 of 3000 values in the ±1e6
range were inexact. The failure lives at extreme exponents (the mismatch was
`-5.196972490273514e-183`). So:

- **Reading is exact** with `printf('%!.17g', …)`, which the sqlite driver's
  SELECT must wrap float columns in. Use `printf`, not `format` — `format()` is
  sqlite 3.38+, `printf()` is 3.8.3+.
- **Writing an extreme-exponent float through a decimal literal can lose one
  ULP.** The exact fix is `sqlite3_bind_double`, which is the `#c` float
  capability — so **@PLN128 E3 pays for the WRITE path on sqlite**, and only
  there. It stays out of the critical path for everything else.
- PostgreSQL is unverified without a server; its wire protocol is text already
  and its parser is `strtod`, so it is expected to be exact. **Expected, not
  measured** — the gate must check it.

### P3 — the float round trip on all FOUR backends. **The loss is on the WRITE side.**

Run locally against live servers (PostgreSQL 16.14, MariaDB 10.11.14, duckdb via
`LD_LIBRARY_PATH`), 2000 random bit-pattern doubles each except duckdb (500,
driven through the loft fixture).

| backend | rendering | inexact |
|---|---|---|
| sqlite 3.45 | `CAST(v AS TEXT)` | 1887 / 2000 |
| sqlite 3.45 | `printf('%.17g', v)` | 928 / 2000 |
| sqlite 3.45 | **`printf('%!.17g', v)`** | **1 / 2000** |
| PostgreSQL 16 | `extra_float_digits = 0` | 1887 / 2000 |
| PostgreSQL 16 | **`extra_float_digits = 1` (the modern default) or `3`** | **0 / 2000** |
| MariaDB 10.11 | **plain `v`, or `CAST(v AS CHAR)`** | **0 / 2000** |
| MariaDB 10.11 | `FORMAT(v, 17)` | 1144 / 2000 |
| duckdb | plain `v` / `CAST(v AS VARCHAR)`, **through loft's literal** | **19 / 500 — diagnosed, see below** |

**Each engine renders exactly once told to.** PostgreSQL and MariaDB are exact on
BOTH sides — their parsers are correctly rounded, so a decimal literal makes the
round trip. Two rules fall out, and neither is a default you can lean on:
`extra_float_digits` must be SET by the driver (it defaulted to 0 before PG12, and
a session can change it), and `FORMAT()` is a locale formatter, not a renderer.

**duckdb's own round trip is exact** — `v = CAST(CAST(v AS VARCHAR) AS DOUBLE)`
answered `yes` for every failing case, so the loss is in the hand-off, in
duckdb's parse of the literal loft hands it.

**Diagnosed (2026-08-06) by testing duckdb alone in C**, reading the value back
with `duckdb_value_double` so duckdb's own text rendering is out of the loop and
exactly one stage is under test. Of the 19 failures:

| class | count | what happens |
|---|---|---|
| **the 10²⁵⁶ bug** | **15** | the parsed value is *exactly* `correct / 10^256`, silently |
| 1–2 ULP | 4 | ordinary parser rounding, at ordinary magnitudes |

**The 10²⁵⁶ bug has a sharp, reproducible boundary, and it is the LITERAL FORM
that triggers it — not the value.** With the same doubles written in exponent
notation, duckdb is correct everywhere:

| the value, as `5.754124332515439e<E>` | full decimal expansion | exponent form |
|---|---|---|
| E = 272 (expansion 273 chars) | ok | ok |
| **E = 274 … 293 (expansion 275–294 chars)** | **WRONG — off by 10²⁵⁶** | ok |
| E = 294 and above | ok | ok |

A *window*, not a threshold: it recovers above E = 293. In the window the answer
is a plausible number roughly 10²⁵⁶ too small — the failure class C80 exists to
prevent, arriving as data.

**The mechanism is a TYPE decision, and `typeof` says it outright:**

| the literal | duckdb types it | result |
|---|---|---|
| 294-digit integer | **`HUGEINT`** | **silently truncated** — 5.75e293 does not fit in 128 bits |
| the same digits QUOTED, cast to `DOUBLE` | string → double | correct |
| `5.754124332515439e293` | `DOUBLE` | correct |
| 316-digit integer | `DOUBLE` | correct |

So duckdb reads a long bare integer literal as a 128-bit integer (max ≈ 1.7e38),
overflows it **without an error**, and only gives up on `HUGEINT` for a much
longer digit run — which is exactly why the failure has an upper edge as well as
a lower one. Nothing about the value is out of range for `DOUBLE`; it is the
literal's TYPE that is wrong.

Two spellings avoid it entirely, and either is a fine fix for the driver:
**exponent notation** (typed `DOUBLE`) or **quoting the digits** and casting.

**loft is what walks into it.** `"{v}"` renders a float as a full decimal
expansion with **no exponent**, so `5e-324` is 326 characters and any float above
~1e274 is a 275+ character digit run. Every SQL literal loft builds from a float
is therefore in expansion form by construction.

**An earlier version of this section got this wrong twice, and both errors are
worth keeping.** First it named long literals as the cause without probing —
a hypothesis published as a finding. Then it "disproved" that with literals of
252, 294 and 303 characters that round-tripped fine — but 252 is *below* the
window and the other two were TINY magnitudes, where the long digit run is after
the decimal point and truncating it costs precision rather than magnitude. A
disproof drawn from unrepresentative cells is not a disproof. The mechanism was
right; the reasoning on both passes was not.

**What this settles for the design:** a driver must render a float in **exponent
notation**, quote it, or bind it — never emit `"{v}"` as a bare literal. Reading
is exact everywhere once the rendering is specified.

**Two things to take upstream / next door**, neither fixed here:
- **duckdb 1.5.5** (the current release) types a 275–294 digit bare integer
  literal as `HUGEINT` and overflows it silently, giving a value 10²⁵⁶ too small.
  Minimal repro: `SELECT typeof(<294 digits>)` → `HUGEINT`;
  `SELECT CAST(<294 digits> AS DOUBLE)` → wrong; `SELECT CAST('<same digits>' AS
  DOUBLE)` → right. The silent-`HUGEINT`-overflow FAMILY is known upstream
  (duckdb#24081, duckdb#14580, both closed) but for aggregates, not literals;
  no issue was found for this case.
- **loft's float→text has no exponent form at all.** `1e300` is a 301-character
  string in every context, not just SQL, and `5e-324` is 326. That is what walks
  loft into the above, and it is worth deciding on its own merits.

### P4 — can a fault inside the nested fetch be contained? **YES, but it leaks.**

Containment is save-run-take-restore: run the nested frame while
`database.runtime_error` is none; if the callee raised, TAKE the error, truncate
the call stack to the saved depth, restore `call_depth` / `stack_pos` /
`code_pos`, clear `had_fatal`, and hand the reason back as a value.

Measured with a fetch function that overflows the call stack on its first call
and succeeds on its second:

- the fault was **contained** — `contained fault: call stack overflow`;
- the lookup answered **null**, which is the C80 / arc C shape;
- the outer program **continued** with every local intact;
- a **later** nested call succeeded and delivered its record;
- exit status 0.

**The caveat, and it is the finding: `Warning: 1 stores not freed at program
exit`.** Truncating the call stack abandons whatever the faulting callee had
allocated. A control run of the same probe without the fault leaks nothing, so
the leak is the contained fault's.

Containment therefore needs an unwind that RELEASES what the aborted callee held,
not just a stack truncation — otherwise a traversal over an unreachable source
leaks once per failed fetch, which is exactly the long-running case arc C's
sticky fault counter exists for. Recorded as the first thing S8 must solve.

## Implementation steps — small, safe, each one green

Ordered by **risk, not dependency**: the two unknowns are probed before anything
depends on them, and everything that can land INERT does so first. Every step
below leaves the tree green and nothing half-wired, so a step that goes wrong is
one revert rather than an unpicking.

**The safety net is already built.** @PLN129's tests assert query COUNTS — one
lookup, one query; a repeat lookup, zero — and arc F asserts identity across a
graph traversal. Those are the oracle for every step that replaces the driver: the
values can stay right while the counts go wrong, which is exactly the failure a
swap causes.

### Probes — nothing ships

| # | do | answers |
|---|---|---|
| **P1** | ~~a nested loft call from inside a lookup~~ | **DONE 2026-08-06 — PASSES.** Option B is viable. See § Probe results. |
| **P2** | ~~float through a text round trip~~ | **DONE 2026-08-06 — reads exact, sqlite writes lose one ULP at extreme exponents.** See § Probe results. |
| **P3** | ~~the float round trip on all four backends~~ | **DONE 2026-08-06.** Reads exact everywhere once the rendering is SPECIFIED; writes lose bits on sqlite and duckdb. |
| **P4** | ~~contain a fault inside the nested run~~ | **DONE 2026-08-06 — contained, but it LEAKS.** Needs a releasing unwind, not a stack truncation. |

### Inert — pure values and pure functions, nothing calls them

| # | do | green because |
|---|---|---|
| **S1** | Reflection gains the collection KIND, its key fields and the direction bit, each carrying `position`. Additive only. | existing reflection output is byte-identical; the new fields have no reader yet |
| **S2** | `TableDef` the value, and `derive(T)` from reflection. Must exclude synthetic fields the way `LayoutField::is_data` does. | a pure function with unit tests |
| **S3** | `render(TableDef, dialect) -> DDL`, including the index the collection kind implies. | unit-tested against hand-written expected DDL per dialect — hand-written, so agreement between two generators cannot pass for correctness |
| **S4** | `reconcile(want, have) -> Binding \| Refusal`, carrying per-column conversions. | pure; tested on hand-built pairs: exact match, missing column, incompatible type, extra column, missing index, a number in a `VARCHAR` |
| **S5** | The connection-string parser: `scheme:rest` → backend name. | pure; nothing routes through it |

### Wiring — one consumer at a time, sqlite kept as the control

| # | do | safe because |
|---|---|---|
| **S6** | `introspect(conn, table) -> TableDef?` in the loft library, sqlite only. | read-only; changes no existing behaviour |
| **S7** | The backend registry, used by the LIBRARY's own connect. No core change. | the library's four backends already pass their tests; the registry must not move them |
| **S8** | Core's lazy fault calls loft **for non-sqlite backends only**. Core's sqlite path is untouched. | every existing @PLN129 test still runs the old path — the suite is the control while the new path is proven beside it |
| **S9** | Switch sqlite to the loft path too. | the count assertions are the oracle: same counts, same identity, both backends, or the step is wrong |
| **S10** | Delete core's 15 typed externs and `sql_query.rs`. | a deletion whose proof is the suite that was green in S9 |

### Create-or-follow, then the write side

| # | do | |
|---|---|---|
| **S11** | Absent → `render(derive(T))`. Only into a table that is not there. | a fresh database becomes usable with no setup |
| **S12** | Present → `reconcile`, refusing through arc C's channel with the column or index NAMED. | a foreign database becomes usable with no rewrite |
| **S13** | `insert(TableDef)` and the ORM write path (@PLN23 S5). | |
| **S14** | **The gate** (below), run twice. | |

**Every step's cross-backend claim is a LOCAL measurement.** CI gates sqlite only;
PostgreSQL, MariaDB and duckdb are run locally and their results written down
where they were measured — see [TESTING.md § Database backends](../../TESTING.md).
A step that says "all four agree" without a local run beside it has not been
checked, and that is exactly the gap P3 found in the float rendering.

**What each wiring step must NOT do:** S8 must not touch the sqlite path (that is
what makes it revertible); S9 must not change the counts (that is what makes it
provable); S11 must not touch a table that exists (that is the whole of "the
database is the authority").

## The gate — the round trip, run twice

Write a struct graph through the ORM to a database, bind a collection lazily to
the **same connection string**, traverse it, and get back what was written:
values, identity across two paths, and the query COUNTS laziness predicts. On
**both loft backends** and on **at least two database backends** — a round trip
that only closes on sqlite has not tested the string.

**Run it twice**, and the second run is the one that matters:

1. Into an **EMPTY** database — loft creates the schema and its index.
2. Into a table created **by hand**, with different column types, a different
   column order and one extra column — loft follows it.

Same loft program, same assertions, both green. The first run passes even if
`reconcile` is a stub that always agrees; only the second proves requirement 3.

The gate cannot pass vacuously: if the two derivations disagree the bind fails, if
the writer omits the index the bind is refused, if the snapshot is pinned wrong
the traversal reads nothing, and if a conversion is missing a float comes back
almost right.

## What would falsify this design

- The re-entrancy probe fails (step 0) — then A, and the cost is four Rust
  drivers plus a decision about what the loft library is for.
- **A float cannot be round-tripped through text on some backend** (step 1). Then
  that backend needs the `#c` float capability after all, and @PLN128 E3 returns
  to being a prerequisite — for that backend only, which is worth knowing before
  it is promised.
- **`reconcile` cannot express some real foreign schema** — a composite primary
  key spread across columns loft models as one field, a table where the "key" is a
  view, a type whose text form is not parseable back. Then requirement 3 holds for
  a NAMED class of schemas rather than universally, and the class has to be
  written down instead of implied.
- A fifth backend appears whose protocol the `SqlDb` interface cannot express —
  then "one driver per string" is a smaller claim than it looks and the interface
  needs widening before this is built on.
- **Reflection cannot carry a keyed collection's keys without becoming a second
  description of the layout.** @PLN127's stated design rule is that reflection
  must not drift into a parallel account of the same bytes. If exposing keys
  cannot be done as a READ of the same descriptor core uses, the derivation stays
  in Rust and requirement 2 is met by core generating the DDL instead — a smaller
  unification, and one that leaves the ORM asking core for its schema.
- **The round trip needs a writable path the ORM has not built.** @PLN23's S5–S7
  are designed, not built. This design assumes the writer exists; if the ORM
  slips, the gate cannot run — but S1–S12 still stand on their own and deliver
  requirements 1 and 3 without it.

## Cross-plan effect

- **@PLN129** (closed) — S10 deletes code it added; S8 grants it the three
  backends it never had. Its count assertions become this plan's oracle.
- **@PLN23** — its client becomes the ONLY driver, which raises the bar on it: no
  longer a fixture but the thing core depends on. Its S5–S7 ORM shares one
  derivation with the lazy read.
- **@PLN24** — option B preserves its thesis (`#c`, no rustc in a library) and
  makes it load-bearing; option A retires it for databases. Its arc F is already
  closed by @PLN23 S1; its arc E (wasm) is untouched by this.
- **@PLN127** — S1 extends its reflection surface, additively.
- **@PLN128** — its E3 float refusal moves from prerequisite to optimisation, on
  the strength of P2.
- **@PLN125** — associated types would let the per-column conversion be one
  generic method rather than one per kind. Not a blocker.

## See also

- [LAZY_STORES.md](../../LAZY_STORES.md) — the read side as built, including the
  derivation this plan moves and the refusals it retires.
- [plans/23-db-clients/OBJECT_MAPPING.md](../23-db-clients/OBJECT_MAPPING.md) —
  the S1–S7 ladder whose write half this depends on.
- [plans/129-lazy-database-stores/BINDING.md](../129-lazy-database-stores/BINDING.md)
  — the binding contract, whose "no DDL, no migrations" decision requirement 3
  revises.
- [plans/24-c-abi-binding/README.md](../24-c-abi-binding/README.md) — the `#c`
  machinery, the trampoline ladder, and the capability gap this design routes
  around.
- [INTERFACES.md](../../INTERFACES.md) — `SqlDb` is an ordinary bounded generic;
  § Interpolation targets is how its statements are built safely.
