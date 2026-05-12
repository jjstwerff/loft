<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 06 — Cleanup + doc + deferred follow-ups

**Status: open.**

## Goal

After phases 01–05 land, finalise the user-facing surface:

- Document the REPL + introspection tool in CLAUDE.md, README,
  CHANGELOG.
- Add a `--help` for the `repl` subcommand (currently auto-documented
  by the argv parser; expand to a proper help block).
- Delete any scaffolding files / dead code from earlier phases.
- File deferred follow-ups in `PROBLEMS.md` so they don't get
  lost.

## Documentation

### CLAUDE.md

Add a "Key commands" entry:

```bash
loft introspect myprogram.loft       # bytecode + Rust + slots dump
loft repl                            # interactive prompt
```

Add a doc index entry pointing at `doc/claude/REPL.md` (new).

### `doc/claude/REPL.md` (new)

User-facing reference: how the REPL works, command list, session
file format (if implemented), troubleshooting.  Mirror style of
`doc/claude/STDLIB.md` / `doc/claude/TESTING.md`.

### `doc/claude/INTROSPECTION.md` (new)

Reference for the introspection tool: flag reference, output
format, integration with `LOFT_LOG` (which still works).  ~2 pages.

### CHANGELOG.md

User-facing entry under the next release:

```
### Added

- **REPL** — interactive `loft repl` prompt with persistent state
  across inputs, multi-line block support, and `:cmd` introspection
  commands.  See doc/claude/REPL.md.
- **Introspection tool** — `loft introspect <file>` dumps bytecode,
  generated Rust, and per-function variable slot tables side-by-
  side.  See doc/claude/INTROSPECTION.md.
```

### CHANGELOG_TECHNICAL.md

Add per-phase technical entries (one per phase, summarising the
internal changes — appendable bytecode, statement parser entry,
state reset API, etc.).

## Deferred follow-ups

These get filed in `PROBLEMS.md` (or carried as ROADMAP items):

| ID | Description | Tier |
|----|-------------|------|
| **REPL.W** | WASM browser playground — wraps the REPL in a web UI; the parse + execute machinery already runs in WASM (modulo `rustyline` swap for browser-side input). | Tier 3 |
| **REPL.S** | Save / restore session — serialise `__repl_session` + user fn defs to a `.loftrc` file; `:save` / `:load` commands restore. | Tier 3 |
| **REPL.C** | Tab completion — fn names, locals, struct field names.  Hooks into the parser's symbol table. | Tier 2 |
| **REPL.H** | Syntax highlighting in the prompt — `syntect`-based or hand-rolled.  Polish, not blocking. | Tier 3 |
| **INSP.D** | Diff-mode introspection — `loft introspect --diff a.loft b.loft` shows bytecode / slot deltas across two versions. | Tier 3 |
| **INSP.J** | JSON output mode for the introspection tool (machine-readable for IDE integration). | Tier 2 |

## Cleanup checklist

- [ ] Delete any temporary `tests/data/repl_*.loft` fixtures that
  have golden equivalents in `tests/golden/`.
- [ ] Verify `cargo doc` builds without warnings on the new
  modules (`introspect`, `repl`).
- [ ] Run `cargo clippy --release --all-targets`; fix any new
  warnings that landed across phases 01–05.
- [ ] Confirm `make ci` is green at the phase-06 commit.
- [ ] Update `doc/claude/QUICK_START.md` (if it mentions
  development tools) to point at the new commands.

## Acceptance criteria

1. New documentation files (`REPL.md`, `INTROSPECTION.md`) exist
   and are linked from CLAUDE.md's documentation index.
2. CHANGELOG entries written.
3. PROBLEMS.md / ROADMAP.md updated with deferred items.
4. `cargo doc` clean.
5. `cargo clippy --release --all-targets` clean.
6. `make ci` green.

## Effort

**XS (~half day).**  Pure documentation + cleanup.

## See also

- [README.md](README.md) — plan-08 index.
- All prior phase docs (00–05).
