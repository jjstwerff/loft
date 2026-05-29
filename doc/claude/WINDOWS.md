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
| **Server networking** (`server` bind/accept) | ✅ **Verified 2026-05-29** — the v2 probe (PR #228) un-ignored `v2_single_client_completes_game` on Windows and it PASSED; the other 9 P229b ignores were dropped in the follow-up.  `@P229b` closed without code change (incidental fix in a recent Rust toolchain or transitive dep update). |
| **`parallel { … }`** | ⚠️ **Half-open** — `@P229` worker-stack issue |

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

## Previously fixed Windows-only issues (for context)

- **`@P229b` — server cannot bind on Windows (closed 2026-05-29).** The 10
  `multiplayer_v{2,3,5}.rs` scenarios marked `#[cfg_attr(target_os = "windows",
  ignore = "P229b…")]` are all un-ignored.  The v2 probe (PR #228 commit
  `baa9c3e2`) un-ignored `v2_single_client_completes_game` and CI showed it
  PASSING on `windows-latest` — the 2026-05-21 "bind-then-drop race"
  hypothesis was incorrect; @P229b incidentally resolved in some recent
  Rust toolchain or transitive dep update.  Other 9 dropped in the same
  cycle.  Linux + macOS still pass; Windows now matches.
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
