// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN140 — the profiling instruments, checked against programs whose answer is
//! known in advance.
//!
//! A profiler is unusually easy to ship broken, because every failure mode produces a
//! *plausible* report rather than an obviously empty one: a cache hit read as a
//! compile, a build read as a run, a one-sample symbol read as a hot spot. So nothing
//! here asserts "a report appeared". Each test states the answer first — this program
//! spends its time in `hot`, this one holds its memory at line 4, this one allocates
//! down two paths at 9:1 — and fails when the instrument says something else.
//!
//! Shares are asserted with generous floors rather than exact figures: the exact split
//! moves with the machine, and pinning it would make these change-detectors instead of
//! oracles. The floors are far enough from the wrong answer (naming a different
//! function, or missing a path entirely) that no amount of noise reaches them.
//!
//! The corpus-scale version of the same idea, over `bench/`, is
//! `scripts/profile_corpus.sh` and `bench/profile_oracle.tsv`.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Run `body` under the interpreter with `envs` set, returning stdout+stderr.
fn run(name: &str, body: &str, envs: &[(&str, &str)]) -> String {
    let path = std::env::temp_dir().join(format!("loft_prof_{name}.loft"));
    std::fs::write(&path, body).expect("write probe");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret").arg(&path);
    cmd.env("LOFT_TIMEOUT", "120");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The first row of a `── <section> ──` block, which is the instrument's answer.
fn top_row<'a>(out: &'a str, section: &str) -> &'a str {
    out.lines()
        .skip_while(|l| !l.starts_with(section))
        .nth(1)
        .unwrap_or("")
}

/// `hot` runs ~100× the iterations of `cold`, so its share is not a matter of opinion.
const TWO_FUNCTIONS: &str = r#"
fn hot(n: integer) -> integer {
  acc = 0;
  for h_i in 0..n { acc += h_i % 7; }
  acc
}
fn cold(n: integer) -> integer {
  acc = 0;
  for c_i in 0..n { acc += c_i % 7; }
  acc
}
fn main() {
  t = hot(2000000) + cold(20000);
  println("result: {t}");
}
"#;

#[test]
fn cpu_profile_names_the_function_that_is_actually_hot() {
    let out = run("cpu_hot", TWO_FUNCTIONS, &[("LOFT_PROFILE", "1")]);
    let row = top_row(&out, "── by function");
    assert!(
        row.contains("hot") && !row.contains("cold"),
        "the hottest function is `hot` by two orders of magnitude.\nGot: {row}\n{out}"
    );
    // 100:1 in iterations; anything under half is the profiler measuring something
    // other than the work.
    let share: f64 = row
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    assert!(share > 50.0, "`hot` should dominate, got {share} %\n{out}");
}

#[test]
fn cpu_profile_names_the_hot_line_inside_it() {
    let out = run("cpu_line", TWO_FUNCTIONS, &[("LOFT_PROFILE", "1")]);
    let row = top_row(&out, "── by line");
    // The literal opens with a newline, so `fn hot` is line 2 and its loop body —
    // `for h_i in 0..n { acc += h_i % 7; }`, the only line executed two million
    // times — is line 4.
    assert!(
        row.contains(":4") && row.contains("hot"),
        "the hot line is the loop body of `hot` (line 4).\nGot: {row}\n{out}"
    );
}

/// A profiler that cannot name `main` names nothing: four out of five real programs
/// would also pass an instrument that simply reported the deepest frame.
#[test]
fn cpu_profile_names_main_when_main_is_the_work() {
    let out = run(
        "cpu_main",
        r#"
fn main() {
  acc = 0;
  for m_i in 0..2000000 { acc += m_i % 7; }
  println("result: {acc}");
}
"#,
        &[("LOFT_PROFILE", "1")],
    );
    let row = top_row(&out, "── by function");
    assert!(
        row.contains("main"),
        "with no helper, the whole run is `main`.\nGot: {row}\n{out}"
    );
}

/// The one thing a flat hot-spot table cannot fake.
#[test]
fn cpu_profile_shows_recursion_in_the_path() {
    let out = run(
        "cpu_path",
        r#"
fn fib(n: integer) -> integer {
  if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
}
fn main() { println("result: {fib(24)}"); }
"#,
        &[("LOFT_PROFILE", "1")],
    );
    let row = top_row(&out, "── hottest paths");
    assert!(
        row.matches("fib").count() >= 2,
        "`fib` is reached from `fib`; a path showing it only from `main` is not a path.\
         \nGot: {row}\n{out}"
    );
}

/// Off by default, and silent — a profile nobody asked for on stderr is a regression
/// in the tool, not a feature.
#[test]
fn nothing_is_reported_unless_it_was_asked_for() {
    let out = run("silent", TWO_FUNCTIONS, &[]);
    assert!(
        !out.contains("CPU profile") && !out.contains("allocation"),
        "an unarmed run must say nothing about profiling:\n{out}"
    );
}

/// The defect arc A exists to fix: the memory is held DURING the run and released
/// before exit, so an exit-triggered report sees an empty heap and says nothing.
#[test]
fn memory_report_is_taken_at_the_peak_not_at_exit() {
    let out = run(
        "mem_peak",
        r#"
fn build(n: integer) -> vector<integer> {
  return [for b_i in 0..n { b_i * 3 }];
}
fn main() {
  v = build(400000);
  println("result: {len(v)}");
}
"#,
        &[("LOFT_ALLOC_SITES", "1")],
    );
    assert!(
        out.contains("allocation hot spots"),
        "the report must appear:\n{out}"
    );
    let row = top_row(&out, "════ allocation hot spots");
    assert!(
        row.contains("build") && row.contains(":3"),
        "the heap was taken by `build` at line 3, and it is free again by exit — an \
         exit-time report would have nothing to say.\nGot: {row}\n{out}"
    );
    assert!(
        row.contains("MiB"),
        "400 000 integers is megabytes, and bytes are what the report is about \
         (store COUNTS would weigh this the same as one small record).\nGot: {row}"
    );
}

/// arc C — the case no `bench/` program can falsify: one allocation site, two paths,
/// a known ratio. A fixed-period sampler reported 100 % / 0 % here.
#[test]
fn allocation_paths_report_both_paths_at_their_true_ratio() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bench/profile_oracle/alloc_paths.loft");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&root)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_ALLOC_PATHS", "1")
        .output()
        .map(|o| {
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        })
        .expect("spawn loft");
    let hot = out.contains("main → hot → make");
    let cold = out.contains("main → cold → make");
    assert!(
        hot && cold,
        "both paths to `make` must appear (hot: {hot}, cold: {cold}) — one path alone \
         is what a sampler in lock-step with the program reports.\n{out}"
    );
    // The rare path is 1 in 10; a sampler locked to the program's period reports it as
    // 0, and one that lost the site reports 50.
    let cold_n: u64 = out
        .lines()
        .zip(out.lines().skip(1))
        .find(|(_, next)| next.contains("main → cold → make"))
        .and_then(|(row, _)| {
            row.split_whitespace()
                .nth(2)
                .and_then(|s| s.parse::<u64>().ok())
        })
        .unwrap_or(0);
    let hot_n: u64 = out
        .lines()
        .zip(out.lines().skip(1))
        .find(|(_, next)| next.contains("main → hot → make"))
        .and_then(|(row, _)| {
            row.split_whitespace()
                .nth(2)
                .and_then(|s| s.parse::<u64>().ok())
        })
        .unwrap_or(0);
    assert!(cold_n > 0 && hot_n > 0, "both paths need samples\n{out}");
    let ratio = hot_n as f64 / cold_n as f64;
    assert!(
        (4.0..20.0).contains(&ratio),
        "the true ratio is 9:1; got {ratio:.1}:1 ({hot_n} / {cold_n})\n{out}"
    );
}

// ── loft#860: a test run is a loft workload, and was the one the sampler could not see ──

/// Two test files whose hot loops are BOTH on line 4, and whose iteration counts are
/// 4:1. The line collision is the point — see
/// [`merged_profile_keeps_each_row_s_own_file`].
const TEST_FILE_A: &str = r#"
fn work_a(n: integer) -> integer {
  acc = 0;
  for a_i in 0..n { acc += a_i % 7; }
  acc
}
fn test_a() { assert(work_a(800000) > 0); }
"#;

const TEST_FILE_B: &str = r#"
fn work_b(n: integer) -> integer {
  acc = 0;
  for b_i in 0..n { acc += b_i % 7; }
  acc
}
fn test_b() { assert(work_b(200000) > 0); }
"#;

/// Write the two-file suite into a fresh directory and run `--tests` over it.
fn run_tests_dir(name: &str, envs: &[(&str, &str)], extra_args: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!("loft_prof_tests_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create suite dir");
    std::fs::write(dir.join("a_file.loft"), TEST_FILE_A).expect("write a");
    std::fs::write(dir.join("b_file.loft"), TEST_FILE_B).expect("write b");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--tests");
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.arg(&dir);
    cmd.env("LOFT_TIMEOUT", "120");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let _ = std::fs::remove_dir_all(&dir);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// loft#860 — the report existed, the sampler worked, and a test run got neither:
/// `arm_profiler` had exactly one call site, on the program path. A suite is usually
/// the biggest interpreted workload a project owns, so this was the workload most
/// worth profiling and the only one that could not be.
#[test]
fn a_test_run_is_profiled() {
    let out = run_tests_dir("armed", &[("LOFT_PROFILE", "1")], &[]);
    assert!(
        out.contains("loft CPU profile"),
        "a test run must be profiled when LOFT_PROFILE asks for it — accepting the \
         variable and reporting nothing reads as 'the profiler found nothing'.\n{out}"
    );
    let row = top_row(&out, "── by function");
    assert!(
        row.contains("work_a"),
        "`work_a` runs 4× the iterations of `work_b`, so it is the answer.\nGot: {row}\n{out}"
    );
}

/// The merge has to happen on RESOLVED labels, not on bytecode positions: each test
/// compiles its own `Data`, so the same `pc` names different code in each one. Both
/// files here are hot on line 3, which is exactly the collision a `pc`-keyed merge
/// would fold into one row — and it would look like an ordinary profile.
#[test]
fn merged_profile_keeps_each_row_s_own_file() {
    let out = run_tests_dir("merged", &[("LOFT_PROFILE", "1")], &[]);
    assert!(
        out.contains("a_file.loft:4") && out.contains("b_file.loft:4"),
        "both files are hot at line 4 and must stay separate rows; merging raw \
         positions collapses them into one.\n{out}"
    );
    assert!(
        out.contains("across 2 runs"),
        "the banner must say how many runs it covers — 2 s over one run and 2 s over \
         420 invite different readings of the percentages.\n{out}"
    );
    // `work_a` runs 4× the iterations of `work_b`, so it must rank above it and
    // `work_b` must keep a real share. Only the ORDER and a floor are asserted, not
    // the split: the run that goes first also pays the run-once costs (page faults,
    // first touch of the stdlib), so the measured ratio sits above the iteration
    // ratio — around 7:1 here — and pinning it would make this a change-detector.
    // The floor is what matters: a dropped run reads as a function that went quiet.
    let share = |name: &str| -> f64 {
        out.lines()
            .find(|l| l.contains(name) && l.contains('%'))
            .and_then(|l| l.split_whitespace().next().and_then(|s| s.parse().ok()))
            .unwrap_or(0.0)
    };
    let (a, b) = (share("work_a"), share("work_b"));
    assert!(
        a > b && b > 2.0,
        "both runs must survive the merge (work_a {a} %, work_b {b} %) — a lost run \
         reads as a quiet function.\n{out}"
    );
}

/// Still off by default. The suite runs 3931 tests; a profile on stderr for every
/// unprofiled one would be the loudest regression in the tool.
#[test]
fn a_test_run_is_silent_unless_asked() {
    let out = run_tests_dir("quiet", &[], &[]);
    assert!(
        !out.contains("CPU profile") && !out.contains("allocation paths"),
        "an unarmed test run must say nothing about profiling:\n{out}"
    );
}

/// The heap ledger ranks a process-wide peak by bytecode position, and a suite's peak
/// may be reached in any of its runs — each of which compiled its own bytecode. So it
/// REFUSES rather than resolving those positions against whichever run finished last,
/// which would name real lines in the wrong file.
#[test]
fn alloc_sites_refuses_a_test_run_instead_of_going_quiet() {
    let out = run_tests_dir("sites", &[("LOFT_ALLOC_SITES", "1")], &[]);
    assert!(
        out.contains("LOFT_ALLOC_SITES is not available under a test run"),
        "an instrument that cannot answer must say so — silence reads as 'nothing to \
         report'.\n{out}"
    );
    assert!(
        !out.contains("allocation hot spots"),
        "…and it must not print a table it cannot attribute:\n{out}"
    );
}
