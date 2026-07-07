<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# @PLN97 — Formal memory/file layout contract + per-structure conformance tests

> **Subject:** loft   ·   **Type:** plan   ·   **Area:** runtime · store · formal
> **Effort:** H   ·   **Value:** **S** (silent-corruption prevention) + **F** (foundation for
> reload-without-data-loss, live-swap, durable store, schema evolution)
> **Consumes:** `src/data.rs` (the `layout(τ)` function), `src/store.rs` (header + the
> `.dmeta` durable sidecar pattern), `src/ir_schema.rs` (`data_to_json` — a golden-pinned
> schema serializer), `formal/heap.md` (store semantics — this adds the format layer it omits)
> **Live status · lifecycle:** [loft-lang/plans ▸ @PLN97](https://github.com/loft-lang/plans/issues/97) ← single source of truth

## Status

Open — design settled (this README), no implementation yet. Filed 2026-07-07 out of a
design discussion prompted by #477 (nested-vector stride `4→8`/`16→8`), a layout change
that was **noticed only by breaking persisted data**, never by a check.

## Goal

Give loft's store layout a **written formal contract** and a **conformance test that pins the
exact byte layout of every structure loft can define**, so any layout change is caught by a red
test at commit time — not by invalidated data — plus a **separate schema-description sidecar**
that makes a persisted store self-describing (never silently misread) and the basis for a
**formal schema switch** (Drop&Add first, data migration later).

**Not a freeze.** The layout *will* change — that is expected. The contract's job is to make
each change **deliberate and detected** (a red golden test + a re-derived layout-algo hash),
never a silent slip, and to tell the reload/persistence path exactly **when** a handoff needs
the old data serialized first.

## Why this is load-bearing

This is not persistence hygiene — a lot of loft rides on it. **Reloading a binary without
dropping data** — the [@PLN18 08 whole-build-swap](../18-engine-host/08-live-build-swap.md)
*under a running world* ("the build is replaceable; the state/store persists") — is only sound
if the new build's layout is **verified identical** to the old build's or **explicitly
migrated**. Today it is *assumed* identical with nothing that checks it, so a layout-changing
fix silently corrupts a hot-swap or a persisted store. Same premise under live-reload, the
durable store ([plans/43](../43-loft-store-durable/)), and any future persistence. This plan is
the safety floor beneath all of them.

### The handoff — how an app lives through a layout change

A running app hands its store to a new build. The **layout-algo hash decides the handoff**:

- **Identical layout** (`new.hash == old.hash` — the common rebuild): hand over the raw store.
  Bit-identical, zero-copy. Nothing to serialize.
- **Changed layout** (`new.hash != old.hash`): the old, still-running version **serializes its
  live data before the handoff** (to the schema-described neutral form — the sidecar schema +
  values); the new version deserializes into its new layout (E1 Drop&Add, or E2 migration). The
  app lives through the version change.

The one thing you must have is **knowing which case you are in** — the hash comparison. Without it
(today) the swap silently assumes "identical" and corrupts on any layout-changing fix. So the
layout-algo hash is not merely a persistence guard; it is the **handoff decision variable** for
live reload. (08-S5 already serializes via `show_json` *unconditionally*; this makes it
*conditional and correct* — raw when safe, serialize-first when the hash says it must.)

## The one format (no split — FINAL)

loft's store is **one thing**: the durable file is bit-for-bit identical to the in-memory store
(`store.rs`: *"durability is a metadata layer, not a payload-layout change"*). So there is one
`layout: Type → bytes` function (`data.rs` `element_size`/`element_align`/`element_offsets` +
the store header `[0=SIG, 4=free_idx, 8=rec_size, 12=content]` + the `DbRef` encoding). The
user's hard constraints:

- **One format — no in-memory/on-disk split.** The store payload is sacred; never modified.
- **The only on-disk addition is a SEPARATE file** describing the schemas the store was built on.
- **The version/identity lives in memory** — the running program's live schema + layout-algo
  hash is the source of truth; the sidecar merely records it.

## The crux — two axes, or the check misses #477

A schema-only check catches only one of two independent axes:

- **schema** — the *types* (structs/fields/`known_type`); what `ir_schema::data_to_json` already
  serialises (and already a golden-pinned on-disk contract).
- **layout algorithm** — *how types become bytes* (`data.rs`). **#477 changed this, not the
  schema**: same types, different bytes. A type-only sidecar would say "schemas match" and still
  misread.

So both the in-memory identity and the sidecar must carry **schema AND a hash of the layout
algorithm** (the hash phase B's golden test defines). One fact — the layout table and its hash —
drives the dev-time test, the in-memory identity, and the sidecar (Goal E: one home).

## Composition matrix — Stage A (the "every structure" census IS the matrix)

The feature is *done* when every structure loft can define has a pinned byte layout, green on
**both backends**. Phase A enumerates the cells; phase B is the matrix as a test. Axes:

- **Structure kind:** scalar (each base type 0..=6), struct, enum, vector, **nested vector**
  (#477), hash, index, sorted, tuple (incl. the native synthetic `__tuple<…>`), reference/`DbRef`,
  closure record.
- **Layout inputs beyond the kind (the hidden ones — writing them down is the point):** nullable
  widening + null-sentinel representation, keyed-dense (@PLN25), narrow-int storage (#399),
  nesting depth, inline-vs-boxed, backend ABI (interpreter store vs native `DbRef`).

Each cell = a corpus entry: a constructed value whose exact bytes (header + offsets + strides +
serialized payload) are pinned. A layout input that turns out **not** to be a function of the
static type is a finding to fix, not a blocker — that discovery is the value.

## Sub-arcs

| Item | Builds | Status |
|---|---|---|
| **A** — Structure census + `layout(τ)` map | The enumerated cell list (above) + identification of every hidden layout input; the falsification pass on "layout is a pure function of Type" | Open |
| **B** — Golden layout-conformance test *(the instrument)* | For the corpus spanning **every** cell: pin exact bytes on both backends; **self-audit** that a new `Type`/structure kind cannot be added without a corpus entry (enumerate variants → assert coverage). This is what would have caught #477 | Open |
| **C** — `formal/layout.md` | The written `layout(τ)` contract (per-type size/align/offsets, header format, `DbRef` encoding, null representation, the nullable/keyed-dense/narrow-int axes) + the sidecar format + the invariant *"no silent cross-version misread"*; rules + deviation `D-layout-1` (cite #477). Wire into `formal/README.md` + `formal/ROADMAP.md` | Open |
| **D** — Schema-description sidecar *(self-describing store)* | A separate file (the `.dmeta` pattern) carrying `(data_to_json schema, layout-algo hash)`; on load, compare vs the program's in-memory identity → identical / Drop&Add-compatible / needs-migration / reject-via-`on_corruption`. Payload untouched. Enabler for E | Open |
| **E1** — Formal schema switch: **Drop&Add** *(first)* | The serialize-before-handoff path taken when the hash says layouts differ: a formal, verified A→B transition for additive/subtractive schema change — part added → default/null, part dropped → ignored. The lenient-serialization contract (already partial in 08-S5 `show_json`/`populate_struct_from_jsonvalue`) made formal over the store, using the sidecar's old schema as the map | Open |
| **E2** — Formal schema switch: **data migration** *(later — deferred)* | Read old-layout bytes with the old schema (from the sidecar), transform, rewrite in the new layout. **Trigger:** a real layout change must preserve live data (dogfood-driven) | Deferred |

## Phase ordering

1. **A** (census) — the axes everything else covers.
2. **B** (golden test) — the **critical path**: it delivers the "caught by verification, not by
   breaking" guarantee and *defines the layout-algo hash* the rest consume. Ship this first.
3. **C** (formal doc) and **D** (sidecar) proceed in parallel after B — both read B's hash.
4. **E1** (Drop&Add) rides on D's self-describing store. **E2** stays deferred behind its trigger.

## Open design questions

1. **Golden form:** raw serialized bytes vs. a structured layout table (offsets/sizes/strides) vs.
   both. *Lean: both* — the table is the human-readable diff; the raw bytes catch encoding changes
   the table misses.
2. **Layout-algo hash source:** hash of the layout-table dump (auto-derived, cannot be forgotten;
   a test asserts `identity.hash == recompute()`) vs. a hand-bumped integer. *Lean: derived hash,
   with a human version field beside it for migration decisions.*
3. **Coverage enforcement:** the exact mechanism that fails when a new `Type`/structure kind is
   added without a corpus entry (variant enumeration + coverage assertion) — must be robust to
   future kinds.
4. **Sidecar placement vs. @PLAN38:** does the schema sidecar extend the existing `.dmeta`, or is
   it a second sidecar? (One file is simpler; `.dmeta` is currently fixed-size integrity only.)
5. **Drop&Add scope (E1):** which changes are "parts" (fields, enum arms, vector element widening)
   vs. changes that *require* E2 (reordering that shifts offsets, type narrowing that loses bits).

## Cross-arc dependencies

- [`formal/heap.md`](../../formal/heap.md) — store *semantics* (0 deviations); this adds the
  *format/layout* layer heap.md deliberately omits, and gives the @PLN89 differential oracle the
  **definitional** layout spec its heap cases currently lack (shrinks D-op-1).
- `src/ir_schema.rs` — `data_to_json`/`from_json` (reuse for the sidecar schema; already
  golden-pinned).
- [store-durable (plans/43)](../43-loft-store-durable/) — the `.dmeta` sidecar + `on_corruption`
  rebuild path the sidecar + reject flow reuse.
- [@PLN18 08 live-build-swap](../18-engine-host/08-live-build-swap.md) — the primary consumer:
  reload-without-data-loss + the lenient snapshot (`show_json`) E1 formalises.
- **Layout axes the corpus must cover:** @PLN25 (null/keyed-dense), narrow-int (#399), #477
  (nested-vector stride — the motivating instance).

## See also

- [`formal/README.md`](../../formal/README.md) — the strict-spec model (rules + deviations to zero)
  this doc joins · [`DATABASE.md`](../../DATABASE.md) / [`INTERMEDIATE.md`](../../INTERMEDIATE.md) —
  the store / `Value`·`Type` reference this formalises.
