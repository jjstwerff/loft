// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Trust-root public keys for the loft package registry.
//!
//! PKG.REG R3.5.  See
//! [PKG_REGISTRY.md § Index signing](../doc/claude/PKG_REGISTRY.md#index-signing--indexjsonsig)
//! for the threat model + key-rotation procedure.
//!
//! Each entry is a 32-byte Ed25519 public key (raw, NOT PEM/DER).
//! The corresponding private key is held by the loft maintainers and
//! used by the `loft-lang/registry` CI workflow to sign
//! `index.json` → `index.json.sig`.  Clients verify the signature
//! before trusting any data in the index.
//!
//! Multiple keys can be embedded simultaneously so key rotation is
//! seamless: during the transition window the index is signed by
//! BOTH the old and new keys, and clients accept either.  Once the
//! ecosystem has upgraded past the rotation point, the old key is
//! removed in a subsequent loft release.
//!
//! ## Current trust roots
//!
//! Today this list is intentionally **empty** — no production registry
//! exists yet (R3 bootstrap pending).  When the
//! `loft-lang/registry` repo is bootstrapped:
//!
//! 1. Maintainers generate an Ed25519 keypair offline.
//! 2. The 32-byte public key is hex-encoded and added to
//!    `TRUSTED_PUBLIC_KEYS` below.
//! 3. The private half is stored as a `loft-lang/registry` repo
//!    secret + a backup copy in the maintainers' password manager.
//! 4. A loft minor release ships the binary with the new
//!    constant; future binaries built from this source tree pick it
//!    up automatically.
//!
//! Until step 2 lands, clients invoking signature-verified flows
//! (R4+) MUST be passed `--allow-unsigned` explicitly — otherwise
//! the install refuses to proceed.  This is by design: a missing
//! trust root is a configuration error, not a "trust everything"
//! signal.

/// 32-byte Ed25519 public keys embedded at compile time.  Empty until
/// R3 bootstrap.  Format: `[key_bytes; 32]` per entry.
///
/// `&'static [[u8; 32]]` keeps the type explicit so adding a key is
/// just appending a new 32-byte literal — no allocations, no parsing,
/// trivially zero-runtime-cost.
pub const TRUSTED_PUBLIC_KEYS: &[[u8; 32]] = &[
    // Add entries here when the real registry is bootstrapped:
    //
    // [
    //     0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, // ...32 bytes total
    // ],
];

/// True when the binary embeds at least one trust root and can verify
/// registry signatures.  When false, all signature-requiring code
/// paths refuse to run unless the caller explicitly opted out via
/// `--allow-unsigned`.
#[must_use]
pub fn has_trust_root() -> bool {
    !TRUSTED_PUBLIC_KEYS.is_empty()
}
