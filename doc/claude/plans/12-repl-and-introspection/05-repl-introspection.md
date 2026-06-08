<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 05 — REPL introspection commands

**Status: shipped (first cut, 2026-06-08).** The REPL `:command` dispatcher
gains `:bytecode [fn]`, `:rust [fn]`, `:slots [fn]` (each compiles the current
session and emits that phase-01 introspection section to stdout, optionally
filtered to named fns) and `:fns` (lists user-defined functions + return type,
excluding stdlib + synthetic generation wrappers).  `ReplSession::introspect` +
`list_fns` in `src/repl.rs`; tests in `tests/repl.rs`.

Deferred: `:type <expr>` (needs a type-only parse entry) and value-bearing
`:vars` (needs result capture — the same in-process value-read gap noted in
phase 03/04).  `:vars`/`:type` will land with that capture work.

## Goal

Layer phase-01's introspection routines onto phase-04's `:cmd`
handler so the user can inspect the currently-defined session
without leaving the REPL.

## Surface

```
loft> fn dbl(n: integer) -> integer { n + n }

loft> :bytecode dbl
fn n_dbl [d_nr=520]:
  000:[0]    Reserve(8)
  002:[8]    OpVarInt 0 -> [stack[0]]
  ...

loft> :rust dbl
fn n_dbl(stores: &mut Stores, var_n: i64) -> i64 {
  return (var_n) + (var_n);
}

loft> :slots dbl
fn n_dbl:
  idx | arg | name | type    | scope | slot range  | live
  0   |  *  | n    | integer | -     | [0..8)      | [0..2]
  1   |     | __   | integer | -     | [8..16)     | [1..1]

loft> :type x + 1
integer

loft> :vars
x: integer = 7
y: text = "hello"

loft> :fns
dbl(n: integer) -> integer
```

## Design

### Command dispatcher

Phase 04 already routes `:cmd` to a dispatcher.  Phase 05 adds
new arms.  The dispatcher invokes phase-01's `introspect::run`
with options selecting one section.

```rust
fn dispatch_cmd(cmd: &str, args: &[&str], session: &mut Session) {
    match cmd {
        "bytecode" => {
            let opts = IntrospectOptions {
                bytecode: Some(stdout_writer()),
                rust: None,
                slots: None,
                fn_filter: args.iter().map(|s| s.to_string()).collect(),
                all_fns: false,
            };
            crate::introspect::run_with(&session.parser.data,
                &session.state, opts);
        }
        "rust" => { … }      // similar
        "slots" => { … }
        "type" => {
            let expr = args.join(" ");
            match session.parser.parse_type_only(&expr) {
                Ok(t) => println!("{}", t.show(&session.parser.data, …)),
                Err(e) => eprintln!("{e}"),
            }
        }
        "vars" => print_session_locals(session),
        "fns" => print_session_fns(session),
        "reset" => session.reset(),
        "help" => print_help(),
        "quit" | "q" => session.quit = true,
        other => eprintln!("unknown command: :{other}  (type :help)"),
    }
}
```

### `:type <expr>`

A new parser entry `parse_type_only(&str) -> Result<Type>` runs the
parser through pass 2 with codegen disabled, returning the
inferred type without executing.

Implementation: the parser has a `lookup_only` mode that's
roughly equivalent.  Phase 05 exposes it as a public method.

### `:vars`

Walks `__repl_session`'s fields and prints each one with its
current value.  Reuses `dump_value` from
`src/state/debug.rs`.

### `:fns`

Walks `data.user_fn_d_nrs()` and prints the signature of each
fn whose source-position file is the synthetic REPL file (so
stdlib fns are filtered out — same rule as phase 01's default
behaviour).

## Implementation outline

| Step | Files | Effort |
|------|-------|--------|
| 1. Wire `:bytecode` / `:rust` / `:slots` to `introspect::run` | `src/repl.rs` | XS |
| 2. Add `Parser::parse_type_only` for `:type` | `src/parser/mod.rs` | S |
| 3. `:vars` + `:fns` printing | `src/repl.rs` | XS |
| 4. `:help` listing | `src/repl.rs` | XS |
| 5. Tests covering each command | `tests/repl.rs` | S |

## Tests

```rust
#[test]
fn repl_bytecode_command() {
    let stdin = b"fn dbl(n: integer) -> integer { n + n }\n:bytecode dbl\n:quit\n";
    let stdout = run_repl(stdin);
    assert!(stdout.contains("OpVarInt"));   // some bytecode opcode
}

#[test]
fn repl_type_command() {
    let stdin = b"x = 42\n:type x + 1\n:quit\n";
    let stdout = run_repl(stdin);
    assert!(stdout.contains("integer"));
}

#[test]
fn repl_vars_command() {
    let stdin = b"x = 42\ny = \"hi\"\n:vars\n:quit\n";
    let stdout = run_repl(stdin);
    assert!(stdout.contains("x: integer = 42"));
    assert!(stdout.contains("y: text = \"hi\""));
}
```

## Acceptance criteria

1. Each `:cmd` produces the documented output for a fresh
   session with one user-defined fn / value.
2. `:bytecode` / `:rust` / `:slots` output matches phase-01's CLI
   output for the same fn (modulo formatting differences).
3. `:type <expr>` infers without side effects (no allocations,
   no codegen, no execute).
4. `:vars` reflects the live `__repl_session` contents.
5. Errors in `:cmd` arguments (unknown fn, bad expr) print
   diagnostics but don't crash the REPL.

## Effort

**S (~½–1 day).**  Phase 01 carries the introspection plumbing;
phase 05 wires existing functions to new command names.

## Out of scope

- **`:debug` / breakpoints** — DAP-style stepping is a separate
  feature tracked in `doc/claude/lib_plans/future/09-lsp/README.md` (LSP.3 / NDB.0).
- **`:save` / `:load` of a session to disk** — deferred to
  phase 06 cleanup if there's user demand.
- **Tab completion** of fn names, locals, struct fields — useful
  but a polish phase 04 follow-up, not phase 05.

## See also

- [01-introspection-cli.md](01-introspection-cli.md) — the
  underlying introspection plumbing this phase calls.
- [04-repl-shell.md](04-repl-shell.md) — the `:cmd` dispatcher
  this phase extends.
