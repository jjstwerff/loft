// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN53 F2 — keyed-container program generator (F2.1).
//!
//! Generates valid-by-construction, self-checking loft programs that exercise
//! the schema-coupled keyed collections (`hash` / `sorted` / `index`), the way
//! `program_ownership` does for the ownership grammar. Reified as a library so
//! `cargo test` drives generation over the spec space on stable; the libfuzzer
//! target (F2.5) is a shim over [`generate_keyed`].
//!
//! Contract: `doc/claude/plans/53-program-level-fuzzing/F2-DESIGN.md`. Keys are
//! DISTINCT by construction (drawn from an indexed pool), so one invariant is
//! uniform across all three types: the collection is exactly the KEY→VALUE map
//! the op sequence defines — population equals the surviving-key count, every
//! surviving key looks up its value, removed/absent keys look up `null`, and
//! `for` visits exactly the survivors in the declared key order.
//!
//! The brittleness cure (F2-DESIGN.md § re-assertion sites): the survivor set
//! is computed ONCE and drives both the emitted statements and the baked
//! assertions — one model, no drift.

use crate::compile::byte_code;
use crate::fuzz_oracle::stdlib;
use crate::parser::Parser;
use crate::scopes;
use crate::state::State;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

/// Which keyed collection a spec targets. The grammar is parameterized per type
/// (F2-DESIGN.md § over-unification guard): `index` has a multi-part key and no
/// `.len()`; all three share the distinct-key map invariant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Hash,
    Sorted,
    Index,
}

/// A generated keyed-collection program: insert `n_keys` distinct keys, then
/// remove the indices in `remove`.
#[derive(Clone, Debug)]
pub struct KeyedSpec {
    pub kind: Kind,
    pub n_keys: u32,
    pub remove: Vec<u32>,
}

/// The distinct value stamped for key index `i` (injective, so a lookup pins
/// the exact key→value mapping).
fn value_of(i: u32) -> i64 {
    i64::from(i) * 7 + 1
}

/// Zero-padded key text for `hash`/`sorted`, so lexicographic order equals index
/// order (keeps the expected iteration order trivial to compute).
fn key_text(i: u32) -> String {
    format!("k{i:03}")
}

/// Build a valid-by-construction, self-checking program for `spec`.
pub fn generate_keyed(spec: &KeyedSpec) -> String {
    let n = spec.n_keys.clamp(1, 20);
    let removed: BTreeSet<u32> = spec.remove.iter().copied().filter(|&i| i < n).collect();
    // ONE model: the survivor set drives both the emitted ops and the baked
    // assertions — there is no second copy to drift out of sync.
    let survivors: Vec<u32> = (0..n).filter(|i| !removed.contains(i)).collect();
    let pop = survivors.len();

    let mut s = String::new();
    match spec.kind {
        Kind::Index => {
            s.push_str("struct E { n: integer, k: text, v: integer }\n");
            s.push_str("struct C { m: index<E[n, k]> }\n");
        }
        Kind::Hash => {
            s.push_str("struct E { k: text, v: integer }\n");
            s.push_str("struct C { m: hash<E[k]> }\n");
        }
        Kind::Sorted => {
            s.push_str("struct E { k: text, v: integer }\n");
            s.push_str("struct C { m: sorted<E[k]> }\n");
        }
    }
    s.push_str("fn main() {\n  c = C { m: [] };\n");

    // Inserts — all n keys, in index order.
    for i in 0..n {
        match spec.kind {
            Kind::Index => {
                let _ = writeln!(s, "  c.m += [E{{n:{i}, k:\"x\", v:{}}}];", value_of(i));
            }
            _ => {
                let _ = writeln!(
                    s,
                    "  c.m += [E{{k:\"{}\", v:{}}}];",
                    key_text(i),
                    value_of(i)
                );
            }
        }
    }
    // Removes.
    for &i in &removed {
        match spec.kind {
            Kind::Index => {
                let _ = writeln!(s, "  c.m[{i}, \"x\"] = null;");
            }
            _ => {
                let _ = writeln!(s, "  c.m[\"{}\"] = null;", key_text(i));
            }
        }
    }

    // Population + iteration order (a `;`-separated string, unambiguous for any n).
    s.push_str("  order: text = \"\";\n  cnt: integer = 0;\n");
    match spec.kind {
        Kind::Index => s.push_str("  for e in c.m { cnt += 1; order += \"{e.n};\"; }\n"),
        _ => s.push_str("  for e in c.m { cnt += 1; order += e.k + \";\"; }\n"),
    }
    let _ = writeln!(s, "  assert(cnt == {pop}, \"pop {{cnt}}\");");
    // `index` has no `.len()`; hash/sorted get the extra len() check too.
    if spec.kind != Kind::Index {
        let _ = writeln!(s, "  assert(c.m.len() == {pop}, \"len {{c.m.len()}}\");");
    }

    // Per-key lookups: survivors return their value, removed keys return null.
    for i in 0..n {
        let present = !removed.contains(&i);
        match (spec.kind, present) {
            (Kind::Index, true) => {
                let _ = writeln!(
                    s,
                    "  assert(c.m[{i}, \"x\"].v == {}, \"lk{i}\");",
                    value_of(i)
                );
            }
            (Kind::Index, false) => {
                let _ = writeln!(s, "  assert(!c.m[{i}, \"x\"], \"rm{i}\");");
            }
            (_, true) => {
                let _ = writeln!(
                    s,
                    "  assert(c.m[\"{}\"].v == {}, \"lk{i}\");",
                    key_text(i),
                    value_of(i)
                );
            }
            (_, false) => {
                let _ = writeln!(s, "  assert(!c.m[\"{}\"], \"rm{i}\");", key_text(i));
            }
        }
    }

    // Iteration order: survivors in index order (= declared key order for all
    // three types under this key scheme).
    let expected: String = survivors
        .iter()
        .map(|&i| match spec.kind {
            Kind::Index => format!("{i};"),
            _ => format!("{};", key_text(i)),
        })
        .collect();
    let _ = writeln!(s, "  assert(order == \"{expected}\", \"order {{order}}\");");
    s.push_str("}\n");
    s
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Run one generated program in-process. `Ok(())` means it compiled, ran, and
/// every self-check passed. A compiler panic propagates untouched — the
/// observer (the F2.1 test / libfuzzer) records it.
///
/// # Errors
/// Returns `Err` when the program is rejected at parse (a GENERATOR bug —
/// valid-by-construction should always compile) or when a self-check assertion
/// / runtime fault fires (`had_fatal`), which is a real FINDING.
pub fn check_generated(src: &str) -> Result<(), String> {
    let (data, db) = stdlib();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;

    let tmp = std::env::temp_dir().join(format!(
        "loft_f2_{}_{}.loft",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::write(&tmp, src).is_err() {
        return Ok(()); // an I/O error is a harness condition, not a finding
    }
    let path = tmp.to_string_lossy().to_string();
    p.parse(&path, false);
    let _ = std::fs::remove_file(&tmp);

    let rejected = !p.diagnostics.is_empty()
        && p.diagnostics
            .lines()
            .iter()
            .any(|l| !l.starts_with("Warning:") && !l.starts_with("Debug:"));
    if rejected {
        return Err(format!(
            "GENERATOR BUG (rejected): {}\n--- program ---\n{src}",
            p.diagnostics.lines().join(" | ")
        ));
    }

    let mut data = p.data;
    let database = p.database;
    scopes::check(&mut data);
    let mut state = State::new(database);
    byte_code(&mut state, &mut data);
    state.execute("main", &data);
    while state.database.frame_yield {
        state.resume();
    }
    if state.database.had_fatal {
        let msg = state
            .database
            .runtime_error
            .as_ref()
            .map_or_else(|| "runtime fault".to_string(), |e| e.message.clone());
        return Err(format!(
            "FINDING (self-check failed): {msg}\n--- program ---\n{src}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    /// A representative set of remove patterns for `n` keys: none, first, last,
    /// a middle key, all-but-first, and all.
    fn remove_patterns(n: u32) -> Vec<Vec<u32>> {
        let mut v = vec![vec![], vec![0]];
        if n > 1 {
            v.push(vec![n - 1]);
            v.push(vec![n / 2]);
            v.push((1..n).collect()); // all but first
            v.push((0..n).collect()); // all
        }
        v
    }

    /// F2.1 exit: every generated program compiles, runs, and passes its
    /// self-checks on the interpreter. A rejection is a generator bug; a failed
    /// self-check or a compiler panic is a finding.
    #[test]
    fn generated_programs_compile_run_and_selfcheck() {
        let mut failures: Vec<String> = Vec::new();
        let mut total = 0;
        for kind in [Kind::Hash, Kind::Sorted, Kind::Index] {
            for &n in &[1u32, 2, 5, 10] {
                for remove in remove_patterns(n) {
                    let spec = KeyedSpec {
                        kind,
                        n_keys: n,
                        remove,
                    };
                    let src = generate_keyed(&spec);
                    total += 1;
                    match catch_unwind(AssertUnwindSafe(|| check_generated(&src))) {
                        Ok(Ok(())) => {}
                        Ok(Err(msg)) => {
                            failures.push(format!(
                                "{kind:?} n={n}: {}",
                                msg.lines().next().unwrap_or("")
                            ));
                        }
                        Err(_) => failures.push(format!("{kind:?} n={n}: PANIC (compiler ICE)")),
                    }
                }
            }
        }
        eprintln!(
            "F2.1: generated + ran {total} keyed programs, {} failure(s)",
            failures.len()
        );
        assert!(
            total > 30,
            "too few specs generated ({total}) — enumeration broken"
        );
        assert!(
            failures.is_empty(),
            "{}/{total} generated programs failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
