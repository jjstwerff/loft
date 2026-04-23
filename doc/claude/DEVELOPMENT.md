
# Development Workflow

Step-by-step process for taking a PLANNING.md item from backlog to merged.

**Session start:** Review [CLAUDE.md](../../CLAUDE.md) at the project root — it contains the project overview, architecture, branch policy, and documentation index.

---

## Contents
- [Branch Naming](#branch-naming)
- [Development Phase — Single WIP Commit](#development-phase--single-wip-commit)
- [Validation Against CODE.md](#validation-against-codemd)
- [Structured Commit Sequence](#structured-commit-sequence)
  - [Step 1 — Tests with `#[ignore]`](#step-1--tests-with-ignore)
  - [Step 2 — Code Changes](#step-2--code-changes)
  - [Step 3 — Enable Tests](#step-3--enable-tests)
  - [Step 4 — Structural Refactors](#step-4--structural-refactors)
  - [Step 5 — Documentation](#step-5--documentation)
- [Splitting High-Effort Items](#splitting-high-effort-items)
- [Bytecode Economy](#bytecode-economy)
- [CI Validation](#ci-validation) — local gate (before every commit) + remote CI (after push)
- [Commit Message Style](#commit-message-style)

---

## Branch Policy — Main is Read-Only

**Direct commits to `main` are not allowed.**

`main` is the release branch; every commit on it must be releasable.  All
development happens on feature branches and reaches `main` only through a
reviewed, CI-green pull request.

Rules:
- Never `git commit` directly on `main`.
- Never `git push` without an explicit user instruction.
- **Never create a branch unless the user explicitly says "create a branch".**
  Do not create branches as part of a workflow, sprint start, or task planning.
  Work on the current branch and commit locally.  The user decides when to
  branch, push, or open a PR.
- Never create a feature branch from another feature branch — always branch from `main`.
- Merging to `main` is done via a GitHub pull request, not a local `git merge`.

---

## Sprint Branches

Development is organized into **sprints** (see [ROADMAP.md](ROADMAP.md) for
the sprint plan).  Each sprint gets **one branch** containing up to ~4 items.
The branch is merged to main via a single PR when all items pass CI.

### Why sprints, not per-item branches

- A sprint groups related items that touch overlapping files (e.g. PKG.1 +
  PKG.2 + PKG.6 all touch `compile.rs` and `main.rs`).
- Fewer PRs = less CI wait time and merge churn.
- Each commit within the branch is still one coherent item (test + code +
  enable), so `git log` stays bisectable.

### Sprint branch naming

```
sprint-{N}-{short-description}
```

Examples:
- `sprint-1-pkg-infrastructure`
- `sprint-2-stdlib-extraction`
- `sprint-4-http-client`

### Sprint workflow

**Every sprint branch MUST start from a merged, up-to-date `main`.**
If the previous sprint's PR has not been merged yet, wait for it.
Never branch from another feature branch.

**Do not create the branch yourself.**  Wait for the user to say
"create a branch" — then follow the naming convention below.

```
1. Merge the previous sprint's PR (wait for CI green)
2. (user creates branch from main)
3. For each item in the sprint (up to ~4):
   a. Write tests with @EXPECT_FAIL / @EXPECT_ERROR
   b. Implement the code change
   c. Remove annotations, verify tests pass
   d. Commit: "{ID}: {description}"
4. Update all relevant documentation (see checklist below)
5. cargo fmt && cargo clippy --tests -- -D warnings && cargo test
6. (user says "push" → git push -u origin {branch})
7. (user says "create PR" → gh pr create)
8. Wait for CI green on all 3 platforms
9. (user says "merge" → gh pr merge --squash)
```

### Announce each step — MANDATORY

**State the name of every step as you start or finish it.**  This applies to
the sprint workflow, individual items, and sub-steps within each item.
Always include the issue/item ID when one exists.

Examples:
- "Starting H4.1: HttpResponse struct"
- "Starting H4.1: writing test for http_get"
- "Finished H4.1 — interpreter + native tests pass"
- "Starting: clippy fixes for loft_register_v1 refactor"
- "Finished: clippy clean, 0 warnings"
- "Starting step 5: documentation updates"
- "Finished step 6: CI green, 548 passed"

**Why:** silent progress is invisible progress.  The user cannot see tool
calls in real time — they only see text output.  Naming each step gives the
user a running status line so they know where things stand, can interrupt
early if the plan is wrong, and can resume efficiently if context runs out.

**Why this matters:** branching from an unmerged feature branch creates
a dependency chain.  If the earlier branch needs changes during review,
the later branch must be rebased — causing merge conflicts and wasted
work.  Sequential merges keep the history linear and each PR reviewable
in isolation.

### Item limit per sprint

**Target: ~4 items per branch.** This keeps PRs reviewable (<500 lines of
non-test code) and limits blast radius if something goes wrong.  A sprint
with fewer than 4 items is fine — never pad a sprint to reach the target.

If an item turns out larger than expected, split the sprint: merge what's
done, create a new branch for the remainder.

### Documentation updates — MANDATORY per sprint

**Every sprint must update all documentation affected by its changes before
the PR is created.**  Documentation is not a follow-up task — it ships with
the code.  **Never create a separate docs branch or PR** — documentation
commits belong in the same sprint branch as the code they describe.

#### Checklist (step 5 in the sprint workflow)

Run through this list before pushing.  Skip items that are clearly unaffected.

| Document | Update when… |
|---|---|
| `CHANGELOG.md` | Always — add entries under `## Unreleased` for every user-visible change |
| `doc/claude/ROADMAP.md` | Sprint items were completed or reprioritised |
| `doc/claude/PLANNING.md` | Items were completed (remove) or new items discovered (add) |
| `doc/claude/PROBLEMS.md` | Bugs were fixed (mark resolved) or **any new bug found during the sprint** (add with reproducer) |
| `doc/claude/CAVEATS.md` | Edge cases were fixed or **any new workaround discovered** (add with test reference) |
| `doc/claude/TESTING.md` § Coverage Gaps | Test coverage improved or new gaps identified |
| `README.md` | New user-facing features, CLI commands, or examples added |
| Feature design doc (e.g. `PACKAGES.md`, `OPENGL.md`) | Implementation diverged from design, or phases completed |
| `doc/claude/STDLIB.md` | New stdlib functions or types added |
| `doc/claude/LOFT.md` | Language syntax or semantics changed |
| `doc/claude/INTERNALS.md` | New opcodes, state changes, or native functions added |
| `.claude/skills/loft-write/SKILL.md` | New patterns, caveats, or conventions for writing `.loft` files |

**Filing bugs is not optional.** Every workaround, test simplification, or
failure encountered during the sprint — even if worked around — must be
filed in PROBLEMS.md or CAVEATS.md with a reproducer.  Unfiled bugs get
rediscovered in future sprints, wasting time.

**Why this matters:** stale documentation causes wasted time in future
sessions.  Claude reads these docs at session start — if they describe
features that don't exist yet or omit features that do, the first 10 minutes
of the next session are spent rediscovering the current state.  Keeping docs
in sync with code is cheaper than reconstructing context later.

---

## Branch Naming

For non-sprint branches (bug fixes, documentation, one-off tasks), use
item ID + short suffix:

```
{id}-{short-name}
{id}-{id}-{short-name}        # two items
```

IDs use the single-letter prefix scheme: `l1`, `p1`, `p1-1`, `a6`, `n2`, `r1`, `w1`.
Phase sub-steps use the dot notation lowercased: `p1-1`, `p1-2`, `a6-3`.

Examples:

| Planning item(s) | Branch name |
|---|---|
| L2 — Nested match patterns | `l2-nested-match-patterns` |
| P1.1 + P1.2 + P1.3 — Lambda expressions (all 3 phases) | `p1-1-p1-2-p1-3-lambda-expressions` |
| A6.1 — Stack slot assign_slots standalone | `a6-1-assign-slots-standalone` |
| N2 + N3 + N4 — output_init/output_set/format fixes | `n2-n3-n4-output-fixes` |

Branches are created from the tip of `main`.  **Do not create branches
yourself** — wait for the user to ask.  When they do:

1. Commit any uncommitted work on the current branch first.
2. `git checkout main && git pull`
3. `git checkout -b {branch-name}` (only when the user says to)

Never use `git stash`.  Never create a feature branch from another
feature branch.

---

## Development Phase

For **trivial one-file fixes** (e.g. a single clippy suppression, a doc typo),
work directly without a structured commit sequence — just run the local CI gate
before committing.

For **all planned items** (anything in PLANNING.md with an ID), follow the
[Structured Commit Sequence](#structured-commit-sequence) below.  Do not collapse
a planned item into a single amending WIP commit; bisectability and item-traceability
require separate commits for tests, implementation, and docs.

Verify locally at any point using the full CI gate:

```bash
make ci       # fmt → clippy → test; stops at first failure; full output in result.txt
```

The order matters: `cargo fmt --check` and `cargo clippy --tests -- -D warnings` run
first so formatting and lint errors are fixed before the slower `cargo test` runs.
If `make` is unavailable, run the three commands manually in the same order:

```bash
cargo fmt -- --check                    # no formatting diff; run `cargo fmt` to fix
cargo clippy --tests -- -D warnings     # zero warnings, including test code
cargo test                              # all tests pass
```

---

## Validation Against CODE.md

Before committing, check new code against every rule in [CODE.md](CODE.md):

| Check | Command | Exception |
|---|---|---|
| No clippy warnings | `cargo clippy --tests -- -D warnings` | Skip pre-existing `too_many_lines` and `cognitive_complexity` violations in functions you did not write — fixing them would disrupt unrelated code and obscure the feature diff |
| Formatted | `cargo fmt -- --check` | None |
| Naming conventions | Manual review | `n_<name>` for global natives; `t_<LEN><Type>_<method>` for methods |
| Function length | `cargo clippy` | If **new** code you wrote triggers `too_many_lines`, move the refactor to Step 4 of the commit sequence rather than mixing it with the functional change |
| Null sentinels | Manual review | Any new numeric function returning null must use `i32::MIN` / `i64::MIN` / `f64::NAN`, never `0` |

The line-count and complexity exceptions exist because fixing these in files
touched incidentally by a feature would inflate the diff and make the real change
hard to review.  Such refactors belong in a dedicated commit (Step 4) if they are
necessary, or left for a separate cleanup task if they are pre-existing.

---

## Commit Rules

A branch may contain **any number of commits** as long as every commit satisfies the
local CI gate — see [CI Validation](#ci-validation) for the exact commands.  In short:

```bash
make ci
```

Run this **before every `git commit`** (including amends).  A commit that breaks
any of these must be fixed before the session ends.  Never rely on the remote CI to
catch failures that could have been caught locally.

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

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
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

1. **Passes all three CI checks on its own** (`make ci`).
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

Verify: `make run-tests` must pass with the new tests reported as ignored, not failed.

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
| Scope and variable analysis | `src/scopes.rs`, `src/variables/` |

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

Verify after each commit: run `make ci` — all three checks must pass.

### Step 3 — Enable Tests

Remove the `#[ignore]` annotations from all tests added in Step 1.  No other
changes.

```
P1.1: enable lambda parser tests

All three tests now pass. Removes the #[ignore] markers added in the
initial test commit.
```

Verify: `make run-tests` must pass with zero ignored tests among the new ones.

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

Verify: `make run-tests` unchanged; `cargo clippy --tests -- -D warnings` clean.

### Step 5 — Documentation

Documentation changes **must be in their own commit**, separate from code,
tests, and refactors.  Never mix doc edits with any of Steps 1–4.

Review **every file in `doc/claude/`** for references to the feature or affected
behaviour and update them as needed.  Common files to check:

| File | Update when |
|---|---|
| `CHANGELOG.md` | Always — add a feature or bug-fix entry under Unreleased |
| `PLANNING.md` | Always — remove the item section and Quick Reference row |
| `ROADMAP.md` | Always — remove or update the row(s) for the completed item(s) |
| `RELEASE.md` | Gate criteria or release checklist changed |
| `PROBLEMS.md` | A known bug was fixed or a new one was discovered |
| `STDLIB.md` | A standard-library function was added or changed |
| `PACKAGES.md` | Library resolution or manifest handling changed |
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

Verify: `make run-tests` still passes (documentation changes are non-functional).

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

**When you do add one**, follow the 10-step bootstrap procedure in
[`plans/02-narrow-collection-elements/04b-short-encoding.md` §
Opcode-addition procedure](plans/02-narrow-collection-elements/04b-short-encoding.md#opcode-addition-procedure-verified-2026-04-22).
Short summary: add Store methods first, declare in `default/01_code.loft`,
grow the `OPERATORS` array in `src/fill.rs` to match, append placeholder
identifiers + empty function bodies, build, then `cargo test --test issues
regen_fill_rs -- --ignored --nocapture` to regenerate `src/fill.rs`.
Rebuild the WASM rlib and the `native_pkg` fixture cdylib (the
freshness checks in `tests/html_wasm.rs` and `tests/native_loader.rs`
catch this).  Audit `src/codegen_runtime.rs` manually — regen does
NOT touch it.  **Run `cargo test --release --test native native_dir`
before commit** to catch the silent-hang class of regression that
bit the P184 Phase 4b attempt on 2026-04-21.  Never reorder existing
opcode declarations while adding new ones — opcode numbers are
positional and any reorder invalidates stored / pre-compiled
packages that embed them.

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

CI validation has two distinct phases: a **mandatory local gate** that must pass before
every commit, and the **remote CI** that GitHub runs after a push.  Most failures happen
because the local gate is skipped.

### Pre-existing vs. newly-introduced failures — always irrelevant

**A red CI gate is a red CI gate, regardless of who or what made it red.**
The working tree must be stable and usable after every commit.  It does
not matter whether a `cargo fmt --check` diff, a clippy error, a
`no-default-features` build break, or a test failure was already present
on the base branch before your work, was caused by a toolchain upgrade,
or was introduced by the current change.  If `make ci` is red when you
reach for `git commit`, **fix it first** — then commit.

The reasoning is flat: downstream contributors, CI runs, and future you
cannot distinguish "broken by this commit" from "broken by a prior
commit" from the working-tree symptom alone.  Leaving a red gate in
place forces every later contributor to diagnose it from scratch, and
turns every future `make ci` run into noise that hides genuinely-new
regressions.  A clean gate is a shared resource; "it wasn't me" is not
a reason to leave it dirty.

Practical shape:

- A toolchain upgrade surfaced new clippy lints on existing code → fix
  them (apply the suggestion, restructure the doc comment, or add a
  scoped `#[allow(...)]` with a comment explaining *why* the lint is a
  false positive).  Do not revert the toolchain.
- `cargo fmt --check` reports drift in a file you did not touch → run
  `cargo fmt` and commit the result.  The bundled drift lands alongside
  your intended change; flag it in the commit message so reviewers can
  recognise the stylistic churn.
- A pre-existing dead-code warning fires because of a build-profile
  cfg gate → gate the item with the same cfg, or `#[allow(dead_code)]`
  with a comment pointing at the gated call site.
- A test regresses on your branch *and* on `origin/main` → the fix
  blocks your commit either way.  If the root cause is clearly outside
  your work's scope, land a minimal repair in a separate preparatory
  commit (still on your branch) before the feature commit.

This policy is stricter than "don't introduce regressions."  It is
"leave the gate cleaner than you found it, every time."

### Local CI gate (mandatory before every commit)

Run all four checks and confirm they are clean **before** `git commit`.  Never commit
when any check fails — fix first, then commit.

```bash
make ci   # fmt → clippy → test in order; stops at first failure; output in result.txt
```

Or run the checks individually:

```bash
cargo fmt --check                              # 1. formatting
cargo clippy --tests -- -D warnings            # 2. pedantic lints as errors
cargo check --no-default-features              # 3. feature-gated code compiles
cargo test                                     # 4. all tests pass
```

**All four checks are required.** Skipping any one causes CI failures after push.

These are the same checks the remote CI runs.  Running them locally catches errors that
would otherwise only surface after a push, which cannot be taken back.

#### Common pitfalls

| Pitfall | Why it fails CI | How to avoid |
|---------|----------------|--------------|
| Running `cargo clippy` without `-D warnings` | Project uses `#![warn(clippy::pedantic)]` in `lib.rs` and `main.rs`; CI promotes pedantic warnings to errors | Always use `cargo clippy --tests -- -D warnings` |
| Skipping `--no-default-features` check | CI tests feature-gated builds; `#[cfg(feature = "...")]` on imports and functions must be correct for stripped builds | Always run `cargo check --no-default-features` |
| Running `cargo test` but not `cargo fmt --check` | `cargo test` does not check formatting | Run fmt check first |
| Adding `#[cfg(feature = "X")]` to `FUNCTIONS` table entries | Changing registration order causes `library_names` index mismatch — tests crash with "index out of bounds" | Use `#[cfg]` on array entries to preserve order but conditionally include them |
| New files with crypto/FFI constants | SHA-256 K-tables, base64 lookup tables trigger `unreadable_literal`, `many_single_char_names`, `cast_lossless` pedantic lints | Add `#[allow(clippy::...)]` on the specific function or constant |
| Stale WASM rlib after touching core sources | `cargo test` never rebuilds `target/wasm32-unknown-unknown/release/libloft.rlib`. A `--html` or `html_wasm::*` test will fail with rustc errors citing line numbers from an older source (e.g. `cr_rand_int` at the pre-migration position) or `E0599` on methods that were renamed. | After any change to `src/codegen_runtime.rs`, `src/ops.rs`, or the stack layout: `cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random`. Do NOT use `--features wasm` for the `--html` rlib — that pulls in wasm-bindgen and the resulting bundle imports from `__wbindgen_placeholder__`, breaking Node-stub instantiation. |
| Stale `tests/lib/native_pkg/native` fixture cdylib | The fixture `.so` is not rebuilt automatically by `cargo test`, `make ci`, or the CI workflow. A signature change to the fixture source (e.g. the C54 `*const i32` → `*const i64` swap) is invisible until `native_loader::*` tests mis-read memory and report "expected N, got M" from a pre-rebuild `.so`. | After editing `tests/lib/native_pkg/native/src/lib.rs` or after any Phase-migration change that shifts vector element layout: `cd tests/lib/native_pkg/native && cargo build --release`. |

#### When to run

- Before every `git commit` (including amends)
- Before reporting a branch as done
- After any stash pop or cherry-pick that brings in new code

#### Workflow: push first, test in parallel

To save wall-clock time, push the branch and create the PR **before** running
the local test suite.  CI starts immediately on the remote while the local
tests run in parallel:

```bash
git push -u origin <branch>       # 1. push
gh pr create --title "..." ...     # 2. create PR (CI starts)
cargo test                         # 3. local tests (runs in parallel)
```

This avoids waiting for local tests before discovering remote CI failures.
However, the full local gate (fmt + clippy + no-default-features) must still
pass **before** pushing.

If `cargo clippy --tests -- -D warnings` reports errors for violations that were already present on `main` and in
code you did not write, suppress them with `#[allow(...)]` on the specific function —
see [Validation Against CODE.md](#validation-against-codemd) for the exception policy.

### Remote CI / Pull Request

Once the local gate is clean and the user asks to push, open a pull request against `main`.
Do **not** push automatically — wait for an explicit instruction:

```bash
git push -u origin p1-1-p1-2-p1-3-lambda-expressions
gh pr create --title "P1: lambda expressions (all 3 phases)" \
             --body "Implements fn(params)->type block inline lambdas with map/filter/reduce integration."
```

The CI pipeline (`.github/workflows/ci.yml`) runs five jobs:

| Job | Command | Must pass |
|---|---|---|
| Format | `cargo fmt -- --check` | No diff |
| Clippy | `cargo clippy --tests -- -D warnings` | Zero warnings |
| Test (ubuntu) | `cargo check --no-default-features` then `cargo test` | Both pass |
| Test (macOS) | `cargo check --no-default-features` then `cargo test` | Both pass |
| Test (windows) | `cargo check --no-default-features` then `cargo test` | Both pass |

Do not merge until all three jobs are green on all platforms.  If a job fails:

- **Test failure on one platform only** — usually a path-separator or timing
  issue; reproduce with `cargo test` locally in a container or VM.
- **Clippy failure** — a lint that passes locally may become an error under
  `-D warnings` if it was suppressed or not triggered.  The Makefile's `make test`
  uses `-W` (warn only) and will not catch these.  Run
  `cargo clippy --tests -- -D warnings` locally, fix all errors, and push again.
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
```

If the branch was already pushed under the old name, the remote must be updated —
but only when the user explicitly instructs a push:

```bash
# Only on explicit user instruction:
git push origin --delete old-name
git push -u origin new-name
```

The branch name appears in the merge commit and PR title.  A misleading name
makes history harder to navigate.  Rename before opening the PR, not after.

---

## Debugging a Regression — MANDATORY APPROACH

### Never use `git bisect` or `git checkout HEAD -- <files>`

**`git bisect` is prohibited.**  It requires running tests against many historical
commits.  Claude cannot do this reliably: context windows are finite, intermediate
compile states are inconsistent, and the process almost always requires reverting
in-progress files — destroying multi-session work that is not yet committed.

**`git checkout HEAD -- <file>` to "reset and try again" is prohibited.**  This silently
discards uncommitted changes on the named files.  When a feature branch has several
files in flight (e.g. codegen, fill, debug, mod, scopes all modified together), resetting
individual files breaks cross-file invariants and produces a state that is harder to
debug than the original problem.

**The correct approach for every regression:**

1. **Write a minimal `.loft` reproducer first** — create a short script in
   `tests/scripts/` that triggers the bug.  Use `fn test_*()` entry points.
   If the test fails, add `// @EXPECT_FAIL: <message>` directly above the
   failing function so CI stays green while you work on the fix.  If it's a
   parse error, use `// @EXPECT_ERROR: <message>` instead.
2. Run the failing test with `LOFT_LOG=minimal cargo test --test <suite> <name>` and
   read `tests/dumps/<name>.txt` — the full IR, bytecode, and execution trace are there.
3. If the trace is too long, use `LOFT_LOG=crash_tail:50` to see the last 50 steps
   before the panic.
4. Read the 3–5 source files that the trace implicates.  Reason about the code path.
   The root cause is almost always visible within one careful read.
5. If you need to know what a recent commit changed, use `git show <sha>` or
   `git diff <sha>^ <sha>` — read the diff, do not re-run old code.
6. Fix forward.  Do not revert; do not bisect.
7. **Remove the `@EXPECT_FAIL` / `@EXPECT_ERROR` annotation** once the fix is
   verified.  The test must pass cleanly — `wrap.rs` will print `FIXED` for
   functions that pass despite having `@EXPECT_FAIL`, confirming the annotation
   can be removed.

---

## Closed-by-Decision Register

Before proposing a feature, fix, or language change, check
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md).  It records
questions that have been evaluated and explicitly declined —
feature proposals (e.g. Rust-style literal suffixes), accepted
limitations (e.g. WASM `par()` sequential), and design choices
(e.g. closure capture by value).  The register exists so the
same questions don't resurface every session.

**Rules**:
- Closed items are **not** backlog.  They don't belong in
  ROADMAP.md's milestones, PLANNING.md's priorities, or
  QUALITY.md's active tables.  A short cross-reference to
  DESIGN_DECISIONS.md in an "Out of scope" section is enough.
- **Re-opening** requires new evidence (a concrete use case,
  incident report, or measurement) that wasn't available at the
  decision.  Put it at the top of the revived entry; don't
  silently flip.
- **Adding** a new entry requires the same rigor: question,
  evaluation, decision with date, and "revisit when" trigger.

When declining a proposal, strike it (`~~…~~`) in its source doc
and append a pointer to its new DESIGN_DECISIONS.md entry.  Keeps
the git history discoverable without cluttering active tables.

---

## See also

- [CODE.md](CODE.md) — Naming conventions, function-length rules, clippy policy, null sentinels
- [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) — Closed-by-decision register (see above)
- [TESTING.md](TESTING.md) — Test framework, `code!` / `expr!` macros, LogConfig debug presets
- [PLANNING.md](PLANNING.md) — Backlog, version milestones, effort estimates
- [PROBLEMS.md](PROBLEMS.md) — Open bugs; update here when fixing a known issue
- [RELEASE.md](RELEASE.md) — Gate criteria and release checklist
