#!/usr/bin/env bash
# doc_probe_sweep — run every checked-in `.loft` file under `doc/` on the current
# build and report which ones HARD-FAULT.
#
# Why this exists: `doc/claude/plans/**/probes/` holds ~860 executable `.loft`
# files that no suite reaches.  They are the residue of finished investigations,
# and they kept working as a corpus long after their plan closed — loft#1113 (a
# SIGSEGV in a three-condition closure shape, months old) was found only because
# an unrelated change happened to walk that directory and run one.
#
# ⚠ This scores CRASH CHANNELS ONLY — exit signal, panic, timeout.  It cannot say
# whether a probe computed the right answer, because these files carry no
# expectations (that is what `scripts/probe-matrix` wants and they do not have).
# So a clean run means "nothing faulted", never "everything is correct".
#
# It is a REPORT, never a gate.  A checked-in probe may fault ON PURPOSE — some
# are named for the crash they demonstrate — so a pass/fail verdict would need a
# baseline this does not keep.  Read the list and judge.
#
# Usage:
#   scripts/doc_probe_sweep.sh [--jobs N] [--bin PATH] [--dir DIR] [--tsv OUT]
#                              [--timeout SECONDS]

set -u

root="$(cd "$(dirname "$0")/.." && pwd)"
jobs=6
bin="$root/target/release/loft"
dir="$root/doc"
tsv=""
# A bound too TIGHT manufactures faults, which is the mirror of a bound that is
# not a bound: at 20s under six-way load this sweep reported a 16s performance
# probe and a 28s parse as crashes.  Generous by default, and a timeout is
# reported as its own class rather than as a fault, because the honest reading
# of one is "re-run this alone".
inner=60
outer=120

while [ $# -gt 0 ]; do
  case "$1" in
    --jobs) jobs="$2"; shift 2 ;;
    --bin)  bin="$2";  shift 2 ;;
    --dir)  dir="$2";  shift 2 ;;
    --tsv)  tsv="$2";  shift 2 ;;
    --timeout) inner="$2"; outer=$(( $2 * 2 )); shift 2 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "doc_probe_sweep: unknown option '$1'" >&2; exit 2 ;;
  esac
done

[ -x "$bin" ] || { echo "doc_probe_sweep: no binary at $bin (cargo build --release --bin loft)" >&2; exit 2; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# `.loft` is BOTH a source extension and the name of the on-disk cache directory
# a run writes beside its script, so a bare `-name '*.loft'` matches directories
# and everything inside them.  The first run of this sweep scored 20 cache dirs
# as files and reported them all as failures.
find "$dir" -type f -name '*.loft' -not -path '*/.loft/*' | sort > "$work/given"
given=$(wc -l < "$work/given")

cat > "$work/one.sh" <<'ONE'
#!/bin/bash
# One probe → one TSV row.  The entry point is chosen by the file's own shape:
# `--interpret` on a main-less file runs nothing and `--tests` on a main-ful one
# runs only its helpers, so one shared entry point would measure neither.
f="$1"; bin="$2"; inner="$3"; outer="$4"
if grep -q '^fn main' "$f"; then set -- --interpret "$f"
else set -- --interpret --tests "$f"; fi
tmp=$(mktemp)
# Two bounds, because they catch different hangs: loft's own names the phase it
# stalled in, and the outer one covers a hang loft's bound cannot reach (a
# `rustc` child, a socket) — an outer kill scores 124, a difference like any other.
# loft's own bound aborts, so it arrives as 134 with `[timeout]` on stderr; that
# text is what tells a timeout apart from a real SIGABRT.
LOFT_TIMEOUT="$inner" timeout -k 5 "$outer" "$bin" "$@" >/dev/null 2>"$tmp" </dev/null
rc=$?
printf '%s\t%s\t%s\n' "$f" "$rc" "$(tr '\n' '\v' <"$tmp" | cut -c1-400)"
rm -f "$tmp"
ONE
chmod +x "$work/one.sh"

# `</dev/null` on the whole pipeline: xargs and its children read stdin, and a
# sweep that shares stdin with its own work list silently processes a PREFIX of
# it and then exits 0 — 51 of 186 refs, reported as a completed run.
xargs -a "$work/given" -P "$jobs" -n 1 -I{} "$work/one.sh" {} "$bin" "$inner" "$outer" \
  > "$work/results" 2>/dev/null </dev/null
processed=$(wc -l < "$work/results")

[ -n "$tsv" ] && cp "$work/results" "$tsv"

echo "== doc probe sweep =="
echo "  binary    : $bin"
echo "  given     : $given"
echo "  processed : $processed"
if [ "$given" -ne "$processed" ]; then
  echo "  ⚠ PROCESSED != GIVEN — the sweep did not run everything it was handed."
fi

# The channels, most severe first.  A signal or a panic is the interesting half;
# exit 1 is usually a REFUSAL, which for a probe testing a diagnostic is the
# passing answer, so it is counted but not listed.
awk -F'\t' '
  { n[$2]++ }
  END { for (c in n) printf "  exit %-4s : %d\n", c, n[c] }
' "$work/results" | sort -k2 -n

# A run killed by either bound is NOT counted as a fault: under parallel load a
# slow-but-correct probe crosses it, and the only honest verdict is "re-run alone".
is_timeout='($2==124) || ($2==134 && $3 ~ /\[timeout\]/)'
timeouts=$(awk -F'\t' "$is_timeout" "$work/results" | wc -l)
faults=$(awk -F'\t' "\$2!=0 && \$2!=1 && !($is_timeout)" "$work/results" | wc -l)
echo
if [ "$faults" -eq 0 ]; then
  echo "  no hard faults (signal / panic)."
else
  echo "  $faults hard fault(s) — signal or panic:"
  awk -F'\t' "\$2!=0 && \$2!=1 && !($is_timeout) {
    split(\$3, L, \"\\v\")
    printf \"    [exit %s] %s\\n\", \$2, \$1
    for (i=1; i<=3 && i<=length(L); i++) if (L[i] != \"\") printf \"        %s\\n\", substr(L[i],1,110)
  }" "$work/results"
fi
if [ "$timeouts" -gt 0 ]; then
  echo
  echo "  $timeouts hit the ${inner}s/${outer}s bound — re-run alone before reading as a hang:"
  awk -F'\t' "$is_timeout { printf \"    %s\\n\", \$1 }" "$work/results"
fi
echo
echo "  Crash channels only — these files carry no expected values, so a clean"
echo "  run means nothing faulted, not that anything computed the right answer."
