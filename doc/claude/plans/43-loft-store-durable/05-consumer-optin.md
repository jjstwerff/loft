<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 05 — First-consumer opt-in

**Status:** Open

## Goal

Wire the first three consumers to the new durability tiers,
proving the API works in production:

- **Plan-37 indexer** (Tier 1) — opts in via phase-08 daemon
  startup.
- **TTT v5 multiplayer server** (Tier 2) — design doc
  updated; implementation lands when TTT v5 ships its
  persistent-session arc.
- **Plan-36 audience-generative-art demo** (Tier 3) —
  same: design doc updated; implementation lands when
  the demo ships.

This phase is about wiring; the new code lives mostly in
the consumer plans, not here.  Phase 05 ships the proof-of-
production check + the design-doc updates that lock the
contracts in.

## What ships

### Plan-37 indexer (Tier 1) — actual wiring

In `tools/indexer/scan.loft` (the loft daemon from
@PLN42 phase 07):

```loft
use store_durable;

fn main() {
    config = load_config(".tracker/config.toml");
    store = store_durable::open(
        ".tracker/state/tags.store",
        DurabilityMode.IntegrityOnly,
        on_corruption_fn,   // → full_rescan(config)
    );
    daemon_loop(store, config);
}

fn on_corruption_fn(path: text) {
    println("indexer: tags.store corrupt at {path}; rebuilding from filesystem");
    full_rescan_to(path);
}
```

The full-rescan path already exists from @PLN42 phase
00 (the bash scanner does it; the loft daemon mirrors
the logic).  Tier 1 just plumbs the callback.

Verified end-to-end:

1. Start the indexer daemon.
2. Wait for index to populate.
3. `kill -9 <daemon-pid>` mid-write.
4. Restart the daemon.
5. The integrity check fires; rebuild callback runs;
   index is back in < 2 sec.

### TTT v5 design doc update (Tier 2)

`plans/39-tic-tac-toe/README.md` (or the v5-specific
phase doc) gains a § "Persistence" subsection:

```markdown
## Persistence (Tier 2 via plan-38)

TTT v5 holds per-session state: in-flight games, move
history, the catch-up cache for reconnecting clients.
Lost session state means players see "your game
disappeared" on reconnect — survivable but degrading.

Persistence: `Store::open_durable("ttt/sessions.store",
DurabilityMode::SnapshotEvery(Duration::from_secs(5))) `
via plan-38's lib/store_durable.  Bounded loss = 5 sec
of moves; the t4 catch-up protocol fills the gap when a
player reconnects.

Implementation lands with the v5-server arc; design
contract locked here.
```

### Plan-36 design doc update (Tier 3)

`plans/6-audience-generative-art/README.md`
gains a § "Persistence" subsection:

```markdown
## Persistence (Tier 3 via plan-38)

The audience demo accumulates contributions in real
time: drawings, color votes, decay timers.  An OS kill
mid-show that loses contributions in front of the
audience is the worst failure mode.

Persistence: `Store::open_durable("demo/world.store",
DurabilityMode::WAL { wal_path: "demo/world.wal",
snapshot_every_n_writes: 500, group_commit_window:
Duration::from_millis(2), .. })`.  Every contribution
fsync'd before the projector acknowledges; rolling
checkpoints keep the WAL bounded.

Acceptance: kill -9 the projector mid-show; restart;
zero contributions lost.  Phase 04 stress test from
plan-38 validates this contract empirically.
```

### Smoke test — `tests/store_durable_smoke.rs`

A tiny end-to-end test that exercises each tier's
consumer-shaped use:

- Open + write + clean close + reopen → all writes
  present (Tier 1).
- Open + write 100 + sleep > snapshot_interval + crash
  via panic + reopen → most writes present, none torn
  (Tier 2).
- Open + sync_write 100 + crash via panic + reopen →
  all 100 present (Tier 3).

This is fast (<1 sec); ships in `make ci`.

## Critical files

| Path | Action |
|---|---|
| `tools/indexer/scan.loft` (@PLN42 phase 07/08) | EXTEND: open store via `IntegrityOnly` |
| `plans/39-tic-tac-toe/README.md` | ADD § Persistence (Tier 2) |
| `plans/6-audience-generative-art/README.md` | ADD § Persistence (Tier 3) |
| `tests/store_durable_smoke.rs` | NEW: per-tier consumer-shaped end-to-end smoke |

## Existing functions / utilities to reuse

- All three tier APIs from phases 01-03.
- The full-rescan logic from @PLN42 phases 00 + 07.
- `lib/store_durable/` package from phases 02 + 03.

## Test surface

`tests/store_durable_smoke.rs`:

- Tier 1 round-trip (clean close → reopen → all data
  present).
- Tier 1 corruption fallback (delete the tail marker;
  on_corruption fires; rebuilt store opens clean).
- Tier 2 snapshot timing (write × 100, sleep, panic,
  reopen; verify ≤ 1 snapshot interval lost).
- Tier 3 sync write (write × 100 sync, panic, reopen;
  verify all 100 present).

## Acceptance

- `cargo test --test store_durable_smoke` passes.
- Indexer (@PLN42 phase 07/08) demonstrably uses Tier 1
  in production: a deliberate `kill -9` of the daemon +
  restart triggers the rebuild path.
- TTT v5 + @PLN6 design docs cite @PLN43 tiers + the
  acceptance contract.
- Cross-link from this plan's README to each consumer.
- No regression: existing non-durable Store usage (not
  going through `open_durable`) unchanged.

## Risks

| Risk | Mitigation |
|---|---|
| Indexer's Tier 1 callback (full rescan) is slow on huge trees | Phase 04 stress baseline measures rescan time per repo size; phase 06 closeout documents the floor |
| TTT v5 / @PLN6 not yet shipping their persistence arcs → design-doc updates feel premature | The contracts are still useful: when v5 / @PLN6 implementation begins, the API is already there + tested.  No surprise dependency. |
| Consumer wires Tier 1 to data that needs Tier 3 | STDLIB.md (phase 06) has a clear "when to pick which tier" table; named modes make the choice visible at the call site |

## Cross-references

- [Phases 01-03 — the tier APIs](README.md#phases)
- [Phase 04 — stress test](04-stress-test.md) — the empirical contract this phase wires consumers into
- [Phase 06 — closeout](06-closeout.md)
- [Plan-37 phase 08](../42-tracker-index/08-multi-project-deploy.md) — Tier 1 consumer
- [Plan-32 TTT v5](../39-tic-tac-toe) — Tier 2 consumer
- [Plan-36 audience demo](../6-audience-generative-art) — Tier 3 consumer
