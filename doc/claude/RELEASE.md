# Release Planning

## What "0.9.0" and "1.0.0" mean

**0.9.0 — Production-ready standalone executable.**
The interpreter is complete, stable, and efficient enough to rely on for real programs.
All planned language features (lambdas, aggregates, nested patterns, full parallel support)
are present.  No known crashes or silent wrong results.  Pre-built binaries ship for all
four platforms.

**1.0.0 — Stable language + fully working IDE.**
1.0.0 is the **stability contract**: any program valid on 1.0.0 compiles and runs
identically on any 1.0.x or 1.x.0 release.  The contract covers:
- The core language surface (syntax, type system, documented stdlib API, CLI flags).
- The public IDE API (WASM `compileAndRun` / `getSymbols` JS interface).
- The interpreter does not panic or silently produce wrong results.
- A user can write, run, and share a real program — from the terminal or the browser.

The Web IDE (W1–W6) is part of 1.0.0, not post-1.0.  See [PLANNING.md § Milestone
Reevaluation](PLANNING.md#milestone-reevaluation) for the full reasoning.

---

## Gate Items — MUST for 1.0

These block a 1.0 release because they cause panics on valid programs, ship incorrect public identity, or leave public keywords in a permanently-broken state.

Completed gate items (T0-1 through T0-7, T1-5, PROBLEMS #10, #37–#40, A4 pre-gate,
Cargo.toml, README, CHANGELOG, CI pipeline, R1) are recorded in CHANGELOG.md.

No open gate items remain.  All known crashes on valid programs have been fixed.

---

## Nice-to-Have for 1.0

Include if bandwidth exists before tagging 1.0; ship without if they push the date out significantly.

| Item | Value | Effort |
|---|---|---|
| **T1-2** wildcard imports (`use mylib::*`) | Genuine friction removal; medium payoff | Medium |
| **T1-4** match expressions | Largest language feature gap; makes language feel complete | High |
| **T2-0** code formatter (`loft --format`) | Professional tooling polish; zero correctness risk | Small–Medium |
| HTML reference on GitHub Pages | Users can read docs without cloning the repo | Small (CI step) |
| Pre-built release binaries | Users can install without Rust toolchain | Small (GitHub Actions matrix) |

If T1-4 ships in 1.0, update `doc/09-enum.html` and add `doc/21-match.html`.
If T1-4 does not ship in 1.0, INCONSISTENCY #6 must be prominently documented as a known limitation in CHANGELOG.md and the HTML reference.

---

## Items by milestone

### 0.9.0 gate items

| Item | Notes |
|---|---|
| L1 error recovery | Cascading errors after one bad token; high UX impact |
| P1 lambda expressions | Core language completeness; unblocks P3 and A5 |
| P3 vector aggregates | Stdlib completeness; depends on P1 |
| L2 nested match patterns | Language completeness |
| A9 vector slice CoW | Correctness: mutating a slice must not corrupt parent |
| A6 stack slot pre-pass | Architectural: eliminates slot-conflict category of bugs |
| A8 destination-passing for strings | Efficiency: eliminates double-copy in format expressions |
| A3 optional Cargo features | Lean binary; clean dependency management |
| Tier N (N2–N9, N1) native codegen fixes | Efficiency: turn existing but broken generator into working `--native` path |
| ~~A1 parallel workers full~~ | ~~Feature completeness for existing parallel construct~~ *(done — all return types supported incl. struct/ref, both interpreter and native)* |
| TR1 stack trace introspection | `stack_trace()` stdlib; prerequisite for coroutines |
| A7 native extension libraries | `#native` annotation + `cdylib` loading for external packages |

### 1.0.0 gate items (on top of 0.9.0)

| Item | Notes |
|---|---|
| R1 workspace split | Prerequisite for WASM target |
| W1 WASM foundation | Enables all other IDE work |
| W2 editor shell | Visible IDE |
| W3 symbol navigation | Go-to-definition, find-usages |
| W4 multi-file projects | IndexedDB persistence |
| W5 docs/examples browser | Integrated documentation |
| W6 export/import + PWA | Offline support; closes the loop |

### Explicitly 1.1+

| Item | Notes |
|---|---|
| P2 REPL | Browser IDE covers the interactive use case; revisit if needed |
| A2 logger production mode | Low user impact until logger is widely used |
| A4 spacial<T> full implementation | After pre-gate added in 0.8.0 |
| A5 closure capture | Very high effort; depends on P1 |

---

## Open Inconsistencies for 1.0

Of the 6 still-open entries in INCONSISTENCIES.md, none are hard blockers, but the following need documentation coverage before 1.0:

| Entry | Action |
|---|---|
| #6 — plain enums cannot have methods | Document as known limitation if T1-4 (match) is deferred; resolved by T1-4 if included |
| #10 — sizeof(u8) returns 4 | Document as accepted behaviour in LOFT.md (compiler minimum alignment) |
| Others | Verify each is documented in LOFT.md / INCONSISTENCIES.md; no code change needed |

---

## Project Structure Changes

### For 1.0 — no crate split needed

The current single-crate layout is correct for the project's scale.  A Cargo workspace split is warranted only when W1 (WASM) starts, so that the `loft-core` library can use `crate-type = ["cdylib","rlib"]` without affecting the CLI binary.

### Cargo.toml changes before 1.0

```toml
[package]
name        = "loft"          # ✓ done 2026-03-15
version     = "1.0.0"             # bump at release
description = "loft — interpreter for the loft scripting language"  # ✓ done 2026-03-15
homepage    = "https://github.com/jjstwerff/loft"  # ✓ done 2026-03-15
repository  = "https://github.com/jjstwerff/loft"  # ✓ done 2026-03-15
keywords    = ["language", "interpreter", "scripting"]  # ✓ done 2026-03-15
categories  = ["command-line-utilities", "compilers"]   # ✓ done 2026-03-15
```

**Note:** `rand_core` and `rand_pcg` are actively used in `src/native.rs` for random number generation — do **not** remove them.  The earlier claim that they were unused was wrong.

**Note on renaming to "loft":** ~~Do it now.~~  **Done 2026-03-15.**  Renaming was free because the package had not yet been published to crates.io.

### Future workspace layout (for W1)

```
Cargo.toml                  (workspace root)
loft-core/              (Cargo.toml: crate-type = ["cdylib","rlib"])
  src/
loft-cli/               (Cargo.toml: [[bin]])
  src/main.rs
loft-gendoc/            (Cargo.toml: [[bin]])
  src/gendoc.rs
default/                    (standard library .loft files)
tests/
doc/
ide/                        (web IDE — added at W1)
```

---

## No Automated Releases

**Releases must never be created or triggered automatically.**  Every release
requires a human validation phase (the checklist below) that cannot be scripted:
hands-on testing of pre-built binaries on each platform, review of the CHANGELOG,
and a deliberate decision to tag and publish.

Do not push release tags, trigger release workflows, draft GitHub Releases, or
run `cargo publish` programmatically.  Always wait for the owner to do this
manually after completing the validation checklist below.

---

## Pre-Release Documentation Review

Run these steps before tagging a release.  They are manual; treat each as a gate item.

### 1 — Audit doc/claude/ for stale problem documentation

- Open PROBLEMS.md: every bug entry there should either be open or clearly crossed out / labelled FIXED with the fix date.  Remove entries that are fixed and already recorded in CHANGELOG.md.
- Open PLANNING.md: every item should be open.  Done items must have been removed (not marked done in-place) before this release.
- Open project_status.md in memory/: verify it reflects current state.

### 2 — Verify code links in doc/claude/

Walk every file in `doc/claude/` looking for references of the form `src/foo.rs`, `src/foo/bar.rs`, function names, struct names, or opcode names.  For each:
- Confirm the file/symbol still exists at that path/name.
- Update any that have moved or been renamed.

Helpful command: `grep -rn 'src/' doc/claude/` and cross-check against `ls src/`.

### 3 — Verify doc/claude/ discoverability

- Every file in `doc/claude/` must be reachable from at least one other file or from the MEMORY.md index.
- Files that are only referenced from MEMORY.md should still link to at least one sibling document.
- Orphaned files (nothing links to them) must be added to an existing doc or removed.

### 4 — Compact verbose sections

Read through any doc/claude/ file that has grown since the previous release and identify passages that are longer than necessary (e.g. multi-paragraph context that can be reduced to a bullet list, repeated caveats, implementation notes already captured in CHANGELOG.md).  Shorten these in place.

### 5 — Validate user documentation against this release

For each feature and bug-fix entry in CHANGELOG.md under `[Unreleased]`:
- Find the corresponding section in the HTML reference (a file in `tests/docs/*.loft` or `doc/`).
- Confirm the user-visible behaviour is correctly described.
- If the feature has no user documentation, add it (either a new `.loft` example or an update to an existing one).

### 6 — Validate DEVELOPERS.md caveats and language-comparison pages

- **`doc/DEVELOPERS.md`**: re-read the compiler pipeline description and all "caveat" or "known limitation" callouts.  Update any that are stale relative to source changes in this release.
- **`doc/00-vs-rust.html`** and **`doc/00-vs-python.html`**: verify that the claims in each comparison table remain accurate for the current language surface (null safety, type inference, collection API, etc.).  Update any cell that no longer holds.

### 7 — Validate user documentation topic flow

- Open `doc/` and list all `NN-*.html` files in order.
- Read the first sentence of each page and verify the sequencing makes sense for a reader progressing top-to-bottom (introductory concepts before advanced ones).
- If a topic added in this release landed at the end of the sequence but logically belongs earlier, renumber and update all cross-links.

### 8 — Validate coding standards and clean up clippy suppressions

```bash
cargo clippy -- -D warnings
```

All warnings must be errors-free.  Additionally, review every `#[allow(clippy::...)]`
annotation in the codebase and attempt to remove it by fixing the underlying code:

```bash
grep -rn "#\[allow(clippy::" src/
```

For each suppression found:
- If the function has been refactored or shortened since the annotation was added, remove
  the `#[allow]` and verify clippy still passes.
- If the suppression covers a genuine structural constraint (e.g. a dispatch function that
  cannot be split without losing clarity), keep it and add a brief comment explaining why.

The goal is to keep suppressions intentional and minimal, not to accumulate them as a
release-over-release debt.

### 9 — Generate HTML and PDF

```sh
# Regenerate HTML reference
cargo run --bin gendoc

# Compile PDF
typst compile doc/loft-reference.typ
```

Verify that `gendoc` completes without warnings and that the generated HTML files look correct in a browser.  Attach `loft-reference.pdf` to the GitHub release.

---

## Release Artifacts Checklist

| Artifact | Required | How |
|---|---|---|
| GitHub release tag `v1.0.0` | Yes | `git tag v1.0.0` |
| Linux static binary (`x86_64-unknown-linux-musl`) | Yes | GitHub Actions + `cross` |
| macOS Intel binary (`x86_64-apple-darwin`) | Yes | GitHub Actions matrix |
| macOS ARM binary (`aarch64-apple-darwin`) | Yes | GitHub Actions matrix |
| Windows binary (`x86_64-pc-windows-msvc`) | Recommended | GitHub Actions matrix |
| `loft-reference.pdf` attached to release | Yes | `typst compile doc/loft-reference.typ` |
| HTML docs on GitHub Pages | Recommended | `cargo run --bin gendoc` → `gh-pages` branch (automated in release.yml) |
| crates.io publish as `loft` | Recommended | `cargo publish` (automated in release.yml via `CARGO_REGISTRY_TOKEN`) |
| `loft.1` man page | Optional | Generate from README with `pandoc` |

---

## Post-1.0.0 Versioning Policy

**Semantic versioning with a roughly monthly release cadence:**

- **1.0.x patch** — bug fixes only; no new language features; no behaviour changes; always backward-compatible.  Example: fix a crash found after 1.0.0 ships.
- **1.x.0 minor** — new language features that are strictly additive (new syntax, new stdlib functions, new CLI flags, new IDE capabilities).  Any program valid on 1.0.0 must compile and run identically on 1.x.0.  Candidates: P2 (REPL), A5 (closures), A7 (native extensions), Tier N (native codegen).
- **2.0** — reserved for breaking language changes.  Not expected in the near term.

The stability guarantee applies to the **loft language surface** (syntax, type system, documented stdlib, CLI flags) and the **public IDE API** (`compileAndRun` / `getSymbols` JS interface).  The Rust library API (`lib.rs`) is not a public stable API until explicitly stabilised.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog; source for gate-item IDs
- [ROADMAP.md](ROADMAP.md) — Items grouped by milestone with effort estimates
- [DEVELOPMENT.md](DEVELOPMENT.md) — Branch naming, commit sequence, and CI workflow
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — All known inconsistencies must be resolved or accepted before 1.0.0
