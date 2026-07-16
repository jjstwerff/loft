// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 case B (soften-nullflow-discharge.md) — the sign/lower-bound lattice that lets a
//! domain-fault op with a PROVABLY in-domain argument type non-null, so no `??` discharge is
//! forced.  Opt-in behind `LOFT_MATH_DOMAIN` (default off) until the B5 flip, so this drives
//! the binary as a subprocess with/without the flag.  The soundness half — unprovable args
//! MUST stay `float?` even under the flag — is the load-bearing assertion (a wrong non-null
//! proof would store a runtime null into a non-null slot).

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Compile `src` on the interpreter, with `LOFT_MATH_DOMAIN` set iff `flag`.
/// Returns `(compiled_ok, stdout+stderr)`.
fn compile(tag: &str, src: &str, flag: bool) -> (bool, String) {
    let path = std::env::temp_dir().join(format!("loft_mathdom_{}_{tag}.loft", std::process::id()));
    std::fs::write(&path, src).expect("write temp");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(&path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env_remove("LOFT_MATH_DOMAIN");
    if flag {
        cmd.env("LOFT_MATH_DOMAIN", "1");
    }
    let out = cmd.output().expect("invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), combined)
}

// Provably in-domain EXPRESSION arguments: square, sum-of-squares, max-with-positive, abs,
// pow-of-nonneg-base, ln-of-positive.  Forced (float?) by default, softened under the flag.
const PROVABLE: &str = "\
fn f(x: float, y: float) {
  p1: float = sqrt(x * x + y * y);
  p2: float = sqrt(max(x, 0.01));
  p3: float = sqrt(abs(x));
  p4: float = pow(abs(x), 2.4);
  p5: float = ln(max(x, 0.01));
}
fn main() { }
";

// Unprovable arguments — each MUST stay float? even under the flag (soundness controls):
// unknown sign, distinct-operand product, subtraction, and ln of a merely-NonNeg value.
const UNPROVABLE: &str = "\
fn f(x: float, y: float) {
  n1: float = sqrt(x);
  n2: float = sqrt(x * y);
  n3: float = sqrt(x - 1.0);
  n4: float = ln(max(x, 0.0));
}
fn main() { }
";

#[test]
fn math_domain_off_by_default_keeps_expression_args_forced() {
    let (ok, diag) = compile("default", PROVABLE, false);
    assert!(
        !ok,
        "without LOFT_MATH_DOMAIN a non-constant fault-op arg must stay float? (forced); diag={diag}"
    );
}

#[test]
fn math_domain_softens_provable_args() {
    let (ok, diag) = compile("provable", PROVABLE, true);
    assert!(
        ok,
        "with LOFT_MATH_DOMAIN a provably in-domain arg must type non-null (no ?? forced); diag={diag}"
    );
}

#[test]
fn math_domain_keeps_unprovable_args_nullable() {
    let (ok, diag) = compile("unprovable", UNPROVABLE, true);
    assert!(
        !ok,
        "unprovable args must stay float? even under LOFT_MATH_DOMAIN (soundness); got success"
    );
    for v in ["n1", "n2", "n3", "n4"] {
        assert!(
            diag.contains(&format!("Variable '{v}'")),
            "control {v} must remain forced float?; diag={diag}"
        );
    }
}
