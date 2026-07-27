#!/usr/bin/env bash
# H7 boundary matrix — every probe appends 3 elements to an empty vector, so the
# expected answer is 3 unless the table in ../../doc/claude/plans/h7-retbuf-alias/
# says otherwise.  Run against a candidate fix: all of 01/04/06/07/08/13/16 must
# turn correct, and 02/03/05/11/12/15/17 must STAY correct (they pass today).
set -uo pipefail
LOFT="${LOFT:-./target/release/loft}"
for f in "$(dirname "$0")"/*.loft; do
  i=$(LOFT_TIMEOUT=60 "$LOFT" --interpret "$f" 2>&1 | tail -1)
  n=$(LOFT_TIMEOUT=180 "$LOFT" --native   "$f" 2>&1 | tail -1)
  printf "  %-34s interpret=%-22s native=%-22s%s\n" \
    "$(basename "$f" .loft)" "$i" "$n" "$([ "$i" = "$n" ] || echo '  ← BACKENDS DISAGREE')"
done
