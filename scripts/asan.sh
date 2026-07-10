#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Run the AddressSanitizer suite locally in an ISOLATED target dir.
#
# WHY: ASan needs a nightly toolchain (`-Zsanitizer=address`).  A nightly build
# run into the shared `target/` compiles `libloft_ffi` (and everything else) with
# nightly rustc; a later STABLE build/test then hits E0514 ("incompatible version
# of rustc") because the native test harness links that rlib directly and its
# mtime looks fresh — neither cargo nor mtime rebuilds it.  Keeping nightly
# artifacts in `target/asan/` means the stable `target/` is never polluted, so
# `find_problems.sh` / native tests always reuse clean stable artifacts.  (CI's
# `asan` job in .github/workflows/miri.yml is safe already — it runs on a fresh,
# separately-cached checkout; this wrapper gives local runs the same isolation.)
#
# Mirrors that CI job's flags.  Extra args pass through to nextest, e.g.
#   ./scripts/asan.sh -E 'test(store_)'
#
# If a stray nightly build already polluted `target/`, find_problems.sh's
# ffi_toolchain_guard self-heals the stale rlib on its next run.
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

if ! rustc +nightly --version >/dev/null 2>&1; then
  echo "asan.sh: a nightly toolchain is required (rustup toolchain install nightly)" >&2
  exit 1
fi

exec env \
  CARGO_TARGET_DIR="$REPO_ROOT/target/asan" \
  RUSTFLAGS='-Zsanitizer=address' \
  ASAN_OPTIONS='detect_leaks=0' \
  cargo +nightly nextest run --profile ci --release "$@"
