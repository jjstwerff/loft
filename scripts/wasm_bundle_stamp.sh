#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Print a stamp for the sources the browser bundle (`doc/pkg`) is built from.
#
# `make wasm` writes it to `doc/pkg-src.stamp`, and the two browser tests
# (`engine_host_connector`) recompute it and refuse to run against a bundle whose stamp
# disagrees.  A browser test whose subject is a checked-in artefact otherwise reports on
# whatever was last committed rather than on the tree under test — loft#1189, where the bundle
# was a year old and predated a native the shared client loop calls on every turn.
#
# ⚠ The list is NARROW ON PURPOSE, and that makes the stamp an APPROXIMATION: these are the
# files that decide what the browser kernel and the two fixture pages do, so a change to any of
# them plausibly invalidates the bundle.  A change ANYWHERE ELSE in `src/` can still change the
# bundle without moving the stamp.  The exact stamp is the whole build input, and it was not
# taken because it would redden these tests on every commit that touches `src/` — a tax that
# ends in either a skipped test or a 2 MB binary re-committed several times a day.  What this
# catches is drift at the scale that actually happened.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
{
  cat src/engine_host.rs src/wasm.rs src/native.rs src/compile.rs \
      lib/engine_host/src/engine_host.loft \
      doc/kernel-differential.html doc/kernel-swap.html doc/loft-rt.js
  cat default/*.loft
} | sha256sum | cut -d' ' -f1
