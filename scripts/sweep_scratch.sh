#!/bin/bash
# sweep_scratch.sh — reclaim loft's own scratch from the directories given.
#
#   scripts/sweep_scratch.sh [--sessions] [--days N] <dir>...
#
# What loft writes to a temp directory, and the rule that removes each (TESTING.md § Scratch
# hygiene):
#
#   loft_native_bin_<pid>, loft_native_<pid>.rs   a `--native` run's artefacts; a run that ends
#                                                  normally removes them, one killed from outside
#                                                  cannot — removed when <pid> is DEAD
#   loft_native_<stem>*, loft_test_native_<stem>*  the native suites' per-file caches —
#                                                  removed when older than --days (default 1)
#   loft_html_*, loft_p*, loft_rebuild_*, loft-*   the html/probe/rebuild/serve scratch of the
#                                                  test suites — removed when older than --days
#                                                  (`loft-falsify` excepted: it prunes itself)
#   <any>/.loft/cache/<entry>                      the program cache a test wrote beside its
#                                                  probe (every probe has a fresh name, so the
#                                                  cache only grows) — entries older than --days
#   --sessions: claude-<uid>/<project>/<session>   the agent harness's per-session scratch, next
#                                                  to the directories given — older than 14 days
#
# Never another program's files, never a live process's, never a sibling checkout's gate
# scratch (pass only your own).  Prints one line when something was removed, nothing when
# nothing was.
set -u
days=1; sessions=0; dirs=()
while [ $# -gt 0 ]; do
  case "$1" in
    --sessions) sessions=1;;
    --days) days="$2"; shift;;
    -h|--help) sed -n '2,24p' "$0"; exit 0;;
    *) dirs+=("$1");;
  esac
  shift
done
[ ${#dirs[@]} -gt 0 ] || { echo "usage: $0 [--sessions] [--days N] <dir>..." >&2; exit 2; }
removed=0; bytes=0
gone() { # <path> — remove, counting
  local b
  b=$(du -sb "$1" 2>/dev/null | cut -f1); b=${b:-0}
  rm -rf -- "$1" 2>/dev/null && { removed=$((removed + 1)); bytes=$((bytes + b)); }
}
for d in "${dirs[@]}"; do
  [ -d "$d" ] || continue
  # 1. dead-process native artefacts (the pid is the trailing digit run of the stem)
  # Only the two shapes the runtime writes per process; the test suites name theirs by
  # script stem, and a stem ending in digits is not a pid.
  for f in "$d"/loft_native_bin_* "$d"/loft_native_*.rs; do
    [ -e "$f" ] || continue
    name=${f##*/}
    pid=${name#loft_native_bin_}; [ "$pid" = "$name" ] && { pid=${name#loft_native_}; pid=${pid%.rs}; }
    case "$pid" in ''|*[!0-9]*) continue;; esac
    [ -d "/proc/$pid" ] && continue
    gone "$f"
  done
  # 2. aged scratch of loft's families
  while IFS= read -r f; do gone "$f"; done < <(
    find "$d" -mindepth 1 -maxdepth 1 \( -name 'loft_native_*' -o -name 'loft_test_native_*' \
      -o -name 'loft_html_*' -o -name 'loft_p[0-9]*' -o -name 'loft_rebuild_*' \
      -o \( -name 'loft-*' ! -name 'loft-falsify' \) \) \
      -mtime "+$days" 2>/dev/null)
  # 3. program-cache entries a test wrote beside a probe
  while IFS= read -r f; do gone "$f"; done < <(
    find "$d" -mindepth 4 -maxdepth 4 -path '*/.loft/cache/*' -mtime "+$days" 2>/dev/null)
  # 4. the harness's per-session scratch beside the directory
  if [ "$sessions" = 1 ]; then
    while IFS= read -r f; do gone "$f"; done < <(
      find "$d"/claude-[0-9]* -mindepth 2 -maxdepth 2 -type d -mtime +14 2>/dev/null)
  fi
done
[ "$removed" -gt 0 ] && echo "sweep_scratch: removed $removed entries, $((bytes / 1048576)) MB"
exit 0
