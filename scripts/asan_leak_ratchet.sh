#!/usr/bin/env bash
# @PLN85 — the interpreter LEAK RATCHET gate.
#
# Runs an ASan-instrumented binary under `detect_leaks=1` and asserts the number
# of Direct leak ROOTS does not exceed a documented, SHRINKING baseline. The
# residual roots are the `skip_free`-orphan text temps (the p329/p330 generic
# tuple-of-text returns) that are interpreter-only orphans (native drops them via
# RAII). A NEW leak bumps the count past the baseline and fails the job; fixing a
# baseline leaker lets `LEAK_BASELINE` ratchet DOWN — reaching 0 turns
# `detect_leaks=1` into a plain zero-leak gate and the memory model is CI-verified.
#
# Why a COUNT, not an LSan allowlist: every residual text leak shares the same
# stack frames (`loft::fill::append_text` <- `execute_argv`), so a `leak:<frame>`
# suppression cannot tell a known leaker from a new one — it would suppress both.
# A total-count baseline gates the aggregate instead, so any new leak is caught.
# The ONE genuinely distinguishable class — ir_read's intentional bounded
# `Box::leak` of &'static IR names — IS frame-suppressed (.github/lsan_suppressions.txt).
#
# CALIBRATION: LSan frame/root counts differ Linux-vs-macOS, so the exact baseline
# MUST be read off the FIRST CI run's "Direct leak roots" line and pinned into
# `LEAK_BASELINE` in the workflow. Until pinned, set it generously and treat the
# first run as report-only.
#
#   Usage: LEAK_BASELINE=N [LSAN_OPTIONS=suppressions=...] asan_leak_ratchet.sh <bin> [args...]
set -u
BIN=${1:?usage: asan_leak_ratchet.sh <asan-binary> [args...]}
shift
: "${LEAK_BASELINE:?set LEAK_BASELINE to the pinned Direct-leak-root count}"

if [ ! -x "$BIN" ]; then
  echo "::error::leak-ratchet binary not found or not executable: '$BIN' (glob matched nothing?)"
  exit 2
fi

log=$(mktemp)
# `|| true`: LSan aborts the process non-zero when unsuppressed leaks remain — we
# do our own count-vs-baseline verdict, so the raw exit is expected and ignored.
ASAN_OPTIONS="detect_leaks=1:${ASAN_OPTIONS:-}" "$BIN" "$@" >"$log" 2>&1 || true
roots=$(grep -c '^Direct leak' "$log" || true)

echo "=== ASan leak ratchet ==="
echo "Direct leak roots (ir_read suppressed): $roots"
echo "baseline:                               $LEAK_BASELINE"

if [ "$roots" -gt "$LEAK_BASELINE" ]; then
  echo "::error::interpreter leak roots ($roots) exceed baseline ($LEAK_BASELINE) — a NEW leak was introduced"
  echo "--- leak owners (dedup, ir_read/main/std filtered) ---"
  # Demangle if a filter is available (frames may be Rust v0-mangled, e.g.
  # `_RNv..._4loft4fill11append_text`); the COUNT above is header-based so it
  # needs no demangling. Match demangled `loft::` OR mangled `4loft`.
  demangle() { if command -v rustfilt >/dev/null; then rustfilt; \
    elif command -v c++filt >/dev/null; then c++filt; else cat; fi; }
  awk '/^Direct leak/{p=1} p; /^$/{p=0}' "$log" \
    | sed -E 's/^[[:space:]]*#[0-9]+ 0x[0-9a-f]+ in //; s/\+0x.*$//' \
    | demangle \
    | grep -E 'loft::|4loft' | grep -vE 'ir_read|loft::main|4loft4main|__rust' \
    | sort | uniq -c | sort -rn | head -30
  exit 1
fi

if [ "$roots" -lt "$LEAK_BASELINE" ]; then
  echo "::warning::leak roots ($roots) are BELOW baseline ($LEAK_BASELINE) — a leaker was fixed; ratchet LEAK_BASELINE down to $roots"
fi

echo "leak ratchet OK ($roots <= $LEAK_BASELINE)"
