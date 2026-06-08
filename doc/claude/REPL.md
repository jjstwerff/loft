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
or a `:command` when the line starts with `:`. After a `.`, Tab completes
**members**: a value's methods — shown with a trailing `(` so you can see
they're callable (`"hi".` → `starts_with(`, `length(`) — a struct variable's
fields (`p.` → `x`, `y`), and an enum type's variants (`Color.` → `Red`,
`Green`). (A member after an index or call result — `xs[0].` — needs type
inference the completer doesn't run yet, so it offers nothing rather than
guessing.)

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
| `:break <fn>` | Set a breakpoint at the start of a function's body — also `<fn>:<line>` (a line in that function). Both are *function-scoped*, the only form unique in the REPL (every input restarts line numbering under the synthetic `<repl>` file, so a bare line isn't unique; file:line is for a file-run debugger). `:break` lists, `:break clear` removes. The next call that reaches it **suspends** into the paused sub-mode (below). |
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

### Paused at a breakpoint

When a call reaches a breakpoint the REPL **suspends** inside that frame and the
prompt changes to `(dbg)`. The frame's in-scope variables are shown, and you can
inspect them, change a value, and step:

```
loft> :break calc
breakpoint set: calc
loft> calc(5)
⏸ paused in calc | n = 5
(dbg) n * 3
15
(dbg) n = 99
⏸ paused in calc | n = 99
(dbg) :continue
▶ resumed — run finished
990
```

At the `(dbg)` prompt:

| Input | What it does |
|---|---|
| `:step` (`:s`) | Run to the next source line, **into** any call. |
| `:next` (`:n`) | Run the current line's calls to completion, **over** them, then stop at the next line. |
| `:finish` (`:o`) | Run to the current function's return — **out** to the caller. |
| `:continue` (`:c`) | Run to the next breakpoint, or to the end of the call. |
| `:vars` | Re-show the current frame. |
| `name = <expr>` | **Edit** a scalar local (`integer` / `float` / `single` / `boolean` / `character`) in the live frame; the RHS is evaluated against the frame, so `n = n + 1` and `b = !b` work. The resumed call uses the new value. |
| *any expression* | **Evaluate** it against the frame's live variables (`n * 3`, `pt.x * pt.y`) and print the value. |
| `:quit` (`:q`) | Leave the REPL. |

The frame *is* a REPL: type any expression and it is evaluated against the paused
variables (every type — scalars, text, structs, vectors), just like the top-level
prompt but scoped to the function you're stopped in. Editing a value (`n = 99`,
`f = 2.0`, `b = !b`) writes straight into the live frame, so when you `:continue`
the rest of the function runs with the change — the call above returns `99 * 10`
rather than `5 * 10`. Editing is for **scalar** locals; a `text` / struct / vector
local can be read but not yet written (its slot holds a store pointer). The verbs
also work without the leading colon (`step`, `next`, `continue`). Breakpoints
persist across calls until `:break clear`.

> **Where breakpoints land.** A breakpoint currently attaches to a source-mapped
> bytecode op, and those are emitted at fault-prone arithmetic (`+ - * / % << >>`),
> so a function whose reached path has no such op (a pure `if`/constant body) may
> not pause. Give the function a normal arithmetic statement, or break on a line
> that has one. Widening source-span coverage so every statement is breakable is
> tracked separately.

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

When you bind a name to a value, the REPL runs the right-hand side **once** and
snapshots the value — for **every** type (numbers, text, structs, vectors,
enums) — so a side effect (printing, reading input) in the binding happens a
single time, however often you read the name later (`name = read_line()` prompts
once).

Current limits, with their planned fixes in
[plans/12-repl-and-introspection/](plans/12-repl-and-introspection/):

- For a long session the REPL still **re-runs the accumulated bindings** (now all
  side-effect-free literals) each time you observe a value, so cost grows with
  session length. It stays *correct* (the literals are pure); the planned fix is
  the store-resident session that keeps values without replay
  ([plans/14-store-resident-repl-session/](plans/14-store-resident-repl-session/README.md)).
- Auto-resume re-runs your bindings rather than restoring their stored values,
  so a non-deterministic result (`random()`, `now()`) is recomputed on resume.
  Exact value-for-value restore is the same store-resident model
  ([@PLN14](plans/14-store-resident-repl-session/README.md)).

## See also

- [LOFT.md](LOFT.md) — the language the REPL evaluates.
- [STDLIB.md](STDLIB.md) — the functions available in every session.
- [plans/12-repl-and-introspection/](plans/12-repl-and-introspection/) — the
  design and per-phase notes for this work.
