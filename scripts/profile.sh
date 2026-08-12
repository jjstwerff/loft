#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Sampling profiler for loft — "which line is this run spending its time in?"
#
#   scripts/profile.sh -- --interpret --check prog.loft     # profile a loft run
#   scripts/profile.sh --annotate -- --check prog.loft      # + source-line view
#   scripts/profile.sh --calls -- --check prog.loft         # + who called the hot fn
#   scripts/profile.sh --no-cache -- --check prog.loft      # compile, don't reload
#   scripts/profile.sh --no-warm -- prog.loft               # don't pre-build (native)
#   scripts/profile.sh --keep -- …                          # keep perf.data
#
# Everything after `--` is passed to the loft binary unchanged.
#
# Four defaults here are the difference between a useful profile and a
# misleading one, so they are not options:
#
#   SELF TIME, not inclusive.  loft's hot paths are recursive tree walkers
#   (`scopes::scan`, `use_analysis::collect_defs`).  Inclusive time attributes
#   ~100% to the walker at the root and names nothing; self time names the
#   function actually burning the cycles.  Pass --calls when you then want to
#   know who calls it.
#
#   FRAME-POINTER unwinding, not DWARF.  `--call-graph=dwarf` copies stack
#   memory per sample; against a walker that recurses hundreds deep that is slow
#   AND silently truncates the chains you came for.  The profiling profile is
#   built with `-Cforce-frame-pointers=yes` so `fp` unwinding is exact and cheap.
#
#   The `profiling` cargo profile, not `release`.  Same optimisation, plus the
#   line tables `perf annotate` needs.  Release stays byte-identical (RELEASE.md
#   pins its sha; `make speed` measures it).
#
#   THE BUILD IS NOT THE RUN.  loft's default backend is the COMPILER: `loft
#   prog.loft` generates Rust, shells out to rustc, and runs the binary it built.
#   perf follows forks, so recording that command records the BUILD — rustc, LLVM
#   and lld take the whole top of the profile, and the few samples that ARE the
#   program come back as bare hex because the binary is stripped.  A native run is
#   therefore built once unprofiled (`--no-warm` opts out) and profiled with
#   `--native-debug`, and what is left of rustc is reported.
set -uo pipefail
cd "$(dirname "$0")/.."

ANNOTATE=0; CALLS=0; KEEP=0; NOCACHE=0; WARM=1; FREQ=${LOFT_PROFILE_FREQ:-499}
while [ $# -gt 0 ]; do
  case "$1" in
    --annotate) ANNOTATE=1; shift;;
    --calls)    CALLS=1; shift;;
    --keep)     KEEP=1; shift;;
    --no-cache) NOCACHE=1; shift;;
    --no-warm)  WARM=0; shift;;
    --freq)     FREQ="$2"; shift 2;;
    --)         shift; break;;
    # Anchored on the text, not a line range: adding a usage line must not
    # silently truncate --help.
    -h|--help)  sed -n '/^# Sampling profiler/,/^# Everything after/p' "$0" | sed 's/^# \?//'; exit 0;;
    *) echo "profile.sh: unknown option '$1' (did you forget '--' before the loft args?)" >&2; exit 2;;
  esac
done
if [ $# -eq 0 ]; then
  echo "profile.sh: nothing to profile — pass loft's arguments after '--'" >&2
  echo "  e.g. scripts/profile.sh -- --interpret --check prog.loft" >&2
  exit 2
fi

command -v perf >/dev/null 2>&1 || {
  echo "profile.sh: perf is not installed." >&2
  echo "  Debian/Ubuntu: sudo apt-get install linux-tools-common linux-tools-\$(uname -r)" >&2
  exit 1
}

# perf needs permission to sample. Report the exact fix rather than the kernel's
# bare 'Permission denied', which names neither the knob nor the value.
PARANOID=$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo 4)
if [ "$PARANOID" -gt 1 ] 2>/dev/null; then
  echo "profile.sh: perf_event_paranoid is $PARANOID — sampling a user process needs <= 1." >&2
  echo "  One-time, persists across reboots:" >&2
  echo "    echo 'kernel.perf_event_paranoid = 1' | sudo tee /etc/sysctl.d/99-perf.conf" >&2
  echo "    sudo sysctl --system" >&2
  echo "  Or for this boot only:  sudo sysctl -w kernel.perf_event_paranoid=1" >&2
  exit 1
fi

OUT="${LOFT_PROFILE_DIR:-${TMPDIR:-/tmp}}/loft-profile"
mkdir -p "$OUT"
DATA="$OUT/perf.data"

echo "── building (profiling profile: release + line tables, frame pointers) ──" >&2
RUSTFLAGS="${RUSTFLAGS:-} -Cforce-frame-pointers=yes" \
  cargo build --profile profiling --bin loft >&2 || exit 1
BIN=target/profiling/loft

# ── is this a native run, i.e. one that rustc has to build first? ─────────────
#
# Every loft subcommand is the FIRST positional token (`src/main.rs`: "every
# subcommand is the FIRST positional, never a later one"), so a plain program run
# is exactly "the first positional is a .loft path" — minus the modes that stop
# before rustc is ever reached.  The value-taking options are stepped over so
# their argument is not mistaken for the script.
#
# Getting this wrong is not silent: the rustc-share guard after recording says so.

# Set before the warm-up, not just before recording: the warm-up only warms the
# right build if it runs in the same environment as the run it is warming for.
if [ "$NOCACHE" = 1 ]; then export LOFT_NO_CACHE=1; fi

NATIVE=1
skip_value=0
first_positional=""
for a in "$@"; do
  if [ "$skip_value" = 1 ]; then skip_value=0; continue; fi
  case "$a" in
    --path|--project|--lib|--log-conf|--timeout)                 skip_value=1;;
    --interpret|--check|--dump|--repl)                           NATIVE=0;;
    --native-emit*|--native-wasm*|--html*|--native-android*)      NATIVE=0;;
    -*) ;;
    *)  [ -z "$first_positional" ] && first_positional="$a";;
  esac
done
case "$first_positional" in *.loft) ;; *) NATIVE=0;; esac

if [ "$NATIVE" = 1 ]; then
  # `--native-debug` is the lever that survives the binary cache, and the only
  # one: the cache key hashes THIS FLAG (src/main.rs), not the environment, so
  # `LOFT_NATIVE_KEEP_SYMBOLS=1` on its own hands back an already-cached STRIPPED
  # binary and changes nothing.  The flag keeps symbols, emits DWARF line tables,
  # and preserves the generated .rs that `--annotate` needs to show a source line.
  # It adds debug info without touching optimisation, so the run profiled is the
  # run that was asked for.  It goes in FRONT: loft forwards every flag AFTER the
  # script path to the program, so appending it would hand it to the program.
  has_nd=0
  for a in "$@"; do [ "$a" = "--native-debug" ] && has_nd=1; done
  [ "$has_nd" = 0 ] && set -- --native-debug "$@"

  if [ -n "${LOFT_NATIVE_NO_CACHE:-}" ]; then
    echo "note: LOFT_NATIVE_NO_CACHE is set, so the build cannot be cached —" >&2
    echo "      rustc will compile inside the sampled window." >&2
  elif [ "$WARM" = 1 ]; then
    # One unprofiled run to get rustc's work into the binary cache, so recording
    # covers the program rather than the compiler.  It really does run the
    # program, side effects and all — hence the announcement, and --no-warm.
    echo "── warming (building the native binary; your program runs once, unprofiled) ──" >&2
    WARMLOG="$OUT/warm.log"
    if ! "$BIN" "$@" >/dev/null 2>"$WARMLOG"; then
      echo "note: the warm-up run exited non-zero. Its last stderr lines:" >&2
      tail -5 "$WARMLOG" >&2
    fi
    rm -f "$WARMLOG"
  fi
fi

echo "── recording (perf, ${FREQ}Hz) ──" >&2
perf record -F "$FREQ" --call-graph=fp -o "$DATA" -- "$BIN" "$@" >/dev/null
rc=$?
if [ ! -s "$DATA" ]; then
  echo "profile.sh: perf wrote no samples (exit $rc) — the run may have been too short to sample." >&2
  exit 1
fi

# A percentage computed from a handful of samples is noise wearing a number's
# clothes — at 50 samples a 2% row is one sample, and `--annotate` will annotate
# whatever won the coin toss (it has picked a 1-sample `getenv` from libc).  The
# count is printed rather than the rows filtered, because the fix is a longer run
# or a higher --freq, not a quieter report.
REPORT=$(perf report -i "$DATA" --stdio --no-children -g none --percent-limit 1 2>/dev/null)
SAMPLES=$(printf '%s\n' "$REPORT" | sed -n 's/^# Samples: *\([0-9KMG.]*\).*/\1/p' | head -1)

echo
echo "════ self time — ${SAMPLES:-?} samples (where the cycles actually burn) ════"
SELF=$(printf '%s\n' "$REPORT" | grep -vE '^#|^$' | head -20)
echo "$SELF"

# A native profile can still be a profile of the BUILD — the warm-up was skipped,
# the binary cache was disabled, or the source changed between the two runs.  A
# top full of rustc/LLVM/lld reads exactly like a hot program (plausible symbols,
# real percentages), so the share is measured and named rather than left to be
# recognised.  `--sort comm` is what separates them: rustc's own threads are
# `rustc`, LLVM's codegen units `opt cgu.N`, the linker `rust-lld`.
BUILD_PCT=$(perf report -i "$DATA" --stdio --no-children -g none --sort comm --percent-limit 0 \
  2>/dev/null | awk '
    /^[[:space:]]*[0-9]/ {
      pct = $1; sub("%", "", pct)
      comm = $0; sub("^[[:space:]]*[0-9.]+%[[:space:]]+", "", comm)
      if (comm ~ /^(rustc|rust-lld|ld|cc|cc1|cc1plus|collect2|opt cgu)/) s += pct
    }
    END { printf "%.0f", s + 0 }')
if [ "${BUILD_PCT:-0}" -ge 20 ] 2>/dev/null; then
  echo
  echo "⚠  ${BUILD_PCT}% of this profile is the BUILD (rustc / LLVM / lld), not your program." >&2
  echo "   The native binary was compiled inside the sampled window. Drop --no-warm," >&2
  echo "   or unset LOFT_NATIVE_NO_CACHE, so the build is cached before recording." >&2
elif [ "$NATIVE" = 1 ]; then
  echo
  # The program's command name is not fixed — a cache hit runs `<stem>-<hash>`,
  # a fresh compile runs `loft_native_bin_<pid>` — so it is described by what it
  # is not, rather than by a name that would be wrong half the time.
  echo "(the 'loft' rows are the front end — parse, IR, codegen, cache lookup;"
  echo " the other command is your compiled program, its fns named n_<yours>)"
fi

# A profile of a CACHE HIT looks like a profile of a compile, and reads as one:
# same command, same output, a tenth of the time, and a flat graph that blames
# the store loader.  loft answers a second run of an unchanged file from the
# startup cache, so this is the normal outcome of profiling the same file twice
# — which is how it wastes an afternoon rather than announcing itself.
if echo "$SELF" | grep -qE "startup_cache|ir_read::open_bundle|warm_load"; then
  echo
  echo "⚠  this run was answered from the STARTUP CACHE, not compiled." >&2
  echo "   You are profiling the cache loader. Re-run with --no-cache, or point" >&2
  echo "   at a file whose content has changed since the last run." >&2
fi

if [ "$CALLS" = 1 ]; then
  echo
  echo "════ callers of the hot functions ════"
  perf report -i "$DATA" --stdio --no-children -g graph,0.5,caller --percent-limit 3 2>/dev/null |
    grep -vE '^#|^$' | head -60
fi

if [ "$ANNOTATE" = 1 ]; then
  HOT=$(printf '%s\n' "$SELF" | head -1 | sed 's/.*\] //')
  echo
  echo "════ source lines in: $HOT ════"
  perf annotate -i "$DATA" --stdio -l --percent-limit 1 "$HOT" 2>/dev/null | head -40
fi

echo
if [ "$KEEP" = 1 ]; then
  echo "perf.data kept at $DATA — 'perf report -i $DATA' for the interactive view."
else
  rm -f "$DATA"
  echo "(--keep retains perf.data for 'perf report'; --annotate for source lines; --calls for callers; --no-cache to compile rather than reload; --no-warm to skip the native pre-build)"
fi
