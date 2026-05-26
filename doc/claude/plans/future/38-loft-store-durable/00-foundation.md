<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Foundation: integrity + tail marker on existing Store

**Status:** Open — bundled into the `store-durable-phase1`
branch with [Phase 01](01-tier-1-integrity.md); ships as one
PR.  See
[README § First slice — Phases 00 + 01 as one PR](README.md#first-slice--phases-00--01-as-one-pr).

## Goal

Land the on-disk shape that all three durability tiers share,
with **zero behaviour change** for non-durable stores.  No
durability machinery yet; just the integrity layer that the
later tiers build on.

## What ships

### Format additions

The existing `src/store.rs` Store layout starts with `SIGNATURE
= "StoreV01"` (4 bytes at offset 0).  The durable variant
extends this:

```
Offset  Size  Field
------  ----  -----
0       8     signature: "DStoreV1\0"
8       2     tier_id (0 = none, 1 = IntegrityOnly, 2 = SnapshotEvery, 3 = WAL)
10      2     flags (bitfield: little-endian, currently reserved)
12      4     header_crc (CRC32 of bytes 0..12)
16      8     last_clean_ns (set on graceful close, zero otherwise)
24      8     reserved
32      ...   payload (existing Store records)
...
end-16  8     tail_signature: "DStoreCommit"
end-8   8     tail_crc (CRC32 of the entire payload region)
```

Existing non-durable stores keep `signature = "StoreV01"` and
no header/tail framing — they're untouched by this phase.

### `src/store.rs` additions

```rust
pub enum StoreFormat {
    Legacy,           // "StoreV01" — no integrity machinery
    Durable(u16),     // "DStoreV1" + tier_id
}

impl Store {
    /// Detect the on-disk format by reading the signature.
    pub fn detect_format(path: &Path) -> io::Result<StoreFormat> { ... }

    /// Validate header + tail integrity for durable stores.
    /// Returns Ok(StoreIntegrity::Clean) if all checks pass,
    /// Ok(StoreIntegrity::Corrupt(reason)) if any fail.
    pub fn validate_integrity(path: &Path) -> io::Result<StoreIntegrity> { ... }
}

pub enum StoreIntegrity {
    Clean,
    Corrupt(CorruptReason),
}

pub enum CorruptReason {
    SignatureMismatch,
    HeaderCrcMismatch,
    TailMarkerMissing,         // file killed mid-write
    TailCrcMismatch,           // payload bytes torn
    TruncatedFile,
}
```

`detect_format` is cheap (reads first 8 bytes).
`validate_integrity` reads the full file and computes the
tail CRC — for the indexer's ~MB-scale store that's <10ms.

### CRC32 utility

Cross-platform; pick a vetted implementation.  `crc32c` crate
(hardware-accelerated where supported) is the lowest-friction
choice; matches the format used by ext4 / btrfs metadata.

Add to `Cargo.toml`:

```toml
[dependencies]
crc32c = "0.6"
```

### What does NOT ship in phase 00

- No `Store::open_durable` API yet (phase 01).
- No tier 2 / tier 3 mechanics (phases 02 / 03).
- No `tail_signature` writing — phase 01 does that on
  clean close.
- No callbacks, no rebuild paths, no snapshots.

The bar for phase 00: `cargo test` passes with the new
format types added; `Store::detect_format` correctly
distinguishes legacy from durable signatures on synthetic
test fixtures.

## Critical files

| Path | Action |
|---|---|
| `src/store.rs` | EXTEND: add `StoreFormat`, `StoreIntegrity`, `CorruptReason`, `detect_format`, `validate_integrity` |
| `Cargo.toml` | ADD `crc32c = "0.6"` to `[dependencies]` |
| `tests/store_durable_format.rs` | NEW: synthetic-fixture tests for format detection + integrity validation |

## Existing functions / utilities to reuse

- `Store::new` / `Store::open` already handle the
  legacy `"StoreV01"` signature.  The new `detect_format`
  reads first 8 bytes and dispatches.
- `Store::ptr` + raw read primitives — used by
  `validate_integrity` for the CRC pass.
- The `Stores::allocations` machinery — durable stores
  live alongside non-durable in the same `Vec<Store>`.

### Open-path refactor (preparatory)

`Store::open` (`src/store.rs:265-307`) currently panics on any
signature other than `"StoreV01"`
(`assert_eq!(... SIGNATURE, "Unknown file format")`).  Phase 01
needs to open `DStoreV1` files via the same mmap setup without
hitting that assertion.

The cleanest seam, introduced here in phase 00, is a private
helper:

```rust
fn open_with_format(path: &Path, expected: StoreFormat) -> io::Result<Store>;
```

`open` becomes a thin wrapper calling
`open_with_format(path, StoreFormat::Legacy)`, and phase 01's
`open_durable` calls it with the durable variant.  The
assertion moves inside `open_with_format` and compares against
the expected format rather than a hard-coded constant.

This refactor lands in phase 00 (no behavior change for legacy
stores; the assertion still fires on truly unknown formats) so
phase 01's diff stays focused on the durable code path.

## Test surface

`tests/store_durable_format.rs`:

- Fresh non-durable Store → `detect_format` returns
  `StoreFormat::Legacy`.
- Synthetic durable store with valid header + tail →
  `detect_format` returns `Durable(1)`,
  `validate_integrity` returns `Clean`.
- Same with corrupted header CRC →
  `Corrupt(HeaderCrcMismatch)`.
- Same with absent tail marker → `Corrupt(TailMarkerMissing)`.
- Same with payload byte flipped → `Corrupt(TailCrcMismatch)`.
- Truncated file (last N bytes removed) →
  `Corrupt(TruncatedFile)`.

## Acceptance

- `cargo test --test store_durable_format` passes.
- `cargo test` overall (all existing tests) passes — no
  regression for legacy stores.
- `cargo clippy --release --all-targets -- -D warnings`
  clean.
- New code documented inline + cross-referenced from
  `DATABASE.md`'s Store section.

## Risks

| Risk | Mitigation |
|---|---|
| `crc32c` crate has a transitive dep that conflicts with loft's existing graph | If it does, fall back to a hand-rolled CRC32 (~50 lines, well-known polynomial); the API at the type level stays the same. |
| Existing tests assume `SIGNATURE = "StoreV01"` and break | Phase 00 doesn't change that signature; only adds new variants.  Spot-check `src/store.rs` for `SIGNATURE` references and confirm the existing path is unchanged. |
| Integrity check on multi-MB stores adds startup latency | Phase 01 makes it lazy / opt-in.  Phase 00 just provides the function; consumers decide when to call it. |

## Cross-references

- [README § Foundation — integrity at the file level](README.md#foundation--integrity-at-the-file-level)
- [Phase 01 — Tier 1: IntegrityOnly](01-tier-1-integrity.md) — first consumer of these types
- [`src/store.rs`](../../../../src/store.rs) — extension target
- [`DATABASE.md`](../../../DATABASE.md) — gets a § "Durable stores" subsection in phase 06 closeout
