#!/usr/bin/env bash
# @PLN85 — per-test leak-OWNER sweep over the ASan issues test binary.  Confirms
# the remaining leakers share the append_text<-execute_argv site and classifies
# each by shape (see text-tail-return-leak.md § Session 4 verified analysis).
#   Usage: BIN=<asan issues test bin> ./sweep_owners.sh
set -u
DIR=$(cd "$(dirname "$0")" && pwd)
BIN=${BIN:?set BIN to the ASan-instrumented issues test binary}
"$BIN" --list --format terse 2>/dev/null | sed 's/: test$//' \
  | xargs -P 6 -I{} "$DIR/leakowner.sh" "$BIN" {} | sort
