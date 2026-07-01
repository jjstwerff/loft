// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! Platform-specific helpers shared across the crate.

use std::sync::OnceLock;

/// Process-scoped native-compile timing — a gated singleton.  The expensive
/// native work (cdylib `cargo build`s, per-fixture `rustc`) runs across
/// SPAWNED loft processes, so a threaded object can't span the boundary: each
/// process owns this singleton, subsystems record into it, and every event
/// also emits a `[loft-timing]` stderr line so it survives into the parent /
/// CI log (`ci-probe` aggregates the lines into a per-package breakdown).
///
/// Disabled unless `LOFT_TIMING` is set — [`Timing::global`] returns `None`,
/// so recording is one cached env read and a no-op.
pub struct Timing {
    /// `(kind, name, cache_hit, compile_seconds)`.  Accumulated for an
    /// in-process summary; the per-event stderr line is the cross-process channel.
    events: std::sync::Mutex<Vec<(&'static str, String, bool, Option<f64>)>>,
}

static TIMING: OnceLock<Option<Timing>> = OnceLock::new();

impl Timing {
    /// The process singleton, or `None` when `LOFT_TIMING` is unset.
    #[must_use]
    pub fn global() -> Option<&'static Timing> {
        TIMING
            .get_or_init(|| {
                let on = std::env::var("LOFT_TIMING").is_ok()
                    || std::env::var("LOFT_TIMING_LEDGER").is_ok();
                on.then(|| Timing {
                    events: std::sync::Mutex::new(Vec::new()),
                })
            })
            .as_ref()
    }

    /// Record a native-compile event.  `kind` is `"cdylib"` or `"fixture"`
    /// (compiles), or `"lockwait"` / `"lockheld"` (the global build-lock — a
    /// `lockwait` with no matching `lockheld` is a process stuck behind
    /// another's build); `secs` is the wall-clock on a cache MISS / the lock
    /// wait duration (`None` on a hit or a lock-wait START).
    ///
    /// Two channels, because the expensive native work runs in SPAWNED loft
    /// processes whose stderr a test harness CAPTURES (visible only on
    /// failure):
    /// - a `[loft-timing]` stderr line (for a direct `loft` invocation), and
    /// - when `LOFT_TIMING_LEDGER=<dir>` is set, an append to
    ///   `<dir>/timing-<pid>.tsv` — a file survives the capture, so the parent
    ///   / CI reads it back (the same side-channel shape as the skip ledger).
    pub fn record(&self, kind: &'static str, name: &str, hit: bool, secs: Option<f64>) {
        let cache = if hit { "hit" } else { "miss" };
        match secs {
            Some(s) => eprintln!("[loft-timing] {kind} {name} cache={cache} secs={s:.2}"),
            None => eprintln!("[loft-timing] {kind} {name} cache={cache}"),
        }
        if let Ok(dir) = std::env::var("LOFT_TIMING_LEDGER")
            && std::fs::create_dir_all(&dir).is_ok()
        {
            use std::io::Write;
            let path =
                std::path::Path::new(&dir).join(format!("timing-{}.tsv", std::process::id()));
            let secs_s = secs.map_or_else(String::new, |s| format!("{s:.2}"));
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "{kind}\t{name}\t{cache}\t{secs_s}");
            }
        }
        if let Ok(mut e) = self.events.lock() {
            e.push((kind, name.to_string(), hit, secs));
        }
    }
}

/// Record a native-compile event into the process [`Timing`] singleton — a
/// no-op when `LOFT_TIMING` is unset.  The one call subsystems use, so they
/// don't each repeat the `global()` gate.
pub fn timing_record(kind: &'static str, name: &str, hit: bool, secs: Option<f64>) {
    if let Some(t) = Timing::global() {
        t.record(kind, name, hit, secs);
    }
}

/// `true` when the runtime filesystem uses `'\\'` as the path separator (Windows).
/// Initialised once at startup from [`std::path::MAIN_SEPARATOR`].
static WINDOWS_FS: OnceLock<bool> = OnceLock::new();

/// Returns `true` when the runtime filesystem uses `'\\'` (Windows).
pub fn is_windows_fs() -> bool {
    *WINDOWS_FS.get_or_init(|| std::path::MAIN_SEPARATOR == '\\')
}

/// Platform path separator as a `char`: `'\\'` on Windows, `'/'` elsewhere.
/// Use this single token instead of probing for both `'/'` and `'\\'`.
#[must_use]
pub fn sep() -> char {
    if is_windows_fs() { '\\' } else { '/' }
}

/// Platform separator as a `&str`, for use as the replacement in [`str::replace`].
#[must_use]
pub fn sep_str() -> &'static str {
    if is_windows_fs() { "\\" } else { "/" }
}

/// The separator that is *not* native to this platform, as a `&str`.
/// Used to normalise incoming paths that may carry the foreign separator.
#[must_use]
pub fn other_sep() -> &'static str {
    if is_windows_fs() { "/" } else { "\\" }
}

// ---------------------------------------------------------------------------
// tmpfs-overflow safeguards for the native-compile paths.
//
// A native compile writes a static binary (~36MB unstripped) plus rustc/cc
// intermediates into the temp dir.  When that dir is a small RAM-backed tmpfs
// (the Linux default for /tmp), a parallel native run can write several GB —
// which is several GB of *RAM* — and exhaust memory hard enough to hang the
// machine, not merely fail a write.  The knobs below let every native path
// (a) strip the binaries so the footprint is tiny, (b) refuse to start a
// compile that would overflow, reclaiming loft's own stale artefacts first,
// and (c) scale parallelism to the available headroom.  They live here, in a
// pub lib module, so both the binary crate (main.rs, test_runner.rs) and the
// integration tests (tests/native.rs, which link `loft` as a library) share
// one implementation.
// ---------------------------------------------------------------------------

/// Directory for loft's own native-compile scratch — the generated `.rs`
/// source, the compiled binary, and the rustc/cc intermediates.
///
/// Honours the loft-specific `LOFT_TMPDIR` env var so a checkout can keep
/// these multi-MB artefacts off the system temp dir, falling back to the
/// system temp dir when unset.  Deliberately a loft-private knob rather than
/// a `TMPDIR` override, so it relocates only loft's own artefacts — never
/// every other tool's temp.  Created if missing, since rustc and loft both
/// assume the directory exists before writing into it.
#[must_use]
pub fn scratch_dir() -> std::path::PathBuf {
    let dir =
        std::env::var_os("LOFT_TMPDIR").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Bytes currently available on the filesystem backing `path`.
///
/// Uses `df -P -k` (POSIX output → guaranteed single, unwrapped data row) and
/// parses the Available column.  Returns `None` when `df` is missing or its
/// output is unparseable; callers treat `None` as "unknown — don't block", so
/// a missing `df` degrades to the pre-safeguard behaviour rather than refusing
/// to run.
#[must_use]
pub fn fs_avail_bytes(path: &std::path::Path) -> Option<u64> {
    let out = std::process::Command::new("df")
        .arg("-P")
        .arg("-k")
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // Line 0 = header, line 1 = the data row.  Columns (POSIX):
    //   Filesystem  1024-blocks  Used  Available  Capacity  Mounted-on
    let avail_kb: u64 = text
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()?;
    Some(avail_kb.saturating_mul(1024))
}

/// Opt-in native-compile timing (`LOFT_TIMING=1`).  OFF by default — a single
/// cached env read, zero cost when unset.  When set, the native-compile sites
/// (cdylib `auto_build_native`, the test runner's per-fixture rustc) emit one
/// `[loft-timing] …` line per event — cache hit vs miss, and the compile
/// wall-clock on a miss — to stderr.  `ci-probe` (and any local run)
/// aggregates these into a where-did-the-time-go view inside loft that the
/// CI step-level timing cannot see.
#[must_use]
pub fn timing_enabled() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_TIMING").is_ok())
}

/// Minimum free space (bytes) a native-compile path insists on before
/// spawning a `rustc` that writes into the temp filesystem.  Default 512MB;
/// override with `LOFT_TMPFS_MIN_FREE_MB` for unusual setups.
#[must_use]
pub fn tmpfs_min_free_bytes() -> u64 {
    const DEFAULT_MB: u64 = 512;
    let mb = std::env::var("LOFT_TMPFS_MIN_FREE_MB")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_MB);
    mb.saturating_mul(1024 * 1024)
}

/// The process id embedded in a `loft_native_<pid>.rs` /
/// `loft_native_bin_<pid>` / `loft_test_native_<pid>*` scratch name — the
/// trailing digit run.  `None` when the name carries no parseable pid.
fn scratch_owner_pid(name: &str) -> Option<u32> {
    let stem = name.split('.').next().unwrap_or(name);
    stem.rsplit('_').next()?.parse().ok()
}

/// Whether `pid` is a live process — `Some(true/false)` on Linux (procfs),
/// `None` (unknown) elsewhere.
fn pid_alive(pid: u32) -> Option<bool> {
    if cfg!(target_os = "linux") {
        Some(std::path::Path::new(&format!("/proc/{pid}")).exists())
    } else {
        None
    }
}

/// Delete loft's leftover native-compile artefacts from `dir` to reclaim
/// space, returning the bytes freed.  Only removes files matching loft's own
/// `loft_native_*` / `loft_test_native_*` naming — never another program's
/// temp files — and only files that are provably STALE:
///
/// - a file whose embedded pid is THIS process or a still-running one is
///   skipped — the original sweep deleted the `.rs` the current compile had
///   just emitted (its own pid matched the prefix), so under CI disk
///   pressure every space-constrained compile destroyed itself ("couldn't
///   read loft_native_<pid>.rs") and could equally race a parallel test's
///   in-flight file;
/// - when liveness is unknowable (no pid in the name, or no procfs), only
///   files older than an hour are deleted.
///
/// Those files ARE the binary cache, so this is called only when a path is
/// already space-constrained; the next run recompiles.
pub fn reclaim_native_scratch(dir: &std::path::Path) -> u64 {
    let own_pid = std::process::id();
    let mut freed = 0u64;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("loft_native_") || name.starts_with("loft_test_native_")) {
            continue;
        }
        let pid = scratch_owner_pid(&name);
        let proven_stale = match pid {
            Some(p) if p == own_pid => false,
            Some(p) => pid_alive(p) == Some(false),
            None => false,
        };
        if !proven_stale {
            // Liveness unknown (foreign pid without procfs, or no pid in the
            // name) — fall back to age: anything under an hour old may be an
            // in-flight emission of a parallel process.
            if pid.is_some_and(|p| p == own_pid || pid_alive(p) == Some(true)) {
                continue;
            }
            let old_enough = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 3600);
            if !old_enough {
                continue;
            }
        }
        let len = entry.metadata().map_or(0, |m| m.len());
        if std::fs::remove_file(entry.path()).is_ok() {
            freed = freed.saturating_add(len);
        }
    }
    freed
}

/// Preflight check for a single native compile writing into `scratch`.
/// Returns `true` when there is enough headroom to proceed.  When space is
/// below [`tmpfs_min_free_bytes`], first reclaims loft's own stale artefacts
/// and re-checks; only if it is *still* low does it return `false` (callers
/// skip that compile with a warning rather than risk overflowing RAM).
pub fn native_compile_space_ok(scratch: &std::path::Path) -> bool {
    let floor = tmpfs_min_free_bytes();
    match fs_avail_bytes(scratch) {
        // df unavailable → unknown → don't block (preserve old behaviour).
        None => true,
        Some(avail) if avail >= floor => true,
        Some(_) => {
            let freed = reclaim_native_scratch(scratch);
            if freed > 0 {
                eprintln!(
                    "loft: low space in {} — reclaimed {} MB of stale native artefacts",
                    scratch.display(),
                    freed / (1024 * 1024)
                );
            }
            fs_avail_bytes(scratch).is_none_or(|a| a >= floor)
        }
    }
}

/// Should native-compile binaries be stripped of symbols?  Stripping cuts
/// each binary from ~36MB to ~1MB (the bulk is debug info pulled in from
/// `libloft.rlib` + std, useless to a run-and-check test).  Set
/// `LOFT_NATIVE_KEEP_SYMBOLS=1` to keep symbols when debugging a native
/// crash; the generated `.rs` is always retained, so a single fixture can be
/// recompiled with `-g` on demand.
#[must_use]
pub fn native_strip_symbols() -> bool {
    std::env::var_os("LOFT_NATIVE_KEEP_SYMBOLS").is_none()
}

/// Worker-thread count for a parallel native run of `job_count` jobs in
/// `scratch`: the CPU parallelism, capped by the job count and by how many
/// concurrent compiles the temp filesystem can hold.  `reserve_per_worker` is
/// the peak temp footprint of one in-flight compile (callers pass a larger
/// value when binaries are unstripped).  On a roomy disk this never clamps; on
/// a tight tmpfs it scales down so the run finishes instead of hanging.
///
/// Used only by the parallel test suite (`tests/native.rs`); the binary's own
/// native paths compile serially, so it reads as dead in the binary view of
/// this dual lib+bin module.  The suppression goes away once the lib/bin
/// double compilation is removed — see PERFORMANCE.md § Design: BUILD1.
#[must_use]
#[allow(dead_code)]
pub fn native_worker_count(
    cpu_max: usize,
    job_count: usize,
    scratch: &std::path::Path,
    reserve_per_worker: u64,
) -> usize {
    let mem_cap = match fs_avail_bytes(scratch) {
        Some(avail) if reserve_per_worker > 0 => (avail / reserve_per_worker).max(1) as usize,
        _ => usize::MAX,
    };
    cpu_max.min(job_count).min(mem_cap).max(1)
}

#[cfg(test)]
mod reclaim_tests {
    use super::*;

    #[test]
    fn reclaim_spares_live_and_fresh_files() {
        let dir = std::env::temp_dir().join(format!("loft_reclaim_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let own = dir.join(format!("loft_native_{}.rs", std::process::id()));
        // u32::MAX-1 — no real pid (Linux pid_max caps far below); provably dead.
        let dead = dir.join("loft_native_4294967294.rs");
        let fresh_no_pid = dir.join("loft_native_bin_notapid");
        std::fs::write(&own, "live").unwrap();
        std::fs::write(&dead, "stale").unwrap();
        std::fs::write(&fresh_no_pid, "fresh").unwrap();
        let freed = reclaim_native_scratch(&dir);
        assert!(own.exists(), "own-pid file must survive the reclaim");
        assert!(
            fresh_no_pid.exists(),
            "a fresh file without a parseable pid must survive (age floor)"
        );
        if cfg!(target_os = "linux") {
            assert!(!dead.exists(), "dead-pid file must be reclaimed");
            assert_eq!(freed, 5);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
