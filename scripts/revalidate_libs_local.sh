#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# revalidate-libs, on this box, from a work branch.
#
# `.github/workflows/revalidate-libs.yml` compiles every PUBLISHED library against
# this loft, and it triggers on `pull_request` and `push` to `main` ONLY — never on
# a work branch.  A language change that retro-breaks a shipped library is therefore
# invisible for as long as the branch stays unmerged, which on a bundled branch is
# days.  On 2026-08-19 that was nine libraries losing their entire public surface to
# one resolution rule, green on every branch gate for a full day
# (DEVELOPMENT.md § Opening a PR is the owner's call).
#
# Running ONE library's suite is the advice that incident produced, and it is not the
# gate: the gate is the whole registry.  This script is that, locally.
#
# What it does, per published package at its LATEST non-yanked version:
#   1. reads the matrix from ../loft-registry/index.json (the same source the
#      workflow's `discover` job uses),
#   2. extracts the release TAG's tree with `git archive` from the sibling clone,
#   3. runs `loft --interpret --tests tests` against the loft you built,
#   4. on failure re-classifies exactly as the workflow does — recompile every
#      `.loft` with `--dump`; still compiles => environment/native-deps (this box
#      lacks ALSA, a display, a network); fails to compile => a genuine language
#      break, which is what the freeze forbids.
#
# READ-ONLY on the sibling repos, deliberately.  `git archive <tag> | tar -x` into a
# scratch directory writes nothing to their tree or their `.git`, which matters
# because those repos are usually somebody's live working tree — a `git checkout`
# there lands in another agent's uncommitted work, and `loft test` INSIDE a package
# rebuilds `native-auto/` and writes `.loft/` caches (CLAUDE.md § Dogfood loop).
#
# Usage:  scripts/revalidate_libs_local.sh [--self-test] [package ...]
#
#   --self-test   inject a compile break and a runtime break into one package and
#                 assert the two are reported DIFFERENTLY.  A sweep that reports
#                 "all green" is worth nothing until its harness is shown able to
#                 go red, and to distinguish the two classes it claims to.
#
# Exit status is 1 if any package COMPILE-BREAKS; a runtime/env failure is reported
# and does not fail the run (the workflow makes the same call, for the same reason).
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
siblings="$(dirname "$root")"
registry="$siblings/loft-registry/index.json"
loft="$root/target/release/loft"
work="${TMPDIR:-/tmp}/loft-revalidate-$$"
self_test=0
want=()
for a in "$@"; do
  case "$a" in
    --self-test) self_test=1 ;;
    -h|--help) sed -n '5,45p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) want+=("$a") ;;
  esac
done

[ -x "$loft" ] || { echo "no release binary at $loft — run: cargo build --release --bin loft" >&2; exit 2; }
[ -f "$registry" ] || { echo "no registry index at $registry — clone loft-lang/loft-registry beside this checkout" >&2; exit 2; }
mkdir -p "$work"
trap 'rm -rf "$work"' EXIT

# The matrix: package -> latest non-yanked version, its repo, its tag, its subpath.
matrix="$work/legs.tsv"
python3 - "$registry" > "$matrix" <<'PY'
import json, re, sys
idx = json.load(open(sys.argv[1]))
def key(v): return [int(x) for x in re.findall(r'\d+', v)]
for name, meta in sorted(idx["packages"].items()):
    yanked = set(meta.get("yanked", []))
    vers = {k: v for k, v in meta.get("versions", {}).items() if k not in yanked}
    if not vers:
        continue
    latest = max(vers, key=key)
    m = re.match(r"https://github\.com/([^/]+/[^/]+)/releases/download/([^/]+)/",
                 vers[latest].get("url", ""))
    if not m:            # non-GitHub / malformed URL — nothing to check out
        continue
    print("\t".join([name, latest, m.group(1), m.group(2),
                     vers[latest].get("subpath", name)]))
PY

# Extract one package's release tree into $work and echo its package directory.
extract() {                       # extract <repo> <tag> <sub> <dest-name>
  local repo="$1" tag="$2" sub="$3" dest="$4"
  local clone="$siblings/$(basename "$repo")"
  git -C "$clone" rev-parse --verify -q "refs/tags/$tag" >/dev/null 2>&1 || return 1
  rm -rf "${work:?}/$dest"; mkdir -p "$work/$dest"
  git -C "$clone" archive "$tag" | tar -x -C "$work/$dest" 2>/dev/null || return 1
  [ -d "$work/$dest/$sub" ] && echo "$work/$dest/$sub" || echo "$work/$dest"
}

# The workflow's re-classification: 0 = every .loft still compiles, 1 = a real break.
still_compiles() {                # still_compiles <pkg-dir> <log>
  local p="$1" log="$2" rc=0 n=0 f
  # `-type f` is load-bearing.  `loft --dump` WRITES a `tests/.loft` cache directory
  # beside the file it compiles, and the glob `*.loft` matches that NAME.  `find`
  # streams through the process substitution, so the directory this very loop creates
  # can be handed straight back to it; `--dump` then fails on a directory and a
  # runtime failure is reported as a compile break.  Reachable on any package with two
  # or more test files, and the shipped gate had the same line (fixed with it).
  while IFS= read -r f; do
    n=$((n + 1))
    (cd "$p" && "$loft" --dump "$f") >/dev/null 2>>"$log" || rc=1
  done < <(cd "$p" && find tests -type f -name '*.loft' 2>/dev/null)
  [ "$n" -gt 0 ] || return 1      # nothing to reclassify — fail conservatively
  return $rc
}

if [ "$self_test" = 1 ]; then
  read -r n v repo tag sub < <(head -1 "$matrix")
  p="$(extract "$repo" "$tag" "$sub" "selftest")" || { echo "self-test: cannot extract $n@$tag" >&2; exit 2; }
  victim="$(cd "$p" && find tests -type f -name '*.loft' | head -1)"
  [ -n "$victim" ] || { echo "self-test: $n has no test .loft" >&2; exit 2; }
  cp "$p/$victim" "$work/victim.bak"

  printf '\nfn ~~~broken~~~( {\n' >> "$p/$victim"
  if (cd "$p" && LOFT_TIMEOUT=120 "$loft" --interpret --tests tests) >/dev/null 2>&1; then
    echo "self-test FAILED: a syntax error did not fail the suite"; exit 1; fi
  still_compiles "$p" /dev/null && { echo "self-test FAILED: a compile break read as env/deps"; exit 1; }
  echo "self-test: compile break     -> reported as COMPILE-BREAK      ok"

  cp "$work/victim.bak" "$p/$victim"
  printf '\nfn test_injected_runtime_failure() { assert(1 == 2, "injected"); }\n' >> "$p/$victim"
  if (cd "$p" && LOFT_TIMEOUT=120 "$loft" --interpret --tests tests) >/dev/null 2>&1; then
    echo "self-test FAILED: a false assertion did not fail the suite"; exit 1; fi
  still_compiles "$p" /dev/null || { echo "self-test FAILED: a runtime break read as a compile break"; exit 1; }
  echo "self-test: runtime break     -> reported as runtime/env         ok"
  echo "self-test: the two classes are distinguished, on $n@$tag"
  exit 0
fi

printf '%-18s %-9s %s\n' PACKAGE VERSION VERDICT
breaks=0 passed=0 skipped=0 envfail=0
while IFS=$'\t' read -r n v repo tag sub; do
  if [ ${#want[@]} -gt 0 ]; then
    printf '%s\n' "${want[@]}" | grep -qxF "$n" || continue
  fi
  if ! p="$(extract "$repo" "$tag" "$sub" "$n")"; then
    printf '%-18s %-9s %s\n' "$n" "$v" "SKIP (tag $tag not in $(basename "$repo"))"
    skipped=$((skipped + 1)); continue
  fi
  if [ ! -d "$p/tests" ]; then
    printf '%-18s %-9s %s\n' "$n" "$v" "SKIP (no tests/)"; skipped=$((skipped + 1)); continue
  fi
  log="$work/$n.log"
  if (cd "$p" && LOFT_TIMEOUT=240 "$loft" --interpret --tests tests) >"$log" 2>&1; then
    printf '%-18s %-9s %s\n' "$n" "$v" "PASS"; passed=$((passed + 1))
  elif still_compiles "$p" "$log"; then
    printf '%-18s %-9s %s\n' "$n" "$v" "runtime-fail, COMPILES (env/native-deps — VERIFY)"
    envfail=$((envfail + 1))
    grep -aiE 'FAIL |assertion|Error:|panic|expected|got ' "$log" | tail -5 | sed 's/^/    /'
  else
    printf '%-18s %-9s %s\n' "$n" "$v" "*** COMPILE-BREAK ***"
    breaks=$((breaks + 1))
    tail -12 "$log" | sed 's/^/    /'
  fi
done < "$matrix"

echo
echo "$passed pass, $envfail runtime/env, $skipped skipped, $breaks COMPILE-BREAK"
[ "$skipped" -gt 0 ] && echo "a SKIP is not a pass — clone the missing repo beside this one, or fetch its tags"
[ "$breaks" -eq 0 ] || echo "a published library no longer compiles against this loft — the freeze forbids that"
exit $((breaks > 0))
