<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# tuxedo-work — scope for two tracks (flaky test-infra + loft Android target)

Working notes for the `tuxedo-work` branch. Two independent tracks the user grouped here.

## Resume state (2026-07-14, post-#569 merge)

- **PLN104 / loft#568 — DONE, MERGED to main** (PR #569, squash). This branch was stacked on
  it and has now been **rebased onto `origin/main`** (7 commits ahead, 0 behind, builds green).
- **Track A — fix LANDED, soaking** (§ below): `reap_port` hardening + stderr flake-probe in
  `tests/engine_host_kernel.rs`. Root cause pinned (leaked swap-child orphan the stem pgrep
  misses); "flake gone" UNPROVEN (intermittent, could not force). RESUME: soak-watch s5/s7 in
  CI; file a `loft-lang/loft` bug (pre-existing, both-mode).
- **Track B — B0 CONFIRMED + B1 LANDED + B2 SPIKE PROVEN** (`106-android-build-target/`,
  tracked as **@PLN106**): loft Android target for `../ssh_home`. Invariant held (generated
  core cross-compiles to `aarch64-linux-android` unchanged); `loft --native-android prog.loft`
  produces a bionic AArch64 `.so`. Code: `src/android.rs` + `--native-android` in `main.rs` +
  `tests/android_target.rs`. **B2 spike** (`106-android-build-target/b2-spike/`) proved the
  APK pipeline end to end: a NativeActivity APK launched on a headless KVM emulator, logcat
  showed `android_main reached` + the loft program's output (`sum of squares 0..8 = 140`).
  Toolchain installed locally: NDK r27c (`~/android-ndk-r27c`), Android SDK
  (`~/Android/Sdk` — platform-tools/build-tools 34/platform 34/emulator/x86_64 image), JDK 17
  (`~/jdk17`). RESUME (all in @PLN106): B2 *feature* (emit `android_main` from
  `--native-android` instead of the hand wrapper), B3 (EGL/ANativeWindow GL surface), B4
  (touch/IME). Emulator target is x86_64 (`LOFT_ANDROID_TARGET=x86_64-linux-android`);
  ship target stays aarch64. Networking/TLS (`ring`) is a later runtime-rlib feature-flag.
- No PR on this branch yet. Nothing is blocking.

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

**Investigation update (do not fix on a guess — the mechanism is not yet pinned):**
- **RULED OUT — s5/s7 mutual port collision.** `test_port(base) = base + LOFT_TEST_PORT_OFFSET`
  (offset per-checkout), so distinct bases give distinct ports; s5 uses 18100, s7 uses 18108,
  and the kernel tests' bases are all distinct — no cross-talk between them.
- **Narrowed to load-dependence.** The engine binary passes 16/16 (threaded, one process,
  V2 on AND off); the flake needs the FULL nextest suite (process-per-test, max CPU
  pressure). The s5 subprocess is `loft` on the DEFAULT backend = `--native` → a rustc
  compile per spawn, then a live rebuild for S3/S4 — all slow under load. The first-ask
  `v2:a#47` (version already flipped + 47 events before the test's first `ask`) is consistent
  with the subprocess running far ahead by the time the test connects under pressure, OR a
  `ws_recv` returning a queued/later frame. Both are timing, not logic.
- **RULED OUT — generic CPU load.** s5+s7 pass 10/10 under 16-core saturation in isolation,
  so raw pressure is not it; the trigger is *concurrent engine_host tests* (full-suite only).
- **PINNED (commit `3ff5cf56`) — leaked swap-child orphan on the port.** `s5_kill_stale` greps
  `pgrep -f <cache-stem>`, but the native SWAP CHILD (hot-swapped v2 build) runs from
  `loft_native_bin_*` in the scratch dir — its command line lacks the stem, so the stem pgrep
  MISSES it. When a prior run's `Guard::drop` process-group kill is skipped (nextest SIGKILLs
  a timed-out test → no unwinding → no Drop), the orphan survives on the port and the next
  `ws_connect(port)` binds the stale, already-flipped, high-count world → `v2:a#47` /
  `got:a#236`. **Confirmed empirically:** 30 leaked `loft_native_bin_*` orphans from local
  runs, NONE matched by the stem pgrep (`pgrep -af 18100` → empty).
- **Fix landed:** `reap_port(port)` (lsof) reaps ANY process bound to the port before the test
  reuses it — after `s5_kill_stale`, before spawn. Plus the s5 first-ask assertion now dumps
  subprocess stderr so a future CI flake captures the proof directly. Verified: s5+s7 isolated
  green, full engine binary 16/16, fmt+clippy clean, 30 local orphans reaped to 0.
- **Caveat (honest):** the flake is intermittent and I could not force it on demand (it needs a
  prior run to have leaked an orphan on THIS test's exact port), so I could not prove the flake
  is gone — only that the confirmed reaping gap is closed and the rest is instrumented. Final
  confirmation is soak: watch these two in CI; if one still flakes, the new stderr dump tells us
  whether it is the orphan (empty err file) or in-subprocess event accumulation (`LOFT_LIVE_FLIP`
  auto-flip) — the one mechanism I did not fully trace.

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
