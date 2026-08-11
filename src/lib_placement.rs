// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN119 arc A — out-of-process libraries: placement as policy, the store as the wire.

//! Where a library RUNS, declared as deployment policy rather than written into
//! the source that calls it.
//!
//! A library marks itself `[library] placement = "process"` in its `loft.toml`
//! and its consumers keep calling the same typed `pub fn` surface they already
//! use. The call travels over a shared memory-mapped file to a worker process
//! that holds the library, and comes back as an ordinary loft value. Nothing in
//! the calling source names a process, a pipe, or a command line — which is the
//! whole point, and also the invariant the parity gate checks:
//!
//! > A call to a library is indistinguishable — in type, effect,
//! > ownership/lifetime, and error behaviour — from the same call in-process.
//!
//! # Why the handshake spins before it sleeps
//!
//! The obvious wire is "write the request, `FUTEX_WAKE` the worker, sleep until
//! it answers". Measured, that costs ~4 µs a call — 77× a native in-process
//! call and 246× an interpreted one (@PLN119 Q4). At that price placement is
//! *not* policy, because moving a library would change the shape of the code
//! calling it: callers would have to batch.
//!
//! So each side spins briefly before sleeping, and — the load-bearing half —
//! publishes a **sleeper count** so the other side pays the `FUTEX_WAKE`
//! syscall only when someone is genuinely asleep. Back-to-back calls then never
//! reach a syscall at all (~130 ns, 2.4× a native in-process call) while an idle
//! worker still gives its core back. The naive version is a performance bug,
//! not a simpler variant of this one.
//!
//! # What arc A carries, and what it does not
//!
//! Arc A ships the declaration, the attach handshake, and calls whose arguments
//! and return are **scalars or text** — the subset [`crate::host::Value`]
//! already marshals, reused deliberately so the boundary has one vocabulary
//! rather than two. A signature outside that subset is refused at load time with
//! a message naming arc B, and the function stays in-process; it never becomes a
//! call that fails later.
//!
//! Structs, vectors and references crossing the boundary need the arena marshal
//! and the ownership rules (@PLN119 arcs B and C) and are not implemented here.
//! The handshake already exchanges the store-numbering base those arcs translate
//! against, so attaching does not have to change when they land.
//!
//! # Platform
//!
//! The declaration is portable — every platform parses `placement` and every
//! platform agrees on what it means. The *transport* is Linux-only, because the
//! handshake is built on `futex`. Elsewhere a process-placed library runs
//! in-process, which by the invariant above is the same program; set
//! `LOFT_REQUIRE_PLACEMENT=1` to make the lost isolation an error instead.

/// Where a library's functions execute.
///
/// Parsed from `[library] placement` in the library's own `loft.toml` — it is
/// the library author's declaration, not the consumer's, because the library is
/// what knows whether it is safe and worthwhile to isolate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    /// Ordinary in-process calls. The default, and what every library that
    /// declares nothing gets.
    #[default]
    InProc,
    /// A worker process of this machine holds the library; calls cross a shared
    /// memory-mapped file.
    Process,
}

impl Placement {
    /// Read a `placement = "..."` manifest value.
    ///
    /// # Errors
    /// The spelling, when it is not one this version knows. A typo has to be
    /// loud: silently treating `"proces"` as in-process would hand back a
    /// program that runs correctly and isolates nothing, and the difference is
    /// invisible in the output.
    pub fn parse(s: &str) -> Result<Placement, String> {
        match s.trim() {
            "inproc" => Ok(Placement::InProc),
            "process" => Ok(Placement::Process),
            // `remote` is arc E. Name it as known-but-unbuilt rather than
            // unknown, so a library written against the finished plan gets an
            // accurate answer instead of "did you mean".
            "remote" => Err(
                "placement = \"remote\" is @PLN119 arc E and is not implemented yet; \
                 use \"process\" for a worker on this machine"
                    .to_string(),
            ),
            other => Err(format!(
                "unknown placement \"{other}\" — expected \"inproc\" or \"process\""
            )),
        }
    }

    /// Does this placement need a worker process?
    #[must_use]
    pub fn is_out_of_process(self) -> bool {
        self == Placement::Process
    }
}

/// The call arena — the shared store a struct or a vector crosses in, rather
/// than being re-encoded into a second wire vocabulary (@PLN119 arc B).
#[cfg(target_os = "linux")]
pub mod arena;

/// The Linux transport: the shared mapping, the spin-then-sleep handshake, the
/// frame codec, and the two ends of a call.  Gated because the handshake is
/// built on `futex`; see the Platform note above.
#[cfg(target_os = "linux")]
pub mod wire;

/// Routing a placed library's calls from the interpreter into its worker —
/// what makes the declaration take effect on an ordinary `use`.
#[cfg(target_os = "linux")]
pub mod dispatch;

#[cfg(target_os = "linux")]
pub use wire::{Wire, Worker, serve};
