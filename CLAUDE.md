
# Claude Code Instructions for the Loft Project

## What loft is

**loft** is a tree-walking interpreter (in Rust) for the **loft** language: statically typed,
expression-oriented, with struct/enum support, a store-based heap, and a stdlib loaded from
`default/*.loft`.

loft is the **language** layer of a three-layer stack:

- **lavition** — the **engine**: an editor with loft as its built-in scripting language,
  positioned as a rapid-prototyping game engine for indie devs/studios. The long-term
  destination and the ownable brand. (History + naming rationale: [LAVITION.md](doc/claude/LAVITION.md).)
- **loft** — the **language** (this repo): a deliberately generic, descriptive name; ships
  under the lavition umbrella, never as a standalone brand.
- **moros** (RPG) and **dryopea** (sci-fi tower-defence) — games built on lavition in loft;
  the canonical dogfood consumers that drive language work.

---

## Development cadence — the dogfood loop

> **Build a real consumer → harvest the language lessons → fix the language → ship as a release.**

Not toy programs — real tools that have to work. The canonical consumers (branch-review viewer
[@PLAN35](doc/claude/plans/finished/35-branch-review-viewer/README.md), tracker indexer
[@PLN42](doc/claude/plans/42-tracker-index/README.md), [`lib/markdown/`](lib/markdown))
each drove a wave of language work that landed BEFORE the next minor release.

When picking work, prefer the path that exercises the language against a real consumer. When a
feature slice surfaces a language gap, fix it on the spot when XS/S, else route to its canonical
home — [DEVELOPMENT.md § Inserting Discovered Enhancements](doc/claude/DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan).

---

## Key commands

```bash
cargo run --bin loft -- myprogram.loft        # run a loft program
cargo run --bin loft -- repl                   # interactive REPL (or bare `loft`); see REPL.md
cargo run --bin loft -- introspect prog.loft   # bytecode + Rust + slots + types dump
cargo run --bin loft -- --help                # CLI help
cargo run --bin gendoc                        # regenerate doc/*.html
make ci                                       # fmt → clippy → test (full local gate)
make test                                     # clippy + test; output in result.txt
./scripts/find_problems.sh --bg               # background full-suite run
./scripts/find_problems.sh --peek             #   inspect mid-run
./scripts/find_problems.sh --wait             #   block for summary
make index                                    # rebuild index/tags.json (plan-37)
./scripts/idx tag:@P259                       # tracker-ref lookup (plan-37; prefer over grep -rn)
make view                                     # branch-aware doc + code viewer (plan-35; SSH port-forward 8765)
```

### Bounding a run — `--timeout` / `LOFT_TIMEOUT` (@PLAN49)

loft runs **unbounded by default** — that is deliberate, so long-running programs
(servers, game loops) work. There is **no** process-wide default timeout, and we do
not want one. **Testing, by contrast, runs under a timeout:** `loft test` / `--tests`
already arms the watchdog at 300s.

So when YOU run loft ad-hoc — a probe, a one-shot script, and especially
`--native` (which compiles via `rustc` and can hang) — **bound it yourself** so a
runaway can't hang your session:

```bash
LOFT_TIMEOUT=60 loft --native prog.loft        # env form (floor; arms at startup)
loft --timeout 60 prog.loft                     # flag form (0 = disabled)
```

The watchdog is a process-level hard-kill thread (covers the `--native` compile
*and* execution); it kills at `timeout + grace` (grace default 2s,
`LOFT_TIMEOUT_GRACE`). Full reference: [DEBUG.md § Bounding a run](doc/claude/DEBUG.md) and TESTING.md.

<!-- noindex region: don't migrate the bare-name examples that explain the convention. -->
## Tracker tags (plan-37) <!--noindex-->

Tracker refs use an `@`-prefix so regex matches are unambiguous (the bare-name `P259` <!--noindex-->
regex collides with `2P259`, `P2590`, and prose):

- **P-issues**: `@P259`, `@P229b`, `@P262`.
- **Plans (canonical)**: `@PLN3` = a [`loft-lang/plans`](https://github.com/loft-lang/plans)
  issue (the cross-ecosystem plan id = its issue number).
- **Plan dirs + phases (legacy/local, being migrated)**: `@PLAN22`, `@PLAN35-01`,
  `@PLAN22-2d-iii.a` — point at the design dir `plans/<NN>/`.  Active + future local dirs are
  migrating to `@PLN` issues under **@PLN27**; `finished/` keeps its `@PLAN<NN>` refs.  File a
  NEW plan as a `loft-lang/plans` issue, not a dir — with a required `status:*` + `subject:*`
  label (every plan carries exactly one of each).

Bare-name forms (`P259`, `plan-22 phase 03`) still work in prose; the indexer tracks both. <!--noindex-->

**Looking up refs — use `./scripts/idx`** (faster than `grep -rn`, returns structured JSON, avoids
pulling file content into context). Run `make index` first if `index/tags.json` is stale; run
`./scripts/idx help` for the full query/flag set (`tag:`, `prefix:`, `file:`, `incoming:`, `broken`,
`broken-links`, plus `--before/--after/--para/--max-bytes` excerpt flags).

For any refactor likely to surface multiple test failures, kick off `find_problems.sh --bg` before
editing — it runs `cargo test --release --no-fail-fast` detached and writes a structured summary to
`/tmp/loft_problems.txt`. See [TESTING.md](doc/claude/TESTING.md) § "Preferred shape — background +
peek + wait".

---

## Architecture — execution path

```
src/main.rs              CLI entry; loads default/ then user file
  └─ src/parser/         Two-pass recursive-descent parser → Value IR
       ├─ mod.rs            Parser struct, constructors, core helpers
       ├─ definitions.rs    Enum/struct/typedef/function parsing
       ├─ expressions.rs    Expressions, assignments, iterator materialisation
       ├─ operators.rs      Operator dispatch, type coercion
       ├─ vectors.rs        Vector literals, comprehensions, lambdas
       ├─ fields.rs         Field access, indexing, iterator operations
       ├─ objects.rs        Variable resolution, struct construction, parse
       ├─ collections.rs    Iterators, for-loops, map/filter, parallel-for
       ├─ control.rs        Control flow, match, parse_call, parse_method
       └─ builtins.rs       Parallel worker helpers
       ├─ src/lexer.rs      Tokeniser
       ├─ src/typedef.rs    Type resolution + field offsets
       ├─ src/variables/  Per-function variable table
       └─ src/scopes.rs     Scope/lifetime analysis
  └─ src/compile.rs      Drives IR → flat bytecode; initialises native registry
  └─ src/state/          Executes bytecode
       ├─ mod.rs            State struct, execute, stack primitives
       ├─ text.rs           String/text operations
       ├─ io.rs             File I/O, database record ops
       ├─ codegen.rs        Bytecode generation (generate, gen_* helpers)
       └─ debug.rs          Dump/trace helpers
       └─ src/fill.rs       opcode implementations
```

---

## Key data structures

| Type | File | Purpose |
|---|---|---|
| `Value` (enum) | `src/data.rs` | IR tree node |
| `Type` (enum) | `src/data.rs` | Static type of a `Value` |
| `Data` | `src/data.rs` | Table of all named definitions |
| `State` | `src/state/mod.rs` | Bytecode stream + runtime stack |
| `Stores` | `src/database/mod.rs` | All stores + type schema |
| `Store` | `src/store.rs` | Raw word-addressed heap |
| `DbRef` | `src/keys.rs` | Universal pointer: (store_nr, rec, pos) |

---

## Important conventions

- User functions are stored as `"n_<name>"` — use `data.def_nr("n_foo")`, not `data.def_nr("foo")`.
- Native stdlib: global functions use `n_<func>`; methods use `t_<LEN><Type>_<method>` (LEN = chars in type name). Example: `t_4text_starts_with`, `t_9character_is_numeric`.
- Operators: `OpCamelCase` in loft source → `op_snake_case` in Rust (`fill.rs`).
- `#rust "..."` annotations in `default/*.loft` supply the Rust body for code generation.
- Full naming and null-sentinel rules: see [CODE.md](doc/claude/CODE.md).
- Response shape (reporting to the user): lead with the ONE highest-leverage item in full — the decision + the minimum to act on it — then a one-line summary of the rest; don't dump long detailed lists. Full norm: [ISSUE_TRACKING.md § The work queue](doc/claude/ISSUE_TRACKING.md).
- Advice vs action: when asked for advice/evaluation/"what should we do about X", give the recommendation (best option + why) — do NOT bounce the decision back as a question. When asked a question, answer it — don't treat it as a trigger to start editing/running. Act only on an explicit do-it instruction.
- Shared knowledge: anything an agent records to its private memory store that is durable and project-relevant must ALSO be documented in the repo (the right canonical doc) so other agents pick it up — memory is per-agent, the repo is the shared channel. Keep machine-specific values out of shared docs (document the convention, not the hostname).

---

## Default standard library load order

```
default/01_code.loft    — operators, math, text, collections
default/02_files.loft   — File I/O, Format, EnvVariable, path helpers
default/03_text.loft    — text utilities
```

---

## Loft language patterns

**Before implementing any non-trivial functionality, check [doc/claude/LIBRARIES.md](doc/claude/LIBRARIES.md) + `loft install` — a registered library may already do it (don't reimplement).**

For writing or reviewing `.loft` files see the **loft-write skill**
(`.claude/skills/loft-write/SKILL.md`) — naming conventions, type reference, format strings, loop
attributes, lambdas, known bugs and workarounds, pre-flight checklist.

Full language reference: [LOFT.md](doc/claude/LOFT.md) and [STDLIB.md](doc/claude/STDLIB.md).

---

## Branch policy — MANDATORY

**Direct commits to `main` are prohibited.** `main` is the release branch — every commit on it
must be releasable. All changes land on a feature branch and reach `main` only via a PR.

1. **Never `git commit` on `main`.** If you land there by accident, move the change to a feature
   branch first.
2. **Pushing is OK by default — unless an open PR on the branch would be disturbed.** Push freely
   after green CI on a long-lived branch with no open PR (the user wants commits visible without
   asking). With an open PR, do NOT push without the user's explicit consent — force-pushes,
   rebases, or surprise commits disrupt review — **except a fix for a blocking failure** (red CI, a
   broken build, a failing required check): a push that *unblocks* the PR is allowed without asking,
   because the PR cannot merge while it is red anyway. Check `gh pr list --head <branch>` if unsure.
3. **Never create a branch or open a PR unless the user explicitly asks** ("create PR", "open a
   PR", "merge", "switch to a new branch"). "fix X" / "implement Y" / "push" / "retry" / "do the
   fixes" are NOT branch/PR instructions; a prior "open a PR" does not authorise the next one. Do
   the work in the working tree; if a protected branch (`main`) blocks the commit, surface that
   and ask which branch — don't silently invent a feature branch. When in doubt, summarise what's
   ready and ask. (Ad-hoc, specifically-named branches get forgotten and strand features.)
4. Branch from the tip of `main` with a **GENERAL name** (`quality-pass`, `cleanup`, `work`) so one
   long-lived branch hosts cross-theme work — new branches keep re-rebasing against a moving `main`
   and failing CI on patterns they didn't author. **Prefix an agent-created working branch with the
   machine hostname** (e.g. `<hostname>-work`, `tuxedo-work`) so one discoverable branch per machine
   consolidates that agent's work and the user controls merge/PR timing without losing features in
   stray branches. Only a substantial plan with its own design doc (e.g. `plan-06-arc`, `lsp-server`)
   earns a specific name. The active cycle's long-lived branch is
   a **monthly release branch** named for its release month, `YYYY-MM` (e.g. `2026-07`); cross-theme
   work lands there and it ships at the start of that month once the tree is stable with a low bug
   count — see [RELEASE.md § Release cadence](doc/claude/RELEASE.md#release-cadence).
5. Merge to `main` via a GitHub PR — never a local `git merge`.

---

## Debugging policy — MANDATORY

### Never use `git bisect` or `git checkout HEAD -- <file>` to investigate

Both destroy uncommitted in-flight work: bisect needs autonomous compile/test across dozens of
commits (unreliable in finite context, routinely reverts WIP files); `git checkout HEAD -- <file>`
silently discards uncommitted changes and breaks cross-file invariants. Instead:

- Read the failing test's dump (`tests/dumps/*.txt`) — full IR + bytecode + trace; the root cause is
  almost always visible there.
- Add `LOFT_LOG=minimal` or `LOFT_LOG=crash_tail:50` to narrow the execution step.
- Read the 3–5 relevant source files and reason about the code path.
- For a recent regression, `git show <commit>` / `git diff <commit>^ <commit>` — read it, don't re-run.

### Before fixing a non-trivial bug: build the boundary matrix (matrix-first)

The urge to apply a fix is the signal you have NOT earned it yet. On any non-trivial bug —
*especially* a crash or silent corruption — run this before touching code. Lightweight default
(`/tmp` probes, no plan); inside an investigation plan it becomes the formal
[`_INVESTIGATION_TEMPLATE.md`](doc/claude/plans/_INVESTIGATION_TEMPLATE.md) flow.

1. **Don't fix on the first read.** A coherent (especially elegant one-line) explanation is a
   *hypothesis* — real bugs are complex-variant; the clean story is usually the part you haven't
   looked at yet.
2. **Build the matrix** in throwaway `/tmp` probes, on `--interpret` only — use
   **`scripts/probe-matrix`** (`init` scaffolds; the runner enforces every rule below as a hard
   error and `--baseline <main-worktree-binary>` auto-classifies REGRESSION vs PRE-EXISTING; usage:
   [DEBUG.md § Boundary-matrix runner](doc/claude/DEBUG.md#boundary-matrix-runner-scriptsprobe-matrix)).
   Vary ONE dimension per probe along the **composition axes**
   ([plans/README § composition axes](doc/claude/plans/README.md#the-composition-axes--the-dimensions-a-matrix-varies)):
   type-kind / construction-path / context / access / depth / null / backend. Distinctive
   collision-resistant values at every index/position — weak probes (small values, only `[0]`, no
   length check) hide cases. SEE on the interpreter (strides/types surface in seconds); `--native`
   pays a rustc compile per probe — that cost belongs at the final verify (step 7).
   **Validate the matrix itself before trusting any cell** (both rules from the 2026-06-12
   vector-ABI session, where a 24-cell matrix was vacuous — every cell a parse error read as
   "clean"):
   - **Hand-compute each cell's EXPECTED value before running it.** A cell without an expected
     value can only detect crashes, never wrong results — and "two binaries agree" is NOT a pass
     (HEAD and main both printed `acc=39` where the true value was 12: agreement on shared
     corruption).
   - **Prove the harness can fail**: a cell that produces no output is vacuous, not clean — check
     the probe's own output appears; keep one deliberately-broken control cell red.
3. **Map pass/fail; find the real boundary.** Expect the filed/assumed scope to be wrong — it
   usually is (#263 was *any runtime fn-ref value*; #262 was *every context*; cluster III was three
   different mechanisms).
   When a cell's mechanism resists two reading passes, STOP theorizing and instrument — one
   `eprintln` behind an env flag (e.g. `LOFT_TRACE_VADD`) settles in one run what code-reading
   debates for thirty minutes (the rec_tp=20 stride bug fell to three prints).
4. **The matrix is how you SEE the root** — the shared mechanism behind a family of "different"
   symptoms is visible in the matrix and invisible in any one repro. "Can't see the root yet" =
   "the matrix isn't finished," NEVER license to patch the one case in hand.
5. **Fix at the chokepoint, enforcing exactly the invariant** the whole failing region violates —
   no narrower (a per-case patch leaves siblings broken), no wider (re-resolving the type drags
   blast radius). An un-generalized remainder is the same bug, unfinished.
6. **If a multi-site fix regresses, bisect by SITE** — apply one site at a time and re-run the
   matrix — after the FIRST regression, not the third.
7. **Verify against the full matrix on BOTH backends** — interp-vs-native divergence is a real
   hazard, and this is where `--native` earns its compile cost. During iteration re-run only the
   touched subset; run the full matrix once at the end. Graduate guarantee probes to `tests/scripts/`.

---

## Bug-filing policy — MANDATORY

> **STANDING RULE for stability work** (queue in
> [STABILITY_ROADMAP.md](doc/claude/STABILITY_ROADMAP.md)): in stability/bug-fixing work — this
> agent's stream; feature building (gaming/engine) belongs to a parallel agent; work-limited,
> not time-limited — the file-instead-of-fix escape hatches below do NOT apply: a surfaced bug
> gets fixed in the same working session, with its regression test. This is the same standing
> rule already documented for investigation plans (below), generalized: fixing IS the work, so
> there is no "later" to file for. An issue may exist only as the record of a fix in flight
> (`fixed-pending-merge`), never as a deferral.

**When you surface a bug, the default is to FIX it — not file it.** Bugs surfaced while
diagnosing/fixing another are the cheapest you'll ever fix: code paths loaded, diagnostics warm,
repro within reach — fix on the spot with a regression test. Filing only documents a bug for
*later*, and "later" re-pays to re-derive the scope/repro/mechanism you have right now. Solving is
the work; a backlog of filed-but-unfixed rows is not progress.

**Origin is never worth recording** — which commit introduced a bug tells you nothing about making
it correct. Scope (what triggers it — the edges) and root cause (the mechanism in the *present*
code) are what you fix from.

**File only when you are NOT fixing now:**

- **It blocks the task you're on** — file a bookmark + use a workaround, keep moving, come back.
- **It's genuinely too big now** (M+ effort / needs design) — route to its canonical home
  ([DEVELOPMENT.md § Inserting Discovered Enhancements](doc/claude/DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan)).

When you DO file: **open a GitHub Issue** (`gh issue create`, the `bug_report` template) — NOT a
PROBLEMS.md row (that's the closed/historical archive; see
[ISSUE_TRACKING.md](doc/claude/ISSUE_TRACKING.md)). Include a minimal reproducer (expected vs
observed on each backend), `sev:` + `area:` labels, and a **`wa:*` workaround label whose claim you
VERIFIED on both backends** (a wrong workaround is worse than `wa:none`). Label meanings:
[`.github/LABELS.md`](.github/LABELS.md). Save the repro to `/tmp/p_followups/` or add a
`tests/scripts/` regression. Close with `Fixes #NNN` — but don't file at all for a bug you fix in
the same change (the fix + its regression test ARE the record).

**When you fix an EXISTING issue but it isn't merged to `main` yet:** after pushing the fix, apply
the **`fixed-pending-merge`** label and keep the issue open — `main` is the release branch, so
closing on the working branch would claim "fixed" while released code still has the bug. Comment
naming the fixing commit + regression test; ensure the commit (or PR body) carries `Fixes #NNN`
(a bare `(#NNN)` mention does NOT auto-close) so the merge closes it in one clean transition. Never
close such an issue by hand. Full lifecycle: [ISSUE_TRACKING.md § Issue lifecycle](doc/claude/ISSUE_TRACKING.md).

**Inside any plan, file a problem only when it reproduces *outside* the plan —
already on `main`.**  A GitHub Issue is a claim about `main`: a pre-existing
`main` bug you stumble on during plan work gets filed (and cross-linked to the
plan), but a breakage the plan's own in-progress work caused is branch-internal —
it lives in the plan's docs and is fixed on the branch, never filed.  Investigation
plans are the strongest case of this rule: the probes + cluster docs already
document every shape, so a separate P-issue would just double-document it (see
[`plans/_INVESTIGATION_TEMPLATE.md`](doc/claude/plans/_INVESTIGATION_TEMPLATE.md)).

This is **not** a license to scope-creep the active fix.  When you're focused on
shipping fix X, an unrelated bug Y you can't fix without derailing X is exactly
the "not fixing it now" case — file Y (or pick it up as its own focused change
next); don't bundle it into X's patch unless they share a single fix site.

### Inserting fixes vs filing — see DEVELOPMENT.md

The consumer-gap case: when a missing language/stdlib feature a real consumer needs is XS or S
(under half a day) AND the consumer code is fresh in working memory, prefer **inserting a step into
the active plan that fixes the gap directly**, then resume — so the language/stdlib gets sturdier
and the workaround never enters shipped code. Route to a canonical home (P-issue / `## Open work`
row in STDLIB/NATIVE/COMPILER / new lib_plans slot) when an inline fix isn't appropriate (M+, needs
design, touches unrelated subsystems). Big deferred features get their own [loft-lang/plans](https://github.com/loft-lang/plans) issue (`@PLN<n>` — no local plan slot), never a row in
a parallel catalog. Full decision tree:
[DEVELOPMENT.md § Inserting Discovered Enhancements](doc/claude/DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan).

---

## Git safety — MANDATORY

### Never use `git stash pop` or `git pull` with uncommitted changes

Both apply changes as a merge and routinely conflict across dozens of files; a failed `stash pop`
leaves the working directory unrecoverable and has destroyed multi-hour sessions. Instead:

- **Compare with main:** `git diff main -- <file>` or `git show origin/main:<file>` — no switch.
- **Check if a bug is pre-existing:** commit current work first (even WIP), then compare.
- **Update from remote / test on clean main:** commit first, then `git pull` / `git checkout main`.

The rule: **always commit before any operation that changes the working tree.**

---

## Documentation index

| File | Topic |
|---|---|
| [LOFT.md](doc/claude/LOFT.md) | Language reference (syntax, types, operators, control flow) |
| [STDLIB.md](doc/claude/STDLIB.md) | Stdlib API (math, text, collections, file I/O, logging, parallel) |
| [LIBRARIES.md](doc/claude/LIBRARIES.md) | Auto-generated catalogue of every installable registry library — check before writing code |
| [REPL.md](doc/claude/REPL.md) | Interactive REPL (`loft repl`) + introspection (`loft introspect`): commands, result echo, session limits (@PLN12) |
| [COMPILER.md](doc/claude/COMPILER.md) | Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode |
| [OWNERSHIP_MODEL.md](doc/claude/OWNERSHIP_MODEL.md) | **The north star**: `deps` should become a sound, complete ownership/borrow system (loft's borrow checker, Rust as the reference model) from which every store-lifetime codegen decision derives mechanically. The store-lifetime bug class = the holes in it. The migration backlog + invariants |
| [CODEGEN_METHOD.md](doc/claude/CODEGEN_METHOD.md) | **How to do compiler work**: parse-tree + types together should INDICATE what to emit; codegen does the local translation. Complex *re-derivation* in codegen (recomputing a non-local fact the types should carry) is a DIAGNOSTIC of a type-system flaw — fix the type. But don't over-correct into a 1:1 type→codegen map (that overburdens types): facts in types, translation in codegen. Build bottom-up per scale (bytecode→types→code), working-vs-broken bytecode proven first on both backends. The whole of loft moves onto this, plan by plan |
| [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) | Value/Type enums; bytecode operators; State layout |
| [DATABASE.md](doc/claude/DATABASE.md) | Store allocator, Stores schema, DbRef, vector/tree/hash/radix |
| [INTERNALS.md](doc/claude/INTERNALS.md) | calc.rs, stack.rs, create.rs, native.rs, ops.rs, png_store.rs, parallel.rs, main.rs, logger.rs |
| [THREADING.md](doc/claude/THREADING.md) | Parallel execution — `par`/`par_light`, thread safety, store isolation |
| [INTERFACES.md](doc/claude/INTERFACES.md) | Interface/trait system — bounded generics, operator overloading |
| [WASM.md](doc/claude/WASM.md) | WASM runtime (wasm32-wasip2, VirtFS, host bridges, threading, frame yield). Major W1.x shipped; lone open item W1.18-6 |
| [SANDBOX.md](doc/claude/SANDBOX.md) | Letting users run scripts without breaking the host (player playgrounds / game mods): admission-time validation (capability/library/loop/recursion limits) + effect-containment (transactional store) + fault-isolation, each pinned to a runnable Check (most RED today) + the buildable-now first slice. Aspirational/design |
| [WINDOWS.md](doc/claude/WINDOWS.md) | Windows support — verified state, gaps G1–G4 + per-gap VM runbook |
| [WINDOWS_SESSION.md](doc/claude/WINDOWS_SESSION.md) | Action checklist for when Windows access arrives (companion to WINDOWS.md) |
| [LOGGER.md](doc/claude/LOGGER.md) | Runtime logging framework (log_info/warn/error/fatal, config, rate limiting) |
| [TESTING.md](doc/claude/TESTING.md) | Test framework, `LogConfig` presets, `LOFT_LOG`, suite files |
| [DOC.md](doc/claude/DOC.md) | HTML doc generation (gendoc.rs + documentation.rs) |
| [DESIGN.md](doc/claude/DESIGN.md) | Algorithm catalog with complexity analysis |
| [CODE.md](doc/claude/CODE.md) | Code quality rules (naming, functions, doc comments, clippy, deps) |
| [DOC_QUALITY.md](doc/claude/DOC_QUALITY.md) | In-code comment quality — keep present-tense why/invariants, trim plan-tag/history narration, write for entry-level + non-native-English readers; evidence + runnable Check (`scripts/lint_comments.sh`) |
| [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) | Dev workflow — branching, WIP commit, rebase, CI |
| [SLOTS.md](doc/claude/SLOTS.md) | Stack slot assignment — two-zone design, diagnostics |
| [ISSUE_TRACKING.md](doc/claude/ISSUE_TRACKING.md) | Where bugs live: open → GitHub Issues; investigations → files; closed → PROBLEMS.md. Labels, `@GH###` refs, workaround-as-signal |
| [.github/LABELS.md](.github/LABELS.md) | Issue-label glossary (`sev:`/`wa:`/`area:`) |
| [PROBLEMS.md](doc/claude/PROBLEMS.md) | Closed/historical bug archive (FIXED rows = regression record; `###` entries = design refs) |
| [QUALITY.md](doc/claude/QUALITY.md) | Open programmer-biting issues, active sprint, designs, compiler blockers, landing order. See § Open work table |
| [DESIGN_DECISIONS.md](doc/claude/DESIGN_DECISIONS.md) | Closed-by-decision register — check before proposing declined features |
| [DESIGN_PROTOCOL.md](doc/claude/DESIGN_PROTOCOL.md) | Design Protocol 1 — a design is a testable hypothesis: name the invariant, count re-assertion sites, probe each load-bearing claim to falsify, validate against the prediction (graduated from DESIGN_VERIFICATION C1; fires on load-bearing designs). **Now the self-contained `design-protocol` skill** (`.claude/skills/design-protocol/`, the DESIGN-mode sibling of `engineering-rigor`); this doc is a stub anchor |
| [DESIGN_VERIFICATION.md](doc/claude/DESIGN_VERIFICATION.md) | Concerns to check a load-bearing design against (append-only; C1 brittleness-over-bugs — GRADUATED → DESIGN_PROTOCOL.md) |
| [FORMATTER.md](doc/claude/FORMATTER.md) | Source formatter design |
| [INCONSISTENCIES.md](doc/claude/INCONSISTENCIES.md) | Known language design inconsistencies |
| [PERFORMANCE.md](doc/claude/PERFORMANCE.md) | Benchmarks, root-cause vs CPython/Rust, wasm-vs-native gap, optimisation designs. Open follow-ups in § Open work |
| [GOALS.md](doc/claude/GOALS.md) | What loft is *for*: purpose (foundation for lavition, fun-on-pickup) + six stack-wide goals A–F, each with a runnable Check |
| [STABILITY_ROADMAP.md](doc/claude/STABILITY_ROADMAP.md) | THE single tracking view: every open stability item in finishing order (order/size/status only; detail stays in the canonical homes) |
| [STABILITY_METHOD.md](doc/claude/STABILITY_METHOD.md) | The three-pass stability method: sweep dual invariants (document, don't fix) → move algorithms to their data structures → de-duplicate |
| [STABILITY_SWEEP.md](doc/claude/STABILITY_SWEEP.md) | The live pass-1 catalog: invariant families F1–F10, per-module work list, findings log |
| [STABILITY_HOTSPOTS.md](doc/claude/STABILITY_HOTSPOTS.md) | Forward risk register H1–H8: the designs that will manufacture future bugs (analysis-dependent arity, dep-list overload, ownership-by-shape-analysis, …) — each with sized mitigation work, landing order, validation gates |
| [STABILITY_REDFLAGS.md](doc/claude/STABILITY_REDFLAGS.md) | Cross-cut red-flag map (4-audit sweep, 2026-06): non-local facts re-derived per-site that a stable future must compute once — 5 clusters (return/bind ownership · stack-signal · container-traversal keystone · null-sentinel codec · manifestation guards) by missing fact + leverage-first landing order. Forward-stability record, not a fix-now list |
| [DEPS_INVENTORY.md](doc/claude/DEPS_INVENTORY.md) | H2 deliverable: the dep-list `Vec<u16>` semantic model (frame vs def address space, five marker overloads), every site classified, corpus-probe findings, the typed-`Deps` migration design |
| [PLANNING.md](doc/claude/PLANNING.md) | Priority-ordered enhancement backlog |
| [ROADMAP.md](doc/claude/ROADMAP.md) | Items in implementation order by milestone (0.9.0 / 1.0.0 / 1.1+) |
| [plans/README.md](doc/claude/plans/README.md) | Multi-phase initiatives (core, runtime, AND library) — flat files `plans/<n>-<slug>.md` numbered to a `loft-lang/plans` `@PLN<n>` issue |
| [lib_plans/README.md](doc/claude/lib_plans/README.md) | LEGACY — library plans absorbed into `plans/`; archive only |
| [BROADENING.md](doc/claude/BROADENING.md) | Using loft beyond games (CLI, server, data), sequenced unlocks |
| [TUPLES.md](doc/claude/TUPLES.md) | Tuple design — multi-value returns, deconstruction, match destructuring |
| [STACKTRACE.md](doc/claude/STACKTRACE.md) | Stack trace introspection — `stack_trace()`, `StackFrame`, `ArgValue` |
| [NATIVE.md](doc/claude/NATIVE.md) | Native codegen pipeline (`src/generation/`), per-Op dispatch. `--native` shipped (CI-gated). Open work in § Open work |
| [PACKAGES.md](doc/claude/PACKAGES.md) | Package format (`loft.toml`), binding model, build pipeline, target matrix, library-owned wasm bridges. Shipped. Open work in § Open work |
| [PKG_REGISTRY.md](doc/claude/PKG_REGISTRY.md) | File-based registry MVP — `loft install` against static `registry.json`. Phases R1-R9 |
| [REGISTRY_BOOTSTRAP.md](doc/claude/REGISTRY_BOOTSTRAP.md) | One-time runbook to bring `loft-lang/registry` online (Ed25519 keypair, CI templates) |
| [REGISTRY_RECOVERY.md](doc/claude/REGISTRY_RECOVERY.md) | Trust-root incident runbooks (A: backup intact; B: backups gone; C: key compromised) + annual drill |
| [REGISTRY_SUBMIT.md](doc/claude/REGISTRY_SUBMIT.md) | Author-facing submission guide (5-step submit flow, releases, yanking, troubleshooting) |
| [LIBRARY_AUTHORING.md](doc/claude/LIBRARY_AUTHORING.md) | End-to-end author narrative — `loft new` → develop → package → publish → maintain |
| [LIBRARY_CHECKLIST.md](doc/claude/LIBRARY_CHECKLIST.md) | What a *correct* library looks like — Goals A–F + doc quality applied per-library, split `[auto]` (library-ci) / `[review]`; the registry `verified` mark is how it's administered |
| [API_SURFACE.md](doc/claude/API_SURFACE.md) | Verifying the two prime programmer-facing surfaces (language/stdlib + libraries) for dup/confusable/undocumented/footgun fns — one `api-lint` over both targets; the stdlib is the library every program imports, so it passes LIBRARY_CHECKLIST too |
| [LAVITION.md](doc/claude/LAVITION.md) | Brand + architecture + library model for lavition; two-tier brand split; loft naming history |
| [DEBUG.md](doc/claude/DEBUG.md) | Debugging utilities and tools |
| [RELEASE.md](doc/claude/RELEASE.md) | Release checklist and version history |
| [MOVING.md](doc/claude/MOVING.md) | One-time runbook: transfer loft into the `loft-lang` org (free-via-redirect vs gotchas, the `scripts/rewrite-org.sh` reference automation, cross-repo + library-naming cleanup) |
| [CHANGELOG.md](CHANGELOG.md) | User-facing release notes (shipped in release archives) |
| [CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md) | Full technical changelog — opcode/slot/phase detail |
| [CAVEATS.md](doc/claude/CAVEATS.md) | Verifiable edge cases + reproducers and test references |
| [COROUTINE.md](doc/claude/COROUTINE.md) | Coroutine design — stackful `yield`, `iterator<T>`, `yield from` (planned, 1.1+) |
| [LIFETIME.md](doc/claude/LIFETIME.md) | Dep tracking + scope-based freeing — dep fields, Text vs Reference, closures |
| [HTML_EXPORT.md](doc/claude/HTML_EXPORT.md) | `loft --html` pipeline: cdylib codegen, WebGL2 bridge, frame-yield contract |
| [../PROMPTS.md](doc/PROMPTS.md) | Working with Claude — practices + when to use each prompt |

---

## Reading by goal

| Goal | Start here |
|---|---|
| Understand the language syntax | [LOFT.md](doc/claude/LOFT.md), then [STDLIB.md](doc/claude/STDLIB.md) |
| Find an existing library before writing code | [LIBRARIES.md](doc/claude/LIBRARIES.md), then `loft install <name>` |
| Add a feature to the compiler | [COMPILER.md](doc/claude/COMPILER.md) → [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) → [INTERNALS.md](doc/claude/INTERNALS.md) |
| Debug a runtime crash | **loft-debug skill** (`.claude/skills/loft-debug/SKILL.md`) → [GitHub Issues](https://github.com/loft-lang/loft/issues) (`gh issue list`) + [PROBLEMS.md](doc/claude/PROBLEMS.md) → [TESTING.md](doc/claude/TESTING.md) § LogConfig → [INTERNALS.md](doc/claude/INTERNALS.md) |
| Add a native (Rust) stdlib function | [INTERNALS.md](doc/claude/INTERNALS.md) § Native Function Registry, then `default/01_code.loft` |
| Plan or review enhancements | [PLANNING.md](doc/claude/PLANNING.md), then [PERFORMANCE.md](doc/claude/PERFORMANCE.md) |
| Improve interpreter or native performance | [PERFORMANCE.md](doc/claude/PERFORMANCE.md) |
| Implement a PLANNING.md item | [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) |
| Understand the parallel execution model | [THREADING.md](doc/claude/THREADING.md), then [INTERNALS.md](doc/claude/INTERNALS.md) § Parallel Execution |
| Set up logging in a loft program | [STDLIB.md](doc/claude/STDLIB.md) § Logging, then [LOGGER.md](doc/claude/LOGGER.md) |
| Understand the heap / memory model | [DATABASE.md](doc/claude/DATABASE.md), then [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) § DbRef |
| Improve the test suite | [TESTING.md](doc/claude/TESTING.md), then `tests/scripts/` and `tests/docs/` |
| Find test coverage gaps | [TESTING.md](doc/claude/TESTING.md) § Test Coverage Gaps |
| Fix a known bug | [GitHub Issues](https://github.com/loft-lang/loft/issues) (`gh issue list --label "wa:none"`) → [TESTING.md](doc/claude/TESTING.md); close with `Fixes #NNN` |
| Retest caveats before release | [CAVEATS.md](doc/claude/CAVEATS.md) |
| Add or fix native code generation | [NATIVE.md](doc/claude/NATIVE.md) → [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) → [INTERNALS.md](doc/claude/INTERNALS.md) § Native |
| Understand slot assignment / stack layout | [SLOTS.md](doc/claude/SLOTS.md) |
| Implement a planned language feature | [ROADMAP.md](doc/claude/ROADMAP.md) → [PLANNING.md](doc/claude/PLANNING.md) → feature design doc (TUPLES.md / COROUTINE.md / STACKTRACE.md) |
| Add HTTP or JSON support | [PLANNING.md](doc/claude/PLANNING.md) § H-tier → [lib_plans/06-web-services/](doc/claude/lib_plans/06-web-services) → [STDLIB.md](doc/claude/STDLIB.md) |
| Implement `loft install <name>` registry | [PKG_REGISTRY.md](doc/claude/PKG_REGISTRY.md) → [PACKAGES.md § Open work](doc/claude/PACKAGES.md#open-work) → [PACKAGES.md](doc/claude/PACKAGES.md) |
| Build or understand the `server` library | [lib_plans/future/08-server/README.md](doc/claude/lib_plans/future/08-server/README.md) |
| Build or understand the `game_client` library | [lib_plans/64-game-client/README.md](doc/claude/lib_plans/64-game-client/README.md) |
| Write or review `.loft` files | `.claude/skills/loft-write/SKILL.md` |
| Understand variable lifetimes / dep tracking | [LIFETIME.md](doc/claude/LIFETIME.md) → [DATABASE.md](doc/claude/DATABASE.md) |

---

## Debug logging — `LOFT_LOG` quick reference

Set before `cargo test` to control what appears in `tests/dumps/*.txt`:

| Value | What you get |
|---|---|
| *(unset)* or `full` | IR + bytecode + execution, slot annotations (default) |
| `static` | IR + bytecode only — fastest for codegen debugging |
| `minimal` | Execution trace for `test` only — cleanest for runtime bugs |
| `ref_debug` | Full + stack snapshots after every Ref/CreateStack op |
| `bridging` | Execution + bridging-invariant warnings |
| `crash_tail:N` | Last N execution lines; flushed on panic |
| `fn:<name>` | Only the named function |
| `variables` | Variable table (name, type, scope, slot, live interval) per function |
| `all_fns` | Bytecode of all functions including `default/` built-ins |

Full API: [TESTING.md](doc/claude/TESTING.md) § LogConfig and `src/log_config.rs`.

Every opcode that produces or consumes a `DbRef` shows an inline struct/vector dump in the trace:
`#3.1 { name: "x", inner: #2.1 { val: 42 } }`. Tune with:

| Env var | Default | Effect |
|---|---|---|
| `LOFT_DUMP_DEPTH` | `2` | Max nesting depth before `{...}` / `[N items...]` |
| `LOFT_DUMP_ELEMENTS` | `8` | Max vector elements before `...N more` |

Also works with `cargo run --bin loft` when `LOFT_LOG` is set (writes to stderr). See
[DEBUG.md](doc/claude/DEBUG.md) § Database / Struct Debug Dumps.
