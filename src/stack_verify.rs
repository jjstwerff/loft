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

/// The width and kind a value was written at, packed into a byte's tag.
///
/// @PLN154 phase 2.  A stack slot is a byte offset and nothing more, so *"is the value I am
/// reading the value that was written here"* has no answer unless the write records what it
/// was.  Two facts answer it and both fit small: a FAMILY (is this a handle, a text, a
/// float, or a plain word) and a WIDTH.  Together they separate the mismatches the
/// monomorph-layout class produces — a 12-byte `DbRef` sentinel written into a slot a
/// monomorph reads as an 8-byte integer (loft#1028), a placeholder record defaulted into a
/// slot that turned out to be a float (loft#1016) — from a slot read at the width it was
/// written.
///
/// A FAMILY rather than a full type id because the tag has to be computed on the write path
/// and compared on the read path: hashing a `TypeId` costs more than the comparison it
/// feeds, and the distinctions that matter here are exactly the ones a family makes.
#[must_use]
pub fn kind_of<T: 'static>() -> u16 {
    use std::any::TypeId;
    // A raw byte BLOCK is not a value and carries no type: the fn-ref slot is written as
    // `[MaybeUninit<u8>; 20]` and read back as an `i64` and a `DbRef`, which is a composite
    // read and not a disagreement.  The block spelling is the fact — a type whose name
    // starts with `[` is an array of bytes, written whole by whoever assembled it — so it
    // joins the raw spans on [`OPAQUE`] rather than earning a family of its own.
    if std::any::type_name::<T>().starts_with('[') {
        return OPAQUE;
    }
    let t = TypeId::of::<T>();
    let family: u16 = if t == TypeId::of::<crate::keys::DbRef>() {
        1
    } else if t == TypeId::of::<crate::keys::Str>() {
        2
    } else if t == TypeId::of::<f64>() || t == TypeId::of::<f32>() {
        3
    } else if t == TypeId::of::<String>() {
        4
    } else {
        0
    };
    (family << 8) | (std::mem::size_of::<T>() as u16 & 0xFF)
}

/// The width half of a [`kind_of`] tag.
#[must_use]
pub fn kind_width(kind: u16) -> u32 {
    u32::from(kind & 0xFF)
}

/// A tag that matches every read: the raw byte moves — a coroutine frame restored from its
/// own snapshot, a worker's stack overlaid with its parent's — carry no type, and inventing
/// one for them would report the restore rather than describe it.
pub const OPAQUE: u16 = u16::MAX;

/// Name a family for a report.  The width is printed beside it, so this says only what the
/// width cannot.
#[must_use]
pub fn kind_name(kind: u16) -> &'static str {
    match kind >> 8 {
        1 => "handle",
        2 => "text",
        3 => "float",
        4 => "string",
        _ => "word",
    }
}

/// What the shadow knows about one read's span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    /// Not one byte of the span was written — the slot has no occupant.  Phase 1's finding.
    Unwritten,
    /// A value was written at this base at a different width or kind.  Phase 2's finding.
    Mismatch { wrote: u16 },
    /// A disagreement that is not a handle crossing a value: a scalar narrower than its
    /// stepped slot, a null sentinel read as the type it stands for, a composite slot read
    /// one field at a time.  Counted, never reported.
    Partial,
    /// Every byte was written, at this width and kind.
    Written,
}

/// How many reports to print before falling silent.  A read inside a loop reports every
/// iteration and the first few already name the site; the summary keeps the total.
const MAX_REPORTS: usize = 20;

#[derive(Default)]
struct Verify {
    /// Reads of a span no write had tagged.
    uninit: u64,
    /// Reads that disagreed with the write about width or kind without a handle crossing a
    /// value — the admitted puns, kept as a number because the frame's composite slots make
    /// them the common case rather than a finding.
    partial: u64,
    /// Reads where a handle crossed a value.  Phase 2's finding.
    mismatch: u64,
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

/// Report a read of a stack slot written at another width or kind.
#[expect(
    clippy::too_many_arguments,
    reason = "one report line, one argument each — the same shape as strict_store_violation"
)]
pub fn report_mismatch(
    what: &str,
    ty: &str,
    at: u32,
    width: usize,
    wrote: u16,
    interior: bool,
    pc: u32,
    line: u32,
    op: u16,
) {
    let ty = ty.rsplit("::").next().unwrap_or(ty);
    let print = with_verify(|v| {
        v.mismatch += 1;
        if !v.seen.insert((pc, at)) || v.printed >= MAX_REPORTS {
            return false;
        }
        v.printed += 1;
        true
    });
    if print {
        let wrote_desc = if interior {
            "it starts inside a value written earlier".to_string()
        } else {
            format!(
                "the value written here is {} {} byte(s) wide",
                kind_name(wrote),
                kind_width(wrote)
            )
        };
        crate::loft_eprintln!(
            "stack verify: {what}<{ty}> reads {width} byte(s) at frame offset {at} — \
             {wrote_desc} [pc={pc} op={op} line={line}]"
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

/// How many findings this run has seen, of either state.
#[must_use]
pub fn violations() -> u64 {
    with_verify(|v| v.uninit + v.mismatch)
}

/// Print the verdict.  Called at program exit when armed.
///
/// It prints on a clean run too: silence is the result this instrument exists to make
/// trustworthy, and a detector that says nothing when it found nothing is
/// indistinguishable from one that never ran.
pub fn report() {
    let (uninit, mismatch, sites, partial, out_of_range) = with_verify(|v| {
        (
            v.uninit,
            v.mismatch,
            v.seen.len(),
            v.partial,
            v.out_of_range,
        )
    });
    if uninit == 0 && mismatch == 0 {
        crate::loft_eprintln!("stack verify: no uninitialised or mistyped stack reads");
    } else {
        crate::loft_eprintln!(
            "stack verify: {uninit} uninitialised and {mismatch} mistyped stack read(s), \
             {sites} distinct site(s)"
        );
    }
    if partial > 0 {
        crate::loft_eprintln!(
            "  {partial} read(s) disagreed with the write about width or kind without a \
             handle crossing a value (the admitted puns)"
        );
    }
    if out_of_range > 0 {
        crate::loft_eprintln!(
            "  WARNING: {out_of_range} checked read(s) fell outside the shadow — the count \
             above is a floor"
        );
    }
}
