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
*zero-rebuild, mmap-the-shipped-file* model.  Open it only when the
@PLAN28 snapshot has shipped and the mmap payoff is next.

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
**per-library JSON snapshot** (loft's own database JSON, not serde —
user-accepted 2026-05-31) that re-parses + rebuilds native `Data` on
load.  Second-class (JSON is re-parsed, not mmap'd) but delivers the
cold-start win without the IR rewrite.  This plan **supersedes** it:
the store struct-enum format replaces JSON and turns the rebuild into a
zero-copy mmap.  @PLAN28 builds the stop-gap format-agnostic so this
plan swaps the encoder underneath without touching startup wiring.

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
| **A** — IR store schema | Define the struct-enum schema for `Value`/`Type`/`Definition`/`Function`/`Attribute` as loft type registrations (the `init`-equivalent for the compiler's own types). | Open — needs design |
| **B** — write path | Materialize a parsed native `Data` into store records (validates the schema; reuses @PLAN28 snapshot work if it landed store-format). | Open |
| **C** — read accessors | `data.def(dnr)` + `value` / `type` matching read from the store instead of `Vec`/`Box`.  The ~940-site migration. | Open — the bulk |
| **D** — mmap load | `Data::open(path)` → `Store::open` → live IR, zero rebuild.  Wire into the startup path behind the bundle cache key. | Open |
| **E** — bundle snapshots | Core `stdlib.store` (shared) + per-script bundle snapshot (core + sorted lib-set), each keyed for drift. | Open |

## Phase ordering

1. **A (schema)** — pin the store schema for the IR types.  Load-bearing;
   everything depends on it.  Prototype by hand-registering `Value`/`Type`
   schemas and round-tripping one `Definition`.
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
