<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 01 — Introspection CLI

**Status: open.**

## Goal

A `loft introspect <file>` (or `loft --introspect <file>`) entry
that emits **bytecode disassembly + generated Rust + variable slot
tables** for any loft program, packaged in one CLI surface.

Today these dumps are reachable via `LOFT_LOG=…` env variants
writing to `tests/dumps/*.txt`, scattered single-purpose flags
(`--dump`, `--native-emit`), and ad-hoc setup.  Phase 01 wraps the
underlying primitives (`state.dump_bytecode`, `dump_variables`,
`Output::output_native`) in a clean ergonomic CLI.

## Surface

### Default — print all three to stdout

```
$ loft introspect myprogram.loft
=== bytecode ===
[N functions, Y opcodes total — see below]

fn n_test [d_nr=520]:
  000:[0]    Reserve(8)
  002:[8]    OpConstInt 42 -> [stack[0]]
  012:[8]    Return [stack[0]]
  ...

fn n_main [d_nr=1]:
  ...

=== rust ===
fn n_test(stores: &mut Stores) -> i64 { ... }
...

=== slots ===
fn n_test:
  idx | arg | name      | type        | scope | slot range  | live
  0   |     | result    | integer     | -     | [0..8)      | [0..2]
  ...
```

### Per-flag selection

```
$ loft introspect --bytecode myprogram.loft     # bytecode only
$ loft introspect --rust myprogram.loft          # Rust only (== --native-emit to stdout)
$ loft introspect --slots myprogram.loft         # slot tables only
```

When more than one is set, the output sections appear in fixed
order: `bytecode`, `rust`, `slots`.

### Function filter

```
$ loft introspect --fn n_test myprogram.loft
```

Restricts every section to a single function (matches `LOFT_LOG`'s
`fn:<name>` preset).  Multiple `--fn` flags are additive.

### Output to files

```
$ loft introspect --bytecode-out myprogram.bc \
                  --rust-out myprogram.rs \
                  --slots-out myprogram.slots myprogram.loft
```

When any `--*-out` flag is set, that section goes to its file
instead of stdout.  Sections without a matching `--*-out` still
print to stdout (so the default-stdout shape is "everything
mixed", and per-file routing is opt-in).

### Default natives filter

By default introspection skips functions in `default/*.loft`
(matches `--dump`'s default — users care about their code, not
stdlib).  `--all-fns` (mirrors `LOFT_LOG=all_fns`) includes the
stdlib.

## Implementation outline

### CLI parsing

Add a new top-level subcommand pattern alongside `--tests`,
`--format`, etc.  Position in `src/main.rs` argv parser:

```rust
} else if arg == "introspect" || arg == "--introspect" {
    // Consume subsequent flags until a non-flag positional arg
    let mut want_bc = false;
    let mut want_rust = false;
    let mut want_slots = false;
    let mut bc_out: Option<String> = None;
    let mut rust_out: Option<String> = None;
    let mut slots_out: Option<String> = None;
    let mut fn_filter: Vec<String> = Vec::new();
    let mut all_fns = false;
    let mut filename: Option<String> = None;
    while let Some(next) = args.next() {
        match next.as_str() {
            "--bytecode" => want_bc = true,
            "--rust" => want_rust = true,
            "--slots" => want_slots = true,
            "--bytecode-out" => bc_out = args.next(),
            "--rust-out" => rust_out = args.next(),
            "--slots-out" => slots_out = args.next(),
            "--fn" => fn_filter.push(args.next().unwrap()),
            "--all-fns" => all_fns = true,
            arg if arg.starts_with('-') => bail("unknown flag {arg}"),
            other => filename = Some(other.to_string()),
        }
    }
    if !want_bc && !want_rust && !want_slots {
        // Default: all three.
        want_bc = true;
        want_rust = true;
        want_slots = true;
    }
    introspect::run(&filename.unwrap(), &Options { … })?;
    return;
}
```

### `introspect` module

New file `src/introspect.rs` (or under `src/main.rs` if the binary
is the only consumer).  Public `run(filename, options) -> Result<()>`:

```rust
pub struct Options {
    pub bytecode: Option<Box<dyn Write>>,
    pub rust: Option<Box<dyn Write>>,
    pub slots: Option<Box<dyn Write>>,
    pub fn_filter: Vec<String>,
    pub all_fns: bool,
}

pub fn run(filename: &str, opts: Options) -> std::io::Result<()> {
    // 1. Parse + compile (same as `--dump`).
    let mut p = Parser::new();
    p.parse_dir("default", true, false)?;          // stdlib
    p.parse(filename, false)?;                      // user file
    scopes::check(&mut p.data);

    let mut state = State::new(p.database.clone());
    compile::byte_code(&mut state, &p.data);

    // 2. For each output sink, dispatch to the existing dumper.
    if let Some(mut w) = opts.bytecode {
        emit_bytecode(&mut w, &state, &p.data, &opts)?;
    }
    if let Some(mut w) = opts.rust {
        emit_rust(&mut w, &p.data, &state.database, &opts)?;
    }
    if let Some(mut w) = opts.slots {
        emit_slots(&mut w, &p.data, &opts)?;
    }
    Ok(())
}

fn emit_bytecode(w: &mut dyn Write, state: &State, data: &Data, opts: &Options) -> Result<()> {
    let log_config = LogConfig {
        bytecode: true,
        ir: false,
        execution: false,
        annotate_slots: true,
        function_filter: opts.fn_filter.clone(),
        include_default: opts.all_fns,
        ..Default::default()
    };
    state.dump_bytecode(w, &log_config, data)
}

fn emit_rust(w: &mut dyn Write, data: &Data, stores: &Stores, opts: &Options) -> Result<()> {
    let mut output = generation::Output::new(data, stores, …);
    output.output_native(w, 0, data.definitions())
}

fn emit_slots(w: &mut dyn Write, data: &Data, opts: &Options) -> Result<()> {
    for d_nr in data.user_fn_d_nrs() {
        if !opts.fn_filter.is_empty()
            && !opts.fn_filter.iter().any(|f| f == &data.def(d_nr).name)
        {
            continue;
        }
        let function = data.def(d_nr);
        writeln!(w, "fn {}:", function.name)?;
        crate::variables::validate::dump_variables(w, &function.variables, data)?;
    }
    Ok(())
}
```

### Sink wiring

Default sink: `io::stdout()`.  Per-flag overrides write to the
named file via `BufWriter<File>`.  The Options struct stores a
`Box<dyn Write>` per dimension; `None` means "skip this section".

### Header lines

Each section prints a `=== bytecode ===` / `=== rust ===` / `===
slots ===` separator when going to stdout.  When per-file routing
is used, no separator (the file IS the section).

## Tests

### Golden snapshot

A small `.loft` file under `tests/data/introspect_golden.loft`:

```loft
fn dbl(x: integer) -> integer { x + x }
fn test() {
  result = dbl(21);
  assert(result == 42, "doubled");
}
```

Three golden files under `tests/golden/introspect/`:

- `introspect_golden.bytecode.txt`
- `introspect_golden.rust.txt`
- `introspect_golden.slots.txt`

A test in `tests/introspect.rs` runs each `loft introspect --xxx
… > tmp` invocation and asserts byte-exact equality with the
golden file.  Mirrors `tests/scripts/snap_smoke.sh` but smaller.

### Function-filter test

Another test verifies `--fn n_test` restricts every section to
just `n_test`'s shape.

### CLI-error tests

- Unknown flag: exits non-zero with a useful message.
- Missing positional argument: exits non-zero, prints usage.
- File doesn't exist: exits non-zero with the file path quoted.

## Acceptance criteria

1. `loft introspect <file>` emits bytecode + Rust + slots to
   stdout in the documented section order.
2. `loft introspect --bytecode` emits ONLY bytecode (no `=== rust
   ===` / `=== slots ===` headers, no other sections).
3. `loft introspect --bytecode-out FILE.bc <file>` writes
   bytecode to `FILE.bc` and Rust + slots to stdout.
4. `--fn n_xxx` filters every section to the named function.
5. `--all-fns` includes `default/*.loft` functions in every
   section.
6. Output is byte-stable across loft versions (golden tests).
7. Existing `--dump` / `--native-emit` flags keep working
   unchanged (no breaking change).
8. New regression test in `tests/introspect.rs` covers the four
   acceptance shapes above.

## Effort

**S (~1 day).**  ~250 lines of new code (CLI handler + introspect
module + 3 sinks).  Underlying primitives unchanged.  Most of the
effort is golden tests and the CLI argv parser.

## Out of scope

- **Live REPL inspection** — phase 05 builds the REPL `:cmd`
  handlers on top of the same introspect module.
- **Editor / IDE integration** — would expose introspect via the
  language server protocol; tracked separately in
  `doc/claude/LSP.md`.
- **Trace mode** — emitting per-step execution trace is
  `LOFT_LOG=full`'s job; introspection covers static dumps only.
- **Profiling output** — flame graphs, opcode-count histograms
  belong in a perf tool, not introspection.
- **Cross-version diff** — comparing two `.loft` files'
  introspection output is a `diff` job for the user; we don't
  build a diff tool in-tree.

## See also

- [00-baseline.md](00-baseline.md) — survey of the underlying
  primitives.
- `src/state/debug.rs::dump_code` — bytecode disassembler.
- `src/variables/validate.rs::dump_variables` — slot dumper.
- `src/generation/mod.rs::output_native` — Rust emitter.
- `src/log_config.rs` — `LOFT_LOG` presets that this CLI's
  sub-flags mirror.
- `doc/claude/TESTING.md` § LogConfig — the user-facing reference
  this introspection tool replaces for non-test users.
