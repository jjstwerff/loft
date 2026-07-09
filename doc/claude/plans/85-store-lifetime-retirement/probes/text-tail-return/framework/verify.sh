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

# actual: strip a leading n_ so generic monomorphs (n_f_*) match the source name.
actual=$(LOFT_TRA_DUMP=1 "$BIN" "$CORPUS" 2>&1 >/dev/null \
  | sed -n 's/^TRA \(.*\) => \(.*\)$/\1 \2/p' \
  | sed 's/^n_//' | sort -u)

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
