<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 06 — Closeout: docs, decision record, finished/

**Status:** Open

## Goal

Lock in the durability tiers as a stable, documented API
surface; capture the design choices in
DESIGN_DECISIONS.md so future contributors don't relitigate
them; move the plan to `finished/`.

## What ships

### `DATABASE.md` § Durable stores

New section under the existing § Stores layer:

```markdown
## Durable stores (plan-38)

`Store::open_durable(path, mode)` opens a Store with one of
three opt-in durability tiers.  Each tier trades cost
against the data-loss appetite of the consumer.

| Mode | Cost | Worst-case loss on OS kill | Pick when |
|---|---|---|---|
| `IntegrityOnly` | One msync at clean close | Page-cache contents (seconds) | Data is derivable from another source |
| `SnapshotEvery(interval)` | Periodic memcpy + msync + atomic rename | One snapshot interval | Bounded loss is acceptable; full reconstruction is expensive |
| `WAL { window, .. }` | One fsync per group commit | Zero for committed writes | Each write is irreplaceable |

### Format on disk

(insert the durable-store on-disk layout from phase 00)

### Recovery contract per tier

(per-tier semantics + recovery cost; cite the stress-test
results from `04-bench.txt`)
```

### `STDLIB.md` § `Store::open_durable`

API doc with copyable examples for each tier.  Cross-
references to plan-38's per-phase docs for design context.

### `DESIGN_DECISIONS.md` — new C-row

A new closed-by-decision entry recording the choices the
plan made + why:

```markdown
## C-NN — Tiered durability for mmap stores (plan-38, 2026-XX)

Three opt-in tiers (IntegrityOnly, SnapshotEvery, WAL)
instead of:

- A single always-on durability layer (rejected — too
  expensive for derived data like the indexer)
- A separate database product (rejected — loft's Store
  is already mmap-backed; the layer of abstraction
  belongs in core)
- WAL-only with adjustable batching (rejected — the
  IntegrityOnly tier serves derived-data consumers
  better with no fsync overhead)

The tiered approach lets each consumer pick the cost that
matches its data's value.  Indexer (cheap derived) →
Tier 1.  Game session state (turn-based, bounded loss
acceptable) → Tier 2.  Audience-demo contributions
(every write irreplaceable) → Tier 3.

No transactional / MVCC / multi-writer semantics —
single-writer assumption matches the loft server pattern
(`lib/server`'s accept loop).  Future plans can add MVCC
if it surfaces as a real need.
```

### `CHANGELOG_TECHNICAL.md` retrospective

Per the plan-22 / plan-15 closeout pattern:

```markdown
### Plan-38 (loft-store-durable) closed YYYY-MM-DD

Plan-38 ran from <start-date> through <close-date>.
Goal: opt-in durability tiers for loft mmap stores.

Per-phase summary:
- 00 Foundation — DStoreV1 signature + per-record CRC
  + tail marker.  No behavior change for legacy stores.
- 01 Tier 1 IntegrityOnly — open_durable API + auto-
  rescan callback.  Indexer adopts.
- 02 Tier 2 SnapshotEvery — lib/store_durable package
  + double-buffered atomic snapshots + msync discipline.
- 03 Tier 3 WAL — append + fsync + grouped commit +
  checkpoint + WAL truncation.
- 04 Stress test — 3000-iteration kill-injection harness
  validates per-tier recovery contracts.  Bench
  baseline in 04-bench.txt.
- 05 Consumer opt-in — indexer (Tier 1) wired; TTT v5
  + plan-36 audience demo design docs reference Tier 2
  / Tier 3.

Bug yield: <P-issues filed during the plan, if any>.

Active plans remaining after close: <N>.
Plan moved to `plans/finished/38-loft-store-durable/`.
```

### `ROADMAP.md` updates

- Remove plan-38 from active section.
- Add to closed section pointing at `finished/38-…`.

### `CAVEATS.md` (if any tier has OS-specific gaps)

If phase 04's stress test surfaced Windows-specific
limitations, document them as caveats with reproducers +
workarounds.  No-op if the tiers all pass cleanly on all
OSes.

### Move to `finished/`

```bash
git mv doc/claude/plans/future/38-loft-store-durable \
       doc/claude/plans/finished/38-loft-store-durable
```

Update intra-plan + sibling-plan link paths per the
plan-22 closeout precedent.  This plan started in
`future/`, so the move is from `future/38-` to
`finished/38-`.

## Critical files

| Path | Action |
|---|---|
| `doc/claude/DATABASE.md` | ADD § Durable stores |
| `doc/claude/STDLIB.md` | ADD § `Store::open_durable` |
| `doc/claude/DESIGN_DECISIONS.md` | ADD C-NN row for tiered durability |
| `doc/claude/CHANGELOG_TECHNICAL.md` | ADD plan-38 retrospective |
| `doc/claude/ROADMAP.md` | REMOVE plan-38 from active; ADD to closed |
| `doc/claude/CAVEATS.md` | ADD per-OS caveats if needed |
| `doc/claude/plans/future/38-loft-store-durable/` | `git mv` to `plans/finished/38-loft-store-durable/` |

## Acceptance

- All 5 prior phases (00-05) shipped + green.
- Documentation updates land.
- Plan dir is at `plans/finished/38-loft-store-durable/`.
- Sibling plans (37, 32, 36) link paths repaired.
- `bash scripts/check_doc_drift.sh` (or
  `cargo test --test doc_hygiene`) clean.
- No active P-issues stem from this plan (any open
  follow-up work is filed against PROBLEMS.md before
  closeout).

## Risks

| Risk | Mitigation |
|---|---|
| Closeout lands while consumers (TTT v5, plan-36) haven't actually shipped their persistent-state arcs | Closeout DOES NOT depend on consumer implementation — the API + tests + design contracts are enough.  Consumer integration is verified by the smoke test in phase 05. |
| Documentation drift between this plan's docs and the consumer plans | Cross-reference paths checked by doc_hygiene.  If TTT v5's design doc cites a Tier 2 API surface that drifts, the link breaks at PR time. |
| C-NN decision number collides with another in-flight plan | Pick the next free number at closeout time; DESIGN_DECISIONS.md has a chronological order, not strict numbering |

## Cross-references

- [README § Acceptance](README.md#acceptance--full-plan)
- [Phase 22 closeout pattern](../../finished/22-mutable-closures/06-closeout.md) — template
- [DATABASE.md](../../../DATABASE.md) — destination for the new section
- [STDLIB.md](../../../STDLIB.md) — destination for the API doc
- [DESIGN_DECISIONS.md](../../../DESIGN_DECISIONS.md) — destination for the closed-decision row
