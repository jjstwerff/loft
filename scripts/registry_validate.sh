#!/usr/bin/env bash
# Validate one REGISTRY package against the current loft + rustc toolchain.
#
# Installs the package from the live registry (the real tarball + sha256 +
# dependency resolution a user gets), builds its native crate if it ships one,
# and runs its own test suite on BOTH backends.  This validates the PUBLISHED
# artifact — not the source repo, whose CI already gates pushes — so it
# catches "the toolchain moved and the released library rotted" (the class
# behind loft-lang/loft-libs-core#14: registry random 0.2.1 was dead on the
# 2026.6 toolchain and no CI anywhere could see it).
#
# Usage:  scripts/registry_validate.sh <package>
# Env:    LOFT               loft binary to validate against (default: loft on PATH)
#         LOFT_REGISTRY_URL  respected by loft itself (defaults to the live index)
#         LOFT_TIMEOUT       per-run watchdog seconds (default 300)
#
# Exit codes: 0 = package is healthy; 1 = any step failed.
set -u

PKG="${1:?usage: registry_validate.sh <package>}"
LOFT="${LOFT:-loft}"
export LOFT_TIMEOUT="${LOFT_TIMEOUT:-300}"

fail() { echo "registry-validate: $PKG: FAIL — $*" >&2; exit 1; }
note() { echo "registry-validate: $PKG: $*"; }

# The loft commit under test — stamped into every failure so a red night names
# the exact loft that broke a package (captured before the cd into the scratch
# dir, while we are still in the loft checkout).
LOFT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
PKG_VER="?"   # filled once the installed version dir is known (step 2)

# On a test failure the raw CI log buries the assertion under cargo/rustc noise.
# Surface the salient lines three ways — a GHA ::error annotation (top of the
# run), a collapsible ::group, and the job Step Summary — each stamped with the
# package version + loft commit, so the cause is visible without log-diving.
emit_failure() {
    local backend="$1" logfile="$2" salient assertln
    salient="$(grep -avE '^[[:space:]]*(Compiling|Downloaded|Updating|Finished|Building|Fresh|Locking|Downloading) ' "$logfile" \
               | grep -aviE 'Node\.js 20|being deprecated|deprecation-of-node' | tail -40)"
    assertln="$(grep -aiE 'assertion failed|Error:|panic|^ *FAIL ' "$logfile" | head -1 | tr -d '[:cntrl:]')"
    {
        echo "::error title=registry-validate $PKG ($backend)::${assertln:-tests failed against loft $LOFT_SHA}"
        echo "::group::registry-validate $PKG $PKG_VER — $backend failure detail (loft $LOFT_SHA)"
        echo "$salient"
        echo "::endgroup::"
    } >&2
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
        {
            echo "### ❌ \`$PKG\` $PKG_VER — $backend tests failed"
            echo "_loft \`$LOFT_SHA\` · rustc \`$(rustc --version 2>/dev/null | awk '{print $2}')\`_"
            echo
            echo '```'
            echo "$salient"
            echo '```'
        } >> "$GITHUB_STEP_SUMMARY"
    fi
}

# Run one backend's test suite with output captured; on failure show the full
# output (raw log) then the distilled diagnostic block.
run_backend() {
    local backend="$1"
    local log="$SCRATCH/test-$backend.log"
    note "tests: --$backend"
    if (cd "$PKG_DIR" && LOFT_ERRORS=pretty "$LOFT" --"$backend" test) >"$log" 2>&1; then
        grep -aiE 'test result|passed' "$log" | tail -1
        return 0
    fi
    cat "$log" >&2
    emit_failure "$backend" "$log"
    return 1
}

command -v "$LOFT" >/dev/null 2>&1 || fail "loft binary '$LOFT' not found"
note "toolchain: $("$LOFT" --version 2>/dev/null | head -1) | loft@$LOFT_SHA | $(rustc --version 2>/dev/null)"

# Work in a scratch project dir so the install's .loft/api stubs and lockfile
# never land in a real tree.
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
cd "$SCRATCH" || fail "cannot enter scratch dir"

# 1. Install from the registry — exercises index fetch, tarball download,
#    sha256 verification, and dependency resolution.
note "loft install $PKG"
"$LOFT" install "$PKG" || fail "loft install failed"

# 2. Locate the installed artifact (highest version dir; hyphen separates the
#    version suffix from the name — package names themselves use underscores).
PKG_DIR="$(ls -d "$HOME/.loft/registry/$PKG"-* 2>/dev/null | sort -V | tail -1)"
[ -n "$PKG_DIR" ] && [ -d "$PKG_DIR" ] || fail "installed package dir not found under ~/.loft/registry/"
PKG_VER="$(basename "$PKG_DIR" | sed "s/^${PKG}-//")"
note "validating $PKG_DIR (version $PKG_VER)"

# 3. Build the native crate against current rustc, if the package ships one.
#    (loft can rebuild these on demand, but an explicit build gives the
#    failure its own step and a readable rustc error.)
if [ -d "$PKG_DIR/native" ]; then
    note "building native crate"
    (cd "$PKG_DIR/native" && cargo build --release) || fail "native crate build failed"
fi

# 4. Run the package's own tests on both backends.  Functional gate only —
#    no LOFT_DENY_WARNINGS: a released artifact must WORK on the new
#    toolchain; warning-cleanliness is the source repo CI's job.
if [ -d "$PKG_DIR/tests" ]; then
    run_backend interpret || fail "interpreter tests failed — see the ::group / Step Summary above"
    run_backend native    || fail "native tests failed — see the ::group / Step Summary above"
else
    # No tests in the tarball — still prove it loads: compile a bare `use`.
    note "no tests/ shipped — smoke-compiling 'use $PKG;'"
    printf 'use %s;\nfn main() { }\n' "$PKG" > smoke.loft
    "$LOFT" --interpret smoke.loft || fail "smoke program failed to run"
fi

note "OK"
