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
# To review-then-sign a SINGLE registry PR (shows the diff + each library
# release + notes, re-checks every tarball sha256, then signs/commits/pushes),
# use `scripts/registry-sign.sh --pr <N>` instead — the per-PR signing path.
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
# RELIABILITY: a green (CI-passed) PR can still have MERGE CONFLICTS if main moved
# since its CI run.  A failed `gh pr merge` must NOT abort the routine — `gh pr
# merge` squash-commits the PR's index.json change to the remote, but the .sig is
# only regenerated by the sign step at the very end; aborting here leaves the
# registry with a changed index.json and a STALE signature (= "signature invalid
# — refusing to load" for every consumer).  So skip the unmergeable PR, record
# it, and carry on to the sign step, which re-signs the resulting index.
SKIPPED_PRS=()
while IFS=$'\t' read -r num _author state _title; do
    if [ "$state" != green ]; then
        echo "skipping $ORG/registry#$num ($state — review by hand)"
        continue
    fi
    mergeable=$(gh pr view "$num" -R "$ORG/registry" --json mergeable -q .mergeable 2>/dev/null || echo UNKNOWN)
    if [ "$mergeable" = CONFLICTING ]; then
        echo "⚠ skipping $ORG/registry#$num — merge conflict (main moved since CI); rebase or close it separately"
        SKIPPED_PRS+=("#$num (conflict)")
        continue
    fi
    echo "merging $ORG/registry#$num ..."
    if ! gh pr merge "$num" -R "$ORG/registry" --squash 2>"$tmp/merge_$num.err"; then
        echo "⚠ could not merge $ORG/registry#$num: $(head -1 "$tmp/merge_$num.err" 2>/dev/null); skipping"
        SKIPPED_PRS+=("#$num (merge failed)")
        continue
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
# RELIABILITY: one own-lib that can't publish (missing release asset, package
# error) must not abort the rest — skip it, record it, keep going so the others
# publish and the index still gets signed.
published=()
SKIPPED_LIBS=()
while IFS=$'\t' read -r name ver repo libdir _why; do
    echo "publishing $name $ver from $repo ..."
    tag="$name-v$ver"
    tarball="$libdir/$name-$ver.tar.gz"
    if ! "$LOFT" package "$libdir" > /dev/null 2>&1 || [ ! -f "$tarball" ]; then
        echo "  ⚠ package/tarball failed — skipping $name $ver"; SKIPPED_LIBS+=("$name-$ver"); continue
    fi
    if ! gh release view "$tag" -R "$ORG/$repo" > /dev/null 2>&1; then
        if ! gh release create "$tag" "$tarball" -R "$ORG/$repo" \
            --target "$(git -C "$tmp/$repo" rev-parse HEAD)" --title "$name v$ver" 2>"$tmp/rel_$name.err"; then
            echo "  ⚠ release create failed ($(head -1 "$tmp/rel_$name.err" 2>/dev/null)) — skipping $name $ver"
            SKIPPED_LIBS+=("$name-$ver"); continue
        fi
    fi
    if ! "$LOFT" publish "$libdir" > "$tmp/pub_$name.out" 2>"$tmp/pub_$name.err"; then
        echo "  ⚠ loft publish failed ($(head -1 "$tmp/pub_$name.err" 2>/dev/null)) — skipping $name $ver"; SKIPPED_LIBS+=("$name-$ver"); continue
    fi
    # The pasteable JSON entry is on stdout; `[publish]` status is on stderr (so
    # do NOT merge with 2>&1).  Strip any #-comment / [publish] / blank line so
    # only the `"<ver>": {…}` member remains — wrapping `{…}` + parsing it below
    # choked on those trailing status lines otherwise.
    grep -vE '^#|^\[publish\]|^[[:space:]]*$' "$tmp/pub_$name.out" > "$tmp/entry_$name.json"
    # First-version packages also need a package block; take the description
    # from the library README's first paragraph.
    desc=$(awk '/^[^#[:space:]]/ { print; exit }' "$tmp/$repo/$name/README.md" 2> /dev/null || true)
    if ! python3 - "$REG_DIR/index.json" "$name" "$ver" "$tmp/entry_$name.json" \
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
    then
        echo "  ⚠ index update failed — skipping $name $ver"; SKIPPED_LIBS+=("$name-$ver"); continue
    fi
    published+=("$name-$ver")
done < "$tmp/work.tsv"

# Summary — never drop a skip silently.
if [ "${#SKIPPED_PRS[@]}" -gt 0 ] || [ "${#SKIPPED_LIBS[@]}" -gt 0 ]; then
    echo
    echo "⚠ skipped (resolve + re-run to include): PRs: ${SKIPPED_PRS[*]:-none} | libs: ${SKIPPED_LIBS[*]:-none}"
fi

echo
git -C "$REG_DIR" --no-pager diff --stat index.json || true

# ── sign + push ───────────────────────────────────────────────────────────────
# ALWAYS re-sign the current index.json.  A `gh pr merge` above changes
# index.json on the remote without regenerating the .sig, so re-signing here is
# what keeps the published signature valid — the routine must never leave the
# index changed but unsigned (that bricks every consumer's install).

if [ -z "$KEY" ] || [ ! -f "$KEY" ]; then
    git -C "$REG_DIR" add index.json
    echo
    echo "no signing key (--key / LOFT_REGISTRY_KEY) — staged but NOT signed/pushed."
    echo "on the machine holding the key, finish with:"
    echo "  $KEYGEN sign --in $REG_DIR/index.json --key <keyfile> --out $REG_DIR/index.json.sig"
    echo "  git -C $REG_DIR add index.json index.json.sig && git -C $REG_DIR commit -m 'publish: ${published[*]:-}' && git -C $REG_DIR push"
    exit 0
fi
"$KEYGEN" sign --in "$REG_DIR/index.json" --key "$KEY" --out "$REG_DIR/index.json.sig"
git -C "$REG_DIR" add index.json index.json.sig
if git -C "$REG_DIR" diff --cached --quiet; then
    echo "registry already current — nothing to commit."
else
    git -C "$REG_DIR" commit -m "publish: ${published[*]:-PR merges / re-sign}"
    git -C "$REG_DIR" push
fi

# Final guard: the index MUST verify against its signature before we call it done
# — this is exactly the check that would have caught the orphaned-sig breakage.
pub="${KEY%.bin}.pub"
if [ -f "$pub" ] && "$KEYGEN" verify --in "$REG_DIR/index.json" \
        --sig "$REG_DIR/index.json.sig" --pub "$(cat "$pub")" >/dev/null 2>&1; then
    echo "signature verifies against index ✓"
else
    echo "⚠ post-sign signature verify did not pass — DO NOT rely on this registry; investigate." >&2
fi

echo
echo "verifying — coverage check should now be clean:"
"$here/scripts/check_registry_coverage.sh"
