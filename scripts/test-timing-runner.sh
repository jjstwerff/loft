#!/bin/bash
# Target runner (`CARGO_TARGET_<TRIPLE>_RUNNER`) recording each TEST's CPU time.
#
# nextest runs ONE TEST PER PROCESS and passes `--exact <name>`, so the child's rusage is
# exactly that test's cost.  That is why this needs no per-binary instrumentation and
# reaches all 230 binaries — including the 130 that never mention `loft::`, which a
# library-side hook could not have covered.
#
# CPU rather than wall, for the reasons `tests/common/timing.rs` sets out: wall under load
# measures contention (a 255x inflation measured on one test), and re-running a test alone
# measures the BUILD (19.1s vs 0.096s for the same test).
#
# Unarmed it execs straight through — one extra `exec` and nothing written.
set -u
if [ -z "${LOFT_TEST_TIMING:-}" ]; then
  exec "$@"
fi

# nextest invokes every binary twice with `--list` to enumerate tests before running any.
# Those are not tests; recording them would add ~400 rows of noise per run and, worse,
# would attribute the enumeration cost to whichever binary happened to be listed.
for a in "$@"; do
  [ "$a" = "--list" ] && exec "$@"
done

bin=$1
name=""
prev=""
for a in "$@"; do
  [ "$prev" = "--exact" ] && name=$a
  prev=$a
done
[ -n "$name" ] || name="(whole-binary)"

# `/usr/bin/time` reports SECONDS; the row is milliseconds, so the conversion happens here
# rather than in the format string — `%e000` looks like a scale and is string
# concatenation, which silently produced `0.00000` for every row on the first attempt.
tmp=$(mktemp) || exec "$@"
/usr/bin/time -o "$tmp" -f "%e %U %S" "$@"
rc=$?
# Column order matches tests/common/timing.rs so scripts/test-timing.py reads both:
# wall_ms, own_ms, kids_ms, cpu_ms, label.  `kids` is 0 because here the child IS the unit.
awk -v n="$(basename "$bin")::$name" \
    '{printf "%.1f\t%.1f\t0.0\t%.1f\t%s\n", $1*1000, ($2+$3)*1000, ($2+$3)*1000, n}' \
    "$tmp" >> "$LOFT_TEST_TIMING"
rm -f "$tmp"
exit $rc
