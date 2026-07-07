<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 01 — Tier 1: IntegrityOnly + auto-rescan hook

**Status:** ✅ **SHIPPED** — merged to `main` with [Phase 00](00-foundation.md) as one PR
([PR #219](https://github.com/loft-lang/loft/pull/219), commit `d494edc`). Tier 1
`IntegrityOnly` (`Store::open_durable` + `DurabilityMode::IntegrityOnly`) is live in
`src/store.rs`; test coverage 10/10 `store_durable_format`, 7/7 `store_durable_tier1`.

## Goal

Ship the cheapest tier — `Store::open_durable(path,
DurabilityMode::IntegrityOnly { on_corruption })` — and prove
it end-to-end with the **training port** (`personal/training`,
`loft-migration` branch) as the first real consumer.  The
indexer (@PLN42 phase 08) becomes the second opt-in when that
plan reaches its consumer phase.

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

### Fresh-file path

When `path` does not exist yet (brand-new database), the same
"corruption" code path fires: `detect_format` returns an
error, `open_durable` invokes `on_corruption(path)`, and the
callback is expected to **create an empty store and populate
it from authoritative sources**.

Consumers MUST therefore implement `on_corruption` as a
"rebuild OR initialise" routine — not a "repair existing
file" one.  This is the natural shape for both the training
port ("re-run sync→store from cached JSON, writing a fresh
file if none exists") and the indexer ("full filesystem
rescan, writing a fresh `tags.store` if none exists").

The `STDLIB.md` doc entry for `Store::open_durable` must
spell this out so first-time consumers aren't surprised by
the callback firing on day 1.

### Drop-on-panic — by design

A panic anywhere between `open_durable` and a clean drop will
skip the tail-marker write.  Next open detects the missing
marker and fires `on_corruption`.  This is the **whole point**
of Tier 1 — no msync on the hot path, recover by rebuild.

`STDLIB.md` must include a "when not to use Tier 1" callout:
do not use it for data that can't be re-derived from another
source.  For irreplaceable data, use Tier 2 (snapshots) or
Tier 3 (WAL), shipped in phases 02 / 03.

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
- Tier 1 consumer integration: the **training port**
  (`personal/training`, `loft-migration` branch) opens its
  store via `IntegrityOnly`; `on_corruption` re-runs the
  sync→store pipeline from cached JSON.  Verified: `kill -9`
  mid-write + restart → detected corruption → rebuild → clean
  state.  (Training-port wiring lives on its own branch and
  is not part of this PR's diff; only the loft-side API
  surface and tests land here.)
- The indexer (@PLN42 phase 08) opt-in is deferred to phase
  05 of this plan (when @PLN42 reaches its consumer phase).
- Non-durable `Store::open` and `Store::new` paths
  unchanged.
- No msync calls outside the clean-close path (perf
  parity with non-durable for the hot write loop).
- `STDLIB.md` § "Durable stores" documents `Store::open_durable`
  with the **fresh-file path** and **drop-on-panic** callouts
  spelled out so consumers aren't surprised on day 1.

## Risks

| Risk | Mitigation |
|---|---|
| Drop is not guaranteed to run on panic | Acceptable — that's the WHOLE point of Tier 1's "tail marker absent → rebuild" path |
| `on_corruption` callback is heavyweight (full rescan) and gets called accidentally on benign cases | Tighten validation: only TRULY corrupted files trigger the callback; clean-close detection must be unambiguous |
| Recursive `open_durable` after rebuild loops if `on_corruption` doesn't actually fix the file | Cap recursion depth at 1 — second corruption in a row → return error |
| Tier 1 advertises itself as the cheap option but consumers misuse it for stake-bearing data | Phase 06 closeout's STDLIB.md doc has a clear "when to use which tier" section; named modes (`IntegrityOnly` vs `WAL`) make the difference visible at the call site |

## Cross-references

- [Phase 00 — foundation](00-foundation.md) — provides the
  format detection + integrity validation; bundled with this
  phase in the same PR
- [Phase 02 — Tier 2 snapshots](02-tier-2-snapshots.md) —
  next tier up; reuses the same DurabilityMode enum
- `personal/training` repo (`loft-migration` branch),
  `MIGRATION.md` § "Loft capability gaps" #2 — first
  Tier-1 consumer (external to this repo)
- [Plan-37 phase 08](../42-tracker-index/08-multi-project-deploy.md)
  — second Tier-1 consumer, opts in when @PLN42 reaches its
  consumer phase
