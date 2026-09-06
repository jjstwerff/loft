#!/bin/bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN154 — build the stack shadow ON A CONTROL TREE.
#
# Usage:  bash doc/claude/plans/154-stack-shadow/shadow-control.sh <ref> [outdir]
# Prints: the path of the built binary and the stdlib `--path` to run it with.
#
# `make falsify` builds the control tree AS IT WAS, which is right for a guard and useless
# for a detector: the detector does not exist there.  So the shadow's commits are
# cherry-picked onto the control before it is built, and the guard is then run under a
# binary that is the control PLUS the instrument.
#
# The cherry-pick conflicts in the same three places every time — the module list, the exit
# report, and the dispatch loop's hoist — because those are one-line insertions into files
# that have moved a lot since.  Everything else in the shadow lives in `store.rs`,
# `stack_verify.rs` and the accessors, which are stable.  So: take the control's side of
# every conflict, then re-insert the three anchors idempotently.  A conflict the resolution
# does not know about therefore comes out as a MISSING piece and fails the build, rather
# than as a silently half-applied detector.
set -uo pipefail
REF="${1:?usage: shadow-control.sh <ref> [outdir]}"
ROOT=$(git rev-parse --show-toplevel)
SHA=$(git rev-parse --short "$REF") || exit 2
OUT="${2:-${TMPDIR:-/tmp}/loft-shadow}"
WT="$OUT/$SHA"; TGT="$OUT/$SHA-target"

# The shadow's own commits, newest last.  Found by their subject so a rebase does not
# silently pin an old copy.
PHASES=$(git log --format=%H --grep='^@PLN154 phase [12]:' --reverse HEAD)
[ -n "$PHASES" ] || { echo "no @PLN154 phase 1/2 commits in this history" >&2; exit 2; }

if [ ! -x "$TGT/release/loft" ]; then
  rm -rf "$WT"
  git worktree add --detach "$WT" "$SHA" >/dev/null 2>&1 || exit 2
  (
    cd "$WT" || exit 2
    # ONE COMMIT AT A TIME, resolving after each: `cherry-pick --continue` finishes the
    # commit it is on and stops when the NEXT one conflicts, so a single continue lands half
    # the detector — which then reports in phase 1's words and reads like a phase-2 miss.
    for c in $PHASES; do
      git cherry-pick "$c" >/dev/null 2>&1
      python3 - <<'PY'
import re, glob, sys
# Take the CONTROL's side of every conflict: the incoming side drags in unrelated code from
# commits between the control and here, which will not compile on this tree.
for p in glob.glob('src/**/*.rs', recursive=True):
    s = open(p).read()
    if '<<<<<<<' not in s:
        continue
    open(p, 'w').write(
        re.sub(r'<<<<<<< HEAD\n(.*?)=======\n.*?>>>>>>> [^\n]*\n', r'\1', s, flags=re.S))

def insert_after(path, anchor, text):
    """Idempotent: no-op when `text` is already there, loud when the anchor is gone."""
    s = open(path).read()
    if text.strip() in s:
        return
    if s.count(anchor) != 1:
        sys.exit(f'{path}: anchor not unique ({s.count(anchor)}): {anchor!r}')
    open(path, 'w').write(s.replace(anchor, anchor + text))

insert_after('src/lib.rs', 'pub mod native;\n', 'pub mod stack_verify;\n')
insert_after('src/main.rs', '    state.report_profile(&p.data);\n',
             '    if loft::stack_verify::enabled() {\n        loft::stack_verify::report();\n    }\n')
insert_after('src/state/mod.rs', '        let mut last_allocs = self.database.stores_allocated;\n',
             '        let verify_on = crate::stack_verify::enabled();\n')
PY
      [ $? -eq 0 ] || exit 2
      git add -A >/dev/null
      git -c core.editor=true cherry-pick --continue >/dev/null 2>&1
    done
    grep -q 'SlotState::Mismatch' src/store.rs || { echo "phase 2 did not land" >&2; exit 2; }
    # `grep` answering "no errors" must not be read as a failed build: keep cargo's own
    # status, and print the errors only when there are some.
    log="$TGT/build.log"
    mkdir -p "$TGT"
    if ! cargo build --release --bin loft --target-dir "$TGT" > "$log" 2>&1; then
      grep -E '^error' -A8 "$log" >&2
      exit 2
    fi
  ) || { echo "control build failed: $WT" >&2; exit 2; }
fi
echo "$TGT/release/loft"
echo "$WT/"
