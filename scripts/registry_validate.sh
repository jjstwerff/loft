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

command -v "$LOFT" >/dev/null 2>&1 || fail "loft binary '$LOFT' not found"
note "toolchain: $("$LOFT" --version 2>/dev/null | head -1) | $(rustc --version 2>/dev/null)"

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
note "validating $PKG_DIR"

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
    note "tests: --interpret"
    (cd "$PKG_DIR" && "$LOFT" --interpret test) || fail "interpreter tests failed"
    note "tests: --native"
    (cd "$PKG_DIR" && "$LOFT" --native test) || fail "native tests failed"
else
    # No tests in the tarball — still prove it loads: compile a bare `use`.
    note "no tests/ shipped — smoke-compiling 'use $PKG;'"
    printf 'use %s;\nfn main() { }\n' "$PKG" > smoke.loft
    "$LOFT" --interpret smoke.loft || fail "smoke program failed to run"
fi

note "OK"
