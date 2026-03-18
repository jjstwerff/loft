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
- [Splitting High-Effort Items](#splitting-high-effort-items)
- [Bytecode Economy](#bytecode-economy)
- [CI Validation](#ci-validation)
- [Commit Message Style](#commit-message-style)

---

## Branch Naming

A branch covers one or more PLANNING.md items (or phases of a single item).
Branch names list all item IDs, lowercased, with a short suffix:

```
{id}-{short-name}
{id}-{id}-{short-name}        # two items or phases
{id}-{id}-{id}-{short-name}   # three items or phases
```

IDs use the single-letter prefix scheme: `l1`, `p1`, `p1-1`, `a6`, `n2`, `r1`, `w1`.
Phase sub-steps use the dot notation lowercased: `p1-1`, `p1-2`, `a6-3`.

Group items in one branch only when they **touch overlapping files** — otherwise
keep them separate to make review straightforward.

Examples:

| Planning item(s) | Branch name |
|---|---|
| L2 — Nested match patterns | `l2-nested-match-patterns` |
| P1.1 + P1.2 + P1.3 — Lambda expressions (all 3 phases) | `p1-1-p1-2-p1-3-lambda-expressions` |
| A6.1 — Stack slot assign_slots standalone | `a6-1-assign-slots-standalone` |
| N2 + N3 + N4 — output_init/output_set/format fixes | `n2-n3-n4-output-fixes` |

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
git checkout -b p1-1-lambda-parser

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
git commit -m "WIP: P1.1 parser — lambda primary expression"
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

### Document findings before committing

When implementing a feature, you often discover things not in the planning:
limitations, edge cases, incorrect assumptions, or new issues.  **Update the
relevant documentation before including it in the commit:**

- **PROBLEMS.md** — new bugs or limitations discovered during implementation
- **PLANNING.md** — **remove the completed item entirely** (both the section and
  the Quick Reference row).  PLANNING.md is strictly for future work; completion
  history belongs in git and CHANGELOG.md.  If only part of an item was done,
  update the section to describe what remains.
- **NATIVE.md** — design corrections found during implementation
- **INCONSISTENCIES.md** — new language quirks discovered

Include these documentation updates in the docs commit at the end of the branch.
Do not wait until later — findings are freshest immediately after implementation.

When multiple PLANNING items share a branch — **including the individual phases of a
multi-phase item** — **each item or phase gets its own separate commit sequence**.
Do not collapse them into one big commit.  A reader bisecting the history must be
able to pin the change to a single item or phase.  Mention the item ID in every
commit message that belongs to it (e.g. `P1.1: …`, `P1.2: …`, `N2: …`).

### Commit message style

```
{scope}: {imperative summary}  (≤ 72 characters)

{body: describe what the feature does in plain language.  Focus on the
user-visible or developer-visible effect, not the implementation.
Mention function or file names only when they clarify the scope.}

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
```

**Scope** is one of:
- `L1`, `P1`, `P1.1`, `A6`, `A6.2`, `N2`, `W1` etc. — planned item or phase
- `fix` — bug fix not tied to a planned item
- `docs` — documentation-only change
- `refactor` — behaviour-neutral code change

**Summary** starts with an imperative verb: *add*, *fix*, *implement*, *remove*,
*enable*, *warn on* — never *added*, *adds*, *implementing*.

**Body** explains what changed and why in clear sentences.  Avoid listing every
file or function touched — the diff shows that.  Use a function name only when
it is the thing being fixed or added (e.g. "fix `output_if` to emit typed nulls")
rather than as implementation detail.

**Good example:**
```
N6.1: implement vector iteration in codegen_runtime

Vector `for` loops now emit an index-based loop using a dedicated
`_iter` counter variable rather than relying on the interpreter's
generic iterate path.  This is the first of three N6 phases; sorted
and reverse iteration follow in N6.2 and N6.3.
```

**Bad example:**
```
N6.1: fix codegen_runtime.rs vector loop

Changed emit_for_vector() at line 412 to add _iter variable and emit
OpGetInt/OpSetInt for the counter. Added match on IterKind::Vector in
three places. Updated output_step() at line 531 to check _iter against
vec_len. Added OpBranchFalse at end of loop body.
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

## Splitting High-Effort Items

Any item rated **Medium–High or higher** in PLANNING.md must be split into
sub-steps before work begins.  A sub-step is a change that:

1. **Passes all three CI checks on its own** (`cargo test`, `cargo clippy -- -D warnings`,
   `cargo fmt -- --check`).
2. **Has at least one test** that was written before the implementation (Step 1 of the
   structured sequence) and enabled immediately after (Step 3).
3. **Leaves the codebase in a better or equal state** — no sub-step may introduce a
   regression, a dead code path, or a half-working feature visible to loft programs.

### How to split

Look for **natural seams** in the planned work.  Good split boundaries:

| Seam | Example |
|---|---|
| Independent areas of the codebase | Parser change + runtime change → two commits |
| Phases of a larger design | A8 destination-passing: Phase 1 compiler, Phase 2 native rewrites |
| Feature flags / opt-in paths | Implement behind a `#[cfg(test)]` stub, then wire it in |
| Layers of correctness | Guard first (panic on bad input), full fix second |
| Subset of cases | Handle the common case first, edge cases in follow-up commits |

If no natural seam exists and the item genuinely cannot be split, document why in the
PLANNING.md item before starting.  This is the exception, not the rule.

### Update PLANNING.md before starting

When splitting a High or Very High item, **rewrite its Fix path section** in
PLANNING.md to list the sub-steps explicitly before the first commit lands.  This:

- Makes the plan reviewable before any code is written.
- Gives future sessions enough context to resume mid-item without re-deriving the plan.
- Forces a check that each sub-step is independently testable — if you cannot write a
  test for a sub-step, the split boundary is wrong.

Example: A8 (destination-passing for text-returning natives) was already split into
phases (compiler, native rewrites, format expressions, scratch buffer removal) in
PLANNING.md before implementation began.  Each phase is independently testable because
existing string tests catch regressions and new tests verify the new calling convention.

### Size budget

A single commit should rarely exceed **~200 lines of non-test Rust**.  If a sub-step
exceeds this, look for a smaller seam.  Large diffs are hard to review, hard to bisect,
and statistically more likely to contain regressions.

---

## Structured Commit Sequence

For each item (or each independent area of a single item) follow the commit order
below.  It is **not required for trivial one-file fixes** — the only hard
requirement is that every commit passes the three checks above.

When a branch contains multiple items **or multiple phases of one item**, repeat
the sequence once per item/phase before writing the shared documentation commit
at the end.  Each phase is treated as an independent item: it has its own test
commit, code commit, and enable-tests commit.

```
[P1.1 — Step 1] tests with #[ignore]
[P1.1 — Step 2] code change
[P1.1 — Step 3] enable tests
[P1.2 — Step 1] tests with #[ignore]
[P1.2 — Step 2] code change
[P1.2 — Step 3] enable tests
[P1.3 — Step 1] tests with #[ignore]
[P1.3 — Step 2] code change
[P1.3 — Step 3] enable tests
[Step 4] any refactors (shared or per-phase)
[Step 5] docs: update PLANNING, PROBLEMS, CHANGELOG for all phases
```

### Step 1 — Tests with `#[ignore]`

Add only the new test file(s) or test functions, with every new test marked
`#[ignore]`.  The `#[ignore]` annotation keeps CI green before the implementation
lands, while making the intent of the tests clear from the first commit.

```rust
#[test]
#[ignore = "P1.1: parser for lambda expressions not yet implemented"]
fn lambda_basic_parse() { ... }
```

Commit message:

```
P1.1: add lambda parser tests (initially ignored)

lambda_basic_parse, lambda_with_return_type, lambda_in_map_call.
All marked #[ignore] until the parser extension lands.
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
| Bytecode generation | `src/state/codegen.rs`, `src/fill.rs` — see [Bytecode Economy](#bytecode-economy) |
| Scope and variable analysis | `src/scopes.rs`, `src/variables.rs` |

Example split for P1.2 (two areas):

**Commit 2a** — IR synthesis:
```
P1.2: synthesise anonymous def for lambda in compile.rs

Lambda expressions are lowered to a `Value::Def` with a generated
name. compile.rs emits the def-nr as an integer constant at the
call site. No codegen changes yet.
```

**Commit 2b** — codegen emission:
```
P1.2: emit def-nr for lambda in codegen.rs

codegen.rs recognises `Value::Lambda` and emits `OpPushInt` with the
def-nr, completing the compile-to-bytecode path for inline lambdas.
```

When there is only a single area, one commit is fine.

Verify after each commit: `cargo build --all-targets` must succeed.

### Step 3 — Enable Tests

Remove the `#[ignore]` annotations from all tests added in Step 1.  No other
changes.

```
P1.1: enable lambda parser tests

All three tests now pass. Removes the #[ignore] markers added in the
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

parse_binary_operator exceeded 55 lines after the L3 constant-zero check.
Extract the new check into its own function per CODE.md § Functions.
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
docs: P1 lambda expressions — update CHANGELOG, PLANNING, LOFT, STDLIB

- CHANGELOG: add P1 feature entry under Unreleased
- PLANNING: remove P1 section (all three phases complete)
- LOFT.md: document lambda syntax in the Declarations section
- STDLIB.md: document map/filter/reduce accepting lambda arguments
```

Verify: `cargo test` still passes (documentation changes are non-functional).

---

## Bytecode Economy

**Never add a new opcode if the problem can be solved by composing existing
opcodes.**  New opcodes increase the `OPERATORS` array size, the opcode
dispatch surface, and the maintenance burden in `fill.rs`, `codegen.rs`, and
`02_images.loft`.

Before proposing a new opcode, check whether the compiler can emit a sequence
of existing opcodes to achieve the same result.  For example, `insert(v, idx,
elem)` reuses the existing `OpInsertVector` (creates space) followed by the
appropriate `OpSetInt`/`OpSetLong`/`OpSetFloat`/`OpSetSingle` (writes the
value) — no new opcode needed.

Only add a new opcode when:
- No existing opcode sequence can express the operation (e.g. a fundamentally
  new runtime primitive like `OpSortVector` that cannot be decomposed).
- Performance is critical and the overhead of multiple opcodes is measurable
  and unacceptable (document the benchmark).

---

## GitHub Issues and Releases — Hard Limits

**Never create or update GitHub issues.**  All planning, status, and design
information lives in the committed documentation (`doc/claude/`).  Interested
contributors can read it there.  Duplicating it into GitHub issues creates a
second source of truth that drifts from the real one.

**Never trigger or automate a release.**  Every release requires a manual
validation phase (see [RELEASE.md](RELEASE.md)) that cannot be automated:
hands-on testing of pre-built binaries on each platform, review of the
CHANGELOG, and a deliberate version-bump decision.  Do not push release tags,
trigger release workflows, or draft GitHub Releases programmatically.

---

## CI Validation

Push the branch and open a pull request against `main`:

```bash
git push -u origin p1-1-p1-2-p1-3-lambda-expressions
gh pr create --title "P1: lambda expressions (all 3 phases)" \
             --body "Implements fn(params)->type block inline lambdas with map/filter/reduce integration."
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

## Renaming a Branch After Completion

When a branch ends up implementing different items than originally planned (e.g.
you started with `l2-nested-patterns` but ended up doing `l2-p3-nested-patterns-aggregates`
instead), rename the branch before pushing the PR so the name reflects the actual
work:

```bash
# Rename the local branch
git branch -m old-name new-name

# If already pushed under the old name, delete the old remote and push the new one
git push origin --delete old-name
git push -u origin new-name
```

The branch name appears in the merge commit and PR title.  A misleading name
makes history harder to navigate.  Rename before opening the PR, not after.

---

## See also

- [CODE.md](CODE.md) — Naming conventions, function-length rules, clippy policy, null sentinels
- [TESTING.md](TESTING.md) — Test framework, `code!` / `expr!` macros, LogConfig debug presets
- [PLANNING.md](PLANNING.md) — Backlog, version milestones, effort estimates
- [PROBLEMS.md](PROBLEMS.md) — Open bugs; update here when fixing a known issue
- [RELEASE.md](RELEASE.md) — Gate criteria and release checklist
