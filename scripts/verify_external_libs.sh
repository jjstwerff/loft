#!/usr/bin/env bash
# verify_external_libs.sh — pre-PR validation of the extracted library
# repos (loft-lang/loft-libs-*) against the LOCALLY-built loft.
#
# Mirrors each chunk repo's `.github/workflows/library-ci.yml` exactly
#   1. loft --interpret test   — interpreter end-to-end
#   2. loft --native test      — native codegen end-to-end
# but builds loft from the CURRENT working tree instead of cloning
# jjstwerff/loft main.  So before merging a loft change (e.g. a parser
# feature the external repos depend on) you can confirm the external
# CI will pass — no round-trip through a PR + GitHub Actions.
#
# Usage:
#   scripts/verify_external_libs.sh                 # all repos, both steps
#   scripts/verify_external_libs.sh --interpret     # interpreter step only
#   scripts/verify_external_libs.sh --repo net      # one chunk (net|core|graphics)
#   scripts/verify_external_libs.sh --src net=/tmp/net-clean
#                                                   # validate a LOCAL clone
#                                                   # (e.g. a PR branch) instead
#                                                   # of cloning loft-lang main
#
# Exit code: non-zero if any selected step fails.  The interpreter and
# native results are reported separately so a known native-infra gap
# (see plan-12 Phase 6.5) does not mask an interpreter regression.

set -uo pipefail

# Cache clean/release rebuilds with sccache when present (no-op otherwise).
source "$(dirname "${BASH_SOURCE[0]}")/sccache_env.sh"

ROOT="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
LOFT="$ROOT/target/release/loft"
CACHE="$ROOT/target/external-libs"
ORG="https://github.com/loft-lang"

# chunk → package list (mirrors each repo's CI matrix).
declare -A PACKAGES=(
  [net]="web server game_protocol"
  [core]="arguments crypto random"
  [graphics]="shapes gridmesh"
)
declare -A REPO_NAME=(
  [net]="loft-libs-net"
  [core]="loft-libs-core"
  [graphics]="loft-libs-graphics"
)
declare -A SRC_OVERRIDE=()   # chunk → local path (via --src)

CHUNKS="net core graphics"
RUN_INTERPRET=1
RUN_NATIVE=1

while [ $# -gt 0 ]; do
  case "$1" in
    --interpret) RUN_NATIVE=0 ;;
    --native)    RUN_INTERPRET=0 ;;
    --repo)      CHUNKS="$2"; shift ;;
    --src)       SRC_OVERRIDE["${2%%=*}"]="${2#*=}"; shift ;;
    -h|--help)   sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
  shift
done

# --- build loft from the current working tree ---------------------------
echo "==> building loft from $(git -C "$ROOT" rev-parse --abbrev-ref HEAD) ($(git -C "$ROOT" rev-parse --short HEAD))"
if ! cargo build --release --bin loft --manifest-path "$ROOT/Cargo.toml" 2>&1 | tail -2; then
  echo "FATAL: loft build failed" >&2; exit 1
fi

PASS=0; FAIL=0
declare -a RESULTS=()

run_step() {  # label, dir, loft-args...
  local label="$1" dir="$2"; shift 2
  if (cd "$dir" && "$LOFT" "$@" >/tmp/velib_out.$$ 2>&1); then
    RESULTS+=("  ok    $label"); PASS=$((PASS+1))
  else
    RESULTS+=("  FAIL  $label  → $(grep -m1 -iE 'error|FAIL' /tmp/velib_out.$$ | sed 's/^ *//' | cut -c1-90)")
    FAIL=$((FAIL+1))
  fi
  rm -f /tmp/velib_out.$$
}

mkdir -p "$CACHE"
for chunk in $CHUNKS; do
  repo="${REPO_NAME[$chunk]:-}"
  [ -n "$repo" ] || { echo "unknown chunk: $chunk" >&2; continue; }

  if [ -n "${SRC_OVERRIDE[$chunk]:-}" ]; then
    src="${SRC_OVERRIDE[$chunk]}"
    echo "==> $repo (local: $src)"
  else
    src="$CACHE/$repo"
    echo "==> $repo (fresh shallow clone)"
    rm -rf "$src"
    git clone --depth=1 "$ORG/$repo.git" "$src" 2>&1 | tail -1
  fi

  for pkg in ${PACKAGES[$chunk]}; do
    [ -d "$src/$pkg" ] || { RESULTS+=("  skip  $repo/$pkg (absent)"); continue; }
    [ "$RUN_INTERPRET" = 1 ] && run_step "$repo/$pkg  [interpret]" "$src/$pkg" --interpret test
    [ "$RUN_NATIVE"    = 1 ] && run_step "$repo/$pkg  [native]   " "$src/$pkg" --native test
  done
done

echo
echo "================ external-library validation ================"
printf '%s\n' "${RESULTS[@]}"
echo "-------------------------------------------------------------"
echo "  $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
