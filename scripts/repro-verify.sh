#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN78 step 7 — rebuild a published release on this platform and compare.
#
# The published sha256 says "this is the artifact the maintainer uploaded".  Rebuilding
# it from source with the compiler that release recorded upgrades that to "this is the
# artifact the source produces" — the claim a user actually wants, and the only one that
# survives a compromised build machine.
#
# ## Why this needs no pinned toolchain and no container
#
# rlibs and binaries are byte-identical only when the compiler is, and the repo pins
# `channel = "stable"` on purpose (@PLAN53 — track the newest rustc, never freeze).
# Those look contradictory and are not: the VERIFIER pins, per release, and throws the
# toolchain away afterwards.  `rustup toolchain install <exact> --profile minimal` into
# a private `RUSTUP_HOME` takes ~15s and ~600MB, and leaves the caller's default
# toolchain untouched.
#
# That same private root is also the canonicalisation lever.  A build embeds absolute
# paths (`$CARGO_HOME/registry/...`, `$RUSTUP_HOME/toolchains/...` — 192 strings for
# loft), so two machines agree only if those paths agree.  Putting RUSTUP_HOME,
# CARGO_HOME and the source under one FIXED root makes them agree without a container —
# which matters, since a container is a dependency not every verifier has.
#
# Usage:
#   scripts/repro-verify.sh                 # newest release, this platform's target
#   scripts/repro-verify.sh --version 2026.7.3
#   scripts/repro-verify.sh --keep          # leave the work tree for inspection
#
# Exit: 0 identical · 1 differs · 3 cannot verify (says why; never a silent pass).
set -uo pipefail

REPO="loft-lang/loft"
# FIXED on purpose — see the canonicalisation note above.  Overridable only to run two
# verifications side by side, which costs byte-identity and says so.
ROOT="${LOFT_REPRO_ROOT:-/tmp/loft-repro}"
VERSION=""
KEEP=0

die() { echo "repro-verify: $*" >&2; exit 3; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:-}"; [ -n "$VERSION" ] || die "--version needs a value"; shift 2 ;;
    --keep)    KEEP=1; shift ;;
    -h|--help) sed -n '5,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)         die "unknown option: $1" ;;
  esac
done

command -v rustup >/dev/null || die "needs rustup to fetch the release's exact rustc"
command -v gh     >/dev/null || die "needs gh to download the release"
command -v unzip  >/dev/null || die "needs unzip"
if command -v sha256sum >/dev/null; then SHA="sha256sum"; else SHA="shasum -a 256"; fi

# The target this runner natively builds — matched to release.yml's matrix, because a
# cross-compiled binary is not expected to equal a natively built one.
host=$(rustc -vV | sed -n 's/^host: //p')
case "$host" in
  x86_64-unknown-linux-gnu)  TARGET="x86_64-unknown-linux-musl" ;;
  x86_64-apple-darwin)       TARGET="x86_64-apple-darwin" ;;
  aarch64-apple-darwin)      TARGET="aarch64-apple-darwin" ;;
  x86_64-pc-windows-msvc)    TARGET="x86_64-pc-windows-msvc" ;;
  *) die "no published target for host $host" ;;
esac

if [ -z "$VERSION" ]; then
  VERSION=$(gh release view -R "$REPO" --json tagName -q '.tagName' 2>/dev/null | sed 's/^v//')
  [ -n "$VERSION" ] || die "cannot determine the latest release"
fi
NAME="loft-$VERSION-$TARGET"
echo "== verifying $NAME =="

rm -rf "$ROOT"; mkdir -p "$ROOT"
[ "$KEEP" = 1 ] || trap 'rm -rf "$ROOT"' EXIT INT TERM

# 1. The published bundle, and what it says it was built with.
( cd "$ROOT" && gh release download "v$VERSION" -R "$REPO" -p "$NAME.zip" -O bundle.zip --clobber ) \
  || die "cannot download $NAME.zip"
unzip -q "$ROOT/bundle.zip" -d "$ROOT/published" || die "cannot unpack $NAME.zip"
pub_root="$ROOT/published/$NAME"; [ -d "$pub_root" ] || pub_root="$ROOT/published"

info="$pub_root/BUILD-INFO"
if [ ! -f "$info" ]; then
  # Deliberately exit 3, not 0.  A release with no BUILD-INFO records no compiler, so
  # nothing here can be compared — and reporting that as a pass would be the exact
  # failure this whole chain exists to prevent: a check that sounds like more than it did.
  die "v$VERSION ships no BUILD-INFO — it predates reproducible builds and cannot be verified"
fi
RUSTC_VER=$(sed -n 's/^rustc = //p' "$info" | sed 's/ .*//')
[ -n "$RUSTC_VER" ] || die "BUILD-INFO names no rustc"
echo "   rustc $RUSTC_VER (from the bundle's BUILD-INFO)"

# 2. The source the release was cut from.  `git archive` of the tag, taken from the
#    release itself so the verifier trusts one artifact set, not a second checkout.
( cd "$ROOT" && gh release download "v$VERSION" -R "$REPO" -p "loft-$VERSION-src.zip" -O src.zip --clobber ) \
  || die "v$VERSION ships no source archive — nothing to rebuild from"
unzip -q "$ROOT/src.zip" -d "$ROOT/srcroot" || die "cannot unpack the source archive"
SRC="$ROOT/src"; mv "$ROOT/srcroot/loft-$VERSION" "$SRC" 2>/dev/null || mv "$ROOT/srcroot" "$SRC"

# 3. The exact compiler, in a private root that is also the canonical path.
export RUSTUP_HOME="$ROOT/rustup" CARGO_HOME="$ROOT/cargo"
echo "   installing rustc $RUSTC_VER (throwaway)"
rustup toolchain install "$RUSTC_VER" --profile minimal --target "$TARGET" >/dev/null 2>&1 \
  || die "cannot install rustc $RUSTC_VER"

# 4. Rebuild, exactly as make-release.sh does.
echo "   building $TARGET"
( cd "$SRC" && CARGO_INCREMENTAL=0 rustup run "$RUSTC_VER" cargo build --release --bin loft --target "$TARGET" ) \
  || die "the rebuild failed"

exe="loft"; case "$TARGET" in *windows*) exe="loft.exe" ;; esac
rebuilt="$SRC/target/$TARGET/release/$exe"
published="$pub_root/bin/$exe"
[ -f "$rebuilt" ]   || die "no rebuilt binary at $rebuilt"
[ -f "$published" ] || die "no published binary at $published"

a=$($SHA "$rebuilt"   | cut -d' ' -f1)
b=$($SHA "$published" | cut -d' ' -f1)
echo "   rebuilt   $a"
echo "   published $b"
if [ "$a" = "$b" ]; then
  echo "IDENTICAL — v$VERSION/$TARGET reproduces from source"
  exit 0
fi
# Size first: a size match with differing bytes points at embedded paths or timestamps,
# a size mismatch at a genuinely different build.  Saying which saves the next reader a
# bisect.
sa=$(wc -c < "$rebuilt"); sb=$(wc -c < "$published")
echo "DIFFERS — rebuilt $sa bytes, published $sb bytes"
[ "$sa" = "$sb" ] && echo "  (same size: look for embedded absolute paths or timestamps)"
exit 1
