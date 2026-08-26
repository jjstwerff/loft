<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# lib-plan 28 — `loft test --deps`: walk a project's dep tree

**Status:** COMPLETE — T1-T3 + T6 shipped 2026-05-28; T4 `--lock=PATH`, T5
`--skip=name,name` and the `--strict-deps` design note shipped 2026-08-25.

A dependency now resolves through four sources in a fixed order — a path dep, an
explicit `--lock` pin, a sibling directory, then the project's own `loft.lock`.
Ranking the implicit lock LAST is what made T4 additive: a multi-package repo
keeps testing its sibling working copies instead of silently switching to
published tarballs out of the cache, while the lock fills the cross-repo hole
that made `--deps` print *"registry resolution not yet wired"* and test nothing.
Measured before the change: of 13 dependency edges across the eight library
repos, 8 resolved and 5 — every cross-repo one — did not.

Two things found while building it, both fixed here rather than filed:

- A code comment promised a `LOFT_DENY_WARNINGS_DEPS=1` opt-in that was read
  nowhere.  `--strict-deps` is that opt-in, and it now exists.
- The design note's own promise did not hold: `--no-warnings` silences the
  PRINTING of a warning, while whether one is FATAL is read from the environment
  the child inherits — so a consumer exporting `LOFT_DENY_WARNINGS=1` was failed
  by lint debt inside a package it does not own.  The dep run now neutralises
  that variable unless `--strict-deps` asks for it.

Cells: `tests/dep_test_walk.rs`.

## Why

A loft project that depends on `lib/graphics`, `lib/server`, etc.
today has no built-in way to say "run my tests AND the tests of
everything I depend on, against the same loft toolchain version."
`loft test` runs the current package only.  `make test-packages`
walks `lib/*/` but is monorepo-internal — it doesn't read the
project's `loft.toml` to scope down to actually-used deps.

Adding the same surface to consumer projects makes:

- **Dep upgrades safe.**  Update `loft.lock`, run `loft test --deps`,
  see if anything in the resolved tree regresses.
- **Chunk-repo CI thorough.**  The
  [`library-ci.yml.example`](../12-library-extraction/library-ci.yml.example)
  gate runs each library's OWN tests; `--deps` would also run its
  transitive deps' tests against the current loft, catching the
  "this version of graphics breaks gridmesh's tests in our environment"
  failure mode that today only the consumer's CI catches (post-release).
- **Pre-flight upgrades.**  `loft test --deps --lock=candidate.lock`
  tests against a candidate lockfile before committing it.

## What we have already

| Piece | Where | Use |
|---|---|---|
| `loft.toml [dependencies]` | per-package | direct deps with version or `{ path = "..." }` |
| `manifest::extract_path_dep()` | `src/manifest.rs:118` | parses path-style dep values |
| `loft.lock` | per project | resolved transitive tree, pinned by version (written by `loft install`) |
| Multi-strategy dep resolver | `src/parser/mod.rs` (`probe_user_installed`, `probe_registry_dir`, `lib_path_manifest`) | already searches `<project>/lib/`, `--lib` dirs, `~/.loft/lib/`, `~/.loft/registry/<id>-<version>/` |
| `loft test [name]` | `src/main.rs:1630` | runs a single package's tests |
| `make test-packages` | `Makefile:812` | walks `lib/*/` + runs `loft test` per package — proves the pattern, monorepo-scoped |

## What's missing

A walker that: given a `loft.toml`, resolves each declared dep to a
directory, runs `loft test` if it has a `tests/` directory, recurses
transitively, avoids cycles.  Plus the CLI surface that drives it.

## Proposed surface

```
loft test --deps                  # transitive — all deps + their deps
loft test --deps=direct           # one level only — current project's direct deps
loft test --deps --lock=PATH      # use a specific lockfile (pre-flight)
loft test --deps --skip=name,name # exclude packages (e.g. known-broken on this platform)
```

`--deps` IMPLIES `--no-warnings` for transitive deps unless
`--strict-deps` is also passed — the consumer should not be penalised
by warnings inside a dep it doesn't control.  The current project's
own tests still honour `LOFT_DENY_WARNINGS` as before.

## Implementation phases

| # | What ships | Effort | Notes |
|---|---|---|---|
| T1 | Lift `probe_*` resolvers from `Parser` into a free function `manifest::resolve_dep(name, value, from_pkg) -> Option<PathBuf>` | XS | Pure refactor; no behaviour change |
| T2 | `loft test --deps[=direct]` CLI flag + direct-deps walker | S | Calls existing `run_tests()` per dep dir; reports rolled-up pass/fail per dep |
| T3 | Transitive walk + cycle detection | XS | `HashSet<String>` of visited package names |
| T4 | `--lock=PATH` to drive from a specific lockfile | XS | Read lock, iterate version-pinned entries, resolve to `~/.loft/registry/<id>-<version>/` |
| T5 | `--skip=` allow-list | XS | Filter the dep set before walking |
| T6 | Library-CI integration — `library-ci.yml.example` gains a final `--deps` step (default: transitive but `--no-warnings` for deps) | XS | Edit the template; document in lib_plans/12 |

## What's NOT in this library

- **`cargo test --workspace` semantics for path deps in unrelated
  projects.**  This walks the dep TREE of one project, not an
  arbitrary list of paths.
- **Continuous-integration runner.**  Reuses the existing test
  runner; doesn't reimplement parallelism / nextest-style output.
- **Build-cache awareness.**  Each dep's tests are run fresh; cargo
  cache covers Rust-side reuse.

## Cross-references

- [lib_plans/12-library-extraction](../12-library-extraction/README.md)
  — primary consumer; ships the
  [`library-ci.yml.example`](../12-library-extraction/library-ci.yml.example)
  template that would gain a final `--deps` step.
- `loft.lock` format — see PKG_REGISTRY.md.
- `make test-packages` — the monorepo precedent for this pattern.
