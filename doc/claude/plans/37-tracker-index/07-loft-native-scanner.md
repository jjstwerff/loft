<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 07 — Loft-native scanner + CLI + file-event monitor

**Status:** MVP shipped 2026-05-15 (single-shot tag scanner).
Continuous-mode `--watch` + WebSocket daemon + CLI client +
JSON emission deferred to follow-up commits.

## What the MVP shipped

`tools/indexer/src/scan.loft` — a ~225-line loft program that:

- Walks a fixed list of source roots (`doc`, `default`, `lib`,
  `src`, `tests`, `tools`, `examples`) plus the top-level
  indexable files (`CLAUDE.md`, `README.md`, `Cargo.toml`,
  etc.).
- Skips `target/` / `.git/` / `node_modules/` / `.loft/` /
  `bin/` / `state/` subtrees.
- Indexable extensions: `.md`, `.rs`, `.loft`, `.toml`,
  `.sh`, `.py` — same set as `tools/indexer/scan.sh`.
- Honors the `<!--noindex-->` opt-out marker.
- Matches the bash regex's `\b` discipline (the four examples
  on the next line are deliberately marked `noindex` because
  two of them are designed-to-fail tokens that bash's
  awk extractor greedily truncates):
  `@P229bing` and `@PLAN37foo` both fail (no boundary), <!--noindex-->
  `@P259` and `@PLAN35-04-iii.a` succeed. <!--noindex-->

Build pipeline: `make index-loft` runs the scanner via
`loft --native --lib lib/` and strips the loft compiler's
warning preamble so stdout is just `<file>:<line>:<tag>` rows.

Test gate: `tests/index_hygiene.rs::index_hygiene_clean`
(extended) refreshes the bash index, runs `make index-loft`,
diffs the two row sets after filtering loft to files bash
also indexed (the bash scanner's `git ls-files` only sees
tracked files, so the filter avoids false positives on
in-flight untracked files).  Both scanners agree at commit
time on the same set of references.

## Loft gaps surfaced

Three native-codegen / language gaps surfaced during the
MVP exercise.  All had clean in-loft workarounds, so they're
documented here as the trio phase 07 found rather than
filed as P-issues:

1. **`const vector<text>` at module scope crashes native** —
   emit reads `stores.const_refs[565]` from a zero-length
   slice.  Worked around by returning the literal from a
   plain `fn source_roots() -> vector<text>`.
2. **`s[i] ?? '<char>'` mis-types in chained comparison** —
   emitted Rust has `_v_v1 == char::from(0)` where one side
   is `i32` and the other is `char`, rustc E0308.  Worked
   around by removing the `??` guards and accepting the
   "may produce null" warnings — every index is preceded
   by an explicit `i < n` guard so runtime is safe.
3. **No `\0` character escape in loft lexer** — only `\n`,
   `\t`, `\r`, `\"`, `\'`, `\\` are supported.  Not
   blocking; would file as a small loft enhancement when
   the bug-filing budget allows.

These will be promoted to P-issues if a future phase trips
the same edges.

## What's still open

The MVP is the foundation; remaining work for the full phase:

- **JSON emission** — produce the same `index/tags.json`
  shape (per-tag arrays + `legacy:` buckets + `broken` +
  `links` + `problems_open` + `plans_*`), so `bin/loft-index`
  becomes a drop-in replacement for `tools/indexer/scan.sh`.
- **`lib/fs_watch/`** — file-event watcher API for `--watch`
  continuous mode.  Needs host-bridge native lib (inotify
  on Linux, kqueue on macOS, ReadDirectoryChangesW on
  Windows).
- **WebSocket daemon** — wire `lib/server`'s WebSocket path
  for live index subscription.
- **`tools/indexer/idx.loft`** — loft port of the bash
  `scripts/idx` CLI.  Talks to the daemon over the
  WebSocket.
- **Standalone binary build** — `bin/loft-index` and
  `bin/loft-idx` as standalone artifacts (currently the
  scanner runs via `loft --native --lib lib/ scan.loft`).

Each of the above can land as its own commit.

---

## Goal

Re-architect the indexer as a **daemon + clients** model in
loft:

```
        [tools/indexer/bin/loft-index] — long-running daemon
          ├─ initial scan → in-memory tag table
          ├─ subscribes to fs events (inotify/kqueue/Win32)
          ├─ rebuilds incrementally on file changes
          ├─ writes index/tags.json snapshot on each rebuild
          │  (for bash-CLI back-compat + git-grep fallback)
          └─ serves localhost:NNNN via lib/server WebSocket
                 |
                 |   binary frames for large payloads (per-tag
                 |   ref dumps, full file excerpts, diff blobs)
                 |   via lib/server's send_binary path
                 v
       ┌─────────┴───────────┬────────────────────────┐
       v                     v                        v
   tools/indexer/        tools/viewer/         scripts/idx (bash)
   bin/loft-idx          bin/loft-view         (fallback if
   (CLI client)          (subscribes for       daemon down)
                          live updates)
```

Three artefacts:

1. **Daemon** — `tools/indexer/scan.loft` →
   `tools/indexer/bin/loft-index`.  Replaces the bash
   scanner; runs continuously as the source of truth.
2. **Loft CLI** — `tools/indexer/idx.loft` →
   `tools/indexer/bin/loft-idx`.  Talks to the daemon over
   the WebSocket; serves `tag:` / `prefix:` / `file:` /
   `all` / `broken` queries with `--before` / `--after` /
   `--para` / `--max-bytes` excerpt flags (matching the
   bash CLI's surface).  Single-digit-ms responses
   because the daemon holds everything in RAM.
3. **Bash artefacts** — `tools/indexer/scan.sh` +
   `scripts/idx` stay as the bootstrap fallback (no
   loft, no daemon required).  Used by CI hygiene tests
   and from machines without loft built.

### Why WebSocket-style transport (not plain HTTP)

`lib/server` ships both raw HTTP and WebSocket; the
WebSocket path supports binary frames + multi-message
streams.  For the indexer's payload shapes:

- Large tag dumps (`tag:legacy:P200` returns 113 refs ×
  full excerpts ≈ 50-200 KB).
- Per-tag streaming as the daemon updates incrementally
  (subscribe-once + receive-on-change).
- File-diff blobs that the viewer fetches alongside tag
  refs.

Plain HTTP would force one request per query + base64
encoding for binary content.  WebSocket binary frames are
the natural shape for chunked, possibly-streaming data
between local processes — and exercises lib/server's
binary path in production, surfacing any rough edges.

The daemon is BOUND to `127.0.0.1` only — no
authentication; security model is "anyone on this VM can
already read these files anyway."

The PRIMARY motivation is **exposing loft to a long-running,
file-event-driven workload** — a class of program loft hasn't
exercised before.  Real-time indexing is a useful feature on
its own (no need to remember `make index`; the index is
always fresh as you type), but the LANGUAGE LEVERAGE is the
real driver: every gap this surfaces becomes a loft
enhancement.

## Why a loft scanner alongside the bash one

Three motivations, all stated by the user:

1. **Performance testing** — a long-running, file-event-
   driven loft program is a class of workload the language
   hasn't exercised.  Surfaces gaps that no existing test
   touches.
2. **Clean end-project with no runtime deps** — the
   ambition is "a few binaries in `/bin`" that handle the
   tooling.  No `jq`, no `bash`, no Python — just the
   compiled loft binaries.  Easier to install, easier to
   ship, easier to reason about.
3. **Multi-project capability** — the binaries should
   serve DIFFERENT AI projects, not just loft.  Different
   tag conventions, different doc layouts, different
   status sources — all driven by per-project config.

Concrete comparison:

| Concern | Bash scanner (phase 00) | Loft scanner (this phase) |
|---|---|---|
| Bootstrap | Works from a fresh checkout with only `bash` + `grep` + `awk` + `jq` | Requires loft + this binary built |
| Cross-platform | POSIX-portable (Linux + macOS + BSD) | Wherever loft runs |
| Runtime dep footprint | bash + coreutils + jq | Single static binary |
| Loft language exposure | Zero | Drives file-event API + long-running programs + text-scan idioms |
| Continuous refresh | No — git-hook-only | Yes — sub-second response to file edits |
| Maintenance burden | ~80 lines bash, fragile to grep/awk changes | ~300 lines loft, tested via the loft suite |
| Multi-project | One repo, hardcoded paths | Per-project config + daemon-per-project |
| Composability with viewer | Viewer reads the JSON either way | Same — plus live WebSocket subscribe |

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
| **Subprocess primitive** (already noted in @PLAN35 as a gap) | Wrapper script approach | Out of scope for this phase; the loft scanner does NOT shell out to `git ls-files` — it walks the filesystem itself and applies an in-loft `.gitignore` matcher |
| **JSON emission for nested structures** | Loft has `n_struct_from_jsonvalue`; emission less ergonomic | If pattern repeats: build a `lib/json_emit/` helper.  This phase contributes use cases. |
| **Long-running program lifecycle** (graceful shutdown on SIGINT, log rotation) | None | Sibling enhancement — file once concrete pain shows up |
| **Regex (or fast text-search)** | `text.find` / `text.rfind` / loops | `lib_plans/future/01-regex/` already planned; this phase contributes a real consumer |

The phase ships even if some of these gaps stay open — the
loft scanner can use slower workarounds initially and switch
to the better APIs as they land.

### Build pipeline

Mirrors @PLAN35's `view-build` shape:

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
- [`plans/finished/35-branch-review-viewer/`](../finished/35-branch-review-viewer/) — the viewer that consumes the same JSON
