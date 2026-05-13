<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 08 — Multi-project deployment + per-project config

**Status:** Open

## Goal

Generalise the indexer + viewer + CLI from "loft project's
own internal tooling" to "AI-project tooling stack that
handles different projects at a time."  Per the user's
direction: a few static binaries in `/bin`, one daemon
per project, no shared global state.

## What this phase ships

### Per-project config — `.tracker/config.toml`

A single file at the project root that tells the daemon how
to scan + classify:

```toml
# .tracker/config.toml — per-project tracker config.
# Lives at the root of any AI/coding project that wants
# the loft tooling stack.

[project]
name = "loft"
version = "0.8.4"

[scanner]
# File globs to scan.
include = [
  "**/*.md",
  "**/*.rs",
  "**/*.loft",
  "**/*.toml",
  "**/*.py",
  "**/*.sh",
]
exclude = [
  "target/**",
  "node_modules/**",
  ".git/**",
  ".tracker/state/**",
]

[tags]
# Tag families to extract.  Each entry is a name + regex.
# The default loft conventions:
[[tags.family]]
name   = "p-issue"
prefix = "@P"
regex  = '@P[0-9]+[a-z]?\b'

[[tags.family]]
name   = "plan"
prefix = "@PLAN"
regex  = '@PLAN[0-9]+(-[a-zA-Z0-9._]+)*\b'

# Other projects can define their own — e.g., GitHub-style
# `#1234` issue refs, Linear `ENG-123`, JIRA `PROJ-456`:
# [[tags.family]]
# name   = "github-issue"
# prefix = "#"
# regex  = '#[0-9]+\b'

[validators]
# Where to find the source of truth for each tag family.
# Empty = no validation; broken[] stays empty for that family.

[validators.p-issue]
type = "table-row"
file = "doc/claude/PROBLEMS.md"
row_pattern = '^\| ([0-9]+) \|'

[validators.plan]
type = "directory"
roots = [
  "doc/claude/plans/[0-9]+-*",
  "doc/claude/plans/finished/[0-9]+-*",
  "doc/claude/plans/future/[0-9]+-*",
  "doc/claude/plans/deferred/[0-9]+-*",
]

[daemon]
port = 0          # 0 = auto-pick free port, written to .tracker/daemon.port
bind = "127.0.0.1"
state_dir = ".tracker/state"
```

The file is **optional** — without it, the daemon falls back
to loft's hardcoded defaults (back-compat for the loft repo
itself).  When present, it overrides per-project.

### `.tracker/` directory — per-project state

```
.tracker/
├── config.toml         # per-project config (above)
├── daemon.port         # daemon's listen port (written at startup)
├── daemon.pid          # PID for graceful shutdown
└── state/
    ├── tags.store      # mmap-backed binary index (primary)
    ├── tags.json       # JSON snapshot (back-compat, written periodically)
    └── tags.bin        # legacy binary serialised form
```

`.tracker/` lives at the project root; gitignored except for
`config.toml`.  Mirrors the `.git/` shape — opaque to the
project, owned by the tooling.

### mmap-backed index — `tags.store`

The daemon's primary index lives in an mmap'd file
(`.tracker/state/tags.store`), NOT in heap memory.  Two
properties this gives:

1. **Survives daemon restart.**  Killing the daemon
   (`SIGTERM`, `Ctrl-C`, OOM kill, machine reboot) does
   NOT lose the index.  The next `loft-index --start`
   re-mmaps the existing file and resumes.  Cold-start time
   is dominated by re-validating mtimes (skip files that
   haven't changed) rather than re-scanning everything.
2. **Zero-copy reads from clients.**  CLI + viewer don't
   need the daemon to serialise + send a JSON blob — they
   can mmap the same file read-only and walk the
   structured layout directly.  WebSocket transport is
   reserved for live notifications + dynamic queries
   (e.g., excerpts that need file reads).

The "survives daemon restart" property + integrity guarantee
are owned by **[plan-38 (loft-store-durable)](../future/38-loft-store-durable/README.md)**.
The indexer opens its store via:

```loft
store = store_durable::open(
    ".tracker/state/tags.store",
    DurabilityMode.IntegrityOnly,
    on_corruption_fn,         // → full rescan from filesystem
);
```

`IntegrityOnly` is plan-38's Tier 1 — appropriate for the
indexer because the filesystem is the source of truth and a
corrupted store rebuilds in <2 sec.  Game servers (TTT v5,
plan-36) opt into Tier 2 / Tier 3 of the same API.

#### Why loft's Store primitive is the right fit

Loft already has `Store` (`src/store.rs`), a word-addressed
heap with optional mmap backing (`feature = "mmap"` →
`MmapStorage`).  The plan-22 closures arc proved out the
rc + cascade-free model on top of stores.  An indexer
backed by a Store gets:

- mmap (when the feature compiles in) for free
- Structured layouts (vectors, sorted indices, hash
  tables) without serialisation glue
- The same `Parts::*` model the loft runtime uses
- ref-count + cascade-free for incremental updates
  (drop a file's tag refs without touching unrelated data)

The `tags.store` schema mirrors the JSON shape:

```
struct TagIndex {
    tags: hash<TagEntry[name]>,    // tag name → entry
    files: hash<FileEntry[path]>,  // file path → metadata + mtime
    broken: vector<BrokenRef>,
    snapshot_ts: integer
}

struct TagEntry {
    name: text,
    refs: vector<TagRef>            // sorted by (file, line)
}

struct TagRef {
    file: ref<FileEntry>,           // dedup'd via the files hash
    line: integer,
    context: text                   // single line, raw
}

struct FileEntry {
    path: text,
    mtime: integer,                 // for incremental rescan
    size: integer
}
```

#### Cold start

```
loft-index --start
  → mmap .tracker/state/tags.store (or create if absent)
  → walk project; for each scanned file:
      if file mtime > FileEntry.mtime → re-extract that file's tags
      else → keep existing entries
  → fs-event subscribe; serve clients
```

A full scan (cold) on the loft tree took 0.85 sec with the
bash scanner.  The mmap'd loft daemon should beat that for
incremental cases (only changed files re-extracted) and
match it for cold-cache cases.

#### File format compatibility

`tags.store` is a Loft Store binary blob — internal format
versioned via the existing Store signature (`StoreV01` per
`src/store.rs:215`).  `tags.json` is the cross-tool
exchange format (bash CLI + external consumers); written
on every full rebuild + on demand via
`loft-idx --snapshot`.

Daemon on shutdown: flushes any pending state to the mmap
(handled automatically by mmap's commit semantics) +
writes a fresh `tags.json` snapshot.  No data loss on
graceful exit.

### Single-binary distribution

The two loft binaries (`loft-index` daemon, `loft-idx` CLI)
become the only runtime artefacts.  Install one of:

1. **Per-user**: `cp tools/indexer/bin/loft-* ~/bin/` —
   user PATH picks them up.
2. **Per-system**: `sudo install -m 755
   tools/indexer/bin/loft-* /usr/local/bin/`.
3. **`make install`** target invokes the per-user copy.

No `jq`, no `bash` requirement at runtime.  The bash
artefacts (phases 00-03) stay shipped for projects that
choose not to install the loft binaries; they're an
opt-out.

### Multi-project daemon discovery

Each project runs its OWN daemon on its OWN port.  The CLI
discovers which daemon to talk to via:

1. `LOFT_TRACKER_PORT=NNNN ./loft-idx tag:@P259` — explicit
   override.
2. Walk up from `cwd` looking for `.tracker/daemon.port` —
   the file is the single source of truth for "which
   daemon serves THIS project."
3. If neither: print "no tracker daemon for this project —
   start with `loft-index --project /path/to/project`".

Two projects → two daemons on two ports → two `.tracker/
daemon.port` files.  No shared global registry; the
filesystem IS the registry.

### Daemon-per-project lifecycle

```
$ cd /path/to/project-A
$ loft-index --start                  # daemon starts, writes .tracker/daemon.port
$ cd /path/to/project-B
$ loft-index --start                  # different daemon, different port
$ loft-idx tag:@P259                  # finds project-B's daemon (cwd lookup)
$ cd /path/to/project-A
$ loft-idx tag:@P259                  # finds project-A's daemon
$ loft-index --stop                   # stops project-A's daemon
```

Daemons clean up `.tracker/daemon.{port,pid}` on graceful
shutdown.  A stale `.pid` (process gone) is recovered
automatically: the next `--start` overwrites it.

### Viewer integration

The viewer (plan-35) gains the same project-discovery
logic.  `loft-view --project /path/to/X` (or run from the
project directory) reads the same `.tracker/config.toml`
and connects to the same daemon.  One viewer per project,
typically forwarded to a different host port:

```
$ ssh -L 8765:localhost:8765 vm  # project A
$ ssh -L 8766:localhost:8766 vm  # project B
```

### Per-project tag conventions

The `[[tags.family]]` config means projects with non-loft
tag conventions work out of the box.  Examples:

- A Rails project tags issues `#1234` → one family entry.
- A JIRA-tracked project tags `PROJ-456` → another.
- A multi-repo monorepo can have several families.

The CLI / viewer surface stays uniform — `loft-idx tag:#1234`,
`loft-idx tag:PROJ-456`.  Per-family config drives the
extraction.

### Per-project validators

Same shape: `[validators.<family>]` blocks tell the daemon
how to validate refs.

- `type = "table-row"` — parse a markdown table for IDs.
- `type = "directory"` — globs that should resolve to dirs.
- `type = "github-api"` (future) — fetch open issues from
  GitHub, validate refs against the live list.
- `type = "linear-api"` (future) — same for Linear.

Phase 08 ships only `table-row` + `directory` (the loft
project's actual needs).  External-API validators are
sibling work.

## Critical files

| Path | Action |
|---|---|
| `tools/indexer/scan.loft` | EXTEND — read .tracker/config.toml; fall back to loft defaults if absent |
| `tools/indexer/idx.loft` | EXTEND — project-discovery via cwd walk |
| `tools/viewer/src/main.loft` | EXTEND — same project-discovery |
| `Makefile` | ADD `install:` target for `~/bin/` install |
| `doc/claude/plans/37-tracker-index/08-multi-project-deploy.md` | THIS FILE |
| `.tracker/config.toml` | NEW for the loft project — first consumer |

## Acceptance

- `make install` copies `loft-index`, `loft-idx`, `loft-view`
  to `~/bin/` (or configurable prefix).
- `loft-index --start` in any project root writes
  `.tracker/daemon.port` and starts indexing.
- `loft-idx tag:@P259` in any project's subdirectory finds
  that project's daemon (no global registry).
- A second project on the same machine runs its own daemon
  on a different port; their CLIs don't cross-talk.
- A project with `[[tags.family]]` for `#1234`-style issue
  refs indexes them correctly.
- A project WITHOUT `.tracker/config.toml` still works for
  the loft tag families (loft-aware fallback).
- Removing the loft repo's `.tracker/config.toml` doesn't
  regress the existing loft-project workflow.

## Risks

| Risk | Mitigation |
|---|---|
| Per-project config sprawl — every project needs to maintain a TOML | Sensible defaults: a project with no config gets the loft conventions out of the box.  Most projects need only a few `[[tags.family]]` entries. |
| Multi-daemon orchestration is complex | The filesystem IS the registry — no shared service to fail or scale.  Each daemon is independent. |
| Port collisions when many projects open simultaneously | `port = 0` default → kernel picks free; daemon writes the chosen port to `.tracker/daemon.port` |
| Stale `.tracker/daemon.{port,pid}` files | Next `--start` checks if PID is alive; overwrites if not |
| The single-binary install assumes loft is available globally | Build artefacts can be vendored — the binaries themselves are static once compiled.  `make install` documents the dependency. |

## Forward-looking — what comes after

Phase 08 ends the "build the tooling" arc.  Anything beyond
is application-of-tooling:

- Tagging conventions for non-loft projects (a small
  `examples/.tracker/config.toml.example` with annotations).
- A `loft-tracker init` command that scaffolds
  `.tracker/config.toml` with detected tag patterns.
- Statistics + history (a `loft-tracker stats` showing
  trend lines for tag count, broken-tag count, etc.).
- Multi-project dashboards (one viewer that talks to N
  daemons, presents an aggregate "all my projects" view).

These belong in a successor plan (or a sibling
`lib_plans/`) once phase 08 ships.

## Cross-references

- [Phase 07 — loft-native scanner + WebSocket daemon](07-loft-native-scanner.md) — the daemon this phase generalises
- [Plan-35 — branch review viewer](../35-branch-review-viewer/README.md) — viewer that consumes the daemon
- [PACKAGES.md](../../PACKAGES.md) — loft package format (relevant for future "tracker package" registry distribution)
