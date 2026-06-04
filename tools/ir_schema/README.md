<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# tools/ir_schema — store-schema generator for the compiler IR (@PLAN54 arc A)

The hybrid pipeline that turns the compiler IR into a store-schema
registration, so `Data`/`Value`/`Type`/… can live as `Stores` records (the
plan-54 mmap end-goal) and be walked by the schema-driven inspection layer
(`Stores::show_json`).

## Pipeline

```
ir.loft  ──(loft --native --show-rust)──▶  generated.rs  ──(extract.py)──▶  schema block
                                                                  │
                              hand-written typed API (src/data_store.rs) ◀──┘
```

1. **`ir.loft`** — the COMPLETE IR transcribed as loft `struct`/`enum`
   declarations (source of truth, hand-maintained to mirror `src/data.rs`).
   Verified to parse + lay out under `--interpret`.
2. **`generated.rs`** — `loft --native --show-rust` output (gitignored,
   ~139 KB; regenerate, never edit):
   ```
   loft --introspect --show-rust --rust-out tools/ir_schema/generated.rs tools/ir_schema/ir.loft
   ```
   Its `init(db)` body holds the authoritative schema: `db.structure` /
   `db.enumerate` / `db.field` / `db.value` / `db.vector` with every field
   offset / width / discriminant resolved by the compiler.
3. **`extract.py`** — pulls ONLY the IR-type registrations out of `init`.
4. **hand layer** (`src/data_store.rs`) — the match-able typed API
   (`store_type(&Type) -> DbRef`, a `StoreType` reader) layered on the
   extracted schema; the bit `--native` does not generate.

Why hybrid: generation owns the schema (mechanical, regenerate when the IR
changes — no hand-sync of ~940 sites); the ergonomic typed API is hand-coded
(codegen never emits an accessor layer — see plan-54 § "What the generated
Rust gives us").

## Extraction findings (2026-06-01)

`extract.py` (probe mode) established:

- **The IR block is name-selectable, not line-sliceable.**  In `init`, our
  type registrations are interleaved with stdlib `db.value(...)` lines (e.g.
  `FieldValue`, `JsonValue` variants) that share the line range but are NOT
  ours.  The extractor selects by type name (`TypeT` / `Node` / the listed
  structs / `Ty*` / `Nd*` variants), not by line span.
- **Our IR types depend only on BASE types** (`t0..t6`: integer 0, single 2,
  float 3, boolean 4, text 5, character 6) — confirmed by scanning every
  `field`/`value` line targeting an IR type.  No stdlib-type dependency.  So
  the extracted block is self-contained: emit a base-type prelude, then our
  registrations with `tN` ids rebased.
- **Variant names are global** and must be unique + CamelCase, hence the
  `Ty`/`Nd` prefixes in `ir.loft` (plan-54 § Arc A reference, finding 1).
- **`--native` needs dependency-respecting definition order** (`Block` before
  `Node`, etc.) or it emits a forward-reference `E0425`; `ir.loft` is ordered
  accordingly.  Moot under `--interpret`.

## Maintaining

When the IR (`src/data.rs`) changes, update `ir.loft` to match, regenerate
`generated.rs`, re-run `extract.py`, and refresh the hand layer's field
accessors.  The round-trip test (native `Data` → store → read back) is the
guard that the transcription stayed faithful.
