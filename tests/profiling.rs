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
