// Expose a BUILD_ID to the compiler so the bytecode cache can detect
// same-version rebuilds (e.g. a parser fix without a version bump).
// Uses the git HEAD commit hash when available, otherwise a timestamp.

fn main() {
    let id = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| {
            // Fallback: seconds since epoch — changes on every build.
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default()
        });
    println!("cargo:rustc-env=LOFT_BUILD_ID={id}");

    // Record the effective RUSTFLAGS this loft build used so the package
    // native-crate build (`extensions::auto_build_native`) can compile shared
    // transitive deps (e.g. `libloading`) with the SAME flags (#274).  A
    // debuginfo divergence — loft built with `-g` (the Makefile does), the
    // package without — gives the shared dep a different SVH; rustc then finds
    // two copies with one `StableCrateId` and aborts at the generated program's
    // link step.  `CARGO_ENCODED_RUSTFLAGS` is set by cargo for the build script
    // and captures flags from env AND `.cargo/config`; its `\x1f` separators
    // decode to a space-joined `RUSTFLAGS` string.
    let rustflags = std::env::var("CARGO_ENCODED_RUSTFLAGS")
        .map(|enc| {
            enc.split('\u{1f}')
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .or_else(|_| std::env::var("RUSTFLAGS"))
        .unwrap_or_default();
    println!("cargo:rustc-env=LOFT_BUILD_RUSTFLAGS={}", rustflags.trim());

    // Stamp the rustc version this loft (and its rlib) was built with, so the
    // native path can detect — up front, before a doomed compile — that the
    // toolchain changed under it.  rlibs are SVH-locked to one rustc, so after a
    // `rustup update` the cached rlib no longer links with the new rustc; loft
    // compares the live `rustc --version` against this stamp and, for a default
    // native run, falls back to the interpreter instead of attempting (and
    // failing) the compile.  Uses cargo's `RUSTC` (the exact rustc in use).
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let rustc_version = std::process::Command::new(&rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=LOFT_BUILD_RUSTC={rustc_version}");

    // Fingerprint loft-ffi's SOURCE — the stable cache key for REGISTRY cdylibs
    // (`extensions::auto_build_native`).  A registry cdylib links loft-ffi (the
    // C-ABI), NEVER libloft.rlib — verified: the cdylib has zero loft undefined
    // symbols, only system NEEDED libs.  So its validity depends on loft-ffi,
    // not the interpreter.  Hashing loft-ffi's SOURCE (not a compiled artifact)
    // gives a key identical across debug/release/test profiles, so the shared
    // ~/.loft/build-cache is reused across every loft build in a job instead of
    // cross-invalidating on the libloft.rlib hash (which differs per profile);
    // it still flips on any real loft-ffi change (ABI or impl, even
    // un-version-bumped).  FNV-1a/64 over the sorted `loft-ffi/src/*.rs` bytes.
    let mut ffi_fp: u64 = 0xcbf2_9ce4_8422_2325;
    let mut ffi_files: Vec<std::path::PathBuf> = std::fs::read_dir("loft-ffi/src")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    ffi_files.sort();
    for f in &ffi_files {
        if let Ok(bytes) = std::fs::read(f) {
            for b in &bytes {
                ffi_fp ^= u64::from(*b);
                ffi_fp = ffi_fp.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    println!("cargo:rustc-env=LOFT_FFI_FINGERPRINT={ffi_fp}");

    // @PLN21 Phase 1 — the target triple this loft was built for (e.g.
    // `x86_64-unknown-linux-gnu`).  cargo sets `TARGET` for build scripts; it is
    // the *authoritative* host triple — `std::env::consts` only yields
    // `<arch>-<os>-<family>` (`x86_64-linux-unix`), which loses the libc/abi a
    // prebuilt cdylib's portability hinges on (gnu vs musl).  Names the
    // `prebuilt/<triple>/` dir loft loads a precompiled cdylib from.
    println!(
        "cargo:rustc-env=LOFT_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap_or_default()
    );

    // @PLN117 — one name for "this build's `par` runs on the page's GLOBAL rayon
    // pool" (`parallel.rs::with_pool`): the wasm-bindgen browser bundle, or
    // loft's own browser threading on a wasm target.  A native build that merely
    // has the threading feature switched on — `cargo clippy --all-features` does
    // — keeps the private native pool, because there is no page to install one.
    println!("cargo:rustc-check-cfg=cfg(browser_pool)");
    let wasm_target = std::env::var("TARGET").unwrap_or_default().starts_with("wasm32");
    let wasm_bindgen_build = std::env::var_os("CARGO_FEATURE_WASM").is_some();
    let loft_browser_threads = std::env::var_os("CARGO_FEATURE_WASM_NATIVE_THREADS").is_some();
    if wasm_bindgen_build || (loft_browser_threads && wasm_target) {
        println!("cargo:rustc-cfg=browser_pool");
    }

    // Re-run when anything that identifies this build changes: git HEAD / refs
    // (committed state), build.rs itself, the compiler source (`src/`) and stdlib
    // (`default/`), and loft-ffi's source.  Without src/ + default/, build.rs never
    // recomputes on an uncommitted WIP edit, so a source-content build id goes
    // stale across WIP rebuilds and the bytecode/stdlib cache mis-reads a store
    // laid out by a different binary (the cross-checkout / WIP-rebuild corruption:
    // `d_nr=u32::MAX`).  loft-ffi/src keeps LOFT_FFI_FINGERPRINT accurate.  git
    // HEAD alone is blind to WIP; these make the rerun fire on the edits a content
    // hash must reflect.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=default");
    println!("cargo:rerun-if-changed=loft-ffi/src");
    // …and when the build flags change, so LOFT_BUILD_RUSTFLAGS stays accurate.
    println!("cargo:rerun-if-env-changed=RUSTFLAGS");
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
    // …and when rustc itself changes, so LOFT_BUILD_RUSTC stays accurate.
    println!("cargo:rerun-if-env-changed=RUSTC");
}
