// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `loft-keygen` — Ed25519 keypair generator for the registry trust root.
//!
//! Run on an **offline / air-gapped machine** when bootstrapping the
//! `loft-lang/registry` repo (REGISTRY_BOOTSTRAP.md Step 1).
//!
//! Usage:
//!
//! ```sh
//! cargo build --release --bin loft-keygen --features registry
//! ./target/release/loft-keygen generate
//! ./target/release/loft-keygen format --in registry-signing-key.pub
//! ```
//!
//! `generate`:
//! - Writes the 32-byte raw private key to `registry-signing-key.bin`
//!   (chmod 600).  This file MUST NOT leave the offline machine
//!   except as a base64-encoded GitHub Actions secret.
//! - Writes the 32-byte public key as hex to `registry-signing-key.pub`.
//! - Prints the public hex and the GitHub-secret base64 of the
//!   private key to stdout for copy/paste.
//!
//! `format`:
//! - Takes a hex-encoded public key (from `--in <file>` or stdin)
//!   and emits a Rust `[u8; 32]` literal ready to paste into
//!   `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS`.
//!
//! No network access.  No persistent state.  The binary doesn't
//! call home, doesn't log anything beyond stdout/stderr, and
//! doesn't keep the key in memory beyond the one stack frame.

#![cfg(feature = "registry")]
#![allow(clippy::missing_docs_in_private_items)]

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use ed25519_dalek::SigningKey;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    match cmd {
        "generate" => cmd_generate(&args[2..]),
        "format" => cmd_format(&args[2..]),
        _ => {
            print_help();
            std::process::exit(if cmd.is_empty() { 0 } else { 1 });
        }
    }
}

fn print_help() {
    eprintln!("loft-keygen — Ed25519 keypair tool for the loft registry trust root.");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  generate                  Create a new keypair.");
    eprintln!("                            Writes registry-signing-key.bin (private)");
    eprintln!("                            and registry-signing-key.pub (public hex).");
    eprintln!();
    eprintln!("  format --in <pub_file>    Read hex public key from <pub_file>, emit");
    eprintln!("                            a Rust [u8; 32] literal ready to paste into");
    eprintln!("                            src/registry_keys.rs::TRUSTED_PUBLIC_KEYS.");
    eprintln!("                            (omit --in to read from stdin)");
    eprintln!();
    eprintln!("Run `generate` on an offline machine.  The .bin file is the");
    eprintln!("master private key — back it up to a hardware token and a sealed");
    eprintln!("offline location, then add its base64 form as the");
    eprintln!("REGISTRY_SIGNING_KEY_BASE64 secret in loft-lang/registry.");
}

fn cmd_generate(_extra: &[String]) {
    // Use the OS CSPRNG via getrandom — `ed25519_dalek::SigningKey`'s
    // `generate` takes a `CryptoRngCore`; OsRng from rand_core 0.6 (the
    // version dalek 2.x expects).  We're already paying for the dalek
    // dependency, so use its bundled prelude here.
    let mut seed = [0u8; 32];
    getrandom_fill(&mut seed).unwrap_or_else(|e| {
        eprintln!("loft-keygen: cannot read OS entropy: {e}");
        std::process::exit(1);
    });
    let signing_key = SigningKey::from_bytes(&seed);
    let priv_bytes = signing_key.to_bytes();
    let pub_bytes = signing_key.verifying_key().to_bytes();

    let priv_path = Path::new("registry-signing-key.bin");
    let pub_path = Path::new("registry-signing-key.pub");

    if priv_path.exists() || pub_path.exists() {
        eprintln!(
            "loft-keygen: refusing to overwrite existing key files in cwd.\n  \
             Move or delete `registry-signing-key.bin` / `registry-signing-key.pub` first."
        );
        std::process::exit(1);
    }

    fs::write(priv_path, priv_bytes)
        .unwrap_or_else(|e| die(&format!("write {}: {e}", priv_path.display())));
    fs::write(pub_path, hex_encode(&pub_bytes))
        .unwrap_or_else(|e| die(&format!("write {}: {e}", pub_path.display())));
    chmod_600(priv_path);

    println!("Generated keypair:");
    println!(
        "  private:    {} (32 bytes, chmod 600)",
        priv_path.display()
    );
    println!("  public:     {} (64-char hex)", pub_path.display());
    println!();
    println!("Public key (paste into src/registry_keys.rs::TRUSTED_PUBLIC_KEYS):");
    println!("    {}", format_rust_literal(&pub_bytes));
    println!();
    println!("GitHub secret (paste into loft-lang/registry as REGISTRY_SIGNING_KEY_BASE64):");
    println!("    {}", base64_encode(&priv_bytes));
    println!();
    println!("Now:");
    println!("  1. Back up registry-signing-key.bin to a hardware token + sealed offline copy.");
    println!("  2. Delete registry-signing-key.bin from this machine once backed up.");
    println!("  3. Edit src/registry_keys.rs and add the public-key literal above.");
    println!("  4. Add REGISTRY_SIGNING_KEY_BASE64 as a secret in loft-lang/registry.");
}

fn cmd_format(extra: &[String]) {
    let hex_text = if let Some(pos) = extra.iter().position(|a| a == "--in")
        && let Some(path) = extra.get(pos + 1)
    {
        fs::read_to_string(path).unwrap_or_else(|e| die(&format!("read {path}: {e}")))
    } else {
        let mut s = String::new();
        io::stdin()
            .read_to_string(&mut s)
            .unwrap_or_else(|e| die(&format!("read stdin: {e}")));
        s
    };
    let hex = hex_text.trim();
    let bytes = hex_decode(hex).unwrap_or_else(|e| die(&format!("invalid hex: {e}")));
    if bytes.len() != 32 {
        die(&format!(
            "expected 32 bytes of hex (64 chars), got {} bytes",
            bytes.len()
        ));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    println!("    {}", format_rust_literal(&arr));
}

fn format_rust_literal(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("[");
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        let _ = write!(s, "0x{b:02x}");
    }
    s.push(']');
    s
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err(format!("odd number of hex chars: {}", s.len()));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks(2) {
        let h = std::str::from_utf8(chunk).map_err(|e| e.to_string())?;
        out.push(u8::from_str_radix(h, 16).map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn base64_encode(bytes: &[u8]) -> String {
    crate::base64_local::encode(bytes)
}

mod base64_local {
    //! Local base64 — duplicate of `src/base64.rs` so this binary can
    //! emit the GitHub-secret format without depending on a feature
    //! we haven't pulled in.  ~30 lines, no allocation hot paths.
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    #[must_use]
    pub fn encode(data: &[u8]) -> String {
        let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b0 = u32::from(chunk[0]);
            let b1 = if chunk.len() > 1 {
                u32::from(chunk[1])
            } else {
                0
            };
            let b2 = if chunk.len() > 2 {
                u32::from(chunk[2])
            } else {
                0
            };
            let n = (b0 << 16) | (b1 << 8) | b2;
            result.push(CHARS[((n >> 18) & 63) as usize] as char);
            result.push(CHARS[((n >> 12) & 63) as usize] as char);
            if chunk.len() > 1 {
                result.push(CHARS[((n >> 6) & 63) as usize] as char);
            } else {
                result.push('=');
            }
            if chunk.len() > 2 {
                result.push(CHARS[(n & 63) as usize] as char);
            } else {
                result.push('=');
            }
        }
        result
    }
}

#[cfg(unix)]
fn chmod_600(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(metadata) = fs::metadata(path) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o600);
        let _ = fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn chmod_600(_path: &Path) {
    // Windows: NTFS ACLs are out of scope for the bootstrap.  The
    // private key file should still be on an air-gapped machine
    // anyway, where filesystem perms are secondary.
}

fn getrandom_fill(buf: &mut [u8]) -> io::Result<()> {
    // /dev/urandom path keeps the dependency surface minimal — no
    // `rand` / `getrandom` crate needed.  Same entropy source on
    // Linux + macOS + the BSDs.  Windows falls back to the OS-level
    // BCryptGenRandom via a tiny wrapper; if the user's air-gap
    // machine is Linux/macOS this code path is what runs.
    #[cfg(unix)]
    {
        let mut f = fs::File::open("/dev/urandom")?;
        f.read_exact(buf)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        // For non-Unix, defer to the user running keygen on a real
        // air-gapped Unix box.  The bootstrap doc recommends this
        // anyway.
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "loft-keygen needs /dev/urandom (Unix-only).  \
             Run on Linux/macOS instead.",
        ))
    }
}

fn die(msg: &str) -> ! {
    eprintln!("loft-keygen: {msg}");
    std::process::exit(1);
}
