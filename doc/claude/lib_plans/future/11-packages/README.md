<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Packages — open follow-ups

The **package format itself is shipped**: `loft.toml` manifests
work today, and 6 `lib/*` packages already use the format
(`lib/server`, `lib/arguments`, `lib/moros_*`, etc.).
Architecture, layout, manifest schema, function binding model,
build pipeline, target matrix, OpenGL case study, and security
model all live as reference in
[`../../../PACKAGES.md`](../../../PACKAGES.md).

This plan tracks the **open follow-up items** still to be
built — central registry, lock file, library extraction.
Each item points at the relevant PACKAGES.md section + the
ROADMAP row that schedules it.

## Status

| Item | ROADMAP row | PACKAGES.md section | Status |
|---|---|---|---|
| **PKG.REG** — central package registry MVP (`loft install <name>`) | ROADMAP row, 0.8.6 | [`../../../PACKAGES.md` § Package Registry](../../../PACKAGES.md) (line 704+) | Open — designed, scheduled.  Largest sub-arc.  Includes `loft install`, `loft publish`, central registry server, package signing / verification, `manifest.toml` index format. |
| **PKG.7** — lock file (`loft.lock`) for reproducible builds | ROADMAP row, 0.8.6 | [`../../../PACKAGES.md` § Implementation phases](../../../PACKAGES.md) | Open — small.  Cite manifest.rs in ROADMAP for current implementation surface. |
| **PKG.EXTRACT** — move `lib/*/` out into per-family GitHub repos via PKG.REG | ROADMAP row, 1.1+ | [`../../../PACKAGES.md` § Implementation phases](../../../PACKAGES.md) | Open — large.  Depends on PKG.REG landing first.  Splits the monorepo into per-family external repos consumable via the registry. |

## Why these items are here, not in PACKAGES.md

PACKAGES.md is reference documentation — it describes how
the package format works today (`loft.toml` schema, package
layout, function binding model, build pipeline, target matrix
across native / interpreter / WASM, security model design).
Anyone reading or modifying package handling reads PACKAGES.md.

The open follow-ups (registry / install / lock file /
extraction) don't fit that purpose: they're items to BUILD,
not architecture to understand.  Per the docs-vs-plans rule
(major dev → plans path; bugs → direct), they belong in
`lib_plans/future/`.  Keeping them visible in the
`lib_plans/future/` index ensures they don't get lost as
PACKAGES.md grows or gets re-organized.

The pointer-plan shape (this README references PACKAGES.md
sections rather than copying their content) avoids
duplication — design details stay in one place.  When an
item ships, the work in PACKAGES.md gets trimmed (or moved
into the proper "how things work" section per the closure
rule) and this plan's row moves to a closure record.

## Phase ordering

Suggested sequence when this plan unpauses:

1. **PKG.7 lock file** — smallest, contained in `manifest.rs`.
   Lands quickly; gives reproducible builds before registry
   work starts.
2. **PKG.REG registry MVP** — bulk of the work.  Phases:
   (a) `manifest.toml` index format spec
   (b) central registry server (could be GitHub Pages +
       static index for MVP)
   (c) `loft install <name>` CLI command
   (d) `loft publish` CLI command
   (e) package signing / verification
3. **PKG.EXTRACT** — only after PKG.REG is solid.  Splits
   `lib/*/` packages out into per-family GitHub repos, each
   consumable via the registry.

## See also

- [`../../../PACKAGES.md`](../../../PACKAGES.md) — full package
  format reference (design + how-it-works-today; lib/* packages
  already use this format)
- Existing `lib/*/loft.toml` files — concrete examples of
  the shipped format
- [`../07-web-ide/`](../07-web-ide/) — Web IDE depends on the
  `cdylib` WASM target which PKG.EXTRACT's R1 workspace split
  enables
- [`../01-regex/`](../01-regex/) — REGEX library will use
  the same package format once stdlib lazy-loading lands
