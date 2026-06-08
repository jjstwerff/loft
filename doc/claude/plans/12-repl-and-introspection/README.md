<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN12 — REPL + interpreter-introspection tool  ·  [loft-lang/plans#12](https://github.com/loft-lang/plans/issues/12)  ·  *(was `@PLAN08`)*

**Status:** SHIPPED (first cut) — phases 00–06 landed 2026-06-07/08 on the
`repl` branch.  `loft repl` (and a bare `loft`) is an interactive prompt;
`loft introspect` dumps a program's intermediate forms.  Follow-ups (stack-
resident execution, non-integer scope, `:type`/`:vars`, line history) are
deferred — see [§ Deferred follow-ups](#deferred-follow-ups).

## Goal

Two related developer tools, sharing a planning doc because they
overlap heavily in the underlying machinery:

1. **Introspection tool** — a clean CLI surface that dumps any
   loft program's intermediate forms in a human-readable shape:
   - **Bytecode** disassembly per function.
   - **Generated Rust** source (the `--native-emit` shape).
   - **Variable slot tables** per function (name, type, slot
     offset + size, scope, live interval).

   Today these are reachable via `LOFT_LOG=…` env variants writing
   to `tests/dumps/*.txt`, scattered `--dump` / `--native-emit`
   flags, and ad-hoc setup.  The introspection tool gives users
   one CLI command that produces all three side-by-side without
   touching env vars or test harnesses.

2. **REPL** — an interactive `loft>` prompt where users type
   expressions / statements one at a time, each is parsed +
   compiled + executed, and **state persists across inputs**:
   variables stay bound, struct/enum/fn definitions remain in
   scope, and the next input can use them.  Special `:cmd`
   commands expose the introspection tool's outputs for the
   currently-defined functions.

## Why these together

The REPL needs the same machinery the introspection tool exposes:
both inspect the same intermediate forms (bytecode, generated
Rust, slot tables).  Building the introspection CLI first gives
the REPL its inspection commands (`:bytecode`, `:rust`, `:slots`)
for free — the REPL's `:cmd` handler dispatches to the same
underlying functions.

The introspection tool also ships value standalone (XS effort,
ships in days), so users get something concrete while the REPL's
deeper machinery (incremental parser, monolithic-bytecode
relaxation, State reset API) takes weeks.

## Architectural blockers (REPL only)

The introspection tool is ready to build today.  The REPL needs
three pieces of new infrastructure (each is its own phase):

| Blocker | What's missing today | Phase |
|---------|---------------------|-------|
| Parser is file-level only | `Parser::parse_file()` and `parse(filename)` work over a whole file; no `parse_statement(&str)` entry exists.  Two-pass parsing assumes the full source is available before pass 2 begins. | 02 |
| Bytecode is monolithic | `state.bytecode: Arc<Vec<u8>>` is built once via `compile::byte_code()`; no append / resume API.  `code_pos` is a position within that single block. | 03 |
| State has no reset API | Between REPL inputs, `stack_pos`, `code_pos`, `call_stack` need clearing without losing `database` (the user's defined values).  No `State::reset_for_repl()` exists. | 03 |

The introspection tool (phase 01) does NOT need any of these — it
operates on a fully-loaded program just like `--dump` does.

## Phases

| # | File | Status | Effort | Summary |
|---|------|--------|--------|---------|
| 0 | [00-baseline.md](00-baseline.md) | done | XS | Survey of existing dump APIs (`LOFT_LOG`, `--dump`, `--native-emit`, `dump_bytecode`, `dump_variables`).  Confirmed the introspection tool is a packaging job; the REPL needs three new pieces of architecture. |
| 1 | [01-introspection-cli.md](01-introspection-cli.md) | shipped | S | `loft introspect <file>` / `--introspect`: emits bytecode + generated Rust + slot tables + per-fn types to stdout (or per-flag files).  Wraps `state.dump_bytecode`, `dump_variables`, `Output::output_native`.  Sub-flags select one dimension or filter to a function.  (2026-06-07: bare `introspect` subcommand added; default-stdlib filter fixed for absolute paths + synthesized internals so all sections show user code only; `tests/introspect.rs` regression guard.) |
| 2 | [02-statement-parser.md](02-statement-parser.md) | shipped | M | `Parser::statement_incomplete` (read-more detector) + `parse_statement` → `Ready`/`NeedMore`/`Error`: top-level defs parse in place, bare expressions wrap in a synthetic fn, parse errors roll `data` back (`Data::rollback_to`). |
| 3 | [03-state-reset-and-append.md](03-state-reset-and-append.md) | shipped | S | The `Arc<Vec<u8>>`→`Vec<u8>` refactor proved unnecessary — `compile::byte_code_from` already appends. `ReplSession` (`src/repl.rs`) gives integer-variable persistence; error recovery fixed (fresh lexer per `parse_str`). See the doc's Revised design. Stack-resident model (no re-run) + non-integer scope remain. |
| 4 | [04-repl-shell.md](04-repl-shell.md) | shipped | M | Interactive `loft>` prompt (`loft repl`, or a bare `loft`): result echo in loft's native form, multi-line input, parse-error + runtime-panic recovery, `:quit`/`:help`/`:reset`. `rustyline`/history deferred. |
| 5 | [05-repl-introspection.md](05-repl-introspection.md) | shipped | S | `:bytecode`/`:rust`/`:slots`/`:fns` wired to phase-01 introspection. `:type <expr>` + value-bearing `:vars` deferred (need a type-only parse / result capture). |
| 6 | [06-cleanup-and-doc.md](06-cleanup-and-doc.md) | shipped | XS | User docs: [REPL.md](../../REPL.md), CHANGELOG, CLAUDE.md key-commands + doc index. Deferred follow-ups recorded below. |

## Deferred follow-ups

Recorded here (not filed as GitHub issues — these are future enhancements, not
`main` regressions):

| ID | Description |
|----|-------------|
| **REPL.X** | Stack-resident execution — run each new line *once* over a preserved frame instead of re-running the accumulated bindings, so side-effecting statements don't repeat.  Removes the integer-only/pure-computation limit (see 03 doc's Revised design). |
| **REPL.T** | `:type <expr>` + value-bearing `:vars` — both need a way to read a value back out of execution (a result-capture API); the same gap blocks in-process result-as-`String` return. |
| **REPL.E** | Line editor + history — `rustyline` (or hand-rolled) for arrow-key recall; currently input is read plain. |
| **REPL.W** | WASM browser playground — the parse + execute machinery already runs under wasm32; the playground is a separate web UI. |
| **REPL.S** | Save / restore session to a `.loftrc`-style file (`:save` / `:load`). |
| **REPL.C** | Tab completion of fn names, locals, struct fields. |
| **INSP.J** | JSON output mode for `loft introspect` (machine-readable, for IDE integration). |

## Total effort

Single-developer estimate: **2–3 weeks** for the full sequence.  Phase
1 alone ships in **1 day** as a standalone tool — users get
introspection value before the REPL machinery starts.

Phases 2–5 are sequential (each depends on the prior).  Phase 6 is
documentation cleanup.

## Ordering rationale

**01 first.**  Standalone value, no architectural prerequisites.  The
existing dump APIs are battle-tested (every `LOFT_LOG` test case
exercises them); we're packaging them into a clean CLI surface.
Ships before REPL work begins.

**02–04 sequentially** (parser → state → shell).  Each is a hard
prerequisite for the next.  Reordering doesn't make sense: the shell
can't run without state-reset, state-reset can't be designed without
knowing how the parser feeds new statements, etc.

**05 after 04.**  REPL shell first, then layer the introspection
commands on top.

**06 last.**  Documentation and any deferred items (browser
playground, IDE integration).

## Out of scope

- **IDE integration** — language server protocol, diagnostic
  pushing, completion.  Tracked separately in `doc/claude/lib_plans/future/09-lsp/README.md`.
- **Debugger / breakpoints** — DAP-style stepping.  Tracked in
  `doc/claude/lib_plans/future/09-lsp/README.md` (LSP.3 / NDB.0).
- **Hot-reload** of `.loft` source files — the REPL reads from
  stdin only.  Editor-side hot-reload is an LSP feature, not a
  REPL feature.
- **Multi-user / networked REPL** — the prompt is single-user
  local-process only.  Browser playground (deferred to phase 6) is
  the sandboxed multi-user analogue.
- **Save / restore session** — REPL state is process-local; no
  persistence across launches in this plan.

## Cross-references

- `src/log_config.rs` — `LOFT_LOG` env-var presets; basis for
  introspection's bytecode-dump filtering.
- `src/state/debug.rs::dump_code` — the disassembler the
  introspection tool calls.
- `src/variables/validate.rs::dump_variables` — the slot-table
  dumper.
- `src/generation/Output::output_native` — generated Rust emitter.
- `src/parser/mod.rs::parse` — file-level parser, the entry that
  phase 02 splits up.
- `src/state/mod.rs::execute_argv` — runtime entry, the function
  phase 03 makes resumable.
- `doc/claude/TESTING.md` § LogConfig — user-facing reference for
  the LOFT_LOG presets that introspection's CLI sub-flags mirror.
- `doc/claude/DEBUG.md` — debug-output framework that the
  introspection tool re-uses.

## Ground rules

Inherits from [doc/claude/plans/README.md](../README.md):

- Every phase preserves all currently-green tests.
- No regression-now-fix-later trades.
- Each phase ships its own `make ci` run + at least one new test.

Specific to @PLN12:

1. **Introspection tool's output is byte-stable** across loft
   versions for the same input.  Tests pin the exact text shape
   so users can grep / diff outputs across releases.
2. **REPL never crashes on user input.**  Parse errors, type
   errors, runtime panics — all recover to the prompt with a
   clean diagnostic.  Crash-only behaviour is for fatal compiler
   bugs (and even those flush a useful trace before exiting).
3. **REPL output is reproducible.**  Same input sequence →
   same output (modulo time-dependent natives like `now()` which
   are flagged in their own way).  No iteration-order non-determinism.
4. **Introspection output stays valid loft IR.**  Bytecode
   disassembly + slot tables describe the EXACT shape the runtime
   sees; the generated Rust is the EXACT shape `cargo build`
   would compile.  No "approximate" or "for-display-only" forms.
