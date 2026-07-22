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
| _ | _(none yet — Stage A)_ | | | | | | |

## Graduation gate (probe → `tests/scripts/`)

A probe graduates only when ALL hold: assertions pass · clean process exit (check the exit
code, not just "PASSED" print) · no leak warning · bounded runtime (seconds). A probe that
passes assertions but fails any other gate stays here with a status note; graduate a
representative sibling from the same cluster instead.
