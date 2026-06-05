
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
[@PLAN37](doc/claude/plans/future/37-tracker-index/README.md), [`lib/markdown/`](lib/markdown/))
each drove a wave of language work that landed BEFORE the next minor release.

When picking work, prefer the path that exercises the language against a real consumer. When a
feature slice surfaces a language gap, fix it on the spot when XS/S, else route to its canonical
home — [DEVELOPMENT.md § Inserting Discovered Enhancements](doc/claude/DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan).

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
./scripts/idx tag:@P259                       # tracker-ref lookup (plan-37; prefer over grep -rn)
make view                                     # branch-aware doc + code viewer (plan-35; SSH port-forward 8765)
```

<!-- noindex region: don't migrate the bare-name examples that explain the convention. -->
## Tracker tags (plan-37) <!--noindex-->

Tracker refs use an `@`-prefix so regex matches are unambiguous (the bare-name `P259` <!--noindex-->
regex collides with `2P259`, `P2590`, and prose):

- **P-issues**: `@P259`, `@P229b`, `@P262`.
- **Plans (canonical)**: `@PLN3` = a [`loft-lang/plans`](https://github.com/loft-lang/plans)
  issue (the cross-ecosystem plan id = its issue number).
- **Plan dirs + phases (legacy/local)**: `@PLAN22`, `@PLAN35-01`, `@PLAN22-2d-iii.a` — point at
  the design dir `plans/<NN>/`.

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
   asking). With an open PR, do NOT push without explicit instruction — force-pushes, rebases, or
   surprise commits disrupt review. Check `gh pr list --head <branch>` if unsure.
3. **Never create a branch or open a PR unless the user explicitly asks** ("create PR", "open a
   PR", "merge", "switch to a new branch"). "fix X" / "implement Y" is NOT a PR instruction; a
   prior "open a PR" does not authorise the next one. When in doubt, summarise what's ready and ask.
4. Branch from the tip of `main` with a **GENERAL name** (`quality-pass`, `cleanup`, `work`) so one
   long-lived branch hosts cross-theme work — new branches keep re-rebasing against a moving `main`
   and failing CI on patterns they didn't author. Only a substantial plan with its own design doc
   (e.g. `plan-06-arc`, `lsp-server`) earns a specific name.
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
2. **Build the matrix** in throwaway `/tmp` probes, on `--interpret` only. Vary ONE dimension per
   probe along the **composition axes**
   ([plans/README § composition axes](doc/claude/plans/README.md#the-composition-axes--the-dimensions-a-matrix-varies)):
   type-kind / construction-path / context / access / depth / null / backend. Distinctive
   collision-resistant values at every index/position — weak probes (small values, only `[0]`, no
   length check) hide cases. SEE on the interpreter (strides/types surface in seconds); `--native`
   pays a rustc compile per probe — that cost belongs at the final verify (step 7).
3. **Map pass/fail; find the real boundary.** Expect the filed/assumed scope to be wrong — it
   usually is (#263 was *any runtime fn-ref value*; #262 was *every context*; cluster III was three
   different mechanisms).
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

**Inside an investigation plan, don't file** — the plan's probes + cluster docs already document
every shape; a separate P-issue would double-document it.

This is **not** license to scope-creep the active fix: an unrelated bug you can't fix without
derailing X is the "not fixing now" case — file it or pick it up next; don't bundle it into X's
patch unless they share a single fix site.

### Inserting fixes vs filing — see DEVELOPMENT.md

The consumer-gap case: when a missing language/stdlib feature a real consumer needs is XS or S
(under half a day) AND the consumer code is fresh in working memory, prefer **inserting a step into
the active plan that fixes the gap directly**, then resume — so the language/stdlib gets sturdier
and the workaround never enters shipped code. Route to a canonical home (P-issue / `## Open work`
row in STDLIB/NATIVE/COMPILER / new lib_plans slot) when an inline fix isn't appropriate (M+, needs
design, touches unrelated subsystems). Big deferred features get their own plan slot, never a row in
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
| [COMPILER.md](doc/claude/COMPILER.md) | Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode |
| [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) | Value/Type enums; bytecode operators; State layout |
| [DATABASE.md](doc/claude/DATABASE.md) | Store allocator, Stores schema, DbRef, vector/tree/hash/radix |
| [INTERNALS.md](doc/claude/INTERNALS.md) | calc.rs, stack.rs, create.rs, native.rs, ops.rs, png_store.rs, parallel.rs, main.rs, logger.rs |
| [THREADING.md](doc/claude/THREADING.md) | Parallel execution — `par`/`par_light`, thread safety, store isolation |
| [INTERFACES.md](doc/claude/INTERFACES.md) | Interface/trait system — bounded generics, operator overloading |
| [WASM.md](doc/claude/WASM.md) | WASM runtime (wasm32-wasip2, VirtFS, host bridges, threading, frame yield). Major W1.x shipped; lone open item W1.18-6 |
| [WINDOWS.md](doc/claude/WINDOWS.md) | Windows support — verified state, gaps G1–G4 + per-gap VM runbook |
| [WINDOWS_SESSION.md](doc/claude/WINDOWS_SESSION.md) | Action checklist for when Windows access arrives (companion to WINDOWS.md) |
| [LOGGER.md](doc/claude/LOGGER.md) | Runtime logging framework (log_info/warn/error/fatal, config, rate limiting) |
| [TESTING.md](doc/claude/TESTING.md) | Test framework, `LogConfig` presets, `LOFT_LOG`, suite files |
| [DOC.md](doc/claude/DOC.md) | HTML doc generation (gendoc.rs + documentation.rs) |
| [DESIGN.md](doc/claude/DESIGN.md) | Algorithm catalog with complexity analysis |
| [CODE.md](doc/claude/CODE.md) | Code quality rules (naming, functions, doc comments, clippy, deps) |
| [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) | Dev workflow — branching, WIP commit, rebase, CI |
| [SLOTS.md](doc/claude/SLOTS.md) | Stack slot assignment — two-zone design, diagnostics |
| [ISSUE_TRACKING.md](doc/claude/ISSUE_TRACKING.md) | Where bugs live: open → GitHub Issues; investigations → files; closed → PROBLEMS.md. Labels, `@GH###` refs, workaround-as-signal |
| [.github/LABELS.md](.github/LABELS.md) | Issue-label glossary (`sev:`/`wa:`/`area:`) |
| [PROBLEMS.md](doc/claude/PROBLEMS.md) | Closed/historical bug archive (FIXED rows = regression record; `###` entries = design refs) |
| [QUALITY.md](doc/claude/QUALITY.md) | Open programmer-biting issues, active sprint, designs, compiler blockers, landing order. See § Open work table |
| [DESIGN_DECISIONS.md](doc/claude/DESIGN_DECISIONS.md) | Closed-by-decision register — check before proposing declined features |
| [DESIGN_PROTOCOL.md](doc/claude/DESIGN_PROTOCOL.md) | Design Protocol 1 — a design is a testable hypothesis: name the invariant, count re-assertion sites, probe each load-bearing claim to falsify, validate against the prediction (graduated from DESIGN_VERIFICATION C1; fires on load-bearing designs) |
| [DESIGN_VERIFICATION.md](doc/claude/DESIGN_VERIFICATION.md) | Concerns to check a load-bearing design against (append-only; C1 brittleness-over-bugs — GRADUATED → DESIGN_PROTOCOL.md) |
| [FORMATTER.md](doc/claude/FORMATTER.md) | Source formatter design |
| [INCONSISTENCIES.md](doc/claude/INCONSISTENCIES.md) | Known language design inconsistencies |
| [PERFORMANCE.md](doc/claude/PERFORMANCE.md) | Benchmarks, root-cause vs CPython/Rust, wasm-vs-native gap, optimisation designs. Open follow-ups in § Open work |
| [GOALS.md](doc/claude/GOALS.md) | What loft is *for*: purpose (foundation for lavition, fun-on-pickup) + six stack-wide goals A–F, each with a runnable Check |
| [PLANNING.md](doc/claude/PLANNING.md) | Priority-ordered enhancement backlog |
| [ROADMAP.md](doc/claude/ROADMAP.md) | Items in implementation order by milestone (0.9.0 / 1.0.0 / 1.1+) |
| [plans/README.md](doc/claude/plans/README.md) | Multi-phase core-language initiatives (≤2-3 active) |
| [lib_plans/README.md](doc/claude/lib_plans/README.md) | Multi-phase library initiatives (≤3 active; numbering independent) |
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
| [LAVITION.md](doc/claude/LAVITION.md) | Brand + architecture + library model for lavition; two-tier brand split; loft naming history |
| [DEBUG.md](doc/claude/DEBUG.md) | Debugging utilities and tools |
| [RELEASE.md](doc/claude/RELEASE.md) | Release checklist and version history |
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
| Add a feature to the compiler | [COMPILER.md](doc/claude/COMPILER.md) → [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) → [INTERNALS.md](doc/claude/INTERNALS.md) |
| Debug a runtime crash | **loft-debug skill** (`.claude/skills/loft-debug/SKILL.md`) → [GitHub Issues](https://github.com/jjstwerff/loft/issues) (`gh issue list`) + [PROBLEMS.md](doc/claude/PROBLEMS.md) → [TESTING.md](doc/claude/TESTING.md) § LogConfig → [INTERNALS.md](doc/claude/INTERNALS.md) |
| Add a native (Rust) stdlib function | [INTERNALS.md](doc/claude/INTERNALS.md) § Native Function Registry, then `default/01_code.loft` |
| Plan or review enhancements | [PLANNING.md](doc/claude/PLANNING.md), then [PERFORMANCE.md](doc/claude/PERFORMANCE.md) |
| Improve interpreter or native performance | [PERFORMANCE.md](doc/claude/PERFORMANCE.md) |
| Implement a PLANNING.md item | [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) |
| Understand the parallel execution model | [THREADING.md](doc/claude/THREADING.md), then [INTERNALS.md](doc/claude/INTERNALS.md) § Parallel Execution |
| Set up logging in a loft program | [STDLIB.md](doc/claude/STDLIB.md) § Logging, then [LOGGER.md](doc/claude/LOGGER.md) |
| Understand the heap / memory model | [DATABASE.md](doc/claude/DATABASE.md), then [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) § DbRef |
| Improve the test suite | [TESTING.md](doc/claude/TESTING.md), then `tests/scripts/` and `tests/docs/` |
| Find test coverage gaps | [TESTING.md](doc/claude/TESTING.md) § Test Coverage Gaps |
| Fix a known bug | [GitHub Issues](https://github.com/jjstwerff/loft/issues) (`gh issue list --label "wa:none"`) → [TESTING.md](doc/claude/TESTING.md); close with `Fixes #NNN` |
| Retest caveats before release | [CAVEATS.md](doc/claude/CAVEATS.md) |
| Add or fix native code generation | [NATIVE.md](doc/claude/NATIVE.md) → [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) → [INTERNALS.md](doc/claude/INTERNALS.md) § Native |
| Understand slot assignment / stack layout | [SLOTS.md](doc/claude/SLOTS.md) |
| Implement a planned language feature | [ROADMAP.md](doc/claude/ROADMAP.md) → [PLANNING.md](doc/claude/PLANNING.md) → feature design doc (TUPLES.md / COROUTINE.md / STACKTRACE.md) |
| Add HTTP or JSON support | [PLANNING.md](doc/claude/PLANNING.md) § H-tier → [lib_plans/future/06-web-services/](doc/claude/lib_plans/future/06-web-services/) → [STDLIB.md](doc/claude/STDLIB.md) |
| Implement `loft install <name>` registry | [PKG_REGISTRY.md](doc/claude/PKG_REGISTRY.md) → [PACKAGES.md § Open work](doc/claude/PACKAGES.md#open-work) → [PACKAGES.md](doc/claude/PACKAGES.md) |
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

Every opcode that produces or consumes a `DbRef` shows an inline struct/vector dump in the trace:
`#3.1 { name: "x", inner: #2.1 { val: 42 } }`. Tune with:

| Env var | Default | Effect |
|---|---|---|
| `LOFT_DUMP_DEPTH` | `2` | Max nesting depth before `{...}` / `[N items...]` |
| `LOFT_DUMP_ELEMENTS` | `8` | Max vector elements before `...N more` |

Also works with `cargo run --bin loft` when `LOFT_LOG` is set (writes to stderr). See
[DEBUG.md](doc/claude/DEBUG.md) § Database / Struct Debug Dumps.
