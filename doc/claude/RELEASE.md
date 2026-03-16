# Release Planning

## What "1.0" means

1.0 is a **stability contract**, not a completeness contract.  It means:
- Programs valid on 1.0 will compile and run identically on any 1.x release.
- The core language surface (syntax, type system, documented stdlib API, CLI flags) is frozen; new features are additive only.
- The interpreter does not panic or silently produce wrong results on any program that passes the type-checker.
- A user can write and ship a real program using the documented features.

The feature-completeness interpretation (every planned feature ships in 1.0) is wrong for a single-developer project; chasing it risks never shipping.  Items like match expressions (T1-4), REPL (T2-2), and the Web IDE (Tier W) are valuable but do not prevent a user from writing correct programs today.

---

## Gate Items — MUST for 1.0

These block a 1.0 release because they cause panics on valid programs, ship incorrect public identity, or leave public keywords in a permanently-broken state.

| Item | Why it blocks 1.0 |
|---|---|
| ~~**T0-1** — `null` literal in scalar field assignment crashes `set_int`~~ | **FIXED 2026-03-15** — `parse_assign_op` now calls `convert()` to substitute the typed-null constant; `debug_assert` boundary check added in `generate_call`; five regression tests in `tests/issues.rs`. Introduced T0-3 regression (type-guarded separately). |
| **T0-2** — LIFO store-free panic (PROBLEMS #37) | Panics at runtime whenever a function has 2+ owned refs in the same scope; 9+ tests fail including `structs`, `enums`, `vectors`, `collections`, `threading`. Fix: one-line `res.reverse()` in `scopes.rs::variables()`. |
| **T0-3** — T0-1 regression: `sorted`/`hash`/`index` key-null removal silently broken (PROBLEMS #38) | `collection[key] = null` does nothing; collection retains all elements. Fix: guard `convert()` call to scalar types only in `parse_assign_op`. |
| **T0-4** — `v += other_vec` shallow copy: text fields in appended struct elements become dangling (PROBLEMS #39) | Panics "Unknown record N" at runtime for any `vector<S>` append where S has text/ref fields. Fix: call `copy_claims` per element in `vector_add`. |
| **T0-5** — `index<T>` struct field: `OpCopyRecord`/`OpClear` panic (PROBLEMS #40) | `copy_claims`/`remove_claims` in `allocation.rs` have no `Parts::Index` arm. Fix: add Index arms to both functions. |
| ~~**T1-5** — validate_slots false-positive panics~~ | **FIXED 2026-03-13** — `find_conflict` exempts same-name/same-slot pairs; P1 pre-init handles ref-typed vars across sequential blocks. |
| ~~**PROBLEMS #10** — garbage format-slot crash~~ | **FIXED 2026-03-15** — `vars.defined` moved inside `has_token("=")` guard. |
| ~~**T3-4 pre-gate** — `spacial<T>` keyword unimplemented~~ | **FIXED 2026-03-15** — emits compile error `"spacial<T> is not yet implemented"`; `spacial_not_implemented` test added. |
| ~~**Cargo.toml identity**~~ | **FIXED 2026-03-15** — crate renamed `loft`; `description`, `homepage`, `repository`, `keywords`, `categories` all set correctly. |
| ~~**README.md rewrite**~~ | **DONE 2026-03-15** — README.md created: one-liner, features, hello-world, install options, known limitations, license. |
| ~~**CHANGELOG.md**~~ | **DONE 2026-03-15** — CHANGELOG.md created: 0.1.0 entry with full language feature list, stdlib summary, known limitations, and unreleased section tracking open T0 items. |
| ~~**GitHub Actions CI + release pipeline**~~ | **DONE 2026-03-15** — `.github/workflows/ci.yml` (test on ubuntu/macos/windows, clippy -D warnings, fmt check) + `release.yml` (4-platform binaries, gh-pages, crates.io). Zero clippy warnings; all tests pass. |
| ~~**T0-7** — `16-parser.loft` `generate_call` size mismatch (PROBLEMS #42)~~ | **FIXED 2026-03-16** — Root cause was `Code.define()` in `lib/code.loft` storing `res: i32` into `hash<Definition[name]>`. Fixed: store a full Definition; fix `get_type()` to use `definitions[nr].typedef`; fix `structure()` to restore `cur_def` after `type_def()`; fix `object()` inverted loop condition; use integer null-check to avoid `ConvRefFromNull` store leak. All 28 wrap tests pass; `16-parser.loft` removed from `SUITE_SKIP`. |
| **R1** — create standalone `loft` GitHub repository | New public repo with correct identity; current repo contains game-engine history. |

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

## Explicitly 1.1+

| Item | Notes |
|---|---|
| T2-1 lambda expressions | Depends on T1-1 (done); natural 1.1 item |
| T2-2 REPL | High effort; not blocking basic usability |
| T3-1 parallel workers extra args / text returns | Deferred in THREADING.md |
| T3-2 logger production mode | Low user impact until logger widely used |
| T3-3 optional Cargo features | Architectural cleanup; no user-visible gap |
| T3-4 spacial<T> full implementation | 1.1+ after pre-gate removal in 1.0 |
| T3-5 closure capture | Very high effort; depends on T2-1 |
| Tier W web IDE | Parallel independent track; no 1.0 dependency |

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

### 8 — Generate HTML and PDF

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

## Post-1.0 Versioning Policy

**Semantic versioning with a roughly monthly release cadence:**

- **1.0.x patch** — bug fixes only; no new language features; no behaviour changes; always backward-compatible.  Example: fix a crash found after 1.0 ships.
- **1.x.0 minor** — new language features that are strictly additive (new syntax, new stdlib functions, new CLI flags).  Any program valid on 1.0 must compile and run identically on 1.x.  Examples: match expressions (T1-4), wildcard imports (T1-2), formatter (T2-0), lambdas (T2-1), REPL (T2-2).
- **2.0** — reserved for breaking language changes.  Not expected in the near term.

The stability guarantee applies to the **loft language surface** (syntax, type system, documented stdlib, CLI flags).  The Rust library API (`lib.rs`) is not a public stable API until explicitly stabilised.
