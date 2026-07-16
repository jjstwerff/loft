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

### C1 — null-flow N-Store discharge — **DONE (2026-07-16)**
Discharge each un-discharged nullable at its SOURCE with the lib's own `?? d` idiom
(`?? 0` int, `?? 0.0` float), NOT at every call site.  Watch two traps proven this
session: (a) never `??` a WRITE target (`x[i] = v` stays bare — an LHS `x[i] ?? 0 = v`
is a parse error); (b) don't `??` a value the f1 fix already proved fit (same-vector
`v[i]` in `for i in 0..len(v)`) — that trips a *redundant*-coalescing warning.  Gate:
each package's `loft test` shows 0 "is stored into" AND golden/round-trip tests still
green (the discharges must be behaviour-preserving — the null never fires).

**Final sweep: 0 own-source+test N-Store across every package.**  Landed one commit per
repo, on each repo's current branch (all pushed):

| unit | package(s) | fix | commit |
|---|---|---|---|
| C1a | graphics (1), gridmesh (2) | computed-index `?? 0`; parallel-vector `ys[i] ?? 0`; **modulo `v % cs` `?? 0` in chunk_loc** (was misreported at chunk_of) | loft-libs-graphics `16633a7` |
| C1b | hex_terrain (3 src + 1 test) | `sqrt(kv)` bound ONCE as `skv = sqrt(kv) ?? 0.0` (reused ×3); test RHS `?? 0.0` (can't `??` the `th[i]` write target) | loft-libs-world `fe917c0` |
| C1c | random (1) | `get()` was `-> integer` but documents+executes `return null` → typed `-> integer?` (honesty fix, not `?? d`; internal `indices()` already `?? 0`) | loft-libs-core `7887a3b` |
| C1d | web (1) | `parts[len-1]` computed-index → `(… ?? "").trim()` | loft-libs-net `3c3fc90` |

**C0 triage (random/regex):** confirmed KNOWN-ENVIRONMENTAL — the failures are the
stale-cdylib bridge (`n_rand_seed has no marshal bridge — rebuild against bridge-capable
loft-ffi`), not a logic break; the pure-loft tests pass.  No fix needed here (a clean CI
rebuilds the cdylib).

**Loft finding (diagnostic bug):** the null-flow **return-value** N-Store reports the
position of the FOLLOWING function definition, not the offending return expression —
`chunk_loc`'s nullable tail was reported at `chunk_of:172`.  Cost ~15 probes to locate.
Worth a diagnostic-position fix in loft's null-flow check (bisect-by-tail-discharge is
the reliable workaround until then).

### C2 — `not null` retirement — **DONE (2026-07-16)**
`not null` is a deprecated no-op (fields are non-null by default; @PLN25 F2).  Pure
DELETE — `perl -i -pe 's/ not null//g'` per source after verifying every occurrence is a
field decl (all were — incl. enum-variant fields and `limit(0,255) not null` colour
fields; none in comments/strings).  Semantics unchanged; `loft test` green per package.

**Final sweep: 0 own-source `not null` (and 0 N-Store) across every sibling package.**
The source counts came in below the work-list numbers (those were files×passes inflated):

| unit | repo | pkgs (source sites) | commit |
|---|---|---|---|
| C2a | loft-libs-world | hex_terrain (50), hex_world (4) | loft-libs-world `d2101ad` |
| C2b | loft-libs-graphics | gridmesh (7), shapes (9), imaging (3) | loft-libs-graphics `5d09bb7` |
| C2c | loft-libs-core | crypto (2), cbor (1), random (2) | loft-libs-core `c8ae4d5` |
| C2d | loft-libs-net | web (2) | loft-libs-net `bc9558e` |

**Registry note:** in-repo `lib/audience_crystal` still shows `not null` (+ `&`) warnings
from its `gridmesh-0.1.2` REGISTRY dependency (a stale copy predating the sibling-repo
fix).  Those clear only when the fixed graphics libs are re-published (loft-ship skill,
touch-gated) — a release step, not a source edit.

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
