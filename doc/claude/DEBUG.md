
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Debugging Strategy

The primary debug surface is the `LOFT_LOG` environment variable, which selects a
preset defined in `src/log_config.rs`. Set it before running a test:

```bash
LOFT_LOG=minimal cargo test -- my_test 2>&1 | head -200
LOFT_LOG=ref_debug cargo test -- my_test 2>&1 | head -500
LOFT_LOG=full cargo test -- my_test 2>&1
```

---

## Contents
- [Interactive debugging (`loft debug`)](#interactive-debugging-loft-debug)
- [Preset Guide](#preset-guide)
- [Debugging a Parse Error or Wrong IR](#debugging-a-parse-error-or-wrong-ir)
- [Debugging a Runtime Crash or Wrong Result](#debugging-a-runtime-crash-or-wrong-result)
- [Before you believe a fault is RANDOM](#before-you-believe-a-fault-is-random)
- [When it fails in CI but passes locally](#when-it-fails-in-ci-but-passes-locally)
- [The debug-assertions calibration run (`target-da`)](#the-debug-assertions-calibration-run-target-da)
- [Debugging a validate_slots Panic](#debugging-a-validate_slots-panic)
- [Debugging a Scope Analysis Bug](#debugging-a-scope-analysis-bug)
- [Using the Test Framework for Quick Iteration](#using-the-test-framework-for-quick-iteration)
- [Open work](#open-work)

---

## Interactive debugging (`loft debug`)

**Before adding `println`s to a loft program: there is a debugger.** It stops the
program at a line and lets you read and change the live frame — which is what
print-and-re-run is a slow approximation of.

```sh
loft debug prog.loft:12            # break at line 12, drop into the `(dbg)` prompt
loft debug prog.loft:12 --lib lib/ # a program whose `use` resolves through --lib
```

At the prompt: type a **name** (or any expression) to evaluate it at the frame ·
`name = <expr>` to CHANGE a local and carry on with the new value · `:vars` to
re-show the frame · `:step` / `:next` / `:finish` (into / over / out) · `:continue`
· `:watch <expr>` to run until an expression changes · `:undo` / `:redo` to walk
your edits · `:help` · `:quit`. Verbs also work bare (`step`), except when the
frame has a local of that name — then the local wins, so `n` and `c` read your
variables rather than stepping.

It is **driveable non-interactively**, which is what makes it usable from a
script or an agent rather than only by hand:

```sh
printf ':vars\ntotal\n:next\ntotal\n:continue\n' | loft debug prog.loft:12
```

For a scripted session with structured output, use the NDJSON RPC surface
(`loft debug prog.loft --rpc`) — breakpoints with conditions, `eval`, `setValue`,
stepping and tracepoints over stdio. The **`loft-debug` skill** § *The agent debug
surface* is the canonical guide with a worked example; the wire contract is
[plans/16-debugger/PROTOCOL.md](plans/16-debugger/PROTOCOL.md). Order matters
there: `launch` loads, `run` starts, and breakpoints go between them.

**What `:vars` shows, and the two markers.** A paused frame lists every local in
**lexical scope** at that line. A local in scope whose value the frame does not
hold is still listed, with the reason instead of a value:

| shown | means | what to do |
|---|---|---|
| `step = <unset>` | in scope, but its assignment has not run yet on this path | break one line later |
| `i = <reused by step>` | in scope, but its stack slot now belongs to `step` | break one line earlier |

The second is not a bug: the slot allocator is **scope-blind**, so two locals in
one scope share a slot whenever their live ranges do not overlap (in
`for i in 0..4 { step = i * 10; total = total + step; }`, `i` and `step` are the
same four bytes). Once `step` has been written, `i`'s value no longer exists
anywhere in the frame — so the debugger names that rather than printing the slot's
contents under `i`'s name. Reading such a local explains itself, and editing it is
refused. Compiler temps (`__work_N`, `i#index`) are hidden; `:vars all` shows them.

Remaining rough edges are tracked in
[@PLN120](plans/120-debugger-shape/README.md).

Editors get the same engine over DAP (`loft-dap`); see [@I91](../features/I91.md).

---

## Preset Guide

| Preset | What it shows | When to use |
|--------|---------------|-------------|
| `minimal` | Bytecode execution trace (opcode + stack state per step) | Stack corruption, wrong opcode, wrong result |
| `ref_debug` | Reference allocation and free events | Double-free, use-after-free, wrong store_nr |
| `full` | IR tree + bytecode + execution | Everything at once; output is very large |
| `static` | IR tree and bytecode only (no execution) | Codegen bugs, wrong IR, wrong opcode selection |
| `crash_tail:N` | Last N lines before panic | Crash triage when full output is too large |
| `locks` | Every store-lock / store-unlock event with store_nr + rec | "Write to locked store at rec=N fld=M" panics — pinpoints which op acquired the lock |
| `type_timeline:<varname>` | Every type-mutation event for a specific named variable (old → new + origin + the SOURCE LINE that wrote it; set `LOFT_TIMELINE_BT=1` for the stack behind it) | "Why is var X type T at this point?" — flip / change_var_type / depend / substitute_type traces.  A dep list is REPLACED, not merged (`Type::depending`), so "who wrote this dep last" is usually the whole question |
| `ir:<fn_name>` | IR tree dump for the named function only (no bytecode, no execution trace) | "What IR did the parser emit for fn X?" — focused codegen-bug diagnosis |
| `slots:<fn_name>` | Slot-allocation summary for the named function — each var's final slot OR a reason why it was skipped | "Why is var X at slot 65535?" — `Incorrect var X[65535]` codegen panics |
| `captures:<fn_name>` | Capture-pipeline summary for the named function + its lambdas — scalars_to_box, mutated_captures, closure_record attrs with auto-Reference status | "Why is closure-record attr X stored inline vs share-by-DbRef?" — closure-encoding diagnosis |

`LOFT_VAR_TABLE=<fn substring>` is the companion to the IR dump, and NOT a `LOFT_LOG`
preset — it prints after `scopes::check` on every path:

```
[vartable] n_f — returned Enum(650, true, Deps { items: [1] })
[vartable]   0   n            int         scope=0  arg def OWNS
[vartable]   1   __retbuf     enum(650)   scope=4  def
[vartable]   3   e            enum(650)   scope=0  arg def OWNS
[vartable]   5   _mv_items_1  vec<int>    scope=4  def deps=[__retbuf(1)]
```

Reach for it whenever a borrow points somewhere that makes no sense.  The IR dump
names variables but never NUMBERS them, so a body reading `e` and a type dep printing
`__retbuf` read as one consistent story — which is what made loft#666 look like a
rename for two sessions.  Here the index is beside the name and each dep is resolved
to `name(index)`, so a code/table desync is visible instead of inferred.  The flags
answer the ownership question in the same line: `arg`, `def`, `skipfree`, `inlineref`,
and `OWNS` (the `Function::owns_store` verdict — the ONE predicate every consumer
reads; an element or a match binding must NOT show it).

---

## Database / Struct Debug Dumps in the Trace

Every opcode that produces or consumes a `DbRef` (struct, enum, or vector) shows a
compact inline dump of the pointed-to record in the execution trace.  The format is:

```
   8:[44] VarRef(var[32]=l) -> #3.1 { name: "diagonal", start: #2.1 { x: 1.5, y: 2.5 }, end_p: #1.1 { x: 10, y: 20 } }[44]
  65:[68] GetField(v1=ref(3,1,8)[56], fld=0) -> #3.1 { }[56]
```

**Reference prefix** `#store.record` — e.g. `#3.1` means store 3, record 1.
This tells you which allocation each struct lives in, making it easy to track
aliasing and double-free issues across opcodes.

**Depth limit** — nested structs expand up to depth 2 by default.  Deeper records
are shown as `{...}`:

```
#3.1 { inner: #5.7 { val: 42, nested: #6.2 {...} } }
```

**Element limit** — vectors show up to 8 elements by default, then `...N more`:

```
#4.3 [ #2.1 { x: 0 }, #2.2 { x: 1 }, ...6 more ]
```

**Depth limit at a vector** — if the depth limit is reached at a vector, shows
the element count instead of expanding: `#4.3 [10 items...]`

**Null fields are hidden** — fields holding the null sentinel are omitted, so a
freshly allocated struct with only one field set shows only that field.  This keeps
traces compact even for large structs.

### Tuning the dump limits

```bash
LOFT_DUMP_DEPTH=3    # expand up to 3 levels of nesting (default 2)
LOFT_DUMP_ELEMENTS=4 # show at most 4 vector elements (default 8)
```

These are read from the environment at runtime; no recompile needed.

### Accessing dumps directly via `cargo run`

When `LOFT_LOG` is set, `cargo run --bin loft` routes execution through
`execute_log` and writes the full trace (including struct dumps) to stderr:

```bash
LOFT_LOG=full  cargo run --bin loft -- myprog.loft 2>trace.txt
LOFT_LOG=minimal cargo run --bin loft -- myprog.loft 2>trace.txt
LOFT_DUMP_DEPTH=3 LOFT_LOG=full cargo run --bin loft -- myprog.loft 2>trace.txt
```

Without `LOFT_LOG`, the program runs without any trace output (production mode).

### Implementation

| File | Role |
|------|------|
| `src/database/mod.rs` | `DumpDb` struct — stores, depth/element limits, compact flag |
| `src/database/format.rs` | `Stores::dump_compact()`, `DumpDb::write()`, `write_struct()`, `write_list()` |
| `src/state/debug.rs` | `dump_limits()`, `dump_result()`, `dump_stack()` — calls `dump_compact()` for inline trace |
| `src/main.rs` | Routes `LOFT_LOG`-enabled runs through `execute_log` instead of `execute_argv` |

---

## Inspecting a Specific Function

### IR inspection with `LOFT_IR`

Set `LOFT_IR` to a function name (substring match) to see the parsed IR tree:

```bash
LOFT_IR=distance loft myprog.loft
```

Output:
```
=== IR: n_distance ===
{#block(1):integer
  [7] OpAddInt(OpMulInt(OpGetInt(p(0), 0), OpGetInt(p(0), 0)),
               OpMulInt(OpGetInt(p(0), 4), OpGetInt(p(0), 4)));
}#block(1):integer
===
```

The IR shows how the parser translated the loft source into internal operations.
Each `Op*` is a bytecode operator; `p(0)` is a variable reference; field offsets
(0, 4) correspond to struct field positions in bytes.

Use `LOFT_IR=*` to dump all user functions.

### Execution trace with `LOFT_LOG`

Set `LOFT_LOG` to trace bytecode execution step by step:

```bash
LOFT_LOG=full loft myprog.loft 2>trace.txt
```

Output (excerpt):
```
Execute main:
    0:[8] ReserveFrame(size=4)
    5:[48] Database(var[36], db_tp=48)
   10:[48] VarRef(var[36]=p) -> #1.1 { }[48]
   13:[60] ConstInt(val=3) -> 3[60]
   18:[64] SetInt(v1=ref(1,1,8)[48], fld=0, val=3[60])
   32:[48] VarRef(var[36]=p) -> #1.1 { x: 3, y: 4 }[48]
   35:[60] Call(d_nr=499, args_size=12, fn=n_distance)
 3487:[64] VarRef(var[48]=p) -> #1.1 { x: 3, y: 4 }[64]
 3490:[76] GetInt(v1=ref(1,1,8)[64], fld=0) -> 3[64]
 3499:[72] MulInt(v1=3[64], v2=3[68]) -> 9[64]
 3513:[72] AddInt(v1=9[64], v2=16[68]) -> 25[64]
 3514:[68] Return(ret=3566[60], value=4, discard=20) -> 25[48]
```

**Reading the trace:**
- `[48]` is the stack position in bytes
- `#1.1 { x: 3, y: 4 }` is an inline struct dump (store 1, record 1)
- `-> 25[64]` shows the result value and where it was pushed on the stack
- `Call(..., fn=n_distance)` shows function entry with the internal name
- `Return(...)` shows the function exit with the returned value

### Filtering by function name

Use `LOFT_LOG=fn:distance` to only trace execution inside `distance`:

```bash
LOFT_LOG=fn:distance loft myprog.loft 2>trace.txt
```

### Combining IR and trace

Both can be used together to see the IR at compile time and the execution at runtime:

```bash
LOFT_IR=distance LOFT_LOG=full loft myprog.loft 2>trace.txt
```

### Quick reference

| Variable | Value | What it shows |
|----------|-------|---------------|
| `LOFT_IR` | `distance` | IR tree for functions matching "distance" |
| `LOFT_IR` | `*` | IR tree for all user functions |
| `LOFT_LOG` | `full` | IR + bytecode + execution trace for all functions |
| `LOFT_LOG` | `minimal` | Execution trace only |
| `LOFT_LOG` | `static` | IR + bytecode only (no execution) |
| `LOFT_LOG` | `fn:distance` | Execution trace for `distance` only |
| `LOFT_LOG` | `crash_tail:50` | Last 50 execution steps before a crash |
| `LOFT_DUMP_DEPTH` | `3` | Struct nesting depth in dumps (default 2) |
| `LOFT_DUMP_ELEMENTS` | `4` | Max vector elements in dumps (default 8) |

---

## Fast iteration loop — `make iter`

For day-to-day "fix one bug, run one test" cycles:

```
make iter TEST=p197                        # all p197* tests
make iter TEST=p194 TFILE=issues           # only p194* in tests/issues.rs
make iter TEST=introspect TFILE=exit_codes # only introspect* in tests/exit_codes.rs
```

`make iter` runs `cargo test` filtered to `$(TEST)`, optionally
restricted to one test binary via `$(TFILE)`.  Defaults to the
**dev profile**, which is specifically tuned in `Cargo.toml`:

- `[profile.dev]` `opt-level = 1` (basic inlining)
- `[profile.dev.package.loft]` `debug-assertions = false`
  (skips the hot-path `Store::addr` / `keys::store` guards that
  add ~270x overhead to interpreter-heavy tests)

Measured here:

| Scenario | Dev profile | Release profile |
|---|---|---|
| Warm cache, no source change | ~0.3s | ~0.3s |
| Single-file edit, incremental rebuild | **~2.4s** | ~26.8s |
| Cold rebuild after `make clean` | ~30s | ~60s |

For most edits, dev profile is **~11x faster** on the inner
debug loop.  Tests that depend on release-only behaviour
(parallel timing windows, perf assertions) take `PROFILE=release`:

```
make iter TEST=par_throughput PROFILE=release
```

Sharing cache with `make test` / `make ci` (both release) means
switching profiles forces a one-time rebuild.  Within a single
debugging session, pick one and stay on it.

`make iter` cleans `tests/dumps/` and `tests/generated/` before
running — they pin per-test codegen output, and stale fixtures
across profile/test-set changes can produce bogus errors
(e.g. `attempt to add with overflow` from u16::MAX placeholder
positions).  This mirrors what `make test` already does.

### `mold` linker (committed default on Linux)

`.cargo/config.toml` activates `mold` on `x86_64-unknown-linux-gnu`.
Linker time is a small fraction of the rebuild (LLVM codegen
dominates), so the direct speedup is modest (~1s).  The bigger
win is a **unified cache**: every `cargo` invocation from this
checkout uses the same `RUSTFLAGS`, so alternating between
`cargo build`, `cargo test`, and `make iter` shares one cache
key.  Without this pin, ad-hoc `RUSTFLAGS=...` overrides force
rebuilds.

First-time setup on Linux x64: `sudo apt-get install mold`.
The CI workflow installs mold on its ubuntu runner.  macOS and
Windows ignore the config (different target triples) and use
their platform-native linkers.

---

## Boundary-matrix runner (`scripts/probe-matrix`)

The mechanics for CLAUDE.md § "Before fixing a non-trivial bug" — runs a
directory of probe cells uniformly and enforces the matrix-validity rules
as hard errors, so a matrix cannot silently measure nothing.

```bash
scripts/probe-matrix init /tmp/p_followups/mybug   # scaffold (template + control cell)
# … copy the template per cell, vary ONE dimension each, hand-write @EXPECT …
scripts/probe-matrix /tmp/p_followups/mybug                          # interp, fast iteration
scripts/probe-matrix /tmp/p_followups/mybug --backend both           # final verify (native pays rustc per cell)
scripts/probe-matrix /tmp/p_followups/mybug \
    --baseline .claude/worktrees/prechange/target/release/loft       # A/B classification
```

Each cell is a plain `.loft` program with header annotations:

| Annotation | Meaning |
|---|---|
| `// @EXPECT: <line>` | expected stdout line (repeat per line, exact match) — **mandatory**; hand-compute it BEFORE the first run, "two binaries agree" is not a pass |
| `// @EXPECT_LEAK` | a `stores not freed` warning is the expected outcome (red-documenting cells) |
| `// @CONTROL` | deliberately wrong expectation; the run errors unless this cell FAILS (proves the harness detects failure) |

The runner FAILS on: any non-control cell mismatch / crash / unexpected
leak, any cell with no stdout (**vacuous** — a parse error reads as silence,
not as green), any cell missing `@EXPECT`, and a missing or *passing*
control cell.  With `--baseline`, failing cells are labelled
`=> REGRESSION` (baseline passes) or `=> PRE-EXISTING` (baseline fails too)
— keep a main-tip worktree built for this (`git worktree add` +
`cargo build --release --bin loft` inside it).  Leak detection: interp
reads the exit warning; native runs under `LOFT_NATIVE_LEAK_CHECK=1`.

**The INSTALLED `loft` is a free before/after oracle.**  `$(which loft)` is
whatever `make install` last put there, so during a fix it is a ready-built
binary from before your edits — no worktree, no second build, and none of the
tree-destroying moves (`git stash`, `git checkout HEAD -- <file>`) the
[debugging policy](../../CLAUDE.md) forbids.  Three uses, all of which paid off
in the nested-narrow-width fix:

```bash
loft --interpret probe.loft            # baseline: is this failure mine or pre-existing?
./target/debug/loft --interpret probe.loft   # ... vs the working tree
loft --interpret tests/scripts/new-guard.loft   # must FAIL — proves a new guard isn't vacuous
```

Check its date first (`ls -l $(which loft)`) so you know which commit you are
comparing against; re-running `make install` mid-investigation destroys the
baseline.

Graduate the cells that earn guarantees into `tests/scripts/` when the fix
lands (protocol step 7) — e.g. `302-vector-buffer-delivery.loft` /
`303-ref-reassign-free.loft` are graduated matrices.

## Introspection CLI (`--introspect`)

`loft --introspect <file>` packages the dump primitives behind one
flag, dumping bytecode + generated Rust + slot tables + per-fn type
tables to stdout (or per-section files).  No env vars, no test
harness.  Use it when you want to inspect compile-time state without
running the program.

### Sections

| Flag | Output | When to use |
|------|--------|-------------|
| `--show-bytecode` | Bytecode disassembly per fn | Codegen bugs, "is the right opcode emitted?" |
| `--show-rust` | Generated Rust (`--native-emit` shape) | Native-codegen bugs, rustc errors |
| `--show-slots` | Stack-slot table per fn (name, type, scope, slot, live interval) | Slot conflicts, lifetime bugs |
| `--show-types` | Per-fn variable type + dep table | **Dep-tracking bugs** — see below |
| `--show-ownership` | Per-binding store ownership (@PLN103): `Owned` / `Borrowed(base=X)` (a live alias of X — the dangerous case) / `Owned (backing=…)` (owns via a delivery buffer) / `Join(base=X)` (a runtime owned-or-borrow split) / `Borrowed(caller-arg)` / `— (scalar)`, plus a per-return `delivery:` line (materialised / owned / borrows). **Also flags the loft#568 interpreter-orphan class STATICALLY** — `⚠ loft#568: owned text returned by value (…) — no &text retbuf` on a text return backed frame-locally with no delivery buffer (the interpreter orphans the `String`; native RAII drops it). Opt-in. | **Store-lifetime / owned-vs-borrowed bugs AND suspected text-return leaks** — reach for this before ASan: it names the leaker class without a sanitizer run. Verdict is backend-shared. |
| `--show-resolution` | Which names each source can SEE: one row per source with its `defined` and `visible` counts, then every import alias, and the `context:` line (stdlib dir + `--lib` paths). Opt-in. | **"Unknown function" / "Library not found" on a name that should resolve** — see below |
| `--why <name>` | The same section narrowed to one name: where it is defined, and every source it is reachable from. Implies `--show-resolution`. | The same, when you already know which name is missing |
| `--bc-roundtrip` | Re-assemble each fn's bytecode from its own dump and compare (`ok`/`DIFFERS`) | Verify the dump is a faithful, editable bytecode representation — see [Bytecode round-trip](#bytecode-round-trip---bc-roundtrip) |
| `--json` (INSP.J) | One machine-readable JSON object over the included sections — a string field per section (`bytecode`/`rust`/`slots`/`types`, plus `ownership` / `resolution` if requested), in canonical order | An editor / agent / the LSP that wants a section by key instead of splitting on `=== header ===` lines; takes precedence over `*-out` / `--diff` |

Combine the four dump flags freely; they emit in fixed order, and
no flags = all four.  `--bc-roundtrip` is **opt-in only** (a
verification check, not a dump — it never runs in the no-flags
default).  `--all-fns` includes the default/* stdlib.  `--fn
<name>` filters to one function.  `--json` renders whichever
sections are selected as one JSON object (parseable by loft's own
`json` reader) instead of the text dump.

### `--show-resolution` when a name will not resolve

A name resolves only if the source you are calling it from can **see** it. loft
numbers sources: `0` is the standard library, `1` is your program, `2` and up are
the libraries it `use`s. A definition is visible in its own source; a `use` adds an
**alias**, so the name is also visible in the source that imported it.

When `Unknown function foo` or `Library 'bar' not found` appears for a name you
believe is there, this section shows which of those steps did not happen:

```sh
loft introspect prog.loft --show-resolution --lib lib/
```

```
context: stdlib="…/default"  lib_dirs=["…/lib"]
sources:
  0    defined 650    visible 650    std (…/default/01_code.loft)
  1    defined 1      visible 2      …/prog.loft
  2    defined 1      visible 1      geom (…/lib/geom.loft)
aliases (1 import binding):
  src 1    <- src 2    #650    n_hex_distance
```

Read it in three steps:

1. **`context:`** — the paths this run searched. `lib_dirs=[]` when you passed
   `--lib` means the flag never reached the session, so no library could load. That
   is a whole class of bug, visible without running the program.
2. **`defined` vs `visible`** — `defined` counts the source's own definitions,
   `visible` counts every name it can reach. Source 1 above defines 1 and sees 2:
   the extra one is the import.
3. **`aliases`** — one line per imported name. `src 1 <- src 2` reads *"source 1
   can see this because source 2 defines it"*. An **empty** list in a program that
   has a `use` means the import never took effect.

To ask about one name instead of reading the table:

```sh
loft introspect prog.loft --why hex_distance --lib lib/
```

```
`hex_distance` is #650, defined in source 2
  visible in source 1 (import alias)
  visible in source 2 (its own)
```

`is not defined in any source` means the library was never parsed — check
`context:` first. Listed as defined but **not** visible from source 1 means the
`use` is missing or did not apply.

### `--show-types` for dep-tracking bugs

The `--show-types` section renders each variable's full type via
`Type::show()`, including the dependency suffix (`text["a"]` =
text borrowed from `a`).  Designed to surface dep-propagation
bugs at a glance — exactly the shape that hid P197 (a `text`
element from a tuple struct field that should have carried the
host as a dep but didn't).

```
fn n_first -> text["a"]:
  #    arg  name                     type [deps]
  ----------------------------------------------------------------------
  0         a                        ref(A)
  1    arg  s                        &text
```

Compare the function's return-type deps against what you expect.
If a returned `text` should track a host but the table shows
plain `text` (no `[host]` suffix), the dep was lost in
`get_val::Type::Tuple`, `field()`'s `t.depending(*nr)`, or
`Type::depending`'s recursion.

#### `--trace` — per-expression tape

Add `--trace` to surface the type at *every* chaining step, not
just the final variable.  Critical for nested expressions where
one intermediate step might lose a dep:

```
$ loft --introspect --show-types --trace foo.loft

fn n_first -> text["a"]:
  #    arg  name                     type [deps]
  ----------------------------------------------------------------------
  0    arg  a                        ref(A)

  trace (per-expression types):
    4:7        ref(A)["a"]
    4:9        (text["a"], text["a"])  ← `.v` step
    5:2        text["a"]                ← `.0` step
```

The two-step tape makes the dep flow visible: `a` → `a.v` →
`a.v.0`, with each step carrying `["a"]`.  Before the P197 fix,
the `.v` step would have rendered `(text, text)` (no `["a"]`)
and the regression would have been obvious without reading any
code.

Implemented as a `Parser::trace_types` flag; `parse_part` calls
`record_type_trace(&t)` after each `.field`/`.tuple_idx`/`[idx]`/
`(args)` chaining step.  Position is the lexer's char-offset
within the line (so `5:2` means line 5, byte 2 of the source).

### `--diff <baseline>`: did my parser tweak change anything?

Capture once, edit, re-run with `--diff`.  Mirrors `diff -u`'s
exit codes (0 identical, 1 differs).

```bash
loft --introspect --show-bytecode myprog.loft > before.bc
# edit the parser
loft --introspect --show-bytecode --diff before.bc myprog.loft
```

Per-section `--*-out` redirects still write to their files;
`--diff` only covers stdout-bound sections.

### Labelled jump targets in the bytecode dump

`--show-bytecode` anchors every jumped-to offset with a `:POS<rel>`
label and rewrites each goto to reference it, so the dump reads as
editable labelled assembly instead of raw byte offsets:

```
 28[48]: GotoFalseWord(jump=:POS46, if_false: boolean)
 43[40]: GotoWord(jump=:POS58)
:POS46
 46[40]: ConstInt(val=2) -> integer var=r[16]:integer
...
127[40]: GotoWord(jump=:POS62)   ← backward loop edge, binds to the label
:POS130
```

Jumps bind to a label *identity*, not a byte offset, so inserting or
removing ops shifts no jumps.  The label `<rel>` is the target's
relative offset within the function (`collect_jump_targets` /
`instruction_len` in `src/compile.rs` — `instruction_len` decodes
each op's real length, so variable-length `ConstText`/`Iterate`
operands advance correctly).

### Bytecode round-trip (`--bc-roundtrip`)

`loft --introspect --bc-roundtrip <file>` dumps each function's
bytecode, re-assembles it from that text via
`compile::reassemble_function` (the inverse of the disassembler),
and compares to the original byte stream — reporting `ok` /
`DIFFERS` / `error` per function plus a tally.

```bash
loft --introspect --bc-roundtrip --all-fns myprog.loft
#   ok      n_classify  (139 bytes)
#   ok      n_main      (95 bytes)
#   ── 201 identical, 0 differing/error ──
```

A clean run proves the labelled dump is a **faithful, editable
representation of the bytecode** — every byte is recoverable from
the text.  Constants encode inline (`ConstText` carries its escaped
string); jumps resolve from `:POS` labels; call targets dump as the
function *name* (`fn=n_classify`, relocation-safe) and static calls
as the native name, both resolved back to offsets on re-assembly.

**Why it's a tool, not just a test** — it's the front half of an
"edit bytecode *outside the parser*" loop: dump a function, change
an op / a constant / a jump / drop in a free, re-assemble, and the
round-trip confirms it's well-formed.  For any **stack-neutral**
tweak that is a real way to ask "what does *this exact* bytecode
do?" without going through the parser.

**Limits** (honest boundaries of the edit workflow):
- *Stack-neutral edits* (swap an op, change a constant, redirect a
  jump, add a free) re-assemble correctly — slot positions and the
  `Return` discard are unchanged.
- *Stack-depth or local-set changes* do **not** round-trip a hand
  edit: var slots are stack-relative (`pos = stack − slot`) and
  `Return(…, discard=N)` is the frame size, both of which shift.
  Re-deriving them needs the slot/layout pass (`scopes.rs`), not a
  text edit.
- The last 20% — **splice-and-run** (append the re-assembled
  function to the code array, repoint `code_position` + caller `to`,
  execute) — is **not built**.  Relative gotos make a single
  function relocatable, so it's a small, self-contained add when the
  need is real.

Implementation: `compile::reassemble_function` + `escape_text` /
`unescape_text` (`src/compile.rs`); the `Roundtrip` section in
`src/introspect.rs`.

### Native-codegen source map

The `--show-rust` (and any `--native` compilation) emits
`// loft:<file>:<line>` comments above each function header and
each statement.  `rustc` errors on `/tmp/loft_native.rs:1450` map
back to a .loft line by reading the nearest preceding comment.

```rust
// loft:/tmp/myprog.loft:7
fn n_first(stores: &mut Stores, mut var_a: DbRef) -> Str {
  ...
  // loft:/tmp/myprog.loft:8
  return Str::new(...)
}
```

When rustc reports a borrow-check error or type mismatch, scroll
upward in the generated file from the error line to the nearest
`// loft:` comment — that's the source line under suspicion.

---

## Debugging a Parse Error or Wrong IR

1. Add `LOFT_LOG=static` and run the failing test.
2. In the output, find the function that contains the wrong code.
3. Compare the emitted IR (`Value` tree) against what you expect.
4. If the IR is wrong: the bug is in the parser. Search for the relevant `Value`
   variant in `src/parser/` and trace through `parse_single` or `parse_operators`.
5. If the IR is correct but the bytecode is wrong: the bug is in `src/state/codegen.rs`,
   in the `value_code` branch for the relevant `Value` variant.

---

## Debugging a Runtime Crash or Wrong Result

1. Reproduce with the smallest possible loft program (isolate to a single function).
2. Add `LOFT_LOG=minimal` and run. Find the last opcode executed before the crash or
   wrong result.
3. If the opcode is a memory access (`set_int`, `get_int`, `set_long`, etc.) and the
   `store_nr` is a large or unexpected value (like 60 or 0x3C), the DbRef on the
   stack is garbage — the bug is in scope analysis or codegen, not in the opcode.
   Switch to `LOFT_LOG=ref_debug` to find where the bad DbRef was created.
4. If the opcode itself is wrong (wrong opcode for the operation), check
   `src/state/codegen.rs` and the `Stack::operator` delta table in `src/stack.rs`.

### When the crash will not repeat — the crash report file

On SIGSEGV / SIGABRT / SIGBUS, `src/crash_report.rs` prints the last opcode, its
bytecode position, the function, and the loft source line. It writes that to
stderr **and to a file**, because a build that pipes stderr through a filter
otherwise discards the one diagnostic that cannot be regenerated: the run that
produced it is by definition the run that will not repeat (loft#717 lost exactly
this, and all that survived the pipe was the header line).

| | |
|---|---|
| Default location | `.loft/loft-crash-<pid>.txt` when a `.loft/` directory already exists (the `loft test` case), else `<tmp>/loft-crash-<pid>.txt` |
| Override | `LOFT_CRASH_FILE=<path>` |
| Turn it off | `LOFT_CRASH_FILE=` (empty) — stderr only |

Nothing is written unless a crash actually fires, and the directory is never
created (a run that does not crash leaves no trace). The stderr report names the
file it wrote, on a line after the diagnostic — so if you see the report but no
such line, the write failed and stderr is all there is.

Reading it back: the `pc:` value is a bytecode position, which
`LOFT_LOG=static` maps to source; `fn:` and `at:` are usually enough on their
own. If the report says `(none — crash outside interpreter)`, the fault was not
in an opcode — look at `--native` code or a library call instead.

### An unexplained SIGSEGV in a package: suspect the auto-built cdylib

`--interpret` does **not** mean no native code ran. A library package with
`[library] compile = "native"` is auto-compiled to `<pkg>/native-auto/*.so` and
dispatched into even when the script interprets, so `gdb bt` on the core belongs
before any interpreter theory.

The generated cdylib hardcodes type-table **indices** and field **offsets**, so
it is valid only against the exact type table it was generated from. Two
defences keep it honest, and they fail differently:

- The artifact's FILENAME carries the caller's type-layout fingerprint (#715), so
  two contexts never name the same file.
- The artifact also DECLARES the layout it was built for
  (`loft_type_layout_fp_v1`), and the adopter verifies it before use (loft#717).
  A mismatch rebuilds rather than dispatching.

The second exists because the first is an argument, not a check: it holds only
while the fingerprint keeps covering every layout difference and while nothing
else can put a file at that name. When an argument like that fails, the artifact
is not slightly wrong — it resolves indices against a foreign table, so reads
land at wrong offsets and the crash surfaces arbitrarily far from the cause.
That is why the verification is worth a `dlopen`: the failure it prevents is
unattributable by construction.

Suspect this whenever a crash is intermittent, appears under a parallel sweep,
and does not repeat afterwards — the artifact is rebuilt on the next run, so the
evidence deletes itself. `rm -rf <pkg>/native-auto` and re-running is not a
diagnosis; it is destroying the only copy of the thing that crashed.

---

## Before you believe a fault is RANDOM

A fault that reproduces some runs and not others is usually not random — it is a
run that starts from different state than you think. Check that *first*, because
"random" is expensive: it sends you to repeat-run harnesses and mechanism traces
when a one-line probe would have settled it.

**The tell is the ratio itself.** `1/12` or `1/20` is the signature of *the first
run differs*, not of randomness — a genuinely random fault lands on a ratio that
moves when you re-measure. Read the *sequence*, not the count.

Two probes, both cheap, before any theory:

```bash
# 1. Is it ordered?  A cold/warm split reads ok, BAD, BAD, BAD — not scattered.
rm -rf ~/.cache/loft/program-*
for i in 1 2 3 4; do loft --native p.loft; done

# 2. Turn the suspected state off.  If the fault vanishes, you have located it.
LOFT_NO_CACHE=1 loft --native p.loft
```

**State a run inherits, none of it cleared by removing `.loft`:** the
whole-program bundle in `$XDG_CACHE_HOME/loft` (`cache::program_cache_paths`,
default-ON — `LOFT_NO_CACHE` disables), the stdlib bundle
(`LOFT_STDLIB_CACHE`), `target/` build artefacts, and an installed
`$(which loft)` on `PATH`.

This is not hypothetical: the native duplicate-type-mint fault
([plans/native-type-mint](plans/native-type-mint/README.md)) was recorded as
"per-process random" for a whole session and a harness was built around that
belief. Its `repeat.sh` cleared the per-directory `.loft` cache before every run
but never touched the program bundle, so it measured
cold-once-then-warm-forever and reported it as 1-in-12. The real fault was
deterministic: warm load, every time.

**So the rule is about the instrument, not the bug.** Before theorising about a
random fault, prove each run really starts from the state you believe. A harness
that clears the wrong cache reports a ratio with total confidence, and the ratio
is fiction.

## When it fails in CI but passes locally

Same commit, same command, opposite result — so the difference is *state*, and the
suspect list is short. A build tree accumulates artefacts CI never has, and one of
them can supply exactly the thing the code fails to find, which turns a real bug
into "works on my machine" indefinitely.

Check, before re-running anything:

```bash
git status --ignored --short target/ | head     # stray artefacts in the build tree
find target -maxdepth 3 -type l                 # symlinks — the quiet ones
```

The worked example: the nightly ASan sweep failed on both runners while the
identical `cargo +nightly nextest … -Zsanitizer=address --target x86_64-…` command
passed locally, including the full 1667-test sweep. The reason was a
`target/x86_64-unknown-linux-gnu/release/default` **symlink to the repo's stdlib**,
left in the tree weeks earlier. It papered over a real defect — `project_dir()`
could not resolve the project root for a `--target` build ([INTERNALS.md §
`project_dir`](INTERNALS.md#project_dir), loft-lang/loft#638) — and it made the
first three hypotheses *unfalsifiable*, because every local control ran against a
tree where the missing thing was present.

Two rules follow:

- **Reproduce by construction, not by re-running.** Build the layout the failing
  environment has (here: copy the binary into a synthetic `<root>/target/<triple>/release/`)
  and vary ONE axis. That produced the CI failure line character-for-character in
  seconds, after ~20 minutes of full-sweep runs had proven nothing.
- **When a local control passes, ask what the local tree is supplying.** A green
  control is only evidence if you know the environment it ran in — otherwise you
  have measured the symlink, not the code.

## Debugging store-ownership bugs (leaks, double-frees, non-determinism)

The word-addressed `Store` arena (`Vec<u64>`) is **invisible to valgrind** —
the buffer is validly allocated, so corruption *within* it (a stale `DbRef`
read, a record reused while still referenced, a length read before it is
written) shows up only as a wrong or **non-deterministic** result, never as a
valgrind error.  `claim()` does NOT zero reclaimed slack, and a freed
tree-tracked block stores its LLRB free-list pointers at **offset 4 — exactly
where a vector's length word lives**.  This family (`@P311`, `@P313`, `@P314`,
`@P317`) is the hardest to pin; these levers cut the time dramatically:

| Lever | What it does | Use when |
|---|---|---|
| `LOFT_STORE_GUARD=1` | Reports each block-confined vector store that is scoped (and freed) later than the block it is confined to — the lifetime model under-freeing (Goal E).  Read-only, off by default.  Confinement is the least-common-ancestor of every reference's scope-path, with escape exclusions (return/yield/break, block-result, tuple-element, dep-aliasing) and loop-internal reuse excluded — adversarially hardened by `plans/2-vector-store-watermark/probes/cluster-I/`. | "Does a program hold more heap than the source implies?"  Drive the store-lifetime fix until it is silent corpus-wide, then promote to a `debug_assertions` assert.  See [GOALS.md Goal E](GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth). |
| `LOFT_LOG=zero_claim` (or `LOFT_ZERO_CLAIM=1`) | Zeroes every freshly-claimed record's payload, so a read-before-write / stale read returns a deterministic `0` instead of arena garbage. | A result is **non-deterministic** run-to-run.  If `zero_claim` makes it deterministic-and-correct → a read-before-write (fix: zero that record at its claim site).  If it stays non-deterministic → NOT a claimed-slack read (rule it out; suspect a deep-copy logic bug or addresses-as-data). |
| `LOFT_LOG=poison_free` | Overwrites a store's buffer with `0xDEADBEEF` on free. | Suspected use-after-free of a *whole store*.  No effect ⇒ not a freed-store UAF. |
| `LOFT_UAF_GEN=1` (+ `LOFT_UAF_SRC=1`) | Detector (c): stamps every DbRef pushed on the operand stack with its store's generation and reports at the matching pop when the slot was freed since.  `_SRC` adds the freeing pc + op. | The freed-then-**reused** read `LOFT_POISON` is blind to (the new occupant is live, so the bytes look fine).  **Scope: only the window between a push and its pop** — a ref that goes stale sitting in a FRAME slot is invisible to it, which is why it never saw loft#723. |
| `LOFT_UAF_GEN_INJECT=1` | Ages every ref just after its push stamps it, so each is stale while live.  The positive control for the row above. | Before believing a silent `LOFT_UAF_GEN` run.  A detector that cannot fire and a clean corpus look identical; under injection ~471/548 corpus scripts report, against 0 without it. |
| `LOFT_NO_SLOT_REUSE=1` **+** `LOFT_POISON=1` | No freed slot is ever reclaimed, so a freed store stays freed *and* poisoned. | **Ground truth for "is this reported stale read real?"** — with reuse off, a genuine stale read must land on `0xDEADBEEF`.  Clean + correct here ⇒ the report is a false positive.  This is what convicted the `LOFT_UAF_GEN` offset-keyed-stamp bug. |
| `LOFT_STORES=log` | Per-alloc/free trace (`+ alloc #N`, `- free #N`). | Find a `free` then `alloc` of the same store while a `DbRef` is still live.  Note: a store is logged under the var name at *free* time, which may differ from its *alloc* name. |
| `LOFT_STORES=warn` | Warns when >30 stores are active. | Catch a runaway leak early.  **Note (@PLN103):** this OVER-warns — a large *working set* (many concurrently-live stores, all freed) trips it though it is not a leak.  Prefer `=timeline` to disambiguate. |
| `LOFT_STORES=timeline` (@PLN103) | Per-store lifeline with a STABLE id `#<store_nr>.<seq>` (the `seq` disambiguates the reused `store_nr` slot, so a `free` prints the same id as its `alloc`), plus an exit SUMMARY: `<allocs>, <frees>, peak <N> concurrently-live (working set)` reconciled with the authoritative leak count. Both backends. | The working-set-vs-leak question `=warn` can't answer: a high `peak` with `NO leak` is a big-but-clean working set; a real leak reports `N user store(s) LEAKED`.  Also: match a freed-then-reused slot by its `.seq`. |
| `LOFT_TEXT_TIMELINE` (@PLN104) | The **text-buffer** analogue of `LOFT_STORES=timeline` — text values are Rust `String`s on the stack frame, so their heap allocation is INVISIBLE to the store timeline / `check_store_leaks`. Any value → exit SUMMARY (`<allocs>, <frees>, peak <N> bytes live`) + a `LEAKED #seq fn=<d_nr> <bytes> <content>` line for every `String` still live at exit; `=timeline` adds a per-op `grow`/`free` lifeline. Interp only (native RAII frees text). Realloc-safe (capacity delta by ptr). | **The loft#568 owned-text-return orphan class** — the leak `LOFT_STORES=timeline` is blind to and the ASan `ir_read` suppression measures UNRELIABLY (stack-substring, false-pos/negs by `malloc_context_size`). This is DETERMINISTIC and names the leaking fn + content. Reach for it first on a suspected text-return leak. |
| `LOFT_TRACE_DB=1` | Every `OpDatabase` call with var, type, current DbRef. | Pin cross-iter slot dangling (a slot's stale DbRef gets `clear+claim`'d, clobbering another var's record).  Added during PLAN51 Cluster II diagnosis. |
| `LOFT_TRACE_CR=1` | Every interp `OpCopyRecord` with src+dst + Canvas field reads BEFORE and AFTER copy. | Pin same-store copy corruption (`remove_claims` frees nested vec records before `copy_block` reads them) or wrong-source mid-copy.  Added during PLAN51 Cluster II diagnosis. |
| `LOFT_TRACE_LEX=1` (#625) | The lexer's POSITION bookkeeping: every recorded identifier position (`idpos`), every `to()` seek, every `revert`, every memory `replay`. | **A diagnostic naming the wrong LINE.** The reporting cursor is shared and long-lived: any warning pass may seek it BACKWARDS to point at an earlier site, and `to()` moves only that cursor — the tokenizer keeps counting lines from wherever it was left, so an unrestored seek shifts every LATER diagnostic and the symptom surfaces in an unrelated message. Run it and **diff the two passes**: the pass that records a token at the wrong line names the seek just before it. |
| `LOFT_TRACE_SCHEMA=1` (#618) | Every `Stores` type registration and rollback, plus the **DEF** behind each (`fill d_nr=… name=… -> reg=…`). | **"Double structure type" aborts**, and any suspicion that a speculative parse (REPL capture, `infer_type`) is not schema-neutral. The abort names only the colliding type; the fault is normally ONE def filled twice (a rolled-back parse re-creating it), which is visible only as the same `d_nr` registering a bare name and then a `src0::`-qualified one. |
| `LOFT_TRACE_COPY=1` | Native-side OpCopyRecord trace (src, dst, size, free_src). | Companion to `LOFT_TRACE_CR` for native; pin schema-mismatch copies (compile-side layout vs runtime-side layout disagree). |
| `LOFT_TRACE_FINISH=1` | Every `finish_type` entry/exit for tuple types (size, align, field_groups count). | Pin tuple-schema propagation gaps (compiler side has groups, runtime side doesn't → wrong size).  Added during PLAN51 V-a diagnosis. |
| `LOFT_KEEP_NATIVE_RS=1` | Preserves the generated Rust at `/tmp/loft_native_*.rs` instead of cleaning it. | Read the generated Rust at a specific line a runtime panic cites.  Added during PLAN51 V-c diagnosis. |
| `check_store_leaks` (interp, **`--interpret` only** — see note) / `LOFT_NATIVE_LEAK_CHECK=1` (native) | At-exit summary of unfreed stores, **aggregated by type** (`kt=68 ChunkKey×6026`). | Pin *which type* leaks.  Run the **same** repro on both backends — a leak on one and not the other means a backend-specific free emission bug (the @P317 symptom-2 shape). |
| `--native-emit out.rs` | Writes the generated Rust and exits. | A native-only bug.  Read the generated function: look for a `null_named(...)` placeholder that is overwritten without a free, or a missing/extra `OpFreeRef`. |
| `"Allocating a used store #N (known_type=…, requested by=…)"` panic (`allocation.rs:104`) | The store-pool tripwire (free-bitmap vs `store.free` disagree), now with slot + type + requester. | Fires at the *next* allocation after the real over-free/leak — a tripwire, not the bug site.  The pool near `u16::MAX` ⇒ a leak exhausted the pool and `max` wrapped to 0; otherwise a double-free. |

> **Gotcha — the interpreter leak check needs `--interpret`.** Bare `loft prog.loft` runs the
> **default `--native` mode** (`main.rs` `native_mode = true`): it compiles + runs the native binary
> and exits via the subprocess status BEFORE the interpreter's `check_store_leaks` is reached, so a
> bare run prints NO leak warning even on a real interpreter leak. Always leak-check with
> `loft --interpret prog.loft` (or rely on the test harness, which is interpreter-based:
> `tests/leak.rs::leaks_for`, `loft_suite`). Native leak-checking is the separate
> `LOFT_NATIVE_LEAK_CHECK=1` axis. Note also that some leaks are interpreter-only: the eager
> `OpInitRef` null-init allocates a store in the interpreter but native lowers it to `DbRef::NULL`
> (no allocation), so an interpreter `kt=65535` leak can be genuinely absent under native.

Workflow: reproduce minimally, run on **both** backends (divergence localises
the backend), use `zero_claim` to classify the non-determinism, then `--native-emit`
+ `LOFT_STORES=log` to pin the site.  Mirror the @P311/@P313 fix shape (a
missing/spurious `0x8000` free-source bit or a `null_named`-vs-sentinel
choice in `src/generation/dispatch.rs::emit_null_dbref`).

---

## The debug-assertions calibration run (`target-da`)

**Every lib-side `debug_assert!` / `#[cfg(debug_assertions)]` check is compiled
OUT of every ordinary build** — dev, test, AND `--release` —
by `[profile.dev.package.loft] debug-assertions = false` (dev/test) and the
release profile default.  That covers the H5 two-pass contract
(`assert_pass2_def_attr_stable`), `Store::valid`/`Store::validate`, the
`keys.rs`/`store.rs` boundary guards, codegen sanity asserts
(`generate_set`/`generate_call`), the `get_stack` corrupt-DbRef guard, and the
`[set_var]` width warnings.  The only standing build that checks them is the
cargo-fuzz target.  So for any claim guarded by a debug assert, "the suite is
green" is a **calibration failure** — the instrument is not installed in that
build.  The first-ever full calibration (2026-07-03, @PLN85) found four
long-latent H5 producers plus a latent-assert inventory; the open cells live in
`plans/85-store-lifetime-retirement/fuzz-proof-gate.md` § final honest DA map.

Run the calibration in a **separate target dir** (one-time ~full rebuild,
then incremental):

```bash
# one-time: the CLI resolves the stdlib relative to the exe, and
# project_dir() hardcodes target/release|debug — non-standard dirs miss it
ln -sfn ../../default target-da/release/default

RUSTFLAGS="-C debug-assertions=on" CARGO_TARGET_DIR=target-da \
  cargo test --release --no-fail-fast
```

**Never set `RUSTFLAGS` against the MAIN target dir.**  Cargo keeps BOTH
flag-generations of every dep in `target/release/deps/` — including two
`libloft_ffi-*.rlib` — and anything that sweeps `deps/` (the cdylib
auto-builds pass `--extern` for every rlib there) then dies on
`colliding StableCrateId values`, while `loft-ffi`'s fingerprint change
invalidates every cached cdylib.  Recovery, in order: `cargo clean --release`
→ full `cargo build --release` → `make rebuild-native-cdylibs` → rebuild the
registry graphics cdylib (`cd ~/.loft/registry/graphics-*/native && cargo
build --release`) → clear `~/.loft/build-cache` and any failing package's
`native-auto/` → rebuild the wasm rlib (the `html_wasm` staleness guard
checks it against source mtimes).

**Prevent + auto-heal it.**  Run sanitizer/nightly builds through
**`scripts/asan.sh`**, which sets `CARGO_TARGET_DIR=target/asan` so nightly
artifacts never land in the shared `target/` — the pollution simply cannot
happen.  If a stray nightly build already polluted it (E0514 "incompatible
version of rustc" on the native tests), **`find_problems.sh` self-heals**: its
`ffi_toolchain_guard` reads the rustc version embedded in each
`libloft_ffi-*.rlib`, and when it differs from the active `rustc` it deletes the
stale rlib so the next build recompiles it — turning the silent, mtime-immune
E0514 into an automatic rebuild.

Reading the results: CLI-spawning tests fail en masse if the stdlib symlink
is missing (lens artifact, not a finding); confirm any surprising cell
against an `origin/main` control build in a throwaway worktree before
calling it a regression — most DA findings are long-latent, first seen the
day the assert is first *checked*, not the day it was written.

---

## Debugging a validate_slots Panic

`validate_slots` panics in debug builds when two variables with overlapping live
intervals share the same stack slot. The panic message includes both variable names,
their slot range, and their live intervals.

1. Identify which function and which two variables conflict.
2. Add a minimal reproducer to `tests/slot_assign.rs`.
3. Check whether the live intervals truly overlap (can both variables be live at the
   same time?) or whether `compute_intervals` is computing a conservatively wide range.
4. If the overlap is real: a bug in scope analysis assigned the same slot to two
   simultaneously-live variables. Check `scopes.rs::copy_variable`.
5. If the overlap is spurious (a sequential block reuse): the exemption in
   `find_conflict` may need to be extended.

---

## Debugging a Tricky Compiler Bug (use logging first)

For non-obvious bugs — wrong use counts, unexpected variable lifetimes, closure leaks,
dead-assignment warnings that fire or don't fire — **always add targeted debug logging
before attempting a fix**.

Reasoning alone about multi-pass parser/compiler state is unreliable; logging shows
exactly what is happening.

Pattern:
1. Add `eprintln!` to the tracking function closest to the symptom (e.g. `in_use`,
   `track_write`, slot-assignment helpers).
2. Run the failing test and read the output to confirm your hypothesis.
3. If the call site is still unclear, add `std::backtrace::Backtrace::capture()` at the
   suspicious point and print it. This pinpoints the exact source location.
4. Fix the root cause, then **remove all debug prints before committing**.

Example: when investigating why a dead-assignment warning stopped firing, adding
`eprintln!` to `in_use` and `track_write` immediately revealed an extra `uses` increment
from a captured variable re-read, and the backtrace pointed to the exact `parse_var` call.

### When interp codegen is opaque or hangs — read the native-generated Rust

`LOFT_LOG=static` shows the bytecode, but a *codegen* bug (wrong channel, missing
cast, a termination test that never fires) is usually easier to see as Rust than as
bytecode — and if codegen itself loops, there is no complete bytecode to read.

`loft --check <file>` runs the **native** backend, which emits readable Rust and stops
at the `rustc` diagnostics. The generated source persists at `loft_native_*.rs` in the
build temp dir (`$LOFT_TMPDIR`, default the system temp; the path is printed in any
`E0xxx`). It encodes the same for-loop / yield / dispatch logic the interpreter
compiles to bytecode, but as named-variable Rust — so a type mismatch, a wrong
sentinel, or a doomed loop condition is visible directly.

Reach for this especially when a process **hangs**: `gdb` attach (`ptrace_scope`) and
`perf` (`perf_event_paranoid=4`) are both blocked in this sandbox, so you cannot
backtrace or sample a live hang. The generated Rust — or env-gated counter-panics
(§ above) — is the substitute.

Worked example (#401): an `iterator<float>` for-loop hung the interpreter at codegen.
The native `.rs` showed the loop as `let var_x: f64 = coroutine_next_i64(..); if
!var_x.is_nan() { … } break;` — a NaN value-sentinel termination reading an i64
channel — which exposed the root (the value-sentinel only terminates when the
element type's null sentinel matches the i64 transport's) in one read, after gdb/perf
and hand-placed counters had all stalled. Confirm the mechanism this way **before**
editing: a "fix" applied to a hypothesis (there, a guessed return-type change) was a
no-op that cost a rebuild.

---

## Debugging a Scope Analysis Bug

Scope analysis bugs are the hardest to diagnose. The gap between the wrong IR
insertion and the runtime crash is large.

Strategy:
1. Use `LOFT_LOG=ref_debug` to capture all allocation and free events.
2. Look for a `free` event on a DbRef whose `store_nr` does not match any live
   allocation — that is the double-free or wrong-store free.
3. Search backwards in the log for the `alloc` event for that DbRef. The function and
   variable name tell you where the wrong free was inserted.
4. In `src/scopes.rs`, find the `get_free_vars` or `exit_scope` call that produced
   the wrong `OpFreeRef` / `OpFreeText`, and fix the scope assignment for that variable.

---

## Using the Test Framework for Quick Iteration

The `code!` and `expr!` macros in `tests/testing.rs` let you write a loft program
inline in a Rust test:

```rust
#[test]
fn my_feature() {
    expr!("my_expr_result").result(Value::Int(42)).run();
    code!("fn main() { assert(1 + 1 == 2, \"math\"); }").run();
}
```

Use `.error("expected error message")` to assert on compile-time diagnostics.
Use `.warning("expected warning")` for non-fatal diagnostics.

For end-to-end tests on `.loft` files, add to `tests/docs/` and the `wrap.rs`
runner will pick it up automatically.

---

## Two false failures that look exactly like real ones

Both waste a bisect if you take them at face value.  The rule underneath is the same:
**a surprising red is a suspect environment before it is a suspect commit** — check the
cheap environmental cause first, because each of these mimics a deterministic bug.

**A stale server process on the port.**  A networked test
(`tests/engine_host_connector.rs`, the `eh_*` family) failed in `make ci`, then failed
standalone three runs in a row at an *identical* 15.80s, and it post-dated a green run
— every tell of a real regression.  It was two leftover servers under
`target/test-tmp/.loft/cache/` holding the ports; the test then waits out its deadline.
Killing them made it pass in 3s.  So before bisecting one of these:

```bash
pgrep -af "target/test-tmp/.loft/cache/eh_"      # stale servers from an earlier run
```

**Identical timing across runs is the tell** — that is a deadline expiring, not logic
failing.  Real logic bugs vary by a few ms; a deadline does not.

**The installed binary is not always a "before" oracle.**  `$(which loft)` is only a
pre-change reference if it was installed BEFORE the change.  A session used it to
conclude a consumer-reported fault was pre-existing; the binary turned out to be dated
*after* that morning's commits, and the fault was in fact a regression introduced by
them.  `ls -l --time-style=long-iso $(which loft)` first, and when it is not older than
the work, build the parent commit in a worktree instead:

```bash
git worktree add /tmp/pre <commit>^
cd /tmp/pre && CARGO_TARGET_DIR=/tmp/pre-target cargo build --release --bin loft
ln -s /tmp/pre/default /tmp/pre-target/release/default   # it loads default/ beside the binary
```

Related: **never run `find_problems.sh --bg` while building in the foreground** — they
share `target/`, and the contention produces failures that vanish on a settled tree.

---

## Bounding a run — `--timeout` / `LOFT_TIMEOUT` (@PLAN49)

**loft has no process-wide default timeout, by design.** Long-running programs —
servers, game loops, anything that should run until interrupted — must be able to
run unbounded, so we will not add a default. **Testing is the exception:** `loft
test` / `--tests` arms the watchdog at 300s automatically (a hung test or looping
compile in the suite can't be killed interactively).

When you run loft **ad-hoc** in an agent session — a `/tmp` probe, a one-shot
script, and especially `--native` (it shells out to `rustc`, which can hang on a
pathological program) — **bound it yourself**, or a runaway hangs the session:

```bash
LOFT_TIMEOUT=60 loft --native prog.loft     # env form — arms at startup, is the floor
loft --timeout 60 prog.loft                  # flag form — re-arms; 0 disables
LOFT_TIMEOUT_GRACE=5 LOFT_TIMEOUT=60 loft …  # grace before the hard kill (default 2s)
```

Mechanics (`src/timeout.rs`): `arm(secs, grace)` spawns a `loft-watchdog` thread
that sleeps to `secs + grace`, prints a breadcrumb, and **process-aborts** — so it
bounds the WHOLE process: the `--native` compile, the interpreter loop, everything.
`arm` is idempotent (first deadline wins) and `secs == 0` leaves it disarmed (the
default for ad-hoc runs — hence the hang risk). `LOFT_TIMEOUT` is read before argv,
so it is the floor; an explicit `--timeout` only re-arms if nothing armed yet.

Rule of thumb: **server/long-task run → unbounded; test or throwaway probe →
always pass `LOFT_TIMEOUT`.**

---

## Tracker-tag indexer (`make index` + `./scripts/idx`)

The tracker-tag indexer (@PLN42) maintains
`index/tags.json`, a structured map of every `@P-id` /
`@PLAN-id` reference in the tree (plus the `legacy:`
bare-name forms during the migration).  Replaces
`grep -rn '@P259'` with O(1) JSON lookups.

### Usage

```bash
make index                           # rebuild index/tags.json
./scripts/idx tag:@P259              # exact tag → JSON refs
./scripts/idx prefix:@PLAN22         # all PLAN22-* tags
./scripts/idx file:doc/.../X.md      # tags in one file
./scripts/idx all | jq '.[:10]'      # top-N by reference count
./scripts/idx help                   # full usage block
```

### Auto-refresh on commit

After fresh checkout, install the pre-commit hook so
`index/tags.json` stays fresh whenever you commit doc or
code changes:

    make index-install-hook

The hook is idempotent (re-running won't double-install)
and safe with existing pre-commit content (it appends a
marker-bracketed block).  The hook re-runs the scanner
when any `*.md`, `*.rs`, `*.loft`, `*.toml`, `*.py`, or
`*.sh` file is staged; commits that only touch other
paths skip the scan.  Adds ~1 sec to commits that touch
indexed files.

If the scanner fails for any reason, the hook prints a
warning but does NOT block the commit (broken hooks
erode trust faster than stale index data does).

### Where it lives

| Path | Purpose |
|---|---|
| `tools/indexer/scan.sh` | The scanner |
| `tools/indexer/install-hook.sh` | Hook installer (idempotent) |
| `tools/indexer/ARCHITECTURE.md` | Design notes |
| `scripts/idx` | CLI query wrapper |
| `index/tags.json` | Output (gitignored) |
| `doc/claude/plans/42-tracker-index/` | Plan + per-phase docs |

---

## Open work

Diagnostic tooling enhancements surfaced by recurring debug
sessions across @PLAN22's 02d sub-phases (Sept-Oct 2026).
Each row is a focused, single-commit improvement; collectively
they would have shaved 10-20 hours of `eprintln!`-and-rerun
diagnosis time across @PLAN22 phases 02d-iii through 02d-vii.
Listed in ROI order (highest leverage first).

| Tool | Effort | Where it would have helped | Notes |
|---|---|---|---|
| ~~`LOFT_LOG=locks`~~ — log every `set_locked(true/false)` with store_nr + rec | ~~XS~~ Shipped 2026-05-12 | 02d-vii ("Write to locked store at rec=8 fld=2" with no provenance) | Instrumented at both `Stores::lock_store(&r)` (per-DbRef arm) and `Store::lock()` (low-level arm — catches direct callers like compile.rs const-store init).  Reads `LOFT_LOG` directly via `lock_trace_enabled()`.  Use alongside `LOFT_LOG=full` (set both, locks-trace interleaves with bytecode trace) — single-mode `LOFT_LOG=locks` also shows bytecode (the preset extends `full`). |
| ~~Better always-on panic context~~ | ~~XS~~ Shipped 2026-05-12 | All store-related panics | Added a `lock_origin: String` field to `Store`; populated by every `Store::lock_with_origin()` caller (lock_store sets `lock_store(store_nr=N, rec=M)`, compile.rs sets `compile.rs::compile (CONST_STORE init)`, etc.).  Surfaced in `Write to locked store at rec=N fld=M (locked by: …)`, `Claim on locked store …`, and `Delete on locked store …` panic messages.  Always-on (no log mode needed). |
| ~~`LOFT_LOG=type_timeline:<varname>`~~ | ~~S~~ Shipped 2026-05-13 | 02d-iii.a, 02d-v, 02d-vi, 02d-vii (each asked "what type does this var have right NOW?" 3+ times via ad-hoc `eprintln!`) | Instrumented at all 7 type-mutation sites in `src/variables/mod.rs` (`add_variable`, `add_temp_var`, `change_var_type` two arms, `set_type`, `depend`, `substitute_type`).  Each emits `[type_timeline] <name> (v_nr=N) <old> -> <new>  origin=<site>` to stderr.  Filtered to a single varname so output stays focused.  Reads `LOFT_LOG` directly via `type_timeline_target()`; no LogConfig needed. |
| ~~`loft --dump-ir <fn>`~~ | ~~S-M~~ Shipped 2026-05-13 as `LOFT_LOG=ir:<fn_name>` | 02d-iii.e, 02d-vi (used `LOFT_LOG=full` to infer IR shape from bytecode; direct IR dump would be 5× faster to read) | Implemented as an `ir_only` LogConfig preset (phases.ir=true, phases.bytecode=false, phases.execution=false, show_functions filter).  New `compile::show_ir_only(&Data)` helper avoids the `&mut Data` requirement of `show_code`.  Wired into `execute_log_impl` so the IR dump fires before silent execution under `cargo run --interpret` (not just `--dump`).  Substring match on fn names. |
| ~~`LOFT_LOG=slots:<fn>`~~ | ~~M~~ Shipped 2026-05-13 | 02d-v ("Incorrect var b[65535] versus 60" — slot was unallocated because no `Set/v_set` IR marked the var as defined; not visible in any current dump) | Post-allocation summary in `assign_slots` (src/variables/slots.rs:29).  For each var: ASSIGN with slot+size+type, or SKIP with explanation (is_argument, zero-size, no first_def, !is_defined, or unknown).  Substring fn-name match.  Shipped much faster than estimated — the M effort assumed deep instrumentation; a single post-allocation pass turned out to be sufficient. |
| ~~`loft --dump-captures <fn>`~~ | ~~M~~ Shipped 2026-05-13 as `LOFT_LOG=captures:<fn_name>` | 02d-iii.e (5 separate `eprintln!` cycles to inspect closure-record attribute types across passes) | Implemented as a free function `compile::show_captures_summary(writer, &Data)` self-gated on `captures_trace_target()`.  Two-pass: parent fns matching the filter that have non-empty `scalars_to_box`, then ALL lambdas with `closure_record != MAX` (since `__lambda_N` synthetic names don't match user filters).  Per attribute, prints the storage encoding inferred from the type (12B share-by-DbRef auto-Reference, 12B owned Reference, inline Integer/Text/Float/etc.).  Wired into `execute_log_impl` so it fires under `cargo run --interpret`. |
| ~~`scripts/probe-matrix`~~ — boundary-matrix cell runner | ~~S~~ Shipped 2026-06-12 | 2026-06-12 vector-ABI session (the 93-vsort leak): the 18-cell matrix was built by hand-written bash heredocs; the FIRST version was vacuous (all 24 cells parse errors read as "clean") and no cell had a hand-computed expected value, so HEAD/main agreeing on `acc=39` (true value 12) passed as green | See § "Boundary-matrix runner" below.  The three validity rules are HARD ERRORS: missing `@EXPECT`, a no-output (vacuous) cell, and a missing-or-passing `@CONTROL` cell each make the run red.  `--baseline <binary>` classifies every failure as `REGRESSION` vs `PRE-EXISTING` automatically — the A/B-worktree pattern that made the session's regression attribution take seconds.  Dogfooded on the session's real cells (tuple-loop + #360 self-arg). |
| Moment-of-urge matrix hook (settings.json) | XS | Every "rushed fix without a matrix" episode — the #354 session's three cascading non-matrix fixes (hoist-everything leak, `let _ =` break, `callee_forwards` fragility); static doc text + memory entries were loaded and still lost to momentum | The only trigger the harness GUARANTEES: a PostToolUse hook on Bash sets a session flag when output matches `FAIL\|SIGSEGV\|panicked\|leaked\|assertion failed`; a PreToolUse hook on Edit/Write touching `src/**/*.rs` injects one line while the flag is set — "test failure seen this session: does a probe matrix with expected values exist?". Fires exactly at the urge-to-fix moment, where doc-reading has already decayed. No classifier, no blocking — a reminder injection only. Implement via the `update-config` skill. |
| ~~Dep-graph / lifetime visualizer~~ | ~~L (~1 week)~~ **Delivered by @PLN103 (2026-07-12)** — as TWO text views, not a DOT graph: `loft introspect --show-ownership` (static per-binding Owned/Borrowed/Join, backend-shared) + `LOFT_STORES=timeline` (runtime per-store lifeline + working-set-vs-leak summary). Corpus-driven (`plans/103-lifetime-inspector/probes/`). The deferral note below still records WHY a full DOT graph wasn't built. | ~~Deferred as of 2026-05-13.~~  The `Vec<u16>` deps are overloaded (heap-owned / borrow / auto-Reference sentinel / mixed), parallel mechanisms (`work_text` / `inline_ref_vars` / closure_var_map) need to be merged, time-varying deps need snapshots, and graph rendering at scale needs DOT/graphviz — high complexity per debug-bug-saved.  @PLN103 sidestepped all of it: it RENDERS the resolved `use_analysis::ownership_of` verdict per binding (no dep-decoding, no graph), and the time-varying half is the runtime timeline. |
| ~~Two `--show-ownership` UAF overlays~~ (temporal extension of @PLN103) | ~~S~~ **Shipped 2026-07-19** | The captured-group UAFs (`plans/captured-group-elem-uaf.md`) were INVISIBLE to the @PLN103 inspector: the static ownership verdict is temporal-agnostic (a correct free and a use-after-free render identically) and the runtime timeline tracks leaks, not reads-after-free. | TWO complementary walks over the committed IR, both rendered under `--show-ownership`: (1) `use_analysis::free_before_dependent_read` — an `OpFreeRef(S)` followed by a DEREFERENCE of any TRANSITIVE view of `S` (deref-only, so a bare-`Var` delivery is not a false positive); catches 35m. (2) `use_analysis::return_source_freed` — a PLAIN `OpFreeRef(S)` where `S` is a record the return value ALIASES on the same path (path-sensitive, so a plain free on a `return null` path is not a false positive; the safe form is `OpFreeRefIfDistinct`); catches 35c, which the deref overlay cannot see (the return delivers a reference, not a deref). Both verified: fire on the buggy trees, silent on the whole corpus/stdlib. Gate: `tests/introspect.rs::ownership_overlay_silent_after_captured_group_fix` + parser-free `use_analysis::{uaf_overlay_tests, return_source_tests}`. Residual blind spot: a bug whose ROOT is a dropped/missing dep — a dep-based check cannot see a dep that isn't there. |

**Why DEBUG.md and not a plan**: each row is independent
(no cross-dependencies), each ships in a single commit, and
the work doesn't have phases — it's classic light-flow
infrastructure.  Per `loft-plan-workflow` skill, plans are
for genuinely multi-phase initiatives with shared design.

**Cross-cutting motivation**: the loft compiler has multiple
passes (parse-1, parse-2, scope analysis, codegen) that each
transform types, slots, and IR.  Mismatches between passes
manifest as runtime panics with thin context.  The current
debug story relies heavily on `LOFT_LOG=full` which is
high-volume and requires reading bytecode + cross-referencing
codegen.rs.  More targeted log modes that focus on specific
subsystems (locks / types / slots / captures) reduce the
cognitive load per debug session.

## Branch review viewer (`make view`)

A loft-script binary that serves a branch-aware doc + code review
dashboard from a browser.  Useful for reviewing in-flight work
without scrolling through chat snippets.  Built by @PLAN35 (closed
2026-05-14); lives in `tools/viewer/` + `lib/markdown/`.

### Usage

In the VM:

```bash
make view-build          # one-time, when updating the host loft binary
make view                # refreshes git state + starts server on 8765
make view-refresh        # refreshes git state without restarting the server
```

From the host:

```bash
ssh -L 8765:localhost:8765 vm-user@vm-host
```

Open `http://localhost:8765/` in a browser.

### Routes

| Path | Renders |
|---|---|
| `/` | Branch dashboard — branch name + ahead/behind vs `main` + HEAD sha/msg, changed-files list, uncommitted-files list, last 20 commits.  Status badges (M/A/D/R) on every changed file. |
| `/file/<path>` | File view.  `.md` files render via `lib/markdown` (full subset: ATX + setext headings with GH-slug ids, lists with continuation merging, GFM tables with alignment, fenced code, inline formatting, links with relative-path resolution + title attribute, images via `/raw/`, autolinks `<https://…>` / `<email>`, `@P-id` / `@PLAN-id` autolinks, blockquotes, task lists, strikethrough, backslash escapes).  Other files render line-numbered with `<a id="L42">` anchors. |
| `/diff/<path>` | Per-file unified diff vs `main` with hunk colouring (green +, red −, blue hunk header). |
| `/commit/<sha>` | Commit message + per-file diffs via the same hunk-coloured renderer.  Last 20 commits captured. |
| `/tag/<bare>` | Every tracker-tag reference for a P-id or PLAN-id (e.g., `/tag/P259` lists all references to `@P259` and `legacy:P259`).  Reads `index/tags.json` built by `make index` (@PLN42). |
| `/tree/<path>` | Directory listing; sub-dirs are clickable. |
| `/raw/<path>` | Raw file bytes (`text/plain`).  Used by markdown to serve relative image refs. |

### File-page view toggle

Every `/file/<path>` page shows a `[Rendered ¦ Diff vs main]`
toggle in the top-right.  When the file is unchanged on the
current branch (no per-file diff), the "Diff vs main" link
hides — only "Rendered" stays.

### Architecture

| Layer | Where | Purpose |
|---|---|---|
| Server + routes + page templates | `tools/viewer/src/main.loft` | Loft script — HTTP server via `lib/server`, route dispatch, dashboard / tag-page / commit-page / diff-page / file-page rendering |
| Markdown rendering | `lib/markdown/` | Standalone loft library — single-file `src/markdown.loft`, comprehensive `tests/01-render.loft` |
| Git state | `tools/viewer/state/*.json` + `state/diffs/*.diff` + `state/commits/*.diff` | Filled by `tools/viewer/refresh.sh` (uses `git` + `jq`) |
| Tracker-tag index | `index/tags.json` | Filled by `make index` (@PLN42) |
| Static CSS | embedded in `main.loft::BASE_CSS` | Light + dark via `prefers-color-scheme` |

### Dependencies

- **`git`** — used by `refresh.sh` to dump branch state
- **`jq`** — used by `refresh.sh` to safely emit JSON
- **The host loft binary** at `target/release/loft` — built via
  `make view-build`; the viewer is a loft script interpreted
  by it (or `--native`-compiled via the same binary)

No Python, no markdown lib, no syntax-highlighter dep, no
template engine.  All rendering is loft-native through `lib/markdown`
+ string concatenation in `main.loft`.

### Frozen-binary contract

The viewer source (`tools/viewer/src/main.loft`) and the host
loft binary it runs against form a deliberately **frozen pair**.
`make view-build` rebuilds the host binary; `make view` runs
the existing one.  This means the viewer keeps working through
loft refactors — refresh by running `make view-build` against
a known-good loft commit.

### Backends

The viewer runs under **both `--interpret` and `--native`**
(since the seven-bug native arc @P262→@P269 closed 2026-05-13).
`make view` invokes `--interpret` by default for fast iteration;
edit the Makefile target to swap in `--native` for the faster
steady-state runtime.

### Troubleshooting

- **Dashboard shows "No git state. Run `make view-refresh`"** —
  the refresh script hasn't built `tools/viewer/state/*.json`
  yet.  Run `make view-refresh`.
- **`/tag/<bare>` shows "No index found"** — `index/tags.json`
  is missing.  Run `make index`.
- **`/diff/<path>` shows "No diff captured"** — the file isn't
  on the changed-files list (no diff vs `main`), OR refresh.sh
  capped at 100 changed files.  Run `make view-refresh`.
- **`/commit/<sha>` shows "No diff captured"** — refresh.sh
  only keeps the last 20 commits.  For older commits, run
  `git show <sha>` directly.

### See also

- [`plans/finished/35-branch-review-viewer/README.md`](plans/finished/35-branch-review-viewer/README.md) — the full design + per-phase build log
- [`lib/markdown/loft.toml`](../../lib/markdown/loft.toml) — the rendering library

---

## See also
- [../DEVELOPERS.md](../DEVELOPERS.md) — Developer guide: pipeline overview, quality requirements, feature proposals
- [TESTING.md](TESTING.md) — Test framework, `code!` / `expr!` macros, LogConfig debug presets
- [PROBLEMS.md](PROBLEMS.md) — Known bugs with severity, workarounds, and fix paths
- [SLOTS.md](SLOTS.md) — Slot assignment design (for the slots-dump enhancement)
- [LIFETIME.md](LIFETIME.md) — Dep tracking and scope-based freeing (for the dep-graph enhancement)
- [SLOTS.md](SLOTS.md) — Variable scoping and slot assignment details
