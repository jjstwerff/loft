#!/usr/bin/env bash
# @PLN85 text-return analysis framework — verify the SHADOW analysis beside the
# tests.  Runs the compiler with LOFT_TRA_DUMP=1 (prints `TRA <fn> => <verdict>`
# per text-returning fn, WITHOUT changing codegen) and diffs each verdict against
# the `// VERDICT:` annotation above the matching `fn` in corpus.loft.
#   Usage: BIN=<loft> ./verify.sh   (BIN defaults to target/release/loft)
set -u
DIR=$(cd "$(dirname "$0")" && pwd)
ROOT=$(git -C "$DIR" rev-parse --show-toplevel)
BIN=${BIN:-$ROOT/target/release/loft}
CORPUS="$DIR/corpus.loft"

# expected: "<fn> <verdict>" from `// VERDICT: X` immediately above `fn NAME`.
expected=$(awk '
  /^[[:space:]]*\/\/ VERDICT:/ { v=$3; next }
  /^fn [a-zA-Z_]/ && v!="" { name=$2; sub(/[<(].*/,"",name); print name, v; v="" }
' "$CORPUS" | sort)

# actual: LOFT_TRA_DUMP names a FILE the compiler appends verdicts to (a
# deterministic channel — loft's stderr races with process::exit).  Strip a
# leading n_ so generic monomorphs (n_f_*) match the source name.
# LOFT_NO_CACHE forces a fresh parse each run — the program cache is
# content-keyed, so a warm hit would skip the parse (and the dump).
dump=$(mktemp); : >"$dump"
LOFT_NO_CACHE=1 LOFT_TRA_DUMP="$dump" "$BIN" "$CORPUS" >/dev/null 2>&1
actual=$(sed -n 's/^TRA \(.*\) => \(.*\)$/\1 \2/p' "$dump" | sed 's/^n_//' | sort -u)
rm -f "$dump"

pass=0; fail=0
printf "%-22s %-20s %-20s %s\n" FN EXPECTED ACTUAL RESULT
while read -r fn exp; do
  [ -z "$fn" ] && continue
  act=$(echo "$actual" | awk -v f="$fn" '$1==f {print $2}' | head -1)
  if [ "$act" = "$exp" ]; then r=ok; pass=$((pass+1)); else r=MISMATCH; fail=$((fail+1)); fi
  printf "%-22s %-20s %-20s %s\n" "$fn" "$exp" "${act:-<none>}" "$r"
done <<< "$expected"
echo "----"
echo "pass=$pass fail=$fail"
[ "$fail" -eq 0 ]
