<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 03 — Tier 3: WAL + grouped commit

**Status:** Open

## Goal

Ship `DurabilityMode::WAL { wal_path, snapshot_every_n_writes,
group_commit_window, on_corruption }`: write-ahead log with
fsync per record (or per group), periodic checkpoints, WAL
truncation.  Zero-data-loss for committed writes.

Tier 3 is the right choice for any data where each write
matters and rebuild from another source is impossible.
First consumer: @PLN6 audience-generative-art demo (every
audience contribution must persist; on stage, a crashed
projector recovering with lost contributions is the worst-
possible failure mode).

## What ships

### `DurabilityMode::WAL` variant

```rust
pub enum DurabilityMode {
    IntegrityOnly { /* phase 01 */ },
    SnapshotEvery { /* phase 02 */ },

    WAL {
        wal_path: PathBuf,
        snapshot_every_n_writes: u32,
        group_commit_window: Duration,
        on_corruption: Box<dyn Fn(&Path) -> io::Result<()>>,
    },
}
```

### WAL record format

```
Offset  Size  Field
------  ----  -----
0       8     seq (monotonic; last_committed_seq + 1)
8       4     record_len (bytes of payload)
12      ...   payload (the write — DbRef + bytes)
12+plen 4     record_crc (CRC32 of bytes 0..12+plen)
```

Records are append-only; the WAL grows until the next
checkpoint truncates it.

### Write path

```rust
fn write(&mut self, ...) {
    let record = encode_write(...);
    self.pending_records.push(record);

    if self.last_fsync.elapsed() >= self.group_commit_window
       || self.pending_records.len() >= MAX_BATCH
    {
        self.flush_group();
    }

    self.apply_in_memory(...);
    self.writes_since_snapshot += 1;
    if self.writes_since_snapshot >= self.snapshot_every_n_writes {
        self.checkpoint();
    }
}

fn flush_group(&mut self) {
    let pending = std::mem::take(&mut self.pending_records);
    for record in &pending {
        self.wal_file.write_all(record)?;
    }
    self.wal_file.sync_data()?;     // ← THE durability barrier
    self.last_fsync = Instant::now();
    // Notify any blocked callers — their writes are now durable.
    for waiter in self.commit_waiters.drain(..) { waiter.notify(); }
}
```

The `group_commit_window` lets multiple concurrent writers
share one fsync.  Per-write latency = max(time to fill
window, fsync duration).  On modern SSD with batching:
0.1-1 ms per write.

### Checkpoint cycle

When `writes_since_snapshot >= N`:

1. Take a Tier-2-style atomic snapshot (reuse phase 02
   machinery): write the current in-memory state to the
   inactive slot, fsync, atomic-rename the checkpoint
   pointer.
2. Note the seq of the last write included in the snapshot.
3. Open a NEW wal file (`wal.001`, `wal.002`, ...);
   atomic-rename to `wal.current`.
4. The OLD wal file is no longer needed — delete it.

Recovery:
1. Load the snapshot (Tier 2 path).
2. Open `wal.current`; replay every record with
   `seq > snapshot_last_seq`.
3. For each record: validate `record_crc`; if mismatch,
   the WAL was killed mid-write — stop replaying at the
   last valid record.  Anything after that is lost.

### Consumer-facing semantics

Two write modes the consumer can choose between (a flag on
the write call, not on `DurabilityMode`):

- **Async** (default): write returns immediately; the
  caller doesn't know when durability lands.  Use for
  bulk writes where throughput matters more than per-
  write latency.
- **Sync**: write blocks until the next group commit
  completes.  Use for "must-acknowledge-before-replying-to-
  client" semantics (e.g., game server confirming a move).

The `group_commit_window` makes Sync mode tolerable: a
busy server amortises fsync cost across the burst.

## Critical files

| Path | Action |
|---|---|
| `src/store.rs` | EXTEND DurabilityMode + open_durable arm for WAL |
| `lib/store_durable/src/store_durable.loft` | EXTEND public API: open_wal, write_sync, write_async |
| `lib/store_durable/native/src/lib.rs` | EXTEND: WAL append + group commit + checkpoint truncation |
| `tests/store_durable_tier3.rs` | NEW: write-recovery cycles, group commit, kill-resume semantics |

## Existing functions / utilities to reuse

- Phase 01's open_durable skeleton.
- Phase 02's atomic snapshot path (Tier 3 reuses it for
  checkpoints; only difference is the WAL truncation
  step at the end).
- `lib/server`'s host-bridge native pattern.
- `crc32c` from phase 00 for record CRCs.

## Test surface

`tests/store_durable_tier3.rs`:

- Open WAL store; write 100 records; verify all 100
  present after restart.
- Write 100 records; kill -9 mid-write; restart;
  verify all WRITTEN-AND-FSYNCD records present (count
  may be < 100; partially-written WAL records discarded).
- Sync write returns only after fsync: measure latency
  vs group_commit_window.
- Async write returns immediately; later `flush()` makes
  durable.
- Checkpoint at N=50 writes; truncate WAL; verify wal
  file is rotated; recovery still works.
- Concurrent writers (simulated): N writers issuing
  writes; verify all writes are committed in some order;
  no losses.
- Stress: 100k writes with kill -9 at random points;
  recovery loses NO write that returned successfully to
  caller.

## Acceptance

- `cargo test --test store_durable_tier3` passes.
- Per-write latency on SSD (sync mode, 100µs group window):
  < 1 ms (single writer); < 200 µs amortised (10 writers).
- Throughput on SSD (async mode, 5ms group window): >
  10k writes/sec.
- 100k stress test (kill -9 at random) produces zero
  durability violations.
- Plan-36 audience demo design doc updated to reference
  Tier 3 as its persistence layer (the consumer hookup
  ships in phase 05).

## Risks

| Risk | Mitigation |
|---|---|
| fsync latency variance kills tail latency for sync writers | Group commit smooths the tail; document expected p99 latency floors per disk type |
| WAL grows unbounded if checkpoint fails | Phase 04 stress test includes "checkpoint failure" injection; recovery path handles partial-checkpoint state |
| Recovery time scales with WAL size | Checkpoint cadence = trade-off knob; default (every 1000 writes) keeps WAL small |
| Concurrent writers race on the WAL file | Single-writer assumption holds; if multi-writer ever surfaces, it's its own plan |
| Replay finds a record_crc mismatch in the MIDDLE of the WAL | Document: replay stops at the first bad record; everything after is lost.  This is the WAL contract.  Single corrupted record = bounded data loss. |

## Cross-references

- [Phase 02 — Tier 2 snapshots](02-tier-2-snapshots.md) — checkpoint mechanism reused here
- [Phase 04 — stress test](04-stress-test.md) — validates Tier 3 zero-loss claim
- [Plan-36 audience demo](../6-audience-generative-art) — first Tier 3 consumer
