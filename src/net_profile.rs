// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `LOFT_NET_PROFILE` — what the network did, beside `LOFT_PROFILE` (CPU) and
//! `LOFT_ALLOC_SITES` (memory).
//!
//! Those two answer "where did the time go" and "what filled the heap". Neither can see
//! the thing that makes a networked test flake: an operation that COMPLETED, well within
//! its own success criteria, but close enough to a deadline that a slower machine would
//! have missed it. A CPU profile of such a run is unremarkable — the process was waiting.
//!
//! So the metric here is **margin**, not duration. `engine_host.rs` bounds its reads at
//! 500 ms and 20 ms; a read that returns at 19 ms against a 20 ms budget is a failure that
//! has not happened yet, and it is invisible to every other instrument and to a green test
//! run. `near_misses` counts those, which is what turns "it flakes sometimes" into a
//! specific operation with a specific budget to argue about.
//!
//! Every event carries WALL-CLOCK start and end, not just a duration. A duration cannot
//! answer the question an orphaned-server flake actually poses — *did the client connect
//! before the server bound?* — because that needs two PROCESSES' event streams laid on one
//! timeline. Micros-since-epoch is the anchor that survives a process boundary.
//!
//! Events also carry BYTES, so a stalled transfer is distinguishable from a small one and
//! a byte-at-a-time header read is visible as the 1-byte-per-syscall loop it is.
//!
//! Off costs a cached bool. Armed, it costs one `Instant::now()` pair per socket call.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A read that used this fraction of its budget is reported as a near miss.
const NEAR_MISS_FRACTION: f64 = 0.5;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Outcome {
    Ok,
    TimedOut,
    Failed,
}

#[derive(Default)]
struct Site {
    calls: u64,
    total_us: u64,
    max_us: u64,
    timed_out: u64,
    failed: u64,
    near_misses: u64,
    budget_us: u64,
    bytes: u64,
    first_start_us: u64,
    last_end_us: u64,
}

/// Wall-clock micros since the epoch — the anchor that lets two processes' streams be
/// merged.  A monotonic `Instant` cannot do this: it has no meaning outside its process.
fn wall_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
}

static SITES: OnceLock<Mutex<std::collections::BTreeMap<&'static str, Site>>> = OnceLock::new();
static EVENTS: AtomicU64 = AtomicU64::new(0);

/// `LOFT_NET_PROFILE=1` — summary at exit. `=trace` also prints each event as it happens,
/// which is what you want when the ORDER of operations is the question (a bind that lost a
/// race, a client that connected before the listener was up).
#[must_use]
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_NET_PROFILE").is_some())
}

fn tracing() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("LOFT_NET_PROFILE").is_ok_and(|v| v.eq_ignore_ascii_case("trace"))
    })
}

/// Record one completed socket operation.  `budget` is the deadline the call was made
/// under, when it had one — that is what makes a near miss visible.
pub fn record(site: &'static str, dur: Duration, outcome: Outcome, budget: Option<Duration>) {
    record_io(site, dur, outcome, budget, 0);
}

/// As [`record`], plus how many BYTES moved.  `end` is derived rather than sampled again,
/// so the pair is internally consistent even under a clock adjustment.
pub fn record_io(
    site: &'static str,
    dur: Duration,
    outcome: Outcome,
    budget: Option<Duration>,
    bytes: u64,
) {
    if !enabled() {
        return;
    }
    let us = u64::try_from(dur.as_micros()).unwrap_or(u64::MAX);
    let end_us = wall_us();
    let start_us = end_us.saturating_sub(us);
    EVENTS.fetch_add(1, Ordering::Relaxed);
    if tracing() {
        let b = budget.map_or_else(String::new, |b| format!(" budget={}us", b.as_micros()));
        let n = if bytes > 0 {
            format!(" bytes={bytes}")
        } else {
            String::new()
        };
        // start/end are absolute so two processes' traces can be merged and ordered.
        crate::loft_eprintln!(
            "[net] start={start_us} end={end_us} dur={us}us {site} {outcome:?}{b}{n}"
        );
    }
    let map = SITES.get_or_init(|| Mutex::new(std::collections::BTreeMap::new()));
    if let Ok(mut m) = map.lock() {
        let e = m.entry(site).or_default();
        e.calls += 1;
        e.total_us += us;
        e.max_us = e.max_us.max(us);
        e.bytes += bytes;
        if e.first_start_us == 0 {
            e.first_start_us = start_us;
        }
        e.last_end_us = end_us;
        match outcome {
            Outcome::Ok => {}
            Outcome::TimedOut => e.timed_out += 1,
            Outcome::Failed => e.failed += 1,
        }
        if let Some(b) = budget {
            let b_us = u64::try_from(b.as_micros()).unwrap_or(u64::MAX);
            e.budget_us = b_us;
            #[allow(clippy::cast_precision_loss)]
            if outcome == Outcome::Ok
                && b_us > 0
                && (us as f64) >= (b_us as f64) * NEAR_MISS_FRACTION
            {
                e.near_misses += 1;
            }
        }
    }
}

/// Time a socket call and record it.  `budget` is the read/write deadline in force.
///
/// # Errors
/// Propagates the wrapped call's error unchanged; the profiler only observes it.
pub fn time<T: IoVolume, F: FnOnce() -> std::io::Result<T>>(
    site: &'static str,
    budget: Option<Duration>,
    f: F,
) -> std::io::Result<T> {
    if !enabled() {
        return f();
    }
    let t0 = Instant::now();
    let r = f();
    let dur = t0.elapsed();
    let outcome = match &r {
        Ok(_) => Outcome::Ok,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Outcome::TimedOut
        }
        Err(_) => Outcome::Failed,
    };
    let bytes = r.as_ref().map_or(0, IoVolume::volume);
    record_io(site, dur, outcome, budget, bytes);
    r
}

/// How many bytes an io result moved.  `read`/`write` answer a count; everything else
/// (`accept`, `connect`) moves none, and says so rather than guessing.
pub trait IoVolume {
    fn volume(&self) -> u64;
}
impl IoVolume for usize {
    fn volume(&self) -> u64 {
        *self as u64
    }
}
impl IoVolume for () {
    fn volume(&self) -> u64 {
        0
    }
}

/// Print the summary.  Silent when nothing was recorded, so an unprofiled run — or one
/// that never touched a socket — prints nothing at all.
pub fn report() {
    if !enabled() || EVENTS.load(Ordering::Relaxed) == 0 {
        return;
    }
    let Some(map) = SITES.get() else { return };
    let Ok(m) = map.lock() else { return };
    crate::loft_eprintln!("\n[net-profile] socket operations by site");
    crate::loft_eprintln!(
        "  {:<30} {:>6} {:>9} {:>9} {:>7} {:>6} {:>10} {:>10} {:>12}",
        "site",
        "calls",
        "avg_us",
        "max_us",
        "t/out",
        "fail",
        "near-miss",
        "bytes",
        "span_ms"
    );
    for (site, e) in m.iter() {
        let avg = e.total_us.checked_div(e.calls).unwrap_or(0);
        let near = if e.budget_us == 0 {
            "-".to_string()
        } else {
            format!("{} /{}us", e.near_misses, e.budget_us)
        };
        // span = first start to last end, so a site that was active for a long while but
        // cheap per call (a poll loop) is distinguishable from one that was simply slow.
        let span_ms = e.last_end_us.saturating_sub(e.first_start_us) / 1000;
        crate::loft_eprintln!(
            "  {:<30} {:>6} {:>9} {:>9} {:>7} {:>6} {:>10} {:>10} {:>12}",
            site,
            e.calls,
            avg,
            e.max_us,
            e.timed_out,
            e.failed,
            near,
            e.bytes,
            span_ms
        );
    }
    let risky: Vec<_> = m.iter().filter(|(_, e)| e.near_misses > 0).collect();
    if !risky.is_empty() {
        crate::loft_eprintln!(
            "  ⚠ {} site(s) completed within {:.0}% of a deadline — a slower machine misses these",
            risky.len(),
            NEAR_MISS_FRACTION * 100.0
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The instrument must be able to SHOW a transfer, not just time one: a start, an end,
    /// and how much moved.  Without the byte count a stalled transfer and a small one look
    /// identical; without absolute stamps two processes' traces cannot be merged, which is
    /// the whole point when the question is whether a client connected before a server
    /// bound.
    #[test]
    fn record_io_carries_stamps_and_volume() {
        // SAFETY: single-threaded test process; the var is read through a OnceLock cache,
        // so it must be set before the first `enabled()` call in this process.
        unsafe { std::env::set_var("LOFT_NET_PROFILE", "1") };
        assert!(enabled(), "profiler must arm from the env var");

        let before = wall_us();
        record_io(
            "test/send",
            Duration::from_millis(4),
            Outcome::Ok,
            Some(Duration::from_millis(20)),
            1500,
        );
        let after = wall_us();

        let m = SITES.get().expect("a recorded event creates the map");
        let g = m.lock().expect("lock");
        let e = g.get("test/send").expect("the site was recorded");
        assert_eq!(e.calls, 1);
        assert_eq!(e.bytes, 1500, "byte volume is carried");
        assert_eq!(e.budget_us, 20_000, "the deadline is carried");
        assert!(
            e.first_start_us >= before.saturating_sub(4_000) && e.last_end_us <= after,
            "start/end are wall-clock and bracket the call: {} .. {} not within {before} .. {after}",
            e.first_start_us,
            e.last_end_us
        );
        assert_eq!(
            e.last_end_us.saturating_sub(e.first_start_us),
            4_000,
            "end - start reproduces the duration"
        );
        // 4ms of a 20ms budget is 20% — under the near-miss line, so not flagged.
        assert_eq!(e.near_misses, 0, "a comfortable call is not a near miss");
    }

    /// The margin metric is the reason this exists: a call that SUCCEEDED close to its
    /// deadline is a failure that has not happened yet, and nothing else reports it.
    #[test]
    fn a_call_close_to_its_deadline_is_flagged() {
        unsafe { std::env::set_var("LOFT_NET_PROFILE", "1") };
        record_io(
            "test/near",
            Duration::from_millis(19),
            Outcome::Ok,
            Some(Duration::from_millis(20)),
            8,
        );
        let m = SITES.get().expect("map");
        let g = m.lock().expect("lock");
        let e = g.get("test/near").expect("site");
        assert_eq!(
            e.near_misses, 1,
            "19ms against a 20ms budget must be reported as a near miss"
        );
    }
}
