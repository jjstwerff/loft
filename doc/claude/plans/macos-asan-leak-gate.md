<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# RESOLVED: macOS ASan interpreter leak gate

**Status: fixed on a real macOS box (aarch64-apple-darwin), 2026-07-20.** Kept as the record of
root cause + fix because the failure is Darwin-only and the original handoff carried a misdiagnosis.

## Outcome

The macOS `asan-leak-gate` is green with NO real leak masked and NO weakening of the Linux leg.
Verified locally on macOS: the per-file scan is `0 leaking file(s) of 513`, and the full nextest
leak gate is `1629 passed, 0 leaked` (incl. `library_suite` + `loft_suite`).

## Root cause (two real issues — and one debunked theory)

1. **Symbolizer choice was wrong for macOS.** The gate forced `llvm-symbolizer` on *both* legs. On
   macOS that is actively harmful: llvm-symbolizer leaves the **dyld system frames unsymbolized**
   (bare `libdyld.dylib+0x…`), so the benign thread-TLS class can never match a suppression. The
   macOS system symbolizer `atos` is the right one — it resolves `dyld::ThreadLocalVariables::…`
   AND, although it leaves Rust names *mangled* (`..._4loft4fill11append_text`), a **bare-name**
   suppression (`leak:append_text`) is a substring of both the mangled and the demangled forms, so
   one suppression file works on both legs. Linux keeps llvm-symbolizer (it needs the demangling).
2. **The suppression missed the INDIRECT dyld-TLS allocation.** `leak:addTermFunc` matched only the
   128-byte direct term-func registration; the 6984-byte *indirect* block is allocated by
   `dyld::ThreadLocalVariables::instantiateVariable` via `_tlv_get_addr` — a different frame.
   Matching the enclosing dyld type — `leak:ThreadLocalVariables` — covers both in one line.
3. **DEBUNKED: "macOS ignores the suppression file for `library_suite`."** It does not. Under `atos`
   the file is honored fine (`addTermFunc`/`ThreadLocalVariables` show up in "Suppressions used").
   The earlier evidence was just issue #1 (forced llvm-symbolizer left the frame unsymbolized).

None of this is a loft leak. The dyld thread-TLS class is a known LeakSanitizer-on-macOS false
positive (dyld tears the TLS list down *after* LSan's at-exit scan); it is macOS-only and not ours
to free. The Rust classes are the same benign ones Linux already suppresses.

## The fix (three files)

- `.github/lsan_suppressions.txt` — switched every `module::fn` entry to a **bare name** that
  matches both mangled + demangled (`fill::append_text`→`append_text`, `ops::format_text`→
  `format_text`, `ops::format_long`→`format_long`, `database::snapshot`→`snapshot`,
  `registry_index::http_get_bytes`→`http_get_bytes`); `ir_read`/`ir_schema`/`text_tl_fmt` were
  already bare; `addTermFunc`→`ThreadLocalVariables` (covers the indirect TLS block too).
- `.github/workflows/miri.yml` — the symbolizer step is now **Linux-only** (`if: runner.os ==
  'Linux'`); macOS intentionally leaves `ASAN_SYMBOLIZER_PATH` unset so the runtime uses `atos`.

## Local repro on macOS (aarch64), nightly toolchain

`cargo`/`rustc` on this box resolve to the *stable* toolchain binaries directly (a `rust-toolchain.toml`
pins `channel = "stable"`), so the rustup proxy is needed to force nightly: use the proxy at
`/opt/homebrew/Cellar/rustup/<ver>/bin/cargo` (put it first on PATH) with `cargo +nightly …`.
`llvm-symbolizer` for the Linux-equivalent experiment: `brew install llvm` → `$(brew --prefix llvm)/bin/llvm-symbolizer`.

```bash
export PATH="/opt/homebrew/Cellar/rustup/$(ls /opt/homebrew/Cellar/rustup)/bin:$PATH"   # rustup proxy → +nightly works
RUSTFLAGS='-Zsanitizer=address -Cforce-frame-pointers=yes' cargo +nightly build --release --bin loft --target aarch64-apple-darwin
ln -sfn "$PWD/default" target/aarch64-apple-darwin/release/default

# (B) per-file scan — atos (do NOT set ASAN_SYMBOLIZER_PATH):
env -u ASAN_SYMBOLIZER_PATH ABIN=target/aarch64-apple-darwin/release/loft \
  LSAN_OPTIONS="suppressions=$PWD/.github/lsan_suppressions.txt" \
  bash scripts/asan_leak_scan.sh tests/scripts/*.loft tests/docs/*.loft   # → 0 leaking of 513

# (A) nextest leak gate — atos:
RUSTFLAGS='-Zsanitizer=address -Cforce-frame-pointers=yes' cargo +nightly build --release --tests --target aarch64-apple-darwin
env -u ASAN_SYMBOLIZER_PATH RUSTFLAGS='-Zsanitizer=address -Cforce-frame-pointers=yes' \
  ASAN_OPTIONS='detect_leaks=1' LSAN_OPTIONS="suppressions=$PWD/.github/lsan_suppressions.txt" \
  cargo +nightly nextest run --profile ci --release --no-fail-fast \
  --target aarch64-apple-darwin --lib --test issues --test wrap --test strings --test frame_vars \
  -E 'not (test(fill_rs_up_to_date) | test(n9_generated_fill_matches_src) | test(native_rs_functions_up_to_date) | test(deep_nesting_guard))'
```

Note: the `plan59_par_worker_over_wrapper_promoted_callee` test fails LOCALLY only — it spawns the
plain `target/release/loft` (a stale non-ASan build predating current `default/`) and *skips* on the
real ASan CI job (which never builds that binary). Not leak-related.

## Residual (optional, not blocking)

`http_get_bytes` is the one class that could be *eliminated* rather than suppressed (drop the
process-lifetime ureq/rustls client before exit; it is reachable-at-exit, so Linux never even flags
it). Low value; the other classes are intentional (`Box::leak` interner), OS-owned (dyld TLS), or
deliberately declined (@PLN102 fault-path text). See @PLN54.

## Guardrails honored

- Linux leak gate + ratchet baseline (0) unchanged; bare names are substrings of the demangled Linux
  frames too, and `ThreadLocalVariables` is a macOS-only no-op on Linux.
- Only benign owners suppressed. `src/ops.rs` shift fix untouched.
