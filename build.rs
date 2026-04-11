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
    // Only re-run when the git HEAD changes or build.rs itself changes.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    println!("cargo:rerun-if-changed=build.rs");
}
