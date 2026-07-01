
# Claude Code Instructions for the Loft Project

> These instructions OVERRIDE default behavior — follow them exactly. The MANDATORY
> sections (branch / debugging / bug-filing / git-safety) are hard rules.

## What loft is

**loft** is a tree-walking interpreter (Rust) for the **loft** language: statically typed,
expression-oriented, struct/enum, store-based heap, stdlib from `default/*.loft`. Two backends:
the interpreter and `--native` (compiles via `rustc`). It's the **language** layer of a stack:
**lavition** (the engine/brand) → **loft** (this repo) → games **moros**/**dryopea** + consumer
libs (crawler, `lib/markdown`) that dogfood the language. History: [LAVITION.md](doc/claude/LAVITION.md).

## Dogfood loop

**Build a real consumer → harvest the lessons → fix the language → ship.** Prefer the path that
exercises a real consumer; when a slice surfaces a gap, fix on the spot if XS/S, else route to its
canonical home ([DEVELOPMENT.md § Inserting Discovered Enhancements](doc/claude/DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan)).
**Two-agent split:** this stream BUILDS + FIXES the language and documents the contract; the
consumer's own agent USES + adversarially BREAKS it and reports gaps.

## Key commands

```bash
cargo run --bin loft -- prog.loft        # run     |  -- repl  |  -- introspect prog.loft  |  -- --help
cargo run --bin gendoc                   # regenerate doc/*.html
make ci                                  # fmt → clippy → test (full local gate)
make test                                # clippy + test → result.txt
./scripts/find_problems.sh --bg|--peek|--wait   # background full-suite run + inspect/block
make index ; ./scripts/idx tag:@P259     # rebuild + query the tracker index (prefer over grep -rn)
make view                                # branch-aware doc/code viewer (SSH-forward 8765)
```

**Bound ad-hoc runs** (loft is unbounded by default; tests already arm a 300s watchdog). Especially
for `--native` (rustc can hang): `LOFT_TIMEOUT=60 loft --native p.loft` or `loft --timeout 60 p.loft`
(0 = off). Hard-kills at `timeout+grace` (grace 2s, `LOFT_TIMEOUT_GRACE`). Ref: DEBUG.md, TESTING.md.

For any multi-failure refactor, start `find_problems.sh --bg` before editing (detached
`cargo test --release --no-fail-fast` → `/tmp/loft_problems.txt`).

## Tracker tags <!--noindex-->

`@`-prefixed so regex is unambiguous: **`@P259`** P-issues; **`@PLN3`** = a `loft-lang/plans` issue
(canonical; plan id = issue number); **`@PLAN22`** = legacy local plan dir (migrating to `@PLN`).
File a NEW plan as a `loft-lang/plans` issue with one `status:*` + one `subject:*` label. Look refs
up with `./scripts/idx` (`make index` first if stale; `./scripts/idx help` for queries).

## Architecture — execution path

```
src/main.rs            CLI; loads default/ then user file
 └ src/parser/         two-pass recursive-descent → Value IR
     mod.rs(core) definitions.rs(enum/struct/typedef/fn) expressions.rs(expr/assign)
     operators.rs(op dispatch/coercion) vectors.rs fields.rs(index/field) objects.rs(vars/construct)
     collections.rs(for/map/filter/par) control.rs(match/parse_call) builtins.rs
   src/lexer.rs  src/typedef.rs(type resolution+offsets)  src/variables/  src/scopes.rs(scope/lifetime)
 └ src/compile.rs      IR → flat bytecode; inits native registry
 └ src/state/          executes bytecode: mod.rs(State/execute) text.rs io.rs(file/db) codegen.rs debug.rs
   src/fill.rs         opcode implementations
```

## Key data structures

| Type | File | Purpose |
|---|---|---|
| `Value` / `Type` / `Data` | `src/data.rs` | IR node / static type / table of named defs |
| `State` | `src/state/mod.rs` | bytecode stream + runtime stack |
| `Stores` / `Store` | `src/database/mod.rs` / `src/store.rs` | all stores+schema / raw word-addressed heap |
| `DbRef` | `src/keys.rs` | universal pointer (store_nr, rec, pos) |

## Conventions

- User fns stored as `"n_<name>"` — `data.def_nr("n_foo")`, not `"foo"`.
- Native stdlib: globals `n_<func>`; methods `t_<LEN><Type>_<method>` (LEN = chars in type name),
  e.g. `t_4text_starts_with`. Operators `OpCamelCase` (loft) → `op_snake_case` (`fill.rs`).
- `#rust "..."` in `default/*.loft` supplies the Rust body for codegen. Full naming + null-sentinel
  rules: [CODE.md](doc/claude/CODE.md).
- stdlib load order: `01_code.loft` (operators/math/text/collections) → `02_files.loft` (I/O) →
  `03_text.loft`.
- **Before non-trivial functionality, check [LIBRARIES.md](doc/claude/LIBRARIES.md) + `loft install`** — don't reimplement.
  Writing/reviewing `.loft`: **loft-write skill**. Language ref: [LOFT.md](doc/claude/LOFT.md), [STDLIB.md](doc/claude/STDLIB.md).
- **Response shape:** lead with the ONE highest-leverage item in full (decision + minimum to act),
  then a one-line summary of the rest; no long dumps.
- **Advice vs action:** asked for advice/evaluation → give the recommendation (best option + why),
  don't bounce it back. Asked a question → answer it. Act/edit only on an explicit do-it instruction.
- **Shared knowledge:** anything durable an agent records to private memory must ALSO land in the
  right repo doc (memory is per-agent; the repo is the shared channel). Keep machine-specific values
  out of shared docs.

---

## Branch policy — MANDATORY

`main` is the release branch; every commit on it must be releasable.
1. **Never commit on `main`** — land on a feature branch, reach main only via a GitHub PR (never a
   local `git merge`).
2. **Push proactively — it's a SAFETY rule.** Once a change settles (compiles / tests green), commit
   + push to the feature branch so it isn't lost. Separate from opening a PR.
3. **Never create a branch, open, or merge a PR without an explicit user ask** ("create PR",
   "merge", "switch branch"). "fix X" / "push" / "retry" are NOT such asks; a prior ask doesn't carry
   over. If a protected branch blocks a commit, surface it and ask — don't invent a branch.
4. With an **open PR**, hold non-blocking pushes for the user's consent (force-push/rebase/surprise
   commits) — EXCEPT a push that unblocks a red required check (allowed; it can't merge while red).
5. **While a PR is unmerged, branch from the TIP of that in-flight work — NEVER fork a fresh
   branch off `main`.** `main` lacks the unmerged foundation, so a `main`-based branch can't build
   on it and **development there is impossible** (new work almost always needs what's still in the
   open PR — e.g. @PLN85's fuzz-proof needs @PLN25, which sits in the PR). Stack the new branch on
   the PR branch; rebase the whole stack onto `main` only AFTER the PR merges. Fork from `main`
   only when there is no in-flight work to build on. **Trade-off (respect it):** stacking couples
   the new work to the PR's merge clock — a clean PR merges in minutes, but a problematic one
   blocks everything stacked on it for **hours**, so keep the PR mergeable and land it promptly.
   General branch name; prefix agent branches with the hostname (`<host>-work`); the cycle's
   long-lived branch is the **monthly release branch** `YYYY-MM`; only a substantial design-doc'd
   plan earns a specific name.
6. **Before opening a PR and before requesting merge, verify the head is current on `origin/main`**
   (`git fetch`; `git merge-base --is-ancestor origin/main <head>`). `mergeStateStatus: BEHIND`
   merges as BLOCKED even when `mergeable=MERGEABLE` — rebase + re-push first.

## Debugging policy — MANDATORY

**Never `git bisect` or `git checkout HEAD -- <file>`** — both destroy uncommitted work. Instead:
read the failing test's dump (`tests/dumps/*.txt` — IR+bytecode+trace), narrow with
`LOFT_LOG=minimal`/`crash_tail:50`, read the 3–5 relevant files, `git show <commit>` for regressions.

**Matrix-first for any non-trivial bug (esp. crash / silent corruption) — the urge to fix is the
signal you haven't earned it:**
1. Don't fix on the first read — a clean one-line story is a hypothesis.
2. Build the matrix in throwaway `/tmp` probes on `--interpret` (`scripts/probe-matrix`), varying ONE
   composition axis per probe, distinctive values everywhere.
3. **Hand-compute each cell's expected value** (agreement between two binaries is NOT a pass);
   prove the harness can fail (a no-output cell is vacuous); assert **value AND length AND leak**.
4. Map pass/fail → find the REAL boundary (filed scope is usually wrong). Resisting a read twice →
   instrument with one env-gated `eprintln`, don't theorize.
5. Fix at the chokepoint enforcing exactly the violated invariant — no narrower, no wider.
6. Verify the full matrix on **BOTH backends**; graduate guarantee probes to `tests/scripts/`.

Full flow: [DEBUG.md](doc/claude/DEBUG.md), [plans/_INVESTIGATION_TEMPLATE.md](doc/claude/plans/_INVESTIGATION_TEMPLATE.md).

## Bug-filing policy — MANDATORY

**Default is FIX, not file** — bugs surfaced while fixing another are the cheapest to fix (paths
loaded, repro warm). In **stability work** the file-instead-of-fix escape hatches do NOT apply: fix
in the same session with a regression test. Record scope + root cause, never origin commit.

**File only when NOT fixing now:** it blocks the current task (bookmark + workaround), or it's
genuinely M+/needs-design (route to its canonical home). When you file: a **GitHub Issue**
(`gh issue create`, `bug_report` template) — NOT a PROBLEMS.md row (that's the closed archive) —
with a minimal both-backend repro, `sev:`/`area:` + a VERIFIED `wa:*` label, and `Fixes #NNN`.

**Fixing an existing issue not yet on `main`:** push the fix, write `Fixes #NNN`, keep the issue open
(the `fixed-pending-merge` label is automated off that trailer) — never hand-close. **Inside a
plan:** file only if it reproduces on `main`; branch-internal breakage stays in the plan's docs.
Don't scope-creep the active fix with unrelated bugs.

## Git safety — MANDATORY

**Never `git stash pop` / `git pull` / `git checkout HEAD -- <file>` with uncommitted changes** —
they merge-conflict across files and have destroyed sessions. **Always commit before any operation
that changes the working tree.** Compare without switching: `git diff main -- <file>`,
`git show origin/main:<file>`.

---

## Documentation index

**Language / stdlib:** [LOFT.md](doc/claude/LOFT.md) syntax · [STDLIB.md](doc/claude/STDLIB.md) stdlib API ·
[INTERFACES.md](doc/claude/INTERFACES.md) traits/generics · [TUPLES.md](doc/claude/TUPLES.md) ·
[COROUTINE.md](doc/claude/COROUTINE.md) (1.1+) · [INCONSISTENCIES.md](doc/claude/INCONSISTENCIES.md).

**Compiler / internals:** [COMPILER.md](doc/claude/COMPILER.md) parser/two-pass/types ·
[INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) Value/Type/opcodes/State · [INTERNALS.md](doc/claude/INTERNALS.md) ·
[SLOTS.md](doc/claude/SLOTS.md) stack slots · [NATIVE.md](doc/claude/NATIVE.md) `--native` codegen ·
[THREADING.md](doc/claude/THREADING.md) par · [CODEGEN_METHOD.md](doc/claude/CODEGEN_METHOD.md) how to do compiler work.

**Runtime / memory:** [DATABASE.md](doc/claude/DATABASE.md) stores/DbRef · [LIFETIME.md](doc/claude/LIFETIME.md) deps/freeing ·
[OWNERSHIP_MODEL.md](doc/claude/OWNERSHIP_MODEL.md) the deps north-star (borrow system) ·
[LOGGER.md](doc/claude/LOGGER.md) · [WASM.md](doc/claude/WASM.md) · [HTML_EXPORT.md](doc/claude/HTML_EXPORT.md) ·
[BROWSER_INTEROP.md](doc/claude/BROWSER_INTEROP.md) · [WINDOWS.md](doc/claude/WINDOWS.md) / [WINDOWS_SESSION.md](doc/claude/WINDOWS_SESSION.md).

**Testing / debug:** [TESTING.md](doc/claude/TESTING.md) framework/`LOFT_LOG`/LogConfig ·
[DEBUG.md](doc/claude/DEBUG.md) tools + boundary-matrix runner · [CAVEATS.md](doc/claude/CAVEATS.md) edge cases ·
[PERFORMANCE.md](doc/claude/PERFORMANCE.md) benchmarks.

**Quality / stability / formal:** [CODE.md](doc/claude/CODE.md) · [DOC_QUALITY.md](doc/claude/DOC_QUALITY.md) ·
[QUALITY.md](doc/claude/QUALITY.md) open work · [GOALS.md](doc/claude/GOALS.md) (purpose + goals A–F) ·
[STRONG_POINTS.md](doc/claude/STRONG_POINTS.md) · [DESIGN.md](doc/claude/DESIGN.md) algorithms ·
[DESIGN_DECISIONS.md](doc/claude/DESIGN_DECISIONS.md) declined-features register ·
[DESIGN_PROTOCOL.md](doc/claude/DESIGN_PROTOCOL.md) / [DESIGN_VERIFICATION.md](doc/claude/DESIGN_VERIFICATION.md) ·
[FORMATTER.md](doc/claude/FORMATTER.md) · stability: [STABILITY_ROADMAP.md](doc/claude/STABILITY_ROADMAP.md)
(the tracking view) · [STABILITY_METHOD.md](doc/claude/STABILITY_METHOD.md) /
[_SWEEP](doc/claude/STABILITY_SWEEP.md) / [_HOTSPOTS](doc/claude/STABILITY_HOTSPOTS.md) /
[_REDFLAGS](doc/claude/STABILITY_REDFLAGS.md) · [DEPS_INVENTORY.md](doc/claude/DEPS_INVENTORY.md) ·
formal lens: [FORMALIZATION.md](doc/claude/FORMALIZATION.md) / [TYPING_RELATION.md](doc/claude/TYPING_RELATION.md) ·
strict: [formal/README.md](doc/claude/formal/README.md) (rules + deviations driven to zero).

**Plans / roadmap:** [plans/README.md](doc/claude/plans/README.md) · [PLANNING.md](doc/claude/PLANNING.md) backlog ·
[ROADMAP.md](doc/claude/ROADMAP.md) by milestone · [BROADENING.md](doc/claude/BROADENING.md) beyond games ·
[lib_plans/README.md](doc/claude/lib_plans/README.md) (legacy) · [STACKTRACE.md](doc/claude/STACKTRACE.md) · [SANDBOX.md](doc/claude/SANDBOX.md).

**Libraries / registry / packages:** [LIBRARIES.md](doc/claude/LIBRARIES.md) installable catalogue ·
[PACKAGES.md](doc/claude/PACKAGES.md) format/targets · [PKG_REGISTRY.md](doc/claude/PKG_REGISTRY.md) registry MVP ·
[LIBRARY_AUTHORING.md](doc/claude/LIBRARY_AUTHORING.md) / [LIBRARY_CHECKLIST.md](doc/claude/LIBRARY_CHECKLIST.md) ·
[REGISTRY_SUBMIT.md](doc/claude/REGISTRY_SUBMIT.md) / [REGISTRY_BOOTSTRAP.md](doc/claude/REGISTRY_BOOTSTRAP.md) /
[REGISTRY_RECOVERY.md](doc/claude/REGISTRY_RECOVERY.md) · [API_SURFACE.md](doc/claude/API_SURFACE.md) ·
publishing is the **loft-ship skill** (touch-gated signing). REPL: [REPL.md](doc/claude/REPL.md).

**Process / issues / release:** [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) workflow ·
[ISSUE_TRACKING.md](doc/claude/ISSUE_TRACKING.md) (open→Issues, closed→[PROBLEMS.md](doc/claude/PROBLEMS.md)) ·
[.github/LABELS.md](.github/LABELS.md) · [RELEASE.md](doc/claude/RELEASE.md) · [MOVING.md](doc/claude/MOVING.md) ·
[CHANGELOG.md](CHANGELOG.md) / [CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md) ·
[DOC.md](doc/claude/DOC.md) · [LAVITION.md](doc/claude/LAVITION.md) · [PROMPTS.md](doc/PROMPTS.md).

**Skills** (`.claude/skills/`): `loft-write` (.loft authoring) · `loft-debug` (runtime crashes) ·
`loft-test` · `loft-codegen` · `loft-ship` (library cross-target + publish) · `engineering-rigor` /
`design-protocol` (rigor) · `doc-quality` · `draw` · `loft-plan-workflow`.

## `LOFT_LOG` quick reference

Set before `cargo test` (controls `tests/dumps/*.txt`; also works with `cargo run` → stderr):
`full` (default: IR+bytecode+exec+slots) · `static` (IR+bytecode only) · `minimal` (exec trace) ·
`crash_tail:N` (last N lines, flushed on panic) · `fn:<name>` · `variables` · `ref_debug` ·
`bridging` · `all_fns`. DbRef dumps tune via `LOFT_DUMP_DEPTH` (2), `LOFT_DUMP_ELEMENTS` (8).
Full API: [TESTING.md § LogConfig](doc/claude/TESTING.md), [DEBUG.md](doc/claude/DEBUG.md).
