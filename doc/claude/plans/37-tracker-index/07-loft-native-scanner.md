<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 07 — Loft-native scanner + file-event monitor

**Status:** Open

## Goal

Re-implement the indexer in loft, with continuous file-event
monitoring instead of git-hook-driven refresh.  The bash
scanner stays as the bootstrap; the loft scanner becomes the
preferred path once it's stable.

The PRIMARY motivation is **exposing loft to a long-running,
file-event-driven workload** — a class of program loft hasn't
exercised before.  Real-time indexing is a useful feature on
its own (no need to remember `make index`; the index is
always fresh as you type), but the LANGUAGE LEVERAGE is the
real driver: every gap this surfaces becomes a loft
enhancement.

## Why a loft scanner alongside the bash one

| Concern | Bash scanner (phase 00) | Loft scanner (this phase) |
|---|---|---|
| Bootstrap | Works from a fresh checkout with only `bash` + `grep` + `awk` + `jq` | Requires loft + this binary built |
| Cross-platform | POSIX-portable (Linux + macOS + BSD) | Wherever loft runs |
| Loft language exposure | Zero | Drives file-event API + long-running programs + text-scan idioms in loft |
| Continuous refresh | No — git-hook-only | Yes — sub-second response to file edits |
| Maintenance burden | ~80 lines bash, fragile to grep/awk changes | ~300 lines loft, tested via the loft suite |
| Composability with viewer | Viewer reads the JSON either way | Same |

The bash scanner stays as the **canonical bootstrap path**
(documented in CLAUDE.md, used by CI hygiene tests, runs
from machines where loft itself isn't built).  The loft
scanner is the **preferred development path** once it ships
— `make watch` starts it, leaves it running, indexes refresh
within a second of any save.

## What ships

### `tools/indexer/scan.loft` — the loft port

A loft binary that mirrors `scan.sh`'s behaviour but uses
loft primitives:

```loft
// tools/indexer/scan.loft — phase 07: loft-native scanner.
// Compiles via `make index-build` to tools/indexer/bin/loft-index.
// Two modes:
//   loft-index           — single-shot scan (replaces scan.sh)
//   loft-index --watch   — continuous mode (file-event driven)

use server;          // for stats endpoint? (stretch)
use index_io;        // file-event API, see "loft enhancements" below

fn main(args: vector<text>) {
    if args.length() > 1 && args[1] == "--watch" {
        run_watch();
    } else {
        run_once();
    }
}

fn run_once() {
    files = walk_repo();
    tags = scan_files(files);
    write_json("index/tags.json", tags);
}

fn run_watch() {
    // Initial full scan.
    run_once();
    // Then subscribe to file events; debounce + re-scan
    // changed files only.
    watch_loop();
}
```

Same `index/tags.json` output schema as the bash scanner;
shipping is a binary substitution, not a data-format
migration.

### Loft enhancements this phase will need (drives loft itself)

This phase is the JUSTIFICATION for several loft-side
additions.  None block phase 07 entirely — each can be a
sibling P-issue or `lib_plans/future/` plan that this phase
drives.

| Loft gap | Today's workaround | Loft enhancement |
|---|---|---|
| **File-event watcher API** (inotify on Linux, kqueue on macOS, ReadDirectoryChangesW on Windows) | None — the bash scanner relies on the git pre-commit hook (phase 02) | `lib/fs_watch/` package with a streaming `watch(path: text) -> iterator<FsEvent>` API.  Cross-platform via the same host-bridge pattern `lib/server` uses |
| **Subprocess primitive** (already noted in plan-35 as a gap) | Wrapper script approach | Out of scope for this phase; the loft scanner does NOT shell out to `git ls-files` — it walks the filesystem itself and applies an in-loft `.gitignore` matcher |
| **JSON emission for nested structures** | Loft has `n_struct_from_jsonvalue`; emission less ergonomic | If pattern repeats: build a `lib/json_emit/` helper.  This phase contributes use cases. |
| **Long-running program lifecycle** (graceful shutdown on SIGINT, log rotation) | None | Sibling enhancement — file once concrete pain shows up |
| **Regex (or fast text-search)** | `text.find` / `text.rfind` / loops | `lib_plans/future/01-regex/` already planned; this phase contributes a real consumer |

The phase ships even if some of these gaps stay open — the
loft scanner can use slower workarounds initially and switch
to the better APIs as they land.

### Build pipeline

Mirrors plan-35's `view-build` shape:

```make
index-build:  ## Compile the loft-native scanner
	@./target/release/loft --native --lib lib/ tools/indexer/scan.loft
	@cached=$$(ls -t tools/indexer/.loft/cache/scan-* 2>/dev/null | head -1); \
	    cp -f "$$cached" tools/indexer/bin/loft-index; \
	    chmod +x tools/indexer/bin/loft-index

index-watch:  ## Run the loft-native scanner in continuous mode
	@if [ ! -x tools/indexer/bin/loft-index ]; then \
	    echo "tools/indexer/bin/loft-index missing — run: make index-build"; exit 1; \
	fi
	./tools/indexer/bin/loft-index --watch
```

`make index` continues to invoke the bash scanner — it's the
guaranteed-fast bootstrap.  `make index-watch` is the
opt-in continuous mode.

### File-event watch loop (architecture)

```
[startup]
   |
   v
full scan → write index/tags.json
   |
   v
subscribe to fs events on repo root
   |
   v
[event: file modified]
   |
   v
debounce 200 ms (coalesce burst saves from editor / git ops)
   |
   v
incremental rescan: only the changed file's tags
   |
   v
merge with existing tags.json
   |
   v
write tags.json atomically (temp file + rename)
   |
   v
[loop]
```

Atomic write avoids the viewer reading a half-written file.

Debounce + per-file incremental rescan keeps response time
sub-second even for editor-batched events (vim's swap-file
churn, git's index-rewrite during checkout).

### Stretch — observability HTTP endpoint

If the watcher long-runs in a VM, expose a tiny HTTP
endpoint via `lib/server`:

- `GET /stats` — last-scan timestamp, total tags, broken
  count, watched file count.
- `GET /tags.json` — serve the current index directly
  (saves a disk round-trip for the viewer).

Optional; this phase ships without it.

## Critical files

| Path | Action |
|---|---|
| `tools/indexer/scan.loft` | NEW — loft-native scanner (~300 lines) |
| `tools/indexer/bin/loft-index` | Built artifact |
| `tools/indexer/scan.sh` | Stays as bootstrap path |
| `Makefile` | ADD `index-build`, `index-watch` targets |
| `lib/fs_watch/` | NEW package (file-event API) — driven BY this phase, but lands as its own commit/sibling plan |
| `doc/claude/DEBUG.md` | Extend § Tracker-tag indexer with `make index-watch` notes |

## Acceptance

- `make index-build` compiles `tools/indexer/scan.loft` via
  loft's native backend; produces `bin/loft-index`.
- `./tools/indexer/bin/loft-index` (no args) produces the
  same `index/tags.json` shape as `tools/indexer/scan.sh`
  (validated by a diff test in `tests/index_hygiene.rs`).
- `./tools/indexer/bin/loft-index --watch` starts; editing
  any indexed file triggers a re-scan within 1 second;
  `index/tags.json` mtime advances.
- `Ctrl-C` stops the watcher cleanly (no orphan file
  descriptors, no half-written `tags.json`).
- The bash scanner stays the CI canonical (avoids the
  bootstrap-loop where the loft scanner depends on a loft
  binary that depends on a working tree).

## Risks

| Risk | Mitigation |
|---|---|
| File-event API requires loft enhancement that takes weeks | Single-shot mode (no `--watch`) ships independently; `--watch` is a stretch within phase 07 |
| Loft scanner diverges from bash output schema | `tests/index_hygiene.rs` adds a diff test: run both scanners, assert byte-identical `tags.json` |
| Watcher consumes resources (open fd per file) | `inotify` on Linux uses one fd for the whole subtree; macOS kqueue + Windows ReadDirectoryChangesW have their own efficiency profiles.  The host-bridge wrapper picks the right primitive per OS |
| Continuous mode hides bugs that batch mode catches | CI continues to use the bash scanner; the loft scanner is dev-loop only |
| Bootstrap requires loft to build to index loft itself | Bash scanner remains the no-loft-required path; documented as the canonical CI path |

## Why this phase justifies itself

**Pure feature view**: a continuous file-watcher that keeps
`index/tags.json` fresh in real time is nice but not
critical — the pre-commit hook from phase 02 covers 95% of
the freshness need.

**Language-leverage view**: building a long-running,
file-event-driven loft program surfaces gaps that no
existing loft test or example exercises.  Each gap closed
becomes infrastructure for every future loft program in the
same shape.  This is the user's stated reason for asking
for it: "I want the exposure of loft to this kind of
workload."

The phase is sequenced AFTER phases 00-03 (the bash scanner
+ CLI + hooks + validator) so the indexer feature set is
complete + stable before the loft port begins.  Phase 07
slots between phase 04 (viewer integration) and phase 05
(Claude integration) — orthogonal to both.

## Cross-references

- [Phase 00 — bash scanner](00-convention-and-scanner.md) — the spec this phase ports
- [Phase 02 — pre-commit hook](02-auto-refresh.md) — covers the freshness case the watcher complements
- [Phase 03 — broken-tag validator](03-broken-validator.md) — `tests/index_hygiene.rs` extended here for the schema-diff test
- [`lib/server/src/server.loft`](../../../../lib/server/src/server.loft) — pattern for a long-running loft program with a host-bridge native lib
- [`lib_plans/future/01-regex/`](../../lib_plans/future/01-regex/) — text-search primitive that would simplify the scanner
- [`plans/35-branch-review-viewer/`](../35-branch-review-viewer/) — the viewer that consumes the same JSON
