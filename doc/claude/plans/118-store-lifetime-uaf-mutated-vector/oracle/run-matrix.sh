#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN118 arc F — the probe matrix, each cell run through the differential oracle
# and checked against its HAND-COMPUTED expected verdict.  This is what the arc-F
# session should have built first (build the oracle + matrix, read the boundary off
# them — do not theorize past the first contradictory trace).
#
# Composition axis isolated: how a shared/cdylib fn produces its struct return.
#   direct struct-literal return   → clean (caller forwards a stable retbuf; no fallback)
#   NESTED-call return             → leak  (caller forwards null; bridge allocates a
#                                            fallback dest the inner fn orphans)
# The negative controls prove the trigger is specifically the nested return, and the
# FLIP (LOFT_NO_BRIDGE_ORPHAN_FREE=1) is the positive control: it MUST resurrect the
# leak, otherwise the whole matrix is vacuous.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
probes="$here/../probes"
lib="$probes/lib"
oracle="$here/leak-oracle.sh"
fails=0

# cell <name> <expected-exit> <probe> [oracle-args...]
cell() {
  local name="$1" want="$2" probe="$3"; shift 3
  bash "$oracle" "$probe" "$@" >/tmp/arcf_oracle_out.$$ 2>&1
  local got=$?
  cat /tmp/arcf_oracle_out.$$ | sed 's/^/    /'
  if [ "$got" = "$want" ]; then
    echo "  ✓ $name (exit $got == expected $want)"
  else
    echo "  ✗ $name (exit $got, EXPECTED $want)"; fails=$((fails+1))
  fi
  rm -f /tmp/arcf_oracle_out.$$
  echo
}

echo "═══ @PLN118 arc F — leak oracle matrix ═══"
# 0 = no interp leak, 2 = interp-only leak (the bug)
cell "direct struct-literal return (negative control)" 0 \
     "$probes/arcF-control-direct-struct.loft" --lib "$lib"
cell "nested-call return, fix ON (must be clean)" 0 \
     "$probes/arcF-min-nested-struct.loft" --lib "$lib"
cell "nested-call return, fix OFF (POSITIVE CONTROL — must leak)" 2 \
     "$probes/arcF-min-nested-struct.loft" --lib "$lib" --flip

echo "═══ result ═══"
if [ "$fails" = 0 ]; then
  echo "ALL CELLS PASS — fix holds, positive control fires (oracle non-vacuous)."
  exit 0
else
  echo "$fails CELL(S) FAILED."
  exit 1
fi
