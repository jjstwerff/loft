<!-- Copyright (c) 2026 Jurjen Stellingwerff -->
<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->

# @PLN102 arc E — F9: the layout identity must distinguish `τ` from `τ?`

> **Status: DESIGN (2026-07-19).** The nullability half of F9 (the endianness half
> shipped 2026-07-17). A raw-persistence handoff between a full-width `integer` store
> and an `integer?` store is currently accepted as "same layout" — it is a silent data
> break the freeze must close. This designs the audit-preferred **fix (a)** (add a
> schema component to the layout identity) as an additive, inert-first ladder, records
> **fix (b)** as the more-invasive alternative, and pins the sentinel edge hazard.
> Root-cause + code-points from the 2026-07-17 investigation; line numbers below are
> current-tree (they drift — grep the named symbol).

## The defect (root-caused)

`LayoutIdentity` (`src/schema_sidecar.rs:26`) is `{ layout_hash, dump }` — an FNV-1a
over `Stores::layout_dump` and nothing else. `layout_dump` renders a full-width
nullable field identically to its non-null twin: `db_type` (`src/database/types.rs`,
the `else { self.name("integer") }` arm ~`:1549`) maps BOTH `integer` and `integer?`
to the same `integer` known-type — full-width nullability (`Type::Optional`) is
DROPPED at resolution, before the storage table. (Only NARROW ints keep it, via
`Parts::Byte(_, nullable)` → the `null=` dump token; full-width has no per-`Field`
flag.) So `NN{i:integer}` and `NL{i:integer?}` produce a byte-identical dump → equal
`layout_hash` → the load gate `layout_gate_ok` (`src/database/allocation.rs:2710`)
classifies the cross-nullability handoff as `Match` and reads the foreign bytes raw.

The nullability the identity drops **is** carried by the schema:
`ir_schema::data_to_json` preserves `Type::Optional` explicitly
(`src/ir_schema.rs:122`, with a symmetric `data_from_json` read-back). The
`layout_algo_hash` doc-comment (`src/database/types.rs:1685`) already says *"pair it
with the schema (`ir_schema::data_to_json`) for the full identity"* — **the gap is
simply that the guard never pairs with the schema.** That is the whole fix.

## The invariant (design-protocol step 1)

> **Two stores raw-hand-off-compatible ⟺ their persisted values are
> reinterpretation-safe in BOTH directions.** A nullability difference on a full-width
> field is NOT reinterpretation-safe (a value equal to the null sentinel means
> "present" under `τ` and "absent" under `τ?`), so it must make the identities differ
> and the gate refuse. Additive corollary: adding the schema to the identity may only
> ever make the gate *refuse more* (fail-safe), never accept a handoff it refused
> before.

## Why fix (a), not fix (b)

Both close the defect; they differ by blast radius.

- **(a) — add a schema component to the identity (RECOMMENDED, matches the doc).**
  Purely *additive* to the identity: the storage layout, sizes, and sentinel behaviour
  are untouched. The identity gains a `schema` field from `data_to_json`; the gate
  compares it. Risk is concentrated in one place (the `classify` comparison) and in the
  plumbing (`Data` is not in scope at the deep guard — solved by precomputing the
  schema string where `Data` IS live and stashing it on `Stores`).
- **(b) — carry full-width nullability into the storage table.** A per-`Field`
  full-width-nullable flag set in `db_type`/`fill_database`, rendered like narrow's
  `null=`. This changes the resolution→layout pipeline for **every** full-width
  nullable field in the language — a broad blast radius (sizes, sentinels, every
  `τ?` store), and it is a semantic pipeline change, not an identity refinement. Kept
  as the documented alternative if (a)'s schema-scoping proves inadequate; not the
  first choice.

## Fix (a) — the inert-first ladder

Each step is one commit; steps 1–3 are byte-identical/inert (no gate behaviour change),
the behaviour flip is isolated to step 4, and step 5 protects already-persisted data.

| # | Step | Proof | Effort |
|---|---|---|---|
| 1 | **Add an inert `schema: String` to `LayoutIdentity`** (default empty), leaving `layout_hash` and the `classify` comparison UNCHANGED. `of()` keeps its `(stores, roots)` signature and sets `schema: String::new()`. | existing sidecars parse; hash + `is_raw_safe` verdicts byte-identical (positive: a full run of the persist/load tests unchanged) | S |
| 2 | **Compute the program schema where `Data` is live and stash it on `Stores`.** Add `Stores.layout_schema: String`, populated once at store/program setup (the CLI paths where both `p.data` + `p.database` exist — `src/main.rs:3638`, `:3950`, and the run/persist entry). Scope it to the **root closure** (a `schema_for_roots(data, roots)` mirroring `program_roots` + `layout_closure`) so an unrelated type's nullability change does not over-refuse; whole-program `data_to_json` is the conservative fallback (note: over-scoping = a false refusal = a compatibility break, so prefer the scoped form). Nothing reads the field yet. | the field is populated on both backends; suite byte-identical (nothing consumes it) | S–M |
| 3 | **Carry the schema into the identity + sidecar text.** `of()` reads `stores.layout_schema` into `schema`; `to_sidecar`/`from_sidecar` gain a `schema=<…>` line (written + parsed), like the `@endian` line. Still NOT in `classify` — carried, not compared. | sidecar round-trips with the new line; an OLD sidecar (no `schema=`) parses as "schema absent" (grandfather, step 5); hash still excludes it → byte-identical verdicts | M |
| 4 | **The flip — compare schema in `classify`** (`src/schema_sidecar.rs:115`). Two identities are `Identical`/`Match` only if hash AND dump AND (both-present) schema agree; a schema mismatch → a `LayoutDiff` → `is_raw_safe()` false → the gate refuses. Behaviour change isolated here. | positive control `not_null_flip_is_never_a_raw_handoff` (mirror `endian_flip_is_never_a_raw_handoff`): `NN{i:integer}` vs `NL{i:integer?}` → refused; identical schema → still `Match`; both backends | M |
| 5 | **Grandfather already-persisted stores (never-break for data).** A sidecar written before step 3 has no `schema=` line → treat absent-schema as *unknown*, and fall back to the pre-F9 verdict (hash+dump match ⇒ allow) so no EXISTING persisted store is suddenly refused. The tightening applies only to sidecars that DO carry a schema (written at/after this change). Re-persisting upgrades a store to the strict identity. | an old-format sidecar (schema absent) with matching hash still loads; a new-format one with a nullability diff refuses; documented in DATABASE.md / @PLN97 notes | S–M |
| 6 | **Fold F9 into the flip-gate baseline.** The layout-hash golden ([flip-gate.md](flip-gate.md) gate 1) regenerates to include the schema line, re-blessed once. | the golden includes `schema=`; a nullability change to a persisted type now trips gate 1 | S |

## The sentinel edge hazard (named, not dropped)

A full-width value that equals the reserved null sentinel is the genuine hazard: under
`τ` it is a real value, under `τ?` it is *absent*. Fix (a) does not change storage, so
it does not itself create such a value — it **refuses the handoff** that would
reinterpret one across the nullability boundary, which is exactly the right outcome
(the invariant). The residual to verify: a store persisted as non-null `integer` whose
bytes happen to hold the sentinel value, loaded by a program expecting `integer?` —
fix (a) refuses it (schema differs), so the ambiguous read never happens. A probe in
step 4 should assert this refusal explicitly (persist `integer` with the sentinel
value → attempt an `integer?` raw load → refused, not silently read as null).

## Falsification (design-protocol steps 3–4)

- **"Additive can only refuse more."** Attacked: if step 2 over-scopes the schema
  (whole-program), an unrelated nullability edit refuses a genuinely-safe load — that
  IS a break (false refusal). Mitigation: scope to the root closure (step 2); the
  whole-program form is only a labelled conservative fallback, and step 5's grandfather
  keeps existing stores safe regardless.
- **"The schema carries nullability."** Verified, not assumed: `ir_schema.rs:122`
  preserves `Optional` and `data_from_json` reads it back — a round-trip test already
  exists; extend it to assert `integer` vs `integer?` serialize differently.
- **"Plumbing is safe."** The deep guard has no `Data`; the design stashes a
  precomputed string on `Stores` rather than threading `Data` through `bind_path`.
  Risk: the stash is stale if the schema isn't recomputed when the bound program
  changes. Mitigation: populate at the single program→store binding point; a probe
  binds two different-schema programs to one Stores and asserts the identity tracks the
  current one.
- **"Fix (a) suffices."** If schema-scoping cannot be made precise without effectively
  reconstructing the layout table, fall back to fix (b) (per-`Field` flag) — recorded
  above with its broader blast radius; do not silently half-do (a).

## See also
- `src/schema_sidecar.rs` — `LayoutIdentity` / `of` / `classify` / the sidecar text (the fix's home).
- `src/ir_schema.rs:122` (`write_type` Optional arm), `:1382` (`data_to_json`) — the nullability source.
- `src/database/types.rs:1549` (`db_type` full-width drop), `:1685` (`layout_algo_hash` doc: "pair with the schema"), `:1704` (`layout_dump` + `@endian` precedent).
- `src/database/descriptor.rs:295` — the dump twin (kept in sync by `descriptor_render_reproduces_layout_dump`).
- `src/database/allocation.rs:2540` (persist/write sidecar), `:2710` (`layout_gate_ok` load gate).
- [formal-audit.md](formal-audit.md) § F9 · [flip-gate.md](flip-gate.md) (the Formats precondition) · [../../DATABASE.md](../../DATABASE.md) / @PLN97.
