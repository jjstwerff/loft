<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# SANDBOX.md — letting users run scripts without breaking the host

> **The mechanism.** Let users run scripts that have FAST, direct access to the
> host's data structures, without letting them break the host. The answer is
> **not** wasm isolation (marshalling host state across a boundary every frame
> defeats the point), but two cooperating, *validatable* ideas:
>
> 1. **Admission control** — *validate the script before allowing it*: restrict
>    what it can reach (libraries / capabilities) and bound what it can do (loops,
>    recursion). Users do not have to be able to do everything.
> 2. **Effect containment + fault isolation** — run the admitted script against a
>    **transactional sandbox world** (a journaled / scratch store) so its changes
>    are *discardable*, and catch every fault at the script boundary so a "break"
>    aborts *the script*, never the host.
>
> An admitted script runs **in-process, at full interpreter speed, with direct
> store (`DbRef`) access**. The motivating use case is a **player playground for
> testing strange ideas without breaking everything** — but the *why* (game mods,
> an in-game console, a live REPL, any user-scripting surface) does not change the
> *how*: the same admission + containment + bounded-execution mechanism, below.
>
> Written the loft way: every requirement is pinned to a **runnable Check**, so
> the gap is measurable, not aspirational. Most Checks are RED today; this doc is
> the checklist the work flips to GREEN.
>
> **The loft-specific unlock:** the store architecture already carries the pieces
> for a transactional world — a journal (`src/database/journal.rs::snapshot`) and
> cross-store copy (`copy_block_cross_store`) — and the live surface exists too
> (`src/repl.rs`, `src/live_reload.rs`, `src/live_dispatch.rs`). So this is
> *assembling existing substrate behind an admission gate*, not green-field.

## The model — validate before allow, don't isolate after

The trilemma: **{ full isolation · fast rich data access · arbitrary untrusted
code }** — you get two. Games choose *fast-data + admission-validated-code* and
give up *arbitrary-untrusted*. "Users do not have to be able to do everything":
a script earns admission by being expressible in a restricted, checkable subset.

Three tiers, by author trust:

| Tier | Mechanism | Data access | Perf | This doc |
|---|---|---|---|---|
| **Trusted** (you / your studio) | in-process, full language | direct store | native | rely on correctness + S4/S5 |
| **Admitted-restricted** (community mods) | **in-process, admission-validated subset** | direct store | native | **the target — S1–S6** |
| **Untrusted** (arbitrary internet code) | wasm-isolated (`wasm32`, no caps, fuel) | marshalled | slower | footnote tier only |

**Why admission beats a runtime sandbox for games:** it is a *one-time* cost at
load (not per-frame), it preserves direct store access (no marshalling), and a
rejection is deterministic and auditable (you can show *why* a script was
refused). A runtime sandbox pays its tax on every data access, forever.

**The two independent guarantees (both required, different jobs):**
- **Admission control limits what a script can EXPRESS** (which libraries, calls,
  loops, recursion) — this doc.
- **A memory-safe interpreter limits what a script can EXPLOIT** — a script that
  triggers a store-lifetime UAF/double-free escapes *regardless* of admission, so
  [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) / the `@PLN85` store-lifetime work
  is a hard dependency. Admission narrows the *language*; the store work removes
  the *escape hatch*.

## The chokepoints that make admission feasible

loft is unusually well-suited to static admission: a two-pass compiler with a
typed IR, **one** import resolver (`Parser::lib_path`, `src/parser/mod.rs`), and a
fully resolved call graph (every call is a `def_nr`; every native is an `n_*`
symbol in the registry). So "validate before allow" is a single pass over the
already-built IR, enforced at known points — not a new analysis engine.

---

## The validatable invariants

Each: **Invariant · Chokepoint · Check (runnable) · Status today**.

### S1 — Capability allowlist (libraries + natives)
An admitted script references only allowlisted symbols: the engine's exposed game
API + vetted pure-loft libraries. No file / network / env / process / FFI natives;
`use` only an allowlisted lib (never one with a `[native]` cdylib).
- **Chokepoint:** the `use` resolver + a walk of the resolved IR's referenced
  `def_nr` / `n_*` symbols against the policy set.
- **Check:** a script that calls `file(...)` / `env_variable(...)` / `write(...)`
  (`default/02_files.loft`) or a `web`/`server` native — or `use`s a native-cdylib
  lib — is **rejected at admission** with a named-symbol reason.
- **Status: 🔴 RED.** Those are plain `pub fn`s any script calls; there is no
  admission gate (no deny-by-default capability model anywhere in `src/`).

### S2 — No native FFI, no rustc
An admitted script never loads a cdylib and never runs through the native (rustc)
backend (generating + compiling Rust on the host is RCE by construction).
- **Chokepoint:** the backend selector + `extensions::resolve_native_lib`.
- **Check:** admission forces `--interpret` and refuses any `[native]` lib /
  `dlopen`. `cargo build --no-default-features` (drops the `native-extensions`
  feature) compiles the `libloading` path out entirely — a buildable proof the
  FFI surface can be removed.
- **Status: 🔴 RED.** `native` is the **default** backend (`loft --help`:
  *"native is default"*); cdylibs `dlopen` freely via `libloading`
  (`src/extensions.rs`). Feature-gated but on by default.

### S3 — Bounded recursion / call depth
No unbounded recursion: either statically reject cycles in the call graph the
compiler already has, or cap call depth at runtime so a script cannot exhaust the
stack.
- **Chokepoint:** the resolved call graph (admission) + a depth counter (runtime).
- **Check:** a deeply-recursive / deeply-nested script is **rejected or
  depth-capped — never a crash**.
- **Status: 🔴 RED — and worse than unbounded: it CRASHES.** A 5000-deep nested
  expression **segfaults the parser** (`rc=139`):
  ```sh
  python3 -c "print('fn main(){x='+'('*5000+'1'+')'*5000+';}')" > /tmp/n.loft
  ./target/release/loft --interpret /tmp/n.loft   # → SIGSEGV (139)
  ```

### S4 — Bounded loops / per-script fuel
Loop back-edges (and calls) decrement a **per-script** budget; exhaustion aborts
**the script, not the process** — the engine continues the frame.
- **Chokepoint:** the bytecode loop back-edge / a fuel counter in the VM dispatch.
- **Check:** `fn main(){ while true {} }` aborts via budget and the host keeps
  running (a recoverable error, not a process death).
- **Status: 🔴 RED.** The only bound is a process-level wall-clock `--timeout`
  (`src/timeout.rs`, @PLN49) that **hard-kills the whole process** (SIGABRT):
  ```sh
  printf 'fn main(){x=0; while true {x=x+1;}}' > /tmp/l.loft
  ./target/release/loft --interpret --timeout 3 /tmp/l.loft
  # → "[timeout] hard-kill after 3s+2s grace" + SIGABRT  (kills the engine, not just the script)
  ```
  Useless for a frame loop: it ends the game, not the runaway mod.

### S5 — Crash-safe admission (the FOUNDATIONAL prerequisite)
The validator — i.e. the parser — must **reject** hostile input, never crash. You
cannot gate on a validator that segfaults: S1/S3 are unreachable until the parse
that feeds them is itself bounded.
- **Chokepoint:** parser recursion-depth limit (return a parse error at depth N).
- **Check:** the S3 nested-source probe returns a clean parse error, not SIGSEGV.
- **Status: 🔴 RED.** Same `rc=139` segfault as S3. **Land this first.**

### S6 — Performance preserved (the whole point)
An admitted script runs in-process with direct store (`DbRef`) access — no
per-access marshalling — so per-frame data work stays at interpreter speed.
- **Chokepoint:** the script shares the engine's `Stores` (no boundary copy).
- **Check:** a benchmark — a script reading/writing N entities/frame stays within
  a frame budget vs a native baseline (graduate to `benches/` once script mode
  exists).
- **Status: ⚪ N/A today** (no script mode) — but the asset is real: loft's direct
  `DbRef` store access is exactly what makes the in-process tier fast. This is the
  reason NOT to choose the wasm-marshalling tier for game scripts.

### S7 — Effect containment (transactional sandbox world)
A script READS the live world (fast, rich, direct) but its WRITES land in a
journaled / scratch store; on error or on "discard," the effects roll back and the
live world is untouched until explicitly committed. "Break everything" becomes
"discard this experiment."
- **Chokepoint:** run the script against a `snapshot` / journal of `Stores`;
  commit-or-discard at the script boundary.
- **Check:** run a script that mangles state in the sandbox, then discard → the
  live world is byte-identical to before; commit → the changes apply atomically.
- **Status: 🔴 RED, but substrate exists.** Scripts share the live store directly;
  no transaction boundary. The pieces are there: `src/database/journal.rs::snapshot`
  + `copy_block_cross_store` (`src/database/structures.rs`).

### S8 — Fault isolation (the host survives any script fault)
A script that errors / overflows / hangs is caught at the *embedding boundary* and
surfaced as a value; the host's next operation still runs. The host embeds loft and
gets a `Result`, never an `exit()` / abort / segfault.
- **Chokepoint:** an embedding entry — `run_script(src, policy, world) -> Result<_,
  ScriptError>` — that catches runtime errors + fuel-exhaustion (S4) and never runs
  on an unparsed-safe input (S5).
- **Check:** a script doing OOB / div-by-zero / `while true {}` returns a
  script-level error and the host continues serving.
- **Status: 🟡 PARTIAL.** The *errors* are modeled and clean — OOB index returns
  `null` (exit 0), div-by-zero is a clean error (exit 1) — so they are recoverable
  by design. The gaps: loft is a CLI that **exits the process** (no `run_script`
  API that *returns* the fault to an embedder), and the two true crashes — the
  parser segfault (S5) and the loop `SIGABRT` (S4) — bypass any boundary. Close
  S5 + S4, then deliver faults through an embedding API.

---

## Buildable now — the first slice (no substrate rework)

These are XS/S, individually validatable, and need none of the big pieces
(transactional world, wasm tier, full fuel). Each flips a RED Check above to
GREEN and stands on its own.

1. **Fix the parser-recursion segfault (S5) — XS, a real DoS bug today.** Add a
   nesting-depth guard to the recursive-descent expression parser
   (`src/parser/expressions.rs` / `operators.rs`); return a parse *error* past a
   configurable depth instead of overflowing the native stack. **Validate:** the
   nested-input probe (`rc=139` → clean parse error). Foundational — S1/S3/S8
   cannot gate on a validator that crashes — and worth fixing on its own merits.

2. **Graduate the probes to a committed RED suite (`tests/sandbox.rs`) — XS.**
   Lock the loop (S4), parser (S5), ambient-capability (S1), and fault (S8)
   probes as regression guards asserting today's behaviour, so every invariant
   has a permanent home that flips RED→GREEN as it lands. The cheapest way to
   make this doc's "validatable spots" durable.

3. **An interpret-only, no-FFI script profile (S2) — XS/S.** A policy that forces
   `--interpret` and refuses any `[native]` lib at the import resolver. The
   `native-extensions` feature already proves the `dlopen` path is removable.
   **Validate:** `cargo build --no-default-features` has no `libloading`; a
   `use <native-lib>` under the profile is refused.

4. **A `run_script(src, policy) -> Result<_, ScriptError>` library entry (S8) — S.**
   loft is already a library (`src/lib.rs`); wrap the interpret path so a fault
   *returns* instead of `exit()`/abort. Immediately yields fault isolation for the
   already-clean errors (OOB → `null`, div-by-zero → error). **Validate:** a host
   harness runs a faulting script and keeps serving.

5. **A coarse op-budget counter (S4) — S.** A feature-gated instruction counter in
   the bytecode dispatch loop that returns a "budget exhausted" error at N ops —
   before full per-loop fuel. **Validate:** `while true {}` under a budget returns
   a recoverable error; the host survives (vs today's process `SIGABRT`).

6. **A capability denylist at the import/native chokepoint (S1, first cut) — S.**
   Thread a policy through the parser; the `use` resolver (`Parser::lib_path`) and
   the native-symbol binder reject the file/net/env/process natives (`file`,
   `write`, `env_variable`, …) unless granted. **Validate:** a script calling
   `file(...)` is rejected at admission with a named-symbol reason.

The bigger pieces — S3 full call-graph depth, S7 the transactional world, S6 the
perf benchmark, and the wasm tier — follow these.

## Landing order

1. **S5** — bound parser recursion (clean error, not SIGSEGV). Foundational;
   blocks S1/S3/S8. Small + immediately validatable.
2. **S8** — an embedding `run_script() -> Result` boundary that catches faults and
   never exits the host (gives fault isolation for the clean errors at once).
3. **S2** — force the interpreter + ban FFI for script mode. Coarse, cheap, big
   risk reduction (no rustc, no cdylib).
4. **S1** — the capability/library allowlist over the resolved IR.
5. **S4 + S3** — per-script fuel (recoverable abort) + call-depth cap; completes
   S8 for hangs/overflows.
6. **S7** — the transactional sandbox world (effect rollback) over `journal`.
7. **S6** — benchmark that admission preserved native-speed data access.

In parallel, the [store-lifetime work](STABILITY_REDFLAGS.md) removes the *exploit*
escape hatch that admission alone cannot close — admission limits what a script can
**express**, that work removes what a memory-safety bug lets it **exploit**.

## The honest edge (out of scope here)

Admission control limits what a script can **express**; it does **not** contain a
memory-safety **exploit** in the interpreter, nor a CPU/cache side-channel. For
genuinely-hostile, arbitrary code (not vetted community mods), the only sound
boundary is the **wasm-isolation tier** (compile to `wasm32`, run in an embedded
`wasmtime` with zero capabilities + fuel + memory limits — loft has no embedded
runtime today: `grep -i wasmtime Cargo.toml` is empty). That tier pays the
marshalling cost this doc's tier avoids, which is precisely why it is reserved for
code that cannot earn admission.
