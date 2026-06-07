<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 04 — REPL shell

**Status: shipped (first cut, 2026-06-08).** `loft repl` (and a bare `loft`
with no file/subcommand, like `python`/`node`) starts an interactive `loft>`
prompt: definitions, bindings, and expressions evaluate against the live
session; a bare expression's value is echoed in loft's native rendering
(`{expr}` — `3`, `{a:1,b:2}`, `[1,2,3]`); multi-line input accumulates on
`NeedMore`; parse errors and runtime panics (caught via `catch_unwind`, isolated
to the per-eval database clone) don't crash the session; `:quit` / `:help` /
`:reset` work.  Implemented on `ReplSession` (`src/repl.rs::run_repl`); prompts +
errors go to stderr, results to stdout.  Tests: `tests/repl.rs` (subprocess) +
`tests/repl_session.rs`.

Deferred from this first cut: `rustyline` line editor + history (plain
stdin for now), in-process result-as-`String` return (results print to stdout),
the richer `:vars`/`:fns` commands, and the phase-05 introspection commands.

## Original design (below) — pre-implementation; some APIs differ from `ReplSession`

## Goal

An interactive `loft>` prompt that drives phase 02's statement
parser + phase 03's incremental execute, printing results and
errors and **never crashing** on user input.

## Surface

```
$ loft repl
loft 0.8.4 — interactive
Type :help for commands, :quit to exit.

loft> x = 1 + 2
loft> x
3
loft> fn dbl(n: integer) -> integer { n + n }
loft> dbl(x)
6
loft> fn broken(
...... > 
...... > }
error: Expect token ; at line 1:13
loft> :reset
session reset.
loft> :quit
$
```

## Design

### Prompt + line editor

Use `rustyline` (cross-platform line editor with history).  Already
a familiar dep choice for Rust REPLs.  Initial prompt `loft> `;
continuation prompt `...... > ` when `parse_statement` returns
`NeedMore`.

History persists to `~/.cache/loft/history` (configurable via
`LOFT_HISTORY` env var).  Up/down arrow recalls.

### Input loop

```rust
fn repl_main(opts: ReplOptions) -> Result<()> {
    let mut p = Parser::new();
    p.parse_dir("default", true, false)?;
    let mut state = State::new(p.database.clone());
    compile::byte_code(&mut state, &p.data);   // stdlib only

    let mut rl = rustyline::Editor::<()>::new()?;
    let _ = rl.load_history(&history_path());

    let mut accumulator = String::new();
    let mut prompt = "loft> ";

    loop {
        match rl.readline(prompt) {
            Ok(line) => {
                accumulator.push_str(&line);
                accumulator.push('\n');
                let _ = rl.add_history_entry(&line);
                match p.parse_statement(&accumulator) {
                    ParseResult::Ready { entry_def_nr } => {
                        compile::byte_code(&mut state, &p.data);
                        match state.execute_at_def_safe(entry_def_nr, &p.data) {
                            Ok(value) => print_value(&value, &p.data, &state.database),
                            Err(e) => eprintln!("runtime error: {e}"),
                        }
                        state.reset_for_repl();
                        accumulator.clear();
                        prompt = "loft> ";
                    }
                    ParseResult::NeedMore => {
                        prompt = "...... > ";
                    }
                    ParseResult::Error(diags) => {
                        for d in diags { eprintln!("{d}"); }
                        accumulator.clear();
                        prompt = "loft> ";
                    }
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                accumulator.clear();
                prompt = "loft> ";
                println!("(interrupted)");
            }
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        }
    }
    let _ = rl.save_history(&history_path());
    Ok(())
}
```

### `:cmd` builtins

| Command | Effect |
|---------|--------|
| `:quit` / `:q` | Exit cleanly. |
| `:help` / `:h` | List commands. |
| `:reset` | Drop all user state (vars, fns, structs); keep stdlib loaded. |
| `:clear` | Clear the screen (terminal escape). |
| `:vars` | List currently-defined locals + their values. |
| `:fns` | List user-defined fns (signature only). |
| `:bytecode <fn>` / `:rust <fn>` / `:slots <fn>` | Phase-05 introspection — implemented in next phase. |

### Result printing

Every successfully-executed input either:
- Returns a value (expression-level input) → print via the
  type-aware printer.
- Returns void (statement-level input like `x = 1`) → no print.

The type-aware printer uses the existing `dump_value` machinery
from `src/state/debug.rs` (the same pretty-printer used for
`LOFT_LOG=full` execution traces).  Concrete shape:
- Integers: `42`.
- Floats: `1.5_f64`.
- Strings: `"hello"`.
- Booleans: `true` / `false`.
- Structs: `Point { x: 1, y: 2 }`.
- Vectors: `[1, 2, 3]`.

### Error recovery

Three error classes:

1. **Parse error** — `ParseResult::Error`.  Prints diagnostics,
   clears accumulator, returns to prompt.  `data` + `database`
   are restored by phase-02's transactional rollback.

2. **Runtime error** — `panic!` from inside execute.  Caught by
   `std::panic::catch_unwind`; prints `runtime error: <msg>`,
   resets State, returns to prompt.  Database is left in whatever
   state the panic interrupted; if data corruption is
   detected (e.g., a half-finished struct), the REPL may need
   to `:reset` to recover — print a hint.

3. **Compiler bug** — internal panic (e.g., a debug_assert in
   codegen).  In debug builds, propagate; in release builds,
   catch + print `compiler bug: <msg>` + suggest `:reset` or
   exit.

### Multi-line blocks

`parse_statement` returns `NeedMore` for incomplete input.  The
REPL accumulates lines until either:
- Parser returns `Ready` or `Error`.
- User hits Ctrl-C → discard accumulator.
- User enters `\\` on its own line → cancel multi-line mode (rare).

## Implementation outline

| Step | Files | Effort |
|------|-------|--------|
| 1. Add `repl` subcommand to argv parser | `src/main.rs` | XS |
| 2. New `src/repl.rs` module with `repl_main` | new file | S |
| 3. Result printing (reuse `dump_value`) | `src/repl.rs`, `src/state/debug.rs` (re-export if needed) | S |
| 4. `:cmd` dispatch handler | `src/repl.rs` | S |
| 5. `panic::catch_unwind` wrapper around `execute_at_def_safe` | `src/state/mod.rs` (new method) | XS |
| 6. History file I/O | `src/repl.rs` | XS |
| 7. End-to-end test driving stdin | `tests/repl.rs` (new) | S |

## Tests

### Scripted session

```rust
#[test]
fn repl_basic_session() {
    let stdin = b"x = 42\nx\n:quit\n";
    let stdout = run_repl(stdin);
    assert!(stdout.contains("42"));
}
```

The test pipes stdin via a `Cursor` and captures stdout.  Doesn't
exercise the real terminal (line editor) — just the underlying
input loop.

### Multi-line block

```rust
#[test]
fn repl_multi_line_fn() {
    let stdin = b"fn dbl(n: integer) -> integer {\nn + n\n}\ndbl(21)\n:quit\n";
    let stdout = run_repl(stdin);
    assert!(stdout.contains("42"));
}
```

### Error recovery

```rust
#[test]
fn repl_recovers_from_parse_error() {
    let stdin = b"x = ???\nx = 7\nx\n:quit\n";
    let stdout = run_repl(stdin);
    assert!(stdout.contains("error"));
    assert!(stdout.contains("7"));   // session continued
}
```

## Acceptance criteria

1. `loft repl` starts the prompt, accepts input, prints results.
2. Multi-line input works (incomplete → continuation prompt).
3. Parse errors don't crash; user can keep typing.
4. Runtime errors don't crash; `:reset` recovers any corruption.
5. `:quit` exits cleanly.
6. History persists across launches.
7. Tests cover: basic, multi-line, error-recovery, `:cmd`
   handling.

## Effort

**M (~1–2 days).**  Phase 02 + phase 03 carry the architectural
weight; this phase is mostly UX glue.

## Risk

- **rustyline dep** — adds a new dependency.  Cross-platform but
  brings in some weight.  Alternative: hand-rolled line editor
  (~200 lines) — simpler dep but more code.  Decision deferred.
- **panic::catch_unwind across FFI** — par worker panics and
  native-extension panics may not be caught cleanly.  Mitigation:
  document that "compiler bug" panics still abort.

## Out of scope

- **WASM browser playground** — the REPL infrastructure
  (parse_statement + reset + execute) runs in WASM in principle,
  but the playground UI is a separate web app.  Phase 06 adds a
  pointer for future work.
- **Async / tab-complete** — basic line editor first; completion
  (referencing currently-defined fns / locals) is a follow-up
  once phase 04 ships.
- **Pretty syntax highlighting** in the prompt — maybe with
  `syntect` or hand-rolled.  Deferred.

## See also

- [02-statement-parser.md](02-statement-parser.md) — input
  parsing.
- [03-state-reset-and-append.md](03-state-reset-and-append.md) —
  execution + state lifecycle.
- [05-repl-introspection.md](05-repl-introspection.md) — next
  phase, adds `:bytecode` / `:rust` / `:slots` to the prompt.
