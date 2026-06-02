<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 54 — `Data` as a store (IR mirrors the `--native` data model)

## Status

**In-progress — arc A0 + arc A landed; arc B's write path COMPLETE; arc C's store→native reader COMPLETE and the round-trip is now FULLY LOSSLESS.**  The whole real `default/` stdlib round-trips `native → store → native` **bit-for-bit** — the full `compare_data` oracle (every `Definition` field including the per-function variable table: `vars` + `names` + `inline_refs`) is green across all definitions.  The store schema was grown (`Function` gained `names: vector<NameNr>` + `inline_refs: vector<integer>`) after confirming neither is reconstructible from the variable list and that codegen reads both on the load path.  A store-materialised `Data` is now indistinguishable from a fresh parse.  **Arc D probe (proven end-to-end):** the full mmap loop works today — materialize the stdlib into a **file-backed** store, drop, reopen via `Store::open` (mmap), and `read_data` rebuilds the native `Data` with **no re-parse and no schema registration** — **~12× faster than `parse_dir`** (0.92 ms vs 11.4 ms median; see § Open design questions Q5).

- **Arc A0** (typed field cursor, commit `a07ed8d`) — landed as `RecordCursor`/`RecordCursorMut` wrapping `Store`'s raw primitives.  That cursor form has since been superseded by the typed handle layer (see § Arc A0 — handle layer below); `src/data_store.rs` is now the accessor seam, not a bare cursor.
- **Arc A** (IR store schema, commit `ed21b3e`) — landed as `tools/ir_schema/` (hybrid generate-extract pipeline) + `src/ir_schema_gen.rs` (generated, checked-in).  The full IR is registered via `register_ir_schema(db: &mut Stores) -> IrSchemaIds`; every struct/enum is in the schema; `db.finish()` computes all field positions, record sizes, and discriminants including the 34-variant `Node` enum size.
- **Typed handle layer** (`src/data_store.rs`, commit `9d860c5`) — minimum accessor seam: `Value`/`ValuesVector` thin `DbRef` handles with `ValueType` enum covering the IR-walker's current match surface.  Three tests pass (NdCall round-trip, NdBlock round-trip, layout guard).  Fmt-clean, clippy-0.
- **Arc B fork-cleanup (prerequisite, done)** — removed the dead shells-only `ir_schema::register_ir_schema` + its consts/tests, leaving exactly one schema registration (`ir_schema_gen`).  The @PLAN28 JSON codec stays (interim — arc B's traversal skeleton + `compare_data` oracle); its 30 lib tests + 6 round-trip tests still pass.
- **Arc B write path (in-progress)** — both recursive IR enums now materialize fully. `src/data_store.rs` is the write/layout authority: per-variant `Node` writers + generic typed field accessors (`field_int`/`set_field_int`, …float/single/bool/str, `field_vec`/`field_recvec`, `set_discriminant`), `ValueType`/`value_type` over all 34 `Node` variants and `TypeKind`/`type_kind` over all 24 `TypeT` variants, plus a **generic non-`Node` struct-vector layer** (`Record` + `RecVector`, stride-parameterised) — built on the probed fact that **every IR vector is inline `Parts::Vector`, never a linked `Array`**, so one handle serves `vector<Key>`/`vector<TypeT>`/`vector<integer>`/`vector<SortKey>`/`vector<NameRef>`.  `src/ir_store.rs` materializes **all 34 `Node` variants** (`materialize_node`, now an exhaustive match — no deferred arm) **and all 24 `TypeT` variants** (`materialize_type`, with `IntegerSpec` inline + `SortKey`/`NameRef`/`integer` dep lists + box-of-one recursion).  Every baked discriminant + offset + stride is pinned by the `baked_layout_mirrors_loft_schema` guard (probed from the real schema, not guessed; inline sub-struct offsets verified as base + relative).  `Attribute`, `LinkedFieldGroup`, the full `Block`, and the top-level structs `Variable`/`Function` (via the `variables/mod.rs` snapshot seam) + `Definition` (23 fields, inlining `Position` + `Function`) + `Data` now materialize.  **`ir_store::materialize_data(&Data) -> DbRef` is the capstone entry point — the entire native `Data` writes into a store, exercised on the real `default/` stdlib** (`materialize_whole_stdlib_smoke`: every definition name, attribute count, and variable count round-trips through the store).  **Arc B's write path is complete.**  18 lib tests green; whole 438-test lib suite green; fmt-clean, clippy-0.

  Finding (fixed): `Store::claim` reuses freed blocks without zeroing, so a freshly-pushed vector element carried garbage in its unwritten vector-header sub-fields, and the next nested push dereferenced a junk record id (SIGSEGV deep in the real-stdlib walk).  Added `Store::zero_range`; `ValuesVector::push`/`RecVector::push` now clear each new element (mirrors the generated `--native` code that zeroes vector-header slots, `codegen_runtime.rs:1481`).

  **Remaining (arc C territory):** a store→native **read** path so the materialized store can be validated bit-for-bit by `compare_data` against a fresh parse.  ✅ **Done** — `src/ir_read.rs` (`read_value`/`read_type`/`read_data`) + the `Function.names`/`inline_refs` schema growth make `compare_data` green on the whole stdlib (see the arc C bullets below).  Arcs D/E remain open; arc C's bulk read-site migration remains.

- **Arc C read path (in-progress)** — `src/ir_read.rs` is the store→native reader, the exact inverse of `ir_store.rs`.  **`read_value(&Stores, Node) -> Value`** rebuilds all 34 `Node` variants and **`read_type(&Stores, Record) -> Type`** rebuilds all 24 `TypeT` variants, plus every sub-struct reachable from them (`Block`, `ParForBody`, `Position`, `Key`, `IntegerSpec`, `vector<SortKey>`/`vector<NameRef>` key lists, `vector<integer>` dep lists).  Box-of-one `vector<…>` fields read back as `Box<Value>`/`Box<Type>`; N-element vectors as `Vec`.  `Block.name` (`&'static str`) is reconstructed via a bounded `Box::leak`, mirroring the @PLAN28 JSON decoder (open question 2).  Validated by **`native → store → native` round-trips asserted with the IR's own derived `PartialEq`** — a stronger oracle than the JSON re-encode, and needing no JSON.  7 round-trip tests (all `Value` leaves + recursive/box-of-one/Block/Loop/Span/ParFor/Keys/FnRef variants; all 24 `Type` variants + nested recursion; an explicit `forced_size` check since `IntegerSpec`'s `PartialEq` ignores it).  445-test lib suite green; fmt-clean, clippy-0.

- **Arc C Definition/Data reader (complete)** — `src/ir_read.rs` now also has **`read_data(&Stores, DbRef) -> Data`** (the inverse of `materialize_data`) plus `read_definition` / `read_attribute` / `read_field_group` / `read_function`, the inline-`Position`/`Function` readers, `def_type` / `purity` integer-code inverses, and `Vec<u32>`/`Vec<String>`/`Vec<u16>` list readers.  Derived state is reset exactly as the @PLAN28 JSON loader does (`attr_names` rebuilt from the attribute list; `code_position`/`code_length`/`const_ref` recomputed by the compile pass; `Data::rebuild_indices` re-derives the lookup maps).  Two whole-stdlib capstones, both green on the real `default/`: (1) `read_whole_stdlib_round_trips_except_var_names` — **every** definition's non-variable fields round-trip **bit-for-bit** (`definition_to_json` equality with the variable block blanked) and the per-variable nine codegen-read fields round-trip exactly; (2) `read_stdlib_type_level_defs_full_compare_data_green` — the **full** `compare_data` oracle (including the variable block) is green for all 50+ type-level defs (empty variable tables).  447-test lib suite green; fmt-clean, clippy-0.

  **Finding (confirmed, now resolved) — `Function.names` / `inline_ref_vars` were not reconstructible; the store schema grew to hold them.**  The plan's earlier note ("rebuildable from the variable list on load") did **not** hold: `names` is pruned on scope exit (a finished function's `names` map is a *subset* of its variable list — scope-removed entries are gone — so the var list can't faithfully rebuild it), and `inline_ref_vars` is compile-derived (`insert_inline_ref` during scope analysis), absent from the nine stored per-`Variable` fields entirely.  Both are needed for the **mmap end goal**: the @PLAN28 snapshot seam (`variables/mod.rs`) is explicit that codegen **reads** `names` + `inline_ref_vars` on the load path, so a mmap'd `Data` is unusable without them.

- **Arc C schema-growth pass (complete)** — `Function` in `tools/ir_schema/ir.loft` gained `names: vector<NameNr>` (`struct NameNr { name: text, nr: integer }`) and `inline_refs: vector<integer>`; `extract.py` learned the new `NameNr` type; `ir_schema_gen.rs` was regenerated.  The growth shifted the inlined-`Function` tail of `Definition` by +8 bytes (`Function` 12→20; `DEFINITION_STRIDE` 142→150; `DEF_MUTATED_CAPTURES`…`DEF_PUB_VISIBLE` +8) and added `FN_NAMES`/`FN_INLINE_REFS` + the `NameNr` consts — all probed from the regenerated schema and pinned by the `baked_layout_mirrors_loft_schema` guard.  `ir_store::write_function` now writes both vectors; `ir_read::read_function` reads them (no more best-effort reconstruction).  **Result:** `read_whole_stdlib_compare_data_green` — the full `compare_data` oracle on the entire real stdlib — is green; `read_stdlib_function_variables_round_trip` confirms 20+ populated function variable tables re-encode identically.  447-test lib suite green; fmt-clean, clippy-0.

### Bulk read-site migration — slice 1: `state/` `Definition` accessor seam (complete)

The store↔native representation is proven lossless; arc C's remaining work is the **bulk read-site migration** (route the ~940 IR-read sites through accessor methods, so each subsystem's representation can later swap to store-backed — § Incremental migration).  Slicing decision (user, 2026-06-02): do it **subsystem by subsystem**, and **within `state/`, the `Definition` field-accessor seam first** (the tractable, pure-refactor part), deferring the 451 `Value`/`Type` enum-match sites in `state/codegen.rs` to a later slice (those need handle-based dispatch, a real restructuring, not an `as_call()`-style seam).

Slice 1 landed: added read-accessor methods on `Definition` for the **store-backed** fields `state/` reads — `name()` / `native()` / `source()` / `position()` / `attributes()` / `code()` / `returned()` / `op_code()` / `known_type()` / `variables()` — returning the shapes a future store swap can produce (`&str` / `&[Attribute]` / `&Type` / `&Value` / `&Position` / `&Function` / Copy scalars).  Converted every `data.def(d).FIELD` and local `def.FIELD` read in `state/{mod,debug,codegen}.rs` (~120 sites) to the methods.  The codegen-**derived** fields `code_position` / `code_length` are deliberately **not** seamed — they are recomputed on load, never stored, so they stay native field reads.  Pure refactor, **no behaviour change**; the full integration suite passes.  fmt-clean, clippy-0.

**Next slices (open):** `state/codegen.rs`'s `Value`/`Type` walk (handle-based dispatch — the 451 matches); then `generation/`, then `parser/`; then the per-subsystem representation swap (dual-backed `Data` + equivalence assertion).

### Arc D probe — the mmap load loop works end-to-end (~12× faster than parse)

`ir_store::materialize_data_at(stores, root, data)` (a thin variant of `materialize_data` that writes into a caller-provided root record) lets the IR materialize **directly into a file-backed store** (`Store::open(path)`).  The regression test `ir_read::tests::mmap_file_round_trip_stdlib` proves the whole loop on the real stdlib: materialize → file-backed store → drop (mmap flush) → reopen via `Store::open` (mmap) → `read_data` rebuilds the native `Data`, with **no re-parse and no schema registration** (the reader walks the mapped bytes through baked offsets; `DbRef` is store-relative so the root is rebuilt against the reopened store's slot; the whole IR — records, inline vectors, interned strings — lives in one store, so one file captures everything).  The result is bit-for-bit identical to a fresh parse (`compare_data`).

**Measured (`bench_stdlib_load_mmap_vs_parse`, warm page cache, 25 iters):** producing the native stdlib `Data` via `parse_dir` is **11.4 ms** median; via `Store::open` + `read_data` it is **0.92 ms** median — **~12.4×** (12.6× min).  Store file ≈ **6.9 MiB**.  This is *with* the full native rebuild (`read_data` allocates the `Vec`/`Box`/`String` graph); the speedup comes from skipping lexing + two-pass parsing + type resolution + scope analysis.  Both paths still run codegen→bytecode afterward (unchanged), and @PLAN28 measured parse as ~14.7 ms of the ~17 ms cold-start, so this attacks the dominant chunk.  The representation migration (zero-copy reads, § Incremental migration) removes even the ~0.9 ms rebuild later — but the rebuild path is already a large win, confirming the risk posture that "the store layout is good enough to build on" (Q5).

Still open in arc D: wiring this into the real startup path — the bundle cache key + drift detection (Q4), the locked-mmap mutability split (Q1), and the `caller_index` rebuild (Q3).

Original note: This is the **mmap end-goal** that
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

**First concrete steps: arc A0 and arc A (both done, 2026-06-01).**  Arc A0
landed as a typed field cursor (`a07ed8d`), then evolved into the typed handle
layer (§ Arc A0 — handle layer).  Arc A landed as the `tools/ir_schema/`
hybrid pipeline + `src/ir_schema_gen.rs` (`ed21b3e`).  The minimum accessor
seam (`src/data_store.rs`) landed in commit `9d860c5`.

**A second, mmap-independent payoff — IR locality (user, 2026-06-01).**  The
win here is not only zero-copy load.  The native IR is a pointer graph of
separately-allocated `Box<Value>` / `Vec<Definition>` / `String` / `Box<Type>`
nodes scattered across the heap; the store packs a record's fields contiguously
in one rigorously-laid-out buffer.  That tight layout is cache/prefetch-
friendly, so traversing the store-backed IR may touch far fewer cache lines
than chasing the equivalent `Box` graph — potentially a **net speedup on the
hot walk even before mmap**.  A hypothesis to confirm with numbers (open
question 5), but it means "Data-as-store" can be a structural optimisation in
its own right, not just the cold-start enabler.

**Standalone upside — a functional serialise/inspect layer that *converges*
on the database's own JSON (2026-06-01).**  Independent of the cold-start
goal, @PLAN28's codec already gives a rich, working **serialise / deserialise
/ compare** layer over the IR and the store schema (`ir_schema::*_to_json` /
`*_from_json`, `database::snapshot::schema_to_json`, `compare_data`,
`LOFT_DUMP_SNAPSHOT`), proven lossless on the real stdlib — immediately useful
as inspection + debugging tooling (dump any parsed `Data`/`Stores` to readable
JSON, diff two compilations field-by-field, regression-pin IR shapes), and
useful *throughout* this plan (every arc can dump-and-eyeball or `compare_data`
its intermediate state against a fresh parse).

**But it is NOT yet the database's own JSON — and that gap is the point.**
There are two distinct JSON producers today:

| Producer | Walks | Driven by | Shape |
|---|---|---|---|
| `Stores::show_json` (`src/database/format.rs:69`) | **store records** via `DbRef` + `tp` | the database type schema | the database's native record-JSON |
| `ir_schema::data_to_json` + `database::snapshot::schema_to_json` | the **native** `Data` / `Vec<Definition>` / `Box<Value>` graph (+ `Stores.types` as native structs) | hand-rolled per-type walks | tagged objects `{"k":…}` |

The @PLAN28 codec is a *hand-rolled walk over native Rust IR* — it does **not**
emit the same bytes as `show_json`, because the IR does not yet live in store
records.  **The convergence is exactly arc B:** once the IR is materialised
into store records (arc A schema + B write), `Stores::show_json` walks the IR
*directly* — and the hand-rolled native walk is subsumed by the database's own
serialiser.

**Decision (user, 2026-06-02) — the JSON codec is bootstrap scaffolding, slated
to go.**  The @PLAN28 JSON layer (`ir_schema::*_to_json` / `*_from_json` /
`compare_data` / `LOFT_DUMP_SNAPSHOT`) "was useful to get the wagon rolling but
[is] not for the final goal."  It earns its keep *now* — exhaustive native-IR
walk reused as arc B's traversal skeleton (just swap the JSON sink for a
store-writer sink), and `compare_data` as arc B's equivalence oracle — but it is
**not** a permanent facility and is **not** to be polished into a parallel
`show_json` alternative.  The final state is `Stores::show_json` over the
store-backed IR; the native-JSON codec is **retired** once arc B's store walk is
proven (its `compare_data` validation having served its purpose).  Treat it as
interim throughout: lean on it freely while building, but do not invest in it as
an end state.

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

**Explicitly out of scope — settled, not just deferred (user, 2026-06-02):**
independent per-library mmap / per-library IR snapshot that composes arbitrary
libs on demand.  Two reasons, both permanent:

1. **A library cannot cleanly write its own IR.**  The IR is global-index
   (def_nr / `known_type` are absolute, parse-order-dependent — see § Why the
   global-index model is fine), so a library snapshotted in isolation would need
   name-based relocation into whatever prefix it lands in.  That relocation is
   the brittlest possible part of the system and it buys the least-common case.
2. **The loft source is the better representation of a library's state anyway.**
   For distributing / versioning / inspecting a library, the `.loft` source —
   not a serialized IR image — is the right artifact.  And there is **no
   efficiency case** for a serialized per-library form: @PLAN28 already
   established that (de)serialization is not faster than parsing natural loft
   source (~15–24 ms load ≈ ~11–23 ms parse — see § Status).  So a per-library
   IR cache would be a worse, harder-to-relocate stand-in for something the
   source already expresses well *and* parses just as fast.

Caching the **whole bundle** (core + the script's sorted lib-set) sidesteps both
— every index inside one image is internally consistent, no relocation anywhere.
Closed in the decision register: [DESIGN_DECISIONS.md § C69](../../../DESIGN_DECISIONS.md#c69--no-per-library-ir-snapshot--cache).

**Interim stop-gap (precedes this plan):** @PLAN28 Step 2 ships a
**whole-stdlib / whole-bundle JSON snapshot** (loft's own database JSON,
not serde — user-accepted 2026-05-31) that rebuilds native `Data` on
load.  Second-class (JSON is re-parsed, not mmap'd) but delivers the
cold-start win without the IR rewrite.  This plan **supersedes** it: the
store struct-enum format replaces JSON and turns the rebuild into a
zero-copy mmap of the same whole bundle.

**Per-library snapshot — dropped, not deferred (user, 2026-06-02; first raised
2026-05-31):** a per-library deliverable would close the first-landing gap for a
brand-new `use` combination, but a library **cannot cleanly write its own IR**
(global indices need name-based relocation into an arbitrary prefix — the
brittlest part of the stop-gap, optimizing the least-common case), and the
`.loft` **source is the better representation of a library's state anyway** (see
§ What gets cached).  So neither @PLAN28 nor this plan does per-library: both
operate on the **whole bundle as one image** with absolute, internally-
consistent indices — no relocation anywhere.  @PLAN28 builds the stop-gap
format-agnostic so this plan swaps the bundle encoder underneath without
touching startup wiring.

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
- **Design:** arc A0/A settled; arc C seam minimum landed; B/D/E open
- **Last touched:** 2026-06-01

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

## Arc A reference — the IR transcribed as loft types (verified 2026-06-01)

The most efficient way to pin arc A's store schema is **not** to hand-write
`db.structure`/`enumerate`/`value` calls — it is to **transcribe the whole IR
as loft `struct`/`enum` declarations and let `loft --native` generate the
schema + record accessors for it.**  The generated Rust *is* arc A's
`init`-equivalent, produced by loft itself.

The transcription below **parses + lays out + runs under `--interpret`**
(empty `Data`), exercising every type:

```loft
// Mapping from native Rust IR (src/data.rs) to loft types:
//   Box<Self>          -> see findings: reference<OtherType> works; a SELF
//                         reference must be vector<Self> (box-of-one)
//   Vec<Self>          -> vector<Self>
//   u8/u16/u32/i32/i64 -> integer   ;  bool -> boolean
//   f64 -> float ; f32 -> single ; String -> text
//   Vec<u16>           -> vector<integer>
//   Vec<(String,bool)> -> vector<SortKey>   ;  Vec<String> -> vector<NameRef>
//   Option<T>          -> sentinel field (0 / "" = None)

struct Position { file: text, line: integer, pos: integer }
struct Key { type_nr: integer, position: integer }      // i8, u16
struct SortKey { name: text, asc: boolean }
struct NameRef { name: text }
struct IntegerSpec { min: integer, max: integer, not_null: boolean, forced_size: integer }

// Enum variant names are GLOBAL type names → must be unique across all enums
// and not collide with builtins; hence the Ty / Nd CamelCase prefixes.
enum TypeT {
  TyUnknown { n: integer }, TyNull, TyVoid, TyNever,
  TyInteger { spec: IntegerSpec }, TyBoolean, TyFloat, TySingle, TyCharacter,
  TyText { dep: vector<integer> }, TyKeys,
  TyEnum { n: integer, is_ref: boolean, dep: vector<integer> },
  TyReference { n: integer, dep: vector<integer> },
  TyRefVar { inner: vector<TypeT> },
  TyVector { inner: vector<TypeT>, dep: vector<integer> },
  TyRoutine { n: integer },
  TyIterator { step: vector<TypeT>, inner: vector<TypeT> },
  TySorted { n: integer, keys: vector<SortKey>, dep: vector<integer> },
  TyIndex { n: integer, keys: vector<SortKey>, dep: vector<integer> },
  TySpacial { n: integer, names: vector<NameRef>, dep: vector<integer> },
  TyHash { n: integer, names: vector<NameRef>, dep: vector<integer> },
  TyFunction { args: vector<TypeT>, result: vector<TypeT>, dep: vector<integer> },
  TyRewritten { inner: vector<TypeT> }, TyTuple { elems: vector<TypeT> }
}

enum Node {
  NdNull, NdLine { n: integer },
  NdSpan { pos: Position, inner: vector<Node> },
  NdInt { n: integer }, NdEnum { ord: integer, tp: integer },
  NdBoolean { b: boolean }, NdFloat { f: float }, NdLong { n: integer },
  NdSingle { f: single }, NdText { s: text },
  NdCall { def_nr: integer, args: vector<Node> },
  NdCallRef { var: integer, args: vector<Node> },
  NdBlock { block: reference<Block> }, NdInsert { items: vector<Node> },
  NdVar { n: integer }, NdSet { var: integer, inner: vector<Node> },
  NdReturn { inner: vector<Node> }, NdBreak { n: integer },
  NdBreakWith { n: integer, inner: vector<Node> }, NdContinue { n: integer },
  NdIf { cond: vector<Node>, t: vector<Node>, f: vector<Node> },
  NdLoop { block: reference<Block> }, NdDrop { inner: vector<Node> },
  NdIter { var: integer, create: vector<Node>, next: vector<Node>, init: vector<Node> },
  NdKeys { keys: vector<Key> }, NdTuple { items: vector<Node> },
  NdTupleGet { var: integer, idx: integer },
  NdTuplePut { var: integer, idx: integer, inner: vector<Node> },
  NdYield { inner: vector<Node> },
  NdFnRef { def_nr: integer, var: integer, t: vector<TypeT> },
  NdFnRefDnr { n: integer }, NdParallel { arms: vector<Node> },
  NdParFor { body: reference<ParForBody> }, NdRawExpr { s: text }
}

struct Block { name: text, operators: vector<Node>, result: vector<TypeT>, scope: integer, var_size: integer }
struct ParForBody { input: vector<Node>, x_var: integer, r_var: integer,
                    worker: vector<Node>, threads: vector<Node>, body: vector<Node>, stitch_id: integer }
struct Attribute { name: text, typedef: vector<TypeT>, mutable: boolean, constant: boolean,
                   init: boolean, nullable: boolean, primary: boolean, hidden: boolean,
                   value: vector<Node>, check: vector<Node>, check_message: vector<Node>,
                   alias_d_nr: integer, assigned_lambda_d_nr: integer }
struct Variable { name: text, type_def: vector<TypeT>, stack_pos: integer, uses: integer,
                  argument: boolean, stack_allocated: boolean, skip_free: boolean,
                  captured: boolean, caller_hidden_buf: boolean }
struct Function { name: text, file: text, variables: vector<Variable> }
struct LinkedFieldGroup { kind: integer, instance: integer, field_indices: vector<integer>, alignment: integer, size: integer }
struct Definition { name: text, source: integer, def_type: integer, parent: integer, position: Position,
                    attributes: vector<Attribute>, code: vector<Node>, returned: vector<TypeT>,
                    returned_not_null: boolean, rust: text, native: text, op_code: integer, known_type: integer,
                    variables: Function, pub_visible: boolean, closure_record: integer,
                    mutated_captures: vector<NameRef>, scalars_to_box: vector<NameRef>, bounds: vector<integer>,
                    forced_size: integer, purity: integer, field_groups: vector<LinkedFieldGroup>, synthetic: text }
struct Data { definitions: vector<Definition>, source: integer }
```

**Findings (these are the arc A design constraints, learned the cheap way):**

1. **Enum variant names are global type names.**  Every variant (`TyBoolean`,
   `NdCall`, …) registers a `db.structure(name, ord)` whose name lives in the
   one global type namespace, so it must be unique across *all* enums and must
   not collide with a builtin (`boolean`, `vector`, `text`, …).  The real IR
   has `Value::Boolean` *and* `Type::Boolean`; in the store model these need
   distinct registered names (the `Ty`/`Nd` prefixes here).  Variant names must
   also be CamelCase (no underscores).

2. **A self-referential single child cannot be `reference<Self>`; use
   `vector<Self>` (box-of-one).**  `reference<OtherType>` lays out fine
   (`NdBlock { block: reference<Block> }` works — `Block` is a distinct type),
   but `reference<Node>` *inside* `Node` fails layout (`inner:Node@?..?`,
   unresolved size).  The recursion has to route through either a distinct
   wrapper type or a `vector<Self>` (which is a length-prefixed out-of-line
   chunk, so it has a fixed in-record size).  Every `Box<Value>` /
   `Box<Type>` single-child in the real IR maps to a `vector<…>` of length ≤ 1
   here.  **Arc A must decide:** box-of-one vector, or a dedicated indirection
   record (a `NodeRef { target: reference<NodeBox> }` shim).  The box-of-one is
   simplest and is what laid out.

3. **`--native` needs definition order to be dependency-respecting.**  The
   interpreter lays out the whole graph regardless of order, but the generated
   native code referenced `Block`'s type id (`t112`) before it was bound
   (`E0425`) because `Block` is declared after `Node` (which references it).
   Arc A's emitter must topologically order (or forward-declare) type
   registrations; for the reference script under `--interpret` this is moot.

The other open questions (`&'static str` interning, `OnceLock` caller-index,
`Option` mapping, mutability of a locked mmap) are unchanged from § Open design
questions; finding 2 in particular is the first concrete arc A decision.

### What the generated `--native` Rust gives us — and what it does NOT (2026-06-01)

Running `loft --introspect --show-rust` on the transcription (after
dependency-ordering the defs per finding 3) produces ~139 KB of Rust.  Reading
it settles exactly how much of arc A/A0/C the compiler hands us for free:

| Generated artifact | Form | Reuse verdict |
|---|---|---|
| **Schema registration** — the `init(db)` body: `db.enumerate("Node")`, `db.structure("Block",0)`, `db.field(t98,"name",t5)`, `db.value(t65,"NdCall",…)`, `db.vector(...)` | declarative calls into `Stores`, **all offsets / widths / enum discriminants / vector wrappers resolved by the compiler** | **Directly reusable — this IS arc A.**  Arc A's deliverable can be exactly this `init` block (emitted by the build, not hand-written). |
| **Field access** — every read is inline `stores.store(&db).get_int(db.rec, db.pos + 8)`, the enum tag via `get_byte(db.rec, db.pos + 32, 0)`, strings via `get_str(get_u32_raw(...))` | open-coded raw `(rec, fld)` arithmetic at each use site | **Template, not code.**  It documents the exact width + offset recipe per field; it is *not* factored into anything callable. |
| **A Rust `struct`/`enum` for the IR types, or per-type accessor fns** | — | **Does not exist.**  Confirmed: there is no `enum Node`, no `Node::call_def_nr(r)`.  An IR "type" exists only as a store schema + scattered inline reads.  The ~171 generated `fn`s are the program's own functions (each doing inline `get_*`), never type accessors. |

**Consequences for the plan:**

1. **Arc A becomes "extract the generated `init`," not "hand-design a schema."**
   The compiler already computes the authoritative layout; arc A's job is to
   capture that `init` block for the IR types (and topo-order it, finding 3) as
   the schema artifact.  This is the efficiency the transcribe-and-generate
   approach was after.
2. **Arc A0 is confirmed necessary and non-redundant with codegen.**  The
   generated reads are precisely the raw `db.pos + N` / `get_byte(…,32,0)`
   arithmetic A0's typed cursor wraps.  `--native` does **not** emit an
   accessor layer — in a normal loft program the *parser* holds field offsets
   and inlines them, so there is no "accessor object" to generate.  That gap is
   exactly arc A0 (the cursor) + arc C (the seam).  The generated inline reads
   are the **reference recipe** for the bodies of those accessors (which width,
   which offset, per field).
3. **`Data`-as-store is not a generated Rust type.**  It is
   `{ stores: Stores, <the init schema> }` plus a hand-built (A0-cursor-based)
   accessor layer mapping `data.def(d).name()` → `record(rec).str(FLD_NAME)`.
   The generated `get_*(db.pos+offset)` lines are the field-offset source of
   truth for writing those accessors; they are not themselves the accessors.

**Net:** the generated Rust is useful as the **schema source-of-truth (reuse
directly)** and the **per-field access recipe (reuse as template)** — not as
linkable code.  Schema = generated; typed accessor layer = built by us (A0 +
seam).  The two compose; neither alone is the deliverable.

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **A0** — typed `Store` field cursor | A `Record` / `RecordMut` wrapper over `Store`'s raw `get_int`/`set_int`/`addr::<T>` primitives: named, bounds-checked, typed field reads/writes so no IR accessor does `(rec, fld)` offset arithmetic directly.  Pure-additive precondition for A/C; ships value standalone (safer `--native` + fill.rs reads). | **Done** (cursor `a07ed8d`, superseded by the typed handle layer `src/data_store.rs` in commit `9d860c5`); see § Arc A0 — handle layer |
| **A** — IR store schema | **Extract** the `init(db)` schema-registration block `--native` already generates for the IR transcription (§ Arc A reference / § What the generated Rust gives us) — not hand-design.  The compiler resolves all offsets/widths/discriminants; arc A captures that block (topo-ordered, finding 3) as the schema artifact, after deciding finding 2 (box-of-one `vector<Self>` vs a wrapper record for single recursive children). | **Done** (commit `ed21b3e`) — `tools/ir_schema/` pipeline + `src/ir_schema_gen.rs` generated and checked in; see § Arc A reference |
| **B** — write path | Materialize a parsed native `Data` into store records (validates the schema).  **Greenfield, not "largely done":** @PLAN28 shipped a *JSON* snapshot (native-side, `LOFT_DUMP_SNAPSHOT` → `data_to_json`), **not** a store-format one.  Reuses `ir_schema::data_to_json`'s exhaustive native-IR walk as the traversal skeleton (JSON sink → `data_store`-handle store-writer sink) + `compare_data` as the equivalence oracle.  **Write path COMPLETE:** `src/ir_store.rs` materializes the **entire** native `Data` — all 34 Node + 24 TypeT variants, every struct (`Attribute`/`Variable`/`Function`/`Definition`/`Block`/`LinkedFieldGroup`/`Data`), via the generic `Record`/`RecVector` layer — through `materialize_data(&Data) -> DbRef`, exercised on the real stdlib (`materialize_whole_stdlib_smoke`).  All offsets/strides guard-pinned; `Store::zero_range` clears reused element memory.  **Remaining (→ arc C):** a store→native read path for bit-for-bit `compare_data` validation (smoke test currently validates structure: names + per-def attribute/variable counts). | Write path done — whole `Data` materializes on real stdlib; `compare_data` equivalence needs the arc C read path |
| **C** — read accessors | `data.def(dnr)` + `value` / `type` matching read from the store instead of `Vec`/`Box`.  The ~940-site migration — **done incrementally via the accessor seam, never at once** (see § Incremental migration).  Minimum seam (`src/data_store.rs` handle layer) + the full **store→native reader** (`src/ir_read.rs` — `read_value`/`read_type`/`read_data` over all 34 Node + 24 TypeT variants + every struct) landed, and the schema grew to hold `Function.names`/`inline_refs` so the whole real stdlib round-trips **fully bit-for-bit** (`compare_data` green).  Bulk read-site migration started: **slice 1** = `state/`'s `Definition` field-accessor seam (~120 sites, store-backed fields only; `Value`/`Type` matches deferred).  Remainder: `state/codegen.rs` `Value`/`Type` walk, then `generation/` / `parser/`, then per-subsystem representation swap. | In-progress — lossless reader done; bulk migration started (`state/` Definition seam, slice 1) |
| **D** — mmap load | `Data::open(path)` → `Store::open` → live IR, zero rebuild.  Wire into the startup path behind the bundle cache key. | Probe done — the mmap → `read_data` rebuild loop works end-to-end on the real stdlib, **~12× faster than `parse_dir`** (`mmap_file_round_trip_stdlib` + `bench_stdlib_load_mmap_vs_parse`).  Remaining: startup-path wiring (bundle cache key / drift Q4, mutability Q1, `caller_index` Q3). |
| **E** — bundle snapshots | Core `stdlib.store` (shared) + per-script bundle snapshot (core + sorted lib-set), each keyed for drift. | Open |

## Phase ordering

0. **A0 (typed field cursor)** — ✅ done (`a07ed8d`).  Cursor form superseded by
   the typed handle layer (commit `9d860c5`); see § Arc A0 — handle layer.
1. **A (schema)** — ✅ done (`ed21b3e`).  `tools/ir_schema/` pipeline + generated
   `src/ir_schema_gen.rs` register the full IR schema.  Finding 2 (box-of-one
   `vector<Self>` for recursive single-child) resolved in the transcription.
2. **B (write)** — native `Data` → store.  Testable artifact; validates A
   before touching read sites.  Greenfield (@PLAN28 shipped JSON, not a
   store-format snapshot — see arc B row); reuses `data_to_json`'s walk as the
   traversal skeleton + `compare_data` as the oracle.  Prerequisite (module
   fork, below) ✅ done.  **Write path ✅ done:** `ir_store::materialize_data`
   materializes the whole native `Data` (every Node/TypeT variant + every
   struct) into a store, exercised on the real stdlib.  The remaining piece —
   a store→native read path for full `compare_data` equivalence — folds into
   arc C (read accessors).
3. **D (mmap load, read-mostly)** — load a store-backed `Data` and run
   execution against it *while the parser still builds native `Data`*.
   Proves the read path on the hot loop before the full migration.
4. **C (access-site migration)** — convert `data.def()` and the IR
   `match` arms incrementally, by subsystem (state/ first, then
   generation/, then parser/).
5. **E (bundle snapshots)** — core snapshot first (deterministic, shared);
   per-script bundle snapshot second.

## Module fork to reconcile before arc B (found 2026-06-02; step 1 ✅ done)

Arc A left **two `ir_schema` modules with two same-named `register_ir_schema`
functions** in the tree.  This is a transition artifact, not a design — cleaned
up (step 1) before arc B built on top of it:

| Module | `register_ir_schema` | Role | Disposition |
|---|---|---|---|
| `src/ir_schema_gen.rs` (arc A, generated) | `-> IrSchemaIds` | the **complete** store schema (all fields/variants/offsets, `db.finish()`) | **keep** — this is arc A's deliverable |
| `src/ir_schema.rs` (@PLAN28) | `-> usize` | **shells-only** schema (S1 rung, no fields wired); only its own test (`ir_schema.rs:1541`) calls it | **dead — delete** the shells-only register; its job is done by `ir_schema_gen` |
| `src/ir_schema.rs` (@PLAN28) | — | the **JSON codec** (`*_to_json`/`*_from_json`/`compare_data`) | **interim — keep until arc B lands**, then retire (see § Standalone upside decision); reused as arc B's traversal skeleton + oracle meanwhile |

Concretely, before arc B:

1. ✅ **Done** — deleted the shells-only `ir_schema::register_ir_schema` (and its
   `shells_register_without_collision` / `prefix_is_not_a_legal_identifier`
   tests + the `IR_PREFIX`/`IR_ENUMS`/`IR_STRUCTS` consts), so there is exactly
   one schema registration in the codebase (`ir_schema_gen`).  The JSON codec
   (`*_to_json` / `compare_data`) stays.
2. Keep the JSON codec **only** as arc B's scaffolding — its walk is copied into
   the store-writer, and `compare_data` validates the result; it is deleted once
   that validation has run green and `show_json`-over-store works.

## Arc A0 — handle layer (landed; supersedes the cursor design)

**Original plan:** a `RecordCursor`/`RecordCursorMut` that bound `&Store + rec`
once and named the width method, so no accessor did open-coded `(rec, fld)`
arithmetic.  That cursor landed in commit `a07ed8d`.

**As-built:** `src/data_store.rs` has since been rewritten as the **typed
handle layer** (commit `9d860c5`) — the minimum accessor seam the plan's arc C
migration requires.  The cursor form (still green at `a07ed8d`) is superseded;
the handles subsume it.

### Design (three principles, user, 2026-06-01)

**Principle 1 — reuse, don't reimplement.**  Each accessor locates its field
and hands the read or write to an *already-written* primitive:
`Store::get_int`/`get_str`/`set_str`/`get_u32_raw`/`get_byte`,
`vector::length_vector`/`get_vector`/`insert_vector`.  NOT `Stores::show_json`
or `field_content` — that would rebuild the database's schema-walker from
scratch.

**Principle 2 — baked layout constants, no runtime indirection.**  Variant
discriminants (`DISC_NULL=1`, `DISC_INT=4`, `DISC_CALL=11`, `DISC_BLOCK=13`),
field byte offsets (`NDCALL_ARGS=4`, `NDCALL_DEF_NR=8`, `NDBLOCK_BLOCK=8`,
`BLOCK_NAME=16`, `BLOCK_OPERATORS=20`), and the `vector<Node>` element stride
(`NODE_STRIDE=48`) are hard-coded `const`s mirroring loft's schema.  Rationale:
accessors run **millions of times** on every IR walk; a runtime `position()`
lookup or schema name-match is indirection the compiler cannot fold, making the
layer unusable.  Each accessor folds to one store primitive at one constant
offset.  Methods take `&Stores`/`&mut Stores`; `IrSchemaIds` is not needed at
runtime.

**Principle 3 — guard test pins hand-typed consts to loft's real layout.**
`baked_layout_mirrors_loft_schema` (`src/data_store.rs::tests`) asserts every
const equals what `register_ir_schema` + `db.finish()` actually computed
(`stores.position(tp, field)`, `stores.size(node)`, variant discriminants).
The most important assertion is `NODE_STRIDE == size(node)`: the enum size
aggregates over all 34 variants and cannot be eyeballed; it is correct only
because `register_ir_schema` is the **complete** definition run through loft's
layout routine.  A mistyped constant compiles fine and silently reads the wrong
bytes millions of times — the guard turns that into an immediate CI failure.

### Public API (`src/data_store.rs`)

```rust
pub enum ValueType { Null, Int, Call, Block, Other(u8) }

pub struct Value { rec: DbRef }       // handle to one Node record
pub struct ValuesVector { rec: DbRef } // handle to a vector<Node> field

impl Value {
    pub fn new(rec: DbRef) -> Self;
    pub fn db_ref(&self) -> DbRef;     // for callers driving existing fns
    pub fn value_type(&self, stores: &Stores) -> ValueType;
    pub fn call_to(&self, stores: &Stores) -> u32;          // NdCall.def_nr
    pub fn call_parameters(&self) -> ValuesVector;          // NdCall.args
    pub fn block_name<'a>(&self, stores: &'a Stores) -> &'a str;  // NdBlock → Block.name
    pub fn block_name_set(&self, stores: &mut Stores, name: &str); // NdBlock → Block.name
    pub fn block_operators(&self) -> ValuesVector;           // NdBlock → Block.operators
}
impl ValuesVector {
    pub fn len(&self, stores: &Stores) -> u32;
    pub fn is_empty(&self, stores: &Stores) -> bool;
    pub fn get(&self, i: u32, stores: &Stores) -> Value;
}
```

### Verified layout facts (from probing the registered schema)

- `reference<Block>` inside `NdBlock` is **inlined**: a 28-byte `Block` struct
  at offset 8 — no pointer deref.
- `vector<Node>` is stored **inline** (the `is_linked`/P376 `Array` promotion
  is not triggered here); stride = 48 bytes.
- `integer` fields are 8 bytes, read via `Store::get_int` (returns `i64`).

### Status and tests

Minimum implementation: covers `NdNull`/`NdInt`/`NdCall`/`NdBlock` and the
`Block.name`/`Block.operators` sub-fields.  Three tests pass:
`ndcall_reads_back_through_handles`, `ndblock_name_and_operators_round_trip`,
`baked_layout_mirrors_loft_schema`.  Fmt-clean, clippy-0.

### Future direction (not done)

Replace the hand-typed const block by generating it from loft's own output:
write the accessors as *methods* in `ir.loft` (compiling to
`t_<len><Type>_<method>` functions with the offsets baked, uninstrumented
unlike `n_` free functions), then a script lifts those functions and their
offset literals into the generated layer.  The handle API and the layout guard
stay identical across that swap; it is a generation-automation improvement, not
a design change.

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

## Migration step plan — native `Data` → store-backed reads (small steps)

Two distinct payoffs, sequenced **cheapest-first**.  Each step is green and
shippable on its own (one step = one PR, § Incremental migration pacing).

**G1 — cold-start cache (rebuild-on-load).** Parser + codegen stay native; on a
cache hit, `read_data` rebuilds the native `Data` from a mmap'd store (**12×
faster than `parse_dir`**, proven — § arc D probe).  No read-site migration
needed; this is mostly startup wiring and delivers the big user-visible win.

| Step | Deliverable | Validation | Effort |
|---|---|---|---|
| **D1** ✅ | `Data::save(path)` / `Data::open(path)` (thin wrappers over `ir_store::save_data` / `ir_read::open_data`).  Save materializes into a fresh file-backed store with the root at the well-known first record (`IR_ROOT_REC`=1, pos 8) so load needs no sidecar; open mmaps + `read_data`, returning `NotFound` on a missing file (clean cache-miss).  `scopes::check`-skip is deferred to **D2** (only matters once the loaded `Data` is compiled). | `data_save_open_round_trip_stdlib`: save→open→`compare_data` bit-for-bit + `NotFound` check | S — done |
| **D2** | Startup wiring: after parsing the stdlib bundle, write `stdlib.store`; next run `Data::open` it instead of parsing when the cache key matches. | cold-start timing; existing suite unchanged | M |
| **E1** | Bundle cache key = stdlib key + sorted lib-set + content hashes; drift ⇒ reparse (Q4). | drift unit tests | M |
| **E2** | Mutability split (Q1): locked mmap bundle store + writable store for user-program defs; `caller_index` rebuilt on load (Q3). | full suite under cache-on | M |

**G2 — zero-copy store-backed reads.** Removes even the rebuild: codegen / exec
read store fields directly.  Larger; the self-hosting foundation.  Incremental,
behind the accessor seam, validated by a **dual-backing equivalence harness**
(read native AND store, assert equal) so every step is reversible.

| Step | Deliverable | Validation | Effort |
|---|---|---|---|
| **M0** | Dual-backed `Data` (holds native + materialized store) + `LOFT_IR_CHECK` debug harness asserting store-read == native-read per accessor. Additive; nothing switches yet. | harness self-test on stdlib | M |
| **M1a** ✅ | `state/` `Definition` field-accessor seam (done). | suite green | — |
| **M1b** | `generation/` `Definition` field-read seam. | suite green | S |
| **M1c** | `parser/` + `compile.rs` `Definition` read-site seam (read sites only). | suite green | S–M |
| **M2** | `data.def(d)` returns a `DefView` (native \| store) carrying the accessor methods; point the M1 seam at it. | M0 harness | M |
| **M3.0** | Design the node-walk handle: a `match`-able dispatch over the IR node (`kind() -> ValueType` + typed child accessors), backed by native first. | — | S (design) |
| **M3.1…n** | Convert `state/codegen.rs`'s `generate`/`gen_*` `Value`/`Type` matches (451 sites) to the handle, **function-group by function-group** (one commit each, native backing). | suite green per group | several S |
| **M4** | Same handle conversion for `src/generation/` (native codegen) `Value`/`Type` matches. | native suite green | several S |
| **M5** | Per-subsystem **representation swap**: flip a fully-seamed subsystem's backing to store-read (state/codegen first, then generation/); equivalence-assert; ship. | M0 harness + suite + Q5 bench | M each |
| **M6** | Drop the native `Vec<Definition>`/`Box<Value>` body graph once every reader is store-backed — reads become **zero-copy**. | suite green; bench | M |
| **M7** | (Optional, self-hosting) parser emits store-backed IR directly — removes the post-parse materialize. | compare_data vs golden | L |

**Sequencing note:** G1 ships the speed win without touching read sites, so do
it first (or in parallel) — it is independently valuable and de-risks G2 by
exercising the store on the real startup path.  G2's M0 harness is the
prerequisite for every swap; M3/M4 (the `Value`/`Type` walk, ~451+ matches) is
the dominant cost and is deliberately the most finely sliced.

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
5. **Store-read vs `Vec`-index perf — cost AND a locality upside (user,
   2026-06-01).**  `data.def()` is a `Vec` index today (~940 sites, hot); a
   `DbRef` read adds a store indirection — measure the hot-path delta and
   whether a read-through cache is needed.  **First data point (2026-06-02,
   `bench_stdlib_load_mmap_vs_parse`):** *loading* the whole stdlib via
   `Store::open` + `read_data` (rebuild native) is **~12× faster** than
   `parse_dir` (0.92 ms vs 11.4 ms median, warm cache) — so even the
   rebuild-on-load path is a large net win, and the store layout is decisively
   "good enough to build on."  This measures the *load* path, not yet the
   per-`data.def()` hot-read delta (that is what the per-subsystem swap will
   measure).  **But the store layout is not purely a cost.**  The native IR is a graph of separately-heap-allocated
   nodes — `Box<Value>`, `Vec<Definition>`, `String`, nested `Box<Type>` —
   scattered across the allocator, so walking a definition's `code` chases
   pointers into cold cache lines.  The store packs a record's fields (and,
   with co-located `ChildRec`/inline layouts, its sub-records) **contiguously
   in one rigorously-laid-out buffer**.  That tight, sequential layout is
   exactly what caches and prefetchers reward: walking an IR node in the
   store can touch far fewer cache lines than chasing the equivalent `Box`
   graph.  Rust's per-node layout is locally optimal but globally scattered;
   the store trades a small per-access indirection for **whole-IR locality**.

   So the honest hypothesis is a *trade*, not a strict regression: indirection
   cost vs. locality/prefetch win, and the balance is empirical.  It may even
   come out **net-positive on the hot walk** before mmap is considered —
   making "Data-as-store" a structural optimisation in its own right, not only
   the enabler for zero-copy load.  This is the second thing arc C's
   per-subsystem equivalence/bench harness must measure (alongside
   correctness): not just "is store-read fast enough?" but "is the packed
   layout actually faster to traverse?"  Treat the locality win as a
   hypothesis to confirm with numbers, not a given.

   **Risk-posture consequence (user, 2026-06-01) — why "slow IR" is not a
   thing to fear.**  The locality argument is **not a clear win**; it may net
   out slower.  Its real value is as a *floor on the downside*: the store is a
   rigorous, contiguous, cache-coherent layout, so even in the worst case the
   store-backed IR is **a reasonable representation, not a pathological one**.
   That is what makes it safe to commit to the store representation directly,
   rather than treating "land in the store" as risky "slow IR" territory to be
   avoided.  Combined with the migration safety net (the accessor seam +
   dual-backing + per-subsystem equivalence assertion, § Incremental
   migration), the perf question stops being a gate on *whether* to migrate
   and becomes a tuning question *after*: if a hot subsystem measures slower,
   add a read-through cache or keep that subsystem native — the dual-backing
   makes either reversible.  So the design proceeds on "the store layout is
   good enough to build on," with the locality upside as a possible bonus, not
   a load-bearing assumption.

## Cross-arc dependencies

- **[@PLAN28 startup-cache](../../deferred/28-const-store/STARTUP_CACHE_PLAN.md)**
  — direct predecessor.  @PLAN28's rebuild-on-load snapshot delivers the
  cold-start win first; this plan removes the rebuild.  @PLAN28 shipped a
  **JSON** snapshot (native-side), **not** the store struct-enum format — so
  arc B is greenfield, not "mostly done"; what carries over is the JSON codec's
  IR walk (as arc B's skeleton) and `compare_data` (as its oracle), both interim
  (§ Standalone upside decision).  serde is forbidden project-wide (CODE.md) —
  this plan uses the store format, consistent with that.
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
