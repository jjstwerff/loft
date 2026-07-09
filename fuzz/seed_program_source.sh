#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN53 F1 — seed the `program_source` fuzz corpus from the repo's own .loft
# files (the ~1300 test/doc/example programs). The corpus dir is gitignored, so
# this repopulates it on demand rather than committing copies of in-tree files.
#
#   ./fuzz/seed_program_source.sh
#   cargo +nightly fuzz run program_source
set -euo pipefail
cd "$(dirname "$0")/.."
dst="fuzz/corpus/program_source"
mkdir -p "$dst"
n=0
while IFS= read -r -d '' f; do
  # Flatten to a unique name — basenames collide across directories.
  cp "$f" "$dst/$(printf '%05d' "$n")_$(basename "$f")"
  n=$((n + 1))
done < <(find tests doc examples lib -type f -name '*.loft' -print0)
echo "seeded $n .loft files into $dst"
