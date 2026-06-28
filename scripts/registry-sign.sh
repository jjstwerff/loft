#!/usr/bin/env bash
# registry-sign.sh — SEE exactly what you are about to sign into the loft
# registry index, then sign it.  The human review IS the trust root
# (REGISTRY_BOOTSTRAP.md § Step 4 — "Why laptop signing"): a signature is only
# as trustworthy as the maintainer's look at what changed.
#
# For every added/changed package version it shows the LIBRARY RELEASE it points
# at (repo + tag), optionally that release's notes, and DOWNLOADS the tarball to
# confirm its sha256 actually matches the index — then asks before signing.
#
#   scripts/registry-sign.sh [opts]      # run in / pointed at a loft-lang/registry checkout
#     --registry-dir DIR  registry checkout (default: $PWD; if that isn't a
#                         registry, the live loft-lang/registry is cloned)
#     --pr N              clone the registry + check out registry PR #N, then sign
#     --key FILE          Ed25519 private key file (the local-key path)
#                         (default: $LOFT_REGISTRY_KEY or
#                          ~/.loft/trust-root/registry-signing-key.bin)
#     --yubikey           sign ON-CARD with the YubiKey (PIV slot 9C, Ed25519) —
#                         the private key never leaves the card.  Use this when the
#                         trusted signer is a YubiKey (K_yubiA/B), e.g. on a laptop
#                         that has no trusted *file* key.  (or LOFT_REGISTRY_SIGNER=yubikey)
#     --since REF         diff index.json against this git ref (default: auto —
#                         HEAD if you have uncommitted edits, else HEAD~1)
#     --notes             also print each release's full notes (gh release body)
#     --no-download       skip the tarball sha256 re-download check
#     --no-push           sign + commit locally but do NOT push (keeps the clone)
#     --message MSG       commit message (default: "sign: …"; registry_maintain
#                         passes "publish: <libs>")
#     --yes               skip the confirm prompt (scripted use)
#
# Two signing paths (both end at the same trust gate — see below):
#   * default (file key): the LOCAL key signs; a YubiKey TOUCH is the human-presence
#     confirmation — on touch the YubiKey types its one-time OTP (a long ModHex
#     string), read as the "yes".  Waits 10s, else falls back to a typed 'yes'.
#     LOFT_YUBIKEY_PREFIX (your OTP public-id, first ~12 chars) pins the gate to
#     YOUR key.
#   * --yubikey (on-card): the YubiKey itself signs (PIV 9C, Ed25519); its PIN +
#     touch during the sign ARE the presence proof.  Override the sign step with
#     LOFT_YUBIKEY_SIGN_CMD (gets $LOFT_SIG_IN / $LOFT_SIG_OUT), else a default
#     `pkcs11-tool --mechanism EDDSA` call (module auto-found or
#     LOFT_YUBIKEY_PKCS11_MODULE; PIV 9C → id 02 or LOFT_YUBIKEY_PIV_ID).
# `--yes` skips the confirmation.  The TRUST GATE always runs: whatever was signed
# must verify under a key in src/registry_keys.rs::TRUSTED_PUBLIC_KEYS or it is
# NOT committed/pushed — so a wrong key/module fails safe.
#
# On confirm it signs, commits index.json + index.json.sig together (so HEAD's
# index always matches its signature — #377), pushes, and (for an auto-clone)
# deletes the temp checkout.  A failed push keeps the clone so the signed commit
# is never lost.
#
# Refuses to sign (non-zero exit) on a sha256 mismatch, invalid JSON, or a "no"
# at the prompt.  Needs: python3, gh (for notes), and target/release/loft-keygen.
set -euo pipefail

REG_DIR="$PWD"; REG_GIVEN=0; PR=""; SINCE=""; NOTES=0; DOWNLOAD=1; YES=0; PUSH=1; MSG=""
KEY="${LOFT_REGISTRY_KEY:-$HOME/.loft/trust-root/registry-signing-key.bin}"
YUBIKEY="${LOFT_REGISTRY_SIGNER:+$([ "$LOFT_REGISTRY_SIGNER" = yubikey ] && echo 1)}"; YUBIKEY="${YUBIKEY:-0}"
while [ $# -gt 0 ]; do
    case "$1" in
        --registry-dir) REG_DIR="$2"; REG_GIVEN=1; shift;;
        --pr)           PR="$2"; shift;;
        --key)          KEY="$2"; shift;;
        --yubikey)      YUBIKEY=1;;
        --since)        SINCE="$2"; shift;;
        --notes)        NOTES=1;;
        --no-download)  DOWNLOAD=0;;
        --no-push)      PUSH=0;;
        --message)      MSG="$2"; shift;;
        --yes)          YES=1;;
        -h|--help)      sed -n '2,30p' "$0"; exit 0;;
        *) echo "unknown argument: $1" >&2; exit 2;;
    esac
    shift
done

here=$(cd "$(dirname "$0")/.." && pwd)
KG="$here/target/release/loft-keygen"
if [ ! -x "$KG" ]; then
    echo "building loft-keygen (release) ..." >&2
    (cd "$here" && cargo build --release --bin loft-keygen --features registry >/dev/null)
fi

# On-card sign: the YubiKey itself (PIV slot 9C, Ed25519) produces the raw 64-byte
# Ed25519 signature over <in> → <out>.  The private key never leaves the card; the
# touch/PIN happen during this call.  The result is verified against the embedded
# trust roots by the trust gate below, so a wrong module/slot/key fails SAFE (no
# push).  Override the whole step with LOFT_YUBIKEY_SIGN_CMD (gets $LOFT_SIG_IN /
# $LOFT_SIG_OUT); else a default pkcs11-tool EDDSA call, with the module auto-found
# (override LOFT_YUBIKEY_PKCS11_MODULE) and PIV 9C → PKCS#11 id 02 (override
# LOFT_YUBIKEY_PIV_ID).
yubikey_sign() {  # <in> <out>
    local in="$1" out="$2"
    if [ -n "${LOFT_YUBIKEY_SIGN_CMD:-}" ]; then
        LOFT_SIG_IN="$in" LOFT_SIG_OUT="$out" bash -c "$LOFT_YUBIKEY_SIGN_CMD" \
            || { echo "yubikey: LOFT_YUBIKEY_SIGN_CMD failed" >&2; exit 4; }
        return
    fi
    command -v pkcs11-tool >/dev/null \
        || { echo "yubikey: pkcs11-tool not found (install OpenSC) or set LOFT_YUBIKEY_SIGN_CMD" >&2; exit 4; }
    local module="${LOFT_YUBIKEY_PKCS11_MODULE:-}"
    if [ -z "$module" ]; then
        for m in /usr/local/lib/libykcs11.dylib /opt/homebrew/lib/libykcs11.dylib \
                 /usr/local/lib/opensc-pkcs11.so /opt/homebrew/lib/opensc-pkcs11.so \
                 /usr/lib/x86_64-linux-gnu/libykcs11.so /usr/lib/x86_64-linux-gnu/opensc-pkcs11.so; do
            [ -e "$m" ] && { module="$m"; break; }
        done
    fi
    [ -n "$module" ] \
        || { echo "yubikey: no PKCS#11 module found — set LOFT_YUBIKEY_PKCS11_MODULE or LOFT_YUBIKEY_SIGN_CMD" >&2; exit 4; }
    echo "  on-card sign via $module (PIV 9C / id ${LOFT_YUBIKEY_PIV_ID:-02}) — enter PIN + touch when it flashes" >&2
    pkcs11-tool --module "$module" --sign --mechanism EDDSA \
        --id "${LOFT_YUBIKEY_PIV_ID:-02}" --login \
        --input-file "$in" --output-file "$out" \
        || { echo "yubikey: pkcs11-tool sign failed — check module/id/PIN, or set LOFT_YUBIKEY_SIGN_CMD" >&2; exit 4; }
    [ -s "$out" ] || { echo "yubikey: produced an empty signature" >&2; exit 4; }
}

# No explicit checkout and the cwd isn't a registry → clone the live registry, so
# the routine runs standalone (inspection, or signing a PR via --pr).
CLONED=0; SIGNED=0; PUSHED=0
if [ "$REG_GIVEN" != 1 ] && [ ! -f "$REG_DIR/index.json" ]; then
    REG_DIR=$(mktemp -d -t loft-registry.XXXXXX)
    echo "cloning loft-lang/registry → $REG_DIR ..." >&2
    # SSH first: it pushes with your key and never prompts, so the later
    # commit+push succeeds unattended.  GitHub no longer accepts an HTTPS
    # password at the git push prompt, so an HTTPS clone needs a working
    # credential helper — fall back to it (gh, then plain HTTPS) only when SSH
    # is unavailable.
    git clone -q git@github.com:loft-lang/registry.git "$REG_DIR" 2>/dev/null \
        || gh repo clone loft-lang/registry "$REG_DIR" -- -q 2>/dev/null \
        || git clone -q https://github.com/loft-lang/registry "$REG_DIR"
    CLONED=1
    if [ -n "$PR" ]; then
        echo "checking out registry PR #$PR ..." >&2
        git -C "$REG_DIR" fetch -q origin "pull/$PR/head:pr-$PR"
        git -C "$REG_DIR" checkout -q "pr-$PR"
    fi
fi
cleanup() {
    rm -f "${PREV:-}"
    # Delete an auto-clone when nothing was signed, or it was signed AND pushed.
    # Keep it only when there's an unpushed signed commit (push failed / --no-push).
    if [ "$CLONED" = 1 ]; then
        if [ "$SIGNED" != 1 ] || [ "$PUSHED" = 1 ]; then
            rm -rf "$REG_DIR"
        else
            echo "kept temp clone (unpushed signed commit): $REG_DIR" >&2
        fi
    fi
}
trap cleanup EXIT

INDEX="$REG_DIR/index.json"
[ -f "$INDEX" ] || { echo "no index.json in $REG_DIR — use --registry-dir" >&2; exit 2; }
[ "$YUBIKEY" = 1 ] || [ -f "$KEY" ] \
    || { echo "signing key not found: $KEY — use --key / set LOFT_REGISTRY_KEY (or --yubikey)" >&2; exit 2; }

# Shape gate: refuse to even look at a non-JSON index.
python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$INDEX" \
    || { echo "index.json is NOT valid JSON — refusing." >&2; exit 1; }

# Auto-pick the diff base: uncommitted edits → compare to HEAD; otherwise the
# change you're signing is the last commit, so compare to HEAD~1.
if [ -z "$SINCE" ]; then
    if git -C "$REG_DIR" diff --quiet HEAD -- index.json 2>/dev/null; then
        SINCE=HEAD~1
    else
        SINCE=HEAD
    fi
fi
git -C "$REG_DIR" rev-parse "$SINCE" >/dev/null 2>&1 || SINCE=""   # no such ref / no git

echo "================  WHAT YOU ARE SIGNING  ================"
echo "registry : $REG_DIR"
echo "index    : $(wc -c < "$INDEX") bytes   sha256 $(sha256sum "$INDEX" | cut -d' ' -f1)"
echo "key      : $KEY"
echo "diff base: ${SINCE:-<none — treating every version as new>}"
echo

echo "----  1. raw change to index.json  ----"
if [ -n "$SINCE" ]; then
    git -C "$REG_DIR" --no-pager diff "$SINCE" -- index.json || true
else
    echo "  (no git history to diff against)"
fi
echo

echo "----  2. library releases + 3. tarball integrity  ----"
PREV=$(mktemp)   # removed by cleanup() on exit
if [ -n "$SINCE" ]; then
    git -C "$REG_DIR" show "$SINCE:index.json" > "$PREV" 2>/dev/null || printf '{}' > "$PREV"
else
    printf '{}' > "$PREV"
fi

set +e
NOTES="$NOTES" DOWNLOAD="$DOWNLOAD" python3 - "$PREV" "$INDEX" <<'PY'
import json, sys, os, re, hashlib, urllib.request, subprocess
def load(p):
    try:
        with open(p) as f: return json.load(f)
    except Exception:
        return {}
prev, cur = load(sys.argv[1]), load(sys.argv[2])
show_notes = os.environ.get("NOTES") == "1"
download   = os.environ.get("DOWNLOAD", "1") == "1"
pp = (prev.get("packages") or {}) if isinstance(prev, dict) else {}
cp = cur.get("packages") or {}
rel_re = re.compile(r"github\.com/([^/]+)/([^/]+)/releases/download/([^/]+)/")

changes = []
for name in sorted(cp):
    for ver, meta in sorted((cp[name].get("versions") or {}).items()):
        old = (pp.get(name, {}).get("versions") or {}).get(ver)
        if old != meta:
            changes.append((name, ver, meta, old is None))

if not changes:
    print("  (no added or changed versions vs the diff base)")
failures = []  # (name, ver, reason) — collected so the end-of-run summary names each
for name, ver, meta, is_new in changes:
    url, sha, size = meta.get("url", ""), meta.get("sha256", ""), meta.get("size")
    print(f"  [{'NEW' if is_new else 'CHANGED'}]  {name} {ver}")
    print(f"        url      : {url}")
    print(f"        sha256   : {sha}")
    print(f"        size     : {size} bytes")
    biny = meta.get("binaries") or {}
    if biny:
        print(f"        binaries : {', '.join(sorted(biny))}")
    m = rel_re.search(url)
    if m:
        owner, repo, tag = m.group(1), m.group(2), m.group(3)
        print(f"        release  : {owner}/{repo} @ {tag}")
        try:
            out = subprocess.run(
                ["gh", "release", "view", tag, "--repo", f"{owner}/{repo}",
                 "--json", "name,publishedAt,body"],
                capture_output=True, text=True, timeout=30)
            if out.returncode == 0:
                rel = json.loads(out.stdout)
                print(f"        published: {rel.get('publishedAt','?')}  \"{rel.get('name') or tag}\"")
                if show_notes and (rel.get("body") or "").strip():
                    for line in rel["body"].rstrip().splitlines():
                        print(f"           | {line}")
            else:
                err = (out.stderr.strip().splitlines() or ["not found"])[0]
                print(f"        notes    : (gh: {err})")
        except Exception as e:
            print(f"        notes    : (unavailable: {e})")
    if download and url and sha:
        try:
            data = urllib.request.urlopen(url, timeout=60).read()
            got = hashlib.sha256(data).hexdigest()
            if got == sha:
                print(f"        VERIFY   : sha256 MATCH ({len(data)} bytes downloaded)")
                if size is not None and len(data) != size:
                    print(f"        VERIFY   : size MISMATCH !!  downloaded {len(data)} != declared {size}")
                    failures.append((name, ver,
                        f"size mismatch: downloaded {len(data)} bytes, index declares {size}"))
            else:
                print(f"        VERIFY   : sha256 MISMATCH !!  declared {sha[:16]}… got {got[:16]}…")
                failures.append((name, ver,
                    "sha256 mismatch — the uploaded release tarball does not match the index entry "
                    f"(index says {sha[:16]}…, the download hashes to {got[:16]}…).\n"
                    "             Cause: the release artifact is stale, or the lib's source on main "
                    "moved past its released tag.\n"
                    "             Fix: re-cut the release so its bytes match `loft package` at the tag "
                    "(bump the version + re-release if main has changed).\n"
                    f"             url: {url}"))
        except Exception as e:
            print(f"        VERIFY   : download FAILED: {e}")
            failures.append((name, ver,
                f"download failed ({e}) — does the release exist with the named asset?\n"
                f"             url: {url}"))
    print()

if failures:
    n = len(failures)
    print("══════════════════════════════════════════════════════════════════════")
    print("!!  INTEGRITY CHECK FAILED — NOT signing.")
    print(f"    {n} of {len(changes)} new/changed entr{'y' if n == 1 else 'ies'} "
          "failed verification:")
    print()
    for name, ver, reason in failures:
        print(f"  ✗  {name} {ver}")
        print(f"        {reason}")
    print()
    print("    Nothing was signed.  Fix the flagged artifact(s) above, then re-run.")
    print("    (Passing requires each tarball to byte-match `loft package` at its tag —")
    print("     the registry's gate-3 reproducible-build invariant.)")
    sys.exit(1)
sys.exit(0)
PY
rc=$?
set -e
if [ "$rc" -ne 0 ]; then
    # The verification block above prints a named, per-package failure summary.
    echo "registry-sign: refusing to sign — integrity check failed (see the summary above)." >&2
    exit 1
fi

echo "----  4. current signature  ----"
PUB="${KEY%.bin}.pub"
SIG="$REG_DIR/index.json.sig"
if [ -f "$SIG" ] && [ -f "$PUB" ]; then
    if "$KG" verify --in "$INDEX" --sig "$SIG" --pub "$(cat "$PUB")" >/dev/null 2>&1; then
        echo "  current index.json.sig is already VALID under this key (re-sign = no-op)"
    else
        echo "  current index.json.sig does NOT match this key/content (will be replaced)"
    fi
else
    echo "  (no current signature, or no .pub beside the key)"
fi
echo

if [ "$YES" != 1 ]; then
    [ "$PUSH" = 1 ] && verb="Sign, commit & push" || verb="Sign & commit (no push)"
    if [ "$YUBIKEY" = 1 ]; then
        # On-card: the YubiKey's PIN + touch during the sign ARE the human-presence
        # proof, so just take a typed confirm here (the card prompts next).
        echo "$verb this index — signing ON-CARD with the YubiKey."
        printf "  type 'yes' to proceed (the card will ask for PIN + touch): "
        read -r ans
        case "$ans" in yes|YES) ;; *) echo "aborted — NOT signed."; exit 1;; esac
    else
        echo "$verb this index with $(basename "$KEY")."
        # Confirmation gate.  The LOCAL key does the actual signing; a YubiKey TOUCH
        # is the human-presence confirmation (not the crypto).  On touch the YubiKey
        # types its one-time OTP — a long ModHex string + Enter — which we read as the
        # confirmation.  Wait 10s for it; otherwise fall back to a typed 'yes'.  Set
        # LOFT_YUBIKEY_PREFIX to your OTP public-id (first ~12 chars) to pin the gate
        # to YOUR key.  `--yes` skips the gate entirely (scripted use).
        printf "  Touch your YubiKey within 10s to confirm (or type 'yes'): "
        if read -r -t 10 ans; then
            echo
            case "$ans" in
                y|Y|yes|YES) echo "  confirmed (typed)";;
                *)
                    if [ "${#ans}" -ge 32 ] && { [ -z "${LOFT_YUBIKEY_PREFIX:-}" ] || \
                         case "$ans" in "${LOFT_YUBIKEY_PREFIX}"*) true;; *) false;; esac; }; then
                        echo "  confirmed (YubiKey touch)"
                    else
                        echo "aborted — NOT signed."; exit 1
                    fi
                    ;;
            esac
        else
            echo
            printf "  no YubiKey touch within 10s — type 'yes' to sign with the local key: "
            read -r ans
            case "$ans" in yes|YES) ;; *) echo "aborted — NOT signed."; exit 1;; esac
        fi
    fi
fi

if [ "$YUBIKEY" = 1 ]; then
    yubikey_sign "$INDEX" "$SIG"
else
    "$KG" sign --in "$INDEX" --key "$KEY" --out "$SIG"
fi
SIGNED=1
echo "signed: $SIG"

# Trust gate — the signature must verify under a key that CLIENTS trust
# (src/registry_keys.rs::TRUSTED_PUBLIC_KEYS), not merely under the key we
# signed with.  Signing with an untrusted key yields a sig that every
# `loft install` rejects ("registry index signature INVALID").  The old check
# verified only against ${KEY}.pub AND was skipped entirely when that file was
# absent — so a wrong/untrusted key shipped silently (broke the live index
# 2026-06-28).  Refuse to commit/push unless a trusted key validates it.
KEYS_RS="$here/src/registry_keys.rs"
if [ -f "$KEYS_RS" ]; then
    trusted_hex=$(grep -oE '0x[0-9A-Fa-f]{2}' "$KEYS_RS" | sed 's/0x//' | tr -d '\n')
    ok=0; i=0
    while [ "$i" -lt "${#trusted_hex}" ]; do
        if "$KG" verify --in "$INDEX" --sig "$SIG" --pub "${trusted_hex:$i:64}" >/dev/null 2>&1; then ok=1; break; fi
        i=$((i + 64))
    done
    if [ "$ok" != 1 ]; then
        echo "!! registry-sign: the new signature verifies under NONE of the trusted" >&2
        echo "   keys in $KEYS_RS — every 'loft install' would reject this index." >&2
        echo "   Signing key: $KEY is not a registry trust-root key." >&2
        echo "   NOT committing/pushing.  Sign with a trusted key (set LOFT_REGISTRY_KEY" >&2
        echo "   / --key), or add this key's public to TRUSTED_PUBLIC_KEYS and ship a" >&2
        echo "   new client release first." >&2
        exit 3
    fi
    echo "  trust gate OK — signature verifies under a client-trusted key"
else
    echo "  WARNING: $KEYS_RS not found — skipping trust-set verification" >&2
    [ -f "$PUB" ] && "$KG" verify --in "$INDEX" --sig "$SIG" --pub "$(cat "$PUB")"
fi

# Automated git: stage index.json AND its signature together, commit, push.
# Ed25519 is deterministic, so re-signing identical content yields the same bytes
# → nothing to commit.  #377: staging only index.json.sig while a new/dirty
# index.json sat uncommitted silently published a sig/index mismatch — the
# committed .sig verified against content that never landed, and the new version
# was never published.  Staging BOTH keeps HEAD self-consistent: its committed
# index.json always matches its committed index.json.sig.
git -C "$REG_DIR" add index.json index.json.sig
if git -C "$REG_DIR" diff --cached --quiet; then
    echo "index.json.sig unchanged — nothing to commit."
    PUSHED=1   # nothing outstanding → safe to clean up the clone
elif [ "$PUSH" = 1 ]; then
    git -C "$REG_DIR" commit -q -m "${MSG:-sign: commit index.json + regenerate index.json.sig}"
    # Push reusing the gh login as git's credential helper: no username/password
    # prompt on an HTTPS remote (the registry is often cloned over HTTPS), and
    # harmless on an SSH remote (which uses your key).  `gh release create` etc.
    # work because gh uses API auth; plain `git push` uses git's credential
    # system, which without a helper falls back to prompting — and GitHub no
    # longer accepts a password there.  GIT_TERMINAL_PROMPT=0 makes a genuinely
    # missing credential fail fast instead of hanging on an interactive prompt.
    if GIT_TERMINAL_PROMPT=0 git -C "$REG_DIR" \
        -c credential.helper='!gh auth git-credential' push -q 2>/tmp/_rs_push.$$; then
        echo "committed + pushed: $(git -C "$REG_DIR" rev-parse --short HEAD) → $(git -C "$REG_DIR" remote get-url origin 2>/dev/null)"
        PUSHED=1
    else
        echo "!! push FAILED: $(cat /tmp/_rs_push.$$ 2>/dev/null)" >&2
        echo "   signed commit kept at $REG_DIR — push it manually." >&2
        echo "   (auth? run 'gh auth setup-git' once, or use an SSH remote, then re-run.)" >&2
    fi
    rm -f "/tmp/_rs_push.$$"
else
    git -C "$REG_DIR" commit -q -m "${MSG:-sign: commit index.json + regenerate index.json.sig}"
    echo "committed (--no-push): $(git -C "$REG_DIR" rev-parse --short HEAD) at $REG_DIR — push when ready."
fi
