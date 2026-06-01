<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 54 — `Data` as a store (IR mirrors the `--native` data model)

## Status

Open — design, no implementation.  This is the **mmap end-goal** that
[@PLAN28 startup-cache](../../deferred/28-const-store/STARTUP_CACHE_PLAN.md)
named but deferred: rework the compiler's in-memory IR (`Data` and the
`Value` / `Type` / `Definition` / `Function` graph) so it lives in a
`Stores` instance addressed by `DbRef` — **the same representation
`loft --native` already generates for user struct-enums** — instead of
native Rust `Vec<Definition>` / `Box<Value>` / `String`.  Once the IR
is store-backed, `Store::open(path)` (which already mmaps, zero-copy —
`src/store.rs`) loads a precompiled `Data` with **no rebuild step**,
collapsing cold-start parse time (~14.7 ms of the ~17 ms baseline,
measured in @PLAN28 Step 0) to a page-fault.

Large and invasive — touches the ~940 `data.def(...)` read sites and
every `match value { Value::Call(..) }` in parser / codegen / scope
analysis / native generation.  **Not** required for @PLAN28's cold-start
win (a rebuild-on-load snapshot gets that); required for the
*zero-rebuild, mmap-the-shipped-file* model.

**Now promoted to next (2026-06-01).**  @PLAN28 proved by measurement that
a JSON snapshot **cannot** beat the parser — both deserialize text into the
same heap graph (~15–24 ms load ≈ ~11–23 ms parse; see @PLAN28 § Step 3).
So the cold-start goal is unreachable by any serialization format and falls
to *this* plan's zero-copy mmap.  @PLAN28 did not ship the cold-start win,
but it shipped the **reusable foundation** this plan needs:

- the exhaustive `Data` / `Value` / `Type` / `Definition` / `Function`
  traversal (`src/ir_schema.rs`) — arc B's native→store materializer is the
  same walk with a store-writer sink instead of a JSON sink;
- the database-schema enumeration (`src/database/snapshot.rs`) — arc A's
  schema spec;
- `compare_data` — arc C's native-vs-store equivalence oracle;
- `LOFT_DUMP_SNAPSHOT` + the `from_snapshot` `done=true` skip-`scopes::check`
  insight — arc D debugging / load wiring.

**First concrete step: arc A0** — a typed `Record` field cursor over
`Store`'s raw int primitives (§ Arc A0), the precondition for the accessor
seam.  Pure-additive, ships standalone.

**Standalone upside — a functional inspection library inside the database
(2026-06-01).**  Independent of the cold-start goal, @PLAN28's codec already
gives `Database` a rich, working **serialise / deserialise / compare** layer
over the IR and the store schema (`*_to_json` / `*_from_json` /
`schema_to_json` / `compare_data` / `LOFT_DUMP_SNAPSHOT`), proven lossless on
the real stdlib.  That is immediately useful as **inspection + debugging
tooling** — dump any parsed `Data` or `Stores` schema to readable JSON, diff
two compilations field-by-field, regression-pin IR shapes in tests — and it
keeps paying off *throughout* this plan: every arc (A schema, B write, C
per-subsystem swap, D mmap) can dump-and-eyeball or `compare_data` its
intermediate state against a fresh parse.  So the serialisation work is not
sunk cost from a closed JSON-cache idea; it is a permanent database facility
that also happens to seed the mmap path.

## Goal

Represent the compiler IR (`Data` + `Value`/`Type`/`Definition`/
`Function`) as `Stores` records using the same struct-enum store schema
`loft --native` emits, so a precompiled `Data` can be `mmap`-ed from
disk into a live, queryable IR with zero deserialization.

## What gets cached — two snapshots, both whole-prefix

Scope is deliberately **two** snapshot kinds, both *deterministic-
parse-order prefixes* — never independent per-library files:

1. **Core stdlib** — the always-loaded `default/*.loft` prefix.  Parsed
   first, in fixed order, on every run, so its def_nr / `known_type`
   layout is identical every time → one shared, shipped `stdlib.store`
   that every program mmaps.
2. **Full per-script bundle** — core **plus the exact set of libraries
   the script `use`s**, snapshotted as one unit (core + sorted lib-set).
   Keyed on the bundle (`stdlib_cache_key` + the sorted lib list + lib
   content hashes).  A repeated run of the *same* script / app mmaps its
   whole compiled `Data` — stdlib **and** its libs — with zero parse.

**Explicitly out of scope:** independent per-library mmap that composes
arbitrary libs on demand.  Not needed — caching the whole bundle
sidesteps it.

**Interim stop-gap (precedes this plan):** @PLAN28 Step 2 ships a
**whole-stdlib / whole-bundle JSON snapshot** (loft's own database JSON,
not serde — user-accepted 2026-05-31) that rebuilds native `Data` on
load.  Second-class (JSON is re-parsed, not mmap'd) but delivers the
cold-start win without the IR rewrite.  This plan **supersedes** it: the
store struct-enum format replaces JSON and turns the rebuild into a
zero-copy mmap of the same whole bundle.

**Per-library JSON was considered and deferred** (likely dropped — user,
2026-05-31): a per-library deliverable would close the first-landing gap
for a brand-new `use` combination, but it needs name-based relocation
(it drops into an arbitrary prefix), which is the brittlest part of the
stop-gap and optimizes the least-common case.  So neither @PLAN28 nor
this plan does per-library: both operate on the **whole bundle as one
image** with absolute, internally-consistent indices — no relocation
anywhere.  @PLAN28 builds the stop-gap format-agnostic so this plan
swaps the bundle encoder underneath without touching startup wiring.

## Why the global-index model is fine for this scope

`Data.definitions` is one global `Vec`; core and every `use`d library
**append into it** (`add_def` → `rec = self.definitions()`).
Cross-references are global indices — `Type::Reference(u32,…)`,
`Type::Enum(u32,…)`, `Value::Call(u32,…)` carry a global `def_nr`;
`known_type: u16` indexes the global `database.types` schema.  So a
compiled `Data` is **position-dependent on parse order**.

That is exactly why the scope is whole-prefix snapshots, not per-library
files: a snapshot freezes a *complete* parse-order prefix (core, or
core+libs), so every global index inside it is valid as-is when mmap'd
back — no relocation, zero-copy.  Independent per-library mmap would
need source-relative indexing or a relocation pass; whole-bundle caching
avoids the question.  (`--native` itself uses the same global-index
model — it rebuilds one type space at runtime by calling each crate's
`init()` in sequence — so "mirror `--native`" inherits global indices,
consistent with whole-bundle caching.)

The only cost: the bundle cache key is the whole lib-set, so the
**first** run of a never-seen `use` combination still parses fully; the
win lands on every subsequent run.  For the dogfood consumers (games,
servers, the indexer/viewer) run repeatedly, that's the case that
matters.

## Effort + design

- **Effort:** L (large multi-arc — IR rewrite + access-site migration)
- **Design:** — (needs design; this README is the seed)
- **Last touched:** 2026-05-31

## Why mirror `--native` specifically

`loft --native` already solves "represent loft struct-enums as store
records" (NATIVE.md § Architecture): generated code uses
`loft::database::Stores` + `loft::keys::{DbRef, Str, Key}`, and an
`init(db: &mut Stores)` function registers every type schema via
`db.structure()` / `db.enumerate()` / `db.value()` / `db.vector()`
(NATIVE.md § `output_init`).  The compiler's IR enums (`Value`, 34
variants; `Type`, 24 variants) are themselves recursive struct-enums, so
the representation problem is **already solved for user types** — this
plan applies that machinery to the compiler's own types.

Mirroring `--native` rather than inventing a third format buys:

- **One representation to maintain.**  The store struct-enum format is
  exercised by every `--native` build; the IR rides the same code.
- **mmap for free.**  `Store::open` already maps a file and marks it
  `borrowed` + `locked` (`src/store.rs:309`); a store-backed `Data`
  inherits that with no new persistence code.
- **Position-independent records.**  `DbRef { store_nr, rec, pos }` is
  offset-based (verified in the @PLAN28 audit), so a mapped store is
  valid at any base address — the precondition for mmap.  (Global
  *def_nr* indices are a separate axis, handled by whole-prefix
  snapshotting above.)

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **A0** — typed `Store` field cursor | A `Record` / `RecordMut` wrapper over `Store`'s raw `get_int`/`set_int`/`addr::<T>` primitives: named, bounds-checked, typed field reads/writes so no IR accessor does `(rec, fld)` offset arithmetic directly.  Pure-additive precondition for A/C; ships value standalone (safer `--native` + fill.rs reads). | Open — **next**; see § Arc A0 |
| **A** — IR store schema | Define the struct-enum schema for `Value`/`Type`/`Definition`/`Function`/`Attribute` as loft type registrations (the `init`-equivalent for the compiler's own types).  Reuse @PLAN28's `ir_schema` / `database::snapshot` field enumeration as the schema spec. | Open — needs design |
| **B** — write path | Materialize a parsed native `Data` into store records (validates the schema; reuses @PLAN28 snapshot work if it landed store-format). | Open |
| **C** — read accessors | `data.def(dnr)` + `value` / `type` matching read from the store instead of `Vec`/`Box`.  The ~940-site migration — **done incrementally via the accessor seam, never at once** (see § Incremental migration). | Open — the bulk |
| **D** — mmap load | `Data::open(path)` → `Store::open` → live IR, zero rebuild.  Wire into the startup path behind the bundle cache key. | Open |
| **E** — bundle snapshots | Core `stdlib.store` (shared) + per-script bundle snapshot (core + sorted lib-set), each keyed for drift. | Open |

## Phase ordering

0. **A0 (typed field cursor)** — wrap `Store`'s raw int primitives in a
   `Record`/`RecordMut` cursor.  Pure-additive, no forced callers, green +
   shippable on its own.  Everything in A/C reads through it, so it lands
   first.  See § Arc A0.
1. **A (schema)** — pin the store schema for the IR types.  Load-bearing;
   everything depends on it.  Prototype by hand-registering `Value`/`Type`
   schemas and round-tripping one `Definition` **through the A0 cursor**.
2. **B (write)** — native `Data` → store.  Testable artifact; validates A
   before touching read sites.  If @PLAN28 Step 2 shipped a store-format
   snapshot, B is largely done.
3. **D (mmap load, read-mostly)** — load a store-backed `Data` and run
   execution against it *while the parser still builds native `Data`*.
   Proves the read path on the hot loop before the full migration.
4. **C (access-site migration)** — convert `data.def()` and the IR
   `match` arms incrementally, by subsystem (state/ first, then
   generation/, then parser/).
5. **E (bundle snapshots)** — core snapshot first (deterministic, shared);
   per-script bundle snapshot second.

## Arc A0 — typed `Store` field cursor (the precondition, next)

**Why this is the first move.**  This plan's safety rests on the *accessor
seam* (§ Incremental migration): every IR read goes through a method, so a
representation can be swapped behind it without touching call-sites.  But the
seam's methods will ultimately read **store records**, and `Store`'s current
read API is untyped offset arithmetic:

```rust
// today — src/store.rs, the raw primitives every store read uses:
store.get_int(rec, fld) -> i64          // i64::MIN sentinel on invalid
store.get_u32_raw(rec, fld) -> u32      // raw 4-byte (collection headers)
store.get_i32_raw / get_long / get_short / get_byte / get_float /
store.get_single / get_boolean / get_str(rec) -> &str
store.addr::<T>(rec, fld) -> &T         // unchecked typed pointer
store.set_int(rec, fld, val) -> bool    // … + the set_* counterparts
```

Building ~940 IR accessors directly on `(rec, fld)` arithmetic means 940
chances to fumble a field offset or pick the wrong width.  A typed cursor
collapses each read to a named, bounds-checked call **before** any IR type
moves into a store — a pure refactor with no behaviour change, exactly the
"additive, off the critical path, reversible" property the rest of the plan
requires.

**The wrapper (shape, not final API).**

```rust
/// A typed, read-only view of one record in one Store.  All offset
/// arithmetic and width selection lives here, not at the call-site.
pub struct Record<'a> { store: &'a Store, rec: u32 }
pub struct RecordMut<'a> { store: &'a mut Store, rec: u32 }

impl<'a> Record<'a> {
    pub fn int(&self, fld: u32) -> i64;        // → get_int
    pub fn u32(&self, fld: u32) -> u32;        // → get_u32_raw (headers)
    pub fn i32(&self, fld: u32) -> i32;        // → get_i32_raw
    pub fn long(&self, fld: u32) -> i64;
    pub fn float(&self, fld: u32) -> f64;
    pub fn single(&self, fld: u32) -> f32;
    pub fn boolean(&self, fld: u32, mask: u8) -> bool;
    pub fn byte(&self, fld: u32, min: i32) -> i32;
    pub fn dbref(&self, fld: u32) -> DbRef;    // 12-byte stored pointer
    pub fn str(&self, fld: u32) -> &str;       // follow Str record
}
// RecordMut mirrors with set_* ; both obtained via Store::record(rec) /
// Store::record_mut(rec).
```

**Scope of A0 (deliberately narrow — one PR):**
- add `Record`/`RecordMut` + `Store::record(rec)` / `record_mut(rec)`;
- delegate each method to the existing `get_*`/`set_*` primitive (no new
  unsafe — reuse the validated paths, including their sentinel semantics);
- **do NOT** migrate any caller in this PR — additive only.  Optionally
  convert one self-contained reader (e.g. a `database/format.rs` debug dump)
  as a proof-of-use, but the 940-site sweep is later arcs.

**Why it ships standalone (value independent of plan-54):** the same raw
`get_int(rec, fld)` arithmetic is used pervasively by `--native` codegen and
the 233 `fill.rs` opcode bodies.  A typed cursor is a readability + safety win
there immediately, so A0 is a clean green PR on its own merits — not dead
weight waiting on the rest of the arc.

**Acceptance:** unit tests round-trip each width through `Record`/`RecordMut`
on a scratch store and assert identical results to the raw `get_*`/`set_*`
calls (the cursor is a faithful pass-through, including invalid-access
sentinels).  `make ci` green; no behaviour change anywhere else.

**Open A0 questions:**
- Width-by-`Parts`: a record's field widths come from its `Type.parts`
  schema; does the cursor stay schema-agnostic (caller passes the width
  method, as above) or take a `&Type` and dispatch?  Start schema-agnostic
  (smaller, matches the raw API 1:1); a schema-aware layer can wrap it later.
- Lifetime of `str()`: `Store::get_str` returns `&'a str` from a raw pointer
  (`Str` semantics) — the cursor inherits that contract unchanged; document
  it, don't try to fix it in A0.

## Incremental migration — arc C is many small plans, never one

The ~940 `data.def(...)` reads and the `match value { … }` /
`match type { … }` arms cannot move to a store-backed representation in
a single change without breaking the project.  Arc C is therefore a
**series of follow-up plans**, each green and shippable on its own,
enabled by an **accessor seam** introduced *before* any representation
changes.

**The seam (precondition, cheap, additive).**  Route every IR read
through accessor methods instead of touching fields directly:
`data.def(d).name()`, `.returned()`, `.code()`, … and small helpers
over `Value` / `Type` (e.g. `value.as_call()`, `ty.as_reference()`)
that today just `match` the native enum.  This is a pure refactor with
**no behaviour change** — the native `Vec`/`Box` stays underneath — so it
lands incrementally under the normal green-commit discipline and is a
valid stop at any point.  Once a subsystem reads only through the seam,
its representation can be swapped without touching that subsystem again.

**Then migrate behind the seam, one slice per follow-up plan:**

1. **Seam-only plan** — introduce the accessor methods; convert
   call-sites to them mechanically, subsystem by subsystem
   (`state/` → `generation/` → `parser/`).  Representation unchanged.
   Each subsystem is its own commit; the build is green throughout.
2. **Per-subsystem representation swap** — with `state/` reading only
   through the seam, move *its* reads to the store accessor; leave the
   rest on native.  A **dual-backed `Data`** (native `Vec` *and* the
   store, kept in sync during the transition) lets one subsystem read
   from the store while others still read native — this is what makes
   "not at once" possible.  Repeat per subsystem.
3. **Drop the native backing** — once every subsystem reads from the
   store, delete the native `Vec`/`Box` fields and the sync.  Only now
   is `Data` truly store-backed; only now does mmap (arc D) become
   zero-copy for *reads*, not just load.

**Why this is safe the same way @PLAN28's ladder is:** each step is
additive (the seam adds methods, doesn't remove fields), off the
critical path until proven (dual-backing runs both representations and
can assert they agree), and reversible (revert one subsystem's swap
without touching others).  A per-subsystem **equivalence assertion**
(native read == store read, behind a debug flag) is the analogue of
@PLAN28 S3's bytecode gate.

**Plan shape:** the seam is one small plan; each subsystem swap is its
own follow-up plan (or `## Open work` row if it stays small).  None of
them is the whole arc — that is the point.

**Pacing discipline (the real constraint — user, 2026-05-31):** the plan
document is long-lived and that is fine; what matters is that **every
pass finishes fully — lands as a complete, reviewed, merged PR with CI
green — before the next pass begins.**  This is stronger than "one plan
at a time": no pass may start while a previous pass is half-done on a
branch.  One pass = one PR = `main` is releasable again.  A "pass" is a
single seam-conversion-of-one-subsystem, or a single subsystem's
representation swap — sized so it completes and merges as a unit.  The
dual-backing + equivalence assertion exist precisely so each such PR is
independently mergeable without the rest of the arc.  This same
finish-before-continue rule governs @PLAN28's S1–S5 rungs.

## Open design questions

1. **Mutability.**  The parser mutates `Data` heavily during two-pass
   parsing; a mmap'd store is `locked`.  Likely resolution: parse into a
   writable store (or native `Data`), freeze + persist; at runtime read
   the locked mmap'd bundle store + a writable store for any
   user-program defs.  Mirrors the CONST_STORE locked-after-build pattern
   (`src/compile.rs:52`).
2. **`&'static str` / interned labels.**  `Block.name`,
   `Definition.synthetic` are `&'static str`.  In a store they become
   `Str`/record offsets; native `match name { "if" => }` sites need a
   store-string comparison path (or an interned-id scheme).
3. **`OnceLock` caller-index.**  `Data.caller_index` is a derived cache —
   rebuilt on load, never stored.
4. **Bundle drift detection.**  A mmap'd bundle must be rejected if the
   inputs changed — reuse + extend the @PLAN28 `stdlib_cache_key`
   (version + build-id + feature set) with the sorted lib list and lib
   content hashes.
5. **Store-read vs `Vec`-index perf.**  `data.def()` is a `Vec` index
   today (~940 sites, hot).  A `DbRef` read adds store indirection;
   measure the hot-path delta and whether a read-through cache is needed.

## Cross-arc dependencies

- **[@PLAN28 startup-cache](../../deferred/28-const-store/STARTUP_CACHE_PLAN.md)**
  — direct predecessor.  @PLAN28's rebuild-on-load snapshot delivers the
  cold-start win first; this plan removes the rebuild.  If @PLAN28 Step 2
  serializes into the **store struct-enum format**, arc B here is mostly
  done.  serde is forbidden project-wide (CODE.md) — this plan uses the
  store format, consistent with that.
- **[@PLAN38 loft-store-durable](../38-loft-store-durable/)** — shares the
  `Store::open_durable` / persistence surface; coordinate the on-disk
  store format so the IR store and durable user stores stay compatible.
- **NATIVE.md / `src/generation/`** — source of the struct-enum
  schema-emission pattern this plan mirrors; arc A reuses `output_init`'s
  registration approach.

## Relationship to self-hosting (loft compiler in loft)

A full loft-in-loft rewrite — parser, type checker, scope analysis, and
codegen written in loft, running on the interpreter, fast enough to
compile itself — is anticipated but is **a 2.0-scale undertaking, far on
the horizon** (not a 1.0 goal).  This plan is **not an alternative to it;
it is a strict down-payment on it**, chosen because it is the smallest
reversible slice of the same problem and is valuable on its own merits
(cold-start) long before the rewrite is on the table.

**Shared hard problem.**  Self-hosting must represent the compiler's IR
(`Data` / `Value` / `Type` / `Definition`) as loft's own data — there is
no way to write a loft compiler in loft without it.  This plan answers
exactly that question for the data model alone, in the
already-`--native`-validated store format.  Whatever schema this plan
pins (arc A) is the schema a self-hosted front-end would consume.

**Reversibility ladder.**  Each rung's non-throwaway work feeds the
next; enter self-hosting through this keyhole, not head-on:

| Rung | Effort | Permanent contribution to self-hosting |
|---|---|---|
| @PLAN28 JSON stop-gap | days | proves loft data *can* hold the IR; ships the whole-bundle cold-start win (per-library JSON considered + deferred as too brittle) |
| **plan-54 (this)** | L | the store-backed IR schema + read accessors — the first *permanent* self-hosting foundation |
| full loft-in-loft | multi-quarter | the destination |

**This plan removes a self-hosting blocker.**  Self-hosting makes the
interpreter's parse-bound cold-start *worse* (a loft compiler is a
compile-heavy workload on the interpreter).  The startup cache + this
plan attack precisely that bottleneck, so they clear the runway rather
than compete for it.

**Gate, not commitment — and a 2.0 horizon.**  Full self-hosting is a
**2.0-scale** target, deliberately past 1.0.  Two gates keep it there:
(1) language maturity — writing a large compiler in loft before the
syntax settles means writing it twice, so it waits until the 1.x line is
stable; (2) this plan must first prove the IR-in-loft-data model is
**both ergonomic to express and fast enough to read** (open questions 2
and 5).  If `Data`-as-store turns out pleasant and the hot-path read
delta acceptable, self-hosting is materially de-risked; if painful, that
lesson is learned here cheaply, on the smallest slice, before betting a
2.0-scale arc on the rewrite.  Nothing in this plan *commits* to the
rewrite — it only makes the eventual decision cheaper and better-informed.

## See also

- [NATIVE.md](../../../NATIVE.md) — how `--native` represents data as
  `Stores` records (the model this plan mirrors); § Architecture,
  § `output_init`.
- [DATABASE.md](../../../DATABASE.md) — `Stores`, `Store`, `DbRef`,
  word-addressed records, CONST_STORE.
- [@PLAN28 STARTUP_CACHE_PLAN.md](../../deferred/28-const-store/STARTUP_CACHE_PLAN.md)
  — the cold-start cache; its "Architecture C — Data *is* the store" is
  this plan's seed.
- `src/store.rs::Store::open` — the mmap entry point this plan loads
  through.
- `src/data.rs` — the IR types (`Data`, `Value`, `Type`, `Definition`)
  being migrated.
