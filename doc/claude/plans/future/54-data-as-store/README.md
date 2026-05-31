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
`Stores` instance addressed by `DbRef`, **the same representation
`loft --native` already generates for user struct-enums**, instead of
native Rust `Vec<Definition>` / `Box<Value>` / `String`.  Once the IR
is store-backed, `Store::open(path)` (which already mmaps, zero-copy —
`src/store.rs`) loads a precompiled stdlib `Data` with **no rebuild
step**, collapsing cold-start parse time (~14.7 ms of the ~17 ms
baseline, measured in @PLAN28 Step 0) to a page-fault.

This is a large, invasive plan — it touches the ~940 `data.def(...)`
read sites and every `match value { Value::Call(..) => }` in the
parser, codegen, scope analysis, and native generation.  It is **not**
required for the @PLAN28 cold-start win (a rebuild-on-load snapshot
gets that); it is required for the *zero-rebuild, mmap-the-shipped-file*
model.  Open it only when the startup-cache snapshot has shipped and
the mmap payoff is the next priority.

## Goal

Represent the compiler IR (`Data` + `Value`/`Type`/`Definition`/
`Function`) as `Stores` records using the same struct-enum store schema
`loft --native` emits, so a precompiled stdlib `Data` can be `mmap`-ed
from disk into a live, queryable IR with zero deserialization.

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
variants; `Type`, 24 variants) are themselves recursive struct-enums.
So the representation problem is **already solved for user types** — this
plan applies that same machinery to the compiler's own types.

Mirroring `--native` (rather than inventing a third format) buys:

- **One representation to maintain.**  The store struct-enum format is
  exercised by every `--native` build; the IR rides the same code.
- **mmap for free.**  `Store::open` already maps a file and marks it
  `borrowed` + `locked` (`src/store.rs:309`); a store-backed `Data`
  inherits that with no new persistence code.
- **The format is position-independent.**  `DbRef { store_nr, rec, pos }`
  is offset-based (verified in the @PLAN28 audit), so a mapped store is
  valid at any base address — the precondition for mmap.

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **A** — IR store schema | Define the struct-enum schema for `Value`/`Type`/`Definition`/`Function`/`Attribute` as loft type registrations (the `init`-equivalent for the compiler's own types). | Open — needs design |
| **B** — write path | Materialize a parsed native `Data` into store records (validates the schema; reuses the @PLAN28 snapshot work if it landed store-format rather than JSON). | Open |
| **C** — read accessors | `data.def(dnr)`, `value` matching, `type` matching read from the store instead of `Vec`/`Box`.  The 940-site migration. | Open — the bulk |
| **D** — mmap load | `Data::open(path)` → `Store::open` → live IR, zero rebuild.  Wire into the startup path behind the @PLAN28 cache key. | Open |
| **E** — write-during-parse | The parser mutates `Data` as it builds it; decide whether parsing writes store records directly or builds native then converts (B). | Open — design question |

## Phase ordering

1. **A (schema)** — pin the store schema for the IR types.  This is the
   load-bearing design step; everything else depends on it.  Prototype
   by hand-registering `Value`/`Type` schemas in a `Stores` and
   round-tripping one `Definition`.
2. **B (write)** — native `Data` → store.  Gives a testable artifact and
   validates A before touching read sites.  If @PLAN28 Step 2 shipped a
   store-format snapshot, B is largely done.
3. **D (mmap load, read-mostly)** — load a store-backed `Data` and run
   execution against it *while the parser still builds native `Data`*.
   Proves the read path on the hot interpreter loop before the full
   migration.
4. **C (access-site migration)** — convert `data.def()` and the IR
   `match` arms incrementally.  Largest, most invasive arc; sequence by
   subsystem (state/ first, then generation/, then parser/).
5. **E (parse writes store)** — last; only if eliminating the native
   build-then-convert step is worth it.  May stay deferred indefinitely
   (B+D+C already deliver the mmap win for the *shipped* stdlib).

## Open design questions

1. **Mutability.**  The parser mutates `Data` heavily during two-pass
   parsing; a mmap'd store is `locked`.  Resolution likely: parse into a
   writable store (or native `Data`), freeze + persist; user code at
   runtime reads the locked mmap'd stdlib store + a writable store for
   user defs.  Mirrors the CONST_STORE locked-after-build pattern
   (`src/compile.rs:52`).
2. **`&'static str` / interned labels.**  `Block.name`,
   `Definition.synthetic` are `&'static str`.  In a store they become
   `Str`/record offsets; the native-side `match name { "if" => }` sites
   need a store-string comparison path (or an interned-id scheme).
3. **`OnceLock` caller-index.**  `Data.caller_index` is a derived cache
   — rebuilt on load, never stored (same as it would be `#[serde(skip)]`).
4. **Schema-drift detection.**  A mmap'd stdlib store must be rejected
   if the schema changed — reuse the @PLAN28 `stdlib_cache_key`
   (version + build-id + feature set), already built in `src/cache.rs`.
5. **Performance of store reads vs `Vec` index.**  `data.def()` is a
   `Vec` index today (940 sites, hot).  A `DbRef` read adds store
   indirection; measure whether the hot path regresses and whether a
   read-through cache is needed.

## Cross-arc dependencies

- **[@PLAN28 startup-cache](../../deferred/28-const-store/STARTUP_CACHE_PLAN.md)**
  — direct predecessor.  @PLAN28's rebuild-on-load snapshot delivers the
  cold-start win first; this plan removes the rebuild.  If @PLAN28 Step 2
  serializes into the **store struct-enum format** (Architecture B in
  that plan's evaluation), arc B here is mostly done.  serde is
  forbidden project-wide (CODE.md) — this plan uses the store format,
  consistent with that.
- **[@PLAN38 loft-store-durable](../38-loft-store-durable/)** — shares
  the `Store::open_durable` / persistence surface; coordinate the
  on-disk store format so the IR store and durable user stores stay
  compatible.
- **NATIVE.md / `src/generation/`** — the source of the struct-enum
  schema-emission pattern this plan mirrors; arc A reuses
  `output_init`'s registration approach.

## See also

- [NATIVE.md](../../../NATIVE.md) — how `--native` represents data as
  `Stores` records (the model this plan mirrors); § Architecture, §
  `output_init`.
- [DATABASE.md](../../../DATABASE.md) — `Stores`, `Store`, `DbRef`,
  word-addressed records, CONST_STORE.
- [@PLAN28 STARTUP_CACHE_PLAN.md](../../deferred/28-const-store/STARTUP_CACHE_PLAN.md)
  — the cold-start cache; § "Architecture C — Data *is* the store" is
  this plan's seed.
- `src/store.rs::Store::open` — the mmap entry point this plan loads
  through.
- `src/data.rs` — the IR types (`Data`, `Value`, `Type`, `Definition`)
  being migrated.
