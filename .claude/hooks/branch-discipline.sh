#!/usr/bin/env bash
# Branch-discipline reminder (DEVELOPMENT.md § "Stay close to main — rebase rigorously").
# Fires before a Bash branch-CREATION command and injects a non-blocking reminder so the
# one-branch + rebase-often discipline is surfaced at the moment of the action, not just
# in a doc the agent read at session start.  Non-blocking: it never stops a branch the
# user explicitly asked for — it only makes the agent reconsider.
cmd=$(jq -r '.tool_input.command // empty' 2>/dev/null)
if printf '%s' "$cmd" | grep -qE 'git +(checkout +-b|switch +-c|branch +[A-Za-z]|worktree +add)'; then
  printf '%s' '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"BRANCH DISCIPLINE (DEVELOPMENT.md § rebase rigorously): the default is ONE branch held close to main, rebased on origin/main OFTEN — build everything there. Create a branch ONLY if the user EXPLICITLY asked for one. If the current branch has diverged from main, rebase FIRST. Keep `git diff main` usable as your refactor compass; do not spin off per-topic branches."}}'
fi
exit 0
