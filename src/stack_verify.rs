// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I67 — Opcode implementations

//! `LOFT_VERIFY_STACK=1` — read a stack slot that no path wrote, and hear about it.
//!
//! @PLN154 phase 1.  The interpreter's frame is a raw byte store: nothing beside the
//! bytes says whether a slot holds a value at all.  A path that yields nothing therefore
//! leaves the consumer reading whatever the slot held before, and the program answers a
//! plausible number nobody computed — loft#1386 is the measured case, where a
//! value-position `match` arm that produced no value made the whole expression answer
//! `null`, silently, on both backends.
//!
//! The shadow is one tag byte per stack byte, carried by the stack [`Store`] itself
//! (`init_shadow`), so it grows and moves with the buffer it describes rather than being
//! kept in step by hand:
//!
//! * a write through [`Store::addr_mut`](crate::store::Store::addr_mut) TAGS the bytes it
//!   covers — phase 0 measured that 32 of the 33 sites that write the stack reach it
//!   there, where only 74.5 % of the bytes arrive through the typed accessor;
//! * a byte-for-byte move ([`Stores::copy_block`](crate::database::Stores::copy_block) and
//!   friends) carries the tags with the bytes, so a returned value stays as written — or
//!   as unwritten — as the callee left it;
//! * a slot that leaves the live frame loses its tag, because the next occupant of that
//!   offset inherits nothing from the last;
//! * and a read at [`State::get_stack`](crate::state::State::get_stack) /
//!   [`get_var`](crate::state::State::get_var) reports an untagged span.
//!
//! **Tag low, check high** — the low-level reader `Store::addr` is deliberately NOT
//! checked, because the debugger, the frame renderer and the anomaly scanner read
//! uninitialised slots for a living and a check there would report the diagnostics as
//! defects.
//!
//! Off by default and off in every release path: when the shadow is not armed the whole
//! mechanism is one length load and a not-taken branch per store write.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// Is the shadow armed?  `LOFT_VERIFY_STACK=1`.
#[must_use]
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_VERIFY_STACK").is_ok_and(|v| v != "0"))
}

/// `LOFT_VERIFY_STACK_INJECT=1` — the positive control.
///
/// A detector that cannot fire and a clean corpus look identical, so the detector ships
/// with the switch that makes it fire on a program known to be correct: writes stop
/// tagging, every checked read therefore sees an untagged span, and a silent run under
/// this means the CHECK is not reached — not that the program is clean.
///
/// It suppresses the tag at the write hook only.  The moves and the kills still run, so
/// what the control proves is that the check path is live and the report renders; it does
/// not exercise the tag-follows-the-value half, which is what the corpus sweep is for.
#[must_use]
pub fn inject() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_VERIFY_STACK_INJECT").is_ok_and(|v| v != "0"))
}

/// What the shadow knows about one read's span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    /// Not one byte of the span was written — the slot has no occupant.  Phase 1's finding.
    Unwritten,
    /// The span starts on written bytes and runs off the end of them: the occupant is
    /// narrower than the read.  Every sub-word scalar on the eval stack is this, because a
    /// slot is stepped to eight bytes and a `boolean` writes one, so it is COUNTED and not
    /// reported — the width question is phase 2's, and answering it here would report the
    /// language's own stepped slots.
    Partial,
    /// Every byte was written.
    Written,
}

/// How many reports to print before falling silent.  A read inside a loop reports every
/// iteration and the first few already name the site; the summary keeps the total.
const MAX_REPORTS: usize = 20;

#[derive(Default)]
struct Verify {
    /// Reads of a span no write had tagged.
    uninit: u64,
    /// Reads whose occupant was narrower than the read — a phase-2 lead, kept as a number
    /// because the corpus's stepped slots make it the common case rather than a finding.
    partial: u64,
    /// Reads whose span fell outside the shadow — a plumbing fault in the shadow itself,
    /// not a finding about the program.  Counted rather than reported, so it cannot hide
    /// behind a quiet run.
    out_of_range: u64,
    /// One report per `(bytecode position, frame offset)`, so a loop says it once.
    seen: HashSet<(u32, u32)>,
    printed: usize,
}

/// PROCESS-wide, not thread-local: a `par` arm and a placed-library worker each run on
/// their own `State` with their own stack store and their own shadow, and a per-thread
/// tally would print their findings and then close with a main-thread verdict of "no
/// uninitialised stack reads" — the one line a sweep reads.  Measured on
/// `1054-parallel-block-arms-run.loft`, which reported four and summed to none.
static VERIFY: Mutex<Option<Verify>> = Mutex::new(None);

fn with_verify<R>(f: impl FnOnce(&mut Verify) -> R) -> R {
    let mut guard = VERIFY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(guard.get_or_insert_with(Verify::default))
}

/// Report a read of a stack span nothing had written.
///
/// `at` is the frame offset the read landed on and `pc` the bytecode position of the
/// operator that made it — together they are the identity a repeat is deduped by.
pub fn report_uninit(what: &str, ty: &str, at: u32, width: usize, pc: u32, line: u32, op: u16) {
    // `std::any::type_name` spells a path (`loft::keys::Str`); the leaf is what names the
    // slot's width and kind to a reader, and the path is noise in a report that already
    // carries an offset and a pc.
    let ty = ty.rsplit("::").next().unwrap_or(ty);
    let print = with_verify(|v| {
        v.uninit += 1;
        if !v.seen.insert((pc, at)) || v.printed >= MAX_REPORTS {
            return false;
        }
        v.printed += 1;
        true
    });
    if print {
        crate::loft_eprintln!(
            "stack verify: {what}<{ty}> reads {width} uninitialised byte(s) at frame offset \
             {at} — nothing wrote this slot on the path that reached here \
             [pc={pc} op={op} line={line}]"
        );
    }
}

/// Note a checked read whose occupant is narrower than the read itself.
pub fn note_partial() {
    with_verify(|v| v.partial += 1);
}

/// Note a checked read whose span lies outside the shadow.
pub fn note_out_of_range() {
    with_verify(|v| v.out_of_range += 1);
}

/// How many uninitialised reads this run has seen.
#[must_use]
pub fn violations() -> u64 {
    with_verify(|v| v.uninit)
}

/// Print the verdict.  Called at program exit when armed.
///
/// It prints on a clean run too: silence is the result this instrument exists to make
/// trustworthy, and a detector that says nothing when it found nothing is
/// indistinguishable from one that never ran.
pub fn report() {
    let (uninit, sites, partial, out_of_range) =
        with_verify(|v| (v.uninit, v.seen.len(), v.partial, v.out_of_range));
    if uninit == 0 {
        crate::loft_eprintln!("stack verify: no uninitialised stack reads");
    } else {
        crate::loft_eprintln!(
            "stack verify: {uninit} uninitialised stack read(s), {sites} distinct site(s)"
        );
    }
    if partial > 0 {
        crate::loft_eprintln!(
            "  {partial} read(s) wider than the value written at the slot (stepped-slot \
             padding, and phase 2's question)"
        );
    }
    if out_of_range > 0 {
        crate::loft_eprintln!(
            "  WARNING: {out_of_range} checked read(s) fell outside the shadow — the count \
             above is a floor"
        );
    }
}
