#!/usr/bin/env bash
# Detect UNMERGED branches across the loft-libs-* repos — squash-merge safe.
#
# Why not `git merge-base --is-ancestor` / `ahead_by`?  Squash-merge rewrites a
# branch's commits into ONE new commit on the default branch, so the branch's
# originals are never ancestors — every merged branch then looks "unmerged" by
# commit graph.  Instead we compare CONTENT: for the files a branch changed since
# it forked, is the branch's version already identical to the default branch?  If
# yes (squash-merged, or merged any way), it is merged regardless of SHAs.
set -uo pipefail
REPOS=(assets core docs game graphics net world)
WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT
printf "%-9s %-40s %-16s %s\n" REPO BRANCH PR STATUS
printf '%.0s-' {1..100}; echo
for repo in "${REPOS[@]}"; do
  R="loft-lang/loft-libs-$repo"
  def=$(gh repo view "$R" --json defaultBranchRef --jq '.defaultBranchRef.name' 2>/dev/null) || continue
  git clone --quiet --filter=blob:none "https://github.com/$R.git" "$WORK/$repo" 2>/dev/null || continue
  G=(git -C "$WORK/$repo")
  "${G[@]}" fetch --quiet origin '+refs/heads/*:refs/remotes/origin/*' 2>/dev/null
  for b in $("${G[@]}" for-each-ref --format='%(refname:short)' refs/remotes/origin \
              | sed 's|^origin/||' | grep -vxE "$def|HEAD|origin"); do
    # Files the branch changed since it forked (three-dot: excludes default-ahead-only files).
    mapfile -t files < <("${G[@]}" diff --name-only "origin/$def...origin/$b" 2>/dev/null)
    if [ "${#files[@]}" -eq 0 ] || "${G[@]}" diff --quiet "origin/$def" "origin/$b" -- "${files[@]}" 2>/dev/null; then
      status="merged (content on $def — safe to delete)"
    else
      last=$("${G[@]}" log -1 --format='%s' "origin/$b" 2>/dev/null | cut -c1-46)
      status="UNMERGED (${#files[@]} files) — $last"
    fi
    pr=$(gh pr list --repo "$R" --head "$b" --state all --json number,state,isDraft \
          --jq 'first(.[]) | "#\(.number) \(.state|ascii_downcase)\(if .isDraft then "/draft" else "" end)" // "no-PR"' 2>/dev/null)
    printf "%-9s %-40s %-16s %s\n" "$repo" "$b" "$pr" "$status"
  done
done
