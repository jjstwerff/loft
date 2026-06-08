<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The loft REPL and introspection

This page explains two tools for trying loft and looking inside it:

- the **REPL** — an interactive prompt where you type loft one line at a time
  and see the result right away;
- the **introspection** output — bytecode, generated Rust, and variable slot
  tables for any program or REPL session.

Both ship with the `loft` binary. No setup is needed.

## Start the REPL

Run `loft` with no file, or `loft repl`:

```
$ loft
loft REPL — :help for commands, :quit to exit
loft> 1 + 2
3
loft> x = 40 + 2
loft> x
42
```

Type an expression and its value prints. Type `x = …` to name a value; the name
stays available on the next line. Press Ctrl-D (end of input) or type `:quit` to
leave. Arrow keys recall earlier lines (history is kept in `~/.loft_history`),
and Ctrl-C abandons the line you're typing. Press **Tab** to complete a name —
functions and types (yours and the stdlib's) and the variables you've bound —
or a `:command` when the line starts with `:`.

### Your session is restored next time

Start the REPL interactively and it replays the bindings and definitions from
your last session, so `x`, your functions, and your structs are still there:

```
$ loft
restored 2 statement(s) from last session
loft> dbl(x)
84
```

The session is saved to `~/.loft_session` — only the state-changing lines, not
the expressions you merely printed. Start clean with `loft repl --fresh`, or
`:reset` mid-session to clear both the live state and the saved file. A saved
line that no longer parses (say, after a loft upgrade) is skipped on replay and
never blocks the rest.

Because resume re-runs your bindings, a value drawn from `random()` or `now()`
is produced afresh — the original stream is not reproduced (set an explicit seed
if you need reproducibility). Auto-resume is interactive-only: piped or scripted
input never reads or writes the session file, so captured output stays stable.

### Define things and call them

A function or struct you define stays defined for the rest of the session:

```
loft> fn dbl(n: integer) -> integer { n + n }
loft> dbl(x)
84
loft> struct Point { x: integer, y: integer }
loft> Point { x: 1, y: 2 }
{x:1,y:2}
```

A value prints in loft's own form: `42`, `{x:1,y:2}`, `[1, 2, 3]`. (This is not
JSON — struct fields have no quotes. Add the `:j` format to get JSON, e.g.
`"{p:j}"`.)

### Multi-line input

If a line is not finished — an open `{`, `(`, `[`, an unterminated `"…"`, or a
trailing operator — the prompt changes to `..... >` and waits for the rest:

```
loft> fn add(a: integer, b: integer) -> integer {
..... >   a + b
..... > }
loft> add(2, 3)
5
```

### Mistakes don't end the session

A typo prints an error and the prompt comes back; everything you defined before
is still there. A run-time error (such as a failed `assert`) is caught the same
way — the session keeps going.

## REPL commands

Commands start with a colon:

| Command | What it does |
|---|---|
| `:help` (`:h`) | List the commands. |
| `:quit` (`:q`) | Leave the REPL. |
| `:reset` | Forget everything you defined; the standard library stays loaded. |
| `:fns` | List the functions you have defined, with their return type. |
| `:vars` | List the variables you have bound, each with its current value. |
| `:type <expr>` | Show the type of an expression without running it. |
| `:bytecode [fn]` | Show the bytecode — all your functions, or just the named one. |
| `:rust [fn]` | Show the Rust code loft generates for native compilation. |
| `:slots [fn]` | Show each function's variable slot table (name, type, slot range). |

```
loft> fn dbl(n: integer) -> integer { n + n }
loft> :fns
dbl -> integer
loft> :slots dbl
fn n_dbl:
  #    arg  name   type   scope  slot         live
  ----------------------------------------------------------------------
  0    arg  n      int    0      [0, 8)       -
```

## Introspection without the REPL

`loft introspect <file>` prints the same views for a whole program:

```
$ loft introspect myprogram.loft          # bytecode + Rust + slots + types
$ loft introspect --show-bytecode prog.loft   # one section only
$ loft introspect --fn n_main prog.loft       # one function only
$ loft introspect --all-fns prog.loft         # include the standard library
```

By default only your own functions are shown; `--all-fns` adds the standard
library. Sub-flags select one section (`--show-bytecode`, `--show-rust`,
`--show-slots`, `--show-types`) or write a section to a file (`--bytecode-out`,
`--rust-out`, …). This replaces the older `LOFT_LOG=…` dump route for everyday
use; `LOFT_LOG` still works for live execution traces (see
[TESTING.md](TESTING.md) § LogConfig).

## How session state works (and its limits)

The REPL keeps the names you bind by re-running the statements that define them
in one shared scope each time you ask for a value. This works for values of any
type — numbers, text, structs, vectors. As long as a statement only computes a
value (no side effect), re-running it gives the same result: `x = 1; y = x + 2`
always yields the same `y`, and `name = "Alice"` stays bound to `"Alice"`.

Current limits, with their planned fixes in
[plans/12-repl-and-introspection/](plans/12-repl-and-introspection/):

- A statement with a **side effect** (printing, file I/O) would run again each
  time a later line reads a variable — and each time you run `:vars`, which
  realises every value by re-running the body. The fix is the stack-resident
  execution model (phase 03 notes) that runs each new line only once.
- Auto-resume re-runs your bindings rather than restoring their stored values,
  so a non-deterministic result (`random()`, `now()`) is recomputed on resume.
  Exact value-for-value restore is the same stack-resident model above.

## See also

- [LOFT.md](LOFT.md) — the language the REPL evaluates.
- [STDLIB.md](STDLIB.md) — the functions available in every session.
- [plans/12-repl-and-introspection/](plans/12-repl-and-introspection/) — the
  design and per-phase notes for this work.
