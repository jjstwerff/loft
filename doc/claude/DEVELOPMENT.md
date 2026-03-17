# Development Workflow

Step-by-step process for taking a PLANNING.md item from backlog to merged.

---

## Contents
- [Branch Naming](#branch-naming)
- [Development Phase — Single WIP Commit](#development-phase--single-wip-commit)
- [Validation Against CODE.md](#validation-against-codemd)
- [Rebase into a Clean Commit History](#rebase-into-a-clean-commit-history)
  - [Step 1 — Tests with `#[ignore]`](#step-1--tests-with-ignore)
  - [Step 2 — Code Changes](#step-2--code-changes)
  - [Step 3 — Enable Tests](#step-3--enable-tests)
  - [Step 4 — Structural Refactors](#step-4--structural-refactors)
  - [Step 5 — Documentation](#step-5--documentation)
- [CI Validation](#ci-validation)
- [Commit Message Style](#commit-message-style)

---

## Branch Naming

One branch per PLANNING.md item.  Branch names mirror the item ID, lowercased,
with a short suffix that identifies the feature:

```
t{tier}-{nr}-{short-name}
```

Examples:

| Planning item | Branch name |
|---|---|
| T1-2 — Wildcard imports | `t1-2-wildcard-imports` |
| T2-6 — `now()` and `ticks()` | `t2-6-time-functions` |
| T2-11 — `loft.toml` package layout | `t2-11-package-layout` |

Create the branch from the tip of `main`.  **Always start from a clean, up-to-date
`main`** — if you are on a different branch, check for uncommitted documentation
changes first and carry them over:

```bash
# 1. Check for uncommitted changes on the current branch
git status --short

# 2. If doc/claude/*.md files were modified, save them before switching
git stash push -m "doc changes" -- doc/claude/ CHANGELOG.md

# 3. Switch to main and pull the latest merge
git checkout main
git pull

# 4. Create the new feature branch
git checkout -b t2-6-time-functions

# 5. Restore the documentation changes into the new branch
git stash pop
```

If the stash conflicts (the same doc was modified in main), resolve manually:
keep the main version for sections you did not write, keep your additions.

Skip steps 2 and 5 when there are no uncommitted documentation changes.
Never create a feature branch from another feature branch.

---

## Development Phase — Single WIP Commit

Work in a single "work-in-progress" commit until all tests pass.  Combine code
changes and their tests in one place so they can be reviewed and debugged together.

```bash
# Stage all changed files (code + tests together)
git add -p          # or: git add <specific files>
git commit -m "WIP: T2-6 now() and ticks()"
```

As work progresses, amend the WIP commit rather than stacking new ones:

```bash
git add <changed files>
git commit --amend --no-edit
```

Verify locally at any point:

```bash
cargo build --all-targets        # must succeed
cargo test                       # all tests must pass (ignoring any that were
                                 # already ignored on main)
cargo clippy -- -D warnings      # must be clean — same flags CI uses; the
                                 # Makefile's clippy target uses -W (warn only)
                                 # and will not catch errors that fail CI
cargo fmt -- --check             # must produce no diff; run `cargo fmt` to fix
```

---

## Validation Against CODE.md

Before rebasing, check new code against every rule in [CODE.md](CODE.md):

| Check | Command | Exception |
|---|---|---|
| No clippy warnings | `cargo clippy -- -D warnings` | Skip pre-existing `too_many_lines` and `cognitive_complexity` violations in functions you did not write — fixing them would disrupt unrelated code and obscure the feature diff |
| Formatted | `cargo fmt -- --check` | None |
| Naming conventions | Manual review | `n_<name>` for global natives; `t_<LEN><Type>_<method>` for methods |
| Function length | `cargo clippy` | If **new** code you wrote triggers `too_many_lines`, move the refactor to Step 4 of the rebase rather than mixing it with the functional change |
| Null sentinels | Manual review | Any new numeric function returning null must use `i32::MIN` / `i64::MIN` / `f64::NAN`, never `0` |

The line-count and complexity exceptions exist because fixing these in files
touched incidentally by a feature would inflate the diff and make the real change
hard to review.  Such refactors belong in a dedicated commit (Step 4) if they are
necessary, or left for a separate cleanup task if they are pre-existing.

---

## Commit Rules

A branch may contain **any number of commits** as long as every commit satisfies:

```bash
cargo test                       # all tests pass
cargo clippy -- -D warnings      # no warnings
cargo fmt -- --check             # no formatting diff
```

Run all three before every `git commit`.  A commit that breaks any of these must
be fixed or amended before pushing.

### Commit structure

Each commit should be a coherent, self-contained change.  Good splits:

- Code change + its tests in one commit
- Documentation updates in a separate commit
- Refactors that don't change behaviour in their own commit

Multiple PLANNING items may share a branch when they touch the same files
(e.g. `n11-n14-runtime-fixes`).  Mention all item IDs in the commit message.

### Commit message style

```
{scope}: {imperative summary}  (≤ 72 characters)

{body: describe what the feature does in plain language.  Focus on the
user-visible or developer-visible effect, not the implementation.
Mention function or file names only when they clarify the scope.}

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
```

**Scope** is one of: `T{tier}-{nr}` or `N{nr}` for planned items, `fix` for
bug fixes, `docs` for documentation-only, `refactor` for behaviour-neutral changes.

**Summary** starts with an imperative verb: *add*, *fix*, *implement*, *remove*,
*enable*, *warn on* — never *added*, *adds*, *implementing*.

**Body** explains what changed and why in clear sentences.  Avoid listing every
file or function touched — the diff shows that.  Use a function name only when
it is the thing being fixed or added (e.g. "fix `output_if` to emit typed nulls")
rather than as implementation detail.

**Good example:**
```
N15: emit typed nulls for missing else branches

Generated if-expressions without an else branch now produce a
type-appropriate null sentinel (i32::MIN, "", NaN, etc.) instead of
unit `()`.  This fixes 20 compile failures where the true branch
returned a value but the else emitted an incompatible type.
```

**Bad example:**
```
N15: fix output_if in src/generation.rs

Changed output_if() at line 828 to call infer_if_type() which checks
Value::Call and Value::Var and Value::Block result types. Added match
on Type::Integer, Type::Long, Type::Float, Type::Single, Type::Boolean,
Type::Text, Type::Reference, Type::Enum. Updated output_code_inner()
line 747 to not emit "()" when the context requires a typed null.
```

### Documentation commit

The **last commit** on a branch updates documentation:

```
docs: {ID} — update CHANGELOG, PLANNING

- CHANGELOG: add feature/fix entry under Unreleased
- PLANNING: remove completed item section and quick-reference row
```

Review every file in `doc/claude/` for references to the feature and update as needed.

---

## Optional: Structured Commit Sequence for Medium+ Items

For larger features, the following commit order makes review easier.
It is **not required** — the only requirement is that every commit passes
the three checks above.

### Step 1 — Tests with `#[ignore]`

Add only the new test file(s) or test functions, with every new test marked
`#[ignore]`.  The `#[ignore]` annotation keeps CI green before the implementation
lands, while making the intent of the tests clear from the first commit.

```rust
#[test]
#[ignore = "T2-6: not yet implemented"]
fn now_is_positive() { ... }
```

Commit message:

```
T2-6: add time-function tests (initially ignored)

now_is_positive, now_is_not_null, ticks_is_non_negative, ticks_is_monotonic.
All marked #[ignore] until the native functions are implemented.
```

Verify: `cargo test` must pass with the new tests reported as ignored, not failed.

### Step 2 — Code Changes

Stage only the implementation files.  If the feature touches multiple independent
areas of the codebase, split this step into one commit per area.  Common split
boundaries:

| Area | Typical files |
|---|---|
| Standard library | `src/native.rs`, `default/*.loft` |
| Database / runtime state | `src/database/*.rs` |
| Parser | `src/parser/*.rs`, `src/lexer.rs` |
| Bytecode generation | `src/state/codegen.rs`, `src/fill.rs` |
| Scope and variable analysis | `src/scopes.rs`, `src/variables.rs` |

Example split for T2-6 (two areas):

**Commit 2a** — database field:
```
T2-6: add start_time field to Stores

Initialised at Stores::new(); cloned into worker Stores by
clone_for_worker() so all parallel threads share the same
program-start reference point.
```

**Commit 2b** — native functions and stdlib declaration:
```
T2-6: implement now() and ticks() native functions

n_now: milliseconds since Unix epoch via SystemTime::UNIX_EPOCH.
n_ticks: microseconds since program start via stores.start_time.
Declared in default/02_images.loft under a new // --- Time --- section.
```

When there is only a single area, one commit is fine.

Verify after each commit: `cargo build --all-targets` must succeed.

### Step 3 — Enable Tests

Remove the `#[ignore]` annotations from all tests added in Step 1.  No other
changes.

```
T2-6: enable time-function tests

All four tests now pass. Removes the #[ignore] markers added in the
initial test commit.
```

Verify: `cargo test` must pass with zero ignored tests among the new ones.

### Step 4 — Structural Refactors

If the implementation introduced new code that violates CODE.md line-length or
complexity limits, extract the required helpers or split the functions here.
If no such refactoring is needed, skip this step entirely.

This commit must be **behaviour-neutral**: the test suite must still pass
unchanged after this commit.

```
Refactor: split parse_binary_operator — extract check_constant_zero helper

parse_binary_operator now exceeded 55 lines after the T1-11 constant-zero
check. Extract the new check into its own function per CODE.md § Functions.
```

Verify: `cargo test` unchanged; `cargo clippy -- -D warnings` clean.

### Step 5 — Documentation

Documentation changes **must be in their own commit**, separate from code,
tests, and refactors.  Never mix doc edits with any of Steps 1–4.

Review **every file in `doc/claude/`** for references to the feature or affected
behaviour and update them as needed.  Common files to check:

| File | Update when |
|---|---|
| `CHANGELOG.md` | Always — add a feature or bug-fix entry under Unreleased |
| `PLANNING.md` | Always — remove the item section and Quick Reference row |
| `RELEASE.md` | Gate criteria or release checklist changed |
| `PROBLEMS.md` | A known bug was fixed or a new one was discovered |
| `STDLIB.md` | A standard-library function was added or changed |
| `EXTERNAL_LIBS.md` | Library resolution or manifest handling changed |
| `INCONSISTENCIES.md` | A documented language inconsistency was resolved |
| Any other `doc/claude/*.md` | File explicitly describes the feature area |

Stage all files that required a change:

```
docs: T2-6 now()/ticks() — update CHANGELOG, PLANNING, STDLIB

- CHANGELOG: add T2-6 feature entry under Unreleased
- PLANNING: remove T2-6 section and quick-reference row; update 1.0 target list
- STDLIB.md: document now() and ticks() in the Time section
```

Verify: `cargo test` still passes (documentation changes are non-functional).

---

## CI Validation

Push the branch and open a pull request against `main`:

```bash
git push -u origin t2-6-time-functions
gh pr create --title "T2-6: add now() and ticks() time functions" \
             --body "Implements wall-clock now() and monotonic ticks()."
```

The CI pipeline (`.github/workflows/ci.yml`) runs three jobs in parallel:

| Job | Command | Must pass |
|---|---|---|
| Test (ubuntu, macOS, windows) | `cargo test` | All platforms |
| Clippy | `cargo clippy -- -D warnings` | Zero warnings |
| Format | `cargo fmt -- --check` | No diff |

Do not merge until all three jobs are green on all platforms.  If a job fails:

- **Test failure on one platform only** — usually a path-separator or timing
  issue; reproduce with `cargo test` locally in a container or VM.
- **Clippy failure** — a lint that is a warning locally becomes an error under
  `-D warnings`.  The Makefile's `make test` uses `-W` (warn only) so it will
  not catch these.  Run `cargo clippy -- -D warnings` locally, fix all errors,
  and push again.
- **Format failure** — run `cargo fmt` locally, verify with `cargo fmt -- --check`,
  amend the relevant commit, and push again.

---

## See also

- [CODE.md](CODE.md) — Naming conventions, function-length rules, clippy policy, null sentinels
- [TESTING.md](TESTING.md) — Test framework, `code!` / `expr!` macros, LogConfig debug presets
- [PLANNING.md](PLANNING.md) — Backlog, version milestones, effort estimates
- [PROBLEMS.md](PROBLEMS.md) — Open bugs; update here when fixing a known issue
- [RELEASE.md](RELEASE.md) — Gate criteria and release checklist
