<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 04 — Stress test: `kill -9` × 1000 across all tiers

**Status:** Open

## Goal

Empirically validate the durability claims of all three
tiers under adversarial process termination.  Catches the
spec-vs-implementation gaps that synthetic happy-path tests
miss.

The bar: a 1000-iteration kill-injection harness that
spawns a daemon, kills it at random points, restarts it,
and asserts the per-tier durability contract.  Zero
violations across the run is the acceptance gate.

## What ships

### `tests/store_durable_kill.rs`

Three test functions, one per tier:

```rust
#[test]
#[ignore = "stress test — slow; run explicitly: cargo test --test store_durable_kill -- --ignored"]
fn tier1_integrity_only_recovery() {
    run_kill_iterations(DurabilityMode::IntegrityOnly { ... }, 1000, |records, recovered| {
        // Tier 1 contract: recovered EITHER matches the last clean-close state OR
        // the on_corruption callback fires and rebuilds (fresh state).
        // No assertion on ordering; only on integrity post-recovery.
        assert_no_torn_state(recovered);
    });
}

#[test]
#[ignore]
fn tier2_snapshot_every_recovery() {
    run_kill_iterations(DurabilityMode::SnapshotEvery { interval: 50ms, ... }, 1000, |records, recovered| {
        // Tier 2 contract: recovered is EITHER the last snapshot's state
        // OR the previous snapshot's state.  Never a torn state.
        // Number of writes lost: at most one snapshot interval's worth.
        assert_no_torn_state(recovered);
        assert!(recovered.len() <= records.len());
        assert!(records_lost(records, recovered) <= snapshot_window);
    });
}

#[test]
#[ignore]
fn tier3_wal_recovery() {
    run_kill_iterations(DurabilityMode::WAL { group_commit_window: 1ms, ... }, 1000, |records, recovered| {
        // Tier 3 contract: every write that returned successfully (sync mode)
        // OR was confirmed flushed (async mode + flush) is present.
        // No write that completed durably is lost.
        assert_no_torn_state(recovered);
        assert_all_committed_present(records, recovered);
    });
}
```

### Kill-injection harness — `run_kill_iterations`

```rust
fn run_kill_iterations<F>(mode: DurabilityMode, n: u32, validate: F)
where F: Fn(&[Record], &[Record])
{
    for i in 0..n {
        let path = tempfile::tempdir().unwrap();
        let store_path = path.join(format!("test_{i}.store"));

        // 1. Spawn the daemon as a subprocess.
        let mut daemon = spawn_daemon(&store_path, &mode);

        // 2. Send a sequence of writes; record what we sent.
        let records = send_writes(&mut daemon, random_count(50..500));

        // 3. Kill the daemon at a random point in the write stream.
        kill_at_random_offset(&mut daemon);

        // 4. Re-open the store; recover.
        let recovered = read_recovered_state(&store_path, &mode);

        // 5. Validate the per-tier contract.
        validate(&records, &recovered);
    }
}
```

The "daemon" is a tiny test binary (`tests/bin/durable_test_daemon.rs`)
that takes a tier mode + a write stream over stdin and
applies it to a Store.  Killing it tests the kernel's
behaviour with an in-flight mmap'd file, which is the
realistic failure mode.

### Per-OS coverage

Run the same suite under:

- Linux (CI default)
- macOS (CI matrix)
- Windows (manual; the Windows fsync semantics are
  significantly different — phase 04 documents the gap if
  any tier doesn't pass on Windows)

### Random fault injection

Beyond `kill -9`, inject:

- `SIGKILL` (immediate, no cleanup)
- `SIGTERM` (the daemon's drop runs — should give Tier 1
  a clean tail marker)
- Disk full (set a small `fallocate` quota on the temp
  dir, hit ENOSPC mid-write)
- File system unmount mid-write (rare; OS-dependent)

The test runner picks one at random per iteration; the
`assert_no_torn_state` assertion holds regardless.

### Performance baseline

Each test reports:

- Per-write latency p50, p95, p99
- Throughput (writes/sec)
- Snapshot time (Tier 2)
- Recovery time (per tier)
- Total disk bytes written per N writes (a proxy for
  write amplification)

These numbers go into `doc/claude/plans/future/
38-loft-store-durable/04-bench.txt` so future phases can
compare against the baseline.

## Critical files

| Path | Action |
|---|---|
| `tests/store_durable_kill.rs` | NEW: three #[ignore]'d tier tests |
| `tests/bin/durable_test_daemon.rs` | NEW: tiny daemon binary (~100 lines) |
| `Cargo.toml` | Already has `tempfile` in dev-deps (or add) |
| `doc/claude/plans/43-loft-store-durable/04-bench.txt` | NEW: baseline numbers per tier per OS |

## Existing functions / utilities to reuse

- All three tier implementations from phases 01-03.
- `tempfile` crate (used by other loft tests).
- `std::process::Command` for daemon spawn + kill.

## Test surface

The suite IS the test surface.  Each iteration:

1. Spawn fresh daemon.
2. Random write count + random kill point.
3. Recover.
4. Validate.

1000 iterations × 3 tiers = 3000 spawn-kill-validate
cycles per CI run (when --ignored flag passed).  Target:
< 5 minutes total wall-clock on a modern Linux CI runner.

## Acceptance

- `cargo test --test store_durable_kill -- --ignored`
  runs to completion in < 5 minutes on Linux.
- Zero per-tier-contract violations across all 3000
  iterations.
- Bench numbers (p50, p95, p99 latency; throughput;
  recovery time) recorded in `04-bench.txt`.
- Documented Windows-specific gaps (if any tier doesn't
  pass on Windows) — these become known caveats, NOT
  silent regressions.
- Suite added to `make ci-extended` target (separate from
  the default `make ci` because it's slow + opt-in).

## Risks

| Risk | Mitigation |
|---|---|
| Tests are non-deterministic (random kill point) — flaky CI? | Seed the RNG per iteration; record the seed in a failure to allow exact replay |
| 1000 iterations × 3 tiers = high CI cost | Phase 04 starts with --ignored (opt-in); promote to default CI only after the suite is stable |
| Windows fsync / ReadDirectoryChangesW semantics differ enough that a tier doesn't pass | Document as a per-OS caveat in CAVEATS.md; ship the Linux + macOS coverage as the official guarantee |
| `kill -9` doesn't reliably kill mid-write (kernel buffers the SIGKILL handling) | The test asserts post-kill recovery, not the timing of the kill — kernel behaviour is part of the test surface, not adversarial |
| Disk-full injection is fragile | If unreliable, drop it; the kill-injection alone covers the primary failure modes |

## Cross-references

- [Phases 01-03 — the tiers being validated](README.md#phases)
- [Phase 06 closeout](06-closeout.md) — bench numbers + caveats
  go into the closeout doc
