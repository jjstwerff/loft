<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN118 arc C — the differential leak oracle (built AFTER the fact, the lesson learned)

The arc-F session flailed because this was never built (see
[`../cluster-fold-reads-null.md`](../cluster-fold-reads-null.md) § Method retrospective).
It is here now, and it is what found the fix in one pass the second time round.

## The one idea

**Native is the clean reference.** Whole-`--native` compiles the script AND every
library into a single binary — there is **no interp↔cdylib shared-store bridge** — so
a bridge-boundary leak cannot occur there. Therefore:

> **interp-leaked-stores − native-leaked-stores = the bug**, attributed by allocation
> site, with zero inference.

One run gives the boundary that a dozen contradictory ad-hoc traces did not.

## Files

| File | What |
|---|---|
| `leak-oracle.sh` | run ONE probe both backends, print the differential verdict (exit 0 = no interp leak, 2 = interp-only leak = the bug, 3 = leaks on both = not this class) |
| `run-matrix.sh` | drive the probe matrix through the oracle, each cell checked against its hand-computed verdict |

## Run

```bash
# one probe
LOFT_BIN=target/release/loft \
  doc/claude/plans/118-.../oracle/leak-oracle.sh \
  doc/claude/plans/118-.../probes/arcF-min-nested-struct.loft \
  --lib doc/claude/plans/118-.../probes/lib/

# the whole matrix (negative control + fix + FLIP positive control)
LOFT_BIN=target/release/loft doc/claude/plans/118-.../oracle/run-matrix.sh
```

## Why it can't lie (the controls)

- **Negative control** — `arcF-control-direct-struct.loft` (a *direct* struct-literal
  return through the same bridge) must stay clean: proves the trigger is specifically
  the **nested-call return**, not any shared struct return.
- **Positive control** — the same nested probe with `--flip`
  (`LOFT_NO_BRIDGE_ORPHAN_FREE=1`, the arc-D switch) **must** resurrect the interp-only
  leak. If it doesn't, the oracle is vacuous — every "clean" verdict is meaningless.

`--interpret` is passed explicitly so the verdict never depends on the box's default
backend (this dev box defaults to `--native`, which has no bridge and would make every
cell falsely clean).

## The permanent guard

The CI-level version of this oracle is the Rust test
`tests/n3_parity.rs::shared_bridge_nested_return_no_orphan_leak` — same probe, same
positive control, run on every `cargo test`.
