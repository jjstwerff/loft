
# Claude Code Instructions for the Loft Project

## What loft is

**loft** is a tree-walking interpreter for the **loft** programming language, written in Rust.
Loft is a statically typed, expression-oriented language with struct/enum support, a
store-based heap, and a standard library loaded from `default/*.loft`.

### Where loft sits — the three-layer stack

loft is the **language** layer of a larger project:

- **lavition** — the **engine**: an editor with loft as its built-in scripting
  language, positioned as a **rapid-prototyping game engine for indie game
  developers and studios**.  This is the long-term destination, built out over
  time, and is the engine's own name — **not** a former name for loft.  (The
  language used the `.lav` extension while it lived inside the engine; the
  2026-03-08 "move to the loft name" split gave the *language* its own identity
  — `.lav` → `.loft` — distinct from the engine.)
- **loft** — the **language** (this repo): the statically-typed scripting
  language embedded in the engine.  The name was chosen for being **easy and
  descriptive**, deliberately *not* a unique or trademarkable brand word — the
  distinctive, ownable identity lives in *lavition*.  So it doesn't matter that
  "loft" is a common word (and already taken on crates.io by an unrelated
  project): the language ships under the lavition umbrella, never as a
  standalone brand.
- **moros** (RPG) and **dryopea** (sci-fi tower-defence) — **games built on the
  lavition engine**, written in loft.  They are the canonical dogfood consumers
  that drive language work (see the development cadence below).

---

## Development cadence — the dogfood loop

The project's development model is:

> **Build a real consumer → harvest the language lessons → fix the language → ship the lessons as a release.**

Not toy programs.  Not abstract design.  Real tools that have to work.
The branch-review viewer ([@PLAN35](doc/claude/plans/finished/35-branch-review-viewer/README.md)),
the tracker indexer ([@PLAN37](doc/claude/plans/future/37-tracker-index/README.md)),
and [`lib/markdown/`](lib/markdown/) are the canonical consumers — each one
drove a wave of language enhancements (closures, bounded generics, native codegen
maturity, `lib/process`/`lib/fs_watch`/`lib/cache` plan slots, eight P-issues from
the dogfood pass) that landed BEFORE the next minor release.

When picking work, prefer the path that exercises the language against a real
consumer over the path that doesn't.  When a feature slice surfaces a language
gap, the workflow at
[DEVELOPMENT.md § Inserting Discovered Enhancements Into the Active Plan](doc/claude/DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan)
applies — fix it on the spot when XS/S, route to canonical home (P-issue,
`## Open work` section, `lib_plans/future/` slot) when bigger.

Releases bundle the harvest.  See [CHANGELOG.md](CHANGELOG.md) for the
"language lessons → release" cadence in practice — every minor release
since 0.8.3 (WebAssembly), 0.8.4 (Awesome Brick Buster), and 0.8.5 (Language
Maturity, drafted) has been organised around the consumer that drove it.

---

## Key commands

```bash
cargo run --bin loft -- myprogram.loft        # run a loft program
cargo run --bin loft -- --help                # CLI help
cargo run --bin gendoc                        # regenerate doc/*.html
make ci                                       # fmt → clippy → test (full local gate)
make test                                     # clippy + test; output in result.txt
./scripts/find_problems.sh --bg               # background full-suite run
./scripts/find_problems.sh --peek             #   inspect mid-run
./scripts/find_problems.sh --wait             #   block for summary
make index                                    # rebuild index/tags.json (plan-37)
./scripts/idx tag:@P259                       # tracker-ref lookup (plan-37; prefer over grep -rn; --before/--after/--para flags for context)
make view                                     # branch-aware doc + code viewer (plan-35; SSH port-forward 8765; /tag/<bare> for tracker refs)
```

<!-- noindex region: phase-06 sed pass shouldn't migrate the
     bare-name examples that explain the convention. -->
## Tracker tags (plan-37) <!--noindex-->

Tracker references in docs use the `@`-prefixed form so that
regex matches are unambiguous (the bare-name `P259` regex <!--noindex-->
collides with `2P259`, `P2590`, prose like "the P259 fix <!--noindex-->
forward"): <!--noindex-->

- **P-issues**: `@P259`, `@P229b`, `@P262`.
- **Plans (canonical)**: `@PLN3` = a [`loft-lang/plans`](https://github.com/loft-lang/plans)
  issue (the cross-ecosystem plan id = its issue number).
- **Plan dirs + phases (legacy/local)**: `@PLAN22`, `@PLAN35-01`,
  `@PLAN22-2d-iii.a` (sub-phases via `-` and `.`) — point at the design dir
  (`plans/<NN>/`), per-tree.

Adoption is incremental — bare-name forms (`P259`, `plan-22 <!--noindex-->
phase 03`) still work in prose; the indexer (`make index`)
tracks both under separate `legacy:` keys for transition
metering.

### Looking up tracker references — use `./scripts/idx`

Default workflow for "where is X referenced?":

```bash
./scripts/idx tag:@P259               # exact @-prefixed tag
./scripts/idx tag:legacy:P259         # bare-name (transition)
./scripts/idx prefix:@PLAN22          # all PLAN22-* refs
./scripts/idx file:doc/.../PROBLEMS.md  # tags in one file
./scripts/idx incoming:doc/.../PROBLEMS.md  # backlinks (who links to me)
./scripts/idx incoming:plans/finished/22-mutable-closures/  # trailing / → README.md
./scripts/idx all | jq '.[:10]'       # top 10 by reference count
./scripts/idx broken                  # broken @-refs
./scripts/idx broken-links            # broken markdown links (phase 09)
./scripts/idx help                    # usage block
```

For more than just one-line context, `tag:` queries accept
excerpt flags:

```bash
./scripts/idx tag:legacy:P259 --before 2 --after 5
./scripts/idx tag:legacy:P259 --before 1 --para 1
./scripts/idx tag:legacy:P259 --max-bytes 1024
```

`--before` / `--after` are line counts; `--para N` extends
forward until N consecutive empty lines (good for code
comment blocks); `--max-bytes` caps each excerpt (default
4096) so long PROBLEMS.md rows truncate gracefully instead
of dumping kilobytes per ref.

Prefer `./scripts/idx` over `grep -rn '@P259' …` — it's
faster, returns structured JSON, and avoids pulling
unnecessary file content into context.  Run `make index`
first if `index/tags.json` is missing or stale (the
pre-commit hook from phase 02 keeps it fresh on most
workflows).

For any refactor likely to surface multiple test failures, kick off
`find_problems.sh --bg` before going back to editing.  It runs
`cargo test --release --no-fail-fast` detached, tees the log to
`/tmp/loft_test.log`, and writes a structured summary to
`/tmp/loft_problems.txt` on completion (FAILED list, stdout blocks,
SIGSEGV context, plus a wrap-suite `--nocapture` re-run when a
crash masks a specific `.loft` filename).  See
[TESTING.md](doc/claude/TESTING.md) § "Preferred shape —
background + peek + wait" for the full rationale.

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
       └─ src/fill.rs       233 opcode implementations
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

---

## Default standard library load order

```
default/01_code.loft    — operators, math, text, collections
default/02_files.loft   — File I/O, Format, EnvVariable, path helpers
default/03_text.loft    — text utilities
```

---

## Loft language patterns

For writing or reviewing `.loft` files see the **loft-write skill**
(`.claude/skills/loft-write/SKILL.md`) — naming conventions, type reference, format
strings, loop attributes, lambdas, known bugs and workarounds, pre-flight checklist.

Full language reference: [LOFT.md](doc/claude/LOFT.md) and [STDLIB.md](doc/claude/STDLIB.md).

---

## Branch policy — MANDATORY

**Direct commits to `main` are not allowed.**

All changes — features, bug fixes, refactors, documentation updates — must land on a
feature branch and reach `main` only through a pull request.

### Why

`main` is the release branch. Every commit on `main` is expected to be releasable.
Direct commits bypass code review, CI, and the structured commit sequence documented in
[DEVELOPMENT.md](doc/claude/DEVELOPMENT.md). Feature branches keep `main` clean and
give each item a traceable history.

### Rules

1. **Never `git commit` directly on `main`.** If you accidentally land on `main`, move
   the change to a feature branch before anything else.
2. **Pushing commits is OK by default — unless there's an open PR on the branch
   that the push would disturb.**  For a long-lived working branch with no open
   PR, push freely after each green-CI commit so the remote stays in sync (the
   user wants commits visible without having to ask each time).  When the
   branch has an open PR, do NOT push without an explicit user instruction —
   force-pushes, rebases, or unexpected commits disrupt review-in-progress.
   Check with `gh pr list --head <branch>` before pushing if uncertain.
3. **Never create a branch or open a PR unless the user explicitly asks.**
   Each pull request costs the user real review time — more than the code took to
   write.  Default mode is: work on the current branch, commit locally (or push
   per Rule 2), report what changed, and wait.  Only run `gh pr create` or
   `git checkout -b` after the user explicitly says "create PR", "open a PR",
   "merge", or "switch to a new branch".
   - "fix X" or "implement Y" is *not* a PR instruction.  Commit locally and stop.
   - A previous prompt that said "open a PR" does not authorise the next PR.
     Ask each time, or infer from the exact current prompt.
   - When in doubt about PR creation, summarise what is ready and ask.
4. Create branches from the tip of `main`.  **Default to a GENERAL
   name** (`quality-pass`, `cleanup`, `housekeeping`, `work`) so the
   branch can host any theme and accumulate work across sessions —
   each new branch eventually has to rebase against a moving `main`,
   surfaces conflicts in unrelated files, and often fails CI on
   patterns the new branch didn't author.  ONE long-lived working
   branch with cross-theme commits is the cheaper failure mode.
   ONLY a substantial plan (well-defined arc with its own design
   doc — e.g. `plan-06-arc`, `lsp-server`) earns a specific branch
   name.  Do not open a second branch unless the user
   explicitly asks ("start a new branch", "fresh branch for X",
   "switch to a new branch").
5. Merging back to `main` is done via a GitHub pull request — not a local `git merge`.

---

## Debugging policy — MANDATORY

### Never use `git bisect` or `git checkout HEAD -- <files>` to investigate bugs

**`git bisect` is prohibited.**  Running bisect requires compiling and testing dozens of
commits autonomously.  Claude cannot do this reliably: context windows are finite,
intermediate states are inconsistent, and the process routinely requires reverting
working-in-progress files — destroying multi-session work that is not yet committed.

**`git checkout HEAD -- <file>` to "reset and try again" is prohibited.**  This silently
discards uncommitted changes on specific files.  When multiple files are in flight across
a feature branch, resetting individual files breaks invariants between them and produces
states that are harder to debug than the original problem.

**Use these approaches instead:**

- Read the failing test's dump file (`tests/dumps/*.txt`) — it contains the full IR,
  bytecode, and execution trace.  The root cause is almost always visible there.
- Add `LOFT_LOG=minimal` or `LOFT_LOG=crash_tail:50` to the failing test to narrow down
  the execution step.
- Read the relevant source files and reason about the code path.  A focused read of
  3–5 files is faster and safer than any automated bisect.
- If a regression appeared after a specific recent commit, use `git show <commit>` or
  `git diff <commit>^ <commit>` to read that change — do not re-run old code.

---

## Bug-filing policy — MANDATORY

**When you surface a bug, the default is to FIX it — not to file it.**

While diagnosing or fixing a bug you will often surface *other* bugs — sibling
shapes, latent issues flagged in code comments, symptoms unrelated to the active
fix.  These are the **cheapest bugs you will ever fix**: the code paths are loaded
into your head, the diagnostic infrastructure is warm, a reproducer is within
reach.  That is an argument for *fixing* them on the spot (with a regression
test) — **not** for filing them.  Filing only documents a bug *for later*, and
"later" pays again to re-derive the scope, repro, and mechanism you have right
now.  We are usually hunting and solving bugs with no deadline; solving is the
work, and a backlog of filed-but-unfixed rows is not progress.

**Origin is never worth recording.**  Which commit introduced a bug, or its
history, tells you nothing about making it correct.  Scope (what triggers it —
the edges) and root cause (the mechanism in the *present* code) are what you fix
from — never a `git bisect` / archaeology narrative.

**Filing documents a bug for the future, so file only when you are NOT fixing it
now.**  Two cases:

- **It blocks the task you're on.**  File a bookmark + use a workaround so you
  can keep moving, then come back.  This is the clearest reason to file.
- **It's genuinely too big to fix now** (M+ effort / needs design).  Route it to
  its canonical home (see [DEVELOPMENT.md § Inserting Discovered
  Enhancements](doc/claude/DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan)).

When you DO file: **open a GitHub Issue** (`gh issue create`, the `bug_report`
template) — NOT a PROBLEMS.md row (PROBLEMS.md is now the closed/historical
archive; see [ISSUE_TRACKING.md](doc/claude/ISSUE_TRACKING.md)).  Include a minimal
reproducer (expected vs observed on each backend), a `sev:` + `area:` label, and a
**`wa:*` workaround label whose claim you VERIFIED** (run it, both backends — a
wrong workaround is worse than `wa:none`; see
[ISSUE_TRACKING.md § Workarounds](doc/claude/ISSUE_TRACKING.md#workarounds--the-agents-can-you-keep-moving-signal)).
Label meanings: [`.github/LABELS.md`](.github/LABELS.md).  Save the repro to
`/tmp/p_followups/` or add a `tests/scripts/` regression if it deserves CI lock-in.
When the bug is FIXED, reference the issue in the commit (`Fixes #NNN`) so GitHub
closes it — but do **not** file at all for a bug you fix in the same change: the
fix + its regression test ARE the record.

**Inside an investigation plan, don't file at all** — the plan's probes + cluster
docs already document every shape (see
[`plans/_INVESTIGATION_TEMPLATE.md`](doc/claude/plans/_INVESTIGATION_TEMPLATE.md)).
A separate P-issue would double-document the same shape.

This is **not** a license to scope-creep the active fix.  When you're focused on
shipping fix X, an unrelated bug Y you can't fix without derailing X is exactly
the "not fixing it now" case — file Y (or pick it up as its own focused change
next); don't bundle it into X's patch unless they share a single fix site.

### Inserting fixes vs filing — see DEVELOPMENT.md

The rule above already defaults to **fixing**.  This section is the
related *consumer-gap* case — a missing language/stdlib feature a real
consumer needs.  When the gap is XS or S (under half a day) AND the
consumer code that uses the workaround is fresh in working memory,
prefer **inserting a step into the active plan that fixes the gap
directly**, then resuming the feature work — the language / stdlib gets
sturdier and the workaround never enters shipped code.

Routing the discovered item to its canonical home (P-issue / `## Open
work` row in STDLIB.md / NATIVE.md / COMPILER.md / new lib_plans slot)
is what to do when an inline fix isn't appropriate (M+ effort, needs
design, touches unrelated subsystems).

Big deferred features get their own plan slot (`plans/future/<NN>/` or
`lib_plans/future/<NN>/`) — never a row in a parallel catalog.

Full procedure + decision tree: see
[DEVELOPMENT.md § Inserting Discovered Enhancements Into the Active Plan](doc/claude/DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan).

---

## Git safety — MANDATORY

### Never use `git stash pop` or `git pull` with uncommitted changes

**`git stash` + `git stash pop` is prohibited.**  Stash pop applies changes as a
merge, which routinely produces conflicts across dozens of files.  A failed pop
leaves the working directory in an unrecoverable state — all uncommitted work is
destroyed.  This has caused complete loss of multi-hour sessions.

**`git pull` with uncommitted changes is prohibited.**  Pull fetches and merges,
which also conflicts with in-flight work.

**Use these approaches instead:**

- **To compare with main:** use `git diff main -- <file>` or
  `git show origin/main:<file>` — no branch switch needed.
- **To check if a bug is pre-existing:** commit current work first (even as WIP
  on the feature branch), then compare.
- **To update from remote:** commit first, then `git pull`, resolve if needed.
- **To test on clean main:** commit, `git checkout main`, test, `git checkout -`
  to return.

The rule: **always commit before any operation that changes the working tree.**

---

## Documentation index

| File | Topic |
|---|---|
| [LOFT.md](doc/claude/LOFT.md) | Loft language reference (syntax, types, operators, control flow) |
| [STDLIB.md](doc/claude/STDLIB.md) | Standard library API (math, text, collections, file I/O, logging, parallel) |
| [COMPILER.md](doc/claude/COMPILER.md) | Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode |
| [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) | Value/Type enums in detail; 233 bytecode operators; State layout |
| [DATABASE.md](doc/claude/DATABASE.md) | Store allocator, Stores schema, DbRef, vector/tree/hash/radix implementations |
| [INTERNALS.md](doc/claude/INTERNALS.md) | calc.rs, stack.rs, create.rs, native.rs, ops.rs, png_store.rs, parallel.rs, main.rs, logger.rs |
| [THREADING.md](doc/claude/THREADING.md) | Parallel execution — `par(...)`, `par_light(...)`, thread safety analysis, store isolation |
| [INTERFACES.md](doc/claude/INTERFACES.md) | Interface/trait system — bounded generics, operator overloading, phase design |
| [WASM.md](doc/claude/WASM.md) | Reference — WASM runtime architecture: wasm32-wasip2 target, VirtFS, layered FS, host bridges, feature gates, threading two-tier design, frame yield, PNG decoding, logging.  All major W1.x phases shipped (W1.15 CallRef, W1.16 file I/O, W1.17 store locks, W1.18-1..5 worker thread infrastructure, W1.19 random, W1.20 time, frame yield, etc.).  Lone open item: W1.18-6 (test enablement for `19-threading.loft` under Node.js Worker Threads — single small task, not plan-shaped).  The doc's "Implementation Plan" Steps 1-14 + FS-A..FS-F are HISTORICAL build records (all shipped). |
| [WINDOWS.md](doc/claude/WINDOWS.md) | Windows support — honest verified state (`--interpret` ✅; `--native` multi-lib + server networking + `parallel{}` unverified/gated), known gaps G1–G4 with per-gap VM-validation runbook (the failures only repro on a real Windows host), and the close-a-gap loop |
| [WINDOWS_SESSION.md](doc/claude/WINDOWS_SESSION.md) | Session-prep checklist for when temporary Windows access arrives — priority-ordered investigations (v2 probe → G2 LNK1181 → G3 multi-lib rlib → G4 → opportunistic), time budget, pre-flight steps, what NOT to do.  Companion to WINDOWS.md (reference); this is the action plan |
| [LOGGER.md](doc/claude/LOGGER.md) | Runtime logging framework (log_info/warn/error/fatal, config, rate limiting, production mode) |
| [TESTING.md](doc/claude/TESTING.md) | Test framework, `LogConfig` debug-logging presets, `LOFT_LOG` env var, suite files |
| [DOC.md](doc/claude/DOC.md) | HTML documentation generation (gendoc.rs + documentation.rs) |
| [DESIGN.md](doc/claude/DESIGN.md) | Algorithm catalog with complexity analysis and enhancement priorities |
| [CODE.md](doc/claude/CODE.md) | Code quality rules (naming, functions, doc comments, clippy, dependency policy) |
| [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) | Development workflow — branching, WIP commit, rebase sequence, CI |
| [SLOTS.md](doc/claude/SLOTS.md) | Stack slot assignment — two-zone design, diagnostic tools, open issues |
| [ISSUE_TRACKING.md](doc/claude/ISSUE_TRACKING.md) | **Where bugs live: open bugs → GitHub Issues; investigations → files; closed → PROBLEMS.md archive.**  The convention (labels, `@GH###` refs, cross-repo), the workaround-as-signal rule, and the migration plan |
| [.github/LABELS.md](.github/LABELS.md) | Issue-label glossary — what `sev:`/`wa:`/`area:` mean (e.g. `area:codegen`) WITHOUT reading the source |
| [PROBLEMS.md](doc/claude/PROBLEMS.md) | **Closed/historical bug archive** (FIXED rows = regression record; the big `###` entries are design references).  OPEN bugs are now [GitHub Issues](https://github.com/jjstwerff/loft/issues) |
| [QUALITY.md](doc/claude/QUALITY.md) | Reference + open work — open programmer-biting issues, active sprint (P54 JsonValue enum), active designs (Q1-Q4 JSON ecosystem, P54-U unified parser, Dep-inference for native fn returns), compiler blockers (B2-B7 struct-enum bugs), enhancement tiers, recommended landing order.  C54 (integer→i64) historical record kept as the canonical "LANDED via …" closure pattern.  See [§ Open work — actionable summary](doc/claude/QUALITY.md#open-work--actionable-summary) for the at-a-glance status table. |
| [DESIGN_DECISIONS.md](doc/claude/DESIGN_DECISIONS.md) | Closed-by-decision register — check before proposing features already declined (C3 / C38 / C54.D / …) |
| [FORMATTER.md](doc/claude/FORMATTER.md) | Source formatter design and implementation notes |
| [INCONSISTENCIES.md](doc/claude/INCONSISTENCIES.md) | Known language design inconsistencies and asymmetries |
| [PERFORMANCE.md](doc/claude/PERFORMANCE.md) | Reference — performance analysis (benchmark results, root-cause analysis vs CPython / hand-written Rust, how the interpreter executes, wasm-vs-native gap analysis, design content for each planned optimization).  Open optimization follow-ups (P1-P3, N1-N3, W1) in `## Open work` section. |
| [GOALS.md](doc/claude/GOALS.md) | **What loft is *for*, and the goals that serve it.**  Leads with the **Purpose**: loft is the *foundation*, the end is the library/infrastructure on top (lavition); *do the hard plumbing so it's fun to pick up* — **fun-on-pickup** is the acceptance test; built for its own sake, adoption a *consequence not a goal*.  Six **stack-wide** goals (A soundness / B release & legibility / C capability via dogfood / D parity / **E predictable memory** — source is the truth, *surpass Rust on safe-AND-predictable* / **F friction-free** — serve the programmer not the compiler), each with a runnable **Check**; the goals hold for the libraries too — they **don't meet them yet** (the coming shift).  Plus the two-engine (dogfood + sanitizer) model + the method-mirrors-the-goals section |
| [PLANNING.md](doc/claude/PLANNING.md) | Priority-ordered enhancement backlog |
| [ROADMAP.md](doc/claude/ROADMAP.md) | Items in implementation order, grouped by milestone (0.9.0 / 1.0.0 / 1.1+) |
| [plans/README.md](doc/claude/plans/README.md) | Multi-phase **core-language** initiatives (current / future / deferred / finished) — compiler, runtime, validation matrices, codegen arcs, language features.  Max 2-3 active plans. |
| [lib_plans/README.md](doc/claude/lib_plans/README.md) | Multi-phase **library** initiatives (current / future / deferred / finished) — `server`, `game_client`, graphics, regex, package format, asset pipeline, web examples, IDE.  Same `≤3 active` discipline as `plans/`; numbering independent. |
| [BROADENING.md](doc/claude/BROADENING.md) | Strategic evaluation — using loft beyond games (CLI, server, data), sequenced unlocks |
| [TUPLES.md](doc/claude/TUPLES.md) | Tuple design — multi-value returns, deconstruction, match destructuring |
| [STACKTRACE.md](doc/claude/STACKTRACE.md) | Stack trace introspection — `stack_trace()` API, `StackFrame`, `ArgValue` |
| [NATIVE.md](doc/claude/NATIVE.md) | Reference — native code generation pipeline (`src/generation/`), architecture + `codegen_runtime`, per-Op dispatch, N1-N8 implementation history.  `--native` is shipped (CI-gated, 108/108 native tests pass).  Open follow-up work in `## Open work` section. |
| [PACKAGES.md](doc/claude/PACKAGES.md) | Reference — package format spec (`loft.toml`), package layout, function binding model, build pipeline, target matrix (interpreter / native / WASM / `--html`), OpenGL case study, **library-owned wasm bridges** (`[wasm.bridge]` manifest section drives per-library `wasm/src/lib.rs` + `wasm/host.js` extensions; lib/imaging is the canonical example — see [lib_plans/finished/29-library-wasm-bridges](doc/claude/lib_plans/finished/29-library-wasm-bridges/README.md)), security model.  Format is SHIPPED: 14 `lib/*` packages already use it.  Open infrastructure work (registry MVP, lock file) in `## Open work` section; execution arc (per-library extraction from monorepo) in [lib_plans/12-library-extraction/](doc/claude/lib_plans/12-library-extraction/README.md). |
| [PKG_REGISTRY.md](doc/claude/PKG_REGISTRY.md) | Draft — file-based registry MVP design.  `loft install <name>` against a static `registry.json` (GitHub-hosted) + tarballs in GitHub releases.  Migration to a real server later is a drop-in URL swap with **identical end-user behaviour** (§ The invariant).  Implementation phases R1-R9.  Unblocks lib_plans/12-library-extraction Phase 4+. |
| [REGISTRY_BOOTSTRAP.md](doc/claude/REGISTRY_BOOTSTRAP.md) | One-time runbook for bringing `loft-lang/registry` online.  Generates Ed25519 keypair via `loft-keygen` binary, embeds public key in `src/registry_keys.rs`, ships CI templates from `doc/claude/registry_ci_template/`.  Includes § Step 1.5 trust-root storage guidance (3-2-1 backup rule applied to a 32-byte secret). |
| [REGISTRY_RECOVERY.md](doc/claude/REGISTRY_RECOVERY.md) | Trust-root incident runbooks: Scenario A (laptop dead, backup intact — 30min drill, no user impact); Scenario B (laptop dead, backups also gone — multi-key rotation, 6mo transition window, no user impact); Scenario C (key COMPROMISED — same-day emergency distrust, CVE communication, audit window).  Plus annual recovery-drill checklist. |
| [REGISTRY_SUBMIT.md](doc/claude/REGISTRY_SUBMIT.md) | Author-facing library-submission guide: prerequisites, 5-step submit flow (tag → `loft package` → `gh release create` → PR with version row → CI + maintainer review), subsequent releases, yanking, what NOT to ship, troubleshooting (sha256 mismatch, reproducible-build mismatch), mirror policy. |
| [LIBRARY_AUTHORING.md](doc/claude/LIBRARY_AUTHORING.md) | End-to-end author narrative — `loft new <name>` → develop → `loft package` + `gh release create` → `loft publish` → registry PR → maintain (`loft yank`).  Ties the CLI commands shipped in @PLAN12 author UX sprint (6.16 / `loft new` / 6.7a) into one walkthrough.  Companion to [PACKAGES.md](doc/claude/PACKAGES.md) (format reference) + [REGISTRY_SUBMIT.md](doc/claude/REGISTRY_SUBMIT.md) (manual submit flow, pre-CLI). |
| [LAVITION.md](doc/claude/LAVITION.md) | Brand + architecture + library model for [lavition](https://github.com/lavition) — the universal hex-world editor built on loft.  Two-tier brand split (loft = language ecosystem with descriptive symbol names; lavition = engine brand visible in metadata only).  Library model: data primitives stay in `loft-lang/loft-libs-*`; engine core + plugins in `lavition/`.  Discoverability strategy: "lavition + generic-term" search owns the docs, symbols stay bare.  Companion to [`lavition/lavition`](https://github.com/lavition/lavition) (design vision) + [`lib_plans/future/24-universal-editor/`](doc/claude/lib_plans/future/24-universal-editor/) (extraction plan, predates the lavition brand). |
| [DEBUG.md](doc/claude/DEBUG.md) | Debugging utilities and tools |
| [RELEASE.md](doc/claude/RELEASE.md) | Release checklist and version history |
| [CHANGELOG.md](CHANGELOG.md) | User-facing release notes (shipped in release archives) |
| [CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md) | Full technical changelog — opcode/slot/phase detail for contributors |
| [CAVEATS.md](doc/claude/CAVEATS.md) | Verifiable edge cases and limitations with reproducers and test references |
| [COROUTINE.md](doc/claude/COROUTINE.md) | Coroutine design — stackful `yield`, `iterator<T>`, `yield from` (planned, 1.1+) |
| [LIFETIME.md](doc/claude/LIFETIME.md) | Dependency tracking and scope-based freeing — dep field semantics, Text vs Reference, closures |
| [HTML_EXPORT.md](doc/claude/HTML_EXPORT.md) | Reference — `loft --html` pipeline: cdylib codegen, WebGL2 import bridge, frame-yield contract for browser game loops, `wasm-opt` integration, HTML assembly format.  Where each piece lives in the code today.  Closed @PLAN31 (build sequence + commits) at [`plans/finished/31-html-export/`](doc/claude/plans/finished/31-html-export/README.md). |
| [../PROMPTS.md](doc/PROMPTS.md) | Working with Claude — practices and when to use each prompt in `prompts.txt` |

---

## Reading by goal

| Goal | Start here |
|---|---|
| Understand the language syntax | [LOFT.md](doc/claude/LOFT.md), then [STDLIB.md](doc/claude/STDLIB.md) |
| Add a feature to the compiler | [COMPILER.md](doc/claude/COMPILER.md) → [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) → [INTERNALS.md](doc/claude/INTERNALS.md) |
| Debug a runtime crash | [GitHub Issues](https://github.com/jjstwerff/loft/issues) (`gh issue list`) + [PROBLEMS.md](doc/claude/PROBLEMS.md) (closed archive) → [TESTING.md](doc/claude/TESTING.md) § LogConfig → [INTERNALS.md](doc/claude/INTERNALS.md) |
| Add a native (Rust) standard library function | [INTERNALS.md](doc/claude/INTERNALS.md) § Native Function Registry, then `default/01_code.loft` |
| Plan or review enhancements | [PLANNING.md](doc/claude/PLANNING.md), then [PERFORMANCE.md](doc/claude/PERFORMANCE.md) |
| Improve interpreter or native performance | [PERFORMANCE.md](doc/claude/PERFORMANCE.md) — benchmarks, root-cause analysis, optimisation designs |
| Implement a PLANNING.md item | [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) — branching, commit order, CI |
| Understand the parallel execution model | [THREADING.md](doc/claude/THREADING.md), then [INTERNALS.md](doc/claude/INTERNALS.md) § Parallel Execution |
| Set up logging in a loft program | [STDLIB.md](doc/claude/STDLIB.md) § Logging, then [LOGGER.md](doc/claude/LOGGER.md) |
| Understand the heap / memory model | [DATABASE.md](doc/claude/DATABASE.md), then [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) § DbRef |
| Improve the test suite | [TESTING.md](doc/claude/TESTING.md), then `tests/scripts/` and `tests/docs/` |
| Find test coverage gaps | [TESTING.md](doc/claude/TESTING.md) § Test Coverage Gaps |
| Fix a known bug | [GitHub Issues](https://github.com/jjstwerff/loft/issues) (`gh issue list --label "wa:none"` for blockers) → [TESTING.md](doc/claude/TESTING.md); close with `Fixes #NNN` |
| Retest caveats before release | [CAVEATS.md](doc/claude/CAVEATS.md) — each entry has a reproducer and test reference |
| Add or fix native code generation | [NATIVE.md](doc/claude/NATIVE.md) → [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) → [INTERNALS.md](doc/claude/INTERNALS.md) § Native |
| Understand slot assignment / stack layout | [SLOTS.md](doc/claude/SLOTS.md) |
| Implement a planned language feature (Tuples/Coroutines/etc.) | [ROADMAP.md](doc/claude/ROADMAP.md) → [PLANNING.md](doc/claude/PLANNING.md) → feature design doc (TUPLES.md / COROUTINE.md / STACKTRACE.md) |
| Add HTTP or JSON support | [PLANNING.md](doc/claude/PLANNING.md) § H-tier → [lib_plans/future/06-web-services/](doc/claude/lib_plans/future/06-web-services/) → [STDLIB.md](doc/claude/STDLIB.md) |
| Implement `loft install <name>` registry | [PKG_REGISTRY.md](doc/claude/PKG_REGISTRY.md) (file-based MVP design, R1-R9 implementation phases) → [PACKAGES.md § Open work](doc/claude/PACKAGES.md#open-work) (sub-arc list) → [PACKAGES.md](doc/claude/PACKAGES.md) (format reference) |
| Build or understand the `server` library | [lib_plans/future/08-server/README.md](doc/claude/lib_plans/future/08-server/README.md) |
| Build or understand the `game_client` library | [lib_plans/future/10-game-client/README.md](doc/claude/lib_plans/future/10-game-client/README.md) |
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

Every opcode that produces or consumes a `DbRef` shows an inline struct/vector dump
in the trace: `#3.1 { name: "x", inner: #2.1 { val: 42 } }`.  Tune with:

| Env var | Default | Effect |
|---|---|---|
| `LOFT_DUMP_DEPTH` | `2` | Max nesting depth before `{...}` / `[N items...]` |
| `LOFT_DUMP_ELEMENTS` | `8` | Max vector elements before `...N more` |

Also works with `cargo run --bin loft` when `LOFT_LOG` is set (writes to stderr).
See [DEBUG.md](doc/claude/DEBUG.md) § Database / Struct Debug Dumps for details.
