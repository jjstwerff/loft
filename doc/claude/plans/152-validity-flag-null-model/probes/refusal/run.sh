#!/usr/bin/env bash
# Scores the REFUSAL channel: does the type system stop this fault reaching a
# non-null narrow slot?  Fails if the control is refused (instrument is vacuous).
cd "$(dirname "$0")"
L=/home/jurjens/workspace/loft2/target/release/loft
fail=0
for f in *.loft; do
  want=$(head -1 "$f" | sed 's|// want: ||')
  out=$(LOFT_TIMEOUT=30 timeout 60 "$L" --path /home/jurjens/workspace/loft2/ --interpret "$f" 2>&1)
  if echo "$out" | grep -q "^error"; then got=REFUSED; else got=COMPILES; fi
  msg=$(echo "$out" | grep -m1 "^error" | cut -c1-96)
  if [ "$want" = "$got" ]; then st="ok  "; else st="FAIL"; fail=1; fi
  printf '%s %-26s want=%-9s got=%-9s %s\n' "$st" "${f%.loft}" "$want" "$got" "$msg"
done
exit $fail
