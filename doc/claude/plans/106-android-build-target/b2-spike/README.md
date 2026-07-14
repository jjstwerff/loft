<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# B2 spike — NativeActivity APK around a loft program (PROVEN on emulator)

This throwaway spike proved the @PLN106 **B2 (R) gate end to end**: a loft program,
wrapped in an Android `NativeActivity`, installs + launches on a headless emulator and
runs — `android_main` reached, and the loft program computed the right answer on-device.

> **UPDATE (2026-07-14): the wrapper is now emitted by loft itself.** `loft
> --native-android` generates the `android_main` `NativeActivity` entry (see
> `src/android.rs` `ANDROID_MAIN_TAIL`), so `Cargo.toml` / `src/lib.rs` / `cargo-config.toml`
> here are **historical** — you no longer hand-write them. What is still live: **`build_apk.sh`
> + `run_emulator_test.sh`** are the packaging + emulator recipe (loft emits the `.so`;
> wrapping it into a signed APK and launching it is still manual). Point `SO=` at loft's
> `--native-android` output.

**It was a spike, not the feature.** It hand-wrapped the loft-emitted `.rs` to de-risk the
pipeline and pin the exact working recipe before that wrapper moved into loft. The crate
files are reference — reproduce, don't depend.

## Proof (emulator logcat, x86_64 system image, API 34)

```
I loft    : loft_android_spike: android_main reached
I loft    : loft_android_spike: running loft program
I loft-stdout: loft program running inside android_main
I loft-stdout: sum of squares 0..8 = 140          <-- the loft program ran; matches the interpreter
I loft    : loft_android_spike: loft program returned
I loft    : loft_android_spike: android_main exiting
```

`ActivityTaskManager: Displayed com.example.loftspike/android.app.NativeActivity +1s490ms`.
Boot→proof was ~30s on a KVM host.

## The recipe (all paths via env; nothing machine-specific baked in)

Prereqs: NDK r26+ (`$NDK`), the SDK cmdline-tools + `platform-tools` + `build-tools;34.0.0`
+ `platforms;android-34` + `emulator` + `system-images;android-34;google_apis;x86_64`
(`$SDK`), a JDK (for `keytool`/`apksigner`), `rustup target add x86_64-linux-android`.
We build the **x86_64** target so the `.so` runs natively on a KVM x86_64 emulator (the
descriptor's `LOFT_ANDROID_TARGET` makes this a config, not a fork); the shipping artifact
stays `aarch64`.

1. **Emit the loft program's Rust** and make its entry callable from the wrapper:
   ```sh
   loft --native-emit b2-spike/src/prog.rs prog.loft
   sed -i '/^#!\[/d'            b2-spike/src/prog.rs   # strip crate-inner attrs (can't survive include! in a mod)
   sed -i 's/^fn main() {/pub fn main() {/' b2-spike/src/prog.rs
   ```
2. **Cross-build the cdylib** (`Cargo.toml` + `cargo-config.toml` here; the wrapper is
   `src/lib.rs`):
   ```sh
   cp b2-spike/cargo-config.toml b2-spike/.cargo/config.toml   # or set the linkers via env
   export ANDROID_NDK_ROOT=$NDK CC_x86_64_linux_android=$NDK/.../x86_64-linux-android24-clang
   cargo build --release --target x86_64-linux-android          # -> libloft_android_spike.so
   ```
   Verify it exports the entry: `llvm-readelf --dyn-syms …/libloft_android_spike.so | grep -E 'ANativeActivity_onCreate|android_main'`.
3. **Package + sign** (`build_apk.sh` here): `aapt2 link` the manifest against
   `android.jar`, `zip` the `.so` into `lib/x86_64/`, `zipalign -p 4`, `apksigner sign`
   with a debug keystore (`keytool -genkeypair … -storepass android`).
4. **Run on the emulator** (`run_emulator_test.sh` here): `avdmanager create avd`,
   `emulator -no-window -gpu swiftshader_indirect -no-snapshot -accel on`,
   `adb wait-for-device` + poll `getprop sys.boot_completed`, `adb install`, `adb shell am
   start -n com.example.loftspike/android.app.NativeActivity`, `adb logcat -d | grep loft`.

## What this pins for the real B2

- **`android_main` entry**: `#[unsafe(no_mangle)] extern "C" fn android_main(app: AndroidApp)`
  with `android-activity = { features = ["native-activity"] }` exports
  `ANativeActivity_onCreate` — loft's android backend should emit exactly this shape
  (guarded by the target descriptor) instead of the B1 cdylib's stray `fn main`.
- **stdout → logcat**: loft `print()` writes fd 1; `dup2` it to a pipe forwarded to
  `__android_log_write` (see `src/lib.rs`) so loft output is visible on-device. loft's
  android runtime could do this in its entry prologue.
- **Manifest**: `android:hasCode="false"` (no dex), `NativeActivity` + `android.app.lib_name`
  meta-data = the cdylib stem.
- **Gotcha**: the loft-emitted `.rs` carries crate-level `#![allow(...)]` inner attributes;
  they must be stripped before `include!`-ing it into a module (hoist to the wrapper crate
  root). When loft emits `android_main` itself this disappears.

## Non-goals (still B3/B4)

No GL surface (black screen), no touch/IME. B3 wires EGL/ANativeWindow; B4 wires
touch/IME. This spike only proves packaging + launch + loft execution.
