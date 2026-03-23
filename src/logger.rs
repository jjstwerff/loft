// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Runtime logging framework for loft / loft programs.
//!
//! Distinct from `log_config.rs` (the compile/test trace framework):
//! this module handles structured, file-based output from running loft code.

#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Log severity level.  Levels are ordered: Info < Warn < Error < Fatal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warn,
    Error,
    Fatal,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "INFO ",
            Severity::Warn => "WARN ",
            Severity::Error => "ERROR",
            Severity::Fatal => "FATAL",
        }
    }

    fn from_str(s: &str) -> Option<Severity> {
        match s.to_ascii_lowercase().as_str() {
            "info" => Some(Severity::Info),
            "warn" | "warning" => Some(Severity::Warn),
            "error" => Some(Severity::Error),
            "fatal" => Some(Severity::Fatal),
            _ => None,
        }
    }
}

/// Runtime logging configuration (parsed from a `.conf` file or built with defaults).
pub struct RuntimeLogConfig {
    pub log_path: PathBuf,
    pub default_level: Severity,
    pub production: bool,
    pub max_size_bytes: u64,
    pub daily_rotation: bool,
    pub max_files: u32,
    pub rate_per_minute: u32,
    pub file_levels: HashMap<String, Severity>,
}

impl Default for RuntimeLogConfig {
    fn default() -> Self {
        RuntimeLogConfig {
            log_path: PathBuf::from("log.txt"),
            default_level: Severity::Warn,
            production: false,
            max_size_bytes: 500 * 1024 * 1024,
            daily_rotation: true,
            max_files: 10,
            rate_per_minute: 5,
            file_levels: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

struct RateEntry {
    count: u32,
    window_start: Instant,
    suppressed: u32,
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

/// Runtime logger shared across threads via `Arc<Mutex<Logger>>`.
pub struct Logger {
    pub config: RuntimeLogConfig,
    config_path: Option<PathBuf>,
    config_mtime: Option<SystemTime>,
    last_config_check: Instant,
    file: Option<BufWriter<File>>,
    current_size: u64,
    /// (year, month, day) in UTC — used for daily rotation detection.
    current_ymd: (u32, u32, u32),
    rate_map: HashMap<(String, u32), RateEntry>,
}

impl Logger {
    /// Create a logger from an already-built `RuntimeLogConfig`.
    #[must_use]
    pub fn new(config: RuntimeLogConfig, config_path: Option<PathBuf>) -> Self {
        let mut logger = Logger {
            config,
            config_path,
            config_mtime: None,
            last_config_check: Instant::now(),
            file: None,
            current_size: 0,
            current_ymd: (0, 0, 0),
            rate_map: HashMap::new(),
        };
        // Record mtime of config file if we have one.
        if let Some(ref p) = logger.config_path.clone() {
            logger.config_mtime = std::fs::metadata(p).ok().and_then(|m| m.modified().ok());
        }
        logger.open_file();
        logger
    }

    /// Build a production-mode logger that suppresses panics (assert/panic set `had_fatal`
    /// instead of aborting).  Does not write to any log file.
    #[must_use]
    pub fn production() -> Self {
        let config = RuntimeLogConfig {
            production: true,
            ..RuntimeLogConfig::default()
        };
        Logger {
            config,
            config_path: None,
            config_mtime: None,
            last_config_check: Instant::now(),
            file: None,
            current_size: 0,
            current_ymd: (0, 0, 0),
            rate_map: HashMap::new(),
        }
    }

    /// Build a `Logger` from a config file path (or defaults if the file doesn't exist).
    ///
    /// `main_loft_file` is used to determine the default log directory.
    #[must_use]
    pub fn from_config_file(path: &Path, main_loft_file: &str) -> Self {
        let default_log_dir = Path::new(main_loft_file)
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let default_log_path = default_log_dir.join(".loft").join("log.txt");

        let config = if path.exists() {
            if let Ok(content) = std::fs::read_to_string(path) {
                let conf_dir = path.parent().unwrap_or(Path::new("."));
                parse_config_str(&content, conf_dir)
            } else {
                RuntimeLogConfig {
                    log_path: default_log_path,
                    ..Default::default()
                }
            }
        } else {
            RuntimeLogConfig {
                log_path: default_log_path,
                ..Default::default()
            }
        };

        let config_path = if path.exists() {
            Some(path.to_path_buf())
        } else {
            None
        };
        Logger::new(config, config_path)
    }

    /// Write a log record.  Applies rate limiting and level filtering.
    ///
    /// # Panics
    ///
    /// Panics if the internal rate-limit map entry is missing after insertion (indicates a bug).
    pub fn log(&mut self, sev: Severity, loft_file: &str, line: u32, msg: &str) {
        // Level filter
        if sev < self.effective_level(loft_file) {
            return;
        }

        // Rate limiting
        let key = (loft_file.to_string(), line);
        let now = Instant::now();
        let rate = self.config.rate_per_minute;
        if rate > 0 {
            let window = Duration::from_secs(60);
            // Compute the suppression notice (if any) and update state before any borrow
            let suppression_notice: Option<String> = {
                let entry = self
                    .rate_map
                    .entry(key.clone())
                    .or_insert_with(|| RateEntry {
                        count: 0,
                        window_start: now,
                        suppressed: 0,
                    });
                if now.duration_since(entry.window_start) > window {
                    let notice = if entry.suppressed > 0 {
                        let ts = utc_timestamp();
                        let sev_str = sev.as_str();
                        let file = loft_file;
                        let n = entry.suppressed;
                        Some(format!(
                            "{ts} {sev_str}  {file}:{line}  [suppressed {n} identical messages in the last 60s]\n",
                        ))
                    } else {
                        None
                    };
                    entry.count = 0;
                    entry.suppressed = 0;
                    entry.window_start = now;
                    notice
                } else {
                    None
                }
            };
            // Now check rate (re-borrow entry)
            let should_suppress = {
                let entry = self.rate_map.get_mut(&key).expect("just inserted");
                if entry.count >= rate {
                    entry.suppressed += 1;
                    true
                } else {
                    entry.count += 1;
                    false
                }
            };
            if let Some(notice) = suppression_notice {
                self.write_line(&notice);
            }
            if should_suppress {
                return;
            }
        }

        // Check if daily rotation is needed
        let today = today_ymd();
        if self.config.daily_rotation && self.current_ymd != (0, 0, 0) && today != self.current_ymd
        {
            self.rotate();
        }
        self.current_ymd = today;

        // Check size rotation
        if self.config.max_size_bytes > 0 && self.current_size >= self.config.max_size_bytes {
            self.rotate();
        }

        let ts = utc_timestamp();
        let sev_str = sev.as_str();
        let file = loft_file;
        let line_str = format!("{ts} {sev_str}  {file}:{line}  {msg}\n");
        self.write_line(&line_str);
    }

    /// Check if the config file has changed and reload if so.  Only does I/O if 5+ seconds
    /// have passed since the last check.
    pub fn check_reload(&mut self) {
        if self.last_config_check.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.last_config_check = Instant::now();
        let Some(config_path) = self.config_path.clone() else {
            return;
        };
        let new_mtime = std::fs::metadata(&config_path)
            .ok()
            .and_then(|m| m.modified().ok());
        if new_mtime == self.config_mtime {
            return;
        }
        // Config changed — reload
        self.config_mtime = new_mtime;
        let old_path = self.config.log_path.clone();
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            let conf_dir = config_path.parent().unwrap_or(Path::new("."));
            let new_config = parse_config_str(&content, conf_dir);
            let path_changed = new_config.log_path != old_path;
            self.config = new_config;
            if path_changed {
                self.file = None;
                self.current_size = 0;
                self.open_file();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn effective_level(&self, loft_file: &str) -> Severity {
        // Check per-file overrides: exact basename match or path prefix
        let basename = Path::new(loft_file)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(loft_file);

        if let Some(&sev) = self.config.file_levels.get(basename) {
            return sev;
        }
        // Path prefix check (keys ending in "/")
        for (pattern, &sev) in &self.config.file_levels {
            if pattern.ends_with('/') && loft_file.starts_with(pattern.as_str()) {
                return sev;
            }
        }
        self.config.default_level
    }

    fn write_line(&mut self, line: &str) {
        let bytes = line.as_bytes();
        if let Some(ref mut f) = self.file {
            let _ = f.write_all(bytes);
            let _ = f.flush();
            self.current_size += bytes.len() as u64;
        }
    }

    fn open_file(&mut self) {
        let path = &self.config.log_path;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match OpenOptions::new().create(true).append(true).open(path) {
            Ok(f) => {
                let size = f.metadata().map(|m| m.len()).unwrap_or(0);
                self.current_size = size;
                self.file = Some(BufWriter::new(f));
                self.current_ymd = today_ymd();
            }
            Err(_) => {
                self.file = None;
            }
        }
    }

    fn rotate(&mut self) {
        // Flush and close current file
        if let Some(ref mut f) = self.file {
            let _ = f.flush();
        }
        self.file = None;

        let path = &self.config.log_path;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("log")
            .to_string();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| format!(".{s}"))
            .unwrap_or_default();
        let dir = path.parent().unwrap_or(Path::new("."));

        let max = self.config.max_files;

        // Delete the oldest file if at the limit
        if max > 0 {
            let oldest = dir.join(format!(
                "{stem}.{}.{}",
                max - 1,
                ext.trim_start_matches('.')
            ));
            let _ = std::fs::remove_file(&oldest);
        }

        // Shift: log.(N-2).ext → log.(N-1).ext, …, log.1.ext → log.2.ext
        if max > 1 {
            for i in (1..max - 1).rev() {
                let src = dir.join(format!("{stem}.{i}{ext}"));
                let dst = dir.join(format!("{stem}.{}{ext}", i + 1));
                let _ = std::fs::rename(&src, &dst);
            }
        }

        // log.ext → log.1.ext
        let archive = dir.join(format!("{stem}.1{ext}"));
        let _ = std::fs::rename(path, &archive);

        // Open fresh log file
        self.current_size = 0;
        self.open_file();
    }
}

// ---------------------------------------------------------------------------
// Config parsing
// ---------------------------------------------------------------------------

fn parse_config_str(content: &str, conf_dir: &Path) -> RuntimeLogConfig {
    let mut config = RuntimeLogConfig::default();
    let mut section = String::new();
    let mut log_path_set = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim();
            let value = line[eq + 1..].trim().trim_matches('"');
            match (section.as_str(), key) {
                ("log", "file") => {
                    let p = Path::new(value);
                    config.log_path = if p.is_absolute() {
                        p.to_path_buf()
                    } else {
                        conf_dir.join(p)
                    };
                    log_path_set = true;
                }
                ("log", "level") => {
                    if let Some(sev) = Severity::from_str(value) {
                        config.default_level = sev;
                    }
                }
                ("log", "production") => {
                    config.production = value.eq_ignore_ascii_case("true") || value == "1";
                }
                ("rotation", "max_size_mb") => {
                    if let Ok(mb) = value.parse::<u64>() {
                        config.max_size_bytes = mb * 1024 * 1024;
                    }
                }
                ("rotation", "daily") => {
                    config.daily_rotation = value.eq_ignore_ascii_case("true") || value == "1";
                }
                ("rotation", "max_files") => {
                    if let Ok(n) = value.parse::<u32>() {
                        config.max_files = n;
                    }
                }
                ("rate_limit", "per_site") => {
                    if let Ok(n) = value.parse::<u32>() {
                        config.rate_per_minute = n;
                    }
                }
                ("levels", _) => {
                    if let Some(sev) = Severity::from_str(value) {
                        config.file_levels.insert(key.to_string(), sev);
                    }
                }
                _ => {}
            }
        }
    }

    if !log_path_set {
        // Keep default relative path unchanged (will be set by caller if needed)
    }
    config
}

// ---------------------------------------------------------------------------
// Config template
// ---------------------------------------------------------------------------

/// Returns the documented default config template as a static string.
#[must_use]
pub fn generate_config() -> &'static str {
    r#"# Lavition runtime log configuration
# Generated by: loft --generate-log-config
#
# Hot-reload: changes are picked up within ~5 seconds without restart.
# All paths are relative to the directory containing this config file
# unless they start with / or ~ (absolute/home).

[log]

# Path to the log file.
# Relative to the directory of the main .loft file.
# Default: log.txt
file = log.txt

# Minimum severity to write.
# Choices: info | warn | error | fatal
# Records below this level are silently discarded.
# Default: warn
level = warn

# Production mode: when true, panic() becomes a fatal log entry and
# assert() becomes an error log entry.  The program continues running
# instead of aborting.
# Default: false
production = false

[rotation]

# Maximum size of a single log file in megabytes before rotation.
# Set to 0 to disable size-based rotation.
# Default: 500
max_size_mb = 500

# Rotate at midnight UTC even if the size limit has not been reached.
# Default: true
daily = true

# Maximum total number of log files to keep (current + archived).
# Oldest files beyond this count are deleted during rotation.
# Default: 10
max_files = 10

[rate_limit]

# Maximum messages allowed from the same source location (file + line)
# within a 60-second sliding window.  Further messages from that site
# are suppressed; a suppression notice is emitted when the window resets.
# Set to 0 to disable rate limiting.
# Default: 5
per_site = 5

[levels]

# Per-file severity overrides.  The key is the loft source file name
# (basename only, e.g. "score.loft") or a path prefix ending in "/".
# The value overrides the global [log] level for that file.
#
# Examples:
#   "debug_tool.loft" = info
#   "src/"            = error
"#
}

// ---------------------------------------------------------------------------
// Timestamp helpers (no external crates)
// ---------------------------------------------------------------------------

/// Format `SystemTime::now()` as an ISO 8601 UTC timestamp with millisecond precision.
/// Example: `"2026-03-13T14:05:32.417Z"`
fn utc_timestamp() -> String {
    let now = SystemTime::now();
    let since_epoch = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let total_secs = since_epoch.as_secs();
    let millis = since_epoch.subsec_millis();

    let (y, m, d) = days_to_ymd(total_secs / 86400);
    let rem = total_secs % 86400;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;

    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Return today's UTC date as `(year, month, day)`.
fn today_ymd() -> (u32, u32, u32) {
    let since_epoch = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    days_to_ymd(since_epoch.as_secs() / 86400)
}

/// Convert a count of days since the Unix epoch (1970-01-01) to `(year, month, day)`.
///
/// Algorithm from Howard Hinnant: <https://howardhinnant.github.io/date_algorithms.html>
#[allow(clippy::many_single_char_names)] // Standard mathematical variable names from the algorithm
fn days_to_ymd(z: u64) -> (u32, u32, u32) {
    // Shift epoch from 1970-01-01 to 0000-03-01
    let z = z as i64 + 719_468;
    let era: i64 = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // year of era [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month of year [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as u32, m as u32, d as u32)
}
