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
Expected NOW: 0 differing.  All three defects are fixed, so this matrix is a REGRESSION
guard, and a fresh failure here is a real one.

Where it started (2026-07-21), 8 failing combinations — 4 cells × both backends, so every
one was a SHARED defect, never a backend divergence:

  c_u16_direct   compile error "Cannot assign to attribute on type 'OpGetShortRaw'"
  c_i16_direct   same — the 2-byte raw reader was missing from the read→write op map
  c_u32_viafn    SILENT: `x as u32?` returned null for every value, so `?? 0` wrote a zero
  c_u32_read     SILENT: 4000000000 read back -294967296 — no unsigned 4-byte op existed

The u32 read defect was never vector-specific: `c_u32_field` failed identically, which is
what proved the WIDTH and not the container was the axis.  Keep both cells.

To prove this harness can still fail, break one thing on purpose — e.g. drop the
`"OpGetInt4Raw" | "OpGetInt4Full"` arm from `parse_assign`'s read→write map
(`src/parser/operators.rs`) and `c_u32_direct`/`c_u32_viafn` must go red again.  That
omission is how this fix first landed, so it is a live failure mode, not a hypothetical.

Green cells to protect: u8 and i32 on every path — `i32` in particular must keep the
SIGNED 4-byte ops, since its stored bytes are two's complement and are read outside the
narrow-int family.  `integer`/`single` are the control that the harness itself works.
EOF
