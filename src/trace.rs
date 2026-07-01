// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I76 — Logger runtime

//! Loft trace points — env-var-gated, near-zero-overhead diagnostic
//! infrastructure for compile-time debugging across the parser, type
//! system, code generator, and runtime support modules.
//!
//! # Why
//!
//! The fix-discover-fix cadence repeatedly hit the same pattern: add
//! `eprintln!("DBG: …")` at a suspected vantage point, build
//! (~30-60s), run, observe, iterate, then remove the prints before
//! commit and rebuild.  Each session re-invented the same diagnostic
//! vantage points.  The trace points seeded with this module make
//! recurring vantage points permanent, version-controlled, and silent
//! unless explicitly enabled.
//!
//! # Selective adoption
//!
//! Not every temporary `eprintln!` should become a trace point.  When
//! debugging surfaces a useful vantage:
//! - **One-off** (specific to this bug, unlikely to recur): keep the
//!   `eprintln!` local, remove before commit, no trace point.
//! - **Recurring** (likely to be revisited for similar bugs in the
//!   same subsystem): convert to a `loft_trace!(category, …)` call.
//!   Permanent; future sessions enable the category and observe.
//!
//! New categories are added when a real use case appears, not
//! pre-emptively.  See § Adding a category below.
//!
//! # Usage
//!
//! ```bash
//! # Enable specific categories:
//! LOFT_TRACE=call,field cargo run --release --bin loft -- --interpret /tmp/foo.loft
//!
//! # Enable everything currently registered:
//! LOFT_TRACE=all cargo run ...
//!
//! # During tests:
//! LOFT_TRACE=generic cargo test --release --test issues my_test -- --nocapture
//! ```
//!
//! # Currently-registered categories
//!
//! Parser-time:
//! - `call` — function call dispatch in `Parser::call` (post-find_fn).
//! - `field` — field/method access entry in `Parser::field`.
//! - `generic` — generic instantiation: `predict_generic_return_type`
//!   + `try_generic_instantiation`.
//! - `match` — match arm parsing: `expect_match_arm_arrow` + scalar
//!   match arm-loop.
//!
//! Other modules (database, codegen, type system) MAY add categories
//! when a real recurring debug surface justifies one.  No
//! pre-emptive seeding — categories that exist must have a use case.
//!
//! # Adding a category
//!
//! 1. Add a `pub <name>: bool` field to `TraceCategories`.
//! 2. Add a `<name>: all || cats.contains("<name>"),` line to the
//!    initialiser.
//! 3. Add a macro arm (mirror the existing pattern).
//! 4. Update the "currently-registered categories" list above.
//! 5. Document in the loft-test skill if non-trivial.
//!
//! # Cost
//!
//! When `LOFT_TRACE` is unset (default), each trace point compiles
//! to: one bool load + one branch.  The branch predictor learns
//! "always-false" within a few calls and predicts perfectly
//! thereafter; effective overhead is below measurement noise.
//!
//! Format-string arguments are evaluated only when the branch fires.
//! `loft_trace!(call, "name={} state={:?}", name,
//! self.expensive_state())` does not pay the `expensive_state()`
//! cost when tracing is disabled.

use std::collections::HashSet;
use std::sync::LazyLock;

/// Per-category trace switches.  Each field is a single bool;
/// initialised once at first use of any trace point (via `LazyLock`).
/// After initialisation, every trace check is a single bool load.
///
/// Clippy `struct_excessive_bools` is silenced here — this struct's
/// purpose IS to be a flat collection of bool flags (one per
/// category), and using a bitflags-style enum would defeat the
/// goal of branch-predictor-friendly direct-field access.
#[allow(clippy::struct_excessive_bools)]
pub struct TraceCategories {
    pub call: bool,
    pub field: bool,
    pub generic: bool,
    pub match_arm: bool,
}

/// Single global trace-categories struct.  Initialised on first use
/// from the `LOFT_TRACE` environment variable.  Format:
/// comma-separated category names (e.g. `LOFT_TRACE=call,field`)
/// or `all` to enable every registered category.  An empty / unset
/// variable yields all-false (the default fast path).
pub static TRACE: LazyLock<TraceCategories> = LazyLock::new(|| {
    let s = std::env::var("LOFT_TRACE").unwrap_or_default();
    let cats: HashSet<String> = s
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(String::from)
        .collect();
    let all = cats.contains("all");
    TraceCategories {
        call: all || cats.contains("call"),
        field: all || cats.contains("field"),
        generic: all || cats.contains("generic"),
        match_arm: all || cats.contains("match"),
    }
});

/// Write a trace line, gated on the named category.
///
/// Categories are typed at the macro arm level — `loft_trace!(call,
/// "…")` produces a compile error if `call` is misspelled, unlike a
/// string-keyed lookup which would silently disable a typo'd trace.
///
/// Each arm expands to:
///
/// ```text
/// if $crate::trace::TRACE.<cat> {
///     eprintln!(concat!("[<cat>] ", $fmt), $args...);
/// }
/// ```
///
/// The format-string prefix is built at compile time via `concat!`,
/// so disabled trace points have zero runtime overhead beyond the
/// bool load + branch.  Format-string arguments are evaluated only
/// when the branch fires.
#[macro_export]
macro_rules! loft_trace {
    (call, $fmt:literal $($arg:tt)*) => {
        if $crate::trace::TRACE.call {
            eprintln!(concat!("[call] ", $fmt) $($arg)*);
        }
    };
    (field, $fmt:literal $($arg:tt)*) => {
        if $crate::trace::TRACE.field {
            eprintln!(concat!("[field] ", $fmt) $($arg)*);
        }
    };
    (generic, $fmt:literal $($arg:tt)*) => {
        if $crate::trace::TRACE.generic {
            eprintln!(concat!("[generic] ", $fmt) $($arg)*);
        }
    };
    (match_arm, $fmt:literal $($arg:tt)*) => {
        if $crate::trace::TRACE.match_arm {
            eprintln!(concat!("[match] ", $fmt) $($arg)*);
        }
    };
}
