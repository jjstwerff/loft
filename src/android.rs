//! Android build target (`--native-android`) — @PLN106 (B1 + B2).
//!
//! loft's native backend emits ONE target-agnostic Rust core and compiles it per
//! target; a build target is just a *descriptor* — `(rust triple, linker, runtime
//! entry, packaging)` — over that shared core (B0 confirmed the core cross-compiles
//! to `aarch64-linux-android` unchanged; see `106-android-build-target/README.md`).
//! This module is that descriptor for Android: it holds ALL Android-specific
//! knowledge in one place so adding the target is data, not `if target == android`
//! branches scattered through the build. Public entry: [`AndroidTarget::detect`] +
//! [`AndroidTarget::build`].
//!
//! **Runtime entry (B2).** An Android app has no `fn main`; it is a `NativeActivity`
//! whose `.so` exports `ANativeActivity_onCreate`, which calls a Rust `android_main`.
//! So `--native-android` wraps the emitted program in a tiny generated cargo crate:
//! the program becomes the crate root (its `fn main` untouched) and a fixed
//! [`ANDROID_MAIN_TAIL`] adds the `android_main` entry (via the `android-activity`
//! crate) that runs `main()` and forwards its stdout into logcat. Cargo builds the
//! crate, resolving `android-activity`'s dep tree and cross-building loft (a lean
//! path dep) for the triple; the resulting `.so` is dropped in `lib/<abi>/` of an
//! APK (the `b2-spike/` recipe; loft-side APK packaging is the next slice).
//!
//! There is NO Android codegen path: the program `.rs` is the same
//! `output_native_reachable` emit `--native` / `--native-wasm` produce — only the
//! entry tail and the toolchain differ. Requires the Android NDK (r26+): point
//! `ANDROID_NDK_HOME` (or `ANDROID_NDK_ROOT`) at it. Build the `x86_64-linux-android`
//! triple (`LOFT_ANDROID_TARGET`) for a KVM emulator; `aarch64` is the ship target.

use std::path::{Path, PathBuf};

/// A resolved Android cross-compile descriptor: the NDK, the API level, and the
/// Rust/NDK target triple. Build one with [`AndroidTarget::detect`]; the linker,
/// the generated crate, and the `.so` all derive from these three.
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
    /// naming exactly what to set. Validates that the NDK directory and the
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

    /// The NDK `clang` wrapper for `<triple><api>` — serves as BOTH the C compiler
    /// (android-activity's `native_app_glue`, and any future C dep) and the rustc
    /// linker, because it pre-sets the Android sysroot and `--target`.
    fn linker(&self) -> PathBuf {
        self.toolchain_bin()
            .join(format!("{}{}-clang", self.triple, self.api))
    }

    /// Wrap the emitted program `rs_path` in a `NativeActivity` cdylib and build
    /// it into `out_so` — the whole `--native-android` pipeline. `main.rs` only
    /// emits the program `.rs` and calls this.
    ///
    /// Generates a tiny cargo crate under `<tree>/target/loft/android-app/` whose
    /// crate root IS the emitted program (its `fn main` intact) plus the fixed
    /// [`ANDROID_MAIN_TAIL`] entry, with `android-activity` + a lean `loft` path
    /// dep, then `cargo build --release --target <triple>` and copies the cdylib
    /// out. Cargo (not hand-rolled rustc) owns the `android-activity` dep tree and
    /// the cross-build of loft, so there is no transitive-rlib bookkeeping here.
    pub(crate) fn build(&self, rs_path: &Path, out_so: &Path) -> Result<(), String> {
        let tree = crate::native_utils::loft_source_tree().ok_or_else(|| {
            "loft: --native-android needs loft's source tree to cross-build its Android \
             runtime; run it from a loft checkout (a downloaded release ships no Android \
             runtime)."
                .to_string()
        })?;
        let crate_dir = tree.join("target").join("loft").join("android-app");
        let src_dir = crate_dir.join("src");
        std::fs::create_dir_all(&src_dir)
            .map_err(|e| format!("loft: create {}: {e}", src_dir.display()))?;

        // Serialise on the SAME lock the package / runtime auto-builds use: the
        // crate dir is shared across runs (so cargo caches android-activity + loft),
        // so concurrent `--native-android` runs must not clobber each other's lib.rs.
        let _build_lock = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(std::env::temp_dir().join("loft-native-build.lock"))
            .ok();
        if let Some(f) = &_build_lock {
            let _ = f.lock();
        }

        // Crate root = the emitted program (its crate-level `#![allow]` attrs and
        // `fn main` stay valid at the root) + the android_main entry tail.
        let program = std::fs::read_to_string(rs_path)
            .map_err(|e| format!("loft: read emitted program {}: {e}", rs_path.display()))?;
        let lib_rs = format!("{program}\n{ANDROID_MAIN_TAIL}");
        std::fs::write(src_dir.join("lib.rs"), lib_rs)
            .map_err(|e| format!("loft: write lib.rs: {e}"))?;
        std::fs::write(crate_dir.join("Cargo.toml"), cargo_toml(&tree))
            .map_err(|e| format!("loft: write Cargo.toml: {e}"))?;

        let linker = self.linker();
        let mut cmd = std::process::Command::new("cargo");
        cmd.args(["build", "--release", "--target", &self.triple])
            .current_dir(&crate_dir)
            .env("ANDROID_NDK_ROOT", &self.ndk)
            .env("ANDROID_NDK_HOME", &self.ndk)
            .env(cargo_linker_var(&self.triple), &linker)
            .env(format!("CC_{}", self.triple), &linker)
            .env(
                format!("AR_{}", self.triple),
                self.toolchain_bin().join("llvm-ar"),
            )
            // Host RUSTFLAGS (target-cpu etc.) are host-specific and would break the
            // Android build — this crate is target-defined, not host-flag-defined.
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let status = cmd
            .status()
            .map_err(|e| format!("loft: cannot run cargo for --native-android: {e}"))?;
        if !status.success() {
            return Err(format!(
                "loft: Android build failed for {} (generated crate kept at {} — inspect \
                 src/lib.rs).",
                self.triple,
                crate_dir.display()
            ));
        }
        let built = crate_dir
            .join("target")
            .join(&self.triple)
            .join("release")
            .join("libloft_android_app.so");
        std::fs::copy(&built, out_so).map(|_| ()).map_err(|e| {
            format!(
                "loft: copy {} -> {}: {e}",
                built.display(),
                out_so.display()
            )
        })
    }
}

/// The generated crate's `Cargo.toml`: a `cdylib` NativeActivity app depending on
/// `android-activity` (+ logging + libc) and a lean `loft` path dep (matching the B1
/// runtime: `random` + `threading`, no `ring`/registry). The empty `[workspace]`
/// stops cargo walking up into the loft package that contains this directory. The
/// path is Debug-quoted via a `String` (not the `Path`, which the lint rejects as
/// lossy) so it lands as a properly-escaped TOML string.
fn cargo_toml(tree: &Path) -> String {
    let tree = tree.display().to_string();
    format!(
        "[package]\n\
         name = \"loft_android_app\"\n\
         version = \"0.0.0\"\n\
         edition = \"2024\"\n\
         publish = false\n\n\
         [workspace]\n\n\
         [lib]\n\
         crate-type = [\"cdylib\"]\n\n\
         [dependencies]\n\
         android-activity = {{ version = \"0.6\", features = [\"native-activity\"] }}\n\
         log = \"0.4\"\n\
         android_logger = \"0.14\"\n\
         libc = \"0.2\"\n\
         loft = {{ path = {tree:?}, default-features = false, features = [\"random\", \"threading\"] }}\n\n\
         [profile.release]\n\
         opt-level = 2\n"
    )
}

/// The Android `NativeActivity` entry, appended verbatim to the emitted program's
/// crate root. `android-activity` (native-activity) exports `ANativeActivity_onCreate`
/// and calls this `android_main` on a dedicated thread; we run the loft program's
/// `main` in it and forward the program's stdout into logcat so `print()` output is
/// visible via `adb logcat`.
const ANDROID_MAIN_TAIL: &str = r#"
// ---- @PLN106 B2: Android NativeActivity entry (emitted by --native-android) ----
fn __loft_android_redirect_stdio_to_logcat() {
    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: i32, tag: *const libc::c_char, text: *const libc::c_char) -> i32;
    }
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return;
    }
    let (rd, wr) = (fds[0], fds[1]);
    unsafe {
        libc::dup2(wr, libc::STDOUT_FILENO);
        libc::dup2(wr, libc::STDERR_FILENO);
        libc::close(wr);
    }
    std::thread::spawn(move || {
        const TAG: &[u8] = b"loft-stdout\0";
        let mut buf = [0u8; 1024];
        let mut line: Vec<u8> = Vec::new();
        loop {
            let n = unsafe { libc::read(rd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            for &b in &buf[..n as usize] {
                if b == b'\n' {
                    line.push(0);
                    unsafe {
                        __android_log_write(
                            4, // ANDROID_LOG_INFO
                            TAG.as_ptr() as *const libc::c_char,
                            line.as_ptr() as *const libc::c_char,
                        );
                    }
                    line.clear();
                } else {
                    line.push(b);
                }
            }
        }
    });
}

#[unsafe(no_mangle)]
extern "C" fn android_main(app: android_activity::AndroidApp) {
    use android_activity::{MainEvent, PollEvent};
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("loft"),
    );
    log::info!("loft --native-android: android_main reached");
    __loft_android_redirect_stdio_to_logcat();
    // Run the loft program (its `fn main`); stdout is now piped into logcat.
    main();
    log::info!("loft --native-android: program returned");
    // Stay alive to drain lifecycle events (a real app renders here) until the OS
    // destroys the activity, so the process does not exit out from under the app.
    loop {
        let mut destroy = false;
        app.poll_events(Some(std::time::Duration::from_millis(250)), |event| {
            if let PollEvent::Main(MainEvent::Destroy) = event {
                destroy = true;
            }
        });
        if destroy {
            break;
        }
    }
}
"#;

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
