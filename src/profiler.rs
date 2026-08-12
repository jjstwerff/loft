// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN140 arc B/C — the loft-level profiler (CPU hot spots, and paths to allocations)

//! Which **loft** function and line is a run spending its time in, and what path
//! reached it.
//!
//! `perf` cannot answer that for the interpreter, and no sampling frequency fixes it:
//! a loft call creates no machine frame, so perf's stack walk yields the
//! interpreter's own path — `_start → main → execute_argv → put_stack::<i64>` —
//! identical for every program ever run. It is the wrong stack, not a truncated one.
//! loft keeps the right one itself, in [`State::call_stack`](crate::state::State),
//! and this samples it.
//!
//! # The clock (plan open question 1)
//!
//! Three mechanisms were on the table: a `SIGPROF` timer (true time sampling, but
//! signal-safe, and it has to answer for `par` worker threads), a dedicated counter
//! in the dispatch loop (a branch and an increment per op, paid by every run), and
//! the existing per-op `self.debug.is_some()` branch, which costs nothing when off
//! because it is already there.
//!
//! This is the third — with the objection to it fixed. An **op clock is not a time
//! clock**: one op that calls a heavy native counts once, so a program dominated by
//! `sort` or a store operation would be under-reported exactly where it matters. So
//! the op counter chooses *when* to sample and the wall clock says *how much*: each
//! sample carries the nanoseconds elapsed since the previous one, and that interval
//! is credited to the frame the sample landed in. A native call that takes a
//! millisecond inside one op shows up as a millisecond, because the next sample's
//! interval is a millisecond long.
//!
//! What that costs when off: nothing. What it costs when on: one `Instant::now()` and
//! one map update per `interval` ops (1024 by default).
//!
//! What it still cannot do, stated because a profiler that hides its blind spots is
//! worse than none:
//!
//! * **The interval lands *after* the expensive op, not inside it.** Time spent in a
//!   single long native call is credited to the frame executing when the next sample
//!   fires. That is the same loft function in every ordinary case (the op and its
//!   successor are in one body) and wrong across a call boundary.
//! * **`par` workers are not sampled.** A worker runs its own `State`; its frames
//!   never reach this one. The report says so rather than quietly reporting the main
//!   thread's share of a parallel program as the whole.
//! * **`--native` is out of scope here.** There is no dispatch loop to hook; that
//!   backend is `perf`'s, and `scripts/profile.sh` routes to it.

use std::collections::HashMap;
use std::time::Instant;

/// How many innermost frames a recorded path keeps.
///
/// A recursive program has a distinct chain per depth — `fib(38)` alone would mint
/// 38 of them, and a deep walker thousands — which turns the path table into a list
/// of one-sample rows that ranks nothing. Keeping the innermost frames collapses
/// those into the one chain a reader wants (`… → fib → fib → fib`) and bounds the
/// table.
pub const PATH_DEPTH: usize = 8;

/// Ceiling on distinct paths retained. Reached only by a program with an enormous
/// call graph; the report names the number dropped rather than presenting a
/// truncated table as a complete one.
const MAX_PATHS: usize = 20_000;

/// A sampler on a FIXED period samples one phase of a periodic program and reports
/// it as the whole.
///
/// Not a hypothetical: the arc C oracle
/// (`bench/profile_oracle/alloc_paths.loft`) allocates down two paths in a known 9:1
/// ratio, and a fixed every-16th-allocation sampler put **100 %** on one of them and
/// never once saw the other. Two allocating ops per iteration and a period of 16
/// means only odd iterations are ever sampled, and the other path is only ever
/// reached on even ones — so the report was not noisy, it was *confidently wrong*,
/// and no sample count would have revealed it.
///
/// So the interval is jittered: each period is drawn from `[n/2, 3n/2)` with the
/// same mean, which cannot stay in lock-step with any program period. The generator
/// is a fixed-seed xorshift rather than a system source, because a profile that
/// cannot be reproduced cannot be compared with the previous one — which is exactly
/// what the corpus diff does.
#[derive(Debug)]
struct Jitter {
    state: u64,
}

impl Jitter {
    const fn new() -> Jitter {
        Jitter {
            state: 0x2545_f491_4f6c_dd1d,
        }
    }

    /// The next interval: uniform in `[mean/2, 3*mean/2)`, so the mean is `mean`.
    fn next(&mut self, mean: u32) -> u32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        let span = u64::from(mean).max(2);
        let low = span / 2;
        (low + self.state % span).max(1) as u32
    }
}

/// One measured site: how many samples landed there and how long they covered.
#[derive(Debug, Default, Clone, Copy)]
pub struct Site {
    pub samples: u64,
    pub nanos: u64,
}

/// The sampler, owned by the [`Debugger`](crate::debugger::Debugger) so that arming
/// it costs an ordinary run nothing — the dispatch loop's `self.debug.is_some()`
/// branch is already paid for.
#[derive(Debug)]
pub struct Profiler {
    /// Whether CPU sampling is armed, as opposed to allocation paths alone.
    cpu: bool,
    /// MEAN ops per sample; the actual period is jittered around it.
    interval: u32,
    /// Ops left until the next sample.
    tick: u32,
    /// Breaks lock-step with a periodic program — see [`Jitter`].
    jitter: Jitter,
    /// When the previous sample was taken — the start of the interval being credited.
    last: Instant,
    /// When sampling began, for the "over N seconds" banner.
    started: Instant,
    pub samples: u64,
    /// Wall-clock nanoseconds actually accounted for by samples.
    pub nanos: u64,
    /// Self time by bytecode position. Keyed by `pc` rather than by function so a
    /// sample taken outside any frame still lands somewhere, and so the line-level
    /// view comes out of the same data as the function-level one.
    sites: HashMap<u32, Site>,
    /// Time by call path — innermost [`PATH_DEPTH`] frames, innermost last.
    paths: HashMap<Vec<u32>, Site>,
    /// Paths dropped after [`MAX_PATHS`], so the report can say how many.
    pub paths_dropped: u64,

    // ── arc C: paths to ALLOCATIONS ──────────────────────────────────────────────
    /// Sample one allocating op in this many; 0 when allocation paths are off.
    alloc_interval: u32,
    alloc_tick: u32,
    /// Allocating ops seen, and how many of them were sampled.
    pub alloc_events: u64,
    pub alloc_sampled: u64,
    /// `(site pc, call path) → (sampled allocations, stores)`.
    alloc_paths: HashMap<(u32, Vec<u32>), (u64, u64)>,
}

impl Profiler {
    /// Arm from the environment: `LOFT_PROFILE` (CPU, `=<n>` sets the op interval)
    /// and `LOFT_ALLOC_PATHS` (arc C, `=<n>` sets the allocation sample rate).
    /// `None` when neither is set, which is the only thing an ordinary run pays.
    #[must_use]
    pub fn from_env() -> Option<Profiler> {
        let cpu = std::env::var_os("LOFT_PROFILE").is_some();
        let alloc = std::env::var_os("LOFT_ALLOC_PATHS").is_some();
        if !cpu && !alloc {
            return None;
        }
        let now = Instant::now();
        let interval = env_count("LOFT_PROFILE", 1024).max(1);
        let alloc_interval = if alloc {
            env_count("LOFT_ALLOC_PATHS", 16).max(1)
        } else {
            0
        };
        let mut jitter = Jitter::new();
        let first_tick = jitter.next(interval);
        let first_alloc_tick = if alloc {
            jitter.next(alloc_interval)
        } else {
            0
        };
        Some(Profiler {
            cpu,
            interval,
            // A sample on the very first op would credit it the whole of start-up.
            tick: first_tick,
            jitter,
            last: now,
            started: now,
            samples: 0,
            nanos: 0,
            sites: HashMap::new(),
            paths: HashMap::new(),
            paths_dropped: 0,
            alloc_interval,
            alloc_tick: first_alloc_tick,
            alloc_events: 0,
            alloc_sampled: 0,
            alloc_paths: HashMap::new(),
        })
    }

    /// Whether CPU sampling is armed (as opposed to allocation paths alone).
    #[must_use]
    pub fn cpu_armed(&self) -> bool {
        self.cpu
    }

    /// Whether allocation-path capture is armed.
    #[must_use]
    pub fn alloc_armed(&self) -> bool {
        self.alloc_interval > 0
    }

    /// The mean number of allocating ops per captured path.
    #[must_use]
    pub fn alloc_interval(&self) -> u32 {
        self.alloc_interval
    }

    /// One op executed. Returns `true` when this op is a sample point — the caller
    /// then hands over the position and the live call stack via [`Self::record`].
    ///
    /// Kept branch-cheap on purpose: this runs once per op of a profiled run.
    #[inline]
    pub fn tick(&mut self) -> bool {
        self.tick -= 1;
        if self.tick != 0 {
            return false;
        }
        self.tick = self.jitter.next(self.interval);
        true
    }

    /// Credit the interval since the previous sample to `pc` and to the call path
    /// `frames` (outermost first, as `State` keeps it).
    pub fn record(&mut self, pc: u32, frames: &[u32]) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_nanos() as u64;
        self.last = now;
        self.samples += 1;
        self.nanos += dt;
        let e = self.sites.entry(pc).or_default();
        e.samples += 1;
        e.nanos += dt;
        let key = tail(frames);
        if let Some(p) = self.paths.get_mut(&key) {
            p.samples += 1;
            p.nanos += dt;
        } else if self.paths.len() < MAX_PATHS {
            self.paths.insert(
                key,
                Site {
                    samples: 1,
                    nanos: dt,
                },
            );
        } else {
            self.paths_dropped += 1;
        }
    }

    /// arc C — an op allocated `stores` stores; sample one in `alloc_interval` and
    /// record the path that reached it.
    ///
    /// Sampled rather than exhaustive because a frame vector per allocation is orders
    /// of magnitude more than the one `u32` the cheap `created_at` stamp costs, and
    /// that stamp has to stay the default.
    pub fn record_alloc(&mut self, pc: u32, frames: &[u32], stores: u64) {
        self.alloc_events += 1;
        self.alloc_tick -= 1;
        if self.alloc_tick != 0 {
            return;
        }
        self.alloc_tick = self.jitter.next(self.alloc_interval);
        self.alloc_sampled += 1;
        let key = (pc, tail(frames));
        if let Some(e) = self.alloc_paths.get_mut(&key) {
            e.0 += 1;
            e.1 += stores;
        } else if self.alloc_paths.len() < MAX_PATHS {
            self.alloc_paths.insert(key, (1, stores));
        } else {
            self.paths_dropped += 1;
        }
    }

    /// Wall-clock time the profiler was armed for.
    #[must_use]
    pub fn elapsed_ns(&self) -> u64 {
        self.started.elapsed().as_nanos() as u64
    }

    /// Self time per bytecode position, hottest first.
    #[must_use]
    pub fn sites_ranked(&self) -> Vec<(u32, Site)> {
        let mut v: Vec<(u32, Site)> = self.sites.iter().map(|(&pc, &s)| (pc, s)).collect();
        v.sort_by_key(|&(pc, s)| (std::cmp::Reverse(s.nanos), pc));
        v
    }

    /// Time per call path, hottest first.
    #[must_use]
    pub fn paths_ranked(&self) -> Vec<(Vec<u32>, Site)> {
        let mut v: Vec<(Vec<u32>, Site)> =
            self.paths.iter().map(|(k, &s)| (k.clone(), s)).collect();
        v.sort_by(|a, b| b.1.nanos.cmp(&a.1.nanos).then_with(|| a.0.cmp(&b.0)));
        v
    }

    /// Sampled allocations per `(site, path)`, most allocations first.
    #[must_use]
    pub fn alloc_paths_ranked(&self) -> Vec<((u32, Vec<u32>), (u64, u64))> {
        let mut v: Vec<((u32, Vec<u32>), (u64, u64))> = self
            .alloc_paths
            .iter()
            .map(|(k, &s)| (k.clone(), s))
            .collect();
        v.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
        v
    }
}

/// The innermost [`PATH_DEPTH`] frames of `frames`, innermost last.
fn tail(frames: &[u32]) -> Vec<u32> {
    let start = frames.len().saturating_sub(PATH_DEPTH);
    frames[start..].to_vec()
}

/// `NAME=<n>` as a count, or `default` when the variable is set without a usable
/// number.  A typo must not silently pick a rate nobody asked for, so an
/// unparseable value is reported once.
fn env_count(name: &str, default: u32) -> u32 {
    let Ok(v) = std::env::var(name) else {
        return default;
    };
    let t = v.trim();
    if t.is_empty() || t == "1" || t.eq_ignore_ascii_case("yes") || t.eq_ignore_ascii_case("on") {
        return default;
    }
    match t.parse::<u32>() {
        Ok(n) if n > 0 => n,
        _ => {
            eprintln!("loft: {name}='{v}' is not a sample interval — using {default}");
            default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_innermost_frames_of_a_deep_chain() {
        let deep: Vec<u32> = (0..40).collect();
        let t = tail(&deep);
        assert_eq!(
            t.len(),
            PATH_DEPTH,
            "a deep chain is truncated, not dropped"
        );
        assert_eq!(*t.last().unwrap(), 39, "the innermost frame is kept");
        assert_eq!(
            t[0], 32,
            "…and the outermost of the kept window is next to it"
        );
    }

    #[test]
    fn a_short_chain_is_kept_whole() {
        assert_eq!(tail(&[7, 8]), vec![7, 8]);
        assert!(tail(&[]).is_empty());
    }

    /// The defect the arc C oracle caught: a fixed period only ever samples one phase
    /// of a periodic program.  Here the program alternates phase every 2 events and
    /// the mean period is 16 — a fixed sampler lands on the same phase every time and
    /// reports 100 % / 0 %.  The jittered one has to see both.
    #[test]
    fn jitter_breaks_lock_step_with_a_periodic_program() {
        let mut j = Jitter::new();
        let (mut phase_a, mut phase_b) = (0u32, 0u32);
        let (mut event, mut next) = (0u64, u64::from(j.next(16)));
        while event < 100_000 {
            event += 1;
            if event == next {
                // The program's own period: 2 events per iteration, one path taken on
                // every tenth iteration.
                if (event / 2) % 10 == 0 {
                    phase_b += 1;
                } else {
                    phase_a += 1;
                }
                next = event + u64::from(j.next(16));
            }
        }
        assert!(
            phase_a > 0 && phase_b > 0,
            "both phases must be sampled ({phase_a} / {phase_b})"
        );
        // ~1 in 10 of the program's iterations is the rare phase.
        let share = f64::from(phase_b) * 100.0 / f64::from(phase_a + phase_b);
        assert!(
            (4.0..18.0).contains(&share),
            "the rare phase should land near its true 10 % share, not at 0 or 50 (got {share:.1} %)"
        );
    }

    /// The mean must survive the jitter, or every reported rate is wrong by a
    /// constant factor.
    #[test]
    fn jitter_keeps_the_mean_it_was_given() {
        let mut j = Jitter::new();
        let n = 20_000u64;
        let total: u64 = (0..n).map(|_| u64::from(j.next(1024))).sum();
        #[allow(clippy::cast_precision_loss)]
        let mean = total as f64 / n as f64;
        assert!(
            (1000.0..1050.0).contains(&mean),
            "mean period drifted from 1024 to {mean:.1}"
        );
    }
}
