<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN39 — Dogfood-driven loft + stdlib enhancements

**Status:** Future — opened 2026-05-15 from the @PLAN37 phase 07
loft-native scanner port (which was the dogfood exercise that
surfaced these).  Each phase is independently schedulable; pick
in any order.

## Why

Building real loft tools (the @PLAN35 viewer, the @PLAN37 tag
indexer, lib/markdown) systematically surfaces gaps in the
language and stdlib that toy programs and the test suite never
hit.  Per `feedback_dogfood_discovery`: "library-on-library
cross-cuts only fail when a real consumer walks them."  This
plan catalogs the gaps from the last 2 weeks of dogfood work
into schedulable phases, so they can land between feature
slices instead of accumulating as friction.

## Triage

Each item lists where it bit, the proposed enhancement, and
estimated effort:

- **XS** = under 1 hour, single-line / single-fn change
- **S** = under half a day, focused fix
- **M** = 1-3 days, multi-file or new lib
- **L** = a week+, needs its own design doc

Items marked **bug** vs **enhancement**:

- Bugs are observed wrong behavior — work-around exists in the
  consumer; root fix lifts the workaround.
- Enhancements are missing capability — the consumer is doing
  more work than necessary; they'd just write less.

## Phases

| # | Phase | Effort | Type | What ships |
|---|---|---|---|---|
| 0 | **Native codegen quirks (cluster)** | S each, ~M cluster | bug | Three native-only codegen bugs surfaced by `tools/indexer/src/scan.loft`: module-scope `const vector<text>` crashes (`stores.const_refs[NNN]` reads zero-length slice); `s[i] ?? '<char>'` chain-compare type mismatch (E0308 `i32` vs `char`); local `sorted<T[K]> = []` + `+= [T{}]` re-types to `vector<T>`.  All three have in-loft workarounds (function-returning-literal; remove `??` guards; wrap sorted in struct field).  Lifts the workarounds. |
| 1 | **Parser + lexer quirks** | S each | bug | Three small parser/lexer issues: `if X.method(local) { … }` mis-parses with self-slice reassignment in scope (workaround: hoist into a helper fn); type-inference produces `unknown(0)` for conditional reassignment of text branches (workaround: explicit `var: text =` annotation); lexer rejects `\0` / `\xNN` / `\u{NNNN}` escapes (workaround: use `' '`-as-sentinel). |
| 2 | **Compiler tool polish — warnings to stderr + `args()` builtin** | XS | enhancement | `--quiet` / `--no-warnings` flag, OR send compiler warnings to stderr unconditionally in non-test runs.  The current "warnings to stdout under `--native`" pollutes scanner output / viewer state JSON / anything piped.  ALSO: `args() -> vector<text>` builtin so loft programs can dispatch on CLI flags (today scan.loft uses env var `LOFT_INDEX_BUCKETED` as the workaround). |
| 3 | **`vector.sort()` + `vector.sort_by(fn)`** | S | enhancement | Two new methods on `vector<T>`.  `sort()` takes no args (uses default ordering for `T`); `sort_by(fn(T) -> integer)` for custom keys.  Replaces the `sorted<T[K]>` set-as-sort-proxy pattern that shows up in scan.loft (3 places), the viewer's plan-bucket sort (welcome page), and the activity feed's date sort.  ~30 lines saved per use site. |
| 4 | **JSON emission helpers** | S–M | enhancement | Mirror of the existing `json_parse` + `JsonValue` read API for the WRITE side: `to_json(value) -> text` for primitives + `JsonBuilder` for nested structures.  scan.loft has 80+ lines of manual `json_escape` + per-row `"{{...}}"` format-string emission + comma management.  viewer's main.loft has hundreds of `value.field("x").as_text()` reads — paired write helper would close the asymmetry. |
| 5 | **Path helpers — stdlib `path` module** | XS-S | enhancement | `path::dir(p)`, `path::basename(p)`, `path::join(parts...)`, `path::resolve(base, target)`.  scan.loft, the viewer, and lib/markdown each rolled their own `dir_of` / `basename` / `resolve_relative`.  Also: `file().path` returns `./<name>` at root — either normalize in `file()` or provide `path::clean()`. |
| 6 | **Text method gaps — `split(text)`, `starts_with_at(pos, prefix)`** | XS | enhancement | Two missing text methods.  `text.split(text)` (only `split(char)` today) — scan_link_line walks char-by-char to find `](` because it can't `line.split("](")`.  `text.starts_with_at(pos, prefix)` — boundary checks in scan.loft do `line[i+1]=='P' && line[i+2]=='L' && …` instead of `line.starts_with_at(i, "PLAN")`. |
| 7 | **Hash convenience — `.contains(key)` + iteration** | XS | enhancement | `hash<T[K]>` lookup is `h[key] != null` today; `hash.contains(key) -> boolean` is sugar over that.  Also: idiomatic key/value iteration so "set of text" can use `hash<TextSlot[name]>` instead of falling back to `vector<text>` + linear `set_contains` (see scan.loft's valid_pids / valid_plans walk). |
| 8 | **Two-pass forward-resolution for return types** | M | bug (compiler) | Loft's two-pass parser doesn't propagate fn return types in pass-1, so a caller in pass-2 type-checks against `unknown`.  Symptom: caller-defined-after-helper works; caller-defined-BEFORE-helper trips "Expect token ;" or "Cannot assign unknown(0)".  Workaround in scan.loft: extract `is_digit_leaf` / `basename_leaf` to the top of the file, keep original names as one-line aliases.  Cluster of native+text-returning-text fns affected. |
| 9 | **`lib/process/` — subprocess primitive** | M | enhancement (lib) | New library: `process::run(cmd, args) -> {stdout, stderr, code}`, `process::spawn(...)` for streaming.  The single largest architectural unlock: the viewer's `tools/viewer/refresh.sh` exists ONLY because loft can't shell out to git; same for `make index`'s scan.sh-then-loft sequencing.  With `lib/process/`, the viewer reads git state directly, scan.loft can use `git ls-files` to match bash exactly, and `make ci` becomes writable in pure loft. |
| 10 | **`lib/fs_watch/` — file-event watcher** | L | enhancement (lib) | New library: `fs_watch::watch(path) -> iterator<FsEvent>` (inotify on Linux, kqueue on macOS, ReadDirectoryChangesW on Windows).  Needs a Rust host bridge.  Unblocks @PLAN37 phase 07a (WebSocket push) and `make index-watch` continuous mode.  Same pattern lib/server's WebSocket plumbing follows. |
| 11 | **HTML escape + cache primitive** | XS + S | enhancement | Two small bits: `text::escape_html(s)` in stdlib (the viewer rolled its own); `lib/cache/` with mtime-invalidated read-through caching to replace the viewer's per-request disk reads of `index/tags.json` (~150 KB JSON re-parsed on every hit). |

## Sequencing

Most items are independent.  Three soft-dependency clusters:

  - **Compiler quirks (0 + 1 + 8)** — same `src/parser/` /
    `src/generation/` neighborhood; cheaper to land as a 1-2-day
    pass than as separate commits.
  - **lib/process (9) → @PLAN37 phase 07's untracked-files
    parity** — once subprocess lands, scan.loft can use
    `git ls-files` and the test gate's "filter to bash-tracked
    files" workaround goes away.
  - **lib/fs_watch (10) → @PLAN37 phase 07a impl** — the
    WebSocket daemon needs file events.

## Acceptance — full plan

Each phase ships independently; the plan as a whole closes when
all 12 items land OR get rolled into other plans.  No "all-or-
nothing" requirement.

For each phase:
  - Workaround in the consumer is removed (or a comment removed
    documenting it).
  - Test added (regression for bugs; smoke + edge for
    enhancements).
  - Phase-doc closeout note in this README's "Phases" table.

## Risks

| Risk | Mitigation |
|---|---|
| Compiler quirks (phase 0/1/8) cluster has overlapping fixes that interact | Land them in a single `src/` pass under one commit-per-bug; CI catches surprises |
| `vector.sort()` (phase 3) needs comparable-T trait awareness | Restrict to types with built-in ordering first; `sort_by(fn)` covers user types |
| `lib/process/` (phase 9) opens a security surface | Scope to local subprocess only initially; no network / no shell injection paths.  Document in `doc/claude/SECURITY.md`. |
| `lib/fs_watch/` (phase 10) host-bridge complexity | Defer to a focused arc; ship single-shot scanning meanwhile (already shipped) |

## Why a dedicated plan vs distributed entries

The default project flow distributes per-area open work across
reference docs (`## Open work` in `NATIVE.md`, `STDLIB.md`,
etc.).  This plan exists because:

  - The dogfood findings span FOUR areas (native codegen, parser/
    lexer, stdlib, library packages).  Distributing them loses
    the "same root cause: real-tool dogfood surfaced gaps no
    test program touches" narrative that justifies the
    schedule-as-cluster rather than schedule-as-individual.

  - Several items are interdependent (sequencing section
    above).  A dedicated plan README sits at the right level to
    document those dependencies without polluting the
    individual reference docs.

  - The consumer evidence (which loft program hit each gap, in
    which commit) is per-finding context that's most useful
    when collected together.  Distributing scatters it.

When this plan closes, individual phases convert to:
  - P-issues in PROBLEMS.md (the bug entries — phases 0, 1, 8)
  - `## Open work` entries in `STDLIB.md` (the stdlib enhancements
    — phases 3-7)
  - `lib_plans/future/` slots (the library packages — phases
    9, 10)

So this plan is a STAGING SURFACE.  Items leave when they
either ship or get promoted to their canonical home.

## Cross-references

- [`@PLAN37 phase 07`](../../37-tracker-index/07-loft-native-scanner.md)
  — the dogfood exercise that surfaced most of these.  Its
  "Loft gaps surfaced" section is the seed for phases 0 + 1.
- [`@PLAN37 phase 07a`](../../37-tracker-index/07a-websocket-protocol.md)
  — the WebSocket push protocol design that depends on
  phase 10 (`lib/fs_watch/`).
- [`@PLAN35`](../../finished/35-branch-review-viewer/README.md) — the
  viewer that surfaced the JSON-write asymmetry (phase 4) and
  `dir_of` / `basename` duplication (phase 5).
- The `feedback_dogfood_discovery` memory entry codifies this
  discovery pattern (lives outside the doc tree, in
  `.claude/projects/-home-ubuntu-loft/memory/`).
- [`ROADMAP.md`](../../../ROADMAP.md) — the prioritization view
  that picks individual phases of this plan to schedule
  alongside other work.
