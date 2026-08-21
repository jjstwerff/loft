#!/usr/bin/env bash
# @PLN146 W0 — render every corpus scene under the EXISTING Python renderer.
#
#   ./w0.sh            compare each render against the committed golden (the gate)
#   ./w0.sh --bless    (re)write the goldens from the current oracle
#
# Red when a scene will not parse, when a `check` fails, or when a render stops
# matching its golden.  The corpus is the specification arc W's loft port owes:
# whatever `oracle/draw.py` accepts here is the grammar, measured rather than guessed.
set -u
cd "$(dirname "$0")"
bless=0
[ "${1:-}" = "--bless" ] && bless=1
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
mkdir -p golden
fail=0
total=0
for scene in scenes/*.draw; do
  name=$(basename "$scene" .draw)
  total=$((total + 1))
  log="$out/$name.log"
  if ! SKETCH_OUT="$out/$name" python3 oracle/draw.py --once "$scene" >"$log" 2>&1; then
    echo "RED  $name — oracle exited non-zero"
    sed 's/^/       /' "$log"
    fail=$((fail + 1))
    continue
  fi
  if grep -q "UNPARSED" "$out/$name/stats.txt" 2>/dev/null; then
    echo "RED  $name — unparsed line(s)"
    grep -A9 "UNPARSED" "$out/$name/stats.txt" | sed 's/^/       /'
    fail=$((fail + 1))
    continue
  fi
  if [ "$bless" = 1 ]; then
    cp "$out/$name/canvas.png" "golden/$name.png"
    continue
  fi
  if [ ! -f "golden/$name.png" ]; then
    echo "RED  $name — no committed golden (run ./w0.sh --bless)"
    fail=$((fail + 1))
  elif ! cmp -s "$out/$name/canvas.png" "golden/$name.png"; then
    echo "RED  $name — render differs from its golden"
    fail=$((fail + 1))
  fi
done
if [ "$bless" = 1 ]; then
  echo "blessed $total golden(s)"
  exit 0
fi
[ "$fail" = 0 ] && echo "$total scene(s) match their golden" || echo "$fail of $total scene(s) red"
exit $((fail > 0))
