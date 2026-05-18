<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN38 — LOFT_STORE_DURABLE — three-tier opt-in durability for mmap stores

**Status:** Planned (in `future/`).  Promote to active when
@PLAN37 phase 07 (loft-native indexer daemon) lands or when
the first game-server plan needs persistent state — whichever
comes first.

A `Store::open_durable` API + three opt-in durability tiers
on top of loft's existing mmap-backed `Store` primitive
(`src/store.rs`).  Each consumer picks the tier that matches
its data's value:

- **Tier 1 — IntegrityOnly**: signature + checksum + tail
  marker; on corruption, fall back to caller-supplied
  rebuild.  Cheap; ideal for derived data (the indexer).
- **Tier 2 — SnapshotEvery(interval)**: double-buffered
  atomic snapshots via POSIX rename; `msync` per snapshot.
  Bounded loss = one interval's worth.  Ideal for
  turn-based game state (TTT v5 sessions).
- **Tier 3 — WAL**: write-ahead log + `fsync` per record +
  periodic checkpoints.  Zero-loss for committed writes.
  Ideal for real-time game state (@PLAN36 audience demo).

## Drivers

The plan exists because two consumers face an "OS hard kill"
failure mode with very different stakes:

1. **Plan-37's loft-native indexer daemon** (test bed) — can
   lose its entire `tags.store`, recover via 0.85 sec full
   rescan from the filesystem.  Gets Tier 1 essentially for
   free; just needs an integrity check so a half-written
   file doesn't crash the daemon.

2. **Future game servers** (TTT v5 in `plans/future/32-…`,
   @PLAN36 audience-generative-art) — hold session state,
   world state, decay timers, audience contributions that
   ARE the source of truth.  An OS-kill mid-game means
   players lose their moves; an audience-demo crash means
   the projector loses every contribution since the last
   restart.  Need Tier 2 or Tier 3 depending on tick rate.

The shared `Store` foundation makes a unified durability
plan cheaper than per-consumer ad-hoc solutions.  Each
consumer opts into the tier it needs.

The user's [evaluation prompt](../37-tracker-index/README.md)
that opened this plan: "the current task is cheap to recover,
but the eventual game server will lose valuable data in the
same event.  So this is lightly tied to the index."

## Architecture

### Foundation — integrity at the file level

Every durable store starts with the same on-disk shape:

```
[8B signature: "DStoreV1"][2B tier-id][2B flags][4B header-CRC]
[N×8B records, each with leading record-CRC][...]
[16B tail marker: "DStoreCommit\0\0\0\0"][8B last-write-ns]
```

- **Signature**: 8 bytes; allows future format evolution.
- **Tier-id + flags**: which durability layer wrote this.
- **Header CRC**: catches corruption of the header itself.
- **Per-record CRC**: catches torn writes within the file.
- **Tail marker**: present iff the last write completed
  cleanly.  Absent → file was killed mid-write → fall back
  to recovery.
- **Last-write-ns**: timestamp of the last clean commit;
  useful for "is this snapshot fresh enough?" checks.

The existing `src/store.rs::SIGNATURE = "StoreV01"` stays as
the marker for non-durable stores; durable stores use a
different signature so a tool can tell at-a-glance which it
is.

### Tier 1 — IntegrityOnly

```rust
let store = Store::open_durable(
    path,
    DurabilityMode::IntegrityOnly { on_corruption: rebuild_callback },
)?;
```

On open:
1. Validate signature + header CRC + tail marker.
2. If anything fails → call `rebuild_callback` (the consumer
   provides this; for the indexer it's "do a full rescan").
3. If validation passes → mmap as usual.

Writes proceed exactly as today.  No `msync` discipline;
trust the OS page cache.  Worst-case loss = page cache
contents on hard power-off.

### Tier 2 — SnapshotEvery(interval)

```rust
let store = Store::open_durable(
    path,
    DurabilityMode::SnapshotEvery {
        interval: Duration::from_secs(5),
        on_corruption: rebuild_callback,
    },
)?;
```

Two slots on disk:

```
.state/
├── world.A.store
├── world.B.store
└── world.checkpoint    ← 4 bytes: "A" or "B"
```

Background thread (or main-loop tick) every `interval`:
1. Determine inactive slot (the one NOT named in `checkpoint`).
2. Memcpy in-memory store into the inactive slot's file.
3. `msync(MS_SYNC)` to force durability.
4. Write inactive slot name to `.checkpoint.tmp`,
   `fsync`, atomic `rename` → `checkpoint`.

Worst-case loss = `interval` seconds of changes.  POSIX
rename atomicity means the file system never sees a
half-committed state.

### Tier 3 — WAL

```rust
let store = Store::open_durable(
    path,
    DurabilityMode::WAL {
        wal_path: ".state/world.wal".into(),
        snapshot_every_n_writes: 1000,
        group_commit_window: Duration::from_millis(2),
        on_corruption: rebuild_callback,
    },
)?;
```

Each write:
1. Append record to `wal_path`: `[8B seq][4B record-len][record bytes][4B record-CRC]`.
2. `fsync(wal_fd)` — within `group_commit_window`, the daemon
   batches multiple writes into one fsync (latency tradeoff).
3. Apply write to in-memory store.
4. If write count since last snapshot > `snapshot_every_n_writes`:
   trigger Tier-2-style snapshot; on success, truncate WAL.

Recovery on open:
1. Load latest snapshot (Tier 2 path).
2. Replay WAL records since snapshot's seq.
3. Resume.

Per-write cost on modern SSD: 0.1-1 ms with grouped commit
(amortised across the group).  Single-fsync cost without
grouping: 1-10 ms.  Acceptable for any turn-based game; for
60Hz action games the group-commit window keeps it
bounded.

## Ground rules

- **Foundation in core, tiers in a package.**  The signature
  + checksum + tail marker live in `src/store.rs`
  (Tier-0/1 baseline).  Tier 2 + 3 machinery lives in a
  new `lib/store_durable/` package — opt-in for consumers
  who need it; doesn't bloat the runtime for those who don't.
- **Consumer chooses the tier at open-time.**  No
  per-write override.  Keeps the API small.
- **The filesystem is the recovery contract.**  Tier 1
  consumers MUST supply a rebuild callback that reconstructs
  from authoritative sources.  Tier 2 + 3 reconstruction
  is the durability layer's responsibility.
- **No transaction semantics.**  Each write is independent;
  no rollback, no isolation between concurrent writers.
  A future plan can add MVCC if it surfaces as a real need.
- **No cross-process coordination.**  Single-writer
  assumption (the daemon).  Multi-process coordination is a
  deliberate non-goal.

## Phases

| # | Phase | Effort | What ships |
|---|---|---|---|
| 0 | [Foundation: integrity + tail marker on existing Store](00-foundation.md) | XS | `src/store.rs` gains `DStoreV1` signature variant + per-record CRC scaffolding (no behavior change for non-durable stores) |
| 1 | [Tier 1: IntegrityOnly + auto-rescan hook](01-tier-1-integrity.md) | S | `Store::open_durable(.., IntegrityOnly { on_corruption })` API; integrity validation on open; corruption → callback fires; first consumer = @PLAN37 indexer |
| 2 | [Tier 2: double-buffered snapshots](02-tier-2-snapshots.md) | M | `lib/store_durable/` package; `SnapshotEvery(interval)` mode with two-file atomic rotation + `msync` discipline |
| 3 | [Tier 3: WAL + grouped commit](03-tier-3-wal.md) | M-MH | WAL append + fsync + checkpoint + truncate; `group_commit_window` to amortise fsync cost across batches |
| 4 | [Stress test — `kill -9` × 1000 across all tiers](04-stress-test.md) | S | `tests/store_durable_kill.rs` runs an injection harness that spawns a daemon, kills it mid-write, validates recovery semantics per tier |
| 5 | [First-consumer opt-in](05-consumer-optin.md) | S | Plan-37 indexer phase 08 selects Tier 1; TTT v5 design doc updated to declare Tier 2 dependency; @PLAN36 audience demo design doc references Tier 3 |
| 6 | [Closeout — DESIGN_DECISIONS + STDLIB.md + finished/](06-closeout.md) | XS | "C-… durability tier choice" decision recorded; STDLIB.md `Store::open_durable` doc; plan moves to `finished/` |

Total estimated effort: **H** (sum of per-phase letters above).
Sequencing: phases 0-3 are foundation+impl (must ship in
order); 4 is the cross-tier validation; 5+6 wire downstream.

## Acceptance — full plan

- `Store::open_durable(path, DurabilityMode::IntegrityOnly{..})`
  detects all of: missing signature, wrong tier-id, header CRC
  mismatch, missing tail marker → calls `on_corruption` once.
- `DurabilityMode::SnapshotEvery(5s)` writes a snapshot every
  5 sec; killing the process between snapshots loses ≤ 5 sec
  of writes; killing during a snapshot leaves the previous
  generation valid.
- `DurabilityMode::WAL{..}` recovery: every write that
  returned successfully to the caller is present after
  `kill -9 + restart` (verified by `tests/store_durable_kill.rs`).
- The indexer (@PLAN37 phase 08) opts into Tier 1; full-
  rescan-on-corruption succeeds in ≤ 2 sec.
- TTT v5 design doc + @PLAN36 audience demo design doc cite
  Tier 2 / Tier 3 as their persistence layer.
- All 7 phases close → plan moves to
  `plans/finished/38-loft-store-durable/`.

## Risks

| Risk | Mitigation |
|---|---|
| `msync(MS_SYNC)` interaction with the kernel page cache differs across Linux / macOS / Windows | Tier 2 + 3 implementations are gated behind a feature flag per OS; test matrix in phase 04 covers all three explicitly. |
| Tier 3 fsync latency dominates throughput on slow disks | `group_commit_window` + per-tier benchmark in phase 04; document expected throughput floors so consumers can pick the right tier |
| Recovery tests are slow (each iteration spawns a process + kills it) | Phase 04's harness uses fork/exec primitives where available; runs 1000 iterations in < 2 minutes target |
| Plan-37 indexer's `tags.store` schema collides with Tier 1 layout requirements | Foundation (phase 00) freezes the layout BEFORE indexer phase 08 commits to it; coordinated via the indexer plan's design |
| Game-server plans grow needs that exceed Tier 3 (e.g., MVCC, multi-writer) | Out of scope; file as a future plan if it surfaces.  Tier 3 covers single-writer durability, which is the biggest win and matches the lib/server WebSocket-driven server architecture. |
| The foundation-in-core / tiers-in-package split adds API friction | The package re-exports the foundation types so consumers see one cohesive API.  Documented in STDLIB.md. |

## Out of scope (deferred / non-goals)

- **Multi-writer / MVCC.**  Single-writer (the daemon) is the
  loft server pattern; concurrent writers belong in a future
  plan with its own design.
- **Distributed replication.**  Not needed for any current or
  near-term loft consumer.  If a multiplayer server ever
  needs cross-machine durability, that's its own arc.
- **Encryption-at-rest.**  Defer until a security-driven
  consumer surfaces.
- **Transactional semantics across multiple stores.**
  Per-store durability only; cross-store atomicity is a
  separate plan.

## Cross-references

- [`src/store.rs`](../../../../src/store.rs) — the existing
  Store primitive this plan extends.
- [`plans/future/37-tracker-index/`](../37-tracker-index/README.md)
  — the test-bed consumer (Tier 1).
- [`plans/future/37-tracker-index/08-multi-project-deploy.md`](../37-tracker-index/08-multi-project-deploy.md)
  — phase that opts into Tier 1.
- [`plans/future/32-tic-tac-toe/`](../32-tic-tac-toe/) — TTT v5
  multiplayer; Tier 2 consumer.
- [`plans/36-audience-generative-art/`](../36-audience-generative-art/)
  — audience demo; Tier 3 consumer.
- [`lib/server/src/server.loft`](../../../../lib/server/src/server.loft)
  — the server pattern these durability tiers complement.
- [DATABASE.md](../../../DATABASE.md) — Stores schema + DbRef
  semantics; durable stores live within this layer.
