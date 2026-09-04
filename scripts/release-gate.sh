#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Run every nightly against THIS commit in one deliberate CI run, and wait for its verdict.
#
# Dispatches `.github/workflows/release-gate.yml` on the current branch and watches the
# run it started.  The branch must be PUSHED and HEAD must be what the remote has: a
# dispatch runs the commit GitHub holds, and `make release-checklist` accepts only a run
# whose commit is HEAD's — a run for any other commit is not evidence for this one.
#
#   scripts/release-gate.sh              # dispatch, then wait (~60–90 min)
#   scripts/release-gate.sh --no-wait    # dispatch and return; watch with `gh run watch`
#
# Exit status is the gate's verdict when waiting (0 green, 1 red), so it composes.
# It never tags, drafts or publishes anything (RELEASE.md § No Automated Releases).

set -euo pipefail

WORKFLOW=release-gate.yml
wait_for_run=1
for arg in "$@"; do
  case "$arg" in
    --no-wait) wait_for_run=0 ;;
    -h|--help) sed -n '5,17p' "$0"; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

branch=$(git branch --show-current)
if [ -z "$branch" ]; then
  echo "release-gate: detached HEAD — check out the branch (or tag) you mean to gate" >&2
  exit 2
fi
sha=$(git rev-parse HEAD)

if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "release-gate: the tree has uncommitted changes; the gate would test $branch as PUSHED, not this tree" >&2
  echo "release-gate: commit (and push) first" >&2
  exit 2
fi

git fetch --quiet origin "$branch" 2>/dev/null || {
  echo "release-gate: origin has no branch '$branch' — push it first" >&2
  exit 2
}
remote=$(git rev-parse "origin/$branch")
if [ "$remote" != "$sha" ]; then
  echo "release-gate: HEAD ${sha:0:12} is not what origin/$branch holds (${remote:0:12}) — push first" >&2
  exit 2
fi

# Runs created before this instant belong to earlier dispatches.
since=$(date -u +%Y-%m-%dT%H:%M:%SZ)
gh workflow run "$WORKFLOW" --ref "$branch"
echo "release-gate: dispatched $WORKFLOW on $branch @ ${sha:0:12}"

# `gh workflow run` returns nothing to identify the run it queued; find it by commit,
# newest first, created after the dispatch.  Queueing usually takes seconds; allow two
# minutes before calling the dispatch lost.
run_id=""
for _ in $(seq 1 24); do
  run_id=$(gh run list --workflow "$WORKFLOW" --commit "$sha" --limit 5 \
      --json databaseId,createdAt \
      --jq --arg since "$since" '[.[] | select(.createdAt >= $since)] | sort_by(.createdAt) | last | .databaseId // empty')
  [ -n "$run_id" ] && break
  sleep 5
done
if [ -z "$run_id" ]; then
  echo "release-gate: no run appeared for ${sha:0:12} within two minutes — check 'gh run list --workflow $WORKFLOW'" >&2
  exit 1
fi
url="https://github.com/loft-lang/loft/actions/runs/$run_id"
echo "release-gate: run $run_id — $url"

if [ "$wait_for_run" = 0 ]; then
  echo "release-gate: not waiting; 'gh run watch $run_id --exit-status' follows it"
  exit 0
fi

# `--exit-status` makes this exit non-zero on a red run, which is the verdict.
if gh run watch "$run_id" --exit-status --interval 30; then
  echo "release-gate: GREEN for ${sha:0:12} — 'make release-checklist' reads this run"
  exit 0
fi
echo "release-gate: RED for ${sha:0:12} — the 'verdict' job summary at $url names the legs" >&2
exit 1
