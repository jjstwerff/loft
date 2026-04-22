// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Doc-hygiene regression guards.
//!
//! QUALITY.md Tier 4 item 10 calls out that INCONSISTENCIES.md,
//! PROBLEMS.md, and CAVEATS.md drift independently — resolved INCs
//! get a status block in their long-form entry but the Summary-by-
//! Severity tables keep listing them as open.  These tests lock the
//! invariant: every INC appearing as "Resolved as design point" must
//! NOT also appear in the Medium or Low severity tables.

use std::fs;

const DOC: &str = "doc/claude/INCONSISTENCIES.md";
const PROBLEMS: &str = "doc/claude/PROBLEMS.md";
const CAVEATS: &str = "doc/claude/CAVEATS.md";
const QUALITY: &str = "doc/claude/QUALITY.md";

/// The set of `#[ignore = "..."]` entries in `tests/issues.rs` must
/// match the committed baseline at `tests/ignored_tests.baseline`.
/// A drift typically means one of three things worth the author's
/// attention at review time:
///   * An ignored test just got its underlying fix landed — the
///     baseline entry should be deleted and the test unignored.
///   * A new ignored spec was added for a new QUALITY.md item —
///     the baseline should grow.
///   * The `#[ignore = "…"]` reason string was edited — baseline
///     should update.
///
/// Without this guard, silently-passing ignored tests and stale
/// reason strings accumulate invisibly (the failure mode that made
/// B5's documented symptom stale for weeks before anyone noticed).
/// Regenerate the baseline with
/// `python3 tests/dump_ignored_tests.py > tests/ignored_tests.baseline`.
#[test]
fn ignored_tests_baseline_is_current() {
    let src = fs::read_to_string("tests/issues.rs").expect("cannot read tests/issues.rs");
    let mut actual: Vec<(String, String)> = Vec::new();
    let lines: Vec<&str> = src.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("#[ignore") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let Some(start) = rest.find('"') else {
            continue;
        };
        let tail = &rest[start + 1..];
        let Some(end) = tail.rfind('"') else {
            continue;
        };
        let reason = tail[..end].replace("\\\"", "\"").replace("\\\\", "\\");
        // Scan forward up to 10 lines for the `fn NAME(` line.
        for next in lines.iter().skip(i + 1).take(10) {
            let nt = next.trim_start();
            if let Some(after_fn) = nt.strip_prefix("fn ")
                && let Some(paren) = after_fn.find('(')
            {
                actual.push((after_fn[..paren].to_string(), reason.clone()));
                break;
            }
        }
    }
    actual.sort();
    let baseline = fs::read_to_string("tests/ignored_tests.baseline")
        .expect("cannot read tests/ignored_tests.baseline");
    let mut expected: Vec<(String, String)> = Vec::new();
    for line in baseline.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let (name, reason) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("malformed baseline line (missing TAB): `{line}`"));
        expected.push((name.to_string(), reason.to_string()));
    }
    if actual != expected {
        let mut msg = String::from(
            "tests/issues.rs #[ignore] set drifted from tests/ignored_tests.baseline.\n",
        );
        let actual_set: std::collections::BTreeSet<_> = actual.iter().collect();
        let expected_set: std::collections::BTreeSet<_> = expected.iter().collect();
        for added in actual_set.difference(&expected_set) {
            msg += &format!("  + {}\t{}\n", added.0, added.1);
        }
        for removed in expected_set.difference(&actual_set) {
            msg += &format!("  - {}\t{}\n", removed.0, removed.1);
        }
        msg +=
            "Regenerate with: python3 tests/dump_ignored_tests.py > tests/ignored_tests.baseline";
        panic!("{msg}");
    }
}

/// Every `#[test]` attribute in a test file must be immediately
/// followed by either another `#[…]` attribute or a `fn`.  An
/// orphan `#[test]` — left after moving / reshaping a test block —
/// produces a `duplicate_macro_attributes` warning on the *next*
/// test's attribute and makes test output confusing (the real
/// test name shown is right, but the warning blames an innocent
/// line).  Misled the author into opening a QUALITY.md bug
/// about `code!()` once; this guard prevents it reoccurring.
#[test]
fn no_orphan_test_attributes_in_tests_issues_rs() {
    let path = "tests/issues.rs";
    let src = fs::read_to_string(path).unwrap_or_else(|_| panic!("cannot read {path}"));
    let lines: Vec<&str> = src.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed != "#[test]" {
            continue;
        }
        // Scan forward through blank lines, other attributes, and
        // `///` / `//` doc/comment lines until we find the next
        // non-trivial line.  It must be either another `#[…]`
        // attribute or a `fn` declaration.
        let next = lines
            .iter()
            .enumerate()
            .skip(i + 1)
            .find(|(_, l)| {
                let t = l.trim_start();
                !t.is_empty() && !t.starts_with("//")
            })
            .map(|(j, l)| (j, l.trim_start()));
        match next {
            Some((_, t))
                if t.starts_with("#[") || t.starts_with("fn ") || t.starts_with("pub fn ") => {}
            Some((j, t)) => panic!(
                "Orphan #[test] attribute at {path}:{} — next non-comment line is {path}:{} `{t}` (expected another `#[…]` or `fn`). Remove the stray attribute.",
                i + 1,
                j + 1
            ),
            None => panic!(
                "Orphan #[test] attribute at {path}:{} with no following declaration.",
                i + 1
            ),
        }
    }
}

fn read_doc() -> String {
    fs::read_to_string(DOC).unwrap_or_else(|_| panic!("cannot read {DOC}"))
}

/// QUALITY Tier 2 #4 — once the `cargo clippy --no-default-features
/// --all-targets -- -D warnings` gate goes green, CI must run it on
/// every push so the ratchet can't slip.  This guard reads the
/// `Makefile` `ci:` target and asserts the `--no-default-features`
/// clippy invocation is present.  Caught silently-regressed gates
/// before when the `--tests` variant was the only one listed.
#[test]
fn ci_target_runs_no_default_features_clippy() {
    let makefile = fs::read_to_string("Makefile").expect("cannot read Makefile");
    let needle = "cargo clippy --no-default-features --all-targets -- -D warnings";
    assert!(
        makefile.contains(needle),
        "Makefile `ci:` target must invoke `{needle}` so the --no-default-features clippy ratchet is enforced on every push.  See QUALITY.md Tier 2 #4."
    );
}

/// QUALITY Tier 1 #3 — `p122_long_running_struct_loop` was ignored
/// only because it takes ~10 min in debug and ~0.05 s in release,
/// not because the test itself was broken.  Closed 2026-04-14 by
/// switching the attribute to `#[cfg_attr(debug_assertions, ignore)]`
/// so release builds (CI's default) run it automatically.  This
/// guard locks the debug-only ignore form in place: a future edit
/// that reverts to plain `#[ignore]` would silently re-remove the
/// stress regression from CI.
#[test]
fn p122_long_running_struct_loop_is_cfg_attr_ignored_in_debug_only() {
    let src = fs::read_to_string("tests/issues.rs").expect("cannot read tests/issues.rs");
    let idx = src
        .find("fn p122_long_running_struct_loop")
        .expect("tests/issues.rs must define p122_long_running_struct_loop");
    // Look backwards ~30 lines to capture the attribute stack above the fn.
    let preamble_start = src[..idx]
        .rfind("#[test]")
        .unwrap_or(idx.saturating_sub(30 * 120));
    let preamble = &src[preamble_start..idx];
    assert!(
        preamble.contains("cfg_attr(")
            && preamble.contains("debug_assertions")
            && preamble.contains("ignore"),
        "p122_long_running_struct_loop must be gated with \
         `#[cfg_attr(debug_assertions, ignore = …)]` so CI (release) \
         runs it.  See QUALITY.md Tier 1 #3.  Preamble found:\n{preamble}"
    );
}

/// QUALITY Tier 3 #8 — the const-store mmap path is intentionally
/// deferred per [CONST_STORE.md § Phase B] (cache files are 5-10 KB;
/// mmap overhead exceeds memcpy savings at this size).  QUALITY.md
/// must reflect that decision, not ask for a benchmark the design
/// has ruled out.  This guard locks the two docs together: if
/// CONST_STORE.md stops saying "Deferred" next to Phase B, QUALITY.md
/// Tier 3 #8 should be re-opened at the same time; if QUALITY.md
/// loses the closure marker, CONST_STORE.md probably did too.
#[test]
fn quality_const_store_mmap_matches_const_store_md() {
    let cs = fs::read_to_string("doc/claude/CONST_STORE.md")
        .expect("cannot read doc/claude/CONST_STORE.md");
    assert!(
        cs.contains("**Phase B** (mmap): **Deferred.**"),
        "CONST_STORE.md must document Phase B (mmap) as **Deferred.** — if the design position has changed, re-open QUALITY.md Tier 3 #8 and update this guard."
    );

    let q = read_quality();
    // Locate the Tier 3 #8 block heading.  Items are numbered
    // `8. **...` at column 0; find the one that mentions const-store
    // mmap (not the P54-section "8." which is deeper-indented).
    let heading_start = q
        .find("\n8. **")
        .map(|i| i + 1)
        .expect("QUALITY.md must contain a top-level Tier 3 item `8. **...`");
    let block_end = q[heading_start..]
        .find("\n9. ")
        .map_or(q.len() - heading_start, |e| e)
        + heading_start;
    let block = &q[heading_start..block_end];
    assert!(
        block.contains("Const store mmap"),
        "QUALITY.md Tier 3 #8 should be about the const-store mmap path.  Heading found:\n{}",
        &block[..block.find('\n').unwrap_or(block.len())]
    );
    assert!(
        block.starts_with("8. **~~") || block.contains("**~~Const store mmap"),
        "QUALITY.md Tier 3 #8 must be struck-through (`~~Const store mmap…~~`) because CONST_STORE.md § Phase B is deferred-by-design.  Block:\n{block}"
    );
    assert!(
        block.contains("CONST_STORE.md"),
        "QUALITY.md Tier 3 #8 must reference CONST_STORE.md § Phase B as the source of the deferral decision.  Block:\n{block}"
    );
}

/// P54 / Q2 / Q3 / Q4 native-registration guard.  The existing
/// `tests/issues.rs::native_rs_functions_up_to_date` only checks
/// functions that carry a `#rust "..."` annotation in `default/`;
/// the P54 JSON family ships pure-native declarations like
/// `pub fn json_null() -> JsonValue;` with no `#rust` body, so
/// their wiring through `NATIVE_FNS` has no automated guard.  A
/// future edit that deletes an entry from `NATIVE_FNS` without
/// removing the loft declaration would leave the declaration
/// "declared but not implemented" — callers get a silent runtime
/// panic rather than a compile-time error.
///
/// This guard enumerates every `pub fn <name>(…) -> <T>;` header
/// in `default/06_json.loft` (body-less declarations = pure
/// natives) and asserts `"n_<name>"` appears as a key in the
/// `NATIVE_FNS` array in `src/native.rs`.  Covers every Q2 / Q3
/// / Q4 helper shipped on the P54 branch.
#[test]
fn p54_json_natives_registered_for_every_declaration() {
    let stdlib =
        fs::read_to_string("default/06_json.loft").expect("cannot read default/06_json.loft");
    let native = fs::read_to_string("src/native.rs").expect("cannot read src/native.rs");
    let mut missing: Vec<String> = Vec::new();
    // Walk lines; a body-less declaration is `pub fn <name>(…) -> <T>;`
    // ending in `;`.  Declarations with `{` start a loft-side
    // implementation (not a native), skip those.
    for line in stdlib.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("pub fn ") else {
            continue;
        };
        let Some(paren) = rest.find('(') else {
            continue;
        };
        let name = &rest[..paren];
        if name.is_empty() {
            continue;
        }
        // Must end with `;` on the same line (no body).  Multi-line
        // signatures aren't used in 06_json.loft today; if that
        // changes, extend this to join continuations before checking.
        if !rest.trim_end().ends_with(';') {
            continue;
        }
        let needle = format!("\"n_{name}\"");
        if !native.contains(&needle) {
            missing.push(name.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "default/06_json.loft declares {} pure-native fn(s) without a matching NATIVE_FNS entry in src/native.rs:\n  {}\n\
         For each missing name `foo`, add `(\"n_foo\", n_foo)` to NATIVE_FNS and implement `fn n_foo(...)`.  See QUALITY.md § P54 for the JSON native surface.",
        missing.len(),
        missing.join("\n  ")
    );
}

/// QUALITY P54 — `PROBLEMS.md § 54` documents the design contract
/// for the JSON surface to readers who haven't seen QUALITY.md.
/// Earlier drafts of `### 54.` carried `JObject { fields:
/// hash<JsonField[name]> }` (the originally-designed key-indexed
/// hash) but the implementation shipped with `vector<JsonField>`
/// — easier to materialise + frees as one unit with the parent
/// store.  The hash form is a 0.9.0 follow-up.
///
/// This guard pins the doc against the stdlib so a future spec
/// edit can't silently revert to the un-shipped hash form.  The
/// invariant: PROBLEMS.md's `### 54.` body must contain the
/// `vector<JsonField>` storage form, not `hash<JsonField[name]>`.
#[test]
fn problems_p54_jobject_layout_matches_stdlib() {
    let problems = fs::read_to_string("doc/claude/PROBLEMS.md").expect("cannot read PROBLEMS.md");
    let stdlib =
        fs::read_to_string("default/06_json.loft").expect("cannot read default/06_json.loft");

    // The stdlib's actual JObject layout — the source of truth.
    assert!(
        stdlib.contains("JObject { fields: vector<JsonField> }"),
        "default/06_json.loft no longer declares `JObject {{ fields: vector<JsonField> }}` — \
         this guard is pinned to that layout; if the storage form changed, update the guard."
    );

    // PROBLEMS.md must agree.
    assert!(
        problems.contains("vector<JsonField>"),
        "PROBLEMS.md § 54 must reference `vector<JsonField>` as the JObject storage form to \
         match `default/06_json.loft`.  An earlier draft used `hash<JsonField[name]>` (the \
         pre-shipping design).  Update PROBLEMS.md to match the actually-shipped layout, \
         keeping a note that the hash form is a 0.9.0 follow-up."
    );

    // And must NOT carry the obsolete hash form (only mention as historical context).
    let lines_with_hash: Vec<(usize, &str)> = problems
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("hash<JsonField[name]>"))
        .collect();
    // Allow exactly zero matches in a code-block context (the example).
    // Allow plain-prose mentions only as part of a "0.9.0 follow-up" sentence.
    for (i, line) in &lines_with_hash {
        let trimmed = line.trim_start();
        let in_code_block = trimmed.starts_with('|')
            || trimmed.starts_with("pub fn")
            || trimmed.starts_with("pub enum")
            || trimmed.starts_with("pub struct")
            || trimmed.starts_with("JObject")
            || trimmed.starts_with("JArray")
            || trimmed.starts_with("    ");
        let is_followup_note = line.to_ascii_lowercase().contains("follow-up")
            || line.to_ascii_lowercase().contains("0.9.0");
        assert!(
            !in_code_block || is_followup_note,
            "PROBLEMS.md L{}: `hash<JsonField[name]>` appears in code-block / declaration \
             context — should be `vector<JsonField>`.  Line: {}",
            i + 1,
            line
        );
    }
}

/// QUALITY P54 — the JSON stdlib + native doc-comments must not
/// contain stale "step 4" / "stub" / "forward-compatible" /
/// "lands when X" / "today returns JNull" language now that
/// step 4 is complete.  Earlier drafts of `default/06_json.loft`
/// and `src/native.rs` peppered the surface with forward-
/// compatible-stub callouts so callers could write the API shape
/// ahead of the implementation; once the implementations landed
/// those notes turned into wrong claims about today's behaviour.
///
/// This guard catches those references so they get rewritten
/// alongside the next big landing instead of misleading readers.
/// Both files are scanned because the same staleness shape
/// appeared on both sides of the loft/Rust boundary.
///
/// The check is intentionally narrow: only the strings that
/// appeared in pre-step-4 stub language.  General "step N"
/// references in module-level prose are still allowed (the
/// active-sprint headline at the top of the file points at the
/// roadmap legitimately).
#[test]
fn json_stdlib_has_no_stale_stub_language() {
    let stdlib =
        fs::read_to_string("default/06_json.loft").expect("cannot read default/06_json.loft");
    let native = fs::read_to_string("src/native.rs").expect("cannot read src/native.rs");
    let mut findings: Vec<(String, usize, String)> = Vec::new();
    let stale_phrases = [
        "forward-compatible stub",
        "forward-compat stub",
        "back from the stub",
        "until p54 step 4",
        "lands with p54 step 4",
        "pending a q4 follow-up",
        "step 3 (landing next commit)",
        "today returns an empty",
        "today this returns an empty",
        "today returns `jnull`",
        "non-empty input returns `jnull`",
        // src/native.rs-side variants
        "step 3 stub",
        "still returns 0 (arena materialisation",
        "stored on the call today but unused",
        "ahead of p54 step 4's container materialisation",
    ];
    for (file, src) in [
        ("default/06_json.loft", &stdlib),
        ("src/native.rs", &native),
    ] {
        // For src/native.rs, restrict the scan to the JSON-related
        // region so unrelated `///` comments aren't flagged.  The
        // JSON natives live between the `n_json_parse` start marker
        // and the trailing native-fns block.
        let (start, end) = if file == "src/native.rs" {
            let s = src.find("fn n_json_parse").unwrap_or(0);
            let e = src[s..]
                .find("// ===")
                .map(|off| s + off)
                .unwrap_or(src.len());
            (s, e)
        } else {
            (0, src.len())
        };
        let region = &src[start..end];
        // Map region byte offset back to absolute line numbers by
        // counting lines in the prefix.
        let line_offset = src[..start].lines().count();
        for (i, line) in region.lines().enumerate() {
            let trimmed = line.trim_start();
            // Both `///` (stdlib + Rust doc-comments) and `//!`
            // (Rust module-level) count.
            if !trimmed.starts_with("///") && !trimmed.starts_with("//!") {
                continue;
            }
            let payload = trimmed
                .trim_start_matches("///")
                .trim_start_matches("//!")
                .trim();
            let lower = payload.to_ascii_lowercase();
            for phrase in stale_phrases {
                if lower.contains(phrase) {
                    findings.push((file.to_string(), line_offset + i + 1, payload.to_string()));
                    break;
                }
            }
        }
    }
    assert!(
        findings.is_empty(),
        "JSON natives contain {} stale stub-language doc-comment line(s) (post-P54-step-4 — the implementations have landed, the comments need to flip to describe today's behaviour):\n  {}\n\
         For each finding, rewrite the doc-comment to describe what the function does TODAY (post-step-4 arena materialisation, post-Q2/Q3/Q4 ships) and remove the forward-compatible-stub callouts.",
        findings.len(),
        findings
            .iter()
            .map(|(f, n, p)| format!("{f}:{n}: {p}"))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// QUALITY Tier 3 #9 — `wasm32-unknown-unknown` without the `wasm`
/// host-bridge feature has no reachable filesystem.  `file().content()`
/// and `file().exists()` must return safe defaults (empty string /
/// NotExists) rather than depend on `std::fs` behaviour, which varies
/// by browser embedding.  This guard asserts both native implementations
/// carry an explicit `cfg(target_arch = "wasm32")` branch so a refactor
/// that collapses the feature-flag arms can't silently regress the
/// `--html` build target.
#[test]
fn wasm32_file_operations_have_explicit_stubs() {
    for (path, needle) in [
        ("src/state/io.rs", "target_arch = \"wasm32\""),
        ("src/database/io.rs", "target_arch = \"wasm32\""),
    ] {
        let src = fs::read_to_string(path).unwrap_or_else(|_| panic!("cannot read {path}"));
        assert!(
            src.contains(needle),
            "{path} must contain an explicit `{needle}` cfg branch that stubs filesystem operations on browser WASM.  See QUALITY.md Tier 3 #9."
        );
    }
}

/// QUALITY Tier 4 #11 — DESIGN_DECISIONS.md is the closed-by-decision
/// register.  Without a prominent cross-ref at the top of the docs
/// where new items naturally land (PLANNING.md for future work,
/// PROBLEMS.md for bug reports), the same declined proposals keep
/// coming back every quarter.  This guard asserts both files link
/// to DESIGN_DECISIONS.md near their top — inside the first ~80
/// lines where the header / intro lives, not buried in a
/// cross-references section at the bottom where nobody sees it.
#[test]
fn planning_and_problems_link_to_design_decisions() {
    for (path, label) in [
        ("doc/claude/PLANNING.md", "PLANNING.md"),
        ("doc/claude/PROBLEMS.md", "PROBLEMS.md"),
    ] {
        let src = fs::read_to_string(path).unwrap_or_else(|_| panic!("cannot read {path}"));
        let head: String = src.lines().take(80).collect::<Vec<_>>().join("\n");
        assert!(
            head.contains("DESIGN_DECISIONS.md"),
            "{label} must link to DESIGN_DECISIONS.md in its opening ~80 lines so contributors see the closed-by-decision register before adding a new entry.  See QUALITY.md Tier 4 #11."
        );
    }
}

/// QUALITY Tier 4 #12 — `make ship` is the canonical pre-push gate.
/// Four invariants must all be present in the recipe so users who
/// `make ship && git push` get the same guarantee the remote CI
/// applies.  If any step is dropped the ratchet silently weakens
/// (the exact scenario that produced this sprint's hygiene commits
/// when `cargo clippy --no-default-features` started failing
/// unnoticed).
///
/// Required command fragments, in order:
///   1. `cargo fmt --all -- --check`                             (formatting)
///   2. `cargo clippy --release --all-targets -- -D warnings`    (default features)
///   3. `cargo clippy --no-default-features --all-targets -- -D warnings` (ndf)
///   4. `cargo test --release`                                   (release tests)
///
/// This guard checks the `ship:` recipe contains all four fragments
/// and that they appear in chained order (`&&`) so a failure in step
/// N prevents the later steps (and a subsequent `git push`) from
/// running.
#[test]
fn ship_target_chains_all_required_gates() {
    let makefile = fs::read_to_string("Makefile").expect("cannot read Makefile");
    let ship_recipe = extract_recipe(&makefile, "ship:")
        .expect("Makefile must define a `ship:` target — see QUALITY.md Tier 4 #12");
    let required: &[&str] = &[
        "cargo fmt --all -- --check",
        "cargo clippy --release --all-targets -- -D warnings",
        "cargo clippy --no-default-features --all-targets -- -D warnings",
        "cargo test --release",
    ];
    let mut cursor = 0usize;
    for frag in required {
        let found = ship_recipe[cursor..].find(frag).unwrap_or_else(|| {
            panic!(
                "`make ship` recipe is missing or mis-orders `{frag}`.  Full recipe:\n{ship_recipe}"
            )
        });
        cursor += found + frag.len();
    }
    assert!(
        ship_recipe.contains("&&"),
        "`make ship` recipe must chain its gates with `&&` so the first failure aborts the chain (and any subsequent `git push` via `make ship && git push`).  Full recipe:\n{ship_recipe}"
    );
}

/// Read the recipe lines of a Makefile target by name.  Returns the
/// concatenated recipe body (the lines starting with TAB after the
/// target header) up to the next blank line or next target.
fn extract_recipe(makefile: &str, target_header: &str) -> Option<String> {
    let mut lines = makefile.lines();
    for line in lines.by_ref() {
        if line.starts_with(target_header) {
            break;
        }
    }
    let mut out = String::new();
    for line in lines {
        // Recipe lines begin with a tab; a non-tab, non-empty line
        // ends the target.
        if line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    if out.is_empty() { None } else { Some(out) }
}

fn read_problems() -> String {
    fs::read_to_string(PROBLEMS).unwrap_or_else(|_| panic!("cannot read {PROBLEMS}"))
}

fn read_caveats() -> String {
    fs::read_to_string(CAVEATS).unwrap_or_else(|_| panic!("cannot read {CAVEATS}"))
}

fn read_quality() -> String {
    fs::read_to_string(QUALITY).unwrap_or_else(|_| panic!("cannot read {QUALITY}"))
}

/// The main "Open programmer-biting issues" table in QUALITY.md must
/// not contain any row whose issue ID is wrapped in `~~…~~` (i.e. a
/// row marked as closed).  Closed items belong in the paragraph
/// below ("Items that look open in the historical sections ...") or
/// in CHANGELOG.md — not in the live queue.  This is the analogue of
/// the PROBLEMS.md Quick-Reference / long-form guard, applied to
/// QUALITY.md's own main table.
///
/// Also checks that every `~~strikethrough~~` item in the Tier 2
/// enhancement list carries a "Landed YYYY-MM-DD" body marker — the
/// convention this session established for 6a/6b/6c/6d.  Without
/// this guard, the project can claim credit for closing an item
/// whose landing date isn't recorded.
#[test]
fn quality_open_table_has_no_crossed_out_rows() {
    let src = read_quality();
    let (_, rest) = src
        .split_once("## Open programmer-biting issues")
        .expect("'## Open programmer-biting issues' heading missing — QUALITY.md layout changed");
    let (table, _after) = rest
        .split_once("\n---")
        .expect("main table not terminated by `---` — QUALITY.md layout changed");
    for line in table.lines() {
        let l = line.trim_start();
        if !l.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        let first = cells[1];
        if first.is_empty() || first == "#" || first.contains("---") {
            continue;
        }
        assert!(
            !first.starts_with("~~"),
            "QUALITY.md main open-issues table has a crossed-out row `{first}` — move it to the closed paragraph below the table (or to CHANGELOG.md)"
        );
    }
}

/// Every `~~…~~` strikethrough Tier-2 sub-item (6a / 6b / 6c / 6d and
/// their future siblings) must carry a `Landed YYYY-MM-DD` marker in
/// its body, linking the crossed-out claim to an audit trail.
#[test]
fn quality_struck_tier2_items_have_landing_date() {
    let src = read_quality();
    let tier2 = src
        .split("### Tier 2")
        .nth(1)
        .expect("### Tier 2 section missing");
    let tier2 = tier2
        .split("### Tier 3")
        .next()
        .expect("### Tier 2 section not terminated by ### Tier 3");
    // Split into item blocks at lines that look like `6a.` / `6b.` / `6c.`
    // / etc. (digit-then-letter then period).
    let mut blocks: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let bytes = tier2.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() {
        // A block starts at column 0 with `Nx.` where N is a digit and
        // x is a lowercase letter.
        if (i == 0 || bytes[i - 1] == b'\n')
            && bytes[i].is_ascii_digit()
            && bytes[i + 1].is_ascii_lowercase()
            && bytes[i + 2] == b'.'
        {
            if i > start {
                blocks.push(&tier2[start..i]);
            }
            start = i;
        }
        i += 1;
    }
    if start < tier2.len() {
        blocks.push(&tier2[start..]);
    }
    for block in blocks {
        let first_line = block.lines().next().unwrap_or("");
        if !first_line.contains("~~") {
            continue;
        }
        assert!(
            block.contains("Landed") || block.contains("landed"),
            "Struck-through Tier-2 item `{}` has no 'Landed YYYY-MM-DD' marker in its body — add the landing date or remove the strikethrough",
            first_line.trim()
        );
    }
}

/// Every INC# listed in the "Resolved as design point" table must
/// be absent from the Medium and Low severity tables above it.
#[test]
fn inconsistencies_resolved_inc_not_also_listed_as_open() {
    let src = read_doc();
    let (open, resolved) = src
        .split_once("### Resolved as design point")
        .expect("Resolved-as-design-point section missing");
    let resolved_nrs = inc_numbers_in_table(resolved);
    let open_nrs = inc_numbers_in_table(open);
    for n in &resolved_nrs {
        assert!(
            !open_nrs.contains(n),
            "INC#{n} appears in both Resolved table and an open severity table — update the Summary section"
        );
    }
    assert!(
        !resolved_nrs.is_empty(),
        "Resolved-as-design-point table is empty — unexpected; at least INC#3/#9/#12/#17/#26/#29 should be listed"
    );
}

/// Every INC# with a long-form "**Status (YYYY-MM-DD):**" block must
/// appear in the Resolved-as-design-point table (and not in the open
/// severity tables).  Locks the drift where a status block is added
/// but the summary isn't updated.
#[test]
fn inconsistencies_status_blocks_listed_as_resolved() {
    let src = read_doc();
    let mut status_incs: Vec<u32> = Vec::new();
    for (i, line) in src.lines().enumerate() {
        if line.starts_with("## ")
            && let Some(n) = inc_number_from_heading(line)
        {
            // Scan the next ~60 lines (until the next `## ` heading)
            // for a `**Status (` marker.
            let tail: Vec<&str> = src.lines().skip(i + 1).take(80).collect();
            let has_status = tail
                .iter()
                .take_while(|l| !l.starts_with("## "))
                .any(|l| l.contains("**Status ("));
            if has_status {
                status_incs.push(n);
            }
        }
    }
    let (open, resolved) = src
        .split_once("### Resolved as design point")
        .expect("Resolved-as-design-point section missing");
    let resolved_nrs = inc_numbers_in_table(resolved);
    let open_nrs = inc_numbers_in_table(open);
    for n in &status_incs {
        assert!(
            resolved_nrs.contains(n),
            "INC#{n} has a Status block in its long-form entry but is missing from the Resolved-as-design-point table"
        );
        assert!(
            !open_nrs.contains(n),
            "INC#{n} has a Status block but still appears in the Medium/Low severity tables above"
        );
    }
}

/// Every issue that appears in the PROBLEMS.md Quick-Reference as
/// still-open (not wrapped in `~~N~~`) must have a long-form
/// section below whose heading is also not crossed out.  Conversely,
/// if the long-form heading is `### ~~N~~. …` (indicating FIXED /
/// RESOLVED), the Quick-Reference row must also be crossed out or
/// carry a "Done" / "Fixed" marker.  Locks the drift between the
/// two sides of the doc called out in QUALITY.md Tier 4 item 10.
#[test]
fn problems_quickref_matches_longform_status() {
    let src = read_problems();
    let (quickref, longform) = src
        .split_once("## Interpreter Robustness")
        .expect("Interpreter Robustness section missing — PROBLEMS.md layout changed");
    let open_in_quickref = open_issue_numbers_in_quickref(quickref);
    let fixed_longform = fixed_issue_numbers_in_longform(longform);
    for n in &open_in_quickref {
        assert!(
            !fixed_longform.contains(n),
            "Issue #{n} is listed as open in PROBLEMS.md Quick-Reference but its long-form heading is crossed out (FIXED).  Update the Quick-Reference row."
        );
    }
}

/// Every caveat ID whose long-form heading is crossed out
/// (`### ~~CX~~ … DONE` / `### ~~PX~~ … DONE`) must also appear
/// crossed out in the Verification-log table at the bottom.  The
/// table is the reader's at-a-glance index — a stale row there
/// silently claims an already-finished caveat is still in flight.
#[test]
fn caveats_longform_done_matches_verification_log() {
    let src = read_caveats();
    let (body, verification) = src
        .split_once("## Verification log")
        .expect("Verification log section missing — CAVEATS.md layout changed");
    let done_ids = caveat_ids_struck_in_longform(body);
    let open_ids_in_table = caveat_ids_open_in_verification_table(verification);
    for id in &done_ids {
        assert!(
            !open_ids_in_table.contains(id),
            "Caveat '{id}' long-form heading is crossed out (DONE) but the Verification-log table still lists it as open — update the table row to '~~{id}~~ … Done'"
        );
    }
}

/// Extract caveat IDs (C7/P22/C54/…) that appear in `### ~~…~~` headings.
fn caveat_ids_struck_in_longform(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("### ~~")
            && let Some((inner, _)) = rest.split_once("~~")
        {
            // `inner` may be a compound like "C7 / P22" or "P135 / C58".
            for part in inner.split('/') {
                let id = part.trim().to_string();
                if !id.is_empty() {
                    out.push(id);
                }
            }
        }
    }
    out
}

/// Extract caveat IDs in the Verification-log table whose first cell is
/// NOT crossed out (still presented as open).  A compound cell like
/// `C58/P135` with neither side struck counts both; if the cell is
/// `~~C7/P22~~` both sides are treated as struck.
fn caveat_ids_open_in_verification_table(verification: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in verification.lines() {
        let l = line.trim_start();
        if !l.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        let first = cells[1];
        if first.is_empty() || first == "Caveat" || first.contains("---") {
            continue;
        }
        if first.starts_with("~~") {
            continue;
        }
        for part in first.split('/') {
            let id = part.trim().trim_matches('~').to_string();
            if !id.is_empty() {
                out.push(id);
            }
        }
    }
    out
}

/// Returns issue numbers in the Quick-Reference table whose first
/// cell is NOT wrapped in `~~N~~` (i.e. rows still marked open).
fn open_issue_numbers_in_quickref(quickref: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for line in quickref.lines() {
        let l = line.trim_start();
        if !l.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        let first = cells[1];
        if first.starts_with("~~") || first.is_empty() || first == "#" || first.contains('-') {
            continue;
        }
        if let Ok(n) = first.parse::<u32>() {
            out.push(n);
        }
    }
    out
}

/// Returns issue numbers whose long-form heading is crossed out
/// (`### ~~N~~. …`) — indicating FIXED / RESOLVED.
fn fixed_issue_numbers_in_longform(longform: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for line in longform.lines() {
        if let Some(rest) = line.strip_prefix("### ~~")
            && let Some(num_str) = rest.split("~~").next()
            && let Ok(n) = num_str.trim().parse::<u32>()
        {
            out.push(n);
        }
    }
    out
}

fn inc_number_from_heading(line: &str) -> Option<u32> {
    // Heading form: `## 27. ...` — number after `## ` and before `.`.
    let rest = line.strip_prefix("## ")?;
    let (num, _) = rest.split_once('.')?;
    num.trim().parse::<u32>().ok()
}

fn inc_numbers_in_table(section: &str) -> Vec<u32> {
    // Table rows look like `| 27 | ... |` — first column is the INC#.
    let mut out = Vec::new();
    for line in section.lines() {
        let l = line.trim_start();
        if !l.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        if let Ok(n) = cells[1].parse::<u32>() {
            out.push(n);
        }
    }
    out
}

/// RAII guard that restores a file's contents on drop.  The staleness
/// tests below have to let the generator overwrite the real
/// `doc/examples.js` on disk (the script hard-codes that relative
/// path), so this guard makes sure the working tree ends up exactly
/// as it was before the test ran — whether the test panicked, failed,
/// or passed.
struct FileGuard {
    path: std::path::PathBuf,
    original: Vec<u8>,
}

impl FileGuard {
    fn new(path: std::path::PathBuf) -> Self {
        let original = fs::read(&path)
            .unwrap_or_else(|e| panic!("cannot read {} for backup: {e}", path.display()));
        Self { path, original }
    }
}

impl Drop for FileGuard {
    fn drop(&mut self) {
        // Best-effort restore; panicking in Drop would obscure the
        // original test failure.
        let _ = fs::write(&self.path, &self.original);
    }
}

/// Regenerates `output_relative` by running `script` through the
/// release `loft` binary and fails with a clear remediation message
/// if the result differs from the committed version.
fn assert_generator_output_matches_committed(script: &str, output_relative: &str) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output_path = root.join(output_relative);
    let guard = FileGuard::new(output_path.clone());

    let loft_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"));
    let out = std::process::Command::new(&loft_bin)
        .arg("--interpret")
        .arg(script)
        .current_dir(&root)
        .output()
        .unwrap_or_else(|e| panic!("spawn {} failed: {e}", loft_bin.display()));
    assert!(
        out.status.success(),
        "generator `{script}` exited non-zero: {}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let regenerated = fs::read(&output_path)
        .unwrap_or_else(|e| panic!("generator did not write {output_relative}: {e}"));
    if regenerated != guard.original {
        let mut first_diff_line = 0usize;
        let a = String::from_utf8_lossy(&regenerated);
        let b = String::from_utf8_lossy(&guard.original);
        for (i, (l, r)) in a.lines().zip(b.lines()).enumerate() {
            if l != r {
                first_diff_line = i + 1;
                break;
            }
        }
        let a_lines = a.lines().count();
        let b_lines = b.lines().count();
        panic!(
            "\n{output_relative} is stale.\n\nFix:  loft --interpret {script}\nThen: git add {output_relative} && commit\n\n\
             regenerated={a_lines} lines, committed={b_lines} lines, first diff at line {first_diff_line}.\n"
        );
    }
}

/// `doc/examples.js` is committed but auto-generated from
/// `tests/docs/*.loft` by `scripts/build-playground-examples.loft`.
/// Nothing automates the regeneration, so any change to
/// `tests/docs/*.loft` that ships without running the generator
/// leaves GitHub Pages serving stale content.  This test is the
/// guard.
///
/// **Currently ignored:** `build-playground-examples.loft` crashes
/// with SIGSEGV partway through its `for f in file("tests/docs").files()`
/// loop (reproduced on clean `develop` branched from `1753615`; see
/// PROBLEMS.md § "playground generator SIGSEGV").  Until that's
/// fixed, running this test would corrupt `doc/examples.js` on disk
/// (the script opens the file, writes a partial stream, then dies).
/// Remove `#[ignore]` once the generator completes cleanly.
#[test]
#[ignore = "generator SIGSEGV truncates output — see PROBLEMS.md"]
fn doc_examples_js_is_up_to_date() {
    assert_generator_output_matches_committed(
        "scripts/build-playground-examples.loft",
        "doc/examples.js",
    );
}

/// `doc/gallery-examples.js` is committed but auto-generated from
/// `lib/graphics/examples/*.loft` by
/// `scripts/build-gallery-examples.loft`.  Same drift risk as
/// `doc/examples.js` — this guard catches it.
#[test]
fn doc_gallery_examples_js_is_up_to_date() {
    assert_generator_output_matches_committed(
        "scripts/build-gallery-examples.loft",
        "doc/gallery-examples.js",
    );
}
