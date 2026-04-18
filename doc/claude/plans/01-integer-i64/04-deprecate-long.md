# Phase 4 — C54.B: remove `long` + `l` literal suffix

Status: **not started** — blocked by Phase 2.

## Context

Once `integer` is i64 (Phase 2), `long` is a redundant alias and `10l`
is meaningless.  Per QUALITY.md § 455-462 and the release-timing note:

- **Deprecate at 0.9.0**: parser accepts `long` / `l` but emits a
  warning diagnostic pointing at the migrator.
- **Remove at 1.0.0**: hard error.
- **Repo migration is immediate** on the C54.B branch — stdlib, lib,
  tests all switch to `integer` / plain literals to avoid a second
  sweep at 1.0.0.

## Scope

### Compiler

- `src/lexer.rs`: emit deprecation warning on `l` literal suffix
  token; still accept it.
- `src/parser/definitions.rs`: same for `long` type keyword.
- Both warnings reference `--migrate-long` and a doc link.

### Migration tool

`loft --migrate-long <path>`:
- Rewrites `.loft` source files.
- `long` → `integer` (type position only — don't touch identifiers
  that happen to contain "long").
- `10l` / `10L` → `10`.
- Conservative AST-aware rewrite; refuses to rewrite files it can't
  fully parse.
- Dry-run flag.

### Stdlib / lib / tests sweep

Immediate on the branch:
- `default/*.loft` — every `long` reference switched.
- `lib/*.loft` — every lib replaces with `integer`.
- `tests/*.loft`, `tests/scripts/*.loft`, `tests/docs/*.loft` — same.
- Run full workspace suite after each file group to catch regressions
  early.

## Test plan (from QUALITY.md § 460-462)

Un-ignore:
- `c54b_long_type_deprecated`
- `c54b_l_literal_deprecated`
- `c54b_long_migration_tool`
- `c54b_stdlib_no_long`

Plus: `grep -rn "\\blong\\b\\|[0-9]l\\b" default/ lib/ tests/` returns
empty post-sweep.

## Risk

The migration tool has to parse `.loft` well enough to not rewrite
identifiers.  If the conservative AST-aware approach outgrows
~300 LoC, open `04a-migration-tool-design.md`.

## Budget

**360-480 minutes** — the sweep alone can be hundreds of files.

## Deliverables

- Deprecation warnings emitted.
- Migration tool committed with round-trip fixtures.
- Stdlib / lib / tests sweep committed.
- Tests un-ignored.
- ROADMAP.md 0.9.0 entry updated: `long` deprecated; 1.0.0 entry: `long`
  removed.
