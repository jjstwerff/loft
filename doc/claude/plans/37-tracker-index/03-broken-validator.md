<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 03 — Broken-tag validator

**Status:** Open

## Goal

Catch tag references that don't resolve to a real entity:
`@P9999` (no such P-issue), `@PLAN99` (no such plan dir).
Surface as a `broken` key inside `index/tags.json` AND as a
CI test failure so PRs that introduce broken refs get
flagged.

## What ships

### Scanner extension

`tools/indexer/scan.sh` extended to:

1. After scanning, for every `@P<N>` key collect the set of
   distinct `<N>` values.
2. Cross-reference against `doc/claude/PROBLEMS.md` row
   numbers (parse `^| <N> |` rows in the open-issues table).
3. Any `@P<N>` referenced but not in PROBLEMS.md → broken.
4. Same for `@PLAN<N>...` against
   `doc/claude/plans/[0-9]+-*/`,
   `doc/claude/plans/finished/[0-9]+-*/`,
   `doc/claude/plans/future/[0-9]+-*/`,
   `doc/claude/plans/deferred/[0-9]+-*/`.
5. Sub-phase IDs (`@PLAN35-04`) are validated against the
   plan dir's per-phase files
   (`plans/.../35-…/04-*.md` exists).

Output: `index/tags.json` gains a top-level `broken` key:

```json
{
  "@P259": [...],
  "@PLAN35-01": [...],
  "broken": [
    {"tag": "@P9999", "files": ["doc/foo.md:42"]},
    {"tag": "@PLAN99-03", "files": ["doc/bar.md:15"]}
  ]
}
```

### CI hygiene test

`tests/index_hygiene.rs` — single test:

```rust
#[test]
fn no_broken_tracker_tags() {
    // Run `make index` if index/tags.json is missing or stale.
    // Then jq the broken[] array.  Fail if non-empty.
    let status = std::process::Command::new("make")
        .arg("index")
        .status()
        .expect("make index failed");
    assert!(status.success());
    let json = std::fs::read_to_string("index/tags.json").unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let broken = v.get("broken")
        .and_then(|b| b.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(broken, 0,
        "broken tracker tags found:\n{}\n\
         Fix: rename the broken @P-id / @PLAN-id, or add the \
         missing PROBLEMS.md row / plan dir.",
        serde_json::to_string_pretty(&v["broken"]).unwrap());
}
```

Adds `serde_json` to `[dev-dependencies]` (already present
per existing `tests/p254_cache_poisoning.rs`).

## Acceptance

- `index/tags.json` includes a `broken` key (empty array if
  all refs resolve).
- `cargo test --test index_hygiene` passes on a clean tree.
- Introducing a fabricated `@P9999` reference somewhere
  → `cargo test --test index_hygiene` fails with the
  offending file:line.
- Removing the bogus reference → test passes again.

## Risks

| Risk | Mitigation |
|---|---|
| PROBLEMS.md row format changes break the parser | Parser only looks for `^\| <N> \|` (digits between two pipes); robust to column-content changes |
| Plan dirs renamed to `finished/` mid-development | Validator checks all four locations (`plans/<N>-`, `plans/finished/<N>-`, `plans/future/<N>-`, `plans/deferred/<N>-`) |
| Sub-phase ID like `@PLAN22-2d-iii.a` doesn't map to a single file | Treat sub-phases as informational; validate only the parent plan exists |
| False-positive on tag-like strings in code (e.g., a literal "P9999" in a test fixture) | Add a `# noindex` opt-out comment recognised by the scanner; phase 06 closeout audits any false-positives that surface |

## Cross-references

- [Phase 00 — scanner](00-convention-and-scanner.md) — the scanner this phase extends
- [Phase 02 — auto-refresh](02-auto-refresh.md) — same hook can fail-fast on broken refs
- [PROBLEMS.md](../../PROBLEMS.md) — the source of truth for `@P-id` validity
