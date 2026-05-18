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
  - Per-tag + `legacy:*` + `links` buckets: **shipped
    2026-05-15** (gated by `LOFT_INDEX_BUCKETED=1`).
  - `broken` + `broken_links` buckets: **shipped 2026-05-18**.
    Loft output is byte-identical to bash on these two keys
    (asserted by `diff <(jq -S '{broken, broken_links}' loft.json)
    <(jq -S '{broken, broken_links}' bash.json)`).
  - Still open: `problems_open`, `problems_recent`,
    `plans_active`, `plans_future`, `plans_deferred`,
    `plans_recent`, `lib_plans_future` — each parses PROBLEMS.md
    / plan directories to produce structured lists, orthogonal
    to the scanning pipeline.  Each can land as its own commit.
- **`lib/fs_watch/`** — file-event watcher API for `--watch`
  continuous mode.  Needs host-bridge native lib (inotify
  on Linux, kqueue on macOS, ReadDirectoryChangesW on
  Windows).
- **WebSocket daemon** — wire `lib/server`'s WebSocket path
  for live index subscription.  Wire-format + lifecycle
  design doc shipped 2026-05-15:
  [07a-websocket-protocol.md](07a-websocket-protocol.md).
- **`tools/indexer/idx.loft`** — loft port of the bash
  `scripts/idx` CLI.  Talks to the daemon over the
  WebSocket.
  - **MVP shipped 2026-05-18** (commit `a9a96f0f`).  Covers the
    two queries the CI gate uses (`broken` + `broken-links`).
    Wired into `tests/index_hygiene.rs::index_hygiene_clean` via
    `cargo run --bin loft -- tools/indexer/src/idx.loft <query>`,
    replacing the bash `scripts/idx` invocation that surfaced
    PE-format + MSYS-path + native-jq compatibility gotchas on
    Windows.  Driver: every cross-OS bash gotcha listed in
    [§ Bash scripts evaluation — what else benefits from loft?](#bash-scripts-evaluation--what-else-benefits-from-loft).
  - Still open: `tag:` / `prefix:` / `file:` / `incoming:` /
    `all` / `help` queries, plus the `--before` / `--after` /
    `--para` / `--max-bytes` excerpt flags.  Each can land as
    its own commit; the bash `scripts/idx` stays as the
    bootstrap fallback (per CLAUDE.md "Key commands") until
    all queries have a loft equivalent.
- **Standalone binary build** — `bin/loft-index` and
  `bin/loft-idx` as standalone artifacts (currently the
  scanner runs via `loft --native --lib lib/ scan.loft`).

Each of the above can land as its own commit.

## Bash scripts evaluation — what else benefits from loft?

The cross-OS fight that PR-212 surfaced (six commits patching
`scan.sh` for BSD-awk UTF-8 panics, MSYS argv limits, PE-format
rejection, native-jq MSYS-path translation, GNU-only flags, etc.)
prompted an audit of every bash / Python script in `tools/` and
`scripts/`.  Ranked by **dogfood ROI × stability gain**:

| Script | Lines | In CI? | Cross-OS today? | Recommendation |
|---|---|---|---|---|
| **`tools/indexer/scan.sh`** | 630 | Linux + macOS + Windows | No — just absorbed 6 portability patches | **STRONG YES — port next.**  Already half-done (`tools/indexer/src/scan.loft` exists as the loft-side scanner; remaining work: full bucketed tags.json + broken validation + JSON merge to replace bash's role in `make index`).  Highest dogfood ROI: exercises file walking, JSON emission, sorted/hash, format strings; largest stability win — drops the entire bash dependency from `make index`. |
| **`scripts/check_doc_drift.sh`** | 460 | Linux only (non-blocking) | Probably broken on macOS/Windows (heavy `awk` + `find` + `realpath`) | **Yes, but second.**  Not on fire because CI only runs it on Linux, so portability gain is latent.  Good dogfood for `lib/markdown` + text scanning.  Revisit when the non-blocking status escalates OR when the bash version surfaces its first real regression. |
| **`tools/viewer/refresh.sh`** | 135 | No (runs during `make view`) | Suspect — shells out to `git` + `jq` | **Wait for `lib/process`** (planned in [`lib_plans/future/15-process/`](../../lib_plans/future/15-process/)).  The viewer needs git state in JSON; loft can't spawn `git` until `lib/process` lands.  Once that ships, port `refresh.sh` so the viewer becomes a pure-loft tool. |
| **`tools/indexer/install-hook.sh`** | 64 | No (one-shot install) | Maybe | **Skip.**  One-time setup; install-time OS quirks are acceptable. |
| **`scripts/find_problems.sh`** | 361 | No (dev-only) | Linux primary | **Skip.**  Developer harness around `cargo test --no-fail-fast`; spawning cargo + parsing output is awkward without `lib/process`. |
| **`scripts/p09_fast_gate.sh`** | 204 | No (dev-only) | Linux | **Skip.**  Used by devs for fast iteration; not user-visible. |
| **`tools/indexer/fix_broken_links.py`** | 278 | No (manual repair) | Python is already portable | **Skip.**  No portability fire. |
| **`tools/indexer/migrate.py`** | 312 | No (one-shot, already ran) | Python | **Skip** — done. |
| **`scripts/browser/coop_server.py`** | 23 | Yes (browser tests) | Python | **Skip** — tiny + portable. |
| **`scripts/browser/run_golden.sh`** | 123 | Yes (browser tests, Linux only) | Linux only by nature (headless Chrome orchestration) | **Skip** — bound to a Linux CI runner with Chrome installed. |
| **`scripts/browser/run_caps.sh`** | 76 | Yes (browser tests, Linux only) | Linux only | **Skip** — same. |

**Sequencing for follow-up commits:**

0. **Prerequisite** — [`lib_plans/01-regex/`](../../lib_plans/01-regex/) Phase 0 (cdylib bridge MVP).  Opened 2026-05-18.  Without regex, `scan.loft`'s `scan_line()` is 150 lines of hand-rolled character walking to recognise four tag forms; with regex it's 4 patterns and ~20 lines.  Same multiplier for `check_doc_drift.sh`.  Ship the MVP first so the port arcs below don't have to re-implement what the Rust `regex` crate already provides.
1. **Next (gated on Phase 0 above)** — finish porting `scan.sh` →
   `tools/indexer/src/scan.loft` (already exists for tag emission;
   remaining work: broken-tag validation + full bucketed JSON
   merge + `problems_open` / `plans_*` data sources, using regex
   for the markdown-link and PROBLEMS.md row parsing).  Phase 7's
   stated goal.  Drops `make index`'s bash dependency on all three
   platforms.
2. **After `lib/process` lands** — port `refresh.sh` so the
   viewer becomes pure-loft end-to-end.
3. **Eventually (gated on Phase 0 above)** — port
   `check_doc_drift.sh` when its non-blocking status changes OR
   when it surfaces a real bug.  Heavy regex usage; needs the lib
   first.
4. **Don't port** — the 8 scripts that are Python (portable),
   Linux-only by nature (browser orchestration), or one-shot
   installers/migrations.

The driving principle: **port when the bash version costs more
hours patching OS quirks than the loft port costs to write.**
`scan.sh` already crossed that line; nothing else has yet — and
the regex-library prerequisite collapses the per-port cost
further by an order of magnitude.

---

## scan.sh removal — design + sequencing

**Status:** Helper landed (commit `9a163f55`), implementation
sequenced.  Not gated on a single PR — work in this arc lands
incrementally as each bucket is ported.

**Driver:** every Windows CI failure on PR-212 (11 of 12 runs)
traced back to `tools/indexer/scan.sh` accumulating BSD- /
MSYS-incompatibility patches faster than they can be applied
cleanly.  Audit:

| Run window | Windows failure shape | scan.sh fix attempt |
|---|---|---|
| First run | codegen_emitter P269 (loft bug, fixed in `236d058a`) | n/a — real loft bug |
| Runs 2-4 | `make index exited 2` | LC_ALL=C + `awk mktime` → shell cutoff + stat -c / stat -f fallback |
| Runs 5-6 | `./scripts/idx broken exited 1` | bash wrapper + idx.loft MVP (a9a96f0f) |
| Runs 7-12 | empty `VALID_PLANS` → every `@PLAN*` ref broken | (unfixed) `ls -d` on MSYS returns backslash paths; `grep '/[0-9]+-'` matches nothing |

Each patch fixed one shape and revealed the next.  A loft binary
runs identically on every platform — one-and-done.

### Buckets to port (full `tags.json` parity)

| Bucket | Already in scan.loft? | Needed for full parity |
|---|---|---|
| `@P*` / `@PLAN*` per-tag arrays | ✅ (commit `c8140729`, gated by `LOFT_INDEX_BUCKETED=1`) | — |
| `legacy:*` per-bare-name arrays | ✅ (same commit) | — |
| `links` map (markdown link target → inbound refs) | ✅ (same commit) | — |
| `broken` (broken `@`-tag refs) | ✅ (commit `15103884`, today) | — |
| `broken_links` (broken markdown links) | ✅ (commit `15103884`, today) | — |
| `problems_open` | ❌ | parse PROBLEMS.md rows + severity filter `(^\| \|\()open( \|,\|\)\|$)\|\(partial`; per-row `{tag, line, severity, summary, fix}` |
| `problems_recent` | ❌ | same parse, filter `(closed)` but not `(open` / `(partial`; find `Closed (YYYY-MM-DD)` in body; gate on `close_date >= ymd_days_ago(30)`; sort by `.closed` desc |
| `plans_active` / `plans_future` / `plans_deferred` / `lib_plans_future` | ❌ | walk `doc/claude/{plans,lib_plans}/[active\|future\|deferred]/[0-9]+-*/`, emit `{slug, path, title}` per dir with a README, title from first `# ` heading |
| `plans_recent` (finished < 60 days) | ❌ | same walk, filter by dir mtime; needs `file.mtime` accessor (currently scan.sh uses `stat -c %Y` / `stat -f %m`) |
| Summary stats line | ❌ | tail print: counts of refs / link targets / open P-issues / recent / plans per category |

### Helpers already shipped

- `arguments() -> vector<text>` (10.3) — CLI args.
- `text.split('|') -> vector<text>` (10.4 / existing) — row splitting.
- `text.find` / `text.starts_with` / `text.trim` / `text.len` — body parsing.
- `json_parse` + `JsonValue` — read-only JSON nav (already used by idx.loft).
- **`ymd_days_ago(days) -> text`** (commit `9a163f55`, today) — cutoff-date
  computation for the two date-window buckets.  Replaces bash's `date -d`
  / `date -v` fork.

### Helpers still missing

- **`file.mtime() -> long`** — dir-mtime read for `plans_recent`'s
  60-day filter.  scan.sh currently uses `stat -c %Y` / `stat -f %m` with
  a `|| echo 0` fallback.  Loft has `file.size: long` but no mtime
  accessor today.  Small native fn following the same shape as
  `os_directory_native()` — ~10 lines in `src/database/format.rs` +
  one-line declaration in `default/02_images.loft`.  Can ship
  alongside the `plans_recent` bucket.

### Does regex (`lib/regex/`) help this arc?

Audit triggered by 2026-05-18 chat — short answer: **not the
bottleneck.**  Regex would save ~30% of the remaining loft line
count but doesn't unblock anything.

| Bucket | Without regex | With regex Phase 0 (cdylib bridge) | Saved |
|---|---|---|---|
| `problems_open` | ~80 lines: row-detect (`if line.starts_with("\| ")` + digits), split on `'\|'` (builtin), severity-filter as substring scan over a handful of patterns, summary-build (find-period, truncate).  All primitives present, verbose. | ~30 lines: one regex matches row + captures the four cells; severity-filter compiles to a single PCRE. | ~50 |
| `problems_recent` | Same shape + a `Closed (YYYY-MM-DD)` extractor (~20 lines manual scan). | Same regex + one extra `Closed \((\d{4}-\d{2}-\d{2})\)` capture. | ~15 |
| `plans_active` / `_future` / `_deferred` / `_recent` / `lib_plans_future` | Directory walk (`walk_tree`, already in place), README-title from first `# ` line, JSON emit.  **Zero regex needed** — filesystem walks. | n/a | 0 |
| Summary stats | Pure `println` of counts. | n/a | 0 |
| `file.mtime()` native | XS Rust fn. | n/a | 0 |

Net: regex saves ~65 lines on the two PROBLEMS.md parsers (out
of ~250 total for the remaining buckets).  Not gating.

**Where regex IS a force multiplier** — the existing 230 lines of
hand-rolled tag tokenizer (`scan_line`, ~150 lines) +
`scan_link_line` (~80 lines).  Both already shipped + working;
retroactive simplification with regex would drop ~220 lines
without changing behaviour.  A nice clean-up after the scan.sh
removal lands, not a prerequisite.

The other big regex consumer is the `scripts/check_doc_drift.sh`
port (separate plan, deferred) — that one DOES need regex to be
worth doing in loft (~600 lines bash; ~200 lines loft with
regex; ~600 lines loft without regex would be a wash).

**Conclusion:** ship sub-commits B-H of this arc with the
existing primitives.  Regex Phase 0 from
[`lib_plans/01-regex/`](../../lib_plans/01-regex/) lands in
parallel for the other consumers and enables a follow-up
"scan.loft regex pass" that retroactively trims `scan_line` /
`scan_link_line` once the binary is using the canonical engine.

### Sub-commit sequencing

Each row lands as its own focused commit so the arc can pause
between buckets without leaving the tree in a half-state.  Order
chosen so each commit is independently testable on the existing
test gate (the assertion `./scripts/idx broken == "[]"` stays
green throughout):

| # | Sub-commit | Effort | Test signal |
|---|---|---|---|
| **A** | `ymd_days_ago(days)` native primitive | XS | **Shipped** (commit `9a163f55`) — `cargo test fill_rs_up_to_date` green |
| B | `emit_problems_open` in scan.loft + emit when `LOFT_INDEX_BUCKETED=1` | S | spot-check JSON shape against bash output; bash side stays canonical |
| C | `emit_problems_recent` in scan.loft using `ymd_days_ago(30)` | S | same |
| D | `file.mtime() -> long` native (new helper) + declaration | XS | smoke test on any file |
| E | `emit_plans_active` / `_future` / `_deferred` / `lib_plans_future` (the four no-date buckets) | S | shape check |
| F | `emit_plans_recent` using `file.mtime()` + `ymd_days_ago(60)` | S | shape check |
| G | Summary stats line | XS | textual match against bash output |
| H | Switch `make index` to invoke scan.loft (with the existing bash scanner becoming a fallback path: `make index-bash` for emergencies) | S | full diff test: `make index` → tags.json byte-for-byte identical (or jq-projection equivalent) on Linux |
| I | Cross-platform CI verification — Linux green, macOS green, **Windows green** | n/a | `index_hygiene_clean` passes on all three CI runners |
| J | Delete `tools/indexer/scan.sh` + `make index-bash` fallback; update `tests/index_hygiene.rs` to only invoke loft side | XS | clean removal |

### Minimum to land PR-212

PR-212's blocking failure is Windows-only.  Three paths to a green
PR, ranked by directness:

1. **Land the full arc above through commit I** before merging.
   Highest investment; cleanest end state; no follow-up tech debt.
2. **Land the arc through commit B (problems_open)**, then switch
   `make index` once Linux's parity diff is byte-identical for at
   least the broken / broken_links / per-tag arrays buckets (the
   ones the test gate actually checks).  Skip `problems_recent` /
   `plans_*` for this PR (viewer keeps reading them from a stale
   tags.json until they're ported in a follow-up PR).  Medium
   investment; closes Windows without bash-side regressions.
3. **One-line scan.sh patch** for the specific MSYS path-separator
   bug (pipe `ls -d` output through `tr '\\\\' '/'`).  Trivial; gets
   Windows green; leaves the broader bash-portability surface in
   place for the next moving target to find.  This is exactly the
   pattern that produced 11 Windows failures on this PR — adding a
   12th patch is the path of least short-term resistance and most
   long-term cost.

Recommendation: **option 1 or 2.**  Option 3 contradicts the
"remove scan.sh completely" direction from the user's
2026-05-18 chat and the empirical evidence above.  Whether to
pause the arc at commit H or push through commit J depends on
whether the user wants to absorb the larger arc inside this PR
or follow up.

### Cross-references

- `doc/claude/lib_plans/01-regex/` — regex Phase 0 (cdylib bridge MVP) is the next force-multiplier for any future bash-script port; not blocking on this arc since scan.loft's text-matching is already hand-rolled and works.
- `doc/claude/plans/future/41-doc-hygiene-autofix/` — the Phase 0 move-rewriter would close the OTHER PR-212-style cascade (directory moves vs OS-portability); orthogonal to this arc.

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
| **Regex (or fast text-search)** | `text.find` / `text.rfind` / loops | `lib_plans/01-regex/` already planned; this phase contributes a real consumer |

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
- [`lib_plans/01-regex/`](../../lib_plans/01-regex/) — text-search primitive that would simplify the scanner
- [`plans/finished/35-branch-review-viewer/`](../finished/35-branch-review-viewer/) — the viewer that consumes the same JSON
