#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# One-pass-find-all-problems workflow (see doc/claude/TESTING.md).
#
# Default mode: runs `cargo test --release --no-fail-fast` in the
# background, tees the raw log to /tmp/loft_test.<id>.log (or $1), and
# lets you get on with other work.  The summary writes to
# /tmp/loft_problems.txt (or $2) when the run finishes.  Avoids the
# fix-one-rerun-see-next loop that pays the compile + test-startup
# cost on every iteration.
#
# `<id>` is a per-checkout tag derived from the repo root, so two
# working trees (e.g. sibling agent checkouts) can run this script
# concurrently without sharing pid/log/summary files — a shared pid
# file let one tree's --wait consume the other's run.  The summary
# keeps the documented stable path PLUS the per-checkout copy; every
# mode prints the exact paths it used.
#
# Peek mode (no compile): `./scripts/find_problems.sh --peek` inspects
# the in-flight log and prints any failures
# discovered so far.  Shows last script run before a SIGSEGV so
# wrap-suite crashes point at the specific .loft file that blew up.
#
# Usage:
#   ./scripts/find_problems.sh                         # run+wait (foreground)
#   ./scripts/find_problems.sh --bg                    # run in background
#   ./scripts/find_problems.sh /tmp/log /tmp/problems  # custom paths
#   ./scripts/find_problems.sh --peek                  # in-flight peek
#   ./scripts/find_problems.sh --wait                  # wait for a --bg run
#
# Reach for this any time a refactor is expected to surface multiple
# failures (e.g. after renaming a widely-used API, touching parser
# code paths, or replacing a native's signature).  For focused work
# on ONE test family, prefer a prefix filter instead:
#   cargo test --release --test issues q3_to_json
#
# Rule: never run `cargo test --release` (the full suite) in the
# foreground.  Always go through `--bg` so the blocking run does
# not occupy the terminal for 60-90 s.  `cargo clippy` and single-
# file tests stay foreground.
set -euo pipefail

# Cache clean/release rebuilds with sccache when present (no-op otherwise).
source "$(dirname "${BASH_SOURCE[0]}")/sccache_env.sh"

# Per-checkout tag: stable for a given working tree, distinct across trees.
REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
REPO_TAG=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)

# Keep native/wasm compiles OFF a small /tmp tmpfs.  The native test harness
# writes generated `.rs` + cached binaries via `scratch_dir()` (LOFT_TMPDIR,
# else temp_dir()), and rustc/rust-lld put their link intermediates under
# TMPDIR — both default to /tmp, which on this box is a ~7.5G tmpfs.  Parallel
# native compiles then exhaust it and the linker dies with SIGBUS (signal 7),
# manifesting as flaky, unrelated test failures.  Redirect both to a disk-backed
# dir, per-checkout (persistent, so the content-hash native cache survives
# run-to-run).  std::env::temp_dir() honours TMPDIR on Unix, so this one var also
# moves every `temp_dir()` user (cross_mode/exit_codes/html_wasm).  MUST live
# OUTSIDE the repo: a `target/`-relative TMPDIR breaks the package/registry tests
# (they build fixtures in temp_dir and package/extract them — anything under
# `target/` is excluded, so loft.toml goes missing).  /var/tmp is disk-backed.
LOFT_TEST_SCRATCH="/var/tmp/loft-test-scratch-$REPO_TAG"
mkdir -p "$LOFT_TEST_SCRATCH"
export TMPDIR="$LOFT_TEST_SCRATCH"
export LOFT_TMPDIR="$LOFT_TEST_SCRATCH"
LOG_DEFAULT=/tmp/loft_test.$REPO_TAG.log
OUT_DEFAULT=/tmp/loft_problems.$REPO_TAG.txt
OUT_STABLE=/tmp/loft_problems.txt
PID_FILE=/tmp/loft_test.$REPO_TAG.pid

# Refresh every derived artefact that the test suite depends on before
# running it.  There are three classes of stale artefact, each of which
# has caused a cascade of misleading test failures in the past:
#
#   1. Sibling cdylibs under lib/*/native/ — loaded by
#      `extensions::load_all`, linked by `--native`.  Source gains a
#      symbol → rustc: "cannot find function X in crate Y".
#   2. Test fixture cdylibs under tests/lib/*/native/ — native_loader
#      tests detect this and panic with a clear message, but one
#      detection panic per test is still one per test.
#   3. The wasm32-unknown-unknown rlib used by html_wasm tests —
#      the html_wasm suite checks staleness before running.
#
# Cargo is incremental, so each step is ~free on a clean tree.  Logs
# go to /tmp/loft_cdylib.log so the test log stays focused on test
# output.  Failures here print a warning but do not stop the test run;
# the pre-existing in-test detection will surface the underlying
# problem with a specific rebuild command.
# Per-step wall-clock timings accumulate in /tmp/loft_timings.txt and
# stream live to stderr so a slow `--bg` start is visible.  Format:
#   `  cdylib lib/server/native       3.4s`  (always >= 0.1s precision)
# A `=== Wall-clock timing summary ===` block prints at run/wait end.
TIMINGS_FILE=/tmp/loft_timings.$REPO_TAG.txt

# Pick the test runner.  cargo-nextest parallelises at the test level
# (cargo-test only at the binary level), giving 2-3x faster wall-clock
# on the loft suite.  Falls back to plain `cargo test` if nextest
# isn't installed.  `--profile default` matches `.config/nextest.toml`'s
# fail-fast=off-by-default-flag-set / no-retries / immediate-failure
# settings.
test_runner_cmd() {
  if cargo nextest --version >/dev/null 2>&1; then
    echo "cargo nextest run --release --no-fail-fast --status-level fail"
  else
    echo "cargo test --release --no-fail-fast"
  fi
}

# Sweep the disk-backed fixture scratch before a full run: tests write
# fixtures (and their .loft/cache native binaries) under target/test-tmp —
# per-run artifacts that otherwise accumulate run over run.  Owned by THIS
# repo's tests only, so the sweep can be unconditional.
sweep_test_tmp() {
  rm -rf "$(dirname "$0")/../target/test-tmp" 2>/dev/null || true
}

# Run all rebuilds in parallel.  Each cargo invocation has fixed
# startup overhead (~0.05–0.7 s on a no-op rebuild); doing them in
# parallel collapses the serial 1.5–2 s wall-clock to whatever the
# slowest single rebuild costs.  When something genuinely needs
# rebuilding the wins are larger (10s of seconds).
#
# Per-step timings are written to per-PID files and concatenated
# back into TIMINGS_FILE in submission order so the summary stays
# stable.
rebuild_one() {
  local label="$1" dir="$2" cmd="$3" log="$4" timing_file="$5"
  local start_ns end_ns elapsed_ms
  start_ns=$(date +%s%N)
  if ! bash -c "$cmd" >> "$log" 2>&1; then
    echo "warning: rebuild of $dir failed — see $log" >&2
  fi
  end_ns=$(date +%s%N)
  elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))
  printf '  %-44s %6d.%03ds\n' "$label" \
    "$(( elapsed_ms / 1000 ))" "$(( elapsed_ms % 1000 ))" \
    > "$timing_file"
}

rebuild_native_cdylibs() {
  local repo_root
  repo_root=$(cd "$(dirname "$0")/.." && pwd)
  local log=/tmp/loft_cdylib.$REPO_TAG.log
  : > "$log"
  : > "$TIMINGS_FILE"
  local any_src_cdylib=0
  local timing_dir
  timing_dir=$(mktemp -d /tmp/loft_timings.XXXXXX)
  local rebuild_start_ns
  rebuild_start_ns=$(date +%s%N)

  echo "=== rebuild_native_cdylibs (parallel; per-step timings) ===" >&2

  local jobs=()
  local timing_files=()
  local idx=0

  schedule() {
    local label="$1" dir="$2" cmd="$3"
    local tf="$timing_dir/$idx"
    timing_files+=("$tf")
    idx=$(( idx + 1 ))
    rebuild_one "$label" "$dir" "$cmd" "$log" "$tf" &
    jobs+=($!)
  }

  # 1. Sibling cdylibs under lib/*/native/
  for manifest in "$repo_root"/lib/*/native/Cargo.toml; do
    [[ -f "$manifest" ]] || continue
    any_src_cdylib=1
    local dir
    dir=$(dirname "$manifest")
    local rel="${dir#"$repo_root"/}"
    echo "== rebuild $dir ==" >> "$log"
    schedule "cdylib $rel" "$dir" "cd '$dir' && cargo build --release -q"
  done

  # 2. Test fixture cdylibs under tests/lib/*/native/
  while IFS= read -r manifest; do
    [[ -f "$manifest" ]] || continue
    # Skip git-ignored manifests: orphaned generated fixtures left over from old
    # runs (e.g. tests/test_native_pkg/ from a prior plugin-ABI) are not part of
    # the tracked build and may no longer compile against the current tree.
    git -C "$repo_root" check-ignore -q "$manifest" 2>/dev/null && continue
    any_src_cdylib=1
    local dir
    dir=$(dirname "$manifest")
    local rel="${dir#"$repo_root"/}"
    echo "== rebuild $dir ==" >> "$log"
    schedule "cdylib $rel" "$dir" "cd '$dir' && cargo build --release -q"
  done < <(find "$repo_root/tests" -name Cargo.toml -not -path '*/target/*' 2>/dev/null)

  # 3. The wasm32-unknown-unknown rlib used by the html_wasm suite.
  #    Only rebuild if the target directory already exists — the very
  #    first run lets the --html driver build it so we don't impose a
  #    wasm-target install on developers who never touch the HTML gate.
  if [[ -d "$repo_root/target/wasm32-unknown-unknown" ]]; then
    echo "== rebuild wasm32-unknown-unknown rlib ==" >> "$log"
    schedule "wasm32 rlib" "$repo_root" \
      "cd '$repo_root' && cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random -q"
  fi

  # Wait for all parallel rebuilds; `wait` exits after the slowest.
  for pid in "${jobs[@]}"; do wait "$pid"; done

  # Collect timings — one file per scheduled job.  Concatenate in
  # scheduled order so the summary table reads top-to-bottom by
  # submission, even though jobs finished in arbitrary order.
  for tf in "${timing_files[@]}"; do
    if [[ -f "$tf" ]]; then
      cat "$tf" >> "$TIMINGS_FILE"
      cat "$tf" >&2
    fi
  done
  rm -rf "$timing_dir"

  local rebuild_end_ns
  rebuild_end_ns=$(date +%s%N)
  local rebuild_ms=$(( (rebuild_end_ns - rebuild_start_ns) / 1000000 ))
  printf '  %-44s %6d.%03ds\n' \
    "(rebuild_native_cdylibs total wall-clock)" \
    "$(( rebuild_ms / 1000 ))" "$(( rebuild_ms % 1000 ))" \
    | tee -a "$TIMINGS_FILE" >&2

  if [[ "$any_src_cdylib" -eq 0 ]]; then
    echo "no sibling cdylibs found — skipping freshness step" >&2
  fi
}

# Extract a compact failure summary from the raw log.
# $1: log path, $2: output path
summarise() {
  local log="$1" out="$2"
  do_summarise "$log" "$out"
  # Keep the documented stable path readable too (last writer wins when
  # several checkouts run concurrently; the per-checkout file is the
  # authoritative one and every mode prints its exact path).
  if [[ "$out" != "$OUT_STABLE" ]]; then
    cp -f "$out" "$OUT_STABLE" 2>/dev/null || true
  fi
}

do_summarise() {
  local log="$1" out="$2"
  {
    echo "=== Test binaries that reported FAILED ==="
    # Both runner formats: cargo test ("test ... FAILED") and nextest
    # ("        FAIL [   0.040s] (2256/2332) loft::wrap stack_trace_script"
    # + the closing "Summary [...] N failed") — the summary previously
    # missed every nextest failure and reported a failing run as clean.
    grep -a -E "^test .* FAILED$|^[[:space:]]*FAIL \[|failed;|[0-9]+ failed" "$log" \
      | grep -av " 0 failed" || echo "(none)"
    echo
    echo "=== Test stdout blocks for FAILED tests ==="
    grep -a -B1 -A10 "^---- " "$log" || echo "(none)"
    echo
    echo "=== SIGSEGV / signal crashes (with last context) ==="
    # For each SIGSEGV line, include the last 15 lines of context
    # before it — typically captures the last `run "tests/scripts/..."`
    # line so crashes point at a specific .loft file.
    if grep -aq "signal:" "$log"; then
      awk '
        /signal:/ {
          for (i = NR - 15; i < NR; i++) if (i > 0 && buf[i]) print buf[i]
          print "    *** " $0
          print "    ---"
        }
        { buf[NR] = $0 }
      ' "$log"
    else
      echo "(none)"
    fi
    echo
    echo "=== cargo-level target failures (compile or link) ==="
    grep -a -B1 -A3 "error: test failed\|error: .* target\(s\) failed" "$log" || echo "(none)"
    echo
    echo "=== panic! / thread panics (inline) ==="
    grep -a -B1 -A3 "thread .* panicked at" "$log" | head -80 || echo "(none)"
    # If a wrap-suite test SIGSEGV'd, cargo captured its stdout
    # into the void — re-run wrap with --nocapture to recover
    # the last `run "tests/scripts/..."` print before the crash.
    if grep -aq "wrap.* signal:" "$log" || grep -aq "test failed.*--test wrap" "$log"; then
      echo
      echo "=== wrap-suite SIGSEGV rerun with --nocapture ==="
      echo "(to recover the crashing script name)"
      cargo test --release --test wrap loft_suite -- --nocapture --test-threads=1 2>&1 \
        | grep -E '^(run |thread |test |error:|Caused|  process|Warning: [0-9]+ stores)' \
        | tail -50 || echo "(rerun failed)"
    fi
  } > "$out"
}

# `--peek`: look at the in-flight log without starting a run.
if [[ "${1:-}" == "--peek" ]]; then
  LOG="${2:-$LOG_DEFAULT}"
  if [[ ! -f "$LOG" ]]; then
    echo "no log at $LOG yet — run without --peek to start a fresh pass" >&2
    exit 1
  fi
  running="no"
  if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    running="yes (pid $(cat "$PID_FILE"))"
  fi
  echo "=== in-flight peek (log: $LOG, $(wc -l < "$LOG") lines, running=$running) ==="
  failures=$(grep -a -E "^test .* FAILED$|^[[:space:]]*FAIL \[" "$LOG" || true)
  segfaults=$(grep -a "signal:" "$LOG" || true)
  if [[ -z "$failures" && -z "$segfaults" ]]; then
    echo "no failures yet"
    echo "current tail:"
    tail -5 "$LOG"
    exit 0
  fi
  if [[ -n "$failures" ]]; then
    echo "$failures"
    echo
    grep -a -B1 -A10 "^---- " "$LOG" || true
  fi
  if [[ -n "$segfaults" ]]; then
    echo
    echo "SIGSEGV detected — last context before crash:"
    awk '
      /signal:/ {
        for (i = NR - 15; i < NR; i++) if (i > 0 && buf[i]) print buf[i]
        print "    *** " $0
      }
      { buf[NR] = $0 }
    ' "$LOG"
  fi
  exit 0
fi

# `--wait`: wait for a background run to finish, then summarise.
if [[ "${1:-}" == "--wait" ]]; then
  LOG="${2:-$LOG_DEFAULT}"
  OUT="${3:-$OUT_DEFAULT}"
  if [[ ! -f "$PID_FILE" ]]; then
    echo "no background run found (expected $PID_FILE)" >&2
    exit 1
  fi
  pid=$(cat "$PID_FILE")
  echo "waiting for cargo test pid $pid..."
  while kill -0 "$pid" 2>/dev/null; do sleep 2; done
  rm -f "$PID_FILE"
  summarise "$LOG" "$OUT"
  echo
  echo "=== Wall-clock timing summary ==="
  if [[ -f "$TIMINGS_FILE" ]]; then
    cat "$TIMINGS_FILE"
  else
    echo "(no timings recorded — older log)"
  fi
  echo "wrote problems summary to $OUT"
  wc -l "$OUT"
  exit 0
fi

# `--bg`: start the run in the background and return immediately.
if [[ "${1:-}" == "--bg" ]]; then
  LOG="${2:-$LOG_DEFAULT}"
  OUT="${3:-$OUT_DEFAULT}"
  if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    echo "a background run is already in flight (pid $(cat "$PID_FILE"))" >&2
    echo "use --peek to inspect or --wait to block until it finishes" >&2
    exit 1
  fi
  # Remove stale bytecode caches so tests always compile fresh.
  find tests/ -name '*.loftc' -delete 2>/dev/null || true
  find /tmp -maxdepth 1 -name '*.loftc' -delete 2>/dev/null || true
  # Refresh sibling cdylibs before forking; see rebuild_native_cdylibs
  # for the rationale.  Runs in the foreground so the caller sees build
  # errors immediately, not 90 s later inside the test log.
  rebuild_native_cdylibs
  # Tee via a subshell so the script returns after backgrounding.
  # `|| true` after the runner so summarise still fires when tests
  # fail (cargo's non-zero exit would otherwise short-circuit `set -e`
  # in the subshell, leaving /tmp/loft_problems.txt unwritten).
  RUNNER="$(sweep_test_tmp; test_runner_cmd)"
  echo "test runner: $RUNNER"
  (
    start_ns=$(date +%s%N)
    eval "$RUNNER" > "$LOG" 2>&1 || true
    end_ns=$(date +%s%N)
    elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))
    printf '  %-44s %6d.%03ds\n' "$RUNNER" \
      "$(( elapsed_ms / 1000 ))" "$(( elapsed_ms % 1000 ))" \
      >> "$TIMINGS_FILE"
    summarise "$LOG" "$OUT"
    rm -f "$PID_FILE"
  ) > /dev/null 2>&1 &
  # ^ stdio detached: the subshell writes only to files, but an inherited
  #   stdout keeps a caller's pipe (`--bg | tail`) open until the whole
  #   suite ends — silently serialising the "background" run.
  echo "$!" > "$PID_FILE"
  echo "background run started (pid $!), log: $LOG, summary on finish: $OUT"
  echo "use --peek to inspect in flight, --wait to block until done"
  exit 0
fi

# Default: foreground run — stream output AND write summary.
LOG="${1:-$LOG_DEFAULT}"
OUT="${2:-$OUT_DEFAULT}"
find tests/ -name '*.loftc' -delete 2>/dev/null || true
find /tmp -maxdepth 1 -name '*.loftc' -delete 2>/dev/null || true
rebuild_native_cdylibs
RUNNER="$(sweep_test_tmp; test_runner_cmd)"
echo "test runner: $RUNNER"
fg_start_ns=$(date +%s%N)
# `|| true` so test failures don't short-circuit `set -e` and skip
# the post-run summary block.
eval "$RUNNER" 2>&1 | tee "$LOG" || true
fg_end_ns=$(date +%s%N)
fg_elapsed_ms=$(( (fg_end_ns - fg_start_ns) / 1000000 ))
printf '  %-44s %6d.%03ds\n' "$RUNNER" \
  "$(( fg_elapsed_ms / 1000 ))" "$(( fg_elapsed_ms % 1000 ))" \
  >> "$TIMINGS_FILE"
summarise "$LOG" "$OUT"
echo
echo "=== Wall-clock timing summary ==="
cat "$TIMINGS_FILE"
echo "wrote problems summary to $OUT"
wc -l "$OUT"
