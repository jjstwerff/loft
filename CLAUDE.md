
# Claude Code Instructions for the Loft Project

## What loft is

**loft** is a tree-walking interpreter for the **loft** programming language, written in Rust.
Loft is a statically typed, expression-oriented language with struct/enum support, a
store-based heap, and a standard library loaded from `default/*.loft`.

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
```

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
default/02_images.loft  — Image, Pixel, File, Format types
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
   ONLY a longer plan (multi-week, well-defined arc with its own
   design doc — e.g. `plan-06-arc`, `lsp-server`) earns a specific
   branch name.  Do not open a second branch unless the user
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

**File pre-existing bugs encountered during a bug hunt before moving on to any
other bug or feature.**

While diagnosing or fixing a bug you will often surface *other* bugs:

- Sibling shapes ("the original P-issue was `out + s`; my variant probes show
  `s = s + s` and `s = "lit" + s` are also broken differently").
- Latent issues flagged in code comments that never made it to PROBLEMS.md
  (for example, a `// loft text fields initialised to "" read back as null`
  comment in a working example).
- Symptoms surfaced during diagnosis but unrelated to the active fix
  ("native E0502 in this unrelated borrow path").

These findings are the **cheapest bugs you will ever file** — the relevant
code paths are loaded into your head, the diagnostic infrastructure is warmed
up, and a working reproducer is within reach.  Moving on without filing
means re-discovering each one from scratch in a future session.

**Required action before picking up the next bug or feature:**

1. Add a P-issue row to [PROBLEMS.md](doc/claude/PROBLEMS.md) with a minimal
   reproducer (path, expected output, observed output on each backend),
   severity tier, and the workaround if any.
2. If user-visible, mirror the row in
   [USER_FACING.md](doc/claude/USER_FACING.md).
3. If the bug is small enough to test cheaply, save the reproducer to
   `/tmp/p_followups/` (so re-validation later is one command) or, when the
   shape deserves CI lock-in, add a regression test to `tests/scripts/`.

The rule applies even when the bug looks obvious, narrow, or "clearly
unrelated."  One row in PROBLEMS.md costs ~30 seconds.  The cost of
re-discovering the bug six months later — relearning the surrounding
code, rebuilding a reproducer, re-running the diagnostic — is two orders
of magnitude higher.

This rule is **not** a license to scope-creep the active fix.  Continue to
ship the original-report fix as a focused change.  File the follow-ups as
*new* P-issue rows; do not bundle them into the same patch unless they share
a single fix site.

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
| [WASM.md](doc/claude/WASM.md) | WASM architecture — wasm32-wasip2 target, VirtFS, host bridges, feature gates, FS bridge steps |
| [LOGGER.md](doc/claude/LOGGER.md) | Runtime logging framework (log_info/warn/error/fatal, config, rate limiting, production mode) |
| [TESTING.md](doc/claude/TESTING.md) | Test framework, `LogConfig` debug-logging presets, `LOFT_LOG` env var, suite files |
| [DOC.md](doc/claude/DOC.md) | HTML documentation generation (gendoc.rs + documentation.rs) |
| [DESIGN.md](doc/claude/DESIGN.md) | Algorithm catalog with complexity analysis and enhancement priorities |
| [CODE.md](doc/claude/CODE.md) | Code quality rules (naming, functions, doc comments, clippy, dependency policy) |
| [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) | Development workflow — branching, WIP commit, rebase sequence, CI |
| [SLOTS.md](doc/claude/SLOTS.md) | Stack slot assignment — two-zone design, diagnostic tools, open issues |
| [PROBLEMS.md](doc/claude/PROBLEMS.md) | Known bugs, limitations, workarounds, and fix plans |
| [QUALITY.md](doc/claude/QUALITY.md) | Open programmer-biting issues, active sprint (P54), active design (C54), compiler blockers, enhancement tiers |
| [DESIGN_DECISIONS.md](doc/claude/DESIGN_DECISIONS.md) | Closed-by-decision register — check before proposing features already declined (C3 / C38 / C54.D / …) |
| [FORMATTER.md](doc/claude/FORMATTER.md) | Source formatter design and implementation notes |
| [INCONSISTENCIES.md](doc/claude/INCONSISTENCIES.md) | Known language design inconsistencies and asymmetries |
| [PERFORMANCE.md](doc/claude/PERFORMANCE.md) | Benchmarks, optimisation plans, string alloc, const data, block copy analysis |
| [PLANNING.md](doc/claude/PLANNING.md) | Priority-ordered enhancement backlog |
| [ROADMAP.md](doc/claude/ROADMAP.md) | Items in implementation order, grouped by milestone (0.9.0 / 1.0.0 / 1.1+) |
| [plans/README.md](doc/claude/plans/README.md) | Multi-phase **core-language** initiatives (current / future / deferred / finished) — compiler, runtime, validation matrices, codegen arcs, language features.  Max 2-3 active plans. |
| [lib_plans/README.md](doc/claude/lib_plans/README.md) | Multi-phase **library** initiatives (current / future / deferred / finished) — `server`, `game_client`, graphics, regex, package format, asset pipeline, web examples, IDE.  Same `≤3 active` discipline as `plans/`; numbering independent. |
| [BROADENING.md](doc/claude/BROADENING.md) | Strategic evaluation — using loft beyond games (CLI, server, data), sequenced unlocks |
| [lib_plans/future/03-lazy-stdlib/README.md](doc/claude/lib_plans/future/03-lazy-stdlib/README.md) | Future library plan — conditional stdlib loading: trigger-based module load, pay-for-what-you-use cold start.  Critical-path infrastructure: REGEX (lib_plans 01) is the first scheduled consumer once this lands. |
| [plans/future/26-match-peg/README.md](doc/claude/plans/future/26-match-peg/README.md) | Future plan — L3 PEG-style match patterns: sequence / alternation / optional / repetition / multi-variable capture, anchor-revert backtracking modelled on `Lexer::link()` / `revert()`.  Cooperates with the regex library (lib_plans/01-regex): regex handles all text matching, MATCH_PEG handles structural / numeric / Unicode-class patterns.  Base match syntax lives in LOFT.md § Match expressions. |
| [lib_plans/future/01-regex/README.md](doc/claude/lib_plans/future/01-regex/README.md) | Future library plan — regex standalone library: replaces the `r"..."` literal / "regex arm in match" plan with a full-featured library.  First lazy-loaded stdlib consumer. |
| [TUPLES.md](doc/claude/TUPLES.md) | Tuple design — multi-value returns, deconstruction, match destructuring |
| [plans/future/30-sorted-slice/README.md](doc/claude/plans/future/30-sorted-slice/README.md) | Future plan — A8: slicing, open-ended ranges, partial-key match, comprehensions on `sorted` / `index` collections.  Current state table shows mostly ✗ marks; runtime already supports partial-key compare via `key_compare` zip-prefix semantics, only parser changes needed. |
| [STACKTRACE.md](doc/claude/STACKTRACE.md) | Stack trace introspection — `stack_trace()` API, `StackFrame`, `ArgValue` |
| [NATIVE.md](doc/claude/NATIVE.md) | Native code generation (`src/generation/`), `--native` default plan, fix plans |
| [PACKAGES.md](doc/claude/PACKAGES.md) | Package format, registry, governance, external libs, library extraction |
| [plans/deferred/28-const-store/README.md](doc/claude/plans/deferred/28-const-store/README.md) | Deferred plan — constant store.  **Phase A** (P127 fix: heap-backed const store + `OpConstRef` opcode + long-string migration) and **Phase D** (`.loftc` bytecode cache) are SHIPPED.  **Phase B** (mmap) deferred — trigger: Phase C lands a large embedded stdlib cache.  **Phase C** (WASM pre-compiled stdlib) deferred — large effort (`Data` struct serialization across 130+ public members).  Most of the work has shipped; the deferred-tail phases stay parked until their triggers fire. |
| [DEBUG.md](doc/claude/DEBUG.md) | Debugging utilities and tools |
| [LSP.md](doc/claude/LSP.md) | Language server (LSP.1/2) + DAP debugger (LSP.3) + Eclipse / JetBrains / Neovim plugin design |
| [plans/future/25-native-debug/README.md](doc/claude/plans/future/25-native-debug/README.md) | Future plan — GDB / LLDB integration for `--native` builds: DWARF, source maps, plugins.  Three independently-shippable phases NDB.0 / NDB.1 / NDB.2; NDB.0 (`--native-debug` flag) is the smallest first step. |
| [RELEASE.md](doc/claude/RELEASE.md) | Release checklist and version history |
| [WEB_IDE.md](doc/claude/WEB_IDE.md) | Web IDE integration design notes |
| [CHANGELOG.md](CHANGELOG.md) | User-facing release notes (shipped in release archives) |
| [CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md) | Full technical changelog — opcode/slot/phase detail for contributors |
| [CAVEATS.md](doc/claude/CAVEATS.md) | Verifiable edge cases and limitations with reproducers and test references |
| [COROUTINE.md](doc/claude/COROUTINE.md) | Coroutine design — stackful `yield`, `iterator<T>`, `yield from` (planned, 1.1+) |
| [LIFETIME.md](doc/claude/LIFETIME.md) | Dependency tracking and scope-based freeing — dep field semantics, Text vs Reference, closures |
| [WEB_SERVICES.md](doc/claude/WEB_SERVICES.md) | Web services design evaluation — HTTP/JSON approach comparison, issues #54/#55 |
| [WEB_SERVER_LIB.md](doc/claude/WEB_SERVER_LIB.md) | `server` library design — HTTP server, WebSockets, TLS, ACME, auth, RBAC, game server additions |
| [plans/future/23-event-loop/README.md](doc/claude/plans/future/23-event-loop/README.md) | Future plan — prioritised event-loop abstraction (client + server): bidirectional handlers, library-assigned ids, separate tuning phase, library-assembled streaming, JSON-by-default wire format, depends on P213 v4.  Companions: [DISCUSSION.md](doc/claude/plans/future/23-event-loop/DISCUSSION.md) (open issues, alternatives, design history) and [PROTOCOL.md](doc/claude/plans/future/23-event-loop/PROTOCOL.md) (wire-format spec — text-mode `<id>:payload` v1 shipped, binary-mode 12-byte header v2 designed, server-arbited MAP handshake, encoding modes, streaming reassembly). |
| [plans/future/22-mutable-closures/README.md](doc/claude/plans/future/22-mutable-closures/README.md) | Future plan — novice-fit closure capture: four-case classification (A read-only, B co-scoped, C moved, D aliased rejected), implicit-by-body, Reference + cell lowerings, diagnostic shape.  Companion [DISCUSSION.md](doc/claude/plans/future/22-mutable-closures/DISCUSSION.md): alternatives surveyed A-F, implementation analysis sketch, open questions, design history. |
| [TIC_TAC_TOE.md](doc/claude/TIC_TAC_TOE.md) | Protocol-validation vehicle: v1 shipped (server-arbited handshake, integer-id wire format).  v2/v3/v4 are protocol-only ground layers (multi-client, asset-serving + browser, server-side compile + hot-swap) — all verified text-mode.  Visual / playable tic-tac-toe is deferred indefinitely; real-game UX lives in MULTIPLAYER_EDITOR |
| [plans/future/24-multiplayer-editor/README.md](doc/claude/plans/future/24-multiplayer-editor/README.md) | Future plan — first real-game milestone: multi-client hex editor in the moros stack.  Paint hexes red on click, propagate via WebSocket to all connected clients, snapshot replay on connect.  Consumes TIC_TAC_TOE v2 ground layer (multi-client server primitives). |
| [GAME_CLIENT_LIB.md](doc/claude/GAME_CLIENT_LIB.md) | `game_client` library design — WebSocket client, multiplayer protocol, prediction, WASM script loading |
| [plans/future/29-server-features/README.md](doc/claude/plans/future/29-server-features/README.md) | Future plan — language features for server / game-client library ergonomics: C55 type aliases, C56 `?? return` null-coalesce-with-early-return, A15 `parallel { }` structured concurrency, I13 iterator protocol (`for msg in ws` via `fn next`), C57 route decorator syntax (`@get` / `@post` / `@ws`).  Prerequisites for the upcoming server / game-client library work. |
| [HTML_EXPORT.md](doc/claude/HTML_EXPORT.md) | W1.1 single-file HTML export — native WASM compilation for browser |
| [lib_plans/future/02-graphics/README.md](doc/claude/lib_plans/future/02-graphics/README.md) | Future library plan — graphics: 2D RGBA drawing + OpenGL/WebGL/GLB 3D rendering library design.  Companions: [IMPLEMENTATION.md](doc/claude/lib_plans/future/02-graphics/IMPLEMENTATION.md) (step-by-step ordered checklist: canvas → GLB → OpenGL → WebGL), [RENDERER.md](doc/claude/lib_plans/future/02-graphics/RENDERER.md) (high-level renderer — scene-driven PBR with shadows, helper abstractions), [GALLERY.md](doc/claude/lib_plans/future/02-graphics/GALLERY.md) (web example gallery + unified rendering across native OpenGL / WebGL / GLB). |
| [lib_plans/future/05-game-infra/README.md](doc/claude/lib_plans/future/05-game-infra/README.md) | Future library plan — game infrastructure grab-bag: G1-G7 (sprites, tilemap, collision, audio, demo game), GL6.6 (keyboard/mouse input via DOM), W1.1 (single-file HTML export), FFI.1-FFI.4 (generic type marshaller, cdylib loader, glue elimination, native-fn guide), W-warn (developer warnings — Clippy-inspired). |
| [lib_plans/future/04-asset-pipeline/README.md](doc/claude/lib_plans/future/04-asset-pipeline/README.md) | Future library plan — game asset pipeline: AI prototype → artist polish → integration.  Three phases: procedural placeholder sprites/sounds → external-tool authoring (Aseprite, etc.) → integration via `load_sprite_sheet()` etc. |
| [../PROMPTS.md](doc/PROMPTS.md) | Working with Claude — practices and when to use each prompt in `prompts.txt` |

---

## Reading by goal

| Goal | Start here |
|---|---|
| Understand the language syntax | [LOFT.md](doc/claude/LOFT.md), then [STDLIB.md](doc/claude/STDLIB.md) |
| Add a feature to the compiler | [COMPILER.md](doc/claude/COMPILER.md) → [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) → [INTERNALS.md](doc/claude/INTERNALS.md) |
| Debug a runtime crash | [PROBLEMS.md](doc/claude/PROBLEMS.md) (check open issues) → [TESTING.md](doc/claude/TESTING.md) § LogConfig → [INTERNALS.md](doc/claude/INTERNALS.md) |
| Add a native (Rust) standard library function | [INTERNALS.md](doc/claude/INTERNALS.md) § Native Function Registry, then `default/01_code.loft` |
| Plan or review enhancements | [PLANNING.md](doc/claude/PLANNING.md), then [PERFORMANCE.md](doc/claude/PERFORMANCE.md) |
| Improve interpreter or native performance | [PERFORMANCE.md](doc/claude/PERFORMANCE.md) — benchmarks, root-cause analysis, optimisation designs |
| Implement a PLANNING.md item | [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) — branching, commit order, CI |
| Understand the parallel execution model | [THREADING.md](doc/claude/THREADING.md), then [INTERNALS.md](doc/claude/INTERNALS.md) § Parallel Execution |
| Set up logging in a loft program | [STDLIB.md](doc/claude/STDLIB.md) § Logging, then [LOGGER.md](doc/claude/LOGGER.md) |
| Understand the heap / memory model | [DATABASE.md](doc/claude/DATABASE.md), then [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) § DbRef |
| Improve the test suite | [TESTING.md](doc/claude/TESTING.md), then `tests/scripts/` and `tests/docs/` |
| Find test coverage gaps | [TESTING.md](doc/claude/TESTING.md) § Test Coverage Gaps |
| Fix a known bug | [PROBLEMS.md](doc/claude/PROBLEMS.md) (fix path) → [TESTING.md](doc/claude/TESTING.md) |
| Retest caveats before release | [CAVEATS.md](doc/claude/CAVEATS.md) — each entry has a reproducer and test reference |
| Add or fix native code generation | [NATIVE.md](doc/claude/NATIVE.md) → [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) → [INTERNALS.md](doc/claude/INTERNALS.md) § Native |
| Understand slot assignment / stack layout | [SLOTS.md](doc/claude/SLOTS.md) |
| Implement a planned language feature (Tuples/Coroutines/etc.) | [ROADMAP.md](doc/claude/ROADMAP.md) → [PLANNING.md](doc/claude/PLANNING.md) → feature design doc (TUPLES.md / COROUTINE.md / STACKTRACE.md) |
| Add HTTP or JSON support | [PLANNING.md](doc/claude/PLANNING.md) § H-tier → [WEB_SERVICES.md](doc/claude/WEB_SERVICES.md) → [STDLIB.md](doc/claude/STDLIB.md) |
| Implement `loft install <name>` registry | [PACKAGES.md](doc/claude/PACKAGES.md) |
| Build or understand the `server` library | [WEB_SERVER_LIB.md](doc/claude/WEB_SERVER_LIB.md) |
| Build or understand the `game_client` library | [GAME_CLIENT_LIB.md](doc/claude/GAME_CLIENT_LIB.md) |
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
