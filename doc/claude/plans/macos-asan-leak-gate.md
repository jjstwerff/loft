<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Handoff: green the macOS ASan interpreter leak gate

**For a Mac agent.** This task needs a real macOS machine (aarch64-apple-darwin). It could NOT be
resolved from the Linux dev box: the failure is macOS-only and every fix attempt needs the macOS
ASan runtime to verify — a ~20-min CI round each on Linux, which is why it's handed off.

## Goal / done-bar

The nightly **`ASan interpreter leak gate (macos-latest)`** job (`.github/workflows/miri.yml`,
job `asan-leak-gate`) is RED; the same gate on **ubuntu-latest is GREEN on the same code**. Done when:
1. the macOS leg is GREEN (or a *deliberate, documented* structural change makes it so), AND
2. the Linux leg stays GREEN, AND
3. no *real* leak is masked (the enforcing guarantee is Linux's leak-gate at implicit baseline 0 +
   `scripts/asan_leak_ratchet.sh`; only benign/system/at-exit-reachable allocations may be suppressed).

This is **NOT a loft code bug** — the interpreter is leak-clean (Linux proves it). It is a **Darwin
LeakSanitizer + symbolizer infrastructure problem.** So a legitimate outcome is "make the macOS *leak*
gate informational and keep Linux as the enforcing leak gate" (see § Decision) — a Mac just lets you
try for a real fix first, and *verify* whichever path you pick before spending CI rounds.

## Branch + current state

Work branch: **`tuxedo-followups`** (this doc is on it). It carries, on top of `main`:
- a **debug-assertions shift fix** (`src/ops.rs`) — **DONE + verified, unrelated to this task; do NOT
  touch it.**
- **two INEFFECTIVE macOS-ASan attempts** (commit "macOS ASan interpreter leak gate — symbolize for
  LSan + suppress http_get_bytes"): a rustup-`llvm-tools` symbolizer step in `miri.yml` + a
  `leak:registry_index::http_get_bytes` line in `.github/lsan_suppressions.txt`. They did **not** work
  (see § What failed). **Supersede or revert them** once you have the real fix.

## What's red (confirming run 29746783986)

Three leak owners, all macOS-only:
| Owner | Where | What it is |
|---|---|---|
| `loft::fill::append_text` | per-file scan: `159-p385-…`, `35p-iterator-match`, `553-nested-vector-slice`, `85-ncc-container-text-return` | the accepted fault-path in-flight-text class (@PLN102) — already suppressed for Linux |
| `loft::registry_index::http_get_bytes` | per-file scan: `tests/docs/{14-image,21-random,32-time}.loft` | a process-lifetime HTTP/TLS client, reachable at exit (benign) |
| `addTermFunc` | `library_suite` cargo-test: `lib/audience_crystal/tests/0{1,2,3}-*.loft` | a macOS dyld thread-TLS system alloc — already suppressed (`leak:addTermFunc`) |

## The precise diagnosis (two independent Darwin problems)

1. **The runtime does not DEMANGLE.** Leaks print the *mangled* v0 symbol
   `_RNvNtCs…_4loft4fill11append_text`, not `loft::fill::append_text`. LSan suppressions match by
   substring of the *symbolized* frame, and the file uses demangled substrings (`fill::append_text`),
   which don't occur in the mangled form. On the runner `llvm-symbolizer resolved to: NONE`, so the
   ASan runtime fell back to macOS `atos`, which resolves the symbol but does **not** demangle Rust
   names. (Frame pointers ARE present — `-Cforce-frame-pointers=yes` is set — so the frame itself
   appears; only the *name form* is wrong.)
2. **macOS LSan appears not to honor the suppression file for the `library_suite` cargo-test at all.**
   `addTermFunc` is a **bare C symbol** (no demangling needed) that is **already** in
   `lsan_suppressions.txt`, yet it STILL leaks in `library_suite`. That points to a Darwin LSan
   limitation (standalone-LSan/suppression support on Darwin is weaker than Linux), independent of
   problem #1. **Confirm this empirically first** — it decides whether the whole gate is fixable via
   suppressions or needs a structural change.

## What was already tried and FAILED (don't repeat blindly)

- `miri.yml` symbolizer step: added `rustup component add llvm-tools-preview` + a
  `$(rustc --print sysroot)/lib/rustlib/*/bin/llvm-symbolizer` lookup. On the macOS runner it still
  resolved to **NONE** (component/path didn't yield a symbolizer). Needs a Mac to find what actually
  works (likely `brew install llvm` → `$(brew --prefix llvm)/bin/llvm-symbolizer`, or `xcrun`).
- Added `leak:registry_index::http_get_bytes` to `lsan_suppressions.txt` — correct in principle, but
  useless while problems #1/#2 stand (it's demangled, and macOS may ignore the file anyway).

## Reproduce locally on the Mac (this is the whole point)

From a clean checkout of `tuxedo-followups` on macOS (aarch64), nightly toolchain:

```bash
export RUSTFLAGS='-Zsanitizer=address -Cforce-frame-pointers=yes'
# (A) the nextest leak gate — this is where `library_suite`/addTermFunc reds:
ASAN_OPTIONS='detect_leaks=1' LSAN_OPTIONS="suppressions=$PWD/.github/lsan_suppressions.txt" \
  cargo +nightly nextest run --profile ci --release --no-fail-fast \
  --target aarch64-apple-darwin --lib --test issues --test wrap --test strings --test frame_vars \
  -E 'not (test(fill_rs_up_to_date) | test(n9_generated_fill_matches_src) | test(native_rs_functions_up_to_date) | test(deep_nesting_guard))'

# (B) the per-file scan — this is where append_text/http_get_bytes red:
cargo +nightly build --release --bin loft --target aarch64-apple-darwin
ln -sfn "$PWD/default" target/aarch64-apple-darwin/release/default
ABIN=target/aarch64-apple-darwin/release/loft \
  LSAN_OPTIONS="suppressions=$PWD/.github/lsan_suppressions.txt" \
  bash scripts/asan_leak_scan.sh tests/scripts/*.loft tests/docs/*.loft
```

Fast inner loop for one leaking file:
```bash
ASAN_OPTIONS='detect_leaks=1' LSAN_OPTIONS="suppressions=$PWD/.github/lsan_suppressions.txt" \
  ASAN_SYMBOLIZER_PATH="$(brew --prefix llvm)/bin/llvm-symbolizer" \
  target/aarch64-apple-darwin/release/loft --interpret tests/scripts/85-ncc-container-text-return.loft
```

## Investigation plan (settle the two questions, then fix)

1. **Symbolizer:** `brew install llvm`; set `ASAN_SYMBOLIZER_PATH=$(brew --prefix llvm)/bin/llvm-symbolizer`;
   re-run (B). Do the leaks now print `loft::fill::append_text` (demangled)? If yes, the leak-scan
   suppressions should start matching → that half is fixed by making `miri.yml` install/point to a
   real `llvm-symbolizer` on macOS.
2. **Does macOS honor the suppression file at all?** Re-run (A) with the symbolizer set. Does
   `addTermFunc` (bare, already suppressed) get suppressed now? If it STILL leaks even with a working
   symbolizer, macOS LSan is ignoring the file for the cargo-test path → problem #2 is real and
   suppressions can't fix `library_suite`.

## Candidate fixes (verify on the Mac before pushing)

- **If the symbolizer fix makes suppressions match (leak-scan half):** update the `miri.yml` symbolizer
  step to reliably resolve `llvm-symbolizer` on macOS (`brew install llvm` + point at it). Also make
  the suppressions robust to *either* form by using bare function names — `leak:append_text`,
  `leak:format_text`, `leak:format_long`, `leak:http_get_bytes` — which are substrings of BOTH the
  mangled (`…11append_text`) and demangled (`fill::append_text`) frames. (Verify no over-match.)
- **If macOS ignores suppressions for `library_suite` (problem #2 confirmed):** don't fight it — do a
  structural change and verify it: run the `library_suite` leak check **Linux-only**, OR post-filter
  known-benign owners in `scripts/asan_leak_scan.sh` for the scan half and drop `library_suite` from
  the macOS nextest `-E` filter, OR (simplest) split the macOS leg into an **informational**
  (`continue-on-error` / non-required) leak gate while Linux stays the enforcing one.

## Decision (if a real fix proves not worth it)

Absent a clean Mac fix, the recommended outcome is: **make the macOS `asan-leak-gate` informational
(non-blocking)** and keep the macOS **UAF/OOB** sweep (which passes) — the *enforcing* leak gate stays
on Linux, which honors suppressions and is green on the same code. Document the why in `miri.yml`.

## Guardrails

- Do NOT weaken the **Linux** leak gate or the ratchet baseline (0). The whole point is that a *real*
  leak stays visible.
- Only ever suppress a **benign** owner (system frame like `addTermFunc`, an intentional bounded
  `Box::leak` like `ir_read`, a reachable-at-exit one-time client like `http_get_bytes`, or the
  accepted fault-path text class). If you can't justify it as benign, it's a real leak — fix it, don't
  suppress it.
- Do NOT touch the `src/ops.rs` shift fix on this branch (separate, verified).

## References

- `.github/workflows/miri.yml` — the `asan-leak-gate` job (lines ~273–400) + the symbolizer step (~311–337).
- `.github/lsan_suppressions.txt` — the suppression file + its documented classes.
- `scripts/asan_leak_scan.sh` (per-file scan) + `scripts/asan_leak_ratchet.sh` (baseline).
- @PLN54 (sanitizer coverage) — the plan this gate belongs to.
- Confirming run: `gh run view 29746783986 -R loft-lang/loft` (the macOS red this handoff is about).
