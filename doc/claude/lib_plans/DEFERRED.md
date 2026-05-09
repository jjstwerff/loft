<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Deferred Library Work — Internal Index

Single source of truth for every parked library plan, deferred
library design decision, and "noted but not now" item specific
to libraries.  Distinct from
[`plans/DEFERRED.md`](../plans/DEFERRED.md) (core-language /
compiler / runtime parking) and `doc/claude/USER_FACING.md`
(user-visible-only filter across both).

**Convention.** Every row carries a `Trigger to unpause:` value —
the **concrete signal** that should re-activate the work.  No row
without a trigger.  When the signal arrives, the row moves out of
this file (into a current library plan, a P-issue fix, or a
release note).

**Closed-work hygiene** — see
[`plans/README.md § Companion indexes`](../plans/README.md#companion-indexes--every-parked-item-is-discoverable)
for the project-wide rule.  Short version: closed items are
removed entirely; their closure is recorded in git history,
regression tests, plan READMEs, PROBLEMS.md, and CHANGELOG.md.

**Discoverability.** The same grep targets cover both directories:

```bash
# Every parked item with its trigger (both plans/ and lib_plans/):
grep -r "Trigger to unpause:" doc/claude/

# Every parked test (the locked-in regression net):
cargo test --release -- --ignored 2>&1 | grep "^test " | head -50
```

---

## Deferred library plans (full plan parked)

| Plan | Status | Trigger to unpause |
|---|---|---|
| _(empty)_ |  |  |

## Deferred library plan-phase items (within a partly-shipped plan)

| Item | Plan / phase | Trigger to unpause |
|---|---|---|
| _(empty)_ |  |  |

## Deferred library API decisions (not bugs, but choices)

| Item | Context | Trigger to unpause |
|---|---|---|
| _(empty)_ |  |  |

## How rows leave this file

A row exits this file when one of these signals arrives:

- The trigger fires (e.g., user reports the deferred bug shape;
  the prerequisite P-issue closes; contributor appetite arrives).
- The row's plan moves out of `lib_plans/deferred/` into
  `lib_plans/` (current) or `lib_plans/future/`.
- The decision-pending item gets a verdict — moves into
  `DESIGN_DECISIONS.md` (closed-by-decision) or into a current
  library plan (accepted-and-scheduled).

When a row leaves, its closure is recorded in **one** appropriate
place per the closed-work hygiene rule (git history, plan README,
CHANGELOG, etc.) — not in this file.

## Cross-references

- [`plans/DEFERRED.md`](../plans/DEFERRED.md) — core-language
  / compiler / runtime parked work.
- [`../USER_FACING.md`](../USER_FACING.md) — user-visible
  subset of both DEFERRED files, with severity tiers.
- [`../PROBLEMS.md`](../PROBLEMS.md) — the cross-cutting
  P-issue tracker (bugs, regardless of layer).
