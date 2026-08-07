// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#796 — a struct field the layout has no slot for must be a compile error,
//! never an access.
//!
//! `Stores::position` answers `u16::MAX` for a field the layout does not have, and that
//! answer used to flow straight out as the field OFFSET: the interpreter read and WROTE
//! at `record + 65535`. In the reported case a ten-field struct was laid out with nine —
//! the tenth field's type was declared in a module loaded later — so every write to that
//! field landed past the end of the record.
//!
//! What made it expensive to find is that the damage is entirely in the neighbours'
//! bytes, so the SYMPTOM depends on what happened to be next to the record: a SIGSEGV
//! inside a later claim walk on one run, an unbounded allocation on another (one reached
//! 59.6 GiB and had the kernel kill unrelated processes), and a clean pass on a third.
//! Non-deterministic, non-monotonic in scene size, and reported as "same code is fine as
//! a program" — all downstream of one silent out-of-range offset.
//!
//! The layout gap itself is a separate matter: `Roofs` below IS declared in the package,
//! just in a file loaded after the struct that names it, and a later pass resolves the
//! ATTRIBUTE while the layout keeps the hole. That is worth closing on its own. This test
//! pins the part that must never regress — the hole is refused, loudly, at the site.

use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

/// A package whose ENTRY declares `Roofs` and pulls in a module that uses it as a field
/// type — so the field's type is not yet declared when the module's struct is laid out.
fn build_package(root: &Path) {
    write(
        &root.join("fwd/loft.toml"),
        "[package]\nname = \"fwd\"\nversion = \"0.1.0\"\n[library]\nentry = \"src/fwd.loft\"\n",
    );
    write(
        &root.join("fwd/src/fwd.loft"),
        "use inner;\npub struct Roofs { items: vector<integer> }\n",
    );
    write(
        &root.join("fwd/src/inner.loft"),
        "pub struct Sess { s_a: integer, s_roofs: Roofs, s_b: integer }\n\
         pub fn mk() -> Sess { return Sess { s_a: 1, s_roofs: Roofs { items: [] }, s_b: 2 }; }\n",
    );
}

#[test]
fn a_field_with_no_storage_is_refused_at_the_site() {
    let root = std::env::temp_dir().join("loft_796_field_storage");
    let _ = std::fs::remove_dir_all(&root);
    build_package(&root);
    let prog = root.join("use_it.loft");
    write(
        &prog,
        "use fwd;\nfn main() { s = mk(); println(\"{s.s_a}{s.s_b}\"); }\n",
    );

    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--lib")
        .arg(&root)
        .arg(&prog)
        .env("LOFT_TIMEOUT", "60")
        .current_dir(root.join("fwd"))
        .output()
        .expect("spawn loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        all.contains("has no storage"),
        "a field the layout has no slot for must be refused, not written at \
         `record + 65535`.\n{all}"
    );
    // Naming the FIELD and its owner is the whole value: the fault surfaces far from
    // here (in a claim walk over a neighbouring record), so a message that did not name
    // the field would leave the reader where this bug already left several people.
    assert!(
        all.contains("s_roofs") && all.contains("Sess"),
        "the message must name the field and the struct.\n{all}"
    );
    assert!(
        !out.status.success(),
        "the run must FAIL — the alternative is memory corruption.\n{all}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_ordinary_cross_module_field_still_compiles() {
    // The control that keeps the check honest: the SAME shape with the field type
    // declared before it is used must be unaffected.  Without this, a check that
    // refused every cross-module field type would pass the test above.
    let root = std::env::temp_dir().join("loft_796_field_storage_ok");
    let _ = std::fs::remove_dir_all(&root);
    write(
        &root.join("okp/loft.toml"),
        "[package]\nname = \"okp\"\nversion = \"0.1.0\"\n[library]\nentry = \"src/okp.loft\"\n",
    );
    // `roofs` is a module of its own, `use`d BEFORE `inner`, so its type is declared
    // by the time `inner`'s struct is laid out.
    write(
        &root.join("okp/src/roofs.loft"),
        "pub struct Roofs { items: vector<integer> }\n",
    );
    write(&root.join("okp/src/okp.loft"), "use roofs;\nuse inner;\n");
    write(
        &root.join("okp/src/inner.loft"),
        "use roofs;\npub struct Sess { s_a: integer, s_roofs: Roofs, s_b: integer }\n\
         pub fn mk() -> Sess { return Sess { s_a: 1, s_roofs: Roofs { items: [] }, s_b: 2 }; }\n",
    );
    let prog = root.join("use_it.loft");
    write(
        &prog,
        "use okp;\nfn main() { s = mk(); println(\"{s.s_a}{s.s_b}\"); }\n",
    );

    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--lib")
        .arg(&root)
        .arg(&prog)
        .env("LOFT_TIMEOUT", "60")
        .current_dir(root.join("okp"))
        .output()
        .expect("spawn loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !all.contains("has no storage"),
        "a field whose type is declared before use must keep its slot.\n{all}"
    );
    assert!(all.contains("12"), "and the program must run.\n{all}");
    let _ = std::fs::remove_dir_all(&root);
}
