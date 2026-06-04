---
name: loft-debug
description: Operational companion for running a loft bug down on this box — LOFT_LOG presets, dump files, find_problems flow, the native-backend env gotchas that cause FALSE failures, and the operational-safety rules for builds/processes. Routes to the matrix-first protocol (CLAUDE.md) for the METHOD; this is the MECHANICS. Apply when reproducing, investigating, or verifying a loft crash, wrong-result, or codegen bug.
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
