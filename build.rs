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

    // Only re-run when the git HEAD changes or build.rs itself changes.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    println!("cargo:rerun-if-changed=build.rs");
    // …and when the build flags change, so LOFT_BUILD_RUSTFLAGS stays accurate.
    println!("cargo:rerun-if-env-changed=RUSTFLAGS");
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
}
