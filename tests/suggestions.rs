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

// ── steps 3–4: verifying a fix, and applying it ──────────────────────────────

/// A program whose one fix is mechanical, spelled, and placeable: a literal `}` in a
/// format string. Line 2 is the offence.
const BRACE: &str = "fn main() {\n  println(\"a } b\");\n}\n";

/// Run `loft fix` (report) or `loft fix --apply` on a file, returning stdout+stderr.
fn fix_cmd(path: &std::path::Path, apply: bool) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("fix");
    if apply {
        cmd.arg("--apply");
    }
    let out = cmd
        .arg(path)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft fix");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Step 3 — a fix is CHECKED by running it, not by looking plausible.
///
/// The compiler holds the analysis that raised the diagnostic, so a candidate rewrite can
/// be applied to an in-memory copy and the analysis re-run. That is what separates a
/// suggestion that has been tried from one that was pattern-matched, and it is the whole
/// reason `--apply` can be trusted to run unattended.
#[test]
fn a_fix_is_verified_by_running_it() {
    let path = probe(BRACE);
    let out = fix_cmd(&path, false);
    assert!(
        out.contains("double the brace") && out.contains("[verified]"),
        "a mechanical fix that clears its diagnostic must report as verified; output:\n{out}"
    );
    let _ = std::fs::remove_file(&path);
}

/// Reporting must not write. `loft fix` without `--apply` is a read-only question.
#[test]
fn reporting_a_fix_does_not_touch_the_file() {
    let path = probe(BRACE);
    let before = std::fs::read_to_string(&path).expect("probe");
    let out = fix_cmd(&path, false);
    let after = std::fs::read_to_string(&path).expect("probe");
    assert_eq!(
        before, after,
        "`loft fix` without `--apply` must change nothing; output:\n{out}"
    );
    assert!(
        !out.contains("[applied]"),
        "a report that claims an edit it did not make is the one output a reader cannot \
         check; output:\n{out}"
    );
    let _ = std::fs::remove_file(&path);
}

/// Step 4 — `--apply` writes the fix, and the result actually compiles and runs.
///
/// Asserting the file merely CHANGED would pass for a rewrite that broke the program, which
/// is the failure this feature exists to avoid. So the applied program is run, and its
/// output is the one the author was trying to write: a literal `}`.
#[test]
fn applying_a_fix_produces_a_program_that_runs() {
    let path = probe(BRACE);
    let out = fix_cmd(&path, true);
    assert!(
        out.contains("[applied]"),
        "the mechanical fix must be written; output:\n{out}"
    );
    let src = std::fs::read_to_string(&path).expect("probe");
    assert!(
        src.contains("a }} b"),
        "the brace must be doubled in place; got:\n{src}"
    );
    let run = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret"])
        .arg(&path)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run the fixed program");
    assert!(
        run.status.success(),
        "the applied fix must leave a program that compiles: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("a } b"),
        "the fixed program must print the literal brace the author wanted: {}",
        String::from_utf8_lossy(&run.stdout)
    );
    let _ = std::fs::remove_file(&path);
}

/// Applying twice is a no-op — the second run has nothing left to fix.
///
/// An applier that re-applies its own output doubles the brace again on every run, which is
/// the classic way a quick-fix corrupts a file nobody was watching.
#[test]
fn applying_twice_changes_nothing_the_second_time() {
    let path = probe(BRACE);
    fix_cmd(&path, true);
    let once = std::fs::read_to_string(&path).expect("probe");
    fix_cmd(&path, true);
    let twice = std::fs::read_to_string(&path).expect("probe");
    assert_eq!(once, twice, "a second `--apply` must find nothing to do");
    let _ = std::fs::remove_file(&path);
}

/// A rewrite that would introduce a NEW error is refused, not written.
///
/// `x: integer = "5" as integer` is offered the checked cast like any other failing parse,
/// but `as integer?` in that slot yields `integer?` into a non-null `integer` — the fix
/// does not survive its own verification. This is step 3 paying for itself: the suggestion
/// is plausible, the measurement is what says no.
#[test]
fn a_fix_that_would_break_the_program_is_refused() {
    let path = probe("fn main() { x: integer = \"5\" as integer; println(\"{x}\"); }\n");
    let before = std::fs::read_to_string(&path).expect("probe");
    let out = fix_cmd(&path, true);
    assert!(
        out.contains("REJECTED"),
        "a rewrite that introduces an error must be refused; output:\n{out}"
    );
    assert_eq!(
        before,
        std::fs::read_to_string(&path).expect("probe"),
        "a refused fix must not reach the file; output:\n{out}"
    );
    let _ = std::fs::remove_file(&path);
}

/// Nothing to fix, nothing to say — loft is BORING when there is no work.
#[test]
fn a_clean_file_reports_nothing() {
    let path = probe("fn main() { println(\"ok\"); }\n");
    let out = fix_cmd(&path, false);
    assert!(
        out.trim().is_empty(),
        "a file with no fixes must print nothing; output:\n{out}"
    );
    let _ = std::fs::remove_file(&path);
}

/// A fix that does not hold in every shape must not lead — soundness outranks teaching.
///
/// `as τ?` is the better idiom and it makes the expression NULLABLE, which a target declared
/// non-null rejects. The parser cannot see that target: applying the fix changes what pass 1
/// infers, so only a re-parse knows. It therefore ships as a CONDITION the author can check
/// in a second, ranked below the two discharging forms that hold wherever this diagnostic
/// fires — "prefer the fix that teaches" is a tiebreak between EQUALLY SOUND fixes, never a
/// licence to lead with one that works three times in four.
#[test]
fn a_fix_that_does_not_always_hold_ranks_below_ones_that_do() {
    let out = run(
        "fn main() { x: integer = \"5\" as integer; println(\"{x}\"); }\n",
        true,
    );
    let Some(fallback) = out.find("give the parse a fallback") else {
        panic!("the always-sound discharge must be offered; output:\n{out}");
    };
    let Some(checked) = out.find("make the cast checked") else {
        panic!("the checked cast must still be offered; output:\n{out}");
    };
    assert!(
        fallback < checked,
        "a fix that holds in every shape must rank above one that does not; output:\n{out}"
    );
    assert!(
        out.contains("declared non-null"),
        "the checked cast must state the condition its soundness rests on, so the author \
         can check their own declaration; output:\n{out}"
    );
}

/// A conditional fix WITH an edit: verified, reported, and never written unattended.
///
/// This is the shape the tier split exists for, and until the cast fixes were re-tiered
/// nothing shipped it — the rule was pinned only by a unit test. `loft fix` must say the
/// rewrite works AND that the judgement stays with the author: "you must decide" and "it
/// would not have compiled anyway" are different answers.
#[test]
fn a_conditional_fix_is_verified_but_left_to_the_author() {
    let path = probe("fn main() { x = \"5\" as integer; println(\"{x}\"); }\n");
    let before = std::fs::read_to_string(&path).expect("probe");
    let out = fix_cmd(&path, true);
    assert!(
        out.contains("make the cast checked") && out.contains("yours to accept"),
        "a conditional fix that verifies must say so, and say it is still yours; \
         output:\n{out}"
    );
    assert_eq!(
        before,
        std::fs::read_to_string(&path).expect("probe"),
        "`--apply` must not write a conditional fix, however well it verifies; output:\n{out}"
    );
    let _ = std::fs::remove_file(&path);
}
