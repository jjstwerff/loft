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
    let broken_tags = Command::new("./scripts/idx")
        .arg("broken")
        .output()
        .expect("failed to spawn `./scripts/idx broken`");
    assert!(
        broken_tags.status.success(),
        "./scripts/idx broken exited {:?}",
        broken_tags.status.code()
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

    // 3. Phase 09 — broken markdown-link refs.
    let broken_links = Command::new("./scripts/idx")
        .arg("broken-links")
        .output()
        .expect("failed to spawn `./scripts/idx broken-links`");
    assert!(
        broken_links.status.success(),
        "./scripts/idx broken-links exited {:?}",
        broken_links.status.code()
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
}
