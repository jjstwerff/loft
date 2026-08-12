<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 140 — Semi-automatic profiling

## Status

Open — design ready, no implementation yet. The capture side landed first (commit
`1d6f2851`: a native profile named rustc and returned the program as bare hex; it now
warms the build, records with `--native-debug`, reports the rustc share, and prints the
sample count). What remains is **attribution** — turning samples into "this loft function,
this loft line, reached this way" — and the driver that makes it a single command rather
than a hand-driven perf session.

## Goal

`make profile` answers *which loft function or line is hot*, *which loft line allocated the
heap*, and *what path reached each*, on both backends — and declines to present a profile
it can tell is dishonest.

## Effort + design

- **Effort:** MH overall (A: S · B: M · C: M · D: S)
- **Design:** ~ (arc A detailed; arc B's sampling mechanism is the open question)
- **Last touched:** 2026-08-12

## What "semi-automatic" means here

The human picks the target and acts on the findings. The tooling does capture →
attribution → ranking → path, and **refuses to present a profile it knows is wrong**.
That refusal is the load-bearing half: every trap this tooling has hit so far —
a cache hit read as a compile, a build read as a run, a one-sample symbol read as a hot
spot — produced a *plausible* profile, not an obviously broken one. Automation that
ranks faster without checking honesty just reaches the wrong conclusion sooner.

Not in scope: automatic optimisation, or a profiler on by default.

## Why this earns its keep only at scale — and what that means for validation

A profiler's value grows with the program. A probe is small enough to profile *by hand*:
you can read it, time it, or reason the hot loop out of it without any instrument at all.
Which means **a probe is precisely the case where the tool is not needed** — and therefore
the case that cannot validate it.

That is a bar, not an observation — but it splits into **two roles that do not substitute
for each other**, and collapsing them is how an instrument passes without being checked.

**The oracle: the performance-measurement programs we already have.** `bench/01_fibonacci`
… `bench/11_parallel-for` and the `// @speed`-annotated tests are written to be measured —
they run long (so they sample plentifully, and the toy-run noise never arises) and, decisive
here, **their hot spot is known in advance**. `fib`'s time is in `fib`; the matrix benchmark's
is the inner loop. That makes them a falsifier: an instrument that fails to name the known
hot spot is *wrong*, and says so immediately. They are in-tree, need no read-only dance, and
several carry `bench.py` / `bench.rs` twins, so a second profiler can be asked the same
question and its answer compared. This is the cheap first check, and it belongs before any
consumer work.

**The scale test: a real grown consumer.** One of the dogfooding programs (`moros` /
`dryopea` / `crawler` / `lib/markdown`). Here the hot spot is *not* known, so this proves
nothing about correctness — what it proves is **usability**: does a thousand-site report
still fit on a screen, does the ranking put the right thing on top, does the overhead matter.
(Consumer trees are read-only: point a scratchpad package at their libs by path, per
`CLAUDE.md § Dogfood loop`.)

So: benchmarks say the instrument is *correct*, consumers say it is *usable*. Two
consequences follow, and neither shows up on a probe:

* **The small-run noise problem is not the real problem.** The 50-sample profiles that made
  `--annotate` pick a 1-sample `getenv` are an artefact of toy programs; a benchmark samples
  plentifully. Guarding the sample count is still right, but **aggregation and ranking** —
  thousands of sites, one screen of output — is the harder design, and only the consumer
  exercises it.
* **Overhead is measured where it lands.** An op-clock check or a per-allocation stack
  capture costs nothing worth noticing on a probe. Arc B's and arc C's overhead numbers only
  mean something taken on a program big enough to need the instrument — and the benchmark
  corpus already reports drift for exactly that comparison (`make speed`).

## Composition matrix — Stage A

No new composition surface: this plan adds instruments, not a value, type or operation, so
the value/type/operation matrix is N/A. The matrix that *is* the spec is **backend ×
instrument**, and it is where the current asymmetry shows:

| | `--interpret` | `--native` |
|---|---|---|
| CPU hot spot names a loft fn | ✗ interpreter internals only | ✓ shipped (`n_slow_part` 25 %) |
| Path to the CPU hot spot | ✗ structurally impossible via perf | ~ real, truncates above `start_thread` |
| Allocation site names a loft line | ~ built, fires only on a LEAK | ✗ `alloc_pc` is 0 outside interpretation |
| Path to an allocation | ✗ one `u32`, no stack | ✗ |
| Peak vs exit | ✗ exit only (ceiling report excepted) | ✗ |

Done means every cell is green or **explicitly declined with a reason recorded here** — the
two ✗ columns are not symmetric accidents, they are different mechanisms failing, and one
of them (native memory attribution) may be a legitimate decline rather than a gap. A cell
counts as green only when a benchmark whose hot spot is known confirms the instrument names
it, **and** a grown consumer confirms the report is still readable — the two roles above.

Why the interpreter's CPU path is *structurally* impossible rather than merely missing: a
loft call creates no machine frame, so perf's stack walk yields the interpreter's own path,
identical for every program ever run —

```
_start → __libc_start_main → main → std::rt::lang_start → loft::main
       → execute_argv → put_stack::<i64>
```

No sampling frequency or unwinder fixes that. It is the wrong stack.

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **A** — allocation hot spots by site, at peak | this doc | Open |
| **B** — loft-level CPU profiler: sample `State.call_stack` | this doc | Open — mechanism undecided |
| **C** — allocation *paths* (B's capture at allocation time) | this doc | Open — blocked on B |
| **D** — the driver: one command, corpus mode, honesty guards | this doc | Open (guards partly shipped) |

### A — allocation hot spots by site

The attribution already exists and already ranks. `alloc_pc` is the currently-executing
bytecode position, republished per-op by the dispatch loop and stamped into every freshly
allocated store's `created_at` (`src/database/mod.rs:394`) — explicitly *"for EVERY
allocation path"*. `LOFT_LEAK_SITES` (`src/state/mod.rs:5169`) then groups by
`(created_at, known_type)`, resolves `created_at` → source line through `line_numbers`,
sorts by count and prints:

```
[leak-site] 12× Layer (kt=112) allocated at pc=4821 (line 37) free_protected=false
```

That is a memory hot-spot report pointed at the wrong population. Three changes make it one:

1. **Live stores, not leaked ones.** Same loop, different filter.
2. **At the peak, not at exit.** Everything fires from `check_store_leaks()` after the run,
   by which time the peak is over — a program that peaks at 1.5 GiB and ends at 10 MB
   reports nothing. The store-memory ceiling is the model for the trigger: it reports *at*
   the growth that crosses it and names the type that filled the heap, with a
   one-store-vs-many breakdown.
3. **Bytes, not just store counts.** `LOFT_ALLOC_REPORT` today gives whole-program totals
   with no attribution — measured `peak=2 allocs=2 records=0` on a compute-heavy script. A
   large type and a small one currently weigh the same.

Effort S. No new mechanism; the risk is choosing the high-water trigger without adding a
per-allocation cost to ordinary runs. The second risk is only visible at scale: a grown
program has far more allocation sites than fit on a screen, so ranking and roll-up (by type?
by function? by line?) is the part a synthetic script will not exercise.

### B — a loft-level CPU profiler

loft keeps its own call stack: `State.call_stack: Vec<CallFrame>` (`src/state/mod.rs:85`),
pushed on every loft call (`:689`), which `stack_trace()` already renders outermost-first
with `file`, `line`, `function` and `arguments` per frame (STACKTRACE.md). It is maintained
continuously and only *snapshotted* lazily, when `n_stack_trace` asks. Sampling it on a
clock is a loft call tree — hot spots with backtraces, over loft frames.

Three candidate mechanisms, none free:

| Mechanism | Cost when off | Gives |
|---|---|---|
| (i) `SIGPROF` + a published pointer to `code_pos` | zero | true **time** sampling |
| (ii) counter check in the dispatch loop | one branch + increment per op | deterministic **op** sampling |
| (iii) piggyback the existing `self.debug.is_some()` per-op branch | **already paid** | deterministic op sampling |

(iii) is the cheap answer — the dispatch loop already runs `if self.debug.is_some() &&
self.debug_check(op_pos_rt, data)`, so a profiling mode on the `Debugger` adds nothing when
off. Its honest limitation: an op clock is not a time clock. One op that calls a heavy
native counts once, so a program dominated by `sort` or a store operation would under-report
it exactly where it matters. (i) measures time properly but must be signal-safe and has to
answer for `par` worker threads.

Not decided. See open question 1.

### C — allocation paths

B's capture, taken at allocation time instead of on a timer, is JProfiler's allocation-
backtrace view. It is the single most useful thing the current one-`u32` stamp cannot do.

It is also the expensive one: capturing a frame vector per allocation is orders of magnitude
more than storing a `u32`, which is why the cheap stamp was the right first design and must
stay the default. Opt-in, and probably sampled (every Nth allocation) rather than exhaustive.

Blocked on B.

### D — the driver

The "semi" half, and the part that turns three instruments into a tool.

* **One command.** `make profile` already classifies the backend, warms a native build,
  records, and reports; it should also pick the instrument (CPU / memory / both) and rank.
* **Corpus mode.** Run over `bench/` plus the `@speed`-annotated tests, emit a hot-spot
  table, and **diff against the previous capture**. Ranked hot spots that moved are the
  output; a human triages. `make speed` is the precedent — a report, never a gate.
  Note the corpus here is the *same* corpus that serves as the correctness oracle above, so
  arc D gets a second job almost free: a benchmark whose known hot spot stops appearing at
  the top is a **regression in the profiler**, not in the program, and the corpus run is
  where that shows up.
* **Honesty guards** (partly shipped): sample count in the banner, rustc share when the
  build leaked into the window, startup-cache-hit detection. Each was a wrong answer that
  looked right. The remaining one: refuse to `--annotate` a symbol whose share rests on a
  handful of samples, rather than annotating whichever won the toss (it picked a 1-sample
  `getenv` from libc during testing).

## Phase ordering

0. **Pick the oracle benchmarks before building anything** — for each arc, name the
   `bench/` program (or `@speed` test) whose hot spot is known and write down what the
   instrument must say about it. This is the cheapest step in the plan and the only one that
   can prove an instrument wrong rather than merely exercise it; written first, it is also
   reviewable while it is still cheap to change.
1. **A** next — it is the smallest, and it starts from a report that already works. It also
   delivers the one capability with no equivalent anywhere today: memory hot spots by loft
   line.
2. **D's corpus mode + remaining guards** next, on top of A. This is what makes the tooling
   semi-automatic rather than a manual perf session, and it needs no new instrument.
3. **B** — the loft-level CPU profiler. The largest design risk (open question 1) and the
   only one that closes an *impossible* cell rather than a missing one.
4. **C** — allocation paths, reusing B's capture. Do not start before B's mechanism is
   settled; C inherits whatever overhead B chooses.

## Open design questions

1. **B's sampling mechanism** — (i) `SIGPROF` for true time sampling, (iii) the existing
   per-op debug branch for free-when-off op sampling, or both (op sampling by default, time
   sampling opt-in). The op-clock distortion is real: one op can hide a whole native call.
   The watchdog precedent is the warning — an armed timeout does a mutex `try_lock` per
   function call and inflated `fib` ~2× (PERFORMANCE.md), so "cheap per call" is not cheap.
2. **Bytes or store counts in A?** Counts are what exists; bytes are what the user means by
   "memory". Deriving bytes needs each store's size at sample time.
3. **Native memory attribution — gap or decline?** `alloc_pc` is documented as *"Zero
   outside interpretation (native/tooling)"*, so the backend that just gained CPU
   attribution has no allocation attribution, and vice versa. Either the native backend gets
   an equivalent stamp, or memory profiling is declared interpreter-only and the reason is
   recorded here. Do not leave it implicit.
4. **What may ever be on by default?** Nothing in A–C, on current evidence. Worth stating so
   a later session does not re-litigate it.
5. **Does the corpus mode's diff need a stored baseline**, or is capture-and-diff against a
   previous run enough? `make speed`'s bless mechanism is the precedent to copy or reject.

## Cross-arc dependencies

- **C depends on B** — same capture, different trigger.
- **A and D are independent of B** — the memory arc and the driver can ship complete without
  the CPU sampler, which is why they lead.
- **Ownership / deps work** (OWNERSHIP_MODEL.md, LIFETIME.md) is the natural home for a
  *retention* report — "why is this store still alive" — which is loft's stronger analogue
  to a heap walker's paths-to-GC-root, because ownership is recorded rather than inferred.
  Out of scope here; noted so it is not rebuilt inside this plan.

## See also

- [PERFORMANCE.md § Profiling a run](../../PERFORMANCE.md) — the shipped capture side and the
  five choices that make a profile honest.
- [STACKTRACE.md](../../STACKTRACE.md) — `call_stack` / `CallFrame`, the vector arc B samples.
- [DEBUG.md](../../DEBUG.md) — the per-op debugger hook arc B would piggyback.
- [TESTING.md § Store-memory ceiling](../../TESTING.md) — the at-peak trigger arc A copies.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) / [LIFETIME.md](../../LIFETIME.md) — where a
  retention report belongs.
- `loft-lang/plans` issue **@PLN140** — this plan.
