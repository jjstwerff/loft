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
| **`--native`** linking a native library (rlib link) | ❌ **Unverified** — environmental link failures (see G2/G3), test-skipped |
| **Server networking** (`server` bind/accept) | ✅ **Verified 2026-05-29 (CI), re-verified 2026-05-30 (local host)** — the v2 probe (PR #228) un-ignored `v2_single_client_completes_game` on Windows and it PASSED; the other 9 P229b ignores were dropped in the follow-up.  `@P229b` closed without code change (incidental fix in a recent Rust toolchain or transitive dep update).  Independently re-confirmed on a real Windows host (rustc 1.96.0 MSVC): all 10 P229b tests across `multiplayer_v{2,3,5}.rs` pass (v2: 3, v3: 2, v5: 5) when each suite runs in isolation. |
| **`parallel { … }`** (`--interpret`) | ✅ **Verified 2026-05-30 (local host)** — `tests/scripts/80-parallel-block.loft` + `81-parallel-outer-vars.loft` (the @P245 outer-var snapshot guard) both exit 0; `tests/threading.rs` 47/47 pass incl. the `par_ref_buffer_stack_*` worker-stack-snapshot cells. |
| **`parallel { … }`** (`--native`) | ❌ **Unverified** — blocked by the host's WDAC signing policy (see § The 2026-05-30 native-execution wall), not by any parallel logic.  `@P229` G4 native half still open. |

RELEASE.md *intends* to ship a `x86_64-pc-windows-msvc` binary with a
"hands-on smoke test before publishing."  **Until the gaps below are
validated on a real Windows host, that binary's `--native` and server
paths are unverified** — the honest release note is "Windows: interpreter
verified; native-compile + server networking experimental."

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

### G2 — `--native` `windows-targets` link search path (`LNK1181`)  · native

- **Symptom:** `loft --native` (or `--check`) of a program linking a native
  lib fails at link with `LNK1181: cannot open input file
  'windows.0.NN.0.lib'` — the `windows-targets` crate emits a search path into
  its registry source dir, not `OUT_DIR`.
- **Partially worked around:** `src/native_utils.rs::build_script_native_lib_dirs`
  adds those dirs for loft's *own* invocation; the standalone `--check` test
  path still trips it.
- **Skipped:** `tests/codegen_emitter.rs::p310_graphics_vector_ffi_checks_clean`
  (the `LNK1181` branch).
- **VM validation:**
  1. `cargo run --release -- --check --lib lib tests/fixtures/p310_save_png.loft`.
  2. If `LNK1181` fires: confirm `build_script_native_lib_dirs` covers the
     missing `windows-targets` search dir for the `--check` path too (extend
     it to the standalone codegen invocation), then drop the skip branch.

### G3 — `--native` multi-lib transitive-rlib not found (`ureq`/`rustls`)  · native

- **Symptom:** `--check --lib lib` (links the WHOLE `lib/` native stack at once)
  fails at the final link with
  `error: crate `ureq` required to be available in rlib format, but was not
  found in this form` (and `rustls`).  Surfaced on PR #223 (the FFI
  generated-dispatch branch); the *generated Rust + the bridges compiled
  fine* — only the multi-lib rlib **link** failed, and only on Windows.
- **Skipped:** the second branch of `p310_graphics_vector_ffi_checks_clean`
  (matches `"required to be available in rlib format"`).
- **Leading hypothesis:** `add_native_extern_flags` (`src/native_utils.rs`)
  passes `-L dependency=<pkg>/deps` for each native package, but a transitive
  runtime dep shared across packages (web's `ureq`/`rustls`) resolves to a
  build the linked `loft_web.rlib` doesn't match (the `--check` temp build has
  no lockfile → a fresh resolve, e.g. `png 0.18` vs the pinned `0.17`).  On
  Windows the runner's resolve/build order produced the mismatch; Linux/macOS
  happened to stay consistent.
- **VM validation:**
  1. `cargo run --release -- --check --lib lib tests/fixtures/p310_save_png.loft`
     and read the full stderr (past the `Compiling …` noise).
  2. Confirm whether `ureq`/`rustls` rlibs exist under
     `lib/web/native/target/release/deps` and whether their hashes match the
     `loft_web.rlib` the link references.
  3. Likely fix: make the `--native` build resolution **lockfile-consistent**
     across native packages (or restrict `--lib lib` to the packages the
     program actually uses, so a graphics fixture doesn't drag in web's TLS
     stack), then drop the skip branch.

### G4 — `parallel { … }` worker stack snapshot (`@P229`, partial)

- Half-open Windows issue with the parallel worker stack snapshot.  Low
  severity; details in PROBLEMS.md `@P229`.
- **Interpreter half VERIFIED CLEAN on a real Windows host (2026-05-30).**
  The G4 worry was that the worker-thread stack snapshot
  (`execute_at_void_with_snapshot`, @P245) returns garbage on Windows.  It
  does not: `81-parallel-outer-vars.loft` (asserts an outer-scope `outer = 42`
  is observed unchanged inside both arms) exits 0 under `--interpret`, and
  `tests/threading.rs` passes 47/47 including the worker-stack cells.  The
  residual G4 gap is now scoped to the **`--native`** parallel path only.
- **Native half still UNVERIFIED — blocked, not by parallel logic, but by the
  host's code-integrity policy** (see § The 2026-05-30 native-execution wall).
  Once a freshly-built `loft.exe` can execute on the VM, re-run the two
  parallel scripts under `--native` to close or capture the native half.

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
  with a YubiKey-backed signing key), then re-run the `--native` checks (G2,
  G3, G4-native).  Until then, the `--native` gaps stay unverified on this
  machine.

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
6. Once G2 + G3 close, upgrade the `--native` row in § compatibility to ✅ and
   adjust the RELEASE.md Windows note.

## See also

- `.github/workflows/ci.yml` — the 3-OS matrix.
- `src/native_utils.rs` — `build_script_native_lib_dirs`, `add_native_extern_flags`
  (the `--native` link/rlib-discovery the gaps live in).
- PROBLEMS.md `@P229` · RELEASE.md (Windows binary intent).
