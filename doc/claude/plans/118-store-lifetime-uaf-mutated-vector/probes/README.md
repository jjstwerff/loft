<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN118 probes — flat table

Stage-A probes for the store-lifetime UAF. Each is a throwaway `/tmp`-style `.loft` graduated
here once it earns a row. Rules (CLAUDE.md § Debugging policy · the `engineering-rigor` skill):
one composition axis per probe · distinctive values · hand-computed expected cell · assert
**value AND length AND leak** · run on **both** backends (`--interpret` + `--native`) · a
no-output cell is vacuous. Be liberal — a missed shape is the worst failure; curate at the end
of Stage A. Extract at least one probe from the **real moros path**, not only synthetic.

| # | file | axis varied | expected | interp | native | leak | cluster |
|---|---|---|---|---|---|---|---|
| F1 | `arcF-min-leak-hex_to_world.loft` | shared (`StaticCall`) vs local callee | value ok both; leak interp-only | ok | ok | **299× Vec3 LEAK** | arc F |
| F2 | `arcF-control-local-nested-noleak.loft` | control: LOCAL nested-call return | value ok; no leak | ok | ok | none | arc F |

**Arc F (the unmasked interpreter-only leak).** F1 is the minimal repro — `c = hex_to_world(n,0,0)` in a
loop (`use moros_render`); `hex_to_world` is a SHARED/installed function returning `vec3(...)` via retbuf.
Run with `--path <loft>/ --lib <moros>/lib/` (+ `LOFT_LEAK_SITES=1`). F2 is the CONTROL: a byte-identical
shape with a LOCAL callee (resolved to `Call`, not `StaticCall`) — does NOT leak. The diff between them IS
the bug: the interpret↔shared-library `StaticCall` boundary (`state/mod.rs::static_call`) orphans the
retbuf-return store; the local `Call` path (`fn_return`) frees it. See `../cluster-fold-reads-null.md` arc F.

## Graduation gate (probe → `tests/scripts/`)

A probe graduates only when ALL hold: assertions pass · clean process exit (check the exit
code, not just "PASSED" print) · no leak warning · bounded runtime (seconds). A probe that
passes assertions but fails any other gate stays here with a status note; graduate a
representative sibling from the same cluster instead.
