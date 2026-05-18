<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 02 — Auto-refresh on commit

**Status:** **Shipped 2026-05-13.**

## What actually shipped

`tools/indexer/install-hook.sh` (~50 lines) writes a
marker-bracketed snippet into `.git/hooks/pre-commit`.  Three
install paths handled:

1. No existing hook → create with shebang + snippet.
2. Existing hook, snippet not present → append.
3. Existing hook, snippet present → replace in place
   (using awk + the `# >>> plan-37 tracker-index >>>` …
   `# <<< plan-37 tracker-index <<<` markers).

All three idempotent — re-running the installer doesn't
double-install or churn the file beyond the snippet block.

The snippet itself:

```bash
# >>> plan-37 tracker-index >>>
# Refresh index/tags.json on staged doc/code changes.
if git diff --cached --name-only | grep -qE '\.(md|rs|loft|toml|py|sh)$'; then
  if [ -x ./tools/indexer/scan.sh ]; then
    ./tools/indexer/scan.sh >/dev/null 2>&1 || \
      echo "warning: tools/indexer/scan.sh failed; index/tags.json may be stale" >&2
  fi
fi
# <<< plan-37 tracker-index <<<
```

Only fires when an indexed file (`.md`, `.rs`, `.loft`,
`.toml`, `.py`, `.sh`) is staged.  Failures print a warning
to stderr but don't block the commit (broken hooks erode
trust faster than stale data).

### Wiring

- `Makefile` adds `index-install-hook:` target.
- `DEBUG.md` adds a § "Tracker-tag indexer" section under
  the open-work section that documents the install + usage.

## Goal

Keep `index/tags.json` fresh without the user remembering to
run `make index`.  A pre-commit hook runs the scanner when
any indexed file (`.md`, `.rs`, `.loft`, `.toml`, `.py`,
`.sh`) is staged for commit.

## What ships

### `tools/indexer/install-hook.sh`

```bash
#!/usr/bin/env bash
# tools/indexer/install-hook.sh — install the pre-commit hook.
# Run once per fresh checkout.
set -euo pipefail
cd "$(dirname "$0")/../.."

HOOK=".git/hooks/pre-commit"
mkdir -p .git/hooks

# If a pre-commit hook already exists, append our line; else create.
SNIPPET='# Plan-37 tracker-index — refresh index/tags.json on staged doc/code changes.
if git diff --cached --name-only | grep -qE "\.(md|rs|loft|toml|py|sh)$"; then
  ./tools/indexer/scan.sh >/dev/null
fi'

if [ -f "$HOOK" ] && grep -q "Plan-37 tracker-index" "$HOOK"; then
  echo "pre-commit hook already installed"
elif [ -f "$HOOK" ]; then
  echo "" >> "$HOOK"
  echo "$SNIPPET" >> "$HOOK"
  echo "appended tracker-index hook to existing $HOOK"
else
  { echo "#!/usr/bin/env bash"; echo ""; echo "$SNIPPET"; } > "$HOOK"
  chmod +x "$HOOK"
  echo "installed $HOOK"
fi
```

### `Makefile` target

```make
index-install-hook:
	@./tools/indexer/install-hook.sh
```

### DEBUG.md addition

A short subsection under § Debugging utilities:

```markdown
### Tracker-index hook

After fresh checkout, install the pre-commit hook so
`index/tags.json` stays fresh on every commit:

    make index-install-hook

The hook adds ~1 sec to commits that touch indexed files;
no overhead for commits that only touch ignored paths.
```

## Acceptance

- `make index-install-hook` is idempotent (re-running
  doesn't double-append).
- A `.md` change auto-refreshes `index/tags.json` on commit
  (verified by `git status index/tags.json` after committing
  an unrelated `.md` edit — file should be modified or
  un-modified depending on whether the edit added/removed
  tags, never stale).
- A non-indexed change (e.g., editing only `.gitignore`)
  doesn't run the scanner.
- The hook doesn't block commits if the scanner fails
  (warning is acceptable; broken hooks lose user trust fast).

## Risks

| Risk | Mitigation |
|---|---|
| Hook overhead noticeable on low-end machines | Phase 00 already at 0.85 sec; doesn't grow much per file edit |
| Hook silently fails on machines without `jq` | The scanner detects + reports missing `jq`; let that error surface |
| Hook overwrites user's existing pre-commit content | Detect existing hook, append a tagged section instead of replacing |

## Cross-references

- [Phase 00 — scanner](00-convention-and-scanner.md)
- [Phase 03 — broken validator](03-broken-validator.md) — the hook also runs the validator if installed
