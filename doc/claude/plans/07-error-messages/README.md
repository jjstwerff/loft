<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 07 — Better error messages

## Goal

Make every loft error reach the user as **`file:line:col — concrete
message — source line with caret — optional suggestion`**, whether it
originates in the parser, the type system, the bytecode runtime, or a
native-codegen crash.

The bar is: a user can read the message and know which token in their
own source file is wrong, what was expected, and (when possible) what
to try instead — without opening a dump file or re-running with
`LOFT_LOG`.

## Why

Today's diagnostic surface is uneven:

- **Parser errors** carry `Position { file, line, pos }` and render as
  `Error: Expect token > at foo.loft:42:17` (compact, single line, no
  source context).  See `src/lexer.rs:443::diagnostic` and
  `src/diagnostics.rs:DiagEntry::to_string_compact`.
- **Type errors raised during the second pass** (e.g.
  `src/parser/expressions.rs:215`) use the same channel, but the
  message often names a variable without saying *which type was
  expected vs. found* — the user has to grep the source for
  `tp(v_nr).is_unknown()` to find out.
- **IR-walk errors** (scope analysis, slot assignment) sometimes fall
  through to `Level::Fatal` strings without a `Position` attached
  (`add` instead of `add_at` in `src/diagnostics.rs:82`).
- **Runtime errors** are the worst case: a divide-by-zero, null
  dereference, or out-of-bounds index reaches `fill.rs` and either
  panics with no source context (`fill.rs:1677` is the only intentional
  one — every other panic is an interpreter bug or a user bug
  indistinguishable from one) or returns a sentinel that produces a
  silent wrong answer downstream.  `src/crash_report.rs` recovers the
  bytecode `pc` + op-name on SIGSEGV but **not** the originating
  loft source line — that mapping does not exist yet.
- **`Value` IR nodes carry `Value::Line(u32)` markers but not
  full positions.**  `src/data.rs:291` stores line only; column and
  file are inferred from the surrounding `Definition.position`
  (`src/data.rs:1376`).  An expression mid-block has no per-node
  span, so when a runtime opcode fails the interpreter cannot point
  at the exact sub-expression.

The cumulative effect: when I (or a user) write a loft program and
something breaks, the failure is "panic at line 1677 of fill.rs"
rather than "tried to divide by zero at game.loft:88:14 in
`damage / armour`".  Debugging then requires reading
`tests/dumps/*.txt` and reasoning backwards from the bytecode trace.

## Architectural anchor — "every error knows where it came from"

Three things must travel together to every error site:

1. **A source `Position`** (file + line + column) — the parser already
   produces these for tokens; we need to keep them attached to the IR
   nodes the parser builds and to the bytecode positions the codegen
   emits.
2. **A typed kind** (`ParseError`, `TypeError`, `RuntimeError {
   DivByZero | OutOfBounds | NullDeref | TypeMismatch | … }`) so that
   the renderer can group, suggest, and (for runtime errors) decide
   whether to abort or continue.
3. **A renderer** that reads the source file, prints the offending
   line, draws a caret under the column, and optionally appends a
   `did you mean …?` line driven by `suggest_similar` (already in
   `src/diagnostics.rs:181`).

Spans on Value IR + a pc→span side table for bytecode is the
load-bearing change.  Everything else is rendering polish on top.

## Phases

Each phase preserves every currently-green test.  Each phase ships its
own message-fixture corpus under `tests/error_messages/` (new) so that
*the rendered output itself* is a regression target — not just the
fact that the error fires.

| Phase | File | Status | Effort | Summary |
|---|---|---|---|---|
| 0 | [00-survey-and-baseline.md](00-survey-and-baseline.md) | open | S | Catalogue every error site (`grep -n 'diagnostic!\|specific!\|panic!\|unreachable!\|Level::Error\|Level::Fatal'` across `src/`).  Build a "bad program" corpus (~40 short `.loft` files, one per failure mode).  Snapshot today's output for each into `tests/error_messages/baseline/`.  No code change — this is the floor that every later phase must improve. |
| 1 | [01-spans-on-ir.md](01-spans-on-ir.md) | open | M | Attach a `Position` to every `Value` IR node that can fail at runtime: `Call`, `CallRef`, `Set`, `Iter`, `Return`, division/index/field operators, struct construction.  Today only `Value::Line(u32)` exists (line only, no col, no file when crossing files).  Either widen `Value::Line` to `Value::Span(Position, Box<Value>)` or add a side-table `HashMap<*const Value, Position>` keyed by IR address.  Decision in 01.A; rest of the phase is mechanical. |
| 2 | [02-renderer.md](02-renderer.md) | open | S | New `Diagnostics::render_pretty(&self, loader: &dyn SourceLoader)` that prints `error: <msg> --> file:line:col` followed by the source line + caret + optional `note:` lines.  Wire it into `src/main.rs` as the default user-facing path; keep `to_string_compact` as the test harness path.  `FileSourceLoader` caches file contents read once at parse time. |
| 3 | [03-bytecode-pc-to-source-map.md](03-bytecode-pc-to-source-map.md) | open | M | At codegen time emit a `SourceSpanTable { pcs, spans }` per function, stored alongside the bytecode in `Definition`.  Lookup is binary search on `pc` plus a last-hit cache.  `src/crash_report.rs::set_context` already publishes the current `pc` per thread; extend the published `Ctx` with a pre-formatted source-loc string and the SIGSEGV / SIGABRT printer surfaces it.  `LOFT_LOG=crash_tail:N` gains a leading `at file:line:col` line. |
| 4 | [04-runtime-error-kinds.md](04-runtime-error-kinds.md) | open | M | Replace the implicit "panic = bug, sentinel = user error" coin-flip with an explicit `RuntimeError { kind, position, op_pc, backtrace }` raised at a small site list: divide / modulo by zero, vector / text out-of-bounds, null-DbRef deref, narrowing-cast overflow, the `panic("…")` builtin, stack-overflow trap.  Every other panic stays a panic — that's an interpreter bug.  Phase 4 is the boundary: user-attributable faults become typed `RuntimeError`; everything else stays a hard panic with the phase-3 source-line printer attached. |
| 5 | [05-suggestions.md](05-suggestions.md) | open | S | Audit every `name not found` / `unknown method` / `unknown field` / `unknown type` site (parser + type-check) and wire `suggest_similar` (`src/diagnostics.rs:181`) with the right candidate set: in-scope variables, visible global fns, struct fields, methods of `T`, enum variants, type names.  Cap distance at min(2, 25 % of name length).  New `Level::Note` entries render as `= note: did you mean …?` under the parent error.  `LOFT_NO_SUGGESTIONS=1` for the harness. |
| 6 | [06-type-mismatch-detail.md](06-type-mismatch-detail.md) | open | M | Rewrite type-mismatch messages to name both sides and the operation: `cannot assign vector<i32> to variable of type text` not `type mismatch`; `argument 2 of fn 'fight': expected reference<Enemy>, got integer`.  Add `Type::render_user` (loft-surface syntax) and switch errors / formatter / gendoc to it.  Pure rendering change once spans + kinds are in place; landed in 4 batches (assignment + return / calls + structs / operators + iterators / match + format). |
| 7 | [07-cleanup-and-doc.md](07-cleanup-and-doc.md) | open | XS | New `doc/claude/COMPILER.md` § Diagnostics; retire resolved entries from `doc/claude/PROBLEMS.md` / `CAVEATS.md`; update `LOFT_LOG` quick reference in `CLAUDE.md`; user-facing `CHANGELOG.md` entry; per-phase `CHANGELOG_TECHNICAL.md` entries; dead-helper sweep; 5-case end-to-end smoke test. |

## Ground rules

Inherits the global plans rule from
[doc/claude/plans/README.md](../README.md):

> A plan's job is to split work into manageable chunks that each land
> cleanly without introducing new problems.

Specific to this plan:

1. **No degraded message ships.**  Every phase is a strict improvement
   over the phase-0 baseline.  If a fixture regresses (loses
   information, loses a span, gets longer-without-being-clearer), the
   phase pauses.
2. **The compact format stays available.**  `to_string_compact`
   remains the test-harness rendering — pretty rendering is opt-in via
   `--pretty-errors` (default on for `cargo run --bin loft`, default
   off for `cargo test`).  This keeps `tests/issues.rs` and friends
   stable while phase 6 churns the messages.
3. **No new error category without a fixture.**  Every new
   `RuntimeError` kind (phase 4) and every rewritten message (phase 6)
   ships with a `tests/error_messages/<name>.loft` and its
   `<name>.expect` golden output.  Goldens live under
   `tests/error_messages/` so they're easy to bulk-regen with
   `UPDATE_GOLDEN=1 cargo test error_messages` when the format
   changes intentionally.
4. **Spans on IR are additive.**  Phase 1 adds positions; it does not
   change semantics, slot assignment, codegen output, or bytecode
   layout.  If sizing `Value` enum grows the runtime hot path, store
   spans in a side table keyed by node identity instead.  Measured in
   phase 0's `make bench` baseline.
5. **Runtime panics that survive phase 4 are interpreter bugs.**
   After phase 4, any panic reaching the user means the interpreter
   itself is wrong (or a `#rust ""` annotation in the stdlib has a
   bug).  That's the contract: user code produces `RuntimeError`,
   never `panic!`.

## Risks (all addressable, none plan-blocking)

| Risk | Mitigation |
|---|---|
| `Value` enum size grows when every node carries `Position` | Side-table indexed by IR-node identity; phase 1's decision-A picks the cheaper representation after measuring on the bench corpus. |
| pc→span table inflates `.loft` startup cost | Lazy-build only when an error fires; the table is just `Vec<(u32, Position)>` per fn — measured size in phase 3. |
| Pretty rendering breaks dump comparisons | `to_string_compact` stays as the dumps' format; pretty is a separate code path gated on a flag. |
| Suggestions become noisy ("did you mean … ?" on obviously-wrong names) | Distance cap from `suggest_similar`; cap at min(2, 25 % of name length); per-site candidate sets are scoped (in-scope vars only, not all global names). |
| Source-line lookup at crash time touches the filesystem inside a signal handler | `crash_report.rs` is async-signal-safe; phase 3's enrichment lives in the *normal* panic path, not the signal handler.  The signal handler keeps printing `pc + op_name` only.  Pretty source-line resolution happens when we have a normal panic (still inside Rust's panic hook, where allocation is fine). |

## Out of scope

These are not addressed by plan-07 even though they're tempting:

- **A REPL.**  Tracked separately under future ROADMAP work; pretty
  errors do not require a REPL and a REPL does not require pretty
  errors.
- **Stack-trace introspection from loft code.**  That's
  [STACKTRACE.md](../../STACKTRACE.md) — different design surface
  (`stack_trace()` API for user code).  Plan-07 only improves what
  the *interpreter itself* prints when something goes wrong.
- **Compile-time exhaustive type checking.**  Plan-07 improves the
  messages of the existing type checker; it does not add new checks.
- **IDE / LSP diagnostics.**  Tracked under
  [LSP.md](../../LSP.md).  The diagnostic types this plan introduces
  will feed the LSP later; the wire format is not in scope here.
- **Localisation.**  Messages stay English-only.

## Cross-references

- [src/diagnostics.rs](../../../../src/diagnostics.rs) — current
  `DiagEntry` / `Diagnostics` / `suggest_similar`.
- [src/lexer.rs:443](../../../../src/lexer.rs) — `Position` source +
  `diagnostic` / `pos_diagnostic` entry points.
- [src/crash_report.rs](../../../../src/crash_report.rs) — SIGSEGV /
  SIGABRT context publisher; phase 3 extends its output.
- [src/data.rs:291](../../../../src/data.rs) — `Value::Line(u32)`
  marker (replace or augment in phase 1).
- [src/fill.rs:1675](../../../../src/fill.rs) — the one intentional
  `panic!` (the `panic(text)` builtin); every other panic in
  `fill.rs` is a phase-4 candidate.
- [doc/claude/COMPILER.md](../../COMPILER.md) — diagnostic flow
  through the parser; phase 7 adds a Diagnostics section.
- [doc/claude/CAVEATS.md](../../CAVEATS.md) — entries about poor
  error messages get retired in phase 7.
- [doc/claude/plans/README.md](../README.md) — global plan ground
  rules.
