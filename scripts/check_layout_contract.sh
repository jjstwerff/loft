#!/usr/bin/env bash
# @PLN102 arc-E flip-gate — Gate 1 step 3: layout-hash-changed ⇒ CONTRACT_VERSION must bump.
#
# The store layout (tests/golden/layout/corpus.txt + LAYOUT_ALGO_HASH in
# tests/layout_golden.rs) IS the persistence contract: a persisted store written under
# the old layout is misread under a changed one. So POST-FREEZE a change to the layout
# golden may land only ALONGSIDE a CONTRACT_VERSION bump (a declared, epoch-style break).
#
# INERT while CONTRACT_VERSION == 0 (pre-freeze the language is still settling, so layout
# changes are free — you just re-bless the golden). The gate arms automatically at the
# 0 -> 1 flip; nothing here changes on flip day (flip-gate.md Gate 1 step 4).
#
# Non-blocking by design (like api-compat): it INFORMS ("this changed the persistence
# contract without declaring it"); the doctrine is a conscious decision, not prevention.
#
# Usage:  scripts/check_layout_contract.sh [BASE_REF]      (default BASE_REF = origin/main)
#         scripts/check_layout_contract.sh --self-test     (prove the decision table)
set -euo pipefail

GOLDEN="tests/golden/layout/corpus.txt"
MANIFEST="src/manifest.rs"

# Pure decision function — the whole policy, so --self-test can exercise it without git.
#   args: <contract> <layout_changed 0|1> <contract_bumped 0|1>
#   echoes a verdict word (ok|inert|break) and returns 0 for pass, 1 for a break.
decide() {
  local contract="$1" layout_changed="$2" contract_bumped="$3"
  if [ "$contract" -eq 0 ]; then echo "inert"; return 0; fi        # pre-freeze: free
  if [ "$layout_changed" -eq 0 ]; then echo "ok"; return 0; fi     # nothing moved
  if [ "$contract_bumped" -eq 1 ]; then echo "ok"; return 0; fi    # declared break
  echo "break"; return 1                                           # silent break
}

if [ "${1:-}" = "--self-test" ]; then
  fail=0
  check() { # <expect_verdict> <expect_rc> <contract> <layout> <bump>
    local got rc; got=$(decide "$3" "$4" "$5") && rc=0 || rc=$?
    if [ "$got" != "$1" ] || [ "$rc" -ne "$2" ]; then
      echo "SELF-TEST FAIL: decide($3,$4,$5) = ($got,$rc), expected ($1,$2)"; fail=1
    fi
  }
  check inert 0 0 1 0   # contract 0 → always inert, even a layout change
  check inert 0 0 0 0
  check ok    0 1 0 0   # post-flip, layout unchanged → ok
  check ok    0 1 1 1   # post-flip, layout changed WITH a bump → ok
  check break 1 1 1 0   # post-flip, layout changed, NO bump → break
  check break 1 2 1 0   # any contract >= 1 behaves the same
  if [ "$fail" -eq 0 ]; then echo "check_layout_contract self-test: PASS"; exit 0; fi
  echo "check_layout_contract self-test: FAIL"; exit 1
fi

BASE="${1:-origin/main}"

contract=$(grep -oP 'pub const CONTRACT_VERSION: u32 = \K[0-9]+' "$MANIFEST")

# Did the layout golden move vs the base?  A real layout change always rewrites the dump;
# a corpus-only addition also rewrites it (conservative — flags for a human, fail-closed).
if git diff --quiet "$BASE" -- "$GOLDEN"; then layout_changed=0; else layout_changed=1; fi

# Did CONTRACT_VERSION change in this diff?
if git diff "$BASE" -- "$MANIFEST" | grep -qE '^\+[^+].*CONTRACT_VERSION: u32 ='; then
  contract_bumped=1
else
  contract_bumped=0
fi

verdict=$(decide "$contract" "$layout_changed" "$contract_bumped") && rc=0 || rc=$?
case "$verdict" in
  inert) echo "layout-contract gate: INERT (CONTRACT_VERSION=$contract, pre-freeze)"; ;;
  ok)    echo "layout-contract gate: ok (contract=$contract, layout_changed=$layout_changed, bumped=$contract_bumped)"; ;;
  break)
    cat >&2 <<MSG
layout-contract gate: FAIL — the store layout changed but CONTRACT_VERSION did not bump.

A layout change is a SILENT persistence break: a store persisted under the old layout
is misread under the new one. Post-freeze (CONTRACT_VERSION=$contract) that may land only
as a DECLARED break. Either:
  - bump CONTRACT_VERSION in $MANIFEST (and set LAYOUT_CONTRACT = the new value in
    tests/layout_golden.rs), or
  - if the change is layout-neutral, revert the golden re-bless ($GOLDEN).
MSG
    ;;
esac
exit "$rc"
