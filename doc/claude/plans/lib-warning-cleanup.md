<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library warning-cleanup — work list toward the 1.0 "empty tolerated list" goal

**Feeds @PLN102** (drive the `tests/testing.rs` tolerated-warnings list to empty for
1.0 — no warning class quietly tolerated).  This is the LIBRARY side: every installable
package must compile with **zero** warnings.  Data below is a full sweep on the current
loft (`tuxedo-work`), deduped by site (files × passes inflate raw counts).

## Sweep — warnings per package, by class

| repo / package | null-flow (N-Store) | `not null` | `&` advisory | test | note |
|---|---|---|---|---|---|
| **loft2/lib** audience_crystal | 0 ✓ | 0 | 0 | ok | DONE (this session) |
| loft2/lib engine_host | 0 | 0 | 0 | — | native-in-binary; clean |
| **loft-libs-graphics** graphics | 1 | 0 | 0 | ok | |
| gridmesh | 1 | 12 | 7 | ok | arg fix uncommitted; 1 return/field left |
| imaging | 0 | 3 | 0 | ok | |
| shapes | 0 | 9 | 0 | ok | |
| **loft-libs-world** hex_terrain | 3 | 50 | 0 | ok | biggest `not null` block |
| hex_world | 0 | 4 | 0 | ok | |
| hex_grid | 0 | 0 | 0 | ok | clean |
| **loft-libs-core** cbor | 0 | 1 | 0 | ok | |
| crypto | 0 | 2 | 0 | ok | |
| arguments | 0 | 0 | 0 | ok | clean (0.2.0 rewrite) |
| random | 1 | 2 | 0 | **FAILED** | test fail — triage (env cdylib?) first |
| regex | 0 | 0 | 0 | **FAILED** | test fail — triage first |
| **loft-libs-net** game_protocol / server / ssh | 0 | 0 | 0 | ok | clean |
| web | 1 | 2 | 0 | ok | |
| **routing** routing_kernel / basemap / map_kernel / imaging / server / web | 0 | 0 | 0 | ok | consumer already discharged (their 26→0) |

**Totals:** null-flow ≈ **7 sites / 5 pkgs**; `not null` ≈ **85 sites / 9 pkgs**;
`&` ≈ **7 sites / 1 pkg** (gridmesh).

## Ownership

- **loft2/lib** (audience_crystal, engine_host) — this stream owns; lands in loft2.
- **loft-libs-{graphics,world,core,net}** — the lib repos this stream maintains for
  cross-target/compat; land on each repo's current branch.
- **routing/lib** — the CONSUMER agent owns; already at 0 (leave to them; re-check after
  any language change that moves the null-flow surface).

## Compartments (each = one focused, PR-able unit)

Ordered: null-flow first (correctness-adjacent, tiny), then the mechanical `not null`
sweep, then the judgment-call `&` review.

### C1 — null-flow N-Store discharge (≈7 sites) — do first, small
Discharge each un-discharged nullable at its SOURCE with the lib's own `?? d` idiom
(`?? 0` int, `?? 0.0` float), NOT at every call site.  Watch two traps proven this
session: (a) never `??` a WRITE target (`x[i] = v` stays bare — an LHS `x[i] ?? 0 = v`
is a parse error); (b) don't `??` a value the f1 fix already proved fit (same-vector
`v[i]` in `for i in 0..len(v)`) — that trips a *redundant*-coalescing warning.  Gate:
each package's `loft test` shows 0 "is stored into" AND golden/round-trip tests still
green (the discharges must be behaviour-preserving — the null never fires).

| unit | package(s) | sites |
|---|---|---|
| C1a | loft-libs-graphics: graphics (1), gridmesh (1 — arg already fixed, uncommitted) | 2 |
| C1b | loft-libs-world: hex_terrain (3) | 3 |
| C1c | loft-libs-core: random (1 — after its test triage) | 1 |
| C1d | loft-libs-net: web (1) | 1 |

### C2 — `not null` retirement (≈85 sites) — mechanical, per repo
`not null` is a deprecated no-op (fields are non-null by default; @PLN25 F2).  Pure
DELETE — strip ` not null` from each field decl.  A field that SHOULD allow null is a
separate judgment (write `T?`), but the sweep shows these are all on already-non-null
fields, so a scoped per-file `s/ not null//` + a `loft test` re-run per package is
enough.  One PR per repo keeps the diff reviewable.

| unit | repo | pkgs (sites) |
|---|---|---|
| C2a | loft-libs-world | hex_terrain (50), hex_world (4) |
| C2b | loft-libs-graphics | gridmesh (12), shapes (9), imaging (3) |
| C2c | loft-libs-core | crypto (2), cbor (1), random (2) |
| C2d | loft-libs-net | web (2) |

### C3 — `&`-parameter advisory (7 sites, gridmesh only) — JUDGMENT, not mechanical
"`&` only slows it down — drop unless you REASSIGN the whole binding."  NOT a blind
delete: a `&` that write-backs (`p = …`, or field/element mutation the caller must see)
must stay; only a read-only `&` param is droppable.  Review each of gridmesh's 7 against
"is the param reassigned?" — drop the read-only ones, keep the write-back ones (and if a
kept one still warns, that's a lint false-positive to file, not a lib fix).

### C0 — test-failure triage (blocks C1c/C2c) — do before touching random/regex
`random` and `regex` FAIL their suites on current loft.  Confirm env (cdylib
`libloft.rlib`/no-network registry — the known-environmental class) vs a real break with
`--no-fail-fast`; only then discharge/retire their warnings.

## When this is done

Every non-consumer lib compiles warning-free → the N-Store / `not null` / `&` entries can
leave the `tests/testing.rs` tolerated list (the @PLN102 1.0 gate), because no fixture
trips them anymore.  Re-sweep after any language change that shifts the null-flow surface
(a new f-finding), since that can re-open C1.
