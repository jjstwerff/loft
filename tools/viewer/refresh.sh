#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# tools/viewer/refresh.sh — dump git state for loft-view to consume.
#
# Phase 00 stub.  Filled in by plan-35 phase 04 (git state via
# wrapper script).  See doc/claude/plans/35-branch-review-viewer/
# 04-git-state-wrapper.md.
#
# When implemented, this script will dump:
#   tools/viewer/state/branch.json       — branch + ahead/behind
#   tools/viewer/state/changed.json      — git diff --name-status main...HEAD
#   tools/viewer/state/commits.json      — git log --oneline -20
#   tools/viewer/state/uncommitted.json  — git status --short
#   tools/viewer/state/diffs/<safe>.diff — per-file diffs vs main
#   tools/viewer/state/commits/<sha>.diff — per-commit diffs

set -euo pipefail
echo "tools/viewer/refresh.sh: phase 00 stub (filled by phase 04)"
