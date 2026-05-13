// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Plan-37 phase 03 — CI gate for the tracker-tag indexer.
//
// Runs `make index` to refresh `index/tags.json`, then asserts
// `./scripts/idx broken` returns `[]` — i.e., no @P-id /
// @PLAN-id reference points at a non-existent PROBLEMS.md row
// or plan directory.
//
// To pass with an INTENTIONAL fake reference (e.g., a design
// doc explaining what a broken tag looks like), put the
// literal `<!--noindex-->` marker on the same line; the
// scanner skips those lines.
//
// To DEBUG a failing run:  ./scripts/idx broken
// To FIX a failing run:
//   - rename the @P-id / @PLAN-id to a real one, OR
//   - add the missing PROBLEMS.md row / plan dir, OR
//   - add `<!--noindex-->` to the line if the ref is an
//     intentional documentation example.

use std::process::Command;

#[test]
fn no_broken_tracker_tags() {
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

    // 2. Query broken[] via the CLI.
    let idx = Command::new("./scripts/idx")
        .arg("broken")
        .output()
        .expect("failed to spawn `./scripts/idx broken`");
    assert!(
        idx.status.success(),
        "./scripts/idx broken exited {:?}",
        idx.status.code()
    );

    // 3. Output should be `[]` (empty array, possibly with whitespace).
    let stdout = String::from_utf8_lossy(&idx.stdout);
    let trimmed = stdout.trim();
    assert_eq!(
        trimmed, "[]",
        "broken tracker tags found:\n{stdout}\n\
         Fix options:\n  \
         (a) rename the @P-id / @PLAN-id to a real one\n  \
         (b) add the missing PROBLEMS.md row / plan dir\n  \
         (c) add `<!--noindex-->` to the line if the ref is \
         an intentional documentation example\n\
         See: doc/claude/plans/37-tracker-index/03-broken-validator.md"
    );
}
