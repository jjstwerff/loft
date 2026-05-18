// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Plan-37 phase 03 + 09 — CI gate for the tracker-tag indexer.
//
// Two tests, both gated against `index/tags.json`:
//
//   `no_broken_tracker_tags` (phase 03) — every `@P-id` /
//   `@PLAN-id` reference resolves to an existing PROBLEMS.md
//   row or plan directory.
//
//   `no_broken_markdown_links` (phase 09 follow-up, enabled
//   2026-05-14 once the cleanup pass shipped) — every
//   `[text](path.md)` markdown link resolves to an existing
//   file.
//
// To pass with an INTENTIONAL fake reference (e.g., a design
// doc explaining what a broken tag looks like, or a template
// placeholder), put the literal `<!--noindex-->` marker on
// the same line; the scanner skips those lines.
//
// To DEBUG a failing run:
//   ./scripts/idx broken         # for phase-03 failures
//   ./scripts/idx broken-links   # for phase-09 failures
//
// To FIX:
//   - phase 03: rename the @P-id / @PLAN-id to a real one,
//     add the missing PROBLEMS.md row / plan dir, or add
//     `<!--noindex-->` to the line.
//   - phase 09: fix the relative path (often an off-by-one
//     `..` after a file moves to `finished/`), point at the
//     correct doc, or add `<!--noindex-->` for intentional
//     placeholder examples.  `tools/indexer/fix_broken_links.py`
//     auto-fixes the common off-by-one cases.

use std::process::Command;

/// Run the loft-native `idx.loft` query binary instead of bash
/// `scripts/idx`.  The bash script is the bootstrap path
/// (documented in CLAUDE.md, used from machines without a built
/// loft), but it has accumulated half a dozen cross-platform
/// gotchas — BSD awk's UTF-8 panic on em-dashes, MSYS argv
/// limits, Windows PE-format rejection of bash scripts,
/// MSYS-vs-native-jq path translation, GNU-only `xargs -a` /
/// `find -regex` / `stat -c %Y`, etc.  The loft port runs as a
/// single native binary on every platform with consistent
/// behaviour and short-circuits the entire portability surface.
///
/// `idx.loft`'s MVP covers the two queries the CI gate uses
/// (`broken` and `broken-links`); the other queries (`tag:`,
/// `prefix:`, `file:`, `incoming:`, `all`, `help`, excerpt
/// flags) stay in the bash script until a consumer asks.
fn idx_command(args: &[&str]) -> Command {
    let mut cmd = Command::new("cargo");
    cmd.args([
        "run",
        "--bin",
        "loft",
        "--release",
        "--quiet",
        "--",
        "tools/indexer/src/idx.loft",
    ]);
    for a in args {
        cmd.arg(a);
    }
    cmd
}

// Single test for both checks — running `make index` twice
// concurrently (cargo test default parallelism) corrupts
// `index/tags.json`.  Serialising into one test avoids the
// race without needing a global mutex.
#[test]
fn index_hygiene_clean() {
    // 1. Refresh the index.  `make index` must exit 0.
    let make = Command::new("make")
        .arg("index")
        .output()
        .expect("failed to spawn `make index`");
    assert!(
        make.status.success(),
        "make index exited {:?}\nstdout:\n{}\nstderr:\n{}",
        make.status.code(),
        String::from_utf8_lossy(&make.stdout),
        String::from_utf8_lossy(&make.stderr)
    );

    // 2. Phase 03 — broken @-tag refs.
    let broken_tags = idx_command(&["broken"])
        .output()
        .expect("failed to spawn `bash ./scripts/idx broken`");
    assert!(
        broken_tags.status.success(),
        "./scripts/idx broken exited {:?}\nstdout:\n{}\nstderr:\n{}",
        broken_tags.status.code(),
        String::from_utf8_lossy(&broken_tags.stdout),
        String::from_utf8_lossy(&broken_tags.stderr)
    );
    let tags_out = String::from_utf8_lossy(&broken_tags.stdout);
    assert_eq!(
        tags_out.trim(),
        "[]",
        "broken tracker tags found:\n{tags_out}\n\
         Fix options:\n  \
         (a) rename the @P-id / @PLAN-id to a real one\n  \
         (b) add the missing PROBLEMS.md row / plan dir\n  \
         (c) add `<!--noindex-->` to the line if the ref is \
         an intentional documentation example\n\
         See: doc/claude/plans/37-tracker-index/03-broken-validator.md"
    );

    // 2.5 Phase 07 (MVP) — loft-native scanner finds the same
    // set of refs as `scan.sh`.  Both backends now emit JSON
    // Lines (one `{"tag":..,"file":..,"line":..,"context":..}`
    // record per match); we project both sides through `jq` to
    // a `<file>:<line>:<tag>` key for the diff.  Bash's
    // `git ls-files` only sees tracked files — so this test
    // only catches drift on committed code, not in-flight edits
    // (the file-set filter below handles that).
    let loft_scan = Command::new("make")
        .arg("index-loft")
        .output()
        .expect("failed to spawn `make index-loft`");
    assert!(
        loft_scan.status.success(),
        "make index-loft exited {:?}\nstderr:\n{}",
        loft_scan.status.code(),
        String::from_utf8_lossy(&loft_scan.stderr)
    );
    // Project loft's JSON Lines into the same `<file>:<line>:<tag>`
    // key shape as the bash projection below.  `jq -s` slurps
    // the lines into an array; `select(.tag)` keeps only the
    // tag-ref rows (the loft scanner now ALSO emits `link`
    // rows for the markdown-link bucket; those have no `.tag`
    // field and would project to spurious `:null` entries
    // without the filter).
    let loft_jq = Command::new("jq")
        .args([
            "-rs",
            ".[] | select(.tag) | \"\\(.file):\\(.line):\\(.tag)\"",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn `jq` for loft projection");
    {
        use std::io::Write;
        loft_jq
            .stdin
            .as_ref()
            .expect("jq stdin")
            .write_all(&loft_scan.stdout)
            .expect("write loft scan to jq stdin");
    }
    let loft_proj = loft_jq.wait_with_output().expect("jq loft wait");
    assert!(
        loft_proj.status.success(),
        "jq projection over loft scan exited {:?}",
        loft_proj.status.code()
    );
    let loft_rows: std::collections::BTreeSet<String> = String::from_utf8_lossy(&loft_proj.stdout)
        .lines()
        .map(|s| s.trim_end_matches('\r').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    // jq projection over bash output — same `<file>:<line>:<tag>`
    // shape so the two sides are directly comparable.  Pure-stdlib
    // JSON parse isn't worth pulling serde into this binary just
    // for one test; jq is already a CI dep (both scanners use it).
    let bash_jq = Command::new("jq")
        .args([
            "-r",
            "to_entries[] | select(.key | startswith(\"@\") or startswith(\"legacy:\")) | .key as $k | .value[] | \"\\(.file):\\(.line):\\($k)\"",
            "index/tags.json",
        ])
        .output()
        .expect("failed to spawn `jq` — install jq, or skip this test in environments without it");
    assert!(
        bash_jq.status.success(),
        "jq projection over index/tags.json exited {:?}",
        bash_jq.status.code()
    );
    let bash_rows: std::collections::BTreeSet<String> = String::from_utf8_lossy(&bash_jq.stdout)
        .lines()
        .map(|s| s.trim_end_matches('\r').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    // Bash uses `git ls-files`; loft walks the filesystem.  The
    // diff is dominated by untracked files during local dev —
    // filter the loft set to files bash also indexed (i.e., the
    // tracked set inferred from bash's own output).  This keeps
    // the assertion meaningful for committed code without
    // failing every time someone touches an untracked file.
    let bash_files: std::collections::HashSet<String> = bash_rows
        .iter()
        .filter_map(|row| row.split(':').next().map(|s| s.to_string()))
        .collect();
    let loft_rows_tracked: std::collections::BTreeSet<String> = loft_rows
        .iter()
        .filter(|row| {
            row.split(':')
                .next()
                .is_some_and(|f| bash_files.contains(f))
        })
        .cloned()
        .collect();
    let only_loft: Vec<&String> = loft_rows_tracked.difference(&bash_rows).collect();
    let only_bash: Vec<&String> = bash_rows.difference(&loft_rows_tracked).collect();
    assert!(
        only_loft.is_empty() && only_bash.is_empty(),
        "loft scanner ({}) and bash scanner ({}) disagree (after filtering loft to tracked files).\n\
         only-in-loft ({}):\n{}\n\
         only-in-bash ({}):\n{}\n\
         Fix: either patch tools/indexer/src/scan.loft or tools/indexer/scan.sh\n\
         so both find the same set.\n\
         See: doc/claude/plans/37-tracker-index/07-loft-native-scanner.md",
        loft_rows_tracked.len(),
        bash_rows.len(),
        only_loft.len(),
        only_loft
            .iter()
            .take(10)
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        only_bash.len(),
        only_bash
            .iter()
            .take(10)
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    );

    // 3. Phase 09 — broken markdown-link refs.
    let broken_links = idx_command(&["broken-links"])
        .output()
        .expect("failed to spawn `bash ./scripts/idx broken-links`");
    assert!(
        broken_links.status.success(),
        "./scripts/idx broken-links exited {:?}\nstdout:\n{}\nstderr:\n{}",
        broken_links.status.code(),
        String::from_utf8_lossy(&broken_links.stdout),
        String::from_utf8_lossy(&broken_links.stderr)
    );
    let links_out = String::from_utf8_lossy(&broken_links.stdout);
    assert_eq!(
        links_out.trim(),
        "[]",
        "broken markdown links found:\n{links_out}\n\
         Fix options:\n  \
         (a) fix the relative path (often an off-by-one `..`\n      \
             after the source moved to `finished/`)\n  \
         (b) point at the correct doc\n  \
         (c) add `<!--noindex-->` to the line if the link is\n      \
             an intentional placeholder example\n  \
         (d) try `tools/indexer/fix_broken_links.py --apply` for\n      \
             the common off-by-one cases\n\
         See: doc/claude/plans/37-tracker-index/09-backlinks.md"
    );

    // 4. Cross-OS portability sanity (@PLAN37 phase 07 pre-flight P2)
    // — `index/tags.json` must not contain literal backslashes outside
    // JSON `\"` escapes.  Catches the Windows-only "path separator
    // drift" silent failure on every CI runner (not just Windows):
    // when scan.loft runs on MSYS bash and emits `file().path` raw,
    // the `path` / `file` / `target` fields contain `\` separators
    // and the parity assertion breaks downstream.  Linux runners see
    // the same assertion — fails early instead of waiting for a
    // Windows CI cycle to surface the bug.
    let tags_raw = std::fs::read_to_string("index/tags.json").expect("read index/tags.json");
    // Allow `\\` (JSON-escaped backslash, e.g. inside a context string)
    // and `\"` (escaped quote).  Anything else is a raw backslash that
    // shouldn't be there.  Strip both, then count remaining backslashes.
    let stripped = tags_raw.replace("\\\\", "").replace("\\\"", "");
    let stray = stripped.matches('\\').count();
    assert_eq!(
        stray, 0,
        "tags.json contains {stray} stray backslash(es) — Windows path-separator \
         drift?  scan.loft should pass every path-shaped string through \
         `normalize_path()` (see tools/indexer/src/scan.loft).  Sample: \
         look for a `\\` outside `\\\\` / `\\\"` JSON escapes in tags.json."
    );

    // 5. PROBLEMS.md row-parser sanity (@PLAN37 phase 07 pre-flight P4)
    // — every `problems_open` row's `severity` cell must start with a
    // known severity prefix.  A literal `|` in a row body would shift
    // column boundaries during pipe-split and put body text in the
    // severity cell.  Both scan.sh and scan.loft share this edge case;
    // this assertion catches the silent mis-categorisation.  bash side
    // is canonical until sub-commit H; the assertion applies regardless.
    // Use `jq -r` rather than pulling serde_json into this binary just
    // for one assertion — jq is already a CI dep used elsewhere here.
    let sev_jq = Command::new("jq")
        .args(["-r", ".problems_open[] | .severity", "index/tags.json"])
        .output()
        .expect("spawn jq for severity check");
    assert!(
        sev_jq.status.success(),
        "jq severity extraction failed: {}",
        String::from_utf8_lossy(&sev_jq.stderr)
    );
    // Loose contains-check rather than starts_with — @P229's row has a
    // legitimate custom severity (`(a) Closed; (b) Open (Windows)`) for
    // its split-state windows-half-still-open situation, which doesn't
    // start with any standard prefix but still contains a known severity
    // word.  A literal `|` mid-body would produce garbage that contains
    // NONE of these tokens.
    let severity_tokens = [
        "Low", "Medium", "High", "Critical", "Open", "open", "Closed", "closed", "partial",
    ];
    for sev in String::from_utf8_lossy(&sev_jq.stdout).lines() {
        if sev.is_empty() {
            continue;
        }
        let ok = severity_tokens.iter().any(|p| sev.contains(p));
        assert!(
            ok,
            "problems_open row severity `{sev}` doesn't contain any known \
             severity word — PROBLEMS.md row may contain a literal `|` that \
             breaks pipe-split"
        );
    }
}
