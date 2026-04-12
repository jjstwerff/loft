# Long-function audit — single task vs. split candidates

Survey of every function in `src/` longer than 60 lines, classified by
**does it do one thing or many?**  The goal is to identify split
candidates without mass-rewriting — each row is actionable in a
separate focused PR.

**146 functions in-tree are longer than 60 lines.**  Of those, the
survey below covers the 25 longest (above ~130 lines).  The remainder
are mostly in the "legitimate-long" categories below and need
individual inspection if anyone wants to dig further.

## Categorisation

| Kind | What it looks like | Split appropriate? |
|---|---|---|
| **Dispatcher** | Large match/if-chain that delegates one branch per case. Often a `fn handle_X(msg) -> Response`. | **No** — the shape *is* the single task. Splitting produces N tiny functions used once. |
| **State machine** | Loop that walks input one token/byte at a time, branching on state. `lexer::next`, `formatter::scan`, every recursive-descent parse_* that doesn't delegate. | **No** — moving the state out of locals forces it into struct fields, making the code harder to reason about. |
| **Multi-phase workflow** | A→B→C→D pipeline where each phase writes to a new local that the next phase reads. Clear "collect → transform → serialise" structure. | **Yes** — one free function per phase. Each takes the previous phase's output; no shared mutable state. |
| **God function** | Mixes I/O, business logic, formatting, and side effects; hard to test in isolation; multiple reasons to change. | **Yes** — highest-value splits; start here. |

## Survey — 25 longest functions

Column meanings:
- **Kind** — one of the four categories above
- **Single?** — Yes = already one thing, leave it; Split = should split; Maybe = worth another look
- **Lines** — current line count (approx)

| File | Function | Lines | Kind | Single? | Notes |
|---|---|---:|---|---|---|
| `main.rs` | `main` | 1105 | God | **Split** | arg parser + path resolver + 8 subcommand dispatchers all inline. See (A) below. |
| `generation/dispatch.rs` | `output_call_inner` | 443 | Dispatcher | No | Huge match over opcode names, each branch ~5-30 lines emitting Rust. Hard to split without fracturing the decision tree. |
| `main.rs` | `generate_native_stubs` | 349 | Multi-phase | **Split** | parse_toml → load_loft → classify_sigs → emit_stubs → write_file. See (B) below. |
| `state/codegen.rs` | `generate_inner` | 237 | Dispatcher | No | Large match over `Value` variants. Moving variants to helpers shuffles state through &mut stack. |
| `state/debug.rs` | `validate_stack` | 192 | State machine | No | Diagnostic traversal; internal state must remain in locals. |
| `parser/control.rs` | `parse_tuple_match` | 191 | State machine | No | Recursive descent for one construct; the "phases" here are actually tokens consumed. |
| `variables/intervals.rs` | `compute_intervals` | 185 | Multi-phase | **Maybe** | Two loops (assign birth → assign death) + a fixup loop. Could split `assign_birth`, `assign_death`, `fixup_zone2`. Low risk. |
| `scopes.rs` | `get_free_vars` | 171 | Dispatcher | No | Match over variable types; each branch decides emit/skip. Splitting fragments the decision. |
| `compile.rs` | `disassemble` | 170 | Multi-phase | **Split** | Pretty-printer — no side effects, self-contained. Easy to split by section: header / variables / bytecode / trailer. See (C) below. |
| `parser/mod.rs` | `parse_file` | 160 | Multi-phase | **Split** | 4 clear phases: use loop, apply imports, definitions loop, finalize types, recurse. See (D) below. |
| `formatter.rs` | `scan` | 160 | State machine | No | Tokeniser inner loop. |
| `scopes.rs` | `scan_inner` | 158 | Dispatcher | No | Match over `Value` kinds; same shape as `generate_inner`. |
| `state/mod.rs` | `static_call` | 154 | Multi-phase | **Maybe** | Two snapshots (call_stack, variables); splittable into `snapshot_call_stack` + `snapshot_variables`. |
| `parser/control.rs` | `parse_vector_match` | 151 | State machine | No | Parser. |
| `database/io.rs` | `read_data` | 147 | State machine | No | Binary reader; one field per dispatch. |
| `state/mod.rs` | `coroutine_next` | 144 | State machine | No | Scheduler; status transitions. |
| `parser/mod.rs` | `lib_path` | 144 | God-ish | **Maybe** | Library resolution: check VIRT_FS + check mount dirs + check loft.toml + fallback. 4 roughly-independent lookups; could become 4 helpers with short-circuit. |
| `lexer.rs` | `next` | 141 | State machine | No | Tokeniser. |
| `generation/mod.rs` | `output_function` | 141 | Multi-phase | **Maybe** | Header + preamble + body + trailer. |
| `formatter.rs` | `handle_sym` | 141 | State machine | No | |
| `parser/mod.rs` | `apply_pending_imports` | ~140 | Multi-phase | **Maybe** | Partition + apply wildcard + apply named — splittable. |
| `parser/definitions.rs` | `parse_interface` | ~135 | State machine | No | Parser. |
| `parser/fields.rs` | `parse_record_ref_or_fn_call` | ~135 | Dispatcher | No | |
| `typedef.rs` | `fill_attributes` | ~135 | Dispatcher | No | |
| `state/fill.rs` | every fn | 6000+ | Auto-generated | No | Regenerated from `#rust` annotations; never edit by hand. |

## Detailed split proposals

### (A) `src/main.rs::main` — 1105 lines → ~150 + 8 subcommand fns

Today:
```rust
fn main() {
  // 100 lines: parse argv into local flags
  // 200 lines: resolve paths, detect loft.toml, set lib_dirs
  // 50 lines: handle --format/--format-check
  // 50 lines: handle --generate-log-config
  // 50 lines: handle --tests
  // 200 lines: native/native-wasm/html compilation paths
  // 100 lines: install subcommand
  // 100 lines: doc subcommand
  // 100 lines: generate subcommand
  // ... (interleaved)
}
```

Proposed:
```rust
struct CliArgs { /* every --flag here */ }
fn parse_args(argv: &[String]) -> CliArgs;
fn resolve_paths(args: &CliArgs) -> ResolvedPaths;
fn run_format(args, paths) -> ExitCode;
fn run_tests(args, paths) -> ExitCode;
fn run_native(args, paths) -> ExitCode;
fn run_interpret(args, paths) -> ExitCode;
fn run_install(args, paths) -> ExitCode;
fn run_generate_stubs(args, paths) -> ExitCode;
fn run_doc(args, paths) -> ExitCode;

fn main() {
  let args = parse_args(&env::args().skip(1).collect());
  let paths = resolve_paths(&args);
  let code = match args.subcommand {
    Sub::Format => run_format(&args, &paths),
    Sub::Tests  => run_tests(&args, &paths),
    ...
  };
  std::process::exit(code);
}
```

**Risk:** Medium.  The current flow has ordering dependencies (`--tests` forces interpreter, `--native` and `--html` are mutually exclusive with implicit fallback ordering).  An automated split must preserve every branch.
**Benefit:** After the split, adding a new subcommand is a ~20-line change in one file, not a 50-line merge into `main()`.

### (B) `src/main.rs::generate_native_stubs` — 349 lines → 4 helpers

4 phases already visible in the code:
1. `parse_package_toml(pkg_path) -> PackageInfo`
2. `load_loft_sources(info) -> Parser`
3. `classify_signatures(parser) -> Vec<StubSig>`
4. `emit_rust_source(sigs, struct_mods) -> String` (with a sub-helper `emit_one_stub(sig) -> String`)

Then `write_file(path, source)` and done.

**Risk:** Low.  Pure function: reads .loft files, writes one .rs file.
**Benefit:** Each phase becomes testable in isolation; currently only the end-to-end integration test (`tests/lib/native_pkg`) exercises this 349-line function.

### (C) `src/compile.rs::disassemble` — 170 lines → 4 helpers

Sections already labelled in the code:
1. `disasm_header(out, data, def_nr)`
2. `disasm_variables(out, fn_def)`
3. `disasm_bytecode(out, state, start, end)`
4. `disasm_trailer(out, def)`

**Risk:** Lowest of any split here.  Pretty-printer with no side effects on runtime state.
**Benefit:** Each section becomes easy to style-test in isolation.

### (D) `src/parser/mod.rs::parse_file` — 160 lines → 5 helpers

Phases, in order:
1. `parse_use_directives()` — the `while use ...` loop
2. `apply_pending_imports()` — already a helper
3. `parse_top_level_definitions()` — the main loop (`parse_enum`/`parse_struct`/...)
4. `check_trailing_tokens()` — "Syntax error: unexpected ..."
5. `finalize_types()` — `actual_types` + `fill_all` + `enum_fn`

Already labelled in-source; moving the blocks to named helpers is mechanical.

**Risk:** Low.  The `self.todo_files` queue is the only inter-phase state and it's explicit.
**Benefit:** Stack frames during two-pass parsing become visible in debuggers; currently the whole file parse lives in one 160-line frame.

## Remaining long functions (60–130 lines)

121 more functions fall in the 60–130 line range.  Each needs
individual inspection — most are dispatchers (the pattern is
unavoidable for loft's tree-walking architecture) but a handful are
multi-phase in disguise.

Good candidates for future review:
- `src/parser/expressions.rs::parse_assignment` — expression + assign-op + place
- `src/state/text.rs::format_string` — escape parse + segment walk + emit
- `src/database/allocation.rs::remove_claims` — unclaim + free-tree rebalance
- `src/generation/emit.rs::output_block` (just edited for P86n) — patch hoisted returns + operator loop + trailing-value handling; arguably 3 phases

## Methodology for future splits

1. Start from this document; pick one row marked **Split**.
2. Write an IR-level or behaviour-level regression test *before* any refactoring — catches silent semantic changes.
3. Extract *one* phase into a free function with `#[must_use]` on its output.  Compile, test, commit.
4. Repeat per phase.  No "extract five helpers at once" merges.
5. Once the caller is down to ~30 lines of glue, it usually becomes obvious whether the remaining shape is the right one.

## See also

- [BRITTLE.md](BRITTLE.md) — the same 10 subsystems from a different
  angle (fragility rather than size).  Many entries overlap.
- [CODE.md](CODE.md) — naming + quality rules.
- [DESIGN.md](DESIGN.md) — the algorithm catalog that explains *why*
  some of the long dispatcher-style functions are inherently so.
