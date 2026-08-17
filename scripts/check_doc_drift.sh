#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Drift detection for doc/claude/.  Catches the patterns that
# routinely rot in plan / reference docs:
#
#   1. Broken plan links — markdown links of the form
#      `[...](path/to/plan)` where the resolved path doesn't exist.
#      Plans move between current/future/deferred/finished and
#      links don't always follow.
#   2. Time-projection language (multi-week, 2-3 weeks, etc.).
#   3. Stale "is current" claims about retired features (text_code,
#      Type::Long, .loftc, forwarding_smoke.rs).
#
# Reports findings; does NOT fix.  Exit 0 if clean, 1 if drift
# found that's likely real (broken paths, stale claims).  Time
# projections are warnings only.
#
# Usage:
#   scripts/check_doc_drift.sh                 # all checks, verbose
#   scripts/check_doc_drift.sh -q              # all checks, summary only
#   scripts/check_doc_drift.sh paths           # only path drift
#   scripts/check_doc_drift.sh time            # only time projections
#   scripts/check_doc_drift.sh stale           # only stale claims
#   scripts/check_doc_drift.sh roadmap         # only ROADMAP/disk cross-check
#   scripts/check_doc_drift.sh refs            # only finished/deferred refs
#   scripts/check_doc_drift.sh examples        # only worked-example tag resolution
#
# Exit code: 0 = clean (or only time-projection warnings), 1 = drift.

set -u

cd "$(dirname "$0")/.."

QUIET=0
if [ "${1:-}" = "-q" ] || [ "${1:-}" = "--quiet" ]; then
  QUIET=1
  shift
fi
CHECK="${1:-all}"
DRIFT=0
HITS_PATHS=0
HITS_TIME=0
HITS_STALE=0
HITS_ROADMAP=0
HITS_REFS=0
HITS_LIBS=0
HITS_EXAMPLES=0
HITS_EXAMPLES_WARN=0
HITS_EXINDEX=0

red()    { [ $QUIET -eq 0 ] && printf '\033[31m%s\033[0m\n' "$*"; }
yellow() { [ $QUIET -eq 0 ] && printf '\033[33m%s\033[0m\n' "$*"; }
green()  { [ $QUIET -eq 0 ] && printf '\033[32m%s\033[0m\n' "$*"; }
say()    { [ $QUIET -eq 0 ] && echo "$@"; }

# ---- Check 1: broken markdown links to plans ----
check_paths() {
  say "=== Broken plan links ==="
  local hits=0
  # Match markdown links [...](url) where url contains plans/<NN>-<slug>.
  # Resolve the url relative to the containing file.
  #
  # Both loops read from a temp file / here-string rather than a `< <(...)`
  # process substitution: macOS's stock bash 3.2 corrupts its heap and dies
  # (SIGBUS/SIGKILL) on hundreds of nested process substitutions, so a local
  # `make ci` on a Mac never got past this check.  Same output on Linux.
  local links
  links=$(mktemp)
  grep -rn -E '\]\([^)]*(lib_plans|plans)/[^)]*[0-9]+-[a-z0-9-]+' \
       doc/claude/ CLAUDE.md --include='*.md' 2>/dev/null \
     | grep -v 'check_doc_drift.sh' > "$links"
  while IFS= read -r line; do
    file="${line%%:*}"
    rest="${line#*:}"
    lineno="${rest%%:*}"
    text="${rest#*:}"
    # Match markdown-link targets: ](path) — explicit ] before ( so we
    # don't capture surrounding prose.  Multiple links per line OK.
    while read -r target; do
      # Strip trailing fragment (#section) and query.
      clean="${target%%#*}"
      clean="${clean%%\?*}"
      [ -z "$clean" ] && continue
      # Skip external URLs — a cross-repo link like
      # https://github.com/.../plans/future/06-foo is NOT a local path
      # (e.g. dryopea's plans linked from a loft lib_plan).
      case "$clean" in
        http://*|https://*|mailto:*) continue ;;
      esac
      # Skip non-plan targets in the same line.
      case "$clean" in
        *plans/*[0-9]-*) ;;
        *) continue ;;
      esac
      # Resolve relative to the file's directory.  `[ -e ]` resolves an
      # embedded `..` itself, so no `realpath` is needed — which also keeps
      # this portable, as BSD `realpath` rejects `-m --relative-to`.
      # `${file%/*}` (not `dirname`, a subshell) — but for a top-level file
      # with no `/` that leaves the name unchanged, so map that to `.`.
      case "$file" in */*) dir="${file%/*}" ;; *) dir="." ;; esac
      check_path="$dir/$clean"
      if [ ! -e "$check_path" ]; then
        red "  $file:$lineno → $clean (resolved: $check_path)"
        hits=$((hits + 1))
      fi
    done <<< "$(printf '%s\n' "$text" | grep -oE '\]\([^)]*\)' | sed -E 's/^\]\(//; s/\)$//')"
  done < "$links"
  rm -f "$links"
  HITS_PATHS=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits broken plan links"
    DRIFT=1
  fi
}

# ---- Check 2: time-projection language ----
check_time() {
  say "=== Time projections ==="
  local hits=0
  local patterns=(
    'weeks? of focused'
    '[0-9]+-[0-9]+ weeks'
    'multi-week'
    'next [0-9]+ months'
    # A time unit must follow, or this fires on "expected to take the identical
    # spelling" — a warning that is wrong is one people learn to skip past.
    'expected to take [^.]*(hour|day|week|month|session)'
    'Estimated cost.*hours'
    'Estimated cost.*sessions'
  )
  for pat in "${patterns[@]}"; do
    while IFS= read -r match; do
      case "$match" in
        *plans/finished/*|*CHANGELOG*|*plans/_LIFECYCLE.md*|*scripts/check_doc_drift.sh*)
          continue
          ;;
      esac
      yellow "  $match"
      hits=$((hits + 1))
    done < <(grep -rn -E "$pat" doc/claude/ CLAUDE.md --include='*.md' 2>/dev/null)
  done
  HITS_TIME=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    yellow "  $hits time projections (consider effort letters XS/S/M/MH/H/VH/L)"
    # Time projections are warnings, not errors.
  fi
}

# ---- Check 3: stale claims about retired features ----
check_stale() {
  say "=== Stale 'is current' claims about retired features ==="
  local hits=0
  # Tighter patterns: only Rust-code-block or definition-shape mentions
  # (excludes prose mentions in "removed/retired" context).
  local stale_patterns=(
    'pub.*text_code:.*Vec<u8>'
    'text_code: \*const Vec<u8>'
    'pub Long,?\s*//'
    'src/generation/ops/forwarding_smoke\.rs'
    '\.loftc.*current|byte_code_with_cache'
  )
  for pat in "${stale_patterns[@]}"; do
    while IFS= read -r match; do
      file="${match%%:*}"
      case "$file" in
        */CHANGELOG*|*/plans/finished/*|*/plans/deferred/*|*scripts/check_doc_drift.sh)
          continue
          ;;
      esac
      line_text="${match#*:}"
      line_text="${line_text#*:}"
      if echo "$line_text" | grep -qiE 'removed|retired|no longer|previous|legacy|former|was '; then
        continue
      fi
      red "  $match"
      hits=$((hits + 1))
    done < <(grep -rn -E "$pat" doc/claude/ CLAUDE.md --include='*.md' 2>/dev/null)
  done
  HITS_STALE=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits potentially stale claims"
    DRIFT=1
  fi
}

# ---- Check 4: ROADMAP plan-state cross-check ----
# Rules:
#   - Active plans (plans/<NN>, plans/future/<NN>) MAY appear on ROADMAP, but
#     absence is NOT drift — plan tracking lives in the issues (loft-lang/plans),
#     not a hand-maintained ROADMAP table.  Only a dual / wrong-bucket citation
#     of whatever IS on ROADMAP is flagged.
#   - Deferred plans (plans/deferred/<NN>) MUST NOT appear on ROADMAP.
#     Their home is DEFERRED.md (trigger index).
#   - Finished plans (plans/finished/<NN>) MUST NOT appear on ROADMAP as
#     action items.  Closure lives in CHANGELOG + git history.
#     Parenthetical historical mentions are tolerated.
check_roadmap() {
  say "=== ROADMAP plan-state cross-check ==="
  local hits=0
  local roadmap=doc/claude/ROADMAP.md
  if [ ! -f "$roadmap" ]; then
    yellow "  $roadmap missing — skipping"
    return
  fi

  for tracker in plans lib_plans; do
    for bucket in '' future deferred finished; do
      bucket_dir="doc/claude/$tracker${bucket:+/$bucket}"
      [ -d "$bucket_dir" ] || continue
      for plan_dir in "$bucket_dir"/[0-9]*/; do
        [ -d "$plan_dir" ] || continue
        slug=$(basename "$plan_dir")
        if [ -z "$bucket" ]; then
          canonical="$tracker/$slug"
        else
          canonical="$tracker/$bucket/$slug"
        fi

        case "$bucket" in
          finished|deferred)
            # MUST NOT appear on ROADMAP as action item.
            # Tolerate parenthetical / "Shipped 2026-XX" historical mentions.
            offending=$(grep -nE "$tracker/$bucket/$slug" "$roadmap" \
                        | grep -vE 'Shipped [0-9]|\(plans/(finished|deferred)/|^[0-9]+:#' )
            # Also flag short-form citation in active-bucket position
            # (e.g. plans/28-const-store when plan is in deferred/).
            short=$(grep -nE "(^|[^/])$tracker/$slug([^/0-9a-z-]|$)" "$roadmap")
            if [ -n "$offending" ] || [ -n "$short" ]; then
              red "  $canonical → $bucket plan still on ROADMAP:"
              { [ -n "$offending" ] && echo "$offending"; [ -n "$short" ] && echo "$short"; } \
                | sort -u | sed 's/^/    /' | head -4
              hits=$((hits + 1))
            fi
            ;;
          *)
            # Active plan (current or future).  Tracking lives in the issues now
            # (loft-lang/plans), NOT a hand-maintained ROADMAP — so being absent
            # from ROADMAP is FINE, not drift.  Only flag a plan cited at MULTIPLE
            # / wrong buckets (a real inconsistency in whatever IS on ROADMAP).
            if grep -qE "$canonical" "$roadmap"; then
              other_buckets=""
              for other_b in '' future deferred finished; do
                [ "$other_b" = "$bucket" ] && continue
                if [ -z "$other_b" ]; then
                  other_path="$tracker/$slug"
                else
                  other_path="$tracker/$other_b/$slug"
                fi
                [ "$other_path" = "$canonical" ] && continue
                # Match boundary: not followed by alphanum/dash so we don't
                # match plans/14-tuple as a substring of plans/14-tuple-foo.
                if grep -qE "$other_path([^/0-9a-zA-Z-]|/|$)" "$roadmap"; then
                  other_buckets="$other_buckets $other_path"
                fi
              done
              if [ -n "$other_buckets" ]; then
                red "  $canonical → cited at canonical AND wrong path(s):$other_buckets"
                hits=$((hits + 1))
              fi
            fi
            ;;
        esac
      done
    done
  done

  HITS_ROADMAP=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits ROADMAP/disk drift items"
    DRIFT=1
  fi
}

# ---- Check 5: closed/deferred plan references from normal docs ----
# Closure rule: when a plan ships, reference content moves OUT to
# doc/claude/<NAME>.md.  Other docs link to the reference home, NOT
# the closed plan.  Same for deferred plans (their content is
# either reference-extracted or part of design-content kept in the
# plan README; non-plan docs shouldn't deep-link into deferred plan
# files for ongoing reference).
#
# Allowed: links FROM other plan READMEs (cross-arc references) and
# FROM the closed/deferred plan's siblings.  Flagged: links from
# normal reference docs (doc/claude/<NAME>.md, CLAUDE.md, lib/<name>/*.md).
check_refs() {
  say "=== finished/deferred plan refs from normal docs ==="
  local hits=0
  # Find every markdown link target containing plans/finished/ or plans/deferred/.
  # Tolerance pattern (closure narrative + status annotations + design-pointer phrases).
  local tol_pat='closed by|closure record|shipped (via|by)|historical|retrospective|moved to .*finished|moved to .*deferred|\(closed [0-9]|\(active\)|\(shipped|\(deferred|preserved at|originally documented|design lives|design content|Phase A \+ D|fixture catalogue|spec captured in|catalogue from|deferred follow-ups|trigger conditions'
  # Temp file / here-string, not `< <(...)`: bash 3.2 (macOS) crashes on
  # heavy process substitution — see check_paths.
  local refs
  refs=$(mktemp)
  grep -rn -E '\]\([^)]*plans/(finished|deferred)/' \
       doc/claude/ CLAUDE.md lib/ --include='*.md' 2>/dev/null \
     | grep -v 'check_doc_drift.sh' > "$refs"
  while IFS= read -r line; do
    file="${line%%:*}"
    rest="${line#*:}"
    lineno="${rest%%:*}"
    text="${rest#*:}"
    # Skip:
    # - Links FROM plan READMEs themselves (cross-arc references are fine).
    # - Links FROM presentations/ (not real documentation; archaeology).
    case "$file" in
      */plans/*|*/lib_plans/*|*/presentations/*) continue ;;
    esac
    # Build context window: 3 lines back + current + 1 forward.  Markdown
    # paragraphs wrap heavily so a closure-narrative phrase can sit several
    # lines above the link.
    context=$(awk -v n="$lineno" 'NR>=n-3 && NR<=n+1' "$file" 2>/dev/null)
    while read -r target; do
      clean="${target%%#*}"
      clean="${clean%%\?*}"
      [ -z "$clean" ] && continue
      case "$clean" in
        *plans/finished/*|*plans/deferred/*) ;;
        *) continue ;;
      esac
      # Tolerate explicit closure-narrative + status-annotation patterns.
      if echo "$context" | grep -qiE "$tol_pat"; then
        continue
      fi
      # Deep-link to a phase file (not just the README) is always drift —
      # reference content should have moved out per the closure rule.
      case "$clean" in
        *plans/finished/*/*.md|*plans/deferred/*/*.md)
          case "$clean" in
            */README.md|*/README.md\#*) continue ;;
          esac
          red "  $file:$lineno → $clean (deep-link to phase file)"
          hits=$((hits + 1))
          continue
          ;;
      esac
      red "  $file:$lineno → $clean"
      hits=$((hits + 1))
    done <<< "$(printf '%s\n' "$text" | grep -oE '\]\([^)]*\)' | sed -E 's/^\]\(//; s/\)$//')"
  done < "$refs"
  rm -f "$refs"

  HITS_REFS=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits suspect refs (normal doc → finished/deferred plan)"
    red "  Expected: link to the reference home (doc/claude/<NAME>.md) or to the canonical lib doc."
    red "  Tolerated: explicit closure-narrative ('closed by', 'shipped via', 'historical', etc.)"
    DRIFT=1
  fi
}

# ---- Check 7: lib/<name>/ hygiene (warn-only) ----
# Every lib/<name>/ should have:
#   - loft.toml (package manifest — essential for the package format)
#   - src/<name>.loft (the conventional entry file)
#   - a description comment in the entry file beyond the standard
#     Copyright + SPDX-License-Identifier headers (the "what is this
#     library" docstring that gives a reader 5-second orientation)
#
# Notably we do NOT require README.md.  README is downstream-consumer
# documentation — useful when `loft install <pkg>` (PKG.REG) starts
# surfacing it to users who can't see source, but premature today
# while every consumer reads the source tree directly.  The header
# docstring already gives a reader the "what is this" answer; adding
# a one-line README that points at src/ is busywork without
# additional information.  Re-add the README check when PKG.REG ships.
check_libs() {
  say "=== lib/ hygiene ==="
  local hits=0
  [ -d lib ] || { say "  no lib/ — skipping"; return; }
  for d in lib/*/; do
    [ -d "$d" ] || continue
    name=$(basename "$d")
    missing=""
    [ -f "$d/loft.toml" ] || missing="$missing loft.toml"
    entry="$d/src/$name.loft"
    if [ ! -f "$entry" ]; then
      missing="$missing src/$name.loft"
    elif ! has_header_docstring "$entry"; then
      missing="$missing src/$name.loft:header-docstring"
    fi
    if [ -n "$missing" ]; then
      yellow "  $name: missing$missing"
      hits=$((hits + 1))
    fi
  done
  HITS_LIBS=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    yellow "  $hits libraries with missing hygiene files"
    # Warning-only: doesn't set DRIFT.
  fi
}

# Returns 0 (true) when the file has at least one `//` comment line
# beyond the standard Copyright + SPDX-License-Identifier headers,
# scanning the first 10 non-blank lines.  This is the "what does
# this library do" description that orients a reader in 5 seconds.
#
# Examples that pass:
#   // Graphics library — 2D pixel canvas with drawing primitives.
#   // --- HTTP Server + WebSocket ---
#   // crypto — hashing, HMAC, base64, JWT.
#
# Examples that fail (Copyright/SPDX only):
#   // Copyright (c) 2026 Jurjen Stellingwerff
#   // SPDX-License-Identifier: LGPL-3.0-or-later
#   (blank, then code)
has_header_docstring() {
  local file="$1"
  local count
  # Take first 10 non-blank lines, keep `//` comment lines that are
  # NOT Copyright + NOT SPDX-License-Identifier.  At least one such
  # line means a real description is present.
  count=$(awk '
    /^[[:space:]]*$/ { next }
    seen >= 10       { exit }
    { seen++ }
    /^\/\// && !/Copyright/ && !/SPDX-License-Identifier/ { print; matched++ }
    END { exit (matched > 0 ? 0 : 1) }
  ' "$file" 2>/dev/null | wc -l)
  [ "$count" -gt 0 ]
}

# Emit `tag<TAB>relpath:line<TAB>fn` for every `// @AAA-### … <newline> fn` in a
# tree.  A tag binds to the `fn` that FOLLOWS it in the same comment block (a blank
# line breaks the block); an `Example:` line is a citation, never a definition.  The
# `fn` may be ANY function — a `test_*`, or a real function in a first-class
# application's own source — so a worked example can be a live use, not only a test.
#
# The FIRST tag in a block defines it.  An example's prose routinely names a sibling
# example ("the failure path, see @ARG-004"), and letting a later mention win read
# that block as defining the sibling — which surfaced as a `dangling` on the block's
# own tag AND a `duplicate` on the one it mentioned, both pointing away from the
# actual mistake.
examples_defs_in_tree() {
  local root="$1"
  [ -d "$root" ] || return 0
  ( cd "$root" 2>/dev/null && \
    find . -name '*.loft' -not -path './.*' -not -path './target/*' -print0 2>/dev/null \
    | xargs -0 awk '
        FNR==1 { f=FILENAME; sub(/^\.\//,"",f) }
        /^[[:space:]]*\/\/.*Example:/ { p=""; next }
        /^[[:space:]]*\/\// && match($0, /@[A-Z][A-Z][A-Z]-[0-9][0-9][0-9]/) {
          if (p=="") { p=substr($0,RSTART,RLENGTH); pl=FNR } next }
        /^[[:space:]]*\/\// { next }
        /^[[:space:]]*$/ { p=""; next }
        /^[[:space:]]*(pub )?fn / {
          if (p!="") { n=$0; sub(/^[[:space:]]*(pub )?fn /,"",n); sub(/\(.*$/,"",n);
                       printf "%s\t%s:%d\t%s\n", p, f, pl, n; p="" } next }
        { p="" }
      ' 2>/dev/null )
}

# ---- Check: worked-example tags resolve to a real test/function (@PLN141) ----
# A stdlib / library function documents its correct use by CITING a demonstrator:
# `// Example: @AAA-###`, where the tag names a `fn` carrying `// @AAA-###` above it
# — a test, OR a real function in a first-class application.  The acronym names the
# repo that owns the tag (scripts/example_repos.tsv), so a citation can point ACROSS
# repos and still be validated + linked:
#
#   ok           the tag resolves to a fn; a cross-repo hit prints its git link.
#   dangling     cited, but no fn in the owning repo carries it → drift.
#   duplicate    one tag on two fns in this repo → ambiguous → drift.
#   unregistered the acronym isn't in the registry → drift (add its repo + url).
#   unvalidated  a cross-repo tag whose repo has no local checkout → WARNING only
#                (the link is still emitted); a local checkout is validated OFFLINE
#                and preferred over any online lookup.
#
# The `@AAA-###` shape (three letters, hyphen, three digits) is distinct from the
# repo's other tag families (@F1, @P259, @PLN141) — none has that hyphen.
check_examples() {
  say "=== Worked-example tags resolve to a test/function ==="
  local tag_re='@[A-Z][A-Z][A-Z]-[0-9][0-9][0-9]'
  # The registry + gate logic are loft-anchored (this script cd'd to the loft root at
  # startup), but the CITATIONS and the local repo's defs come from EXAMPLES_REPO_ROOT:
  # `.` (loft) for loft's own run, the library checkout when library-ci-reusable.yml
  # runs this same gate against a loft-libs-* repo.  Defaults reproduce loft's self-check
  # byte-for-byte; the library CI sets REPO_ROOT + CITE_ROOTS to the package under test.
  local registry="${EXAMPLES_REGISTRY:-scripts/example_repos.tsv}"
  local repo_root="${EXAMPLES_REPO_ROOT:-.}"
  local cite_roots="${EXAMPLES_CITE_ROOTS:-default lib}"
  # The loft repo hosting this gate is always available in place at `.` (the script cd'd
  # to the loft root at startup) — even in a library CI, where loft is checked out as
  # loft-src rather than a sibling ../loft.  So loft's OWN acronyms (STD/GIT/LEX/…)
  # resolve there for any repo-under-test.  Its registry name is `loft` by convention.
  local host_repo="${EXAMPLES_HOST_REPO:-loft}"
  local self_name; self_name=$(basename "$(cd "$repo_root" 2>/dev/null && pwd)")
  local cited cache
  cited=$(mktemp); cache=$(mktemp -d)
  # Citations under the repo-under-test (loft: default/ + lib/).
  ( cd "$repo_root" 2>/dev/null && grep -rhnE "//[[:space:]]*Example:" $cite_roots --include='*.loft' 2>/dev/null ) \
    | grep -oE "$tag_re" | sort -u > "$cited"
  # Cache a repo's defs by checkout path, so each repo is scanned at most once.
  _defs() {
    local key cf; key=$(printf '%s' "$1" | tr '/.' '__'); cf="$cache/$key"
    [ -f "$cf" ] || examples_defs_in_tree "$1" > "$cf" 2>/dev/null
    printf '%s' "$cf"
  }
  local n_cited; n_cited=$(grep -cvE '^$' "$cited")
  local hits=0 t acr row repo url branch lpath def link
  while IFS= read -r t; do
    [ -z "$t" ] && continue
    acr=${t#@}; acr=${acr%%-*}
    row=$(awk -F'\t' -v a="$acr" '$1!~/^#/ && $1==a {print; exit}' "$registry" 2>/dev/null)
    if [ -z "$row" ]; then
      red "  unregistered: $t — acronym '$acr' not in $registry (add its repo + git url)"
      hits=$((hits + 1)); continue
    fi
    repo=$(printf '%s' "$row" | cut -f2)
    url=$(printf '%s'  "$row" | cut -f3)
    branch=$(printf '%s' "$row" | cut -f4)
    # Resolve the owning repo to a checkout: the repo-under-test itself (in place at
    # repo_root), the loft host repo (always in place at `.`, even as loft-src in a
    # library CI), or a foreign sibling checkout (../<repo>).
    lpath="../$repo"
    if [ "$self_name" = "$repo" ]; then lpath="$repo_root"
    elif [ "$repo" = "$host_repo" ]; then lpath="."
    fi
    if [ ! -d "$lpath" ]; then
      yellow "  unvalidated: $t — no sibling checkout ../$repo; clone it to validate. link: $url"
      HITS_EXAMPLES_WARN=$((HITS_EXAMPLES_WARN + 1)); continue
    fi
    def=$(awk -F'\t' -v tg="$t" '$1==tg{print $2; exit}' "$(_defs "$lpath")")
    if [ -z "$def" ]; then
      red "  dangling: $t is cited but no fn carries it in $repo"
      ( cd "$repo_root" 2>/dev/null && grep -rnE "Example:.*$t" $cite_roots --include='*.loft' 2>/dev/null ) | sed 's/^/      /'
      hits=$((hits + 1)); continue
    fi
    if [ "$lpath" = "$repo_root" ]; then
      say "  ok  $t -> $def"
    else
      link="$url/blob/$branch/$(printf '%s' "$def" | sed 's/:\([0-9][0-9]*\)$/#L\1/')"
      say "  ok  $t -> $link  (validated against $repo)"
    fi
  done < "$cited"
  # DUPLICATE — one tag on two fns in THIS repo (each foreign repo owns its own).
  local d
  for d in $(cut -f1 "$(_defs "$repo_root")" | sort | uniq -d); do
    red "  duplicate: $d tags more than one fn in this repo"
    hits=$((hits + 1))
  done
  rm -rf "$cited" "$cache"
  HITS_EXAMPLES=$hits
  if [ $hits -gt 0 ]; then
    DRIFT=1
  elif [ "${HITS_EXAMPLES_WARN:-0}" -eq 0 ]; then
    green "  ok — $n_cited citation(s) resolve to a test/function"
  fi
}

# ---- Worked-example index: where each tag lives (@PLN141) ----
# examples-index.tsv (repo root) lists every `// @AAA-###`-tagged fn in this repo with
# its file:line and git blob link, so a reader — or loft's cross-repo `idx` — knows
# where a tag resolves WITHOUT a checkout.  Generated, never hand-edited: `write-examples-
# index` writes it; `examples-index` VERIFIES the committed copy is current (fail-on-diff,
# the features-check pattern).  Line numbers churn, so it lives WITH the code it indexes,
# not in the central acronym registry (which stays one stable row per acronym).
EXAMPLES_INDEX_FILE="${EXAMPLES_INDEX_FILE:-examples-index.tsv}"

# Emit the index body: `tag <TAB> file:line <TAB> fn <TAB> blob_url`, one row per def,
# sorted by tag.  The blob link comes from the acronym's registry row (url + branch).
_examples_index_body() {
  local root="$1" reg="$2" tag rest fn acr row url branch link
  examples_defs_in_tree "$root" | sort | while IFS=$'\t' read -r tag rest fn; do
    acr=${tag#@}; acr=${acr%%-*}
    row=$(awk -F'\t' -v a="$acr" '$1!~/^#/ && $1==a {print; exit}' "$reg" 2>/dev/null)
    url=$(printf '%s' "$row" | cut -f3); branch=$(printf '%s' "$row" | cut -f4)
    if [ -n "$url" ]; then
      link="$url/blob/$branch/$(printf '%s' "$rest" | sed 's/:\([0-9][0-9]*\)$/#L\1/')"
    else
      link="-"   # acronym not registered — path still recorded, no link
    fi
    printf '%s\t%s\t%s\t%s\n' "$tag" "$rest" "$fn" "$link"
  done
}

_examples_index_full() {
  local root="$1" reg="$2"
  printf '# Generated by scripts/check_doc_drift.sh write-examples-index (@PLN141) — DO NOT EDIT.\n'
  printf '# Regenerate: make examples-index    Verified in CI: check_doc_drift.sh examples-index\n'
  printf '# Every worked-example tag defined in this repo: where it lives + its git blob link.\n'
  printf '# tag\tfile:line\tfn\tblob_url\n'
  _examples_index_body "$root" "$reg"
}

write_examples_index() {
  local root="${EXAMPLES_REPO_ROOT:-.}" reg="${EXAMPLES_REGISTRY:-scripts/example_repos.tsv}"
  _examples_index_full "$root" "$reg" > "$root/$EXAMPLES_INDEX_FILE"
  say "wrote $root/$EXAMPLES_INDEX_FILE ($(grep -cvE '^#' "$root/$EXAMPLES_INDEX_FILE") tag(s))"
}

# Verify the committed examples-index.tsv matches the tags in the tree.  Absent index is
# fine ONLY when the repo defines no tags; otherwise it is stale/missing drift (red).
check_examples_index() {
  say "=== Worked-example index (examples-index.tsv) is current ==="
  local root="${EXAMPLES_REPO_ROOT:-.}" reg="${EXAMPLES_REGISTRY:-scripts/example_repos.tsv}"
  local f="$root/$EXAMPLES_INDEX_FILE" tmp; tmp=$(mktemp)
  _examples_index_full "$root" "$reg" > "$tmp"
  local defines_tags=0; grep -qE '^@' "$tmp" && defines_tags=1
  if [ ! -f "$f" ]; then
    if [ $defines_tags -eq 1 ]; then
      red "  missing: $EXAMPLES_INDEX_FILE — run 'make examples-index' (repo defines worked-example tags)"
      HITS_EXINDEX=1; DRIFT=1
    else
      green "  ok — no worked-example tags defined; no index needed"
    fi
    rm -f "$tmp"; return
  fi
  if diff -q "$f" "$tmp" >/dev/null 2>&1; then
    green "  ok — $EXAMPLES_INDEX_FILE lists $(grep -cE '^@' "$f") tag(s), all current"
  else
    red "  stale: $EXAMPLES_INDEX_FILE is out of date — run 'make examples-index'"
    [ $QUIET -eq 0 ] && diff "$f" "$tmp" | sed 's/^/      /' | head -20
    HITS_EXINDEX=1; DRIFT=1
  fi
  rm -f "$tmp"
}

# Sep between sections (verbose mode only).
sep() { [ $QUIET -eq 0 ] && echo; }

case "$CHECK" in
  paths)   check_paths ;;
  time)    check_time ;;
  stale)   check_stale ;;
  roadmap) check_roadmap ;;
  refs)    check_refs ;;
  libs)    check_libs ;;
  examples) check_examples ;;
  examples-index) check_examples_index ;;
  write-examples-index) write_examples_index; exit 0 ;;
  all)
    check_paths
    sep
    check_time
    sep
    check_stale
    sep
    check_roadmap
    sep
    check_refs
    sep
    check_libs
    sep
    check_examples
    sep
    check_examples_index
    ;;
  *)
    echo "Usage: $0 [-q|--quiet] [all|paths|time|stale|roadmap|refs|libs|examples|examples-index|write-examples-index]" >&2
    exit 2
    ;;
esac

# One-line summary (always printed; even in quiet mode this is the only output).
total=$((HITS_PATHS + HITS_STALE + HITS_ROADMAP + HITS_REFS + HITS_EXAMPLES + HITS_EXINDEX))
warns=$((HITS_TIME + HITS_LIBS + HITS_EXAMPLES_WARN))
summary="paths=$HITS_PATHS time=$HITS_TIME stale=$HITS_STALE roadmap=$HITS_ROADMAP refs=$HITS_REFS libs=$HITS_LIBS examples=$HITS_EXAMPLES/w$HITS_EXAMPLES_WARN exindex=$HITS_EXINDEX"
if [ $DRIFT -eq 0 ] && [ $warns -eq 0 ]; then
  printf '\033[32mclean\033[0m (%s)\n' "$summary"
  exit 0
elif [ $DRIFT -eq 0 ]; then
  # Only warn-level findings — exit 0 but flag in summary.
  printf '\033[33mclean+warn\033[0m (%s) — %d warning(s)\n' "$summary" "$warns"
  exit 0
else
  printf '\033[31mDRIFT\033[0m (%s) — %d action items\n' "$summary" "$total"
  exit 1
fi
