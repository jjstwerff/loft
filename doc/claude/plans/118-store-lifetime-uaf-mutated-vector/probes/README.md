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

| # | file | axis varied | expected (fix ON) | interp | native | flip (fix OFF) | cluster |
|---|---|---|---|---|---|---|---|
| G1 | `arcF-min-nested-struct.loft` + `lib/arcf/` | shared fn returns a NESTED call (`wrap_v3`→`make_v3`) | clean | clean | clean | **299× V3 LEAK** | arc F |
| G0 | `arcF-control-direct-struct.loft` + `lib/arcf/` | control: shared fn returns a struct LITERAL directly | clean | clean | clean | clean | arc F |
| F2 | `arcF-control-local-nested-noleak.loft` | control: LOCAL nested-call return (no `--lib`, no bridge) | clean | clean | clean | clean | arc F |
| F1 | `arcF-min-leak-hex_to_world.loft` | the original moros repro (`use moros_render`) | clean | — | — | 299× Vec3 LEAK | arc F |

**Arc F — the boundary (corrected).** The self-contained **`lib/arcf/`** fixture replaces the
moros-dependent F1: a shared/cdylib fn returning a **NESTED call** (`wrap_v3` → `make_v3`) leaks the
bridge's fallback dest (**G1**), while a **direct** struct-literal return through the same bridge does
not (**G0**). The prior "cross-lib `StaticCall`" localization was a **stale-cdylib confound** — with a
fresh cdylib a *same-lib* nested return leaks identically. The trigger is the **nested return**: it
makes the interpreted caller forward a null hidden-dest retbuf, so the bridge allocates a fallback
record the inner struct-literal callee then orphans. Interp-only (whole-`--native` has no bridge — F2
and the native columns confirm). Fix in `native_lib.rs::shared_bridge_wrapper`; the FLIP column is
`LOFT_NO_BRIDGE_ORPHAN_FREE=1` (the positive control). Run all cells through
[`../oracle/run-matrix.sh`](../oracle/run-matrix.sh). See `../cluster-fold-reads-null.md` § Arc F — RESOLVED.

Run: `loft --interpret --lib lib/ arcF-min-nested-struct.loft` (+ `LOFT_LEAK_SITES=1`); add
`LOFT_NO_BRIDGE_ORPHAN_FREE=1` to reproduce the pre-fix leak.

## Graduation gate (probe → `tests/scripts/`)

A probe graduates only when ALL hold: assertions pass · clean process exit (check the exit
code, not just "PASSED" print) · no leak warning · bounded runtime (seconds). A probe that
passes assertions but fails any other gate stays here with a status note; graduate a
representative sibling from the same cluster instead.
