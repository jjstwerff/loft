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
loft-libs-graphics  graphics-v0.1.1            graphics
loft-libs-graphics  imaging-v0.1.0           imaging
loft-libs-net       game_protocol-v0.1.2     game_protocol
loft-libs-net       web-v0.1.1               web
loft-libs-world     hex_world-v0.1.0         hex_world
loft-libs-game      time-v0.1.0              time
EOF
)

# Files that intentionally diverge from the pinned upstream tag and
# must NOT be flagged as drift.  Format: `<pkg>/<relpath>`.
#
# - hex_world/tests/hex_world.loft — @P387 local patch: the upstream
#   tag hard-codes `/tmp/...` save paths, which don't exist on Windows
#   CI.  The fixture rewrites them cwd-relative (and deletes the save
#   artifact afterwards) so the loft test suite passes cross-platform.
#   Re-syncing would clobber the patch; the divergence is deliberate.
#   When loft-libs-world ships a tag carrying this fix, drop this line
#   and bump PINNED_REFS instead.
#
# - gridmesh/README.md, hex_world/README.md, hex_world/src/hex_world.loft
#   — @PLN6 tracker renumber: the in-repo renumber of plan
#   `36-audience-generative-art` → `6-audience-generative-art` (old tag
#   `@PLAN36` is a dead illustrative example here, not a live ref) rewrote  <!--noindex-->
#   these doc-comments/links in the
#   committed fixtures, but the pinned upstream tags still carry the old
#   names.  Doc-only divergence (no source/test logic differs).  When the
#   chunk repos ship tags carrying the renumber, drop these lines and bump
#   PINNED_REFS instead.
#
# - shapes/README.md, imaging/README.md, web/README.md — org move
#   (MOVING.md): the in-repo rewrite of `github.com/jjstwerff/loft` →
#   `github.com/loft-lang/loft` updated one @PLAN12 link in each committed
#   fixture, but the pinned upstream tags still carry the old org URL.
#   Doc-only divergence (a single README link per package; no source/test
#   logic differs).  When the chunk repos ship tags carrying the new org
#   URL, drop these lines and bump PINNED_REFS instead.
#
# - imaging/native/Cargo.toml — builds against the IN-REPO loft-ffi crates
#   via [patch.crates-io] (plan-25 F3: generate_register_from_loft_with_bridges
#   is not yet in the published loft-ffi-build 0.2.0; the Cargo.toml's own
#   comment carries the story).  The next loft-ffi-build publish lifts the
#   patch — drop this line then.
#
# - imaging/tests/14-image.loft, imaging/tests/15-regression.loft — local
#   path-anchoring fix: the upstream tag prefixes test data paths with `tests/`,
#   which assumed a package-root cwd, but `loft test` anchors relative paths at
#   the test file's own dir (source_dir), so the prefix is dropped to the bare
#   name.  Re-syncing would clobber the fix.  Drop these lines when
#   loft-libs-graphics ships an imaging tag carrying the corrected paths.
#
# - imaging/src/imaging.loft — `not null` (dense-layout) annotation required by
#   the nullable-by-default gate flip (@PLN25): the upstream tag declares the
#   pixel buffer as `data: vector<Pixel>`, but with nullable-by-default ON that
#   would wrap each element and change the stride, crashing the `#native`
#   `n_load_png`/`n_save_png` FFI byte-copy (which assumes 3 contiguous bytes
#   per Pixel).  The fixture pins `vector<Pixel not null>` to keep the dense
#   element layout the extension assumes.  Drop this line when loft-libs-graphics
#   ships an imaging tag carrying the `not null` annotation.
#
# - gridmesh/src/gridmesh.loft, hex_world/src/hex_world.loft — @PLN25 DN4
#   narrowing-cast enforcement: with DN4 ON (`(N-Cast)`) a narrowing cast of a
#   not-provably-fit value (`kind as u8`, `inew_age as u16`, the loop-bounded
#   byte writes) is a compile error.  The fixtures mask the operand
#   (`(kind & 255) as u8`, …) so the value is provably in range and the dense
#   byte layout is preserved — behaviour-identical for the in-range data these
#   tests use.  Drop these lines when loft-libs-graphics / loft-libs-world ship
#   tags carrying the masks and bump PINNED_REFS instead.  (Same shape as the
#   imaging `not null` patch above — a gate flip the upstream tag predates.)
#
# - web/src/web.loft — @PLN25 DN1 (non-null default): `try_recv` genuinely
#   returns null on no-frame, so under DN1 its return type must be `text?`;
#   the pinned web-v0.1.1 tag predates the flip (`-> text` + `return null`
#   rejects).  Drop this line when loft-libs-net ships the `text?` migration
#   (the web/server republish) and bump PINNED_REFS instead.
#
# - time/src/time.loft — @PLN25 DN1: `parse`/`combine` return null on a bad
#   time string, so their return types must be `integer?`; the pinned
#   time-v0.1.0 tag predates the flip.  Drop when loft-libs-game ships the
#   migration and PINNED_REFS bumps.
LOCAL_PATCHES=$(cat <<'EOF'
imaging/native/Cargo.toml
imaging/src/imaging.loft
imaging/tests/14-image.loft
imaging/tests/15-regression.loft
hex_world/tests/hex_world.loft
hex_world/README.md
hex_world/src/hex_world.loft
gridmesh/src/gridmesh.loft
gridmesh/README.md
shapes/README.md
imaging/README.md
web/README.md
web/src/web.loft
time/src/time.loft
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
            # Exclude build artifacts the repo itself gitignores, so they can
            # NEVER appear in a committed fixture even though upstream tags
            # sometimes commit them — otherwise the diff false-flags an
            # unfixable "drift":
            #   target      — native build output (`rm`d on sync below)
            #   Cargo.lock  — gitignored repo-wide (.gitignore)
            #   .loft       — loft per-source cache dir, gitignored
            diff_out="$(diff -qr -x target -x Cargo.lock -x .loft "$src" "$dest" 2>&1 || true)"
            # Drop any diff line touching a documented LOCAL_PATCHES file for
            # this pkg (intentional, must-keep divergence from the tag).
            while IFS= read -r patch; do
                [[ -z "$patch" || "$patch" != "$pkg/"* ]] && continue
                rel="${patch#"$pkg"/}"
                diff_out="$(grep -vF -- "$rel" <<< "$diff_out" || true)"
            done <<< "$LOCAL_PATCHES"
            if [[ -n "${diff_out//[[:space:]]/}" ]]; then
                echo "[check] DRIFT: tests/fixtures/libs/$pkg vs $chunk@$ref"
                drift=1
            fi
        else
            rm -rf "$dest"
            cp -r "$src" "$dest"
            # Strip artifacts the repo gitignores — fixtures are source-only.
            rm -rf "$dest/native/target" 2>/dev/null || true
            find "$dest" -name Cargo.lock -type f -delete 2>/dev/null || true
            find "$dest" -name .loft -type d -prune -exec rm -rf {} + 2>/dev/null || true
            # Preserve documented LOCAL_PATCHES (e.g. @P387) — restore the
            # committed version so a routine re-sync doesn't silently revert
            # an intentional, cross-platform-required divergence.  To take the
            # upstream version instead, drop the file from LOCAL_PATCHES first.
            while IFS= read -r patch; do
                [[ -z "$patch" || "$patch" != "$pkg/"* ]] && continue
                git -C "$REPO_ROOT" checkout -- "tests/fixtures/libs/$patch" 2>/dev/null || true
            done <<< "$LOCAL_PATCHES"
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
