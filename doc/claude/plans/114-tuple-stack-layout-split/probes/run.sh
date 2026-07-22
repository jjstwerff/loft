#!/usr/bin/env bash
# @PLN114 — run the tuple-placement probe corpus on BOTH backends.
#
# Usage:  ./run.sh [path-to-loft]
#   default: <repo>/target/release/loft
#   pass the @PLN85 debug-assertions build to make width mismatches attribute
#   themselves instead of segfaulting (see README.md).
#
# Prints one row per cell per backend.  EXPECTED is the hand-computed value from
# README.md — a cell whose output merely matches the other backend is NOT a pass.
set -uo pipefail
cd "$(dirname "$0")"

LOFT=${1:-../../../../../target/release/loft}
[ -x "$LOFT" ] || { echo "no loft binary at $LOFT" >&2; exit 2; }

# Hand-computed expectations.  Keep in sync with README.md's table.
declare -A EXP=(
  [int2]="11,22"          [text2]="aa,bb"        [int_text]="11,bb"
  [nested]="1,2,3"        [ref_int]="10,22"      [int_ref]="22,10"
  [vec_int]="7,22"        [fn_int_call]="5 49"   [fn2]="14"
  [ref2_local]="10,20"    [ref2]="10,20"         [ref3]="1,2,3"
  [vec2]="7,9"            [ref_text]="10,x"      [fn_text_read]="tag"
  [fn_text_call]="tag 49" [ref2_min]="10,20"     [fn_text_call_min]="sq-tag 49"
  # step-0 additions: arity, middle-slot position, and the destination axis
  [ref4]="1,2,3,4"        [int_ref_int]="1,5,9"  [ref_int_ref]="5,9,7"
  [ref2_field]="10,20"    [ref2_return]="10,20"  [tupleput_ref]="30,20"
  # step-2 alignment axis: what follows a 12-byte Reference decides it
  [ref_float]="10,2.5"    [ref_bool]="10,true"   [ref_char]="10,A"
  # small-type pairs (the alias-width axis)
  [bool3]="true,false,true" [char2]="A,B"        [u8pair]="7,9"
  [u16pair]="300,400"
)

fail=0
printf "%-17s %-11s %-12s %s\n" CELL BACKEND EXPECTED ACTUAL
printf -- "----------------- ----------- ------------ ------------------------------\n"
for f in *.loft; do
  cell=${f%.loft}
  for backend in --interpret --native; do
    out=$(LOFT_TIMEOUT=60 "$LOFT" "$backend" "$f" 2>&1)
    if grep -q "expected .*B on stack but" <<<"$out"; then
      got=$(grep -o "expected [0-9]*B on stack but generate(.*) pushed [0-9]*B" <<<"$out" \
            | sed 's/ on stack but generate(.*) pushed/ vs/')
    elif grep -q SIGSEGV <<<"$out"; then
      got="SIGSEGV"
    else
      got=$(tr '\n' ' ' <<<"$out" | sed 's/  */ /g; s/ $//')
    fi
    want=${EXP[$cell]:-"(no expectation recorded)"}
    mark=" "
    [ "$got" = "$want" ] || { mark="!"; fail=$((fail + 1)); }
    printf "%-17s %-11s %-12s %s%s\n" "$cell" "${backend#--}" "$want" "$mark" "$got"
  done
done

echo
echo "$fail cell/backend combinations differ from the hand-computed expectation."
echo "Before the fix this is EXPECTED to be non-zero — see README.md for which."
