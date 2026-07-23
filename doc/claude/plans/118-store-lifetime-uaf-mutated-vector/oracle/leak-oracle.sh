#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN118 arc F — differential interp-vs-native store-leak oracle.
#
# The insight the arc-F session lacked (see ../cluster-fold-reads-null.md § Method
# retrospective): NATIVE is the clean reference.  Whole-`--native` compiles the
# script AND every library into one binary — there is NO interp↔cdylib shared
# bridge — so a bridge-boundary leak simply cannot occur there.  Therefore the
# INTERP-MINUS-NATIVE leaked-store set, attributed by allocation site, IS the bug,
# with zero inference.  One run gives the boundary that a dozen ad-hoc traces did not.
#
# It runs the SAME probe two ways and prints a verdict:
#   - interpret (--interpret, LOFT_LEAK_SITES=1)      → the candidate leak + its sites
#   - native    (--native,    LOFT_NATIVE_LEAK_CHECK) → the clean reference
# INTERP-ONLY leak  → the bug (exit 2).  Leak on BOTH → not bridge-specific (exit 3).
# Clean on interp   → no interp leak (exit 0).
#
# Usage:
#   leak-oracle.sh <probe.loft> [--lib <dir>] [--flip]
#     --flip  sets LOFT_NO_BRIDGE_ORPHAN_FREE=1 (the arc-F POSITIVE CONTROL: the fix
#             is disabled, so a nested-struct-return probe MUST show the interp-only leak).
#
# The `--interpret` flag is passed EXPLICITLY so the verdict does not depend on the
# box's default backend (this dev box defaults to --native; CI may default to mixed).
set -u

LOFT=${LOFT_BIN:-"$(cd "$(dirname "$0")/../../../../.." && pwd)/target/release/loft"}
[ -x "$LOFT" ] || LOFT=loft   # fall back to PATH

probe=""; libdir=""; flip=""
while [ $# -gt 0 ]; do
  case "$1" in
    --lib) libdir="$2"; shift 2 ;;
    --flip) flip="1"; shift ;;
    *) probe="$1"; shift ;;
  esac
done
[ -n "$probe" ] || { echo "usage: leak-oracle.sh <probe.loft> [--lib <dir>] [--flip]" >&2; exit 64; }

libarg=(); [ -n "$libdir" ] && libarg=(--lib "$libdir")
flipenv=(); [ -n "$flip" ] && flipenv=(LOFT_NO_BRIDGE_ORPHAN_FREE=1)

# grep the exit-time leak warning + per-site attribution.
leak_lines() { grep -E "stores not freed at program exit|\[leak-site\]" || true; }
has_error() { grep -qiE "^error:|error\[|Syntax error|Unknown .* directive|could not|not found in rlib"; }

# Force a fresh cdylib so a stale/native-built one in the shared `native-auto/` cannot
# mask the interp bridge path or ignore the flip env.
freshen() { [ -n "$libdir" ] && find "$libdir" -type d -name native-auto -exec rm -rf {} + 2>/dev/null; return 0; }

echo "── probe: $probe ${flip:+(FLIP: fix disabled)}"

freshen
interp_raw=$(env "${flipenv[@]}" LOFT_LEAK_SITES=1 LOFT_TIMEOUT=120 "$LOFT" \
  --interpret "${libarg[@]}" "$probe" 2>&1)
if printf '%s' "$interp_raw" | has_error; then
  echo "  interpret: ERROR (probe failed to compile/run)"
  printf '%s\n' "$interp_raw" | grep -iE "^error:|error\[|Syntax error|Unknown .* directive" | head -3 | sed 's/^/    /'
  echo "  VERDICT: probe broken (not a leak signal)"; exit 64
fi
interp=$(printf '%s' "$interp_raw" | leak_lines)
if [ -n "$interp" ]; then
  echo "  interpret: LEAK"
  echo "$interp" | sed 's/^/    /'
else
  echo "  interpret: clean"
fi

# Native needs rustc (and, for graphics-dependent probes, its system deps).  Skip
# gracefully rather than fail the oracle when native cannot build.
if command -v rustc >/dev/null 2>&1; then
  freshen
  nat_raw=$(env "${flipenv[@]}" LOFT_NATIVE_LEAK_CHECK=1 LOFT_TIMEOUT=120 "$LOFT" \
    --native "${libarg[@]}" "$probe" 2>&1)
  native=$(printf '%s' "$nat_raw" | leak_lines)
  if printf '%s' "$nat_raw" | grep -qiE "error\[|error:|could not|not found in rlib"; then
    echo "  native:    build unavailable (skipped — treat interp verdict as-is)"
    native="__SKIP__"
  elif [ -n "$native" ]; then
    echo "  native:    LEAK"
    echo "$native" | sed 's/^/    /'
  else
    echo "  native:    clean (reference)"
  fi
else
  echo "  native:    rustc unavailable (skipped)"
  native="__SKIP__"
fi

echo -n "  VERDICT: "
if [ -z "$interp" ]; then
  echo "no interp leak"; exit 0
elif [ "$native" = "__SKIP__" ]; then
  echo "INTERP LEAK (native reference unavailable — cannot confirm interp-only)"; exit 2
elif [ -z "$native" ]; then
  echo "INTERP-ONLY LEAK  ← the bug (native is the clean reference)"; exit 2
else
  echo "leak on BOTH backends — not the interp↔cdylib bridge class"; exit 3
fi
