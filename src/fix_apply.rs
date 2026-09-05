// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I75 — Diagnostics collector
//
//! @PLN131 steps 3–4 — applying a fix, and checking one before it is offered.
//!
//! The plan's claim is that loft can do what an IDE quick-fix historically could not:
//! **run its own advice**. The compiler holds the analysis that raised the diagnostic, so a
//! candidate rewrite can be applied to an in-memory copy and the analysis re-run — turning
//! "this fix is sound" from an assertion into a measurement.
//!
//! Two things are deliberately NOT here.
//!
//! **The program is never run.** The plan asks for a behaviour comparison across both
//! backends, and that would mean executing the user's code as a side effect of their asking
//! what to write — code that may write files, take a network turn, or not terminate.
//! Verification is therefore static: the diagnostic must disappear and nothing new may
//! appear. That is the half that can be checked without acting on the author's behalf.
//!
//! **Nothing is applied without being asked.** `--verify` reports; `--apply` writes, and
//! only `Mechanical` fixes, because an unattended run has nobody to affirm a condition.

use crate::diagnostics::{DiagEntry, Diagnostics, Edit, Fix, FixKind};

/// What checking a fix against the analysis concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Applied cleanly and the diagnostic is gone, with no new error in its place.
    Clears,
    /// Applied, but the diagnostic is still there — the rewrite does not answer it.
    Remains,
    /// Applied, and the result is worse: an error or a warning the original source did not
    /// have.
    Breaks,
    /// Nothing to check — the fix spells no edit, so there is nothing to apply.
    Unspellable,
    /// It clears, and it is still yours to accept: a `Conditional` fix rests on something
    /// only the author can affirm, so an unattended run never writes it however well it
    /// verifies. Reported so a reader knows the rewrite WORKS and the judgement remains.
    NeedsYou,
}

impl Verdict {
    /// The one-word tag `--verify` prints after a fix line.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Verdict::Clears => "verified",
            Verdict::Remains => "UNVERIFIED (the diagnostic remains)",
            Verdict::Breaks => "REJECTED (the rewrite introduces an error or a warning)",
            Verdict::Unspellable => "not applicable (no edit)",
            Verdict::NeedsYou => "verified — yours to accept (see the condition)",
        }
    }
}

/// Apply `edits` to `source`, returning the rewritten text.
///
/// Applied back-to-front so an earlier edit's span is still valid when it is reached — the
/// standard reason a batch of edits is not applied in reading order. Ties break on the
/// longer span first, so an insertion at the same point as a replacement lands inside the
/// text it belongs to rather than beside it.
///
/// An edit whose span does not exist in `source` is SKIPPED, not clamped: a fix that cannot
/// find its own line has been computed against a different buffer, and writing it anywhere
/// would corrupt the file it was aimed at.
#[must_use]
pub fn apply_edits(source: &str, edits: &[Edit]) -> String {
    let mut lines: Vec<String> = source.split('\n').map(str::to_string).collect();
    let mut ordered: Vec<&Edit> = edits.iter().collect();
    ordered.sort_by(|a, b| {
        b.line
            .cmp(&a.line)
            .then(b.col.cmp(&a.col))
            .then(b.len.cmp(&a.len))
    });
    for e in ordered {
        let Some(line) = lines.get_mut(e.line.saturating_sub(1) as usize) else {
            continue;
        };
        let start = (e.col.saturating_sub(1)) as usize;
        let end = start + e.len as usize;
        if start > line.len() || end > line.len() || !line.is_char_boundary(start) {
            continue;
        }
        if e.len > 0 && !line.is_char_boundary(end) {
            continue;
        }
        // A pure DELETION absorbs one space in front of it, so removing a trailing clause
        // does not leave `t.name ;` behind.  Only when what FOLLOWS is punctuation or the
        // line's end — deleting between two words has to keep the space that separates
        // them.  loft#1003's `?? <default>` is the first deletion edit; every other one is
        // an insertion or a rename, where this cannot fire.
        let mut start = start;
        if e.text.is_empty() && e.len > 0 && start > 0 {
            let rest = line[end..].trim_start_matches('\r');
            let follows_word = rest
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
            if line.as_bytes()[start - 1] == b' ' && !follows_word {
                start -= 1;
            }
        }
        line.replace_range(start..end, &e.text);
    }
    lines.join("\n")
}

/// Every `(entry, fix)` pair in `diags` that spells an edit, in report order.
fn spelled(diags: &Diagnostics) -> Vec<(&DiagEntry, &Fix)> {
    diags
        .entries()
        .iter()
        .flat_map(|e| e.fixes.iter().map(move |f| (e, f)))
        .filter(|(_, f)| f.edit.is_some())
        .collect()
}

/// The messages of `d` that GATE — errors and warnings, the tier where ignoring one can
/// produce a wrong result — as a set-ish sorted list for before/after comparison.  Advice
/// is below the line: a rewrite that earns a deprecation note or a cost note has not made
/// the program wrong.  A rewrite that earns a WARNING has: `x: integer = "5" as integer?`
/// compiles where `as integer` did not, and stores a null into `x` on a bad parse, which
/// `(N-Store)` reports as exactly the warning this list now carries.
fn reported_of(d: &Diagnostics) -> Vec<String> {
    let mut v: Vec<String> = d
        .entries()
        .iter()
        .filter(|e| e.level >= crate::diagnostics::Level::Warning)
        .map(|e| format!("{}:{}:{}", e.code.unwrap_or(""), e.line, e.message))
        .collect();
    v.sort();
    v
}

/// @PLN131 step 3 — check one fix by APPLYING it and re-running the analysis.
///
/// Returns [`Verdict::Clears`] only when the coded diagnostic this fix hangs off is gone
/// from the re-analysis AND no error or warning appeared that the original did not have.
/// Both halves are needed: a rewrite that silences one error by causing another is not a
/// fix, and that is exactly the failure a pattern-matched suggestion makes.
///
/// `code` is matched rather than the message, because prose is free to change and the code
/// is the frozen identity — the reason @PLN131 built the code index first.
#[must_use]
pub fn verify_fix(
    source: &str,
    name: &str,
    stdlib_dir: &str,
    before: &Diagnostics,
    entry: &DiagEntry,
    fix: &Fix,
) -> Verdict {
    let Some(edit) = &fix.edit else {
        return Verdict::Unspellable;
    };
    let rewritten = apply_edits(source, std::slice::from_ref(edit));
    if rewritten == source {
        return Verdict::Unspellable;
    }
    // The BEFORE parse is re-run for its reach, not for its diagnostics: `before` was
    // produced by the caller and may have come from a parse whose reach is unknown here.
    let (before_rerun, before_reached) = crate::lsp::diagnose_reach(source, name, stdlib_dir);
    let (after, after_reached) = crate::lsp::diagnose_reach(&rewritten, name, stdlib_dir);

    // A new error or warning only counts against the fix when the two parses are COMPARABLE.
    //
    // `parse_source` returns early when pass 1 errors, so a truncated parse reports no
    // pass-2 diagnostic at all — casts, shifts, most semantic lints. Fixing the pass-1
    // blocker lets the next parse reach them, and a plain set-difference reads every one as
    // damage the rewrite did. Measured: an unescaped brace hid a bad cast three lines
    // BELOW it and another two lines ABOVE, which is what rules out judging this by
    // position — the mechanism is the phase, not the line.
    //
    // So the comparison is only made when the original got as far as the rewrite did.
    // Where it did not, the fix is judged on its own diagnostic alone: it cleared the
    // blocker, and what the deeper pass then finds was always there. That is also what a
    // person does — fix the syntax error, then read the type errors.
    if before_reached || !after_reached {
        let was = reported_of(before);
        for e in reported_of(&after) {
            if !was.contains(&e) {
                return Verdict::Breaks;
            }
        }
    }
    // Did THIS instance clear — not "did every instance of this code vanish".
    //
    // Asking whether ANY diagnostic with this code remains makes two instances mask each
    // other: a file with two redundant `??`s verified NEITHER fix, because whichever one
    // was applied, the other still reported the same code. That is not a hypothetical
    // — it is what an ordinary file looks like, and it made the first warning-level fix
    // `loft fix` could reach (loft#1003) inapplicable in practice as soon as a second one
    // existed.
    //
    // A COUNT is the position-independent way to ask it: a fix that clears its own
    // diagnostic lowers the tally by one, whichever instance it was. Positions cannot be
    // compared directly — a single-line deletion shifts every later diagnostic on that
    // same line. The count is taken from the re-run BEFORE parse rather than the caller's
    // `before`, so both sides come from the same settings and the same reach.
    //
    // Conservative in the one direction that matters: a rewrite that clears one instance
    // and introduces another nets zero and reads as `Remains`, which refuses rather than
    // writes.
    let tally = |d: &Diagnostics| {
        d.entries()
            .iter()
            .filter(|e| e.code.is_some() && e.code == entry.code)
            .count()
    };
    if tally(&after) >= tally(&before_rerun) {
        Verdict::Remains
    } else {
        Verdict::Clears
    }
}

/// One line of `--apply`'s report: what was written, or why it was not.
pub struct Applied {
    pub title: String,
    pub line: u32,
    pub verdict: Verdict,
    pub written: bool,
}

/// @PLN131 step 4 — apply every MECHANICAL fix that verifies, returning the new source and
/// a per-fix report.
///
/// Three gates, and each rejects a different way of being wrong:
///
/// 1. **`Mechanical` only.** A conditional fix is correct only if a condition holds that
///    the compiler cannot evaluate, so an unattended run has nobody to affirm it. They are
///    reported, never written.
/// 2. **Spells an edit.** A fix that knows the rewrite but not where it goes is not
///    applicable, however right it is.
/// 3. **Verifies.** Every candidate is applied to an in-memory copy and the analysis re-run
///    FIRST. A fix that does not clear its own diagnostic, or that introduces a new error,
///    is not written — which is the whole point of doing step 3 before step 4.
///
/// Fixes are verified one at a time against the ORIGINAL source, then applied together.
/// That is sound here because each candidate's span is disjoint, and it stays honest
/// because the caller re-runs the analysis on the written file.
#[must_use]
pub fn apply_fixes(
    source: &str,
    name: &str,
    stdlib_dir: &str,
    diags: &Diagnostics,
) -> (String, Vec<Applied>) {
    let mut report = Vec::new();
    let mut edits: Vec<Edit> = Vec::new();
    for (entry, fix) in spelled(diags) {
        let mechanical = fix.kind == FixKind::Mechanical;
        // Every spelled fix is checked, both tiers. A conditional one is not written, but a
        // reader still needs to know whether the rewrite WORKS — "you must decide" and "it
        // would not have compiled anyway" are different answers, and collapsing them into
        // one label hides the second.
        let checked = verify_fix(source, name, stdlib_dir, diags, entry, fix);
        let verdict = match (mechanical, checked) {
            (false, Verdict::Clears) => Verdict::NeedsYou,
            (_, v) => v,
        };
        // Every candidate was verified against the ORIGINAL source, so two edits may only be
        // applied together while their spans are disjoint — otherwise the second lands on
        // text the first already rewrote and neither was checked against what it hits.  The
        // condition above claimed disjointness rather than enforcing it, and a diagnostic
        // reported twice at one position is enough to break it: `{  }` -> `[]` applied twice
        // deleted the four characters AFTER the replacement, eating the enclosing `}`.
        // Overlap is a runner-level fact, so it is settled here rather than asked of each
        // emit site (loft#1003).
        let overlaps = |e: &Edit| {
            edits.iter().any(|p: &Edit| {
                p.line == e.line && p.col < e.col + e.len.max(1) && e.col < p.col + p.len.max(1)
            })
        };
        let written = mechanical && verdict == Verdict::Clears;
        if written
            && let Some(e) = &fix.edit
            && !overlaps(e)
        {
            edits.push(e.clone());
        }
        report.push(Applied {
            title: fix.title.clone(),
            line: entry.line,
            verdict,
            written,
        });
    }
    (apply_edits(source, &edits), report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(line: u32, col: u32, len: u32, text: &str) -> Edit {
        Edit {
            line,
            col,
            len,
            text: text.to_string(),
        }
    }

    #[test]
    fn an_insertion_lands_at_its_column() {
        assert_eq!(
            apply_edits("as integer;", &[edit(1, 11, 0, "?")]),
            "as integer?;"
        );
    }

    #[test]
    fn a_replacement_covers_its_span() {
        assert_eq!(apply_edits("a } b", &[edit(1, 3, 1, "}}")]), "a }} b");
    }

    /// Two edits on one line must both land where they were computed — which they only do
    /// if the later one is applied first.
    #[test]
    fn edits_on_one_line_do_not_shift_each_other() {
        let out = apply_edits("a } b } c", &[edit(1, 3, 1, "}}"), edit(1, 7, 1, "}}")]);
        assert_eq!(out, "a }} b }} c");
    }

    /// A span past the end of its line is skipped rather than clamped: an edit that cannot
    /// find its own text was computed against a different buffer, and writing it somewhere
    /// else corrupts the file it was aimed at.
    #[test]
    fn an_out_of_range_span_is_skipped_not_clamped() {
        assert_eq!(apply_edits("short", &[edit(1, 99, 1, "x")]), "short");
        assert_eq!(apply_edits("short", &[edit(9, 1, 1, "x")]), "short");
    }

    #[test]
    fn a_multibyte_line_keeps_its_characters() {
        // `ä` is two bytes; an edit at a non-boundary must not split it.
        assert_eq!(apply_edits("äb", &[edit(1, 2, 1, "x")]), "äb");
    }

    /// Two edits over the same characters must not both be written: each was verified
    /// against the ORIGINAL source, so the second lands on text the first already replaced.
    /// The shape that found this was one diagnostic reported twice at one position —
    /// `{  }` -> `[]` applied twice deleted the four characters after the replacement and
    /// ate the enclosing `}` (loft#1003).
    #[test]
    fn overlapping_edits_do_not_both_apply() {
        // Both spans cover column 1; applying both would rewrite the rewrite.
        let out = apply_edits("{  } x", &[edit(1, 1, 4, "[]")]);
        assert_eq!(out, "[] x", "one edit lands as computed");
        // Measured, not assumed: applying the SAME span twice is not idempotent.  The second
        // edit rewrites columns 1-4 of the already-rewritten line — `[] x` — and takes the
        // ` x` with it.  `apply_edits` is deliberately left this way (it applies what it is
        // given); the guard belongs in `apply_fixes`, the only place that knows each
        // candidate was verified against the original.
        let twice = apply_edits("{  } x", &[edit(1, 1, 4, "[]"), edit(1, 1, 4, "[]")]);
        assert_eq!(
            twice, "[]",
            "an overlapping pair corrupts — which is why apply_fixes drops the second"
        );
    }

    /// A `Conditional` fix is never written unattended, however applicable it looks.
    ///
    /// No shipped conditional fix spells an edit yet, so nothing in the corpus exercises
    /// this — which is exactly why it is pinned here rather than left to a live example.
    /// The rule protects the first one somebody adds: `--apply` has nobody to affirm the
    /// condition, and a click that affirms nothing is how a suggestions feature becomes a
    /// bug generator with good intentions.
    #[test]
    fn a_conditional_fix_is_never_written_unattended() {
        let mut d = Diagnostics::new();
        // Uncoded deliberately: a `Conditional` fix is rejected before verification ever
        // looks at the code, so inventing one here would only teach `e1_code_set`'s scanner
        // that this module declares a diagnostic it does not emit.
        d.add_at(
            crate::diagnostics::Level::Warning,
            "something is off",
            "buf.loft",
            1,
            1,
        );
        d.fix_last(Fix {
            kind: FixKind::Conditional,
            title: "rewrite it".to_string(),
            condition: Some("you meant something else".to_string()),
            edit: Some(edit(1, 1, 2, "XX")),
            concept: "test",
            concept_ref: "@F1",
        });
        let src = "abcdef";
        let (out, report) = apply_fixes(src, "buf.loft", "", &d);
        assert_eq!(out, src, "a conditional fix must not reach the file");
        assert_eq!(report.len(), 1);
        assert!(
            !report[0].written,
            "a conditional fix must not be reported as written"
        );
    }
}
