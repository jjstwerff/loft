<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# CORPUS leakers → 0 (@PLN85) — DONE 2026-07-10

> **RESOLVED 2026-07-10.** All 9 gate-identified corpus leakers are Direct-leak=0
> on BOTH backends; the full `tests/scripts` + `tests/docs` corpus (436 files) and
> the `lib/` `library_suite` (audience_crystal) are 0; the full test suite (2738)
> is green; `fmt`/`clippy` clean. The enforcing nightly leak gate
> (`miri.yml::asan-leak-gate`, scope `--lib --test issues --test wrap --test
> strings --test frame_vars` + the `tests/scripts`/`tests/docs` per-file scan) is
> therefore GREEN. Four principled fixes closed the whole `append_text`
> owned-text-return family; see `leakers.txt` for the fix map and the sections
> below for the method.
>
> **Known follow-up (OUT of leak-gate scope):** `tests/fixtures/libs/web/tests/byte_at.loft`
> Direct-leaks 1 root via `buf = web::pack_take()` — a NATIVE-EXTENSION interop
> leak (the loft-level `buf` IS freed; the native `pack_take` return String is
> orphaned in the bridge), a different subsystem from the codegen text-return
> class. Pre-existing; the fixture-lib suites (`tests/native`, `tests/leak`) are
> NOT in the leak-gate's nextest filter, so it does not red the gate. Track
> separately if a native-extension-interop leak sweep is wanted.

---

Cold-start handoff after the 2026-07-10 session (the original task, now complete).
Read order: this file → [`generic-tuple-return-fix.md`](generic-tuple-return-fix.md)
(the method that worked) → [`skip-free-orphan-class.md`](skip-free-orphan-class.md) →
[`signature-pre-pass-spec.md`](signature-pre-pass-spec.md).

## THE STATE (don't lose this)

The **issues test suite is leak=0** on the interpreter (ASan `detect_leaks=1`). An
**enforcing nightly leak gate** now exists and — as designed — went RED, uncovering a
**second wave** of leakers in the BROADER corpus (`tests/scripts/*.loft` + libraries)
that the issues binary never ran. Those are the target now. Same class family as
everything already fixed: **`append_text` owned-text-return orphans on the interpreter**
(native drops them via RAII).

**The goal is unchanged: drive the leak gate to fully green (all corpus + libraries
leak=0), so `detect_leaks=1` is a trustworthy zero-leak CI gate.**

## FIRST ACTIONS

1. `git fetch`. **Is PR #544 merged?** (`gh pr view 544 --json state`).
   - If MERGED: `git checkout mac-corpus-leaks && git rebase origin/main` (drop the
     now-merged mac-work commits), then push.
   - If still OPEN: it was MERGEABLE/BLOCKED only on a pending `Test (ubuntu)` when this
     was written — verify it went green and merge (the user asked to; branch policy:
     re-check head is current on origin/main first).
2. Build the ASan loft binary (recipe below) — the leak oracle.
3. Enumerate ALL corpus leakers with ONE command (this is the specific-logging tool the
   nightly gate uses):
   ```
   ABIN=<asan-loft> LSAN_OPTIONS=suppressions=$PWD/.github/lsan_suppressions.txt \
     bash scripts/asan_leak_scan.sh tests/scripts/*.loft tests/docs/*.loft
   ```
   It prints, per leaking file: `LEAK <file> roots=N owner=<frame>` (+ a GitHub
   `::error file=` annotation). **Also sweep the LIBRARIES** — not yet enumerated; see
   "Libraries" below.
4. Drive each to 0 with the proven method (below). Re-run the scan after each fix.

## GIT STATE

- **PR #544** = branch `mac-work` → `main`. The whole @PLN85 leak-to-zero arc + the
  nightly leak gate. Clippy/Format/macos-Test green; ubuntu-Test was the last pending
  check. This is the FOUNDATION — let it merge.
- **`mac-corpus-leaks`** (current branch) = stacked on `mac-work`, +1 commit
  (`0263f397`): the per-file leak-scan tool + the corpus-leaker inventory. Continue the
  corpus-leaker FIXES here. After #544 merges, rebase this onto `origin/main`.
- Nightly leak-gate run 29085182904 = `failure` (expected — the corpus leakers).

## WHAT LANDED THIS SESSION (the arc, 6 issues-suite leakers → 0)

| fix | class | commit (on mac-work) |
|---|---|---|
| free `??` ncc text temp after its consuming statement | skip_free-orphan case (a) | `299200ec` |
| generic tuple-of-text return (p329/p330, ×5) — the **reorder** | skip_free-orphan case (b) | `614f659b` |
| promote through a generic-monomorph callee (g1b) | forward-ref | `3e4f53e7` |
| promote `BuiltLocal` text return on a monomorph (plan17_b) | forward-ref | `0fd0fa33` |
| enforcing nightly leak gate (detect_leaks=1, incl. libraries) | CI | `c8c6e8a6` |
| clippy `-D warnings` (tail_call_op assoc fn + collapse if-lets) | — | `535f867e` |

## THE CORPUS LEAKERS (the work) — probes/corpus-leakers/

9 `tests/scripts` leakers found by the gate (all `append_text`). Tracked in
`probes/corpus-leakers/leakers.txt`; scan them with `probes/corpus-leakers/run.sh`.
Their NAMES hint the sub-class (very likely the SAME text-return classes already
solved, just exercised by shapes the issues binary lacked):

- `85-text-optional-null-return.loft`, `500-nested-ncc-optional-text-return.loft` —
  **nullable / `??`-optional text return** (a `text?` return path).
- `199-nwb-text-owned-string.loft` — owned-String text return.
- `45-field-iter.loft`, `86-interfaces.loft` — field-iteration / interface-dispatch
  text return.
- `155-plan52-closure-coalesce.loft`, `157-plan52-config-default.loft` — closure /
  default-value text return.
- `repro_p205.loft`, `repro_p356.loft` — the P205 / P356 repros.

**Libraries are NOT yet enumerated.** The gate's `library_suite` leg is red; sweep each
lib test under the ASan loft with `detect_leaks=1` to list them (the library suite runs
`loft test <stem>` per lib — see `tests/wrap.rs::library_suite` /
`run_lib_test_in_temp_cwd`). Libraries are where loft's real use lives (the user's
priority) — they MUST reach 0 too.

## THE METHOD THAT WORKED (use it)

1. **Capture proven-vs-broken and `diff` into FILES, not glances** (design-protocol).
   For each leaker: find the shape that does NOT leak (non-generic twin / a working
   sibling), capture both `loft introspect` (or `LOFT_LOG=static`) IRs, diff. The
   residual divergence IS the fix. This pinned every fix this session to one line.
2. **Reorder, don't re-derive.** The tuple/forward-ref fixes COLLAPSED sites by
   narrowing existing guards (`is_generic_template` →
   `return_shape_depends_on_type_var`; `n_` → `t_` in the caller pairing) so the case
   rides the ONE existing promotion flow — never by adding a parallel `promote_*` path.
3. **Pass-stable predicates.** Anything keyed on def_nr ordering or `is_generic_template`
   must give the SAME answer on pass 1 and pass 2 (a monomorph is minted pass-2 → reads
   "forward"; map it to its backward TEMPLATE — `control.rs::backward_ref_defnr`).
4. **Verify on BOTH backends + the full suite.** Native no-ops `OpFreeText` (RAII), so a
   leak fix is interp-only, but a promotion change can break native codegen — check both.

## TRAPS THAT COST TIME (avoid)

- **Vacuous leak oracle.** A wrong `LSAN_OPTIONS=suppressions=<bad path>` makes LSan
  error and report FALSE `leak=0` everywhere. The harnesses now PROVE the oracle can
  fail (unsuppressed ir_read must leak). Never trust a `0` from an unproven oracle.
- **Stale ASan binary.** `--native`/nextest rebuild only some binaries; rebuild the
  ASan binary after EVERY src change or you read leaks against old code.
- **Toolchain pollution.** A nightly ASan build pollutes `target/release/deps/libloft_ffi`
  → false `E0514` on stable native tests. Fix: `rm -f target/release/deps/*loft_ffi*`
  then rebuild with stable (mtime won't trigger it). Build ASan ISOLATED
  (`--target-dir target/asan`), unset `RUSTUP_TOOLCHAIN` before stable work.

## THE LEAK GATE (how it works)

Nightly-only, ENFORCING, baseline 0. `miri.yml` job `asan-leak-gate` (matrix
ubuntu-x86_64 + macos-arm):
- Runs the ASan sweep's nextest command with `detect_leaks=1` (per-test attribution) +
  `.github/lsan_suppressions.txt` (only the intentional `ir_read` Box::leak + the macOS
  dyld thread-TLS frame). INCLUDES `library_suite` (+ GL/ALSA deps).
- `if: always()` per-file scan step (`scripts/asan_leak_scan.sh`) names the exact
  leaking `.loft` — the runners run the corpus in ONE process so nextest alone can't.
- **Non-blocking but VISIBLE**: nightly-only (never gates a PR), yet surfaces RED on
  every PR via the existing `nightly-status` job (ci.yml), which iterates ALL miri.yml
  jobs. When the corpus + libraries reach 0, the gate is a trustworthy zero-leak gate.

## TOOLING / REPRO (this box)

- **Build the ASan loft binary** (ISOLATED so nightly never leaks into a stable shell):
  ```
  export PATH="$HOME/.rustup/toolchains/nightly-aarch64-apple-darwin/bin:$PATH"; export RUSTUP_TOOLCHAIN=nightly
  RUSTFLAGS=-Zsanitizer=address cargo build --release --bin loft --target aarch64-apple-darwin --target-dir target/asan
  ln -sfn "$PWD/default" target/asan/aarch64-apple-darwin/release/default
  unset RUSTUP_TOOLCHAIN
  ```
- **Per-file leak scan (specific):** `scripts/asan_leak_scan.sh` (above).
- **Whole-suite count / baseline:** `scripts/asan_leak_ratchet.sh <asan-test-binary>`.
- **Probe matrices (all green, regression guards):** `probes/generic-tuple-return/`,
  `probes/generic-text-return/`, `probes/residual-19/` — each `run.sh` proves its oracle
  can fail before trusting a 0.
- Leak count for one file:
  `ASAN_OPTIONS=detect_leaks=1 LSAN_OPTIONS=suppressions=$PWD/.github/lsan_suppressions.txt <asan-loft> --interpret f.loft 2>&1 | grep -c '^Direct leak'`.
