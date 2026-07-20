<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# macOS ASan interpreter leak gate — green LOCALLY, STILL RED on CI (runner divergence)

**Status (2026-07-20): the fix below greens a real macOS box but NOT the CI `macos-latest` runner.**
The Mac agent verified locally (`0 leaking of 513`; nextest `1629 passed, 0 leaked`), but a
`workflow_dispatch` miri run (29757356253) on `tuxedo-followups` with that same fix left the macOS
leg RED. debug-asserts ✅ and ASan-leak **ubuntu** ✅ in that run, so the bare-name suppressions are
Linux-safe. The root-cause analysis below is correct and load-bearing; only the "0 on CI" claim was
over-stated (it was local). This blocks the owner's first STABLE RELEASE (they won't ship with a red
nightly) — needs a Mac agent iterating **against CI**, not just a local box.

**Reading the ACTUAL CI job log (88403045509) corrects the earlier "3 residuals" framing — see
§ CI-runner divergence for the ground truth:** only ONE step is red (the per-file scan, 3 files),
the `library_suite` "residual" was a flake that passed on retry, and the leak owner is the benign
`ir_read` interner (library-bundle load), NOT `http_get_bytes`. The remaining problem is purely a
CI-runner *symbolization* gap, which a local box cannot reproduce.

## Outcome (LOCAL — not yet CI)

On the Mac agent's box: per-file scan `0 leaking file(s) of 513`, nextest `1629 passed, 0 leaked`
(incl. `library_suite` + `loft_suite`), NO real leak masked, Linux leg unweakened.

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

## CI-runner divergence — GROUND TRUTH from the job log (iterate AGAINST CI, not a local box)

Miri run **29757356253** (`workflow_dispatch`, `tuxedo-followups`, with the fix above),
`ASan interpreter leak gate (macos-latest)` = **failure**. Reading the actual job log
(`gh run view --job 88403045509 -R loft-lang/loft --log`) corrects the earlier speculative
"3 residuals" table. What the log ACTUALLY shows:

- **Only the per-file scan step is red** — `=== leak scan: 3 leaking file(s) of 513 scanned ===`,
  exit 1. The three files: `tests/docs/{14-image,21-random,32-time}.loft`, each `roots=1  owner=?`.
- **The nextest "Leak gate" step PASSED.** `library_suite` shows `FLAKY 2/2 … TRY 2 PASS`; the run
  summary is `1630 passed (1 flaky), 14 skipped`. So the handoff's `ThreadLocalVariables` /
  `library_suite` "residual" was an intermittent FLAKE that passed on retry — NOT a hard blocker
  (worth hardening, but the step is green). The `ThreadLocalVariables` suppression is working.
- The Linux-only symbolizer step correctly shows `-` (skipped) on macOS; the **ubuntu** leak gate is ✅.

### The leak owner is `ir_read` (benign interner), NOT `http_get_bytes`

Reproduced locally (atos, no suppressions) on all three files — the stack is unambiguous and identical:
```
#1 alloc::raw_vec::RawVecInner::try_allocate_in
#2 loft::ir_read::read_block
#3 loft::ir_read::read_value
#4 loft::ir_read::read_definition   (or read_node_list → spec_from_iter)
#5 loft::ir_read::read_data_with
#6 loft::ir_read::open_bundle
#7 loft::ir_read::open_bundle_into
#8 loft::startup_cache::warm_load_program
#9 loft::main
```
This is the **intentional bounded `Box::leak` string interner** (`ir_read`), fired when a file loads
a precompiled **library bundle** (`open_bundle`). It is ALREADY suppressed by `leak:ir_read` — and
locally that match works (the frames symbolize to `…_4loft7ir_read10read_block`, whose substring is
`ir_read`), giving `0 leaking of 513`. It is the SAME benign class the ~16 round-trip lib tests hit.
The handoff's `http_get_bytes` label (and the `src/registry_index.rs` per-call-client theory) was
WRONG — these frames never appear in the stack.

**Why only these 3 files:** the leak needs a library BUNDLE load (`open_bundle`). Plain scripts
(no `use <installed-lib>`) never enter `open_bundle`, so they don't leak `ir_read` (a plain script
like `85-ncc…` leaks only `append_text`). The three are exactly the docs tests that `use` an
installed library (`imaging` / `random` / `time`). `32-time` uses a LOCAL fixture and does NOT even
leak on the local box — confirming the leak is bundle-load-dependent, hence environment-dependent,
hence unstable file-to-file (do NOT hard-code "these 3").

### The real blocker: CI-runner symbolization, which a local box cannot reproduce

Locally, atos symbolizes the `ir_read` frames (mangled, substring matches → suppressed → 0/513).
On the `macos-latest` runner, LSan's frame for this path comes back UNSYMBOLIZED, so `leak:ir_read`
can't match and the scan counts it. The scan's `owner=<unknown>` is a separate red herring — its
owner-grep looks for `loft::` (absent from mangled names when `rustfilt` isn't installed) and even
excludes `ir_read` — so `owner=?` tells us nothing about the true owner or the suppression outcome.
**No name-based suppression can be validated locally; the runner's symbolizer output must be read
from a CI run.** Repro: `gh workflow run miri.yml -R loft-lang/loft --ref <branch>` → the
`macos-latest` leak-gate job log.

### Next step (recommended) + fallback

1. **One diagnostic CI round FIRST.** Add an env-gated full-stack dump to `scripts/asan_leak_scan.sh`
   (print the raw ASan leak report for each leaking file, macOS-scan only) and dispatch miri once.
   That reveals what the runner's symbolizer actually emits for the `ir_read`/`open_bundle` frames —
   mangled, `<unknown>`, or otherwise — which is the only thing that decides the fix (add a
   symbolization-independent anchor, force symbols on the scan binary, or scope the bundle-loaders
   out). Blind pushes are what burned the last two rounds.
2. **Fallback (owner-acceptable, no iteration):** scope the library-bundle-loading docs tests out of
   the **macOS** per-file scan only (the leak is the intentional `ir_read` interner, not loft-program
   memory; Linux keeps the full enforcing scan and suppresses it by name). Prefer 1 to keep the gate
   genuinely green rather than narrowed.
3. Last resort: mark the macOS *leak* leg non-blocking — the owner wants it genuinely green before
   the stable release, NOT hidden, so this is truly last.
