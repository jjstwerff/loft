// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN131 — suggestions: tell the author what to write instead.
//!
//! A diagnostic says what is wrong; a fix says what to write instead; the linked feature
//! says why. These guards pin the parts that rot silently — an opt-in that leaks, a
//! condition that cannot name its use, a concept whose door opens onto nothing, and a code
//! with no index entry to grep to.

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// A program with one avoidable copy: `src` is named into the struct and used again after,
/// so it survives the copy and could not be moved.  Line 7 is the copy; line 8 is the use.
const AVOIDABLE: &str = "struct Holder { v: vector<integer> }\n\n\
     fn use_it(h: Holder) -> integer { len(h.v) }\n\n\
     fn main() {\n  src = [1, 2, 3];\n  h = Holder { v: src };\n  \
     println(\"{use_it(h)} {len(src)}\");\n}\n";

fn probe(src: &str) -> std::path::PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join("loft_pln131");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{}_{n}.loft", std::process::id()));
    std::fs::write(&path, src).expect("write probe");
    path
}

/// Run `--check` on `src`, with `--explain` when asked; return stdout+stderr.
fn run(src: &str, explain: bool) -> String {
    let path = probe(src);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.args(["--interpret", "--check"]);
    if explain {
        cmd.arg("--explain");
    }
    let out = cmd
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The copy notice carries the code a fix attaches to.
///
/// The code is the frozen identity and the prose is free, so a suggestion must hang off the
/// former — attaching it to a message string is how a suggestion and its diagnostic drift
/// apart, and a suggestion that has drifted from its diagnostic is misinformation.
#[test]
fn the_copy_notice_carries_its_code() {
    let out = run(AVOIDABLE, false);
    assert!(
        out.contains("advice[avoidable-copy]"),
        "the copy notice must print its code; output:\n{out}"
    );
}

/// Fix lines are OPT-IN. loft is meant to be quiet: the resolutions are worth reading when
/// you are acting on a diagnostic and noise when you are not.
#[test]
fn fix_lines_are_opt_in() {
    let quiet = run(AVOIDABLE, false);
    assert!(
        quiet.contains("avoidable-copy"),
        "the diagnostic itself must still fire without --explain; output:\n{quiet}"
    );
    assert!(
        !quiet.contains("  fix  "),
        "fix lines must not appear without --explain; output:\n{quiet}"
    );
}

/// `--explain` offers BOTH tiers, most-teaching first.
///
/// Ranking is on what a fix opens up, not on how short it is: "build the value in place"
/// introduces an idiom reusable everywhere, while "drop the later use" is a local deletion.
#[test]
fn explain_offers_both_tiers_teaching_first() {
    let out = run(AVOIDABLE, true);
    let Some(build) = out.find("build the value in place") else {
        panic!("the mechanical fix must be offered; output:\n{out}");
    };
    let Some(drop) = out.find("drop the later use") else {
        panic!("the conditional fix must be offered; output:\n{out}");
    };
    assert!(
        build < drop,
        "the fix that teaches an idiom must rank above the local deletion; output:\n{out}"
    );
}

/// The condition NAMES the surviving use, by line.
///
/// This is @PLN131's Q6.1, and it is the difference between a veteran affirming a condition
/// in one second and going hunting for it: the analysis holds the last use as a traversal
/// index, so without carrying its location the honest wording is only "after here".
///
/// `src` is used again on line 8 of the probe — asserting the line, not merely the presence
/// of a condition, is what makes this fail if the location is dropped again.
#[test]
fn the_condition_names_the_surviving_use_by_line() {
    let out = run(AVOIDABLE, true);
    assert!(
        out.contains("needs:"),
        "a conditional fix must state the condition it affirms; output:\n{out}"
    );
    assert!(
        out.contains("`src` is used again at line 8"),
        "the condition must name WHERE the source survives, not just that it does; \
         output:\n{out}"
    );
}

/// Three homes, no repetition: the message says what is WRONG and nothing about the cure.
///
/// The diagnostics used to carry their own resolution inline, so `--explain` printed the same
/// advice twice — the duplication a reader pays for on every diagnostic. What makes this
/// checkable rather than a matter of taste is the fix's own words: if the message contains the
/// imperative the fix line offers, the two homes have merged again.
#[test]
fn the_message_does_not_repeat_its_own_fix() {
    let quiet = run(AVOIDABLE, false);
    for cure in ["build the value in place", "drop the later use"] {
        assert!(
            !quiet.contains(cure),
            "the message repeats its fix (\"{cure}\") — what is wrong belongs to the \
             diagnostic, what to write instead belongs to the fix; output:\n{quiet}"
        );
    }
}

/// Moving the cure out of the message must not leave a reader with LESS than before.
///
/// Fix lines are opt-in, so a plain run now says only what is wrong — and someone who has
/// never heard of `--explain` would simply be told less. One line per RUN closes that, and it
/// has to be per run: a pointer under each diagnostic would double the output on a file with
/// fifty copy notices, which is the noise the opt-in exists to avoid.
#[test]
fn a_quiet_run_says_where_the_fixes_are_exactly_once() {
    let quiet = run(AVOIDABLE, false);
    assert_eq!(
        quiet.matches("re-run with `--explain`").count(),
        1,
        "a quiet run must point at `--explain` exactly once — not per diagnostic, and not \
         never; output:\n{quiet}"
    );
    // …and it must not nag the reader who already asked.
    let explained = run(AVOIDABLE, true);
    assert!(
        !explained.contains("re-run with `--explain`"),
        "the pointer must vanish once the fixes are shown; output:\n{explained}"
    );
}

/// The concept is a handle plus a door, and the door must open onto something real.
///
/// A door onto nothing is worse than no door — so the catalogue entry the fix names has to
/// exist. Checked against the committed snapshot, which is generated from the canonical
/// issue, so deleting or renumbering the feature breaks this rather than shipping a dead
/// link to a reader.
#[test]
fn the_concept_door_resolves_to_a_real_catalogue_entry() {
    let out = run(AVOIDABLE, true);
    assert!(
        out.contains("[move · @F106]"),
        "each fix must name its concept and the entry it opens onto; output:\n{out}"
    );
    let snapshot = std::fs::read_to_string("index/features.json").expect("features snapshot");
    let found = snapshot.contains("\"number\": 106") || snapshot.contains("\"number\":106");
    assert!(
        found,
        "@F106 is the door the copy fixes open onto — it must exist in the catalogue"
    );
}
