//! Android build target (`--native-android`) — @PLN-android B1.
//!
//! loft's native backend emits ONE target-agnostic Rust core and compiles it
//! per target; a build target is just a *descriptor* — `(rust triple, linker,
//! crate-type, runtime rlib)` — over that shared core (B0 confirmed the core
//! cross-compiles to `aarch64-linux-android` unchanged; see
//! `doc/claude/plans/android-build-target.md`).  This module is that descriptor
//! for Android: it holds ALL Android-specific knowledge in one place so adding
//! the target is data, not `if target == android` branches scattered through the
//! build.  The public entry is [`AndroidTarget::detect`] + [`AndroidTarget::build`].
//!
//! It reuses the same generated `.rs` that `--native` / `--native-wasm` emit
//! (there is no Android codegen path) and mirrors the on-demand runtime-rlib
//! build that `--native-wasm` uses
//! ([`crate::native_utils::ensure_loft_runtime_rlib`]), differing only in the
//! descriptor fields:
//!   * triple `aarch64-linux-android` (override with `LOFT_ANDROID_TARGET`),
//!   * the NDK `clang` wrapper for the API level as both C compiler and linker,
//!   * crate-type `cdylib` — an Android app loads a `.so`, it has no `fn main`.
//!
//! Requires the Android NDK (r26+): point `ANDROID_NDK_HOME` (or
//! `ANDROID_NDK_ROOT`) at it.  The produced `.so` is the JNI/`android_native_app`
//! payload an APK ships under `lib/arm64-v8a/`; packaging it into a runnable APK
//! is the next phase (B2) and lives outside this module.

use std::path::{Path, PathBuf};

/// A resolved Android cross-compile descriptor: the NDK, the API level, and the
/// Rust/NDK target triple.  Build one with [`AndroidTarget::detect`]; everything
/// else (linker path, runtime rlib, program `.so`) derives from these three.
pub(crate) struct AndroidTarget {
    /// NDK root (from `ANDROID_NDK_HOME` / `ANDROID_NDK_ROOT`).
    ndk: PathBuf,
    /// Android API level the NDK `clang` wrapper targets (`LOFT_ANDROID_API`,
    /// default 24 — the lowest with full 64-bit + modern `libc` coverage).
    api: u32,
    /// Rust target triple (`LOFT_ANDROID_TARGET`, default `aarch64-linux-android`).
    triple: String,
}

impl AndroidTarget {
    /// Resolve the Android target from the environment, or return a message
    /// naming exactly what to set.  Validates that the NDK directory and the
    /// per-API `clang` linker actually exist, so a misconfigured NDK fails here
    /// with an actionable error rather than an opaque `rustc`/`cc` failure later.
    pub(crate) fn detect() -> Result<Self, String> {
        let ndk = std::env::var_os("ANDROID_NDK_HOME")
            .or_else(|| std::env::var_os("ANDROID_NDK_ROOT"))
            .map(PathBuf::from)
            .ok_or_else(|| {
                "loft: --native-android needs the Android NDK — set ANDROID_NDK_HOME \
                 (or ANDROID_NDK_ROOT) to an NDK r26+ install (the `clang` linker + \
                 sysroot live there)."
                    .to_string()
            })?;
        if !ndk.is_dir() {
            return Err(format!(
                "loft: ANDROID_NDK_HOME points at '{}', which is not a directory.",
                ndk.display()
            ));
        }
        let triple = std::env::var("LOFT_ANDROID_TARGET")
            .unwrap_or_else(|_| "aarch64-linux-android".to_string());
        let api: u32 = std::env::var("LOFT_ANDROID_API")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24);
        let target = Self { ndk, api, triple };
        // Fail now, with the resolved path, if the toolchain wrapper is absent
        // (wrong host tag, NDK too old for this API, or a partial install).
        let linker = target.linker();
        if !linker.exists() {
            return Err(format!(
                "loft: NDK linker not found at '{}'.  Check the NDK is complete and that \
                 LOFT_ANDROID_API={} / LOFT_ANDROID_TARGET={} match a wrapper in {}.",
                linker.display(),
                target.api,
                target.triple,
                target.toolchain_bin().display()
            ));
        }
        Ok(target)
    }

    /// Prebuilt-toolchain `bin/` for this host inside the NDK
    /// (`toolchains/llvm/prebuilt/<host-tag>/bin`).
    fn toolchain_bin(&self) -> PathBuf {
        self.ndk
            .join("toolchains")
            .join("llvm")
            .join("prebuilt")
            .join(host_tag())
            .join("bin")
    }

    /// The NDK `clang` wrapper for `<triple><api>` — serves as BOTH the C
    /// compiler (for any C dep such as `ring`) and the rustc linker, because it
    /// pre-sets the Android sysroot and `--target`.
    fn linker(&self) -> PathBuf {
        self.toolchain_bin()
            .join(format!("{}{}-clang", self.triple, self.api))
    }

    /// Cross-build loft's own runtime rlib for the Android triple (on demand,
    /// fingerprint-keyed) and compile `rs_path` into `out_so` as an Android
    /// `cdylib`.  This is the whole `--native-android` pipeline; `main.rs` only
    /// emits the generated `.rs` and calls this.
    pub(crate) fn build(&self, rs_path: &Path, out_so: &Path) -> Result<(), String> {
        let libdir = self.ensure_runtime_rlib()?;
        self.compile_program(rs_path, out_so, &libdir)
    }

    /// Ensure `libloft.rlib` for the Android triple exists (building it with
    /// cargo if stale) and return the directory holding it.  Mirrors
    /// [`crate::native_utils::ensure_loft_runtime_rlib`] but injects the NDK
    /// linker so the `cdylib` crate-type in loft's `[lib]` links, and builds
    /// into an ISOLATED target dir (`target/loft/android/`) so a full-feature
    /// host `cargo build --target aarch64-linux-android` can never stomp this
    /// lean runtime, nor vice-versa (the same rlib-stomp guard the `--html`
    /// shape uses).
    ///
    /// The runtime is built `--no-default-features` with `random` + `threading`
    /// only — the headless core.  Networking/TLS (`ring`) and the registry are
    /// deliberately out of this first slice: they need the NDK C compiler wired
    /// through `cc-rs` and are their own phase (see the design's §5 failure
    /// paths).
    fn ensure_runtime_rlib(&self) -> Result<PathBuf, String> {
        let tree = crate::native_utils::loft_source_tree().ok_or_else(|| {
            "loft: --native-android needs loft's source tree to cross-build its Android \
             runtime rlib; run it from a loft checkout (a downloaded release ships no \
             Android runtime)."
                .to_string()
        })?;
        let target_dir = tree.join("target").join("loft").join("android");
        let profile_dir = target_dir.join(&self.triple).join("release");
        let rlib = profile_dir.join("libloft.rlib");
        let fp = loft::cache::loft_build_fingerprint();
        let fresh =
            rlib.exists() && loft::cache::native_artifact_fingerprint_matches(&profile_dir, fp);
        if fresh {
            return Ok(profile_dir);
        }
        if std::env::var_os("LOFT_NO_AUTO_REBUILD").is_some() {
            return if rlib.exists() {
                Ok(profile_dir)
            } else {
                Err(format!(
                    "loft: no Android runtime rlib at {} and LOFT_NO_AUTO_REBUILD is set; \
                     build it with `cargo build --release --lib --target {} --no-default-features \
                     --features 'random threading' --target-dir target/loft/android`.",
                    rlib.display(),
                    self.triple
                ))
            };
        }
        // Serialise cross-process builds on the SAME lock the package / wasm
        // auto-builds use, so parallel `loft` runs don't race cargo's registry
        // index or each other's output dir.
        let _build_lock = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(std::env::temp_dir().join("loft-native-build.lock"))
            .ok();
        if let Some(f) = &_build_lock {
            let _ = f.lock();
        }
        // A process we waited on may have just produced it.
        if rlib.exists() && loft::cache::native_artifact_fingerprint_matches(&profile_dir, fp) {
            return Ok(profile_dir);
        }
        eprintln!(
            "loft: building loft's Android runtime rlib for {} (NDK {}) — a one-time cost \
             until loft's source changes (typically under a minute).  Set \
             LOFT_NO_AUTO_REBUILD=1 to skip and link the existing rlib instead.",
            self.triple, self.api
        );
        let linker = self.linker();
        let mut cmd = std::process::Command::new("cargo");
        cmd.args([
            "build",
            "--release",
            "--lib",
            "--target",
            &self.triple,
            "--no-default-features",
            "--features",
            "random threading",
            "--target-dir",
        ])
        .arg(&target_dir)
        .current_dir(&tree)
        // The cdylib crate-type in loft's [lib] links even for a --lib build, so
        // the Android triple needs its linker set.  CC_/AR_ are set too so a
        // future feature with a C dep (ring) cross-compiles without extra wiring.
        .env(cargo_linker_var(&self.triple), &linker)
        .env(format!("CC_{}", self.triple), &linker)
        .env(
            format!("AR_{}", self.triple),
            self.toolchain_bin().join("llvm-ar"),
        )
        // CLEAN flags: the host RUSTFLAGS loft was built with are host-specific
        // (target-cpu) and would break or mis-key the Android build.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
        match cmd.status() {
            Ok(s) if s.success() && rlib.exists() => {
                loft::cache::write_native_artifact_fingerprint(&profile_dir, fp);
                Ok(profile_dir)
            }
            Ok(_) => Err(format!(
                "loft: could not cross-build loft's Android runtime rlib for {} — check \
                 `rustup target add {}` and that the NDK at {} is complete.",
                self.triple,
                self.triple,
                self.ndk.display()
            )),
            Err(e) => Err(format!(
                "loft: cannot run cargo to build the Android runtime rlib: {e}"
            )),
        }
    }

    /// Compile the emitted program `rs_path` into an Android `cdylib` at
    /// `out_so`, linking loft's Android runtime rlib from `libdir` with the NDK
    /// linker.  The `.so` is a genuine bionic AArch64 shared object
    /// (`NEEDED libc.so`, not glibc's `libc.so.6`).
    fn compile_program(&self, rs_path: &Path, out_so: &Path, libdir: &Path) -> Result<(), String> {
        let rlib = libdir.join("libloft.rlib");
        let deps = crate::native_utils::deps_dir_of(libdir);
        let mut cmd = std::process::Command::new("rustc");
        cmd.arg("--edition=2024")
            .arg("--target")
            .arg(&self.triple)
            .arg("--crate-type")
            .arg("cdylib")
            .arg("-O")
            .arg("-C")
            .arg(format!("linker={}", self.linker().display()))
            // Two native packages can each carry a copy of `loft_register_v1`;
            // merge duplicates (lld keeps the first) exactly as the host binary
            // path does.  Harmless for a single-crate program.
            .arg("-C")
            .arg("link-arg=-Wl,--allow-multiple-definition")
            .arg("-o")
            .arg(out_so)
            .arg("--extern")
            .arg(format!("loft={}", rlib.display()))
            .arg("-L")
            .arg(format!("dependency={}", deps.display()))
            .arg(rs_path);
        let output = cmd
            .output()
            .map_err(|e| format!("loft: failed to launch rustc for --native-android: {e}"))?;
        // Relay rustc's diagnostics so a real codegen/link error is visible.
        let _ = std::io::Write::write_all(&mut std::io::stderr(), &output.stderr);
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "loft: Android cdylib compile failed (inspect the source with \
                 --native-emit; runtime rlib at {}).",
                rlib.display()
            ))
        }
    }
}

/// The cargo linker override env var for a triple, e.g. `aarch64-linux-android`
/// → `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER`.
fn cargo_linker_var(triple: &str) -> String {
    format!(
        "CARGO_TARGET_{}_LINKER",
        triple.to_uppercase().replace('-', "_")
    )
}

/// The NDK prebuilt-toolchain host tag for the machine loft runs on.  Android
/// NDKs since r23 ship only x86_64 host toolchains (there is no aarch64-host or
/// 32-bit build), so every supported host maps to its `*-x86_64` tag.
fn host_tag() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin-x86_64"
    } else if cfg!(target_os = "windows") {
        "windows-x86_64"
    } else {
        "linux-x86_64"
    }
}
