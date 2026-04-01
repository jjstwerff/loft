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
```

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
2. **Never `git push` without an explicit user instruction** — see the Remote CI section
   of [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md).
3. Create branches from the tip of `main` using the naming convention in
   [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) (e.g. `p1-1-lambda-parser`, `benchmark`).
4. Merging back to `main` is done via a GitHub pull request — not a local `git merge`.

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

## Documentation index

| File | Topic |
|---|---|
| [LOFT.md](doc/claude/LOFT.md) | Loft language reference (syntax, types, operators, control flow) |
| [STDLIB.md](doc/claude/STDLIB.md) | Standard library API (math, text, collections, file I/O, logging, parallel) |
| [COMPILER.md](doc/claude/COMPILER.md) | Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode |
| [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) | Value/Type enums in detail; 233 bytecode operators; State layout |
| [DATABASE.md](doc/claude/DATABASE.md) | Store allocator, Stores schema, DbRef, vector/tree/hash/radix implementations |
| [INTERNALS.md](doc/claude/INTERNALS.md) | calc.rs, stack.rs, create.rs, native.rs, ops.rs, png_store.rs, parallel.rs, main.rs, logger.rs |
| [THREADING.md](doc/claude/THREADING.md) | Parallel for-loop (`par(...)`), `fn <name>` references, runtime parallel execution |
| [SAFE.md](doc/claude/SAFE.md) | Thread safety analysis — parallel worker store isolation, risks, and mitigation designs |
| [LIGHT_PAR.md](doc/claude/LIGHT_PAR.md) | `par_light(...)` design — shallow-borrow stores + pre-allocated pool for non-recursive workers |
| [LOGGER.md](doc/claude/LOGGER.md) | Runtime logging framework (log_info/warn/error/fatal, config, rate limiting, production mode) |
| [TESTING.md](doc/claude/TESTING.md) | Test framework, `LogConfig` debug-logging presets, `LOFT_LOG` env var, suite files |
| [DOC.md](doc/claude/DOC.md) | HTML documentation generation (gendoc.rs + documentation.rs) |
| [DESIGN.md](doc/claude/DESIGN.md) | Algorithm catalog with complexity analysis and enhancement priorities |
| [CODE.md](doc/claude/CODE.md) | Code quality rules (naming, functions, doc comments, clippy, dependency policy) |
| [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) | Development workflow — branching, WIP commit, rebase sequence, CI |
| [SLOTS.md](doc/claude/SLOTS.md) | Stack slot assignment — two-zone design, diagnostic tools, open issues |
| [PROBLEMS.md](doc/claude/PROBLEMS.md) | Known bugs, limitations, workarounds, and fix plans |
| [FORMATTER.md](doc/claude/FORMATTER.md) | Source formatter design and implementation notes |
| [INCONSISTENCIES.md](doc/claude/INCONSISTENCIES.md) | Known language design inconsistencies and asymmetries |
| [OPTIMISATIONS.md](doc/claude/OPTIMISATIONS.md) | Planned and implemented runtime/compiler optimisations |
| [PERFORMANCE.md](doc/claude/PERFORMANCE.md) | Benchmark results, root-cause analysis vs Python and Rust, improvement plan |
| [PLANNING.md](doc/claude/PLANNING.md) | Priority-ordered enhancement backlog |
| [ROADMAP.md](doc/claude/ROADMAP.md) | Items in implementation order, grouped by milestone (0.9.0 / 1.0.0 / 1.1+) |
| [MATCH.md](doc/claude/MATCH.md) | Match expression design — pattern types, binding, phase breakdown |
| [TUPLES.md](doc/claude/TUPLES.md) | Tuple design — multi-value returns, deconstruction, stack layout |
| [TUPLE_MATCH.md](doc/claude/TUPLE_MATCH.md) | T1.9: tuple destructuring in match — element patterns, exhaustiveness, IR lowering |
| [SORTED_SLICE.md](doc/claude/SORTED_SLICE.md) | A8: slicing, open-ended ranges, partial-key match, comprehensions on sorted/index |
| [STACKTRACE.md](doc/claude/STACKTRACE.md) | Stack trace introspection — `stack_trace()` API, `StackFrame`, `ArgValue` |
| [NATIVE.md](doc/claude/NATIVE.md) | Native code generation (`src/generation/`) design and fix plans |
| [EXTERNAL_LIBS.md](doc/claude/EXTERNAL_LIBS.md) | External library loading and `loft.toml` package manifest |
| [BYTECODE_CACHE.md](doc/claude/BYTECODE_CACHE.md) | Bytecode cache (`.loftc`) design notes (deferred) |
| [DEBUG.md](doc/claude/DEBUG.md) | Debugging utilities and tools |
| [RELEASE.md](doc/claude/RELEASE.md) | Release checklist and version history |
| [WEB_IDE.md](doc/claude/WEB_IDE.md) | Web IDE integration design notes |
| [WASM_FS_STEPS.md](doc/claude/WASM_FS_STEPS.md) | WASM filesystem bridge — six separately testable implementation steps (FS-A … FS-F) |
| [CHANGELOG.md](CHANGELOG.md) | Release history |
| [QUICK_START.md](doc/claude/QUICK_START.md) | Session-start orientation — commands, file map, naming conventions, key patterns |
| [ASSIGNMENT.md](doc/claude/ASSIGNMENT.md) | Slot assignment design notes — P1/P2 proposals, resolved bugs, two-zone layout |
| [SLOTS.md](doc/claude/SLOTS.md) | Stack slot assignment — two-zone design, diagnostic tools, open issues |
| [CAVEATS.md](doc/claude/CAVEATS.md) | Verifiable edge cases and limitations with reproducers and test references |
| [SLOT_FAILURES.md](doc/claude/SLOT_FAILURES.md) | Slot assignment failure analysis — root-cause matrix, A/B/C bug categories (A6) |
| [FAILURES.md](doc/claude/FAILURES.md) | Historical test failure analysis — slot conflicts, vector append bug, fix evidence |
| [GAPS.md](doc/claude/GAPS.md) | Test coverage gaps — zero-coverage files, missing edge-case areas, priorities |
| [COROUTINE.md](doc/claude/COROUTINE.md) | Coroutine design — stackful `yield`, `iterator<T>`, `yield from` (planned, 1.1+) |
| [LIFETIME.md](doc/claude/LIFETIME.md) | Dependency tracking and scope-based freeing — dep field semantics, Text vs Reference, closures |
| [WEB_SERVICES.md](doc/claude/WEB_SERVICES.md) | Web services design evaluation — HTTP/JSON approach comparison, issues #54/#55 |
| [OPENGL.md](doc/claude/OPENGL.md) | 2D RGBA drawing library + OpenGL/WebGL/GLB 3D rendering design |
| [OPENGL_IMPL.md](doc/claude/OPENGL_IMPL.md) | Step-by-step implementation checklist: canvas → GLB → OpenGL → WebGL |
| [../PROMPTS.md](doc/PROMPTS.md) | Working with Claude — practices and when to use each prompt in `prompts.txt` |

---

## Reading by goal

| Goal | Start here |
|---|---|
| Understand the language syntax | [LOFT.md](doc/claude/LOFT.md), then [STDLIB.md](doc/claude/STDLIB.md) |
| Add a feature to the compiler | [COMPILER.md](doc/claude/COMPILER.md) → [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) → [INTERNALS.md](doc/claude/INTERNALS.md) |
| Debug a runtime crash | [PROBLEMS.md](doc/claude/PROBLEMS.md) (check open issues) → [TESTING.md](doc/claude/TESTING.md) § LogConfig → [INTERNALS.md](doc/claude/INTERNALS.md) |
| Add a native (Rust) standard library function | [INTERNALS.md](doc/claude/INTERNALS.md) § Native Function Registry, then `default/01_code.loft` |
| Plan or review enhancements | [PLANNING.md](doc/claude/PLANNING.md), then [OPTIMISATIONS.md](doc/claude/OPTIMISATIONS.md) |
| Improve interpreter or native performance | [PERFORMANCE.md](doc/claude/PERFORMANCE.md) — benchmark data, root-cause analysis, O1–O7 designs → [OPTIMISATIONS.md](doc/claude/OPTIMISATIONS.md) |
| Implement a PLANNING.md item | [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) — branching, commit order, CI |
| Understand the parallel execution model | [THREADING.md](doc/claude/THREADING.md), then [INTERNALS.md](doc/claude/INTERNALS.md) § Parallel Execution |
| Set up logging in a loft program | [STDLIB.md](doc/claude/STDLIB.md) § Logging, then [LOGGER.md](doc/claude/LOGGER.md) |
| Understand the heap / memory model | [DATABASE.md](doc/claude/DATABASE.md), then [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) § DbRef |
| Improve the test suite | [TESTING.md](doc/claude/TESTING.md), then `tests/scripts/` and `tests/docs/` |
| Find test coverage gaps | [GAPS.md](doc/claude/GAPS.md) → [TESTING.md](doc/claude/TESTING.md) |
| Fix a known bug | [PROBLEMS.md](doc/claude/PROBLEMS.md) (fix path) → [FAILURES.md](doc/claude/FAILURES.md) (test evidence) → [TESTING.md](doc/claude/TESTING.md) |
| Retest caveats before release | [CAVEATS.md](doc/claude/CAVEATS.md) — each entry has a reproducer and test reference |
| Add or fix native code generation | [NATIVE.md](doc/claude/NATIVE.md) → [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) → [INTERNALS.md](doc/claude/INTERNALS.md) § Native |
| Understand slot assignment / stack layout | [SLOTS.md](doc/claude/SLOTS.md) → [ASSIGNMENT.md](doc/claude/ASSIGNMENT.md) → [SLOT_FAILURES.md](doc/claude/SLOT_FAILURES.md) |
| Implement a planned language feature (Tuples/Coroutines/etc.) | [ROADMAP.md](doc/claude/ROADMAP.md) → [PLANNING.md](doc/claude/PLANNING.md) → feature design doc (TUPLES.md / COROUTINE.md / STACKTRACE.md) |
| Add HTTP or JSON support | [PLANNING.md](doc/claude/PLANNING.md) § H-tier → [WEB_SERVICES.md](doc/claude/WEB_SERVICES.md) → [STDLIB.md](doc/claude/STDLIB.md) |
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
