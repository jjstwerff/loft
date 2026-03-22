#!/usr/bin/env bash
# Run the loft benchmark suite.
# NOT a CI suite — run manually for performance comparison.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOFT="${LOFT_BIN:-loft}"
STDLIB_PATH="${LOFT_STDLIB:-$REPO_ROOT/}"

SKIP_PYTHON=0
SKIP_WASM=0
NO_BUILD=0
WARMUP=0
ONLY=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-python) SKIP_PYTHON=1 ;;
    --skip-wasm)   SKIP_WASM=1 ;;
    --no-build)    NO_BUILD=1 ;;
    --warmup)      WARMUP=1 ;;
    --only)        ONLY="$2"; shift ;;
    *) echo "Unknown flag: $1"; exit 1 ;;
  esac
  shift
done

extract_ms() {
  # Extract trailing "time: Xms" from output
  grep -oE 'time: [0-9]+ms' | tail -1 | grep -oE '[0-9]+'
}

run_bench() {
  local dir="$1"
  local name
  name="$(basename "$dir")"

  if [[ -n "$ONLY" && "$name" != "$ONLY" ]]; then
    return
  fi

  local py_ms="-" li_ms="-" ln_ms="-" lw_ms="-" rs_ms="-"

  # Python
  if [[ $SKIP_PYTHON -eq 0 && -f "$dir/bench.py" ]]; then
    [[ $WARMUP -eq 1 ]] && python3 "$dir/bench.py" > /dev/null 2>&1 || true
    py_ms=$(python3 "$dir/bench.py" 2>/dev/null | extract_ms || echo "-")
  fi

  # loft interpreter
  if [[ -f "$dir/bench.loft" ]]; then
    [[ $WARMUP -eq 1 ]] && "$LOFT" run --path "$STDLIB_PATH" "$dir/bench.loft" > /dev/null 2>&1 || true
    li_ms=$("$LOFT" run --path "$STDLIB_PATH" "$dir/bench.loft" 2>/dev/null | extract_ms || echo "-")
  fi

  # loft native
  if [[ $NO_BUILD -eq 0 && -f "$dir/bench.loft" ]]; then
    "$LOFT" build --native --path "$STDLIB_PATH" "$dir/bench.loft" -o "$dir/bench_bin" > /dev/null 2>&1 || true
  fi
  if [[ -f "$dir/bench_bin" ]]; then
    [[ $WARMUP -eq 1 ]] && "$dir/bench_bin" > /dev/null 2>&1 || true
    ln_ms=$("$dir/bench_bin" 2>/dev/null | extract_ms || echo "-")
  fi

  # loft wasm
  if [[ $SKIP_WASM -eq 0 ]]; then
    if [[ $NO_BUILD -eq 0 && -f "$dir/bench.loft" ]]; then
      "$LOFT" build --wasm --path "$STDLIB_PATH" "$dir/bench.loft" -o "$dir/bench.wasm" > /dev/null 2>&1 || true
    fi
    if [[ -f "$dir/bench.wasm" ]] && command -v wasmtime > /dev/null 2>&1; then
      [[ $WARMUP -eq 1 ]] && wasmtime "$dir/bench.wasm" > /dev/null 2>&1 || true
      lw_ms=$(wasmtime "$dir/bench.wasm" 2>/dev/null | extract_ms || echo "-")
    fi
  fi

  # Rust
  if [[ $NO_BUILD -eq 0 && -f "$dir/bench.rs" ]]; then
    rustc -O -o "$dir/bench_rs_bin" "$dir/bench.rs" > /dev/null 2>&1 || true
  fi
  if [[ -f "$dir/bench_rs_bin" ]]; then
    [[ $WARMUP -eq 1 ]] && "$dir/bench_rs_bin" > /dev/null 2>&1 || true
    rs_ms=$("$dir/bench_rs_bin" 2>/dev/null | extract_ms || echo "-")
  fi

  printf "%-20s %-12s %-13s %-13s %-13s %-10s\n" \
    "$name" "${py_ms:+${py_ms}ms}" "${li_ms:+${li_ms}ms}" \
    "${ln_ms:+${ln_ms}ms}" "${lw_ms:+${lw_ms}ms}" "${rs_ms:+${rs_ms}ms}"
}

printf "%-20s %-12s %-13s %-13s %-13s %-10s\n" \
  "bench" "python" "loft-interp" "loft-native" "loft-wasm" "rust"
printf '%s\n' "$(printf '%-20s' '' | tr ' ' '-')$(printf '%-12s' '' | tr ' ' '-')$(printf '%-13s' '' | tr ' ' '-')$(printf '%-13s' '' | tr ' ' '-')$(printf '%-13s' '' | tr ' ' '-')$(printf '%-10s' '' | tr ' ' '-')"

for d in "$SCRIPT_DIR"/*/; do
  [[ -d "$d" ]] && run_bench "$d"
done
