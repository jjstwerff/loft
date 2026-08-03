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

/// A unique per-PROCESS build-scratch directory under [`scratch_dir`], holding
/// one compilation's intermediates (the generated `.rs`, the rustc `.wasm`
/// output + its `*.rcgu.o` objects, bridge rlibs).
///
/// Each `loft` invocation is its own OS process, so keying the directory on the
/// PID isolates concurrent compilations: nextest runs every test in a separate
/// process, and a build script can emit many pages at once. Before this, the
/// `--html` / `--native-wasm` drivers wrote a *shared* `scratch/loft_html.rs`
/// and `-o scratch/loft_html.wasm`, so two parallel runs raced two ways — one
/// rustc read the other's generated source (a page silently embedded the wrong
/// program), and two `rust-lld`s truncated each other's output mid-link
/// (`signal: 7`, SIGBUS). This is the same isolation the `--native` binary path
/// already applies via `loft_native_{pid}`. PID (not a random token) suffices
/// because the racing unit is the *process*, and it keeps the path predictable
/// for `LOFT_KEEP_NATIVE_RS` inspection.
#[must_use]
pub fn build_scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = scratch_dir().join(format!("loft_{tag}_{}", std::process::id()));
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

/// Which naming convention [`lib_variants`] should translate into.
///
/// A parameter rather than a `cfg!` inside the translation, so the macOS and
/// Windows spellings are checkable from any machine. Left implicit, each one is
/// only ever exercised on its own platform — and this whole fallback exists
/// because a platform nobody could run locally had been broken for a while.
#[derive(Clone, Copy, PartialEq)]
pub enum LibOs {
    Linux,
    Macos,
    Windows,
}

/// The declared `[c]` library name, then the same library spelled for THIS host.
///
/// A `[c] libs` entry is one string in a manifest, so it cannot say `.so` on
/// Linux and `.dylib` on macOS. Rather than invent a per-platform manifest key,
/// translate at use time: the declared spelling is authoritative and always
/// tried first, and these are what to try when the host does not use it.
///
/// Shared by the two places that resolve such a name — the interpreter's
/// `dlopen` (`extensions::load_c_library`) and the `--native` LINK line
/// (`native_utils::add_c_library_flags`). They must agree: a declaration that
/// loads under `--interpret` and fails to link under `--native` is the backend
/// divergence @PLN24 keeps refusing to ship, and giving each its own copy of
/// the spelling rules is how that happens.
pub fn host_lib_variants(name: &str) -> Vec<String> {
    lib_variants(name, host_lib_os())
}

/// Which naming convention THIS host uses. The single `cfg!` read, so the
/// translation itself stays a pure function of its argument.
#[must_use]
pub fn host_lib_os() -> LibOs {
    if cfg!(target_os = "macos") {
        LibOs::Macos
    } else if cfg!(windows) {
        LibOs::Windows
    } else {
        LibOs::Linux
    }
}

/// The `[c]` library that is really present beside `dir`, under whichever
/// spelling this host uses. `None` when nothing is there — a bare soname the
/// dynamic linker resolves, or a library that is simply missing.
///
/// The link line needs this as much as the loader does: a package that ships
/// its own library is declared as a PATH, and reading only the declared
/// spelling meant macOS found nothing beside the package, emitted no `-L` and
/// no rpath, and sent `-l dylib=<stem>` to the system search path where the
/// library is not.
#[must_use]
pub fn existing_lib_beside(dir: &std::path::Path, name: &str, os: LibOs) -> Option<String> {
    lib_variants(name, os)
        .into_iter()
        .find(|cand| dir.join(cand).exists())
}

/// Linker flags that pin a freshly built shared library's own name.
///
/// macOS records an INSTALL NAME inside the Mach-O — whatever `-o` said — and
/// every binary that links the library copies that string in as the thing to ask
/// `dyld` for. loft builds a shim to `<stem>.<pid>.tmp` and renames it over the
/// final name so the publish is atomic; the rename moves the FILE and leaves the
/// recorded name alone, so consumers went looking for a `.tmp` that no longer
/// existed:
///
/// ```text
/// dyld: Library not loaded: …/native-auto/lcshim_shim_<key>.71616.tmp
/// ```
///
/// `@rpath/<final name>` rather than the absolute path: loft already emits
/// `-Wl,-rpath,<dir>` for the shim directory, and a relocatable name keeps a
/// built binary working if the package moves. ELF has no equivalent problem —
/// its `SONAME` is only set when asked — so this is empty off macOS.
#[must_use]
pub fn shim_name_args(final_file_name: &str, os: LibOs) -> Vec<String> {
    if os == LibOs::Macos {
        vec![format!("-Wl,-install_name,@rpath/{final_file_name}")]
    } else {
        Vec::new()
    }
}

/// [`host_lib_variants`], with the target convention passed in.
///
/// The versioned forms matter as much as the bare one — a real declaration is
/// `libmariadb.so.3`, whose macOS twin is `libmariadb.3.dylib` (the soversion
/// moves BEFORE the extension) and whose Windows twin drops both the `lib`
/// prefix and the version. Returns just the declared name on Linux, where the
/// spelling already is the host's.
pub fn lib_variants(name: &str, os: LibOs) -> Vec<String> {
    let mut out = vec![name.to_string()];
    if os == LibOs::Linux {
        return out; // the declared spelling already is this host's
    }
    // Split a trailing directory off so the translation only rewrites the file
    // name — `../../liblc_types.so` must stay relative to the same place.
    let (dir, file) = match name.rfind(['/', '\\']) {
        Some(i) => (&name[..=i], &name[i + 1..]),
        None => ("", name),
    };
    // `libfoo.so.3` → stem `libfoo`, version `3`; `libfoo.so` → no version.
    let Some((stem, rest)) = file.split_once(".so") else {
        return out; // not a Linux spelling — nothing to translate
    };
    let version = rest.strip_prefix('.').filter(|v| !v.is_empty());
    let mut push = |f: String| out.push(format!("{dir}{f}"));
    if os == LibOs::Macos {
        if let Some(v) = version {
            push(format!("{stem}.{v}.dylib"));
        }
        push(format!("{stem}.dylib"));
    } else {
        // No `lib` prefix and no soversion in a DLL name; try both spellings
        // because a MinGW-built library keeps the prefix.
        let bare = stem.strip_prefix("lib").unwrap_or(stem);
        push(format!("{bare}.dll"));
        push(format!("{stem}.dll"));
    }
    out
}

#[cfg(test)]
mod shim_name_tests {
    use super::{LibOs, shim_name_args};

    /// macOS bakes the `-o` path into a dylib as its install name, so a library
    /// published by rename must be told the name it will END UP with. Checked
    /// from any host by passing the convention in — left implicit this could
    /// only ever run on a Mac, which is how the `.tmp` install name shipped.
    #[test]
    fn macos_pins_the_final_name_and_other_hosts_add_nothing() {
        assert_eq!(
            shim_name_args("lcshim_shim_bda315af5ea4cb63.dylib", LibOs::Macos),
            ["-Wl,-install_name,@rpath/lcshim_shim_bda315af5ea4cb63.dylib"],
            "the FINAL file name, and @rpath so the emitted -rpath resolves it"
        );
        for os in [LibOs::Linux, LibOs::Windows] {
            assert!(
                shim_name_args("libfoo.so", os).is_empty(),
                "ELF sets a SONAME only when asked, and a DLL has no such record"
            );
        }
    }

    /// The name must carry no directory: an install name is what `dyld` is
    /// ASKED for, and `@rpath/` already supplies the search.
    #[test]
    fn the_install_name_is_relative_to_rpath() {
        let a = shim_name_args("x.dylib", LibOs::Macos);
        assert!(a[0].starts_with("-Wl,-install_name,@rpath/"), "{a:?}");
        assert!(
            !a[0].contains("/native-auto/") && !a[0].contains(".tmp"),
            "neither the staging path nor an absolute directory may survive: {a:?}"
        );
    }
}
