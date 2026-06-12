#!/usr/bin/env bash
# One-command registry maintenance: see what needs publishing, OK once,
# sign once — and the registry is current again.
#
#   scripts/registry_maintain.sh [--dry-run] [--yes] [--key <file>] [--registry-dir <dir>]
#
# The script gathers BOTH populations into one worklist:
#
#   own libs     — every `loft-lang/loft-libs-*` library that is missing from
#                  or newer than the registry index.  These the script can fix
#                  end-to-end: `loft package` → tag + GitHub release →
#                  `loft publish` → index entry.
#   foreign libs — third-party state the maintainer can only review:
#                  open submission PRs on `loft-lang/registry` (merged when
#                  their validation CI is green), and registry packages whose
#                  upstream repo has newer release tags than the index
#                  (informational — publishing is the author's move).
#
# Then ONE confirmation, the publishes + green-PR merges run, the index is
# updated, and the maintainer signs once (`loft-keygen sign`, laptop-only key —
# PKG_REGISTRY.md § Why laptop signing).  Without a key file the script stops
# after staging the commit and prints the two remaining commands.
#
#   --dry-run        gather + print the worklist, change nothing
#   --yes            skip the confirmation prompt
#   --key <file>     Ed25519 private key (default: $LOFT_REGISTRY_KEY)
#   --registry-dir   existing loft-lang/registry checkout (default: temp clone)
set -euo pipefail

ORG=loft-lang
DRY=0
YES=0
KEY="${LOFT_REGISTRY_KEY:-}"
REG_DIR="${LOFT_REGISTRY_DIR:-}"
while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY=1 ;;
        --yes) YES=1 ;;
        --key) KEY="$2"; shift ;;
        --registry-dir) REG_DIR="$2"; shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

here=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for dep in git gh python3 curl; do
    command -v "$dep" > /dev/null || { echo "missing dependency: $dep" >&2; exit 2; }
done
gh auth status > /dev/null 2>&1 || { echo "gh is not authenticated (gh auth login)" >&2; exit 2; }

# loft + loft-keygen binaries (release build; the registry feature is default).
LOFT="$here/target/release/loft"
KEYGEN="$here/target/release/loft-keygen"
if [ ! -x "$LOFT" ] || [ ! -x "$KEYGEN" ]; then
    echo "building loft + loft-keygen (release) ..."
    (cd "$here" && cargo build --release --bin loft --bin loft-keygen --features registry)
fi

# ── gather ────────────────────────────────────────────────────────────────────

echo "fetching registry index ..."
curl -fsSL "https://raw.githubusercontent.com/$ORG/registry/main/index.json" -o "$tmp/index.json"

echo "cloning family repos ..."
repos=$(gh api "orgs/$ORG/repos?per_page=100" --jq '.[].name' | grep '^loft-libs-' || true)
: > "$tmp/own.tsv" # <name>\t<version>\t<repo>\t<libdir>
for r in $repos; do
    git clone --quiet --depth 1 "https://github.com/$ORG/$r.git" "$tmp/$r"
    for t in "$tmp/$r"/*/loft.toml; do
        [ -f "$t" ] || continue
        name=$(sed -n 's/^name *= *"\(.*\)"/\1/p' "$t" | head -1)
        ver=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$t" | head -1)
        [ -n "$name" ] && printf '%s\t%s\t%s\t%s\n' "$name" "$ver" "$r" "$(dirname "$t")" >> "$tmp/own.tsv"
    done
done

# Own-lib worklist: missing/stale vs the index (same rule as
# check_registry_coverage.sh, but keeping the libdir for the publish step).
python3 - "$tmp" <<'EOF'
import json, sys

tmp = sys.argv[1]
index = json.load(open(f"{tmp}/index.json"))["packages"]


def semver(v):
    return tuple(int(p) if p.isdigit() else 0 for p in v.split("."))


work = []
for line in open(f"{tmp}/own.tsv"):
    name, ver, repo, libdir = line.rstrip("\n").split("\t")
    if name not in index:
        work.append((name, ver, repo, libdir, "missing"))
    elif semver(ver) > semver(max(index[name]["versions"], key=semver)):
        work.append((name, ver, repo, libdir, "stale"))
with open(f"{tmp}/work.tsv", "w") as f:
    for row in sorted(work):
        f.write("\t".join(row) + "\n")

# Foreign packages: index entries whose release URL lives outside the org.
foreign = []
for name, pkg in sorted(index.items()):
    newest = max(pkg["versions"], key=semver)
    url = pkg["versions"][newest].get("url", "")
    parts = url.removeprefix("https://github.com/").split("/")
    if len(parts) >= 2 and parts[0] != "loft-lang":
        foreign.append(f"{name}\t{newest}\t{parts[0]}/{parts[1]}")
open(f"{tmp}/foreign.tsv", "w").write("\n".join(foreign) + ("\n" if foreign else ""))
EOF

# Foreign upstream drift: does the author's repo have a newer release tag than
# the registry?  Informational only — the author publishes, not us.
: > "$tmp/foreign_drift.txt"
while IFS=$'\t' read -r name newest frepo; do
    latest=$(gh api "repos/$frepo/releases?per_page=100" \
        --jq ".[].tag_name | select(startswith(\"$name-v\"))" 2> /dev/null |
        sed "s/^$name-v//" | sort -V | tail -1 || true)
    if [ -n "$latest" ] && [ "$(printf '%s\n%s\n' "$newest" "$latest" | sort -V | tail -1)" != "$newest" ]; then
        echo "  $name: registry $newest, upstream $frepo has $latest — nudge the author" >> "$tmp/foreign_drift.txt"
    fi
done < "$tmp/foreign.tsv"

# Foreign submissions: open PRs on the registry repo + their CI verdict.
gh pr list -R "$ORG/registry" --state open \
    --json number,title,author,statusCheckRollup \
    --jq '.[] | [.number, .author.login, ([.statusCheckRollup[]?.conclusion] | if length == 0 then "pending" elif all(. == "SUCCESS") then "green" else "red" end), .title] | @tsv' \
    > "$tmp/prs.tsv" 2> /dev/null || : > "$tmp/prs.tsv"

# ── the worklist ──────────────────────────────────────────────────────────────

n_own=$(wc -l < "$tmp/work.tsv")
n_prs=$(wc -l < "$tmp/prs.tsv")
echo
echo "== own libs to publish ($n_own) =="
awk -F'\t' '{ printf "  %s %s (%s, %s)\n", $1, $2, $3, $5 }' "$tmp/work.tsv"
echo "== foreign submission PRs on $ORG/registry ($n_prs) =="
awk -F'\t' '{ printf "  #%s by %s [%s] %s\n", $1, $2, $3, $4 }' "$tmp/prs.tsv"
echo "== foreign upstream drift (informational) =="
cat "$tmp/foreign_drift.txt"
echo

if [ "$n_own" = 0 ] && [ "$n_prs" = 0 ]; then
    echo "nothing to do — registry is current."
    exit 0
fi
if [ "$DRY" = 1 ]; then
    echo "[dry-run] stopping before any change."
    exit 0
fi
if [ "$YES" != 1 ]; then
    n_green=$(awk -F'\t' '$3 == "green"' "$tmp/prs.tsv" | wc -l)
    read -rp "publish $n_own own lib(s) + merge $n_green green PR(s), then sign? [y/N] " a
    [ "$a" = y ] || [ "$a" = Y ] || { echo "aborted."; exit 1; }
fi

# ── execute ───────────────────────────────────────────────────────────────────

# Merge the green foreign PRs FIRST so the registry pull below includes them.
while IFS=$'\t' read -r num _author state _title; do
    if [ "$state" = green ]; then
        echo "merging $ORG/registry#$num ..."
        gh pr merge "$num" -R "$ORG/registry" --squash
    else
        echo "skipping $ORG/registry#$num ($state — review by hand)"
    fi
done < "$tmp/prs.tsv"

if [ -z "$REG_DIR" ]; then
    REG_DIR="$tmp/registry"
    git clone --quiet "https://github.com/$ORG/registry.git" "$REG_DIR"
else
    git -C "$REG_DIR" pull --ff-only
fi

# A merged PR may have covered an own-lib finding (an author can submit via PR
# before this script runs) — re-filter the worklist against the post-merge
# index so those libraries are not published twice.
python3 - "$tmp" "$REG_DIR/index.json" <<'EOF'
import json, sys

tmp, index_path = sys.argv[1:3]
index = json.load(open(index_path))["packages"]


def semver(v):
    return tuple(int(p) if p.isdigit() else 0 for p in v.split("."))


keep = []
for line in open(f"{tmp}/work.tsv"):
    name, ver = line.split("\t")[:2]
    if name in index and semver(ver) <= semver(max(index[name]["versions"], key=semver)):
        print(f"  {name} {ver} covered by a just-merged PR — skipping")
        continue
    keep.append(line)
open(f"{tmp}/work.tsv", "w").writelines(keep)
EOF

# Publish each own lib: tarball → tag + release (created only when absent,
# so re-runs are safe) → `loft publish` emits the verified index entry.
published=()
while IFS=$'\t' read -r name ver repo libdir _why; do
    echo "publishing $name $ver from $repo ..."
    tag="$name-v$ver"
    tarball="$libdir/$name-$ver.tar.gz"
    "$LOFT" package "$libdir" > /dev/null
    [ -f "$tarball" ] || { echo "  expected tarball $tarball missing" >&2; exit 1; }
    if ! gh release view "$tag" -R "$ORG/$repo" > /dev/null 2>&1; then
        gh release create "$tag" "$tarball" -R "$ORG/$repo" \
            --target "$(git -C "$tmp/$repo" rev-parse HEAD)" --title "$name v$ver"
    fi
    "$LOFT" publish "$libdir" | grep -v '^#' > "$tmp/entry_$name.json"
    # First-version packages also need a package block; take the description
    # from the library README's first paragraph.
    desc=$(awk '/^[^#[:space:]]/ { print; exit }' "$tmp/$repo/$name/README.md" 2> /dev/null || true)
    python3 - "$REG_DIR/index.json" "$name" "$ver" "$tmp/entry_$name.json" \
        "https://github.com/$ORG/$repo/tree/main/$name" "${desc:-loft library $name}" <<'EOF'
import json, sys

index_path, name, ver, entry_path, homepage, desc = sys.argv[1:7]
entry = json.loads("{%s}" % open(entry_path).read())[ver]
index = json.load(open(index_path))
pkg = index["packages"].setdefault(
    name, {"description": desc, "homepage": homepage, "categories": [], "yanked": [], "versions": {}}
)
pkg["versions"][ver] = entry
json.dump(index, open(index_path, "w"), indent=2)
EOF
    published+=("$name-$ver")
done < "$tmp/work.tsv"

if [ "${#published[@]}" -gt 0 ]; then
    echo
    git -C "$REG_DIR" --no-pager diff --stat index.json
    git -C "$REG_DIR" add index.json
fi

# ── sign + push ───────────────────────────────────────────────────────────────

if [ -z "$KEY" ] || [ ! -f "$KEY" ]; then
    echo
    echo "no signing key (--key / LOFT_REGISTRY_KEY) — staged but NOT signed/pushed."
    echo "on the machine holding the key, finish with:"
    echo "  $KEYGEN sign --in $REG_DIR/index.json --key <keyfile> --out $REG_DIR/index.json.sig"
    echo "  git -C $REG_DIR add index.json.sig && git -C $REG_DIR commit -m 'publish: ${published[*]:-}' && git -C $REG_DIR push"
    exit 0
fi
"$KEYGEN" sign --in "$REG_DIR/index.json" --key "$KEY" --out "$REG_DIR/index.json.sig"
git -C "$REG_DIR" add index.json index.json.sig
git -C "$REG_DIR" commit -m "publish: ${published[*]:-PR merges only}"
git -C "$REG_DIR" push

echo
echo "verifying — coverage check should now be clean:"
"$here/scripts/check_registry_coverage.sh"
