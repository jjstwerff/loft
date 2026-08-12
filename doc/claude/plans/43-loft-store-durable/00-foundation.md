<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Foundation: integrity + tail marker on existing Store

**Status:** ✅ **SHIPPED** — merged to `main` with [Phase 01](01-tier-1-integrity.md) as one
PR ([PR #219](https://github.com/loft-lang/loft/pull/219), commit `d494edc`). The foundation
(integrity + `.dmeta` sidecar) is live in `src/store.rs`; non-durable stores are bit-for-bit
unchanged.

## Goal

Land the on-disk shape that all three durability tiers share,
with **zero behaviour change** for non-durable stores.  No
durability machinery yet; just the integrity layer that the
later tiers build on.

## What ships

### Implementation design — sidecar `.dmeta` file

**Locked in implementation:** the original plan described an
*inline* header (32 B prefix, payload starting at offset 32,
16 B tail) embedded into the main store file.  Implementation
review (2026-05-26, during phase 00 + 01 build-out) found this
required offsetting `Store::ptr` and threading a `header_offset`
through every record-claim / resize / mmap-grow path —
non-trivial for the "zero behaviour change for non-durable
stores" guarantee.

The shipped design uses a **sidecar metadata file** instead:

```
<path>          ← main store file (legacy "Sto1" format, unchanged)
<path>.dmeta    ← 40-byte durable-metadata sidecar
```

- The main store file is bit-for-bit identical to a non-durable
  Store.  All existing record/claim/resize code paths are
  untouched.  A durable store's main file *is* a legacy store
  in every respect except for the presence of `.dmeta`.
- The sidecar holds signature + tier + CRC + timestamps.  It
  is rewritten atomically on every clean drop and read on every
  open.
- Crash semantics: a partial main-file write leaves the
  sidecar's `payload_crc` stale relative to the on-disk
  bytes → corruption detected → `on_corruption` fires.  A
  process killed before the sidecar is rewritten loses the
  fresh `last_clean_ns` marker → also detected.

Rationale for the sidecar over inline:

1. **Zero impact on Store's hot paths.**  Every record offset
   computation in `src/store.rs` stays unchanged.  No
   header-offset threading through `addr_mut`, `claim`,
   `fl_take_ge`, `resize_store`.
2. **Phase 2 (snapshots) already uses multiple files** per
   the README design (`world.A.store`, `world.B.store`,
   `.checkpoint` pointer).  Sidecar is consistent with that.
3. **Crash window is the same.**  Inline tail-write + sidecar
   write both have a brief vulnerability window between writing
   the CRC and the file becoming durable on disk.  Tier 1
   doesn't claim to close that window — it claims to *detect*
   any close that happens inside it.
4. **Simpler to write atomically.**  The sidecar is small (40
   bytes), so writing it via `write-tmp + rename` is one
   syscall pair.  An inline tail-write into a multi-MB mmap
   region is not naturally atomic.

### Format additions

Existing non-durable stores keep `SIGNATURE = "Sto1"` and no
durability machinery — they're untouched by this phase.

Durable stores have a 40-byte sidecar at `<path>.dmeta`:

```
Offset  Size  Field
------  ----  -----
0       8     signature: "DStoreV1" (ASCII, no NUL)
8       2     tier_id (u16 LE: 0 = none, 1 = IntegrityOnly,
                              2 = SnapshotEvery, 3 = WAL)
10      2     flags (u16 LE, reserved bitfield, currently 0)
12      4     header_crc (u32 LE: CRC32 of bytes 0..12)
16      8     last_clean_ns (u64 LE: nanoseconds since UNIX_EPOCH
                            at last graceful close, 0 if never)
24      8     payload_len (u64 LE: byte length of main file at
                          clean-close time)
32      4     payload_crc (u32 LE: CRC32 of main file's first
                          payload_len bytes)
36      4     reserved (u32 LE, must be 0)
```

The `payload_crc` is the equivalent of the original plan's
"tail CRC", and `last_clean_ns` is the equivalent of the
original plan's "tail marker present".  Both checks are now
done by reading + validating the sidecar, not by walking to
the end of the main file.

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

Use `crc32fast = "1"` — already transitively available via
`flate2`/`zip` in loft's dep graph (and used by them in
production paths), so promoting it to a direct dep adds zero
binary weight and zero new transitive deps.  Hardware-
accelerated (`pclmulqdq` on x86, `crc32` on ARM).

Add to `Cargo.toml`:

```toml
[dependencies]
crc32fast = "1"
```

(The original plan suggested `crc32c = "0.6"`.  Switched to
`crc32fast` during implementation since it was already in the
dep graph; the polynomial difference is irrelevant for this
"detect bitrot / torn writes" use case — both are 32-bit CRCs
with very similar collision properties.)

### What does NOT ship in phase 00

- No `Store::open_durable` API yet (phase 01).
- No tier 2 / tier 3 mechanics (phases 02 / 03).
- No sidecar writing — phase 01 writes the `.dmeta` file on
  clean close.  Phase 00's `detect_format` and
  `validate_integrity` only *read* sidecars.
- No callbacks, no rebuild paths, no snapshots.

The bar for phase 00: `cargo test` passes with the new
format types added; `Store::detect_format` correctly
distinguishes legacy from durable signatures on synthetic
test fixtures.

## Critical files

| Path | Action |
|---|---|
| `src/store.rs` | EXTEND: add `StoreFormat`, `StoreIntegrity`, `CorruptReason`, `detect_format`, `validate_integrity` (read sidecar) |
| `Cargo.toml` | ADD `crc32fast = "1"` to `[dependencies]` |
| `tests/store_durable_format.rs` | NEW: synthetic-fixture tests for format detection + integrity validation |

## Existing functions / utilities to reuse

- `Store::new` / `Store::open` already handle the
  legacy `"StoreV01"` signature.  The new `detect_format`
  reads first 8 bytes and dispatches.
- `Store::ptr` + raw read primitives — used by
  `validate_integrity` for the CRC pass.
- The `Stores::allocations` machinery — durable stores
  live alongside non-durable in the same `Vec<Store>`.

### Open-path refactor — not needed under the sidecar design

The original phase-00 spec proposed an `open_with_format(path,
expected)` helper to bypass `Store::open`'s hard-coded
signature assertion (`src/store.rs:297-301`).  With the
sidecar design that refactor is **unnecessary**: the main
store file's on-disk signature stays `"Sto1"` for both legacy
and durable stores, so `Store::open`'s existing assertion is
never hit by `open_durable`.  The signature distinction lives
entirely in the sidecar.

Phase 01 calls the existing `Store::open(path)` directly after
validating the sidecar — no signature-check refactor required.

## Test surface

`tests/store_durable_format.rs`:

- Fresh non-durable Store path (no `.dmeta` sidecar present)
  → `detect_format` returns `StoreFormat::Legacy`.
- Synthetic main file + valid sidecar →
  `detect_format` returns `Durable(1)`,
  `validate_integrity` returns `Clean`.
- Sidecar with corrupted `header_crc` →
  `Corrupt(HeaderCrcMismatch)`.
- Sidecar missing (main file exists but `.dmeta` absent) →
  `Corrupt(TailMarkerMissing)` (semantic: clean-close never
  ran).
- Sidecar's `payload_crc` mismatches the recomputed CRC of
  the main file's bytes → `Corrupt(TailCrcMismatch)`.
- Main file's on-disk byte length differs from sidecar's
  `payload_len` → `Corrupt(TruncatedFile)`.
- Sidecar's `signature` field is not `"DStoreV1"` →
  `Corrupt(SignatureMismatch)`.

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
- [`DATABASE.md`](../../DATABASE.md) — gets a § "Durable stores" subsection in phase 06 closeout
