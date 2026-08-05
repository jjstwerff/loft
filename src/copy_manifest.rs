// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN130 — the compile-time manifest of every deep copy the EMITTERS write.
//!
//! The copy diagnostic classifies copies it finds in the **IR**, during `scopes::check`.
//! Both code generators invent more copies *after* that — a whole-record bind `b = a` and a
//! call-return bind are minted at emission time and appear in no IR the analysis ever walks
//! — so they reach no diagnostic even in principle. Measured: with every copy flag on, a
//! program that provably deep-copies reports `none — every structure copy is a move, a
//! literal, or already borrowed` (loft#774, @PLN130 probes 10 and 11).
//!
//! This module closes that by recording what was actually EMITTED, so the guard can prove
//! the diagnostic covers it. The manifest is written where the copy is written, which is the
//! one place that cannot be wrong about whether a copy exists.
//!
//! **Compile-time only.** Nothing here reaches a compiled program: no op changes, no runtime
//! bookkeeping, no cost in the generated binary. A deep copy's *size* is deliberately out of
//! scope — it depends on runtime content (`copy_claims` walks nested vectors and texts), and
//! loft reports **where** a copy happens, never **how much** it moved. That is the same
//! bargain rustc makes with `.clone()`.

use crate::use_analysis::verdicts_for;
use std::cell::RefCell;

/// Which emitter wrote the copy — named so an uncovered site points straight at the code
/// that produced it rather than at a line number alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Interpreter `gen_set_first_ref_var_copy` — a whole-record bind `b = a`.
    InterpRecordBind,
    /// Interpreter first-bind from a call whose return is not a fresh store.
    InterpCallReturn,
    /// Interpreter tuple-destructuring bind.
    InterpTupleBind,
    /// Native whole-record bind (`generation::dispatch`, the `Value::Var(src)` arm).
    NativeRecordBind,
    /// Native call-return bind. Emits a runtime adopt-or-copy branch, so this is a *may
    /// copy* site — still one the diagnostic must account for.
    NativeCallReturn,
}

impl Origin {
    /// Backend that owns this emitter, for grouping a report.
    #[must_use]
    pub fn backend(self) -> &'static str {
        match self {
            Self::InterpRecordBind | Self::InterpCallReturn | Self::InterpTupleBind => "interpret",
            Self::NativeRecordBind | Self::NativeCallReturn => "native",
        }
    }

    /// Whether the emitted code copies unconditionally, or branches at runtime and copies
    /// only on one arm. Reported so a MAY-copy is never read as a definite one — native's
    /// call-return arm emits an adopt-or-copy guard on store identity.
    #[must_use]
    pub fn always_copies(self) -> bool {
        !matches!(self, Self::NativeCallReturn)
    }
}

/// One emitted deep copy.
#[derive(Clone, Debug)]
pub struct CopySite {
    /// Enclosing function definition.
    pub def_nr: u32,
    /// Destination variable the copy fills.
    pub var: u16,
    /// The copied record's type number.
    pub type_nr: u16,
    pub origin: Origin,
}

thread_local! {
    /// Emitted-copy sites for the current compilation. A `Vec` (not a set): two emitters
    /// legitimately write two copies for one binding, and collapsing them would hide one.
    static SITES: RefCell<Vec<CopySite>> = const { RefCell::new(Vec::new()) };
}

/// Record a deep copy at the moment it is emitted.
///
/// Call this from the branch that actually WRITES the copy — never from the function that
/// decides whether to. `gen_set_first_ref_var_copy` returns early on a last-use move, and a
/// site recorded before that branch would claim a copy that never existed.
pub fn record(def_nr: u32, var: u16, type_nr: u16, origin: Origin) {
    // Gated at the RECORD, not just the report: with the guard off this is one cached bool
    // per emitted copy and nothing is accumulated, so compiling a large program does not
    // build a manifest nobody reads. (One cached env read — `OnceLock`.)
    if !crate::keys::copy_manifest_enabled() {
        return;
    }
    SITES.with(|s| {
        s.borrow_mut().push(CopySite {
            def_nr,
            var,
            type_nr,
            origin,
        });
    });
}

/// Every copy emitted so far this compilation.
#[must_use]
pub fn sites() -> Vec<CopySite> {
    SITES.with(|s| s.borrow().clone())
}

/// Drop the manifest (between compilations in one process — the test harness, the REPL).
pub fn clear() {
    SITES.with(|s| s.borrow_mut().clear());
}

/// The guard: emitted copies the copy diagnostic produced no verdict for.
///
/// A site is COVERED when the analysis classified the same destination binding in the same
/// function — that is the fact the user-facing report is built from, so a site the analysis
/// never rowed can never be reported however the rendering changes.
///
/// Conservative in the safe direction: a covered site is dropped even if the report would
/// later filter it out for being `Implicit` or `Internal`, because those are *deliberate*
/// silences. Only a copy the analysis never saw at all is a blind spot.
#[must_use]
pub fn uncovered(data: &crate::data::Data) -> Vec<CopySite> {
    let mut out = Vec::new();
    for site in sites() {
        let classified = verdicts_for(data, site.def_nr)
            .into_iter()
            .any(|r| r.var_nr == site.var);
        if !classified {
            out.push(site);
        }
    }
    out
}

/// Render the guard's finding. Returns the number of uncovered sites so a caller can gate.
///
/// Reports the SITE and the TYPE — never a size. A copy is deep, so a flat record size is
/// not its cost, and a 12-byte-looking copy that moves a megabyte teaches the wrong thing.
///
/// **DRAINS the manifest**, so the two generators can each report the sites they wrote
/// without the second call repeating the first's. Interpreter codegen finishes at
/// `compile::byte_code`; native generation runs later, and in an `introspect` run both
/// happen in one process.
pub fn report(data: &crate::data::Data) -> usize {
    if !crate::keys::copy_manifest_enabled() {
        clear();
        return 0;
    }
    let bad = uncovered(data);
    clear();
    if bad.is_empty() {
        return 0;
    }
    eprintln!(
        "loft copy-manifest guard — {} emitted {} no diagnostic accounts for",
        bad.len(),
        if bad.len() == 1 { "copy" } else { "copies" }
    );
    for s in &bad {
        let def = data.def(s.def_nr);
        let var_name = def.variables.name(s.var);
        // A `_`-prefixed binding is compiler-generated: the author never wrote it, so it is
        // OUR worklist, not theirs. Still reported — the guard's audience is the compiler —
        // but marked, so a stdlib internal is not mistaken for a user-visible copy.
        let origin_note = if s.origin.always_copies() {
            "copies"
        } else {
            "may copy"
        };
        let who = if def.variables.is_compiler_generated(s.var) {
            " (compiler-generated binding)"
        } else {
            ""
        };
        eprintln!(
            "  {} [{:?}]  fn {}  binding `{}`  {} {}{}",
            s.origin.backend(),
            s.origin,
            def.name,
            var_name,
            origin_note,
            data.type_name_str(def.variables.tp(s.var)),
            who,
        );
    }
    bad.len()
}
