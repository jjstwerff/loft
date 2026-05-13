<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 01 — Tier 1: IntegrityOnly + auto-rescan hook

**Status:** Open

## Goal

Ship the cheapest tier — `Store::open_durable(path,
DurabilityMode::IntegrityOnly { on_corruption })` — and prove
it end-to-end with the indexer (@PLAN37) as the first
consumer.

Tier 1 is "trust the OS for in-flight writes; on restart,
detect corruption and let the caller rebuild from
authoritative sources."  No msync discipline, no snapshots,
no WAL.  Suitable for any data where rebuild from another
source is cheap.

## What ships

### `DurabilityMode` enum

```rust
pub enum DurabilityMode {
    /// Tier 1.  Validate signature + tail marker on open;
    /// on corruption, call `on_corruption` (which is
    /// expected to rebuild from authoritative sources, then
    /// the caller re-opens the store fresh).
    IntegrityOnly {
        on_corruption: Box<dyn Fn(&Path) -> io::Result<()>>,
    },

    // Tier 2 + 3 added in subsequent phases:
    // SnapshotEvery { ... },
    // WAL { ... },
}
```

### `Store::open_durable`

```rust
impl Store {
    pub fn open_durable(
        path: &Path,
        mode: DurabilityMode,
    ) -> io::Result<Store> {
        match mode {
            DurabilityMode::IntegrityOnly { on_corruption } => {
                let format = Self::detect_format(path)?;
                match format {
                    StoreFormat::Durable(1) => {
                        match Self::validate_integrity(path)? {
                            StoreIntegrity::Clean => {
                                Self::open(path)  // existing mmap path
                            }
                            StoreIntegrity::Corrupt(reason) => {
                                eprintln!("store_durable: {path:?} corrupt: {reason:?}; rebuilding");
                                on_corruption(path)?;
                                Self::open_durable(path, mode)  // re-validate
                            }
                        }
                    }
                    StoreFormat::Durable(other) => {
                        // Wrong tier on disk; rebuild
                        on_corruption(path)?;
                        Self::open_durable(path, mode)
                    }
                    StoreFormat::Legacy => {
                        // Migrate: rebuild as durable
                        on_corruption(path)?;
                        Self::open_durable(path, mode)
                    }
                }
            }
        }
    }
}
```

### Clean-close protocol

When a Tier 1 Store is dropped cleanly, the destructor:

1. Computes the tail CRC.
2. Writes the tail marker (`"DStoreCommit"` + CRC) +
   `last_clean_ns` timestamp.
3. `msync(MS_SYNC)` — ONE explicit msync before drop.

If the process is killed before drop runs, the tail marker
is absent → next open detects corruption → rebuild.

This is the ONLY msync Tier 1 does.  All other writes go
through the regular page-cache path.

### Format migration path

Existing legacy stores (signature `"StoreV01"`) opened via
`open_durable` are treated as "needs rebuild" — the
`on_corruption` callback fires, the caller rebuilds, and
the new file is written in `DStoreV1` format.

This lets consumers migrate gradually: change the open call
from `Store::open` to `Store::open_durable`, ship one
rebuild-time cost on first run, and gain integrity on
subsequent restarts.

## Critical files

| Path | Action |
|---|---|
| `src/store.rs` | EXTEND: `DurabilityMode` enum (Tier 1 variant only); `open_durable` impl; `Drop` writes tail marker |
| `tests/store_durable_tier1.rs` | NEW: open_durable round-trips; corruption detection; rebuild-callback semantics |

## Existing functions / utilities to reuse

- `Store::detect_format`, `Store::validate_integrity` from
  phase 00.
- Existing `Store::open` for the happy path (clean file).
- `Store::ptr` + raw write primitives for the tail-marker
  write at clean close.

## Test surface

`tests/store_durable_tier1.rs`:

- Open a fresh path → `on_corruption` fires (file doesn't
  exist yet); after rebuild, second open succeeds clean.
- Open a clean durable store → no callback; mmap returns.
- Open a store with corrupted tail → callback fires; after
  rebuild, second open succeeds.
- Drop a Store cleanly → next open finds the tail marker.
- Simulate kill (drop without running the destructor) →
  next open misses the tail marker → callback fires.
- `on_corruption` returns Err → `open_durable` returns the
  same error to the caller (no infinite loop).

## Acceptance

- `cargo test --test store_durable_tier1` passes.
- `cargo test` overall (legacy + tier 1) green.
- Tier 1 consumer integration: indexer phase 08
  (@PLAN37) opens its `tags.store` via `IntegrityOnly`;
  `on_corruption` is the indexer's full-rescan path.
  Verified: `kill -9` mid-write + restart → detected
  corruption → full rescan → clean state in < 2 sec on
  the loft tree.
- Non-durable `Store::open` and `Store::new` paths
  unchanged.
- No msync calls outside the clean-close path (perf
  parity with non-durable for the hot write loop).

## Risks

| Risk | Mitigation |
|---|---|
| Drop is not guaranteed to run on panic | Acceptable — that's the WHOLE point of Tier 1's "tail marker absent → rebuild" path |
| `on_corruption` callback is heavyweight (full rescan) and gets called accidentally on benign cases | Tighten validation: only TRULY corrupted files trigger the callback; clean-close detection must be unambiguous |
| Recursive `open_durable` after rebuild loops if `on_corruption` doesn't actually fix the file | Cap recursion depth at 1 — second corruption in a row → return error |
| Tier 1 advertises itself as the cheap option but consumers misuse it for stake-bearing data | Phase 06 closeout's STDLIB.md doc has a clear "when to use which tier" section; named modes (`IntegrityOnly` vs `WAL`) make the difference visible at the call site |

## Cross-references

- [Phase 00 — foundation](00-foundation.md) — provides the
  format detection + integrity validation
- [Phase 02 — Tier 2 snapshots](02-tier-2-snapshots.md) —
  next tier up; reuses the same DurabilityMode enum
- [Plan-37 phase 08](../../37-tracker-index/08-multi-project-deploy.md)
  — first Tier-1 consumer
