#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# check_bundle_fresh.sh — fail a change-set that edits a committed browser
# bundle's SOURCE without rebuilding + committing the bundle in the same set.
#
# Why this exists: the repo commits generated browser artefacts (the Brick
# Buster --html page; the gallery wasm/js pair). Every time we have shipped a
# broken or stale page it was a PR that changed the source but skipped the
# rebuild step (see doc/claude/GALLERY_CI.md). This gate turns that into a red
# check. It is DETERMINISTIC and needs NO build/toolchain — it compares the PR's
# changed-file set against a declared artefact->source table — so it is a fast
# per-PR gate that can never flake on wasm build non-determinism (unlike a
# byte-diff of a rebuilt bundle).
#
# Scope (deliberate): only artefacts with a NARROW, well-defined source belong
# here — currently the Brick Buster page, whose source is one .loft file. The
# gallery wasm bundle (doc/pkg/*) is the whole compiler compiled to wasm, so its
# "source" is all of src/ and a timestamp/diff gate would fire on every PR; that
# bundle's freshness is covered instead by the gallery job's rebuild + Node
# instantiate. Add a row below only when the mapping stays one-to-few and clear.
#
# This gate catches staleness a PR *introduces*; it does not flag pre-existing
# staleness (that is release-checklist item B — rebuild the committed bundles).
#
# Usage:
#   scripts/check_bundle_fresh.sh [<base-ref>]
# <base-ref> defaults to origin/main. Changed set = `git diff --name-only
# <base>...HEAD` (three-dot: what HEAD added since the merge base).
set -euo pipefail

base="${1:-origin/main}"

# One row per artefact: "<artefact><TAB><source> [<source> ...]".
mapping() {
	printf 'doc/brick-buster.html\ttools/brick-buster/25-brick-buster.loft\n'
}

# Files this change set touches, relative to the merge base with <base>.
if ! changed=$(git diff --name-only "${base}...HEAD" 2>/dev/null); then
	# Base unreachable (shallow clone, detached CI without the base ref, a
	# local branch with no upstream). Do not block on an undecidable input.
	echo "check_bundle_fresh: SKIP — cannot resolve base '${base}' (no merge base)."
	exit 0
fi

is_changed() { printf '%s\n' "$changed" | grep -qxF "$1"; }

fail=0
while IFS=$'\t' read -r artefact sources; do
	[ -z "$artefact" ] && continue
	touched_src=""
	for s in $sources; do
		if is_changed "$s"; then touched_src="$s"; fi
	done
	if [ -n "$touched_src" ] && ! is_changed "$artefact"; then
		echo "::error file=${touched_src}::'${touched_src}' changed in this change set but '${artefact}' was NOT rebuilt. Rebuild the bundle (\`make game\` for Brick Buster, \`make gallery\` for the gallery) and commit '${artefact}' in the same change so the deployed website is not stale."
		fail=1
	fi
done < <(mapping)

if [ "$fail" -ne 0 ]; then
	echo "check_bundle_fresh: FAIL — a committed browser bundle's source changed without a rebuild."
	exit 1
fi
echo "check_bundle_fresh: OK — every touched bundle source was rebuilt in this change set."
