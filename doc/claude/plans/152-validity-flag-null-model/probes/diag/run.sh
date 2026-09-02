#!/usr/bin/env bash
# Scores the DIAGNOSTIC channel: which SEVERITY, and which CODE.  A coarse
# "any diagnostic" test would score a dead-assignment as a pass and cannot
# tell a hard refusal from a warning.
cd "$(dirname "$0")"
L=/home/jurjens/workspace/loft2/target/release/loft
printf '%-22s %-8s %-9s %s\n' CELL WANT GOT "FIRST DIAGNOSTIC"
for f in *.loft; do
  want=$(head -1 "$f" | sed 's|// want: ||')
  err=$(LOFT_TIMEOUT=30 timeout 60 "$L" --path /home/jurjens/workspace/loft2/ --interpret "$f" 2>&1 >/dev/null)
  line=$(printf '%s\n' "$err" | grep -m1 -E '^(warning|error|advice)' )
  case "$line" in
    error*)   got=ERROR ;;
    warning*) got=WARN ;;
    advice*)  got=ADVICE ;;
    *)        got=SILENT ;;
  esac
  printf '%-22s %-8s %-9s %s\n' "${f%.loft}" "$want" "$got" "$(printf '%s' "$line" | cut -c1-78)"
done
