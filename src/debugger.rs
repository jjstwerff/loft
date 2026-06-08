// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN15 debugger — breakpoint registry + captured-frame records.
//!
//! The interpreter's execute loop (`src/state/mod.rs`) consults the optional
//! [`Debugger`] attached to a [`State`](crate::state::State): when the program
//! counter reaches a registered offset it pauses and captures the live frame as a
//! [`BreakHit`].  This is the tracer-bullet slice — record the frame and continue;
//! suspending into a REPL-at-frame and stepping build on the same hook.
//!
//! When a `State` has no debugger (`debug == None`, the normal case) the only
//! per-op cost is one `Option::is_some` branch — there is already a per-op
//! `crash_report` store in that loop, so this is in the noise.

use std::collections::HashSet;

/// The four @PLN15 F step verbs, driving
/// [`State::debug_step`](crate::state::State::debug_step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMode {
    /// Pause at the next source line, **descending into** any call on the way.
    Into,
    /// Run any call on the current line to completion, then pause at the next line
    /// in the same (or a shallower) frame — **over** the call.
    Over,
    /// Run to the current function's return and pause in the caller — **out**.
    Out,
    /// Run to the next breakpoint (or program end) — **continue**.
    Continue,
}

/// A frame captured at a breakpoint hit.
#[derive(Debug, Clone)]
pub struct BreakHit {
    /// The function the breakpoint sits in (user name, no `n_` prefix).
    pub function: String,
    /// The in-scope variables at the pause, each rendered to loft source
    /// (`("n", "42")`).  The slice captures arguments (live at fn entry); a later
    /// slice that breaks mid-body adds the locals assigned so far.
    pub locals: Vec<(String, String)>,
}

/// Debug state attached to a [`State`](crate::state::State) while debugging.
/// Absent on normal runs.
#[derive(Debug, Default)]
pub struct Debugger {
    /// Bytecode offsets that trigger a pause when execution reaches them.
    breakpoints: HashSet<u32>,
    /// Frames captured at each hit, in hit order (record-and-continue mode).
    pub hits: Vec<BreakHit>,
    /// **Stepping mode**: when set, a breakpoint *suspends* execution (the loop
    /// returns to the driver) instead of recording-and-continuing — so a value can
    /// be edited and `resume`d.  Off by default (record-and-continue).
    pub stepping: bool,
    /// The frame captured at the current suspension (stepping mode); `None` while
    /// running.  The driver reads it, optionally writes a value back to the live
    /// frame, and calls `State::resume`.
    pub paused: Option<BreakHit>,
}

impl Debugger {
    /// Register a bytecode offset as a breakpoint.
    pub fn add_offset(&mut self, offset: u32) {
        self.breakpoints.insert(offset);
    }

    /// Whether `offset` is a registered breakpoint.
    #[must_use]
    pub fn is_breakpoint(&self, offset: u32) -> bool {
        self.breakpoints.contains(&offset)
    }
}
