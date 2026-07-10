#!/usr/bin/env bash
BIN="$1"; t="$2"
raw=$(ASAN_OPTIONS="detect_leaks=1" "$BIN" --exact "$t" --test-threads=1 2>&1 | c++filt)
# From each backtrace frame line, pull the function name; keep loft frames that
# are NOT the ir_read baseline / test scaffolding; dedupe → owner fingerprint.
sig=$(echo "$raw" \
  | grep -E '^\s*#[0-9]+ 0x[0-9a-f]+ in ' \
  | sed -E 's/^\s*#[0-9]+ 0x[0-9a-f]+ in //; s/\+0x[0-9a-f]+.*$//' \
  | grep -E 'loft::' \
  | grep -vE 'ir_read|read_block|read_data_with|read_value|loft::main|__rust|as core::ops' \
  | sed -E 's/^<//; s/>::/::/' \
  | sort -u | tr '\n' ' ')
[ -n "$sig" ] && echo "$t :: $sig"
