<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# @PLN97 — Formal memory/file layout contract + per-structure conformance tests

> **Subject:** loft   ·   **Type:** plan   ·   **Area:** runtime · store · formal
> **Effort:** H   ·   **Value:** **S** (silent-corruption prevention) + **F** (foundation for
> reload-without-data-loss, live-swap, durable store, schema evolution) + **U** (compiler-guided
> schema evolution — the programmer just edits code; the handoff stays invisible)
> **Consumes:** `src/data.rs` (the `layout(τ)` function), `src/store.rs` (header + the
> `.dmeta` durable sidecar pattern), `src/ir_schema.rs` (`data_to_json` — a golden-pinned
> schema serializer), `formal/heap.md` (store semantics — this adds the format layer it omits)
> **Live status · lifecycle:** [loft-lang/plans ▸ @PLN97](https://github.com/loft-lang/plans/issues/97) ← single source of truth

## Status

**DONE / SHIPPED — 2026-07-08.** On `tuxedo-add-to-project`; the plan closes on merge (the PR
carries `Closes @PLN97`). Filed 2026-07-07 out of a design discussion prompted by #477
(nested-vector stride `4→8`/`16→8`) — a layout change **noticed only by breaking persisted data**,
never by a check. Rated the highest-value work on loft: the safety floor under
reload-without-data-loss, live-swap, and schema evolution.

**Shipped:**
- **Layout contract B–F** — B golden `tests/layout_golden.rs` (+ coverage audit + both-backend
  parity + `Stores::layout_algo_hash()`/`layout_dump()`); C `doc/claude/formal/layout.md`
  (`layout(τ)`, rules `L-Total`…`L-Sound` + `D-layout-1`); D `src/schema_sidecar.rs`
  (`LayoutIdentity`, the `.dschema` lifecycle, `classify`/`SchemaVerdict`,
  `CorruptReason::SchemaMismatch`); E1 lenient Add/Drop (`tests/scripts/510-layout-e1-add-drop.loft`);
  F the `loft layout accept|check` CLI + migration outline. Detail in the phase table below.
- **Arc G — remote working-set store loader (#522)** — the full fetcher (hash + sorted, local +
  HTTP Range, every copyable shape, int/text keys, point + range), `store_verify`, and the **3b.5
  layout-identity gate** (`.dschema` written on persist; every partial load refuses a mismatched
  layout before reading foreign bytes). Reference: [REMOTE_STORE_LOADER.md](REMOTE_STORE_LOADER.md).

**Deferred (each behind an external trigger — carved out of this plan on close):**
- **E2** reshape data migration — behind a real reshape that must preserve live data (design retained
  in the phase table below + `formal/layout.md`).
- **Arc-G `--html` `fetch()` bridge** — behind a browser consumer (design in
  [REMOTE_STORE_LOADER.md](REMOTE_STORE_LOADER.md)).

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
  values); the new version deserializes into its new layout. The app lives through the change.
  **Add&Drop needs no special mechanism — it falls out of this one path:** lenient deserialize
  *defaults* an added part and *ignores* a dropped one (E1). Only a *reshape* (E2) needs custom
  logic — and even that decomposes into Add/Drop steps (below).

The one thing you must have is **knowing which case you are in** — the hash comparison. Without it
(today) the swap silently assumes "identical" and corrupts on any layout-changing fix. So the
layout-algo hash is not merely a persistence guard; it is the **handoff decision variable** for
live reload. (08-S5 already serializes via `show_json` *unconditionally*; this makes it
*conditional and correct* — raw when safe, serialize-first when the hash says it must.)

### The programmer never manages the handoff

The machinery above stays **invisible to loft programmers** — they just edit their structs. The
compiler diffs the edited schema against a recorded baseline and classifies the drift:

- **adds only** → **automatic** (lenient deserialize defaults the new part).
- **drops only** → **automatic** (the dropped part is ignored).
- **adds *and* drops together** → an **actionable state**: the compiler cannot tell an independent
  add+drop from a **rename/reshape** where the old data should move into the new, so it **aids** —
  a diagnostic that migration may be needed, plus an auto-generated **migration-script outline**
  (the mechanical parts pre-filled; only the old→new backfill left as stubs to write). Phase F.

No hand-managed handoff, no manual hash bookkeeping — the programmer edits code and, at most, fills
a scaffolded backfill when they've moved data.

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
| **B** — Golden layout-conformance test *(the instrument)* | For the corpus spanning **every** cell: pin exact bytes on both backends; **self-audit** that a new `Type`/structure kind cannot be added without a corpus entry (enumerate variants → assert coverage). This is what would have caught #477 | ◐ **v1 SHIPPED** — `tests/layout_golden.rs` + `tests/golden/layout/corpus.txt`: pins record size + `Parts` layout (field positions, narrow-int encodings, collection element strides via a transitive closure) for a representative corpus; stable FNV-1a `LAYOUT_ALGO_HASH`; proven to fail on a perturbed nested-vector `elem_size` (the #477 class). **Coverage self-audit shipped** (`layout_coverage_audit`): exhaustive `Parts` match (a new storage kind → compile error) + a ratchet (a produced kind must be `Covered`) + exact-set completeness; **Corpus expanded** — 11 kinds now Covered (Base/Struct/Byte/ShortRaw/Int/Vector/Hash/Ordered/Index/Enum/EnumValue), incl. a tuple (`__tuple` packing). The expansion caught 3 real facts: `i16`→ShortRaw (not Short); `sorted<T[k]>` field→Ordered (not Sorted); `Item` grows 8→17 in an index (bookkeeping appended). Remaining Gaps = Sorted (needs a local), Short (2-byte shifted), Spacial (1.1+); Internal = Array/DbRef/ChildRec. **Dynamic both-backend parity shipped** (`tests/scripts/509-layout-parity.loft`, interp+native+wasm). **`pub Stores::layout_algo_hash()` + `layout_dump()` exposed** (`src/database/types.rs`) — the storage-level layout identity (catches #477; stack-level `element_size` would not) that **D/F consume**; the golden test now consumes it (deduped) and pins it. **✅ Phase B substantially complete.** Residual Gaps (audit-tracked, not blocking): closure fields (DbRef/ChildRec), Sorted-local, Short, Spacial (1.1+) |
| **C** — `formal/layout.md` | The written `layout(τ)` contract (per-type size/align/offsets, header format, `DbRef` encoding, null representation, the nullable/keyed-dense/narrow-int axes) + the sidecar format + the invariant *"no silent cross-version misread"*; rules + deviation `D-layout-1` (cite #477). Wire into `formal/README.md` + `formal/ROADMAP.md` | ✅ **SHIPPED** — `formal/layout.md`: rules `L-Total`/`L-Scalar`/`L-Narrow`/`L-Ref`/`L-Struct`/`L-Enum`/`L-Tuple`/`L-Null`/`L-Sound` (`L-Null`: nullability is a sentinel, not a layout; `L-Sound`: the no-silent-misread invariant) + `D-layout-1`; wired into the formal README (Areas) + ROADMAP + a heap.md cross-link; doc-drift clean |
| **D** — Schema-description sidecar *(self-describing store)* | A separate file (the `.dmeta` pattern) carrying `(data_to_json schema, layout-algo hash)`; on load, compare vs the program's in-memory identity → identical / Drop&Add-compatible / needs-migration / reject-via-`on_corruption`. Payload untouched. Enabler for E | ◐ **slice 1 SHIPPED** — `src/schema_sidecar.rs`: `LayoutIdentity` (hash + layout dump) built from the live type table; the `.dschema` sidecar text (round-trips, versioned, bad input → reject); `classify(old,new) → Handoff::{Identical (raw handoff), Changed(LayoutDiff)}` + `LayoutDiff::is_actionable()` (reshape ∨ add∧drop → Phase F). 5 unit + 1 e2e test (identity hash == the golden's pinned hash). **Slice 2 SHIPPED** — the `.dschema` file lifecycle: `dschema_path`, `LayoutIdentity::write_beside` (atomic), `check_beside → SchemaVerdict{Fresh/Match/Changed/Unreadable}`, `as_corrupt_reason → CorruptReason::SchemaMismatch` (routes a changed layout through the existing `on_corruption` rebuild). Tested end-to-end beside a store path. **✅ Phase D substantially complete.** Residual: add `data_to_json` to the sidecar for E2 value migration; the actual durable-store CONSUMER wiring lands when a loft builtin drives the durable store (@PLAN38) |
| **E1** — Add&Drop *(falls out of the shared path)* | **Not a dedicated mechanism** — it IS the handoff's serialize→deserialize path with the hash as trigger (D): lenient deserialize *defaults* an added part and *ignores* a dropped one (the 08-S5 `show_json`/`populate_struct_from_jsonvalue` behaviour, made formal over the store, using the sidecar's old schema as the map). E1's work is to **prove** the shared path + lenient rules cover every additive/subtractive change — no new code path | ✅ **PROVEN** — `tests/scripts/510-layout-e1-add-drop.loft`: a schema change round-trips through the one lenient path (`to_json` / `Struct.parse` = `show_json`/`struct_from_jsonvalue`) — an added field defaults, a dropped field is ignored, preserved fields intact; interp + native + wasm, leak-clean. No new code path. **Finding:** an added `not null` field defaults to *null* (old data has no value) — needs a default supplied elsewhere |
| **E2** — Data migration: reshape *(later — deferred)* | The hard but **rare** case: values must be *transformed*, not just added/dropped. **Simplify via expand→contract:** never reshape in place — go through an intermediate **superset** schema so every *step* is a pure Add or pure Drop (E1's path), and the only custom logic is an **additive backfill** (compute new from old) while both coexist. Bounds the hard part to the backfill. **Trigger:** a real reshape must preserve live data (dogfood-driven) | Deferred |
| **F** — Compiler migration aid *(the developer surface)* | The programmer just edits structs; the machinery stays invisible. The compiler diffs the current schema vs. a recorded **baseline** and classifies: identical (silent) · **adds only** (auto) · **drops only** (auto) · **adds ∧ drops together** (the **actionable state** — indistinguishable from a rename/reshape, so surface a diagnostic + an auto-generated **migration-script outline**: mechanical parts pre-filled, only the old→new backfill left as stubs). Accepting a migration updates the baseline. Ties into @PLN28 diagnostics | ✅ **SHIPPED** — `src/schema_sidecar.rs` (baseline `.loft/layout.lock`, `describe_change`, `migration_outline`, `program_roots`) + the **`loft layout accept|check` CLI** (`main.rs`, self-contained/opt-in — zero cost to a normal build). `check` → no-baseline / unchanged / note (auto add-drop) / warning + written `.loft/migration_outline.loft` (actionable). Verified e2e; regression-tested |
| **G** — Remote working-set store loader ([#522](https://github.com/loft-lang/loft/issues/522)) *(consumer + cross-network validation)* | Materialise the working set of a **remote** store over HTTP range reads (`store_load_keys`/`store_load_range`) so a phone (routing) or a **browser game** (asset streaming) pulls only the bytes a query/scene touches — the layout contract's hardest consumer *and* its ultimate test (the layout must be stable AND portable to range-read a remote store; the identity gate rejects a mismatched one, never misreads over the wire). Design: [REMOTE_STORE_LOADER.md](REMOTE_STORE_LOADER.md) | **Design** — build gated on P0 (the read-virtualization + non-regression probe). Two invariants (byte-identical working set · **zero cost to non-users**); read virtualised as a **self-contained Rust builtin** (no new opcodes → IR generation unchanged, no op-set mixing, hot path untouched); 6 verifiable phases (0 probe → 1 heap-load → 5 #517) |

## Phase ordering

1. **A** (census) — the axes everything else covers.
2. **B** (golden test) — the **critical path**: it delivers the "caught by verification, not by
   breaking" guarantee and *defines the layout-algo hash* the rest consume. Ship this first.
3. **C** (formal doc) and **D** (sidecar) proceed in parallel after B — both read B's hash.
4. **E1** (Add/Drop) rides on D's self-describing store. **E2** stays deferred behind its trigger.
5. **F** (compiler migration aid) rides on the schema diff (D) — the developer surface that makes
   the whole thing invisible; the actionable state (add ∧ drop) is where it earns its keep.

**Detailed, file-grounded build steps: [IMPL.md](IMPL.md)** (per-step *what / where / verified*,
the critical path, and the verification gates).

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
5. **Add/Drop vs. reshape boundary:** which changes are pure add/drop (E1's shared path handles
   them — fields, enum arms, widening) vs. a reshape that needs E2 (reordering, narrowing that
   loses bits) — and how far **expand→contract** can decompose a given reshape into a sequence of
   pure Add/Drop steps + an additive backfill, shrinking E2's custom code toward zero.
6. **The baseline the compiler diffs against (F):** where the recorded previous schema lives (a
   committed schema descriptor / lockfile), when it updates (on accepting a migration), and
   whether the aid is compile-time (proactive, vs. the committed baseline) and/or runtime (on
   load, vs. the store's sidecar). The detectable trigger for the actionable state is a schema
   diff with **both** an add and a drop.

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
- **#522 (remote working-set store loader) — now arc G** (see the Sub-arcs table +
  [REMOTE_STORE_LOADER.md](REMOTE_STORE_LOADER.md)). Folded in as the contract's hardest consumer +
  cross-network validation: it range-reads a *remote* store's bytes over HTTP, walking the arena
  layout (`layout.md` — the frame + `L-Total`/`L-Ref`), so it **gates on the layout identity**
  (`schema_sidecar::check_beside`) before trusting a remote store — Phase D's load-time gate across
  a **network** boundary (a mismatched remote store is `SchemaMismatch`, never misread over the
  wire). #522's premise — *a store file IS its own serialization, byte-exact native↔wasm* — is
  exactly what @PLN97's golden + both-backend parity guarantee; and it drove the arena/frame layout
  now in `layout.md`.
- **Layout axes the corpus must cover:** @PLN25 (null/keyed-dense), narrow-int (#399), #477
  (nested-vector stride — the motivating instance).

## See also

- [`formal/README.md`](../../formal/README.md) — the strict-spec model (rules + deviations to zero)
  this doc joins · [`DATABASE.md`](../../DATABASE.md) / [`INTERMEDIATE.md`](../../INTERMEDIATE.md) —
  the store / `Value`·`Type` reference this formalises.
