#!/usr/bin/env bash
# Scores the REFUSAL channel: does the type system stop this fault before it
# reaches a non-null narrow slot, which has no code a null could occupy?
#
# Two controls, because one is not enough:
#   c05      must COMPILE — otherwise this measures "loft rejects things".
#   the binary check below must pass — a missing binary produces no "error"
#   line either, which would read as COMPILES for every cell.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(git -C "$HERE" rev-parse --show-toplevel)"
L="${LOFT_BIN:-$ROOT/target/release/loft}"
[ -x "$L" ] || { echo "no loft binary at $L (set LOFT_BIN, or cargo build --release --bin loft)"; exit 2; }
cd "$HERE"
fail=0
for f in *.loft; do
  want=$(head -1 "$f" | sed 's|// want: ||')
  out=$(LOFT_TIMEOUT=30 timeout 60 "$L" --path "$ROOT/" --interpret "$f" 2>&1)
  if printf '%s\n' "$out" | grep -q '^error'; then got=REFUSED; else got=COMPILES; fi
  msg=$(printf '%s\n' "$out" | grep -m1 '^error' | cut -c1-92)
  if [ "$want" = "$got" ]; then st="ok  "; else st="FAIL"; fail=1; fi
  printf '%s %-26s want=%-9s got=%-9s %s\n' "$st" "${f%.loft}" "$want" "$got" "$msg"
done
exit $fail
