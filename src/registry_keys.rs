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
    // Trust root bootstrapped 2026-06-13 (REGISTRY_BOOTSTRAP.md § Step 1.5) —
    // THREE INDEPENDENT keys: the client accepts a signature from ANY of them,
    // and a lost device's key is REVOKED by deleting just its entry here and
    // shipping a release (the other two keep signing — no rotation).
    //
    // K_laptop — software daily-signer (~/.loft/trust-root/registry-signing-key.bin)
    [
        0xa8, 0x8b, 0xb7, 0xc5, 0x45, 0x46, 0x8c, 0xd5, 0x44, 0x79, 0xa3, 0x9e, 0x3b, 0x29, 0x73,
        0x91, 0xa9, 0xc6, 0x2e, 0x17, 0x79, 0xfb, 0xe9, 0xdf, 0x2f, 0x35, 0xdf, 0xf2, 0xa4, 0x51,
        0xc0, 0x75,
    ],
    // K_yubiA — on-card, non-extractable (YubiKey serial 37327945, PIV slot 9C)
    [
        0x25, 0x6c, 0x53, 0x90, 0x26, 0x54, 0x65, 0xc5, 0x7d, 0x1f, 0x96, 0x46, 0xfe, 0x58, 0x1c,
        0x14, 0xbd, 0xdd, 0xb1, 0x62, 0x03, 0x5a, 0x60, 0x45, 0xb3, 0x5a, 0x94, 0x23, 0x6b, 0x3d,
        0x52, 0x8d,
    ],
    // K_yubiB — on-card, non-extractable (second YubiKey, PIV slot 9C)
    [
        0xbd, 0xf9, 0x91, 0xd2, 0x02, 0x86, 0x73, 0x36, 0xbf, 0x9a, 0x70, 0x20, 0x29, 0x28, 0x65,
        0x04, 0x5f, 0x62, 0xc9, 0x75, 0x1b, 0x7f, 0x09, 0xf3, 0x48, 0x7e, 0x7e, 0x66, 0xd2, 0xfa,
        0xc0, 0x17,
    ],
];

/// True when the binary embeds at least one trust root and can verify
/// registry signatures.  When false, all signature-requiring code
/// paths refuse to run unless the caller explicitly opted out via
/// `--allow-unsigned`.
#[must_use]
pub fn has_trust_root() -> bool {
    !TRUSTED_PUBLIC_KEYS.is_empty()
}
