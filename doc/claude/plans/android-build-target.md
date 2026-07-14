<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design — loft Android build target (`aarch64-linux-android`)

**Status:** design (pre-B0 spike). **Consumer / dogfood:** `../ssh_home` (a pure-loft
SSH phone terminal) is blocked on this — it runs today as Linux `--native` and re-targets
to Android with *the same source* once this lands. **Branch:** `tuxedo-work` (stacked on
@PLN104 PR #569; graduates to its own `loft-lang/plans` issue once B0 confirms the invariant).

## 1. The one invariant (the design is a hypothesis about this)

> **A loft build target is a *descriptor* — `(rust target triple, linker/toolchain, runtime
> entry shape, packaging)` — over ONE target-agnostic generated-Rust core. Android is a new
> descriptor, NOT a new codegen path.**

If this holds, the generated Rust that `--native` already emits compiles for
`aarch64-linux-android` with only *toolchain + entry + packaging* differences — no
per-target branching inside codegen. The whole plan rides on it, so **B0 exists to falsify
it** (design-protocol: probe the cleanest claim first). loft's existing targets are the
evidence for it: `--native` (host), `--native-wasm` (`wasm32-wasip2`), `wasi`, and the
configurable `build.target.*` family all reuse the same IR→Rust emission and differ only in
the descriptor fields above. Android is the fourth such descriptor.

**Re-assertion sites (design-protocol step 2 — where the triple/toolchain must agree, and
omission is a silent link failure, not a compile error):** (a) the rustc `--target`; (b) the
linker (NDK `clang` for that API level); (c) every non-`loft` dependency rlib must be built
for the *same* triple (the wasm path already learned this — `main.rs:3454` builds the
bridge's dep rlibs for wasm32 without cargo); (d) the crate-type (`cdylib` `.so`, not `bin`);
(e) the runtime entry (`android_main`, not `fn main`). Five sites → collapse them into one
**target descriptor struct** the whole build reads, so adding Android is *data*, not five
edits. This is the chokepoint; if the build wants a sixth `if target == android` branch,
the descriptor is wrong.

## 2. Current architecture (verified in `src/main.rs`, `src/native_utils.rs`)

- `--native` → emit Rust → invoke `rustc` for the host; `--native-wasm [out.wasm]` →
  `wasm32-wasip2`; `wasi`; `build.target.*` selects `native | html | wasi | …`.
- The dep-rlib cross-build precedent (`main.rs:3454`) already produces non-`loft` dependency
  rlibs for a *foreign* triple without cargo — Android reuses exactly this machinery for the
  NDK triple.
- `lib/graphics` native backend = glutin + winit + gl + fontdue (backend crate vendored at
  `tests/fixtures/libs/graphics/native/`, host GL via `src/native_utils.rs` / `src/wasm_gl.rs`).

## 3. The three gaps (from `../ssh_home/DESIGN.md` §7; none exist today)

1. **Cross-target + packaging.** No `aarch64-linux-android` target. Need: the NDK toolchain
   (target triple + NDK `clang` linker + sysroot), `cdylib` crate-type, and an APK wrapper
   (`android-activity` `android_main` entry, `AndroidManifest.xml`, the `.so` under
   `lib/arm64-v8a/`). Signing/`aapt`/`zipalign` via the Android SDK build-tools.
2. **`lib/graphics` Android backend.** Desktop = glutin+winit on X11/Wayland; Android = **EGL
   on `ANativeWindow`** via `android-activity`, a **GLES-3.0** subset. Because the stack is
   already winit-based, this is a re-target of the surface/context creation, not a rewrite —
   the `gl_*` API surface is unchanged.
3. **Input: touch + IME.** `gl_key_pressed` is is-key-down over 0–255 keycodes (no
   char/IME); no `Touch`/gesture events. winit already carries both — wire winit `Touch` +
   the IME/char events through `lib/graphics` so the soft keyboard (password entry) and
   tap/drag/pinch work. This gap is cross-target (helps desktop Unicode too).

## 4. Phased plan (each phase independently landable + dogfooded by ssh_home)

**Verification legend** (each phase is gated by ONE concrete, checkable artifact — mirrors
`../ssh_home/PLAN.md`): **(C)** compiles/links (a named `.so`/rlib exists); **(R)** runs on an
emulator/device (a specific `logcat`/stdout line); **(G)** golden PNG (`gl_screenshot` == a
committed golden under tolerance); **(I)** an input event produces a named on-screen effect.
A phase is DONE only when its gate passes on-device, not in prose.

- **B0 — spike / falsify the invariant (CONCRETE, runnable when an NDK is present).**
  The exact steps that either compile or name the blocker:
  ```sh
  # 0. one-time: rustup target + NDK (r26+); NDK provides the clang linker + sysroot
  rustup target add aarch64-linux-android
  export NDK=$ANDROID_NDK_HOME AT=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin
  # 1. capture loft's generated Rust for a TRIVIAL program (no GL, just prints)
  echo 'fn main() { print("hi\n"); }' > /tmp/b0.loft
  LOFT_KEEP_NATIVE_RS=1 loft --native /tmp/b0.loft   # note the /tmp/loft_native_*.rs path it prints
  # 2. cross-compile that .rs as a cdylib against the NDK clang linker
  rustc --target aarch64-linux-android --crate-type cdylib \
        -C linker=$AT/aarch64-linux-android24-clang \
        --extern loft=<libloft-aarch64.rlib> /tmp/loft_native_*.rs -o /tmp/libb0.so
  # 3. the load-bearing sub-question: does libloft ITSELF cross-compile?
  cargo build --release --lib --target aarch64-linux-android   # (with .cargo/config linker set)
  ```
  **Pass/fail:** step 3 producing `libloft.rlib` for the triple + step 2 producing `libb0.so`
  = **invariant holds** (blockers are toolchain wiring → proceed to B1). A codegen error that
  is host-specific (a `#[cfg(target_os)]` the generated Rust hard-codes, a host-only intrinsic)
  = **invariant is WRONG** → the plan changes (the generated core is not target-agnostic).
  Record the FIRST blocker either way (linker, `std`/`libc` symbols, `ring`/build-script host
  assumption, panic strategy). Do NOT build B1+ before B0 answers this. **If no NDK is available
  in the working env, B0 is a spec to run on a machine that has one — it is not skippable, it is
  the gate.**
- **B1 — the target descriptor + cross `rustc`.** Introduce the descriptor struct (§1) and an
  `android` target that sets triple + NDK linker + `cdylib`; reuse the `main.rs:3454` dep-rlib
  cross-build for the NDK triple. **Verify:** the `.so` links for a headless (no-GL) loft
  program; symbols resolve. Host-testable with an NDK; no device/emulator yet.
- **B2 — APK packaging.** `android-activity` entry, manifest, `.so` → aligned/signed APK.
  **Verify:** the APK installs + launches a black-screen loft program on an emulator (or a
  device); `android_main` reached.
- **B3 — `lib/graphics` EGL/ANativeWindow backend.** GLES-3.0 context on the
  `android-activity` window; port the `gl_*` surface. **Verify:** ssh_home's step-0.1 solid
  clear-color golden renders on-device (same golden as the Linux build — the surface is the
  only difference).
- **B4 — touch + IME input.** winit `Touch`/gesture + char/IME → `lib/graphics`. **Verify:**
  tap-to-select, drag-scroll, pinch-zoom, and soft-keyboard password entry drive ssh_home.

## 5. Failure paths to probe (design-protocol — enumerate how it breaks)

- **`std` for `aarch64-linux-android`** — does the generated Rust's `std` usage (threads,
  files, net via the FFI crate) link against the NDK `libc`/`liblog`? (B0.)
- **The SSH FFI crate (`russh`) cross-compiles** — `ring`/crypto often need per-target asm;
  its build-script must target the NDK, not the host. (Mirrors the known consumer-native
  cdylib pain in [[zt-consume-native-loft-libs]].)
- **GLES subset** — desktop GL calls absent in GLES-3.0 must be avoided/emulated in the
  graphics lib; the golden must render identically or the tolerance must cover it.
- **`android-activity` lifecycle** — `onSaveInstanceState`/pause-resume vs loft's single
  `gl_poll_events` loop; the surface can be destroyed under the app (must recreate context).
- **NDK availability in CI** — the ASan/native CI images likely lack an NDK; the Android
  build is a *separate, opt-in* CI leg (like the daily Windows one), never a required check.

## 6. Non-goals (v1)

Public-key auth, IME beyond ASCII+password, GameActivity (NativeActivity/`android-activity`
is enough), Play-store packaging. Android is a *re-target of one source*, so anything that
would fork the loft program between Linux and Android is out of scope by construction.
