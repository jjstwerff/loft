#!/bin/sh
# Check the links in the repo's user-facing Markdown.
#
# Two kinds, checked differently:
#   RELATIVE links (./doc/x.md, ../LICENSE)  — the file must exist.  Free, exact,
#                                              and the class that rots silently
#                                              when a file moves.
#   EXTERNAL links (http/https)              — one HEAD request each.  Only with
#                                              `--external`, because it makes the
#                                              result depend on other people's
#                                              uptime; keep it OUT of `make ci`
#                                              (nightly is the right home) so a
#                                              third party being down cannot
#                                              block a merge.
#
# Usage:
#   scripts/linkcheck.sh              # relative links only (fast, offline, CI-safe)
#   scripts/linkcheck.sh --external   # also verify every http(s) link
#
# Exit 0 = every checked link resolves; 1 = at least one is broken.

set -u
cd "$(dirname "$0")/.."

EXTERNAL=0
[ "${1:-}" = "--external" ] && EXTERNAL=1

# User-facing Markdown: the repo root + the published docs — what a visitor
# actually reads.  Out of scope: `doc/claude/` (agent-facing, and it has its own
# drift checker, scripts/check_doc_drift.sh) and `tests/fixtures/` (fixtures
# contain deliberately dead links as test input).
FILES=$(git ls-files '*.md' \
        | grep -vE '^(doc/claude/|tests/|target/)' \
        | sort)

bad=0
checked=0

for f in $FILES; do
  dir=$(dirname "$f")
  # Pull the target out of every `[text](target)`.  Skip anchors (#x), mailto:,
  # and template placeholders (<...>, {...}).
  # Strip inline code spans first: prose like `OPERATORS[opcode](state)` is not
  # a link, and matching it produces a phantom "broken" target.
  targets=$(sed 's/`[^`]*`//g' "$f" 2>/dev/null \
            | grep -oE '\]\([^)]+\)' \
            | sed -E 's/^\]\(//; s/\)$//' \
            | sed -E 's/[[:space:]]+".*"$//' \
            | grep -vE '^(#|mailto:|<|\{)' || true)
  for t in $targets; do
    case "$t" in
      http://*|https://*)
        [ "$EXTERNAL" -eq 1 ] || continue
        checked=$((checked + 1))
        code=$(curl -s -L -o /dev/null -w '%{http_code}' --max-time 20 "$t" 2>/dev/null || echo 000)
        # 403/429 are bot-blocking, not rot — do not fail the run on them.
        case "$code" in
          2*|3*|403|429) ;;
          *) echo "BROKEN (http $code)  $f -> $t"; bad=$((bad + 1)) ;;
        esac
        ;;
      *)
        # Relative link: strip any #anchor, then resolve against the file's dir.
        path=${t%%#*}
        [ -z "$path" ] && continue
        # Generated on demand and git-ignored by design (`make libcatalogue`),
        # so its absence from a clean checkout is correct, not rot.
        case "$path" in *doc/claude/LIBRARIES.md) continue ;; esac
        checked=$((checked + 1))
        if [ ! -e "$dir/$path" ] && [ ! -e "$path" ]; then
          echo "BROKEN (missing)  $f -> $t"
          bad=$((bad + 1))
        fi
        ;;
    esac
  done
done

if [ "$bad" -gt 0 ]; then
  echo
  echo "linkcheck: $bad broken of $checked checked"
  exit 1
fi
echo "linkcheck: $checked links OK"
