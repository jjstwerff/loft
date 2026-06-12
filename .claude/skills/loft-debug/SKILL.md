---
name: loft-debug
description: Operational companion for running a loft bug down on this box — LOFT_LOG presets, dump files, find_problems flow, the live-debugger RPC surface (loft debug --rpc — scripted breakpoint/eval/setValue sessions), the native-backend env gotchas that cause FALSE failures, and the operational-safety rules for builds/processes. Routes to the matrix-first protocol (CLAUDE.md) for the METHOD; this is the MECHANICS. Apply when reproducing, investigating, or verifying a loft crash, wrong-result, or codegen bug — and whenever about to sprinkle println/probe prints into a loft program (the debugger session replaces them).
user-invocable: false
---

# Loft Debugging Reference (operational)

The **method** lives elsewhere and is always-loaded — this skill is the **mechanics**:
how to actually run a bug down here, the env traps that fake failures, and the
irreversible moves not to make.

## Method — read first, don't duplicate here

- **Matrix-first protocol** — `CLAUDE.md` § Debugging policy. Don't fix on first read;
  build the `/tmp` boundary matrix on `--interpret`; find the real boundary (the filed
  scope is usually wrong); fix at the chokepoint enforcing the *invariant* (no
  narrower / no wider); verify BOTH backends at the end.
- **Composition axes** (what to vary in the matrix) — `doc/claude/plans/README.md`
  § The composition axes.
- **Brittleness vs bugs** — `doc/claude/DESIGN_VERIFICATION.md` § C1 (the real target
  is robust algorithms; a green matrix can still be brittle).
- **Investigation plans** (heavyweight) — `doc/claude/plans/_INVESTIGATION_TEMPLATE.md`.

## Running a bug down — mechanics

- Single file, interpreter: `cargo run --bin loft -- --interpret file.loft`
- Native: `cargo run --bin loft -- --native file.loft`
- ⚠ **The default backend on this box is `--native`.** For the SEEING loop ALWAYS pass
  `--interpret` explicitly — strides/types are IR operands the interpreter surfaces in
  seconds, whereas `--native` pays a rustc compile per probe (that cost belongs at the
  final verify, not the loop).
- **`LOFT_LOG=`** presets (full table: `CLAUDE.md` § Debug logging / `doc/claude/TESTING.md`
  § LogConfig): `minimal` (exec trace — cleanest for runtime bugs), `static` (IR +
  bytecode, fastest for codegen), `crash_tail:N` (last N lines, flushed on panic),
  `ref_debug` (stack snapshots after Ref/CreateStack), `variables` (the per-fn var
  table — name/type/scope/slot), `fn:<name>` (one function). `LOFT_DUMP_DEPTH` /
  `LOFT_DUMP_ELEMENTS` tune the inline struct/vector dumps.
- **Dump files**: a failing wrap/native test writes `tests/dumps/*.txt` — full IR +
  bytecode + execution trace; the root cause is almost always visible there. (See
  `doc/claude/DEBUG.md`.) NEVER `git bisect` / `git checkout HEAD -- <file>` to
  investigate (CLAUDE.md § Debugging policy) — read the dump and reason.
- **Full suite, detached**: `./scripts/find_problems.sh --bg` → `--peek` mid-run /
  `--wait` to block; structured summary on finish in `/tmp/loft_problems.txt`.

## The agent debug surface — `loft debug <file> --rpc` (drive a live session, don't println)

The @PLN16 debugger speaks NDJSON over stdio (the contract:
`doc/claude/plans/16-debugger/PROTOCOL.md`). One `printf` pipes a whole scripted
session — breakpoint → inspect the live frame → eval → edit → resume. Patterns
below verified hands-on against this tree and a real multi-module consumer
(crawler's Sim). ⚠ Version boundary: an installed binary OLDER than the rpc
fixes (commit `9de72ada`) sends NO `verified` field on setBreakpoints and
ignores string-form `"log"` (array only) — when driving `/usr/local/bin/loft`,
prefer `target/*/loft` from this tree if the response lacks `verified`.

```sh
printf '%s\n' \
  '{"id":1,"req":"launch","file":"/abs/path/prog.loft"}' \
  '{"id":2,"req":"setBreakpoints","file":"/abs/path/prog.loft","breakpoints":[{"line":3,"condition":"n == 2"}]}' \
  '{"id":3,"req":"run"}' \
  '{"id":4,"req":"eval","expr":"a + n"}' \
  '{"id":5,"req":"setValue","target":"a","value":"100"}' \
  '{"id":6,"req":"stepOver"}' \
  '{"id":7,"req":"continue"}' \
  '{"id":8,"req":"disconnect"}' \
| loft debug /abs/path/prog.loft --rpc
```

- **Order matters: `launch` LOADS but does not run; `run` starts execution.** Set
  breakpoints between them. (`continue` before `run` just emits `terminated`.)
- **`setBreakpoints` matches files by BASENAME** (relative or absolute both work,
  including a `use`d module's file while launching the consumer; two same-named
  files in one program can't be told apart). The response carries
  `breakpoints:[{line, verified}]` — **check `verified`**: `false` means the
  breakpoint can never fire (no breakable code on that line, or a file the
  program doesn't use); don't wait on a stop that never comes.
- **Multi-lib programs**: append the usual lib flags —
  `loft debug src/x.loft --rpc --lib ../pkg/ …`.
- **`stopped` carries the whole frame**: function, line, and every live local —
  a full crawler `Sim` struct renders inline (vectors elided `…`); one event =
  the variables panel, drill down with `eval`.
- **`eval` runs against the paused frame** (`s.php`, `n * 100`, `len(v)`), and
  the frame holds only the locals **live ON that line**. An out-of-scope name
  returns `value:null`, not an error — don't misread null as "the field is
  empty".
- **`setValue` edits the live run** (scalar / text / enum / struct field /
  nested path / vector element; a whole heap local is rejected by design) —
  execution resumes WITH the edit: probe a hypothesis by injecting the suspect
  value instead of rebuilding.
- **Conditional breakpoints** (`"condition":"p.x == 9"`) filter by frame; a
  tracepoint (`"log":"expr"` or `"log":["e1","e2"]`, `"stop":false`) streams
  `output{category:"trace"}` lines (`expr = value`) without pausing — structured
  trace beats sprinkled printlns. Log entries are EXPRESSIONS, not format strings.
- **Conditions and trace exprs obey the same liveness rule** — a name not
  read/written on that line evaluates null/`?`, so a condition on it silently
  never matches. Anchor breakpoints on a line that uses the variables you test.
- Program stdout arrives as `output` events on the SAME pipe, never interleaved
  with protocol — parse line-by-line as JSON, switch on `event`/`id`.
- The interactive twin is `loft debug <file>` (the `(dbg)` prompt) — same engine,
  for a human; agents always prefer `--rpc`.

## Native-backend env gotchas — these fake FAILURES; rule them out before believing a native failure

1. **Default is `--native`** — see above; pass `--interpret` for the seeing loop.
2. **Toolchain mismatch.** The box's rustup default can differ from the repo's
   `rust-toolchain.toml` (e.g. 1.96 default vs 1.95 rlibs). Then `--native` fails
   `E0514 incompatible rustc`, *or* forcing a toolchain triggers a full from-scratch
   rebuild (~30 min). Fix: run native from **inside the repo** so `rust-toolchain.toml`
   applies; if a snap `rustc` is first on PATH, prefix
   `PATH="$(dirname "$(rustup which rustc)"):$PATH"`.
3. **Stale dependency rlibs.** After a rebase / `Cargo.lock` bump, a `--lib`-only build
   leaves dep rlibs stale → native tests fail `crate rustls / ureq / webpki / ring
   required to be available in rlib format, but was not found`. Fix: a **full**
   `cargo build --release` (NOT `--lib`) — the native harness links the whole dep
   tree. These are FALSE failures.
4. **`rust-lld` SIGBUS in tmpfs `/tmp` under parallel native compiles.** Native tests
   flake under `find_problems`' parallelism. Confirm any native failure **serially**
   (`cargo test --release --test <bin> <name> -- --test-threads=1`) before trusting it.
5. **Unhashed `libloft.rlib`.** After changing `loft-ffi/` or `loft-core` runtime code,
   `cargo build --release --lib` before native tests (else a wave of stale-rlib false
   failures).

Pattern: a sudden wave of native-compile/link failures right after a rebase / dep /
toolchain change is almost always 2–5, not a regression. Rebuild fully + re-run
serially before believing them.

## Operational safety — the irreversible moves

- **Never `kill` / `pkill` a process you did not personally start.** A broad
  `pkill -f "cargo run …"` matches a SIBLING agent's identical command — it has killed
  another agent's build (and the killer's own). To stop your OWN background task, kill
  only its specific PID / task-id.
- **Never touch `/home/jurjen/workspace/loft2`** — a parallel agent's workspace, not
  yours. A `--out-dir .../loft2/…` in a process you're inspecting means it is NOT yours.
- **An anomaly right before a destructive command (kill / rm / force-push / overwrite)
  is a STOP, not a footnote.** Investigate the surprise first — there is no undo, and
  this is exactly where "don't act on partial sight" matters most.

## After the fix — routes

- Found a *sibling* bug while debugging? Default is **FIX** it (cheapest bug you'll
  ever fix), not file it — `CLAUDE.md` § Bug-filing policy. File only if it blocks the
  task or is too big to fix now.
- Before marking `fixed-pending-merge`: the **done-gate** —
  `doc/claude/ISSUE_TRACKING.md` § The done-gate (class coverage + intent match;
  "would the requester file a slight variation?"). Name any residual you couldn't
  close; don't leave it silent.
