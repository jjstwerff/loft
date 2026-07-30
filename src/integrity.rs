// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! Content integrity — "are these the bytes I was promised?"
//!
//! One check, three unrelated consumers: the registry install verifies a downloaded
//! package, `store_load_url` verifies a fetched store image before adopting it, and the
//! prebuilt-binary path verifies a cdylib. None of them is *the registry*, which is why
//! this does not live in `registry_index`: parking it there made it `registry`-gated, and
//! a gate is contagious — `store_load_url` was unavailable on the browser target purely
//! because its hash check was in a module the browser does not compile (loft#678). The
//! check itself needs nothing but `sha2`, which is a base dependency on every target.

/// The SHA-256 of `bytes` as lower-case hex — the digest form that goes into a
/// manifest, a lockfile, or a `store_load_url` call.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut hex = String::with_capacity(64);
    for b in hasher.finalize() {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// Verify the SHA-256 of `bytes` against `expected_hex`.
///
/// Use this before ADOPTING bytes from anywhere you do not control — a registry
/// download, an `http(s)://` store image, a prebuilt artefact. Comparison is
/// case-insensitive, so hand-written upper-case hex in a manifest still matches.
///
/// # Errors
/// Returns `Err` naming both digests when they differ.
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<(), String> {
    let hex = sha256_hex(bytes);
    if hex.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(format!(
            "sha256 mismatch: expected `{expected_hex}`, got `{hex}`"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::verify_sha256;

    /// The SHA-256 of `b"hello world"` (well-known test vector).
    const HELLO_WORLD_SHA: &str =
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    #[test]
    fn verify_sha256_accepts_match() {
        assert!(verify_sha256(b"hello world", HELLO_WORLD_SHA).is_ok());
    }

    #[test]
    fn verify_sha256_rejects_mismatch() {
        let result = verify_sha256(
            b"hello world",
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        let err = result.expect_err("should reject");
        assert!(err.contains("sha256 mismatch"), "msg: {err}");
        assert!(err.contains(HELLO_WORLD_SHA), "msg: {err}");
    }

    #[test]
    fn verify_sha256_case_insensitive() {
        // Upper-case hex from a hand-written PR should still match.
        let upper = HELLO_WORLD_SHA.to_ascii_uppercase();
        assert!(verify_sha256(b"hello world", &upper).is_ok());
    }

    #[test]
    fn verify_sha256_rejects_corruption() {
        // Flip one byte → mismatch.
        let mut bytes = b"hello world".to_vec();
        bytes[0] ^= 0x01;
        assert!(verify_sha256(&bytes, HELLO_WORLD_SHA).is_err());
    }
}
