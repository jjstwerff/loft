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
