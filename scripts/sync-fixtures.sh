#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLAN12 Phase 6.12 — refresh tests/fixtures/libs/ from canonical
# chunk repos.  Run when a chunk ships a new version that the
# loft compiler tests should track (or when bringing a new fixture
# online for the first time).
#
# Why fixtures and not registry-cached installs:
# - Zero network during `cargo test` once committed.
# - Reproducible across machines + CI.
# - Survives Stage B's `lib/<pkg>/` removal — tests don't depend
#   on monorepo lib dirs that no longer exist.
# - A pinned tag is committed; CI can verify fixture-vs-tag.
#
# Edit PINNED_REFS below when a chunk ships a new version that
# we want the loft compiler tests to track.  The fixture is the
# SNAPSHOT, not the latest — intentional, so a fixture-affecting
# library change is a deliberate, reviewable commit, not a
# silent drift.
#
# Usage:
#   scripts/sync-fixtures.sh           # refresh all fixtures
#   scripts/sync-fixtures.sh --check   # verify fixtures match
#                                      # PINNED_REFS without writing

set -euo pipefail

CHECK_ONLY=0
if [[ "${1-}" == "--check" ]]; then
    CHECK_ONLY=1
fi

# Pinned chunk-repo refs.  Each line: `<chunk> <ref> <pkg1,pkg2,...>`.
# Add web/server/imaging/etc. when their fixtures become load-bearing
# (i.e., when Stage B for that chunk is close enough that monorepo
# `lib/<pkg>/` is going away).
#
# Current population (2026-05-31): pure-loft packages only — the
# native-cdylib packages (crypto / random / web / server /
# imaging) can stay in the monorepo's `lib/` until Stage B for
# their chunk runs.  Adding a native-cdylib fixture is fine but
# requires the test workflow to be able to compile the native
# crate on demand (cargo network access), which defeats the
# "zero network during cargo test" goal.
PINNED_REFS=$(cat <<'EOF'
loft-libs-core      arguments-v0.1.1         arguments
loft-libs-graphics  shapes-v0.2.0            shapes
loft-libs-graphics  gridmesh-v0.1.1          gridmesh
loft-libs-graphics  graphics-v0.1.0          graphics
loft-libs-graphics  imaging-v0.1.0           imaging
loft-libs-net       game_protocol-v0.1.1     game_protocol
loft-libs-net       web-v0.1.1               web
loft-libs-world     hex_world-v0.1.0         hex_world
loft-libs-game      time-v0.1.0              time
EOF
)

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_ROOT="$REPO_ROOT/tests/fixtures/libs"
TMPDIR_ROOT="$(mktemp -d -t loft-sync-fixtures.XXXXXX)"
trap 'rm -rf "$TMPDIR_ROOT"' EXIT

mkdir -p "$FIXTURE_ROOT"

drift=0

while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    read -ra parts <<< "$line"
    chunk="${parts[0]}"
    ref="${parts[1]}"
    pkgs_csv="${parts[2]}"

    # Clone per-(chunk, ref) — different tags in the same chunk
    # may point at commits that lack one or more of the other tags'
    # packages (e.g. graphics-v0.1.0's commit doesn't yet have an
    # imaging/ dir; shapes-v0.2.0's commit lacks both).  Key by
    # ref so each line picks up its own snapshot.
    target_dir="$TMPDIR_ROOT/$chunk-$ref"
    if [[ ! -d "$target_dir" ]]; then
        echo "[sync] cloning $chunk @ $ref"
        # --filter=blob:none is a partial clone — fast, no LFS
        # blobs we don't need; we'll only read the source.
        git clone --quiet --depth=1 --branch "$ref" \
            "https://github.com/loft-lang/$chunk.git" "$target_dir" \
            >/dev/null 2>&1 || {
                echo "[sync] FAILED clone $chunk @ $ref" >&2
                exit 1
            }
    fi

    IFS=',' read -ra pkgs <<< "$pkgs_csv"
    for pkg in "${pkgs[@]}"; do
        src="$target_dir/$pkg"
        if [[ ! -d "$src" ]]; then
            echo "[sync] FAILED: $chunk has no $pkg dir at $ref" >&2
            exit 1
        fi
        dest="$FIXTURE_ROOT/$pkg"
        if [[ $CHECK_ONLY -eq 1 ]]; then
            # Exclude `target/` — the sync strips `native/target` from the
            # fixture (build artifacts), so a locally-built native fixture would
            # otherwise false-flag as drift against the (artifact-free) clone.
            if ! diff -qr -x target "$src" "$dest" >/dev/null 2>&1; then
                echo "[check] DRIFT: tests/fixtures/libs/$pkg vs $chunk@$ref"
                drift=1
            fi
        else
            rm -rf "$dest"
            cp -r "$src" "$dest"
            # Strip the chunk-repo's own native build artifacts —
            # the fixtures are source-only.
            rm -rf "$dest/native/target" 2>/dev/null || true
        fi
    done
done <<< "$PINNED_REFS"

if [[ $CHECK_ONLY -eq 1 ]]; then
    if [[ $drift -ne 0 ]]; then
        echo
        echo "[check] fixtures are out of sync with PINNED_REFS."
        echo "  Run \`scripts/sync-fixtures.sh\` to refresh,"
        echo "  or update PINNED_REFS in this script if the drift is intentional."
        exit 1
    fi
    echo "[check] all fixtures match PINNED_REFS"
else
    echo "[sync] all fixtures refreshed"
    echo "  Review the diff (\`git status tests/fixtures/libs/\`) and commit."
fi
