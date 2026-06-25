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
> the gap is measurable, not aspirational.
>
> **The loft-specific unlock:** the store architecture already carries the pieces
> for a transactional world — a journal (`src/database/journal.rs::snapshot`) and
> cross-store copy (`copy_block_cross_store`) — and the live surface exists too
> (`src/repl.rs`, `src/live_reload.rs`, `src/live_dispatch.rs`). So this is
> *assembling existing substrate behind an admission gate*, not green-field.

> ## ✅ Admission control is BUILT — @PLN86 v1
>
> The **admission half** (idea 1) is implemented, tested, and CLI-enforced —
> [plan `86-sandbox-subset-flag`](plans/86-sandbox-subset-flag/README.md). A host
> declares a `[sandbox]` policy in `loft.toml`; designated functions are admitted
> only if proven safe at LOAD, and rejected with actionable errors otherwise.
> Surface: `Parser::sandbox_admission_errors` (+ `has_sandboxed_defs`),
> `src/sandbox.rs`. **Checks S1–S5 are now GREEN; S7/S8 are partially closed**
> (statuses updated per-invariant below). The four arcs an admitted script
> satisfies: **capability** (reaches only allow-listed libraries/groups),
> **termination** (bounded loops + acyclic recursion + total ops), **data
> integrity** (no raw writes to host data), **backend** (no external FFI; the
> proof itself is backend-agnostic — the former force-interpret was dropped).
> The next arcs are **DESIGNED, compile-only** (S9–S10 below): a **data envelope**
> (a load-time peak-heap bound) and **per-member access** (independent read/update/append
> rights per field + enum variant, declared as groups in the loft type defs). Per the
> **compile-only decision**, the transactional world (S7) and runtime guards are **dropped**;
> `run_script` (S8) survives only as the unknown-unknown backstop — see the plan's § Open work.

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
- **Status: 🟢 GREEN (@PLN86).** Library-first admission (`sandbox::admit_capabilities`):
  a reachable trusted symbol admits iff its **library** is allow-listed wholesale
  (`allow_libs`) or its `#cap` **group** is (`allow_caps`), else rejected naming the
  symbol + group + fix. L4-complete — indirect fn-refs (`f = read_file; f(…)`) resolve to
  their target, so they can't escape. A script calling `file(...)`/`write(...)` under a
  policy that grants neither is rejected at load.
- **TOTAL capabilities (host-side lint, @PLN86 prevention #3).** Admission gates *which*
  capabilities a script reaches; `capability_totality_violations` gates whether those
  capabilities can *fault the host*. It is the host-side mirror of the script-side abort-op
  exclusion (3.3): over every `#cap`-tagged function it flags those whose call tree reaches
  an abort op (`assert`/`panic`/`log_fatal`), so the host makes them total (validate + return
  a clean error, never abort) before exposing them. Catches the **loft-bodied** (library)
  capability surface; a **native** capability's Rust is opaque to the lint — the host vouches
  for native totality separately.

### S2 — No native FFI, no rustc
An admitted script never loads a cdylib and never runs through the native (rustc)
backend (generating + compiling Rust on the host is RCE by construction).
- **Chokepoint:** the backend selector + `extensions::resolve_native_lib`.
- **Check:** admission forces `--interpret` and refuses any `[native]` lib /
  `dlopen`. `cargo build --no-default-features` (drops the `native-extensions`
  feature) compiles the `libloading` path out entirely — a buildable proof the
  FFI surface can be removed.
- **Status: 🟢 GREEN (@PLN86 1.4) — interpret-only force DROPPED (2026-06-25).** The
  external-FFI ban stays: `sandbox::reachable_ffi_bridges` rejects a reachable external
  cdylib bridge unless `native_ffi` is granted (that dlopen is the real RCE surface). The
  former forced interpret-only was **removed** — it rested on a false "native traps where
  the interpreter is total" premise (re-probed: div/mod-zero, OOB, overflow all yield
  `null` on `--native` too), so admission is backend-agnostic and a sandboxed program runs
  on whatever backend the host picks (`Parser::has_sandboxed_defs` gates the admission
  walk). `cargo build --no-default-features` compiles the `libloading` path out (verified).
  *Remaining (post-v1):* feature-gate the rustc *codegen* path too, so
  a deployment can build with ZERO host-codegen surface.

### S3 — Bounded recursion / call depth
No unbounded recursion: either statically reject cycles in the call graph the
compiler already has, or cap call depth at runtime so a script cannot exhaust the
stack.
- **Chokepoint:** the resolved call graph (admission) + a depth counter (runtime).
- **Check:** a deeply-recursive / deeply-nested script is **rejected or
  depth-capped — never a crash**.
- **Status: 🟢 GREEN (@PLN86 3.2 + 0.1).** Recursion is rejected at admission:
  `sandbox::recursion_cycles` runs a colour-DFS over the sandboxed call graph and rejects
  any cycle (self + mutual), naming it. And the parser nesting guard (0.1) bounds expression
  depth *inside a sandboxed def* so the once-`rc=139` deep-nesting segfault is now a clean
  LOAD-time error there (limit 128). *(The guard is sandbox-scoped — trusted code is
  unaffected; a global parser-depth cap is separate.)*

### S4 — Bounded loops / per-script fuel
Loop back-edges (and calls) decrement a **per-script** budget; exhaustion aborts
**the script, not the process** — the engine continues the frame.
- **Chokepoint:** the bytecode loop back-edge / a fuel counter in the VM dispatch.
- **Check:** `fn main(){ while true {} }` aborts via budget and the host keeps
  running (a recoverable error, not a process death).
- **Status: 🟢 GREEN by admission (@PLN86 3.1).** v1 solves this at LOAD, not runtime: an
  unbounded `while` in a sandboxed def is **rejected at admission** (`sandbox::admit_totality`
  → `UnboundedLoop`), so it never runs — only bounded `for x in <collection>` / `for i in
  0..N` is admitted. The `O(n^d)` complexity report (3.4) additionally lets the host bound
  inputs so no admitted loop stalls a frame, and a parallel **space** degree (`sandbox_space_degree`) lets it
  bound them for *memory* the same way — a bounded loop building a structure cannot OOM (an
  abort `catch_unwind` can never see); `complexity_report` names both axes. *(A runtime
  per-script fuel counter — for a
  recoverable backstop instead of the process-level `--timeout` SIGABRT — is the post-v1
  runtime complement.)*

### S5 — Crash-safe admission (the FOUNDATIONAL prerequisite)
The validator — i.e. the parser — must **reject** hostile input, never crash. You
cannot gate on a validator that segfaults: S1/S3 are unreachable until the parse
that feeds them is itself bounded.
- **Chokepoint:** parser recursion-depth limit (return a parse error at depth N).
- **Check:** the S3 nested-source probe returns a clean parse error, not SIGSEGV.
- **Status: 🟢 GREEN (@PLN86 0.1).** The parser nesting guard returns a clean LOAD-time
  error past a depth bound while parsing a sandboxed def's body — the validator no longer
  segfaults on the hostile nested-source probe. (Scoped to sandboxed defs; zero cost to
  trusted code.)

### S6 — Performance preserved (the whole point)
An admitted script runs in-process with direct store (`DbRef`) access — no
per-access marshalling — so per-frame data work stays at interpreter speed.
- **Chokepoint:** the script shares the engine's `Stores` (no boundary copy).
- **Check:** a benchmark — a script reading/writing N entities/frame stays within
  a frame budget vs a native baseline (graduate to `benches/` once script mode
  exists).
- **Status: 🟢 ENABLED (@PLN86), benchmark pending.** Admitted scripts run in-process,
  interpret-only, sharing the engine's `Stores` at direct `DbRef` speed — no marshalling.
  The admission model is built, so the fast in-process tier is now realizable; a `benches/`
  per-frame benchmark vs a native baseline is the remaining proof.

### S7 — Effect containment (transactional sandbox world)
A script READS the live world (fast, rich, direct) but its WRITES land in a
journaled / scratch store; on error or on "discard," the effects roll back and the
live world is untouched until explicitly committed. "Break everything" becomes
"discard this experiment."
- **Chokepoint:** run the script against a `snapshot` / journal of `Stores`;
  commit-or-discard at the script boundary.
- **Check:** run a script that mangles state in the sandbox, then discard → the
  live world is byte-identical to before; commit → the changes apply atomically.
- **Status: 🟡 PARTIAL (@PLN86 2.4) + substrate exists.** Admission now forbids **raw
  writes** to host data (`e.health = 0` / `v[i] = 9` → rejected; `sandbox::RawWriteViolation`)
  — a script mutates host data only through allow-listed `*.write` ops, so it cannot corrupt
  an invariant in the first place. The *transactional rollback* (run against a journal,
  commit-or-discard) is the remaining runtime piece; substrate is there
  (`journal.rs::snapshot` + `copy_block_cross_store`).

### S8 — Fault isolation (the host survives any script fault)
A script that errors / overflows / hangs is caught at the *embedding boundary* and
surfaced as a value; the host's next operation still runs. The host embeds loft and
gets a `Result`, never an `exit()` / abort / segfault.
- **Chokepoint:** an embedding entry — `run_script(src, policy, world) -> Result<_,
  ScriptError>` — that catches runtime errors + fuel-exhaustion (S4) and never runs
  on an unparsed-safe input (S5).
- **Check:** a script doing OOB / div-by-zero / `while true {}` returns a
  script-level error and the host continues serving.
- **Status: 🟡 PARTIAL → much stronger (@PLN86).** An *admitted* script is now proven not
  to fault: total ops (3.3 — div-by-zero → `null`, OOB → `null`), no unbounded loop (3.1),
  no recursion (3.2), and the explicit-abort ops `assert`/`panic`/`log_fatal` are **excluded
  at admission**. The two former crash paths are closed for sandboxed code (parser guard S5,
  loop-rejection S4). *Remaining:* an embedding `run_script(src, policy) -> Result<_,
  ScriptError>` boundary so a host (not the CLI) gets the fault as a value — belt-and-
  suspenders for an interpreter bug.

### S9 — Data envelope (compile-time footprint bound)
An admitted script's peak heap is bounded at LOAD by a host-declared budget: the closed-form
footprint `coeff · max_input_n^degree` is proven `≤ data_budget`, else rejected. No runtime
allocation counter, no ceiling, no rollback — OOM (the one fault `catch_unwind` cannot see)
becomes a load-time concern.
- **Chokepoint:** the space analysis (`sandbox_space_degree`) extended with the coefficient
  (`Σ record_size`, exact from the type stride), compared against the profile's `data_budget`
  in the admission walk. Inputs are host-declared (`max_input_n` / `max_depth` /
  `max_string_len`).
- **Check:** a per-entity struct-building loop reports `coeff·n`; a script whose worst case
  exceeds `data_budget`, or whose allocation size can't be tied to a declared bound (uncapped
  string, host-value-sized alloc), is rejected at admission with the figure + fix.
- **Status: 🟡 DESIGNED ([@PLN86 P7](plans/86-sandbox-subset-flag/README.md)).** The degree is
  computed today; the coefficient + budget compare + static-sizing gate are the build. Pure
  compile-time; no @PLN85 dependency (it was only the dropped runtime layer).

### S10 — Capabilities: what a restricted caller may do
A capability is a permission the **host/library** requires of a **restricted caller**: the
host annotates *its own* surface with `group#right` links to explicitly-declared, namespaced
`capability` groups (resolved + validated at compile — an undeclared group is a load error),
the modder's profile is granted a set, and admission gates every point the modder's code
touches the host. **The modder's own code carries no links and is never restricted** — only
host reach is gated. The full model with examples is [@PLN86 §7](plans/86-sandbox-subset-flag/README.md).
Three surfaces:
- **call a function** — the call gate sits in the **signature** (`fn mtime(p: text) -> int fs#read;`),
  a first-class part of the contract, NOT in the `#native`/`#impure`/`#wasm` plumbing block.
  Passing arguments is part of the call (set is inherited — no extra grant).
- **override a parameter** — a parameter tagged `…#default` (`count: int = 1 spawn.count#default`)
  is pinned to its default unless the modder holds the lock; untagged parameters are free.
- **read / update / append a field** — `read` free, `update`/`append` deny-by-default;
  `append` only on a **collection** field (there is no append for a scalar).
- **Check:** an ungranted call, a non-default override of a locked parameter, or an ungranted
  field update/append is a **load error** naming the symbol + right + group; reads and untagged
  parameters admit. Script-owned data is unrestricted (the §2.4 ownership split).
- **Status: 🟡 DESIGNED ([@PLN86 P6](plans/86-sandbox-subset-flag/README.md)).** Pure
  compile-time, no runtime cost / rollback. The **same `capability`/`group#right` mechanism
  carries the S1 *function* capability surface** (the shipped `#cap "fs.read"` strings →
  signature `fs#read` / `fs#update`) — pre-customer, so functions, parameters, and fields land
  on one validated model. (Enum-variant *construction* gating is a separate question, not
  folded into read/update/append.)

> **The compile-only decision (finalizes S7/S8).** Effect containment is now **by
> construction** — a script can only touch members it is granted (S10), never more (S9) —
> **not by rollback**: the transactional world (S7) and any runtime resource guard are
> **dropped** (the perf tax + a rollback/exception path in the language are both rejected).
> `run_script() -> Result` (S8) remains **only** as the alarmed backstop for the
> unknown-unknown interpreter bug, never as a data-limit or write-containment mechanism.

---

## Buildable now — the first slice (no substrate rework)

> **✅ Executed by [@PLN86 v1](plans/86-sandbox-subset-flag/README.md).** This was
> the original first-slice plan; the admission half is now built and CLI-enforced.
> Items 1 (parser guard), 3 (interpret-only + no-FFI), and 6 (capability allowlist —
> grown into the full library-first model) are DONE; items 2/4/5 (a committed
> regression suite, the `run_script` embedding boundary, runtime op-budget) are the
> remaining runtime-side work. Kept below as the historical decomposition.

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
