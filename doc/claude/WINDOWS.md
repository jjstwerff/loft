<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Windows support — verified state, known gaps, and a VM-validation runbook

> **Looking for "what to do when you finally get Windows access for a day"?**
> See [WINDOWS_SESSION.md](WINDOWS_SESSION.md) — priority-ordered checklist,
> time budget, pre-flight steps.  This doc is the reference; the session
> doc is the action plan.

## Honest compatibility statement

The CI matrix runs the full test suite on `windows-latest`
(`.github/workflows/ci.yml`).  But a set of tests are **skipped or
`#[ignore]`d on Windows** because they hit failures no contributor has been
able to reproduce or diagnose without a real Windows machine.  So the
accurate claim today is:

| Path | Windows status |
|---|---|
| **`--interpret`** (single program, no native libs) | ✅ **Verified** — the bulk of the suite runs + passes on Windows CI |
| **`--interpret`** with a `#native` library (dlopen the cdylib) | ⚠️ **Mostly** — imaging etc. pass; multi-lib + networking caveated below |
| **`--native`** linking a native library (rlib link) | ✅ **Verified 2026-05-30 (CI)** — G2 fixed (per-package link-search harvest in `native_utils.rs`); G3 no longer reproduces; `--check --lib lib` exits 0 on `windows-latest` (CI run 26690846366).  `tests/codegen_emitter.rs::p310_graphics_vector_ffi_checks_clean` LNK1181 + G3 silent-skip branches removed; `p310` now asserts `out.status.success()` on every platform and is the cross-platform regression guard for the G2 fix.  Remaining: full Windows `nextest` validation runs on this PR's CI (`windows-latest` leg of `ci.yml`). |
| **Server networking** (`server` bind/accept) | ✅ **Verified 2026-05-29 (CI), re-verified 2026-05-30 (local host)** — the v2 probe (PR #228) un-ignored `v2_single_client_completes_game` on Windows and it PASSED; the other 9 P229b ignores were dropped in the follow-up.  `@P229b` closed without code change (incidental fix in a recent Rust toolchain or transitive dep update).  Independently re-confirmed on a real Windows host (rustc 1.96.0 MSVC): all 10 P229b tests across `multiplayer_v{2,3,5}.rs` pass (v2: 3, v3: 2, v5: 5) when each suite runs in isolation. |
| **`parallel { … }`** (`--interpret`) | ✅ **Verified 2026-05-30 (local host)** — `tests/scripts/80-parallel-block.loft` + `81-parallel-outer-vars.loft` (the @P245 outer-var snapshot guard) both exit 0; `tests/threading.rs` 47/47 pass incl. the `par_ref_buffer_stack_*` worker-stack-snapshot cells. |
| **`parallel { … }`** (`--native`) | ✅ **Verified 2026-05-30 (CI, windows-latest)** — `80-parallel-block.loft` + `81-parallel-outer-vars.loft` compiled + executed under `--native`, exit 0 (CI run 26689698213).  G4 fully closed. |

RELEASE.md *intends* to ship a `x86_64-pc-windows-msvc` binary with a
"hands-on smoke test before publishing."  The `--native` and server paths
are now CI-verified.  The two silent-skip branches in
`tests/codegen_emitter.rs` (LNK1181 and "required to be available in rlib
format") are dropped in this change; `p310` now guards the fix on every
platform.  Remaining: the full Windows `nextest` regression suite runs
automatically on this PR's `windows-latest` CI leg.

This doc is the runbook to close that gap.  The user can spin up a Windows
VM and work each gap → reproduce → capture the real error → fix → un-skip.

## Why these can't be closed from Linux/macOS

Every gap is a *Windows-toolchain* or *Windows-OS-semantics* issue
(MSVC linker search paths, `TIME_WAIT`/exclusive-port semantics, rlib
discovery under the MSVC target).  They do not reproduce on Linux or macOS,
and CI gives only a post-mortem log — not an interactive shell.  A VM with
the MSVC toolchain is the missing piece.

## Windows VM prerequisites

- Rust stable with the **`x86_64-pc-windows-msvc`** target (the default on a
  Windows host) + the **MSVC build tools** (`link.exe`).
- `cargo-nextest` (the suite uses it) and `gh` if validating against CI.
- **No `mold`** — the repo forces `-fuse-ld=mold` only on Linux; confirm the
  Windows build uses the default MSVC linker (check `.cargo/config.toml`
  guards by target).
- Clone, then `cargo build --release` + `cargo build --release --lib` (the
  unhashed `libloft.rlib` the `--native` path links — see
  `reference_native_rlib_rebuild`).
- **Pin the toolchain target.**  If both `-gnu` and `-msvc` toolchains are
  installed, build the rlib with the SAME target `--native` compiles against
  (msvc), or E0461 fires (see § The 2026-05-30 native-execution wall).  Use
  `rustup run stable-x86_64-pc-windows-msvc cargo build --release` to be
  explicit.
- **Native execution needs a runnable, policy-permitted `loft.exe`.**  On a
  host with WDAC / Smart App Control in Enforce, a freshly-built unsigned
  `loft.exe` is blocked at load (CodeIntegrity 3077).  `cargo test` still
  works (cached verdict), but standalone `loft --native …` requires the
  binary to be code-signed with a trusted cert after each build.

## Known gaps

### ~~G2~~ — `--native` `windows-targets` link search path (`LNK1181`) — FIXED 2026-05-30

- **Root cause:** a diamond dependency pulls TWO versions of
  `windows_x86_64_msvc`.  The graphics native stack (winit/glutin/rodio/cpal)
  pulls 0.52.6, built under `lib/graphics/native/target/release/build/…`; the
  top-level stack pulls 0.48.5, built under the top-level
  `target/release/build/…`.  `build_script_native_lib_dirs` was only invoked
  on the TOP-LEVEL build root (`src/main.rs:3199`), so the 0.52.6 crate's
  link-search dir (`…\windows_x86_64_msvc-0.52.6\lib`) was never added →
  its `windows.0.52.0.lib` was passed to the linker as a bare filename with
  no `/LIBPATH` → `LNK1181: cannot open input file 'windows.0.52.0.lib'`.
  The 0.48.5 copy linked fine because its dir WAS harvested.
- **Fix (`windows-validation` branch):** `src/native_utils.rs::add_native_extern_flags`
  now also harvests `rustc-link-search` dirs from each native package's OWN
  `<profile>/build/*/output` (via `build_script_native_lib_dirs(rlib_path.parent())`)
  and adds `-L native=<dir>` for each.  This supplies the missing
  `…\windows_x86_64_msvc-0.52.6\lib` LIBPATH.
- **Verified:** CI run 26690846366 — `loft --check --lib lib tests/fixtures/p310_save_png.loft` exits 0 (was 1 / LNK1181).
- **Open follow-up:** `tests/codegen_emitter.rs::p310_graphics_vector_ffi_checks_clean`
  still carries a silent-skip branch that matches `LNK1181`.  Now that the
  link succeeds, this branch masks a passing case.  Remove it and assert the
  link succeeds on Windows; run a full Windows nextest suite to confirm no
  regressions.

### ~~G3~~ — `--native` multi-lib transitive-rlib not found (`ureq`/`rustls`) — NO LONGER REPRODUCES

- **Status (2026-05-30):** With G2 fixed, the full `--check --lib lib` multi-lib
  link completes clean (exit 0) on `windows-latest` (CI run 26690846366).  G3
  did not reproduce with the current toolchain.  It was always environmental
  (resolve/build order), not a codegen bug — the leading hypothesis (lockfile
  inconsistency across packages) was never confirmed.  Treat as
  resolved-on-current-toolchain.
- **Open follow-up:** `tests/codegen_emitter.rs::p310_graphics_vector_ffi_checks_clean`
  still carries a silent-skip branch matching `"required to be available in rlib
  format"`.  Now that the link succeeds, this branch masks a passing case.
  Remove it (together with the LNK1181 branch — see ~~G2~~ above) and assert
  the link exits clean on Windows.

### ~~G4~~ — `parallel { … }` worker stack snapshot (`@P229`) — FULLY CLOSED 2026-05-30

- **Interpreter half VERIFIED CLEAN (2026-05-30, local host):** `81-parallel-outer-vars.loft`
  exits 0 under `--interpret`; `tests/threading.rs` 47/47 pass incl. the
  `par_ref_buffer_stack_*` worker-stack-snapshot cells.
- **Native half VERIFIED CLEAN (2026-05-30, windows-latest CI run 26689698213):**
  `tests/scripts/80-parallel-block.loft` + `81-parallel-outer-vars.loft`
  compiled + executed under `--native` on the GitHub `windows-latest` runner,
  exit 0.  The runner does NOT enforce the WDAC code-integrity policy that
  blocked the local host, so freshly-built unsigned binaries run normally.
- **`@P229` G4 is fully closed.**  Both interpreter and native parallel paths
  are verified on Windows.  Details in PROBLEMS.md `@P229`.

## The 2026-05-30 native-execution wall

The 2026-05-30 local-host session validated the interpreter + server paths
but could **not** exercise any `--native` path end-to-end.  Two layered
blockers, both **environmental** (not loft bugs):

1. **`E0461` — stale gnu-target rlib.**  The host had both
   `stable-x86_64-pc-windows-gnu` and `-msvc` toolchains installed (msvc
   default), and the pre-existing `target/release/libloft.rlib` was a
   **gnu-target** build, while `--native` compiles against **msvc**:
   `error[E0461]: couldn't find crate 'loft' with expected target triple
   x86_64-pc-windows-msvc`.  No `rust-toolchain.toml` pins the target.
   *Cleared* by an explicit `rustup run stable-x86_64-pc-windows-msvc cargo
   build --release` (rebuilds both `loft.exe` and the rlib for msvc).
   Lesson for G2/G3 work: confirm the rlib's target triple matches the
   `--native` compile target before trusting any link-stage diagnosis.
2. **WDAC code-integrity policy blocks the rebuilt, unsigned `loft.exe`.**
   With the msvc rlib in place, every freshly-built `loft.exe` (release AND
   the `--native` temp binaries) is refused at load:
   `CodeIntegrity` event **3077** — *"did not meet the Enterprise signing
   level requirements or violated code integrity policy (Policy ID:
   {0283ac0f-fff1-49ae-ada1-8a933130cad6})"*; the user-facing error is
   *"This file is blocked by an application control policy."*  Confirmed it
   is **not** a toolchain issue: the older **debug** `loft.exe` (also local,
   also unsigned) still runs — the policy keys on the file **hash /
   reputation**, so rebuilding produces a new hash with no cached-good
   verdict and is blocked.  `cargo test` continues to work because those
   test binaries already carry a cached verdict.  Smart App Control is in
   **Enforce** (`VerifiedAndReputablePolicyState = 1`); disabling SAC is a
   one-way door (requires a Windows reset to re-enable) and was declined.
- **To unblock native validation on this host:** code-sign each freshly
  built `loft.exe` with a trusted cert after every build (the user validates
  with a YubiKey-backed signing key), then re-run the `--native` checks.

**Note (2026-05-30 follow-up):** Although the local host remained WDAC-blocked,
the `--native` gaps (G2 root-cause + fix, G3 non-reproduction, G4-native
verification) were subsequently addressed on the GitHub `windows-latest` CI
runner, which does NOT enforce WDAC and runs freshly-built unsigned binaries
normally.  CI run 26689698213 confirmed G4-native clean; CI run 26690846366
confirmed the G2 fix (per-package link-search harvest) makes `--check --lib lib`
exit 0.  G2 and G4 are now closed; G3 no longer reproduces.  The
`tests/codegen_emitter.rs` silent-skip branches for LNK1181 and "required to
be available in rlib format" are removed in this change; `p310` is now the
cross-platform regression guard.  Remaining: full Windows `nextest`
validation runs on this PR's `windows-latest` CI leg.

## Previously fixed Windows-only issues (for context)

- **`@P229b` — server cannot bind on Windows (closed 2026-05-29; re-verified
  on a local host 2026-05-30).** The 10
  `multiplayer_v{2,3,5}.rs` scenarios marked `#[cfg_attr(target_os = "windows",
  ignore = "P229b…")]` are all un-ignored.  The v2 probe (PR #228 commit
  `baa9c3e2`) un-ignored `v2_single_client_completes_game` and CI showed it
  PASSING on `windows-latest` — the 2026-05-21 "bind-then-drop race"
  hypothesis was incorrect; @P229b incidentally resolved in some recent
  Rust toolchain or transitive dep update.  Other 9 dropped in the same
  cycle.  Linux + macOS still pass; Windows now matches.  WINDOWS_SESSION.md
  Priority 1 was executed on a real Windows host (rustc 1.96.0 MSVC, 2026-05-30):
  all 10 P229b tests pass (`multiplayer_v2` 3/3, `multiplayer_v3` 2/2,
  `multiplayer_v5` 5/5), and no `cfg_attr(windows, ignore)` gate remains in
  those files.  Note: running several `multiplayer_v*` test binaries
  concurrently (plus a parallel single-test instance) once made
  `v2_late_join_independent_games` exceed its 60s client timeout (empty
  stdout, exit `None`) — a CPU/port-contention artifact, not a bind failure
  (a real @P229b bind error surfaces via `diagnose_listen_failure`, not a
  silent client timeout); the test passed on isolated re-run.
- `@P332` — `install::install_one` return on Windows (fixed 2026-05-26).
- `@P333` — `/tmp/` hard-coded paths in two lib fixtures → CWD-relative
  (fixed 2026-05-26).
- Heap-corruption-style failures: TESTING.md § (Windows `STATUS_HEAP_CORRUPTION`
  caught real out-of-bounds that Linux's allocator slack hid — a case where
  Windows CI was *more* sensitive and *useful*).

## Closing a gap — the loop

1. Reproduce on the VM with the command in the gap's "VM validation".
2. Capture the *real* error (CI only shows a post-mortem; the VM gives a shell).
3. Apply the fix; re-run; confirm green on Windows.
4. **Remove the skip/ignore** (the gate is the lie — deleting it is the proof).
5. Move the gap to "Previously fixed"; update PROBLEMS.md / TESTING.md.
6. G2 + G3 are closed; the `--native` row in § compatibility is now ✅.
   The codegen_emitter skip removal is done in this change; update RELEASE.md
   once the full Windows CI run on this PR completes clean.

## See also

- `.github/workflows/ci.yml` — the 3-OS matrix.
- `src/native_utils.rs` — `build_script_native_lib_dirs`, `add_native_extern_flags`
  (the `--native` link/rlib-discovery the gaps live in).
- PROBLEMS.md `@P229` · RELEASE.md (Windows binary intent).
