#!/usr/bin/env bash
# Warn when a loft-lang library is missing from — or newer than — the registry.
#
# Enumerates every `loft-lang/loft-libs-*` family repo (shallow clones, so the
# only GitHub API call is the org repo listing), reads each library's
# `<lib>/loft.toml` version, and compares it against the published registry
# index (`loft-lang/registry` → `index.json`).  Findings:
#
#   missing — the library has no registry entry at all
#   stale   — the repo version is newer than the newest registry version
#   orphan  — the registry lists a package no family repo contains (info only)
#
# Libraries still living in this repo's `lib/` are NOT checked: they are
# unextracted by design (PACKAGES.md § Open work, PKG.EXTRACT) and would be
# permanent noise here.
#
# Advisory by default: findings print as GitHub warning annotations (inside
# Actions) and the script exits 0.  `--strict` exits 1 on any missing/stale
# finding — the local pre-publish gate.  Network problems are a skip, not a
# failure: registry truth lives outside this repo.
set -euo pipefail

STRICT=0
INDEX_FILE=""        # --index <path>: use a local index instead of the CDN URL
while [ $# -gt 0 ]; do
    case "$1" in
        --strict) STRICT=1 ;;
        --index)  INDEX_FILE="$2"; shift ;;
        *) echo "check_registry_coverage: unknown arg: $1" >&2; exit 2 ;;
    esac
    shift
done

ORG=loft-lang
INDEX_URL="https://raw.githubusercontent.com/$ORG/registry/main/index.json"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [ -n "$INDEX_FILE" ]; then
    # Caller supplied a local index (registry_maintain passes its just-pushed
    # copy).  Avoids a spurious "stale" right after a publish, when the CDN-cached
    # raw.githubusercontent URL still serves the pre-push index.
    cp "$INDEX_FILE" "$tmp/index.json" 2> /dev/null \
        || { echo "check_registry_coverage: cannot read --index $INDEX_FILE" >&2; exit 2; }
elif ! curl -fsSL "$INDEX_URL" -o "$tmp/index.json"; then
    echo "registry index unreachable ($INDEX_URL) — skipping (needs network)"
    exit 0
fi

auth=()
[ -n "${GITHUB_TOKEN:-}" ] && auth=(-H "Authorization: Bearer $GITHUB_TOKEN")
if ! curl -fsSL ${auth[@]+"${auth[@]}"} \
    "https://api.github.com/orgs/$ORG/repos?per_page=100" -o "$tmp/repos.json"; then
    echo "GitHub API unreachable — skipping (needs network)"
    exit 0
fi

repos=$(python3 -c "
import json
for r in json.load(open('$tmp/repos.json')):
    if r['name'].startswith('loft-libs-'):
        print(r['name'])
")

# One `<name>\t<version>\t<repo>` line per library found in a family repo.
: > "$tmp/libs.tsv"
for r in $repos; do
    if ! git clone --quiet --depth 1 "https://github.com/$ORG/$r.git" "$tmp/$r"; then
        echo "clone of $r failed — skipping (needs network)"
        exit 0
    fi
    for t in "$tmp/$r"/*/loft.toml; do
        [ -f "$t" ] || continue
        name=$(sed -n 's/^name *= *"\(.*\)"/\1/p' "$t" | head -1)
        ver=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$t" | head -1)
        [ -n "$name" ] && printf '%s\t%s\t%s\n' "$name" "$ver" "$r" >> "$tmp/libs.tsv"
    done
done

python3 - "$tmp" <<'EOF'
import json, os, sys

tmp = sys.argv[1]
index = json.load(open(f"{tmp}/index.json"))["packages"]
annotate = os.environ.get("GITHUB_ACTIONS") == "true"


def semver(v):
    """Order-comparable tuple; non-numeric parts sort as 0 (good enough here)."""
    return tuple(int(p) if p.isdigit() else 0 for p in v.split("."))


libs = {}
for line in open(f"{tmp}/libs.tsv"):
    name, ver, repo = line.rstrip("\n").split("\t")
    libs[name] = (ver, repo)

findings = []
for name, (ver, repo) in sorted(libs.items()):
    if name not in index:
        findings.append(f"missing: {name} {ver} ({repo}) has no registry entry")
        continue
    newest = max(index[name]["versions"], key=semver)
    if semver(ver) > semver(newest):
        findings.append(
            f"stale: {name} ({repo}) repo has {ver}, registry tops out at {newest}"
        )

for f in findings:
    print(f"  {f}")
    if annotate:
        print(f"::warning title=registry coverage::{f}")

orphans = sorted(set(index) - set(libs))
if orphans:
    print(f"  orphan (registry-only, info): {', '.join(orphans)}")

print(
    f"{len(findings)} finding(s) across {len(libs)} libraries / "
    f"{len(index)} registry packages — publish flow: doc/claude/REGISTRY_SUBMIT.md"
)
open(f"{tmp}/count", "w").write(str(len(findings)))
EOF

count=$(cat "$tmp/count")
[ "$STRICT" = 1 ] && [ "$count" != 0 ] && exit 1
exit 0
