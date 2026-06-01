<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 54 — `Data` as a store (IR mirrors the `--native` data model)

## Status

**In-progress — arc A0 + arc A landed; arc B's write path opened (first native→store materializer slice green).**

- **Arc A0** (typed field cursor, commit `a07ed8d`) — landed as `RecordCursor`/`RecordCursorMut` wrapping `Store`'s raw primitives.  That cursor form has since been superseded by the typed handle layer (see § Arc A0 — handle layer below); `src/data_store.rs` is now the accessor seam, not a bare cursor.
- **Arc A** (IR store schema, commit `ed21b3e`) — landed as `tools/ir_schema/` (hybrid generate-extract pipeline) + `src/ir_schema_gen.rs` (generated, checked-in).  The full IR is registered via `register_ir_schema(db: &mut Stores) -> IrSchemaIds`; every struct/enum is in the schema; `db.finish()` computes all field positions, record sizes, and discriminants including the 34-variant `Node` enum size.
- **Typed handle layer** (`src/data_store.rs`, commit `9d860c5`) — minimum accessor seam: `Value`/`ValuesVector` thin `DbRef` handles with `ValueType` enum covering the IR-walker's current match surface.  Three tests pass (NdCall round-trip, NdBlock round-trip, layout guard).  Fmt-clean, clippy-0.
- **Arc B fork-cleanup (prerequisite, done)** — removed the dead shells-only `ir_schema::register_ir_schema` + its consts/tests, leaving exactly one schema registration (`ir_schema_gen`).  The @PLAN28 JSON codec stays (interim — arc B's traversal skeleton + `compare_data` oracle); its 30 lib tests + 6 round-trip tests still pass.
- **Arc B write path (opened)** — `src/data_store.rs` grew the write side (`ValuesVector::push`, `Value::write_null/write_int/write_call/write_block`, `int_value` reader), and `src/ir_store.rs` adds `materialize_node` (native `data::Value` → store records, the `data_to_json` walk with a store-writer sink).  Two round-trip tests green (`materialize_call_tree`, `materialize_block`).  Coverage so far is the handle subset (Null/Int/Call/Block); the remaining variants (the bulk of the arc) are the next increment.  Arcs C/D/E remain open.

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
| **B** — write path | Materialize a parsed native `Data` into store records (validates the schema).  **Greenfield, not "largely done":** @PLAN28 shipped a *JSON* snapshot (native-side, `LOFT_DUMP_SNAPSHOT` → `data_to_json`), **not** a store-format one.  Reuses `ir_schema::data_to_json`'s exhaustive native-IR walk as the traversal skeleton (JSON sink → `data_store`-handle store-writer sink) + `compare_data` as the equivalence oracle.  **Opened:** `src/ir_store.rs::materialize_node` + the `data_store.rs` write side cover the handle subset (Null/Int/Call/Block), two round-trip tests green.  **Remaining:** grow write-accessor + `materialize_node` coverage to all 34 Node + 24 TypeT variants + Definition/Attribute/etc. (the bulk), then a whole-`Data` materializer validated by `compare_data` against a fresh parse. | In-progress — handle subset materialized + round-trip tested |
| **C** — read accessors | `data.def(dnr)` + `value` / `type` matching read from the store instead of `Vec`/`Box`.  The ~940-site migration — **done incrementally via the accessor seam, never at once** (see § Incremental migration).  Minimum seam (`src/data_store.rs` — `Value`/`ValuesVector`/`ValueType`, covering `NdNull`/`NdInt`/`NdCall`/`NdBlock`) is implemented; bulk migration is open. | Open — minimum seam landed (commit `9d860c5`); bulk migration is the remainder |
| **D** — mmap load | `Data::open(path)` → `Store::open` → live IR, zero rebuild.  Wire into the startup path behind the bundle cache key. | Open |
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
   fork, below) ✅ done.  **Opened:** `ir_store::materialize_node` covers the
   handle subset; next increment grows it to all variants + a whole-`Data`
   materializer.
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
   whether a read-through cache is needed.  **But the store layout is not
   purely a cost.**  The native IR is a graph of separately-heap-allocated
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
