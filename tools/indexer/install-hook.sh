#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# tools/indexer/install-hook.sh — install the tracker-index
# pre-commit hook (plan-37 phase 02).
#
# Idempotent.  Safe to re-run: detects existing snippet by
# marker, replaces in place if present, appends if not.
# Preserves any unrelated content already in the hook.

set -euo pipefail
cd "$(dirname "$0")/../.."

HOOK=".git/hooks/pre-commit"
MARKER="# >>> plan-37 tracker-index >>>"
END_MARKER="# <<< plan-37 tracker-index <<<"

read -r -d '' SNIPPET <<'EOF' || true
# >>> plan-37 tracker-index >>>
# Refresh index/tags.json on staged doc/code changes.  See
# doc/claude/plans/37-tracker-index/02-auto-refresh.md.
# Uses `make index` which invokes the loft-native scanner
# (post-@PLAN37 phase 07 H cutover) — runs across Linux /
# macOS / Windows MSYS bash identically.
if git diff --cached --name-only | grep -qE '\.(md|rs|loft|toml|py|sh)$'; then
  if [ -x ./target/release/loft ]; then
    make -s index >/dev/null 2>&1 || \
      echo "warning: make index failed; index/tags.json may be stale" >&2
  fi
fi
# <<< plan-37 tracker-index <<<
EOF

mkdir -p .git/hooks

if [ ! -f "$HOOK" ]; then
  # No hook exists — write a fresh one with our snippet.
  {
    echo "#!/usr/bin/env bash"
    echo ""
    echo "$SNIPPET"
  } > "$HOOK"
  chmod +x "$HOOK"
  echo "installed $HOOK (new file)"
  exit 0
fi

# Existing hook — check for our marker.
if grep -qF "$MARKER" "$HOOK"; then
  # Replace the existing block in place.
  awk -v marker="$MARKER" -v end="$END_MARKER" -v new="$SNIPPET" '
    BEGIN { skip = 0 }
    $0 ~ marker { print new; skip = 1; next }
    skip && $0 ~ end { skip = 0; next }
    !skip { print }
  ' "$HOOK" > "$HOOK.tmp"
  mv "$HOOK.tmp" "$HOOK"
  chmod +x "$HOOK"
  echo "updated $HOOK (existing snippet replaced)"
else
  # Append our snippet at the end.
  echo "" >> "$HOOK"
  echo "$SNIPPET" >> "$HOOK"
  chmod +x "$HOOK"
  echo "appended snippet to existing $HOOK"
fi
