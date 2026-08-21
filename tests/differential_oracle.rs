// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN89 — the differential oracle (formal/operational.md deviations D-op-1/2).
//!
//! The interpreter (`src/state/`) and the native generator (`src/generation/`)
//! are two separate implementations of the same language, kept in agreement only
//! by tests. A program the interpreter runs fine but `--native` miscompiles (or
//! leaks, or halts differently) ships until some test happens to exercise it —
//! a coverage lottery (#433 was the canonical case; this session's overflow-log
//! attempt was another: it broke `--native` with E0499 while the interpreter was
//! perfectly happy).
//!
//! This oracle turns that class into a CAUGHT failure: every program in
//! `tests/oracle/` is run on BOTH backends and their observable outcome must
//! AGREE — normalised stdout (value / null), exit code (halt), stderr (what the program
//! SAID: warnings and the diagnostic a fault renders), and leak-freedom.
//! The operational.md rules guide what the corpus should cover; the corpus
//! GROWS over time, and every fixed divergence graduates a guard program here.
//!
//! The corpus sweep is `#[ignore]` by default: each program shells out to the
//! `loft` binary twice and `--native` invokes `rustc` per program, too heavy for
//! the default `cargo test` path. Run it explicitly:
//!
//! ```bash
//! cargo test --release --test differential_oracle -- --ignored
//! ```
//!
//! The `positive_control_*` test (NOT ignored) proves the comparison can
//! actually fail — a green sweep is only evidence once the detector is known to
//! fire (engineering-rigor: a silent sentinel needs a positive control).

use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Captured outcome of one backend run.
struct ModeRun {
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
}

/// Run one corpus program under `mode_flag` (`--interpret` / `--native`).
/// `LOFT_TIMEOUT` bounds the native `rustc` compile so a runaway can't hang the
/// suite (the native path can otherwise block indefinitely).
fn run_mode(mode_flag: &str, path: &Path, env: &[(&str, &str)]) -> ModeRun {
    let mut cmd = Command::new(loft_bin());
    cmd.arg(mode_flag)
        .arg(path)
        .current_dir(workspace_root())
        .env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn loft in {mode_flag} mode: {e}"));
    ModeRun {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit_code: out.status.code(),
    }
}

/// Normalise stdout for cross-backend comparison: CRLF→LF, trailing
/// whitespace per line, and a collapsed trailing-blank-line tail. (Mirrors
/// `tests/common/cross_mode.rs::normalise_stdout` — the native binary may emit
/// CRLF on Windows.)
fn normalise_stdout(s: &str) -> String {
    let lf = s.replace("\r\n", "\n");
    let mut lines: Vec<&str> = lf.lines().map(str::trim_end).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Normalise stderr for cross-backend comparison: CRLF→LF, trailing blank lines
/// dropped, and the leak line removed — leaks are their own channel below, and the
/// native binary only prints one when `LOFT_NATIVE_LEAK_CHECK` is set, so comparing
/// it here would report the switch rather than the program.
fn normalise_stderr(s: &str) -> String {
    s.replace("\r\n", "\n")
        .lines()
        .filter(|l| !l.contains("stores not freed"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

/// Both backends report a store leak identically — a `… stores not freed at
/// program exit …` line on stderr. The interpreter prints it unconditionally;
/// the native binary only when `LOFT_NATIVE_LEAK_CHECK` is set (which the sweep
/// sets for the native run).
fn leaked(run: &ModeRun) -> bool {
    run.stderr.contains("stores not freed")
}

/// The oracle's invariant, as a pure function so it can be unit-tested: the
/// interpreter and native runs must AGREE on stdout (value / null) and exit
/// code (halt), and NEITHER may leak. Returns the list of divergences for one
/// program — empty means the two backends agree.
fn divergences(interp: &ModeRun, native: &ModeRun) -> Vec<String> {
    let mut d = Vec::new();
    let i = normalise_stdout(&interp.stdout);
    let n = normalise_stdout(&native.stdout);
    if i != n {
        d.push(format!(
            "stdout differs:\n    interp = {i:?}\n    native = {n:?}"
        ));
    }
    if interp.exit_code != native.exit_code {
        d.push(format!(
            "exit code differs: interp={:?} native={:?}",
            interp.exit_code, native.exit_code
        ));
    }
    // What the program SAID about itself — warnings and the diagnostic a fault renders.
    // Captured since the oracle was built and never once compared, which is how a failed
    // `assert` came to print a loft diagnostic on one backend and a Rust panic naming a
    // generated temp file on the other, for as long as both existed (loft#1056).  Seven
    // of the corpus programs write to this channel, so it is exercised rather than
    // agreeing by having nothing in it.
    let ie = normalise_stderr(&interp.stderr);
    let ne = normalise_stderr(&native.stderr);
    if ie != ne {
        d.push(format!(
            "stderr differs:\n    interp = {ie:?}\n    native = {ne:?}"
        ));
    }
    if leaked(interp) {
        d.push(format!(
            "interpreter leaked a store:\n    {}",
            interp.stderr.trim()
        ));
    }
    if leaked(native) {
        d.push(format!(
            "native leaked a store:\n    {}",
            native.stderr.trim()
        ));
    }
    d
}

/// A driver STATICALLY REJECTED the program — it failed parse / type / compile
/// BEFORE running, as opposed to running and hitting a runtime fault. A static
/// reject carries a diagnostic (a loft `error:` line, or a rustc `error[E…]` /
/// "could not compile") AND never produced program output; the empty-stdout guard
/// is what keeps a runtime panic (partial stdout + "thread … panicked") from
/// reading as a static reject.
fn statically_rejected(run: &ModeRun) -> bool {
    run.exit_code != Some(0)
        && normalise_stdout(&run.stdout).is_empty()
        && (run.stderr.contains("error[E")
            || run.stderr.contains("could not compile")
            || run
                .stderr
                .lines()
                .any(|l| l.trim_start().starts_with("error:")))
}

/// Every driver must agree on accept-vs-reject: well-typedness is ONE static
/// judgment, so `--dump` (the pure parse+typecheck, no run), `--interpret`, and
/// `--native` (which includes the rustc compile) must reach the SAME verdict. The
/// #433 class is exactly this property failing — `--interpret` accepts a program
/// `--native` rejects at rustc (E0308). Returns the disagreements for one program.
fn driver_agreement(dump: &ModeRun, interp: &ModeRun, native: &ModeRun) -> Vec<String> {
    let (rd, ri, rn) = (
        statically_rejected(dump),
        statically_rejected(interp),
        statically_rejected(native),
    );
    if rd == ri && ri == rn {
        Vec::new()
    } else {
        vec![format!(
            "accept/reject disagrees (well-typedness is one judgment): \
             --dump rejected={rd}, --interpret rejected={ri}, --native rejected={rn}"
        )]
    }
}

/// Whether the headless-WASM toolchain is present: the `wasm32-wasip2` rustup
/// target AND `wasmtime` on PATH. When absent the sweep SKIPS the wasm leg (rather
/// than failing), so a machine without the toolchain still runs the interp/native
/// oracle; a wasm-capable runner (the nightly gate) exercises it. This leans on the
/// @PLN100 build phase, which auto-builds the wasip2 loft-runtime rlib on first use.
fn wasm_toolchain_present() -> bool {
    let target = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("wasm32-wasip2"))
        .unwrap_or(false);
    let wasmtime = Command::new("wasmtime")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    target && wasmtime
}

/// The THIRD backend: compile `path` to `wasm32-wasip2` (`--native-wasm`) and run
/// it under `wasmtime`, capturing the program's observable outcome. WASM shares the
/// native Rust generator, so this pins interp == native == WASM by construction. A
/// COMPILE failure (a codegen shape that breaks wasm — the compound-`&&` /
/// format-hook / text-if class this cycle broke BOTH native and wasm) is returned
/// as the compile outcome, which reads as a static reject and is caught by the
/// accept/reject check against the accepting interpreter; a successful compile then
/// runs and its stdout / exit must match the interpreter. Returns `None` when the
/// toolchain is absent (the leg is skipped, not failed). (Leak-freedom stays an
/// interp/native check — the native leak sentinel is not wired through wasmtime.)
fn run_wasm(path: &Path) -> Option<ModeRun> {
    if !wasm_toolchain_present() {
        return None;
    }
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let out = std::env::temp_dir().join(format!("loft_oracle_{stem}_{}.wasm", std::process::id()));
    let compile = Command::new(loft_bin())
        .arg("--native-wasm")
        .arg(&out)
        .arg(path)
        .current_dir(workspace_root())
        .env("LOFT_TIMEOUT", "180")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn loft --native-wasm: {e}"));
    if compile.status.code() != Some(0) || !out.exists() {
        return Some(ModeRun {
            stdout: String::from_utf8_lossy(&compile.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&compile.stderr).into_owned(),
            exit_code: compile.status.code(),
        });
    }
    let run = Command::new("wasmtime")
        .arg(&out)
        .output()
        .unwrap_or_else(|e| panic!("failed to run wasmtime: {e}"));
    let _ = std::fs::remove_file(&out);
    Some(ModeRun {
        stdout: String::from_utf8_lossy(&run.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&run.stderr).into_owned(),
        exit_code: run.status.code(),
    })
}

/// Interp-vs-WASM outcome agreement — stdout (value / null) + exit code (halt),
/// labelled so a failure names the wasm leg.
fn wasm_divergences(interp: &ModeRun, wasm: &ModeRun) -> Vec<String> {
    let mut d = Vec::new();
    let i = normalise_stdout(&interp.stdout);
    let w = normalise_stdout(&wasm.stdout);
    if i != w {
        d.push(format!(
            "WASM stdout differs:\n    interp = {i:?}\n    wasm   = {w:?}"
        ));
    }
    if interp.exit_code != wasm.exit_code {
        d.push(format!(
            "WASM exit code differs: interp={:?} wasm={:?}",
            interp.exit_code, wasm.exit_code
        ));
    }
    d
}

/// The corpus: every `.loft` under `tests/oracle/`, in alphabetical order.
fn corpus() -> Vec<PathBuf> {
    let dir = workspace_root().join("tests/oracle");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "loft"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "oracle corpus is empty: {}",
        dir.display()
    );
    files
}

/// Run the whole corpus on both backends; a divergence on ANY program is a
/// caught cross-backend bug. Heavy (rustc per program) — `#[ignore]` by default.
#[test]
#[ignore = "differential oracle — run with --test differential_oracle -- --ignored"]
fn oracle_corpus_agrees_across_backends() {
    let mut report = Vec::new();
    for path in corpus() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let dump = run_mode("--dump", &path, &[]);
        let interp = run_mode("--interpret", &path, &[]);
        let native = run_mode("--native", &path, &[("LOFT_NATIVE_LEAK_CHECK", "1")]);
        let mut d = divergences(&interp, &native);
        d.extend(driver_agreement(&dump, &interp, &native));
        // THIRD backend: headless WASM (wasm32-wasip2 / wasmtime), when the toolchain is present.
        // wasm shares the native Rust generator, so a shape that compiles native but breaks wasm —
        // OR breaks BOTH (the compound-&& / format-hook / text-if class this cycle) — is caught here.
        if let Some(wasm) = run_wasm(&path) {
            d.extend(wasm_divergences(&interp, &wasm));
            // accept/reject must agree with the interpreter: a wasm-compile-fail on a program the
            // interpreter accepts is exactly the #520/#533/#534 class.
            let (ri, rw) = (statically_rejected(&interp), statically_rejected(&wasm));
            if ri != rw {
                d.push(format!(
                    "WASM accept/reject disagrees: --interpret rejected={ri}, --native-wasm rejected={rw}"
                ));
            }
        }
        if !d.is_empty() {
            report.push(format!("✗ {name}\n  {}", d.join("\n  ")));
        }
    }
    assert!(
        report.is_empty(),
        "differential oracle caught {} cross-backend divergence(s):\n\n{}",
        report.len(),
        report.join("\n\n"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(stdout: &str, stderr: &str, code: Option<i32>) -> ModeRun {
        ModeRun {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code: code,
        }
    }

    /// Positive control — the detector must FIRE on each divergence kind, and
    /// must NOT fire when the two runs agree. Without this, a green corpus sweep
    /// proves nothing (the detector could be silently dead).
    #[test]
    fn positive_control_divergences_are_detected() {
        let base = run("x=1\n", "", Some(0));

        // agreement → no divergence (no false positive)
        assert!(
            divergences(&base, &run("x=1\n", "", Some(0))).is_empty(),
            "identical runs must agree"
        );

        // stdout divergence (the silent-corruption class)
        assert!(
            !divergences(&base, &run("x=2\n", "", Some(0))).is_empty(),
            "a stdout difference must be caught"
        );

        // exit-code divergence (the halt class — #433: interp ran, native E0308)
        assert!(
            !divergences(&base, &run("x=1\n", "", Some(1))).is_empty(),
            "an exit-code difference must be caught"
        );

        // stderr divergence (the diagnostic class — loft#1056: the same fault rendered
        // as a loft diagnostic on one backend and a Rust panic on the other)
        assert!(
            !divergences(
                &base,
                &run("x=1\n", "thread 'main' panicked at /tmp/x.rs", Some(0))
            )
            .is_empty(),
            "a stderr difference must be caught"
        );

        // …but the leak line is NOT that difference: it has its own channel, and the
        // native binary prints it only under `LOFT_NATIVE_LEAK_CHECK`.  Without this the
        // stderr channel would double-report every leak and read as flaky.
        assert_eq!(
            divergences(
                &run("x=1\n", "Warning: 1 stores not freed", Some(0)),
                &run("x=1\n", "", Some(0)),
            )
            .len(),
            1,
            "a leak is one divergence, not also a stderr difference"
        );

        // a native store leak (the @PLN85 ownership class)
        assert!(
            !divergences(
                &base,
                &run(
                    "x=1\n",
                    "Warning: 1 stores not freed at program exit",
                    Some(0)
                )
            )
            .is_empty(),
            "a native store leak must be caught"
        );

        // an interpreter leak too
        assert!(
            !divergences(&run("x=1\n", "Warning: 1 stores not freed", Some(0)), &base).is_empty(),
            "an interpreter store leak must be caught"
        );
    }

    /// Driver-agreement positive control — the accept/reject verdict must AGREE
    /// across drivers, a runtime fault must NOT read as a static reject, and the
    /// #433 class (one backend rejects what the others accept) must be caught.
    #[test]
    fn positive_control_driver_disagreement_is_detected() {
        let accepted = run("out\n", "", Some(0));
        let rejected = run(
            "",
            "error: No matching operator '+' on 'integer' and 'text'",
            Some(1),
        );
        let rustc_reject = run(
            "",
            "error[E0308]: mismatched types\nerror: could not compile",
            Some(1),
        );
        // a runtime fault: the program RAN (partial stdout) then panicked.
        let runtime_fault = run(
            "partial\n",
            "thread 'main' panicked at src/…: assert",
            Some(101),
        );

        // a runtime fault is a RUNTIME outcome, never a static reject
        assert!(
            !statically_rejected(&runtime_fault),
            "a runtime panic (partial stdout + panic) must not read as a static reject"
        );
        assert!(
            statically_rejected(&rejected),
            "a loft type-error reject must be detected"
        );
        assert!(
            statically_rejected(&rustc_reject),
            "a rustc compile reject (#433) must be detected"
        );

        // all-accept and all-reject AGREE (no false positive)
        assert!(driver_agreement(&accepted, &accepted, &accepted).is_empty());
        assert!(driver_agreement(&rejected, &rejected, &rejected).is_empty());
        // #433: --dump + --interpret accept, --native rejects at rustc → CAUGHT
        assert!(
            !driver_agreement(&accepted, &accepted, &rustc_reject).is_empty(),
            "native rejecting a program the others accept must be caught"
        );
        // a runtime fault on one backend is NOT an accept/reject disagreement
        // (it is a runtime divergence, caught by `divergences`, not here)
        assert!(
            driver_agreement(&accepted, &runtime_fault, &accepted).is_empty(),
            "a runtime fault must not be reported as an accept/reject disagreement"
        );
    }

    /// stdout normalisation must not mask a real value difference, but must
    /// absorb trailing-newline / CRLF noise (else every program "diverges").
    #[test]
    fn normalisation_absorbs_noise_not_signal() {
        assert_eq!(
            normalise_stdout("a\nb\n"),
            normalise_stdout("a\r\nb\r\n\n\n")
        );
        assert_ne!(normalise_stdout("a=1\n"), normalise_stdout("a=2\n"));
    }
}
