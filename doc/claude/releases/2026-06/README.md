<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `2026-06` — first monthly cycle, and the switch to calendar versioning

> The record of ONE release cycle — its blockers, the evidence each gate produced, and the
> decisions taken.  The process every cycle follows lives in
> [RELEASE.md](../../RELEASE.md); the index of cycles in [releases/README.md](../README.md).

The monthly cadence is **adopted starting `2026-06`**, shipped **mid-month**
(2026-06-14) as a one-time exception to the "ships at the start of the month"
rule — the tree is stable and the library work is ready.  `2026-07` (branch
exists) then rebases onto the new `main` tip and resumes as the next cycle,
shipping at the start of July as normal.

This is also the **switch from semver `0.8.x` to calendar versioning**: the
release is named for its month (`2026-06`), which `Cargo.toml` spells
`2026.6.0` (year.month.patch — cargo needs three numeric parts with no leading
zeros).  Each month bumps the month digit (`2026.7.0`, …, `2026.12.0`,
`2027.1.0`); the patch slot is reserved for in-month security fixes
(`2026.6.1`).  Existing library `loft = ">=0.8"` constraints are still
satisfied by `2026.6.0`.

**Scope (frozen):** what is on `main` + the `../loft2` flat-namespace break +
bug fixes only — no other new features.
