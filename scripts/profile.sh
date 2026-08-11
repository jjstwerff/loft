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
#   scripts/profile.sh --keep -- …                          # keep perf.data
#
# Everything after `--` is passed to the loft binary unchanged.
#
# Three defaults here are the difference between a useful profile and a
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
set -uo pipefail
cd "$(dirname "$0")/.."

ANNOTATE=0; CALLS=0; KEEP=0; NOCACHE=0; FREQ=${LOFT_PROFILE_FREQ:-499}
while [ $# -gt 0 ]; do
  case "$1" in
    --annotate) ANNOTATE=1; shift;;
    --calls)    CALLS=1; shift;;
    --keep)     KEEP=1; shift;;
    --no-cache) NOCACHE=1; shift;;
    --freq)     FREQ="$2"; shift 2;;
    --)         shift; break;;
    -h|--help)  sed -n '5,12p' "$0" | sed 's/^# \?//'; exit 0;;
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

echo "── recording (perf, ${FREQ}Hz) ──" >&2
if [ "$NOCACHE" = 1 ]; then export LOFT_NO_CACHE=1; fi
perf record -F "$FREQ" --call-graph=fp -o "$DATA" -- "$BIN" "$@" >/dev/null
rc=$?
if [ ! -s "$DATA" ]; then
  echo "profile.sh: perf wrote no samples (exit $rc) — the run may have been too short to sample." >&2
  exit 1
fi

echo
echo "════ self time (where the cycles actually burn) ════"
SELF=$(perf report -i "$DATA" --stdio --no-children -g none --percent-limit 1 2>/dev/null |
  grep -vE '^#|^$' | head -20)
echo "$SELF"

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
  HOT=$(perf report -i "$DATA" --stdio --no-children -g none --percent-limit 1 2>/dev/null |
        grep -vE '^#|^$' | head -1 | sed 's/.*\] //')
  echo
  echo "════ source lines in: $HOT ════"
  perf annotate -i "$DATA" --stdio -l --percent-limit 1 "$HOT" 2>/dev/null | head -40
fi

echo
if [ "$KEEP" = 1 ]; then
  echo "perf.data kept at $DATA — 'perf report -i $DATA' for the interactive view."
else
  rm -f "$DATA"
  echo "(--keep retains perf.data for 'perf report'; --annotate for source lines; --calls for callers; --no-cache to compile rather than reload)"
fi
