#!/usr/bin/env bash
# H3 — narrow-width vector elements.  Three independent defects share one matrix because
# they share one question: does a `vector<T>` element behave like every other container of
# T?  The reference container is a plain `integer` vector and a struct FIELD of the same
# width — both known good — so a cell that fails names the width, not the value.
#
# The axes, one file per (type, path):
#   direct  — `v[i] = x` on a local vector
#   viafn   — `v[i] = x` inside a called fn, reached through a struct parameter
#   read    — round-trip of the widest value the type can hold (no assignment at all)
#
# Each cell prints ONE line: `<cell> <actual> | <expected>`.  Expectations are hand
# computed from the type's declared range (doc/claude/LOFT.md § narrow widths), not from
# a second binary — agreement between backends is not a pass here, since a width bug
# lowers the same wrong way on both.
#
# Usage: ./run.sh [path-to-loft]   (default: <repo>/target/release/loft)
set -uo pipefail
cd "$(dirname "$0")"
LOFT=${1:-../../../../../target/release/loft}
[ -x "$LOFT" ] || { echo "no loft binary at $LOFT" >&2; exit 2; }

fail=0
printf "%-22s %-9s %s\n" CELL BACKEND RESULT
for f in c_*.loft; do
  c=${f%.loft}
  for b in --interpret --native; do
    out=$(LOFT_TIMEOUT=120 "$LOFT" "$b" "$f" 2>&1)
    line=$(printf '%s' "$out" | grep -E '^(got|error)' | head -1)
    # A cell passes only when it RAN and its actual half equals its expected half.
    got=${line%%|*}; want=${line#*|}
    if [ -z "$line" ]; then
      verdict="NO OUTPUT (vacuous — the cell did not run)"; fail=$((fail+1))
    elif [ "$(echo "$got" | xargs)" = "got $(echo "$want" | xargs)" ]; then
      verdict="PASS"
    else
      verdict="FAIL  $line"; fail=$((fail+1))
    fi
    printf "%-22s %-9s %s\n" "$c" "${b#--}" "$verdict"
  done
done
echo
echo "$fail cell/backend combinations differ from the hand-computed expectation."
cat <<'EOF'
Baseline on a CLEAN tree (2026-07-21), 8 failing combinations — 4 cells × both backends,
so every one of these is a SHARED defect, not a backend divergence:

  c_u16_direct   compile error "Cannot assign to attribute on type 'OpGetShortRaw'"
  c_i16_direct   same — the 2-byte raw accessor has no assignable counterpart
  c_u32_viafn    SILENT: the write through the struct parameter is discarded, reads 0
  c_u32_read     SILENT: 4000000000 reads back -294967296 — the 4-byte load sign-extends

The u32 read defect is NOT vector-specific: a struct FIELD of type u32 loses the same
value (see c_u32_field, which fails identically).  Keep it in this matrix anyway — it is
what proves the width, not the container, is the axis.

Green cells to protect: u8 and i32 on every path, and `integer`/`single` as the control
that the harness itself is not simply broken.  A run reporting 0 on a clean tree means
the instrument went blind, not that the tree is fixed.
EOF
