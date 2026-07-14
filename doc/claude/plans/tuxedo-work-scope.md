<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# tuxedo-work — scope for two tracks (flaky test-infra + loft Android target)

Working notes for the `tuxedo-work` branch (stacked on the @PLN104 PR #569 until it
merges, then rebase onto `main`). Two independent tracks the user grouped here.

## Track A — the s5/s7 full-suite flake (test-infra hardening)

**Symptom.** `s5_native_swap_under_running_world` and `s7_debugger_loop_end_to_end`
(`tests/engine_host_kernel.rs`) fail ONLY in the full nextest suite (max parallelism),
not individually and not in the engine-binary run. Assertion: expected `v1:a#1t…`, got
`v2:a#47t…` (the numbers drift 46/47/48 across runs).

**Root cause (read, not guessed).** The s5 fixture (`engine_host_kernel.rs:445`) sends
`"{ver}:{payload}#{n}t{ticks}"` where `n` is the running `bump_events` count. The test is
launched with `LOFT_LIVE_FLIP=1 LOFT_FLIP_FNS=bump_events` — the @PLN98 live-tier
**auto-flip**, which fires swaps + re-dispatches on its own schedule. Under full-suite CPU
load the gap between spawn and the test's first `ws_connect`/`ask` widens, so by the first
ask the world has already processed ~47 events AND auto-flipped to v2. The test hardcodes
`v1` + `a#1` on the first ask — a timing assumption that only holds when the subprocess is
near-idle. **Confirmed pre-existing: fails identically with `LOFT_NO_TRET_FIX` (promotion
off)**, so it is NOT the @PLN104 promotion — it rode along on an already-fragile test.

**Fix options (pick after a probe on the live-flip trigger):**
1. **Synchronize** — have the fixture's event loop not count/flip until the first client
   connects (a "ready" barrier), so `n`/version are deterministic at the first ask. Cleanest
   if `engine_host::run` can expose an on-connect hook.
2. **Assertion-relativize** — read the first response's `a#<n>` as the baseline instead of
   asserting `a#1`; assert the *delta* per subsequent ask and the version *transition*, not
   absolute values. Lower-risk, purely test-side.
3. **Gate the auto-flip** — make `LOFT_LIVE_FLIP` flip on an explicit trigger (a control
   message) rather than a timer, so the test drives the flip. Touches the live tier.

Option 2 is the least-invasive and most robust; option 1 is the "correct" fix if the hook
exists. Verify by running the full nextest suite 3× (they must pass every time), not the
isolated test (which already passes). This is a **separate GitHub issue** against the
engine-host/live-flip harness — file it (pre-existing, both-mode).

## Track B — loft Android build target (for ssh_home; loft-side feature)

`../ssh_home` (a pure-loft SSH phone terminal) needs loft to grow an Android target. From
its `DESIGN.md` § 7, three separable loft-side gaps — none exist today (`grep android src/`
is empty):

1. **`aarch64-linux-android` `--native` cross-target.** loft's `--native` compiles the
   generated Rust for the host only. Need: an NDK-toolchain cross-compile path (target
   triple + NDK clang linker), emitting a JNI `.so`, packaged into a
   `NativeActivity`/GameActivity APK. Entry point differs from a host binary (no `main`;
   `android_main` via `android-activity`).
2. **`lib/graphics` Android backend.** Desktop = glutin+winit on X11/Wayland; Android =
   EGL on `ANativeWindow` via `android-activity`, GLES-3.0 subset. Same `gl_*` surface — a
   re-target (winit already abstracts it), not a rewrite. (Backend crate today:
   `tests/fixtures/libs/graphics/native/` + `src/native_utils.rs`/`src/wasm_gl.rs`.)
3. **`lib/graphics` input: touch + IME.** `gl_key_pressed` is is-key-down over keycodes
   (no chars/IME); no `Touch`/gesture events. winit already carries both — wire them
   through `lib/graphics`. Needed for the soft keyboard (password) + tap/drag/pinch.

**Suggested phasing** (each independently landable, dogfooded by ssh_home):
- **B0** — spike: can loft's generated `--native` Rust even `cargo build --target
  aarch64-linux-android` with an NDK? Identify the linker/runtime blockers. (Needs an NDK
  in the env — likely absent here; may be a design-only pass first.)
- **B1** — the cross-target `--native` flag + NDK linker plumbing (host-testable via a
  cross toolchain; run under an emulator or defer runtime to device).
- **B2** — APK packaging (android-activity entry, manifest, `.so`).
- **B3** — `lib/graphics` EGL/ANativeWindow backend.
- **B4** — touch/IME input events through `lib/graphics`.

This is a real multi-session feature and a `loft-lang/plans` issue in its own right (B is
NOT ad-hoc work — it earns its own plan/branch once B0's spike scopes the blockers). Keep
it here only until it graduates.
